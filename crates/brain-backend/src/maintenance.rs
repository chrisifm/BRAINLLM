//! `brain data prepare|maintain`: what a table needs after a bulk import so the planner and the storage match the data.
//! Extended statistics on declared filters, lz4 storage with a row rewrite that fires no trigger, `VACUUM (ANALYZE)`, a
//! bounded warm-up and a buffer gate on the list page. No `VACUUM FULL`, `REINDEX` or `pg_prewarm`: each holds a lock or
//! needs an extension the application does not otherwise require.
use crate::contract::Resource;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Above this many buffers per returned row, a list page is paying for documents it mostly does not use.
pub const BUFFERS_PER_ROW: f64 = 5.0;
const PAGE_ROWS: i64 = 50;
const REWRITE_BATCH: i64 = 500;
const WARM_ROWS: i64 = 500;
const WARM_TIMEOUT_MS: i64 = 5_000;
/// Postgres accepts at most this many expressions in one statistics object.
const STATISTICS_EXPRESSIONS: usize = 8;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// After an import: also recompresses the stored documents.
    Prepare,
    /// Periodic: statistics, vacuum, warm-up and the gate; no row is rewritten.
    Maintain,
}

fn safe(error: sqlx::Error) -> String {
    crate::db::safe(error)
}
fn sql_state(error: &sqlx::Error) -> Option<String> {
    error.as_database_error().and_then(|db| db.code().map(|code| code.to_string()))
}
async fn run(pool: &PgPool, sql: String) -> Result<(), sqlx::Error> {
    // Audited SQL: identifiers come from a validated contract or from the catalog; no request value reaches it.
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).execute(pool).await.map(|_| ())
}

async fn statistics(pool: &PgPool, r: &Resource, table: &str, apply: bool) -> Result<Value, String> {
    let mut expressions = crate::query::statistics_expressions(r);
    expressions.truncate(STATISTICS_EXPRESSIONS);
    let declared = (!expressions.is_empty()).then(|| r.object_name("_brain_st_", &expressions.join(",")));
    let stored: Vec<String> = sqlx::query_scalar("SELECT stxname::text FROM pg_statistic_ext WHERE stxrelid=to_regclass($1) AND stxname LIKE '\\_brain\\_st\\_%'")
        .bind(format!("public.{table}"))
        .fetch_all(pool)
        .await
        .map_err(safe)?;
    let missing = declared.as_ref().is_some_and(|name| !stored.contains(name));
    if apply {
        for name in stored.iter().filter(|name| Some(*name) != declared.as_ref()) {
            run(pool, format!("DROP STATISTICS public.{name}")).await.map_err(safe)?;
        }
        if let (Some(name), true) = (&declared, missing) {
            let list = expressions.iter().map(|e| format!("({e})")).collect::<Vec<_>>().join(",");
            run(pool, format!("CREATE STATISTICS {name} ON {list} FROM {table}")).await.map_err(safe)?;
        }
    }
    Ok(json!({"expressions": expressions.len(), "missing": missing}))
}

/// `ALTER` only affects new values: documents already stored keep their method until they are rewritten.
async fn compression(pool: &PgPool, table: &str, apply: bool, rewrite: bool, warnings: &mut Vec<String>) -> Result<Value, String> {
    let pending: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE pg_column_compression(data)='pglz'")))
        .fetch_one(pool)
        .await
        .map_err(safe)?;
    if !apply {
        return Ok(json!({"pglzRows": pending, "rewritten": 0}));
    }
    if let Err(error) = run(pool, format!("ALTER TABLE {table} ALTER COLUMN data SET COMPRESSION lz4")).await {
        if sql_state(&error).as_deref() == Some("0A000") {
            warnings.push(format!("{table}: this Postgres has no lz4 support, so documents stay as they are; fix: use a build with lz4 to read them faster"));
            return Ok(json!({"pglzRows": pending, "rewritten": 0, "skipped": "lz4 unavailable"}));
        }
        return Err(safe(error));
    }
    if !rewrite || pending == 0 {
        return Ok(json!({"pglzRows": pending, "rewritten": 0}));
    }
    let mut last = Uuid::nil();
    let mut rewritten = 0usize;
    loop {
        let mut tx = pool.begin().await.map_err(safe)?;
        // Triggers stay off: a rewrite that changes no value must not touch updated_at, slugs, search or the audit log.
        if let Err(error) = sqlx::raw_sql("SET LOCAL session_replication_role=replica").execute(&mut *tx).await {
            if sql_state(&error).as_deref() == Some("42501") {
                warnings.push(format!("{table}: {pending} stored documents keep their old compression because this role cannot turn triggers off for the rewrite; fix: run `brain data prepare` once as the database owner with superuser rights; new writes already use lz4"));
                return Ok(json!({"pglzRows": pending, "rewritten": rewritten, "skipped": "needs superuser"}));
            }
            return Err(safe(error));
        }
        let ids: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "WITH batch AS (SELECT id FROM {table} WHERE id>$1 AND pg_column_compression(data)='pglz' ORDER BY id LIMIT {REWRITE_BATCH}) UPDATE {table} t SET data=t.data||'{{}}'::jsonb FROM batch WHERE t.id=batch.id RETURNING t.id"
        )))
        .bind(last)
        .fetch_all(&mut *tx)
        .await
        .map_err(safe)?;
        tx.commit().await.map_err(safe)?;
        let Some(max) = ids.iter().max() else { break };
        last = *max;
        rewritten += ids.len();
    }
    Ok(json!({"pglzRows": pending, "rewritten": rewritten}))
}

/// The newest page, reading each document as a response would: `SELECT data` alone would never open its TOAST chunks.
fn page(table: &str, rows: i64) -> String {
    format!("SELECT octet_length(t.data::text) FROM {table} t ORDER BY t.created_at DESC,t.id DESC LIMIT {rows}")
}
async fn gate(pool: &PgPool, table: &str, warnings: &mut Vec<String>) -> Result<Value, String> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) {}", page(table, PAGE_ROWS)))).fetch_one(pool).await.map_err(safe)?;
    let plan: Value = row.try_get::<Value, _>(0).map_err(safe)?;
    let top = &plan[0]["Plan"];
    let blocks = top["Shared Hit Blocks"].as_f64().unwrap_or(0.0) + top["Shared Read Blocks"].as_f64().unwrap_or(0.0);
    let rows = top["Actual Rows"].as_f64().unwrap_or(0.0);
    let per_row = if rows > 0.0 { blocks / rows } else { 0.0 };
    if per_row > BUFFERS_PER_ROW {
        warnings.push(format!("{table}: a list page reads {per_row:.1} buffers per row (above {BUFFERS_PER_ROW}); fix: ask for ?fields= on lists, or move long text into its own resource"));
    }
    Ok(json!({"rows": rows as i64, "buffers": blocks as i64, "buffersPerRow": (per_row * 10.0).round() / 10.0, "ok": per_row <= BUFFERS_PER_ROW}))
}
/// Bounded on purpose: enough for the first visitors, and it gives up rather than compete with traffic.
async fn warm(pool: &PgPool, table: &str) -> Result<bool, String> {
    let mut tx = pool.begin().await.map_err(safe)?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("SET LOCAL statement_timeout={WARM_TIMEOUT_MS}"))).execute(&mut *tx).await.map_err(safe)?;
    let finished = sqlx::raw_sql(sqlx::AssertSqlSafe(page(table, WARM_ROWS))).execute(&mut *tx).await.is_ok();
    Ok(finished)
}

/// Without `apply` nothing changes: it reports what would be done and measures the gate.
pub async fn run_all(pool: &PgPool, resources: &[Resource], mode: Mode, apply: bool) -> Result<Value, String> {
    let mut results = Vec::new();
    let mut warnings = Vec::new();
    for r in resources {
        let table = format!("_brain_res_{}", r.name);
        let rows: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}"))).fetch_one(pool).await.map_err(|error| {
            safe(error);
            format!("{}: its table is missing or unreadable; fix: run `brain backend migrate` first", r.name)
        })?;
        let statistics = statistics(pool, r, &table, apply).await?;
        let compression = compression(pool, &table, apply, mode == Mode::Prepare, &mut warnings).await?;
        let mut warmed = Value::Null;
        if apply {
            // VACUUM cannot run inside a transaction, and a long one must not hit a connection's statement deadline.
            let mut conn = pool.acquire().await.map_err(safe)?;
            sqlx::raw_sql("SET statement_timeout=0").execute(&mut *conn).await.map_err(safe)?;
            let vacuumed = sqlx::raw_sql(sqlx::AssertSqlSafe(format!("VACUUM (ANALYZE) {table}"))).execute(&mut *conn).await;
            sqlx::raw_sql("RESET statement_timeout").execute(&mut *conn).await.map_err(safe)?;
            vacuumed.map_err(safe)?;
            drop(conn);
            warmed = json!(warm(pool, &table).await?);
        }
        let gate = gate(pool, &table, &mut warnings).await?;
        results.push(json!({"resource": r.name, "rows": rows, "statistics": statistics, "compression": compression, "warmed": warmed, "gate": gate}));
    }
    Ok(json!({"schemaVersion": 1, "ok": true, "applied": apply, "resources": results, "warnings": warnings}))
}
