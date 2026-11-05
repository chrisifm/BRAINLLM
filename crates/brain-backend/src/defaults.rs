//! Explicit, bounded repair for defaults approved before older releases backfilled existing documents.
//! Preview is read-only; applying updates only absent keys unless include_null was explicitly selected.
use crate::Resource;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

pub async fn fill(pool: &PgPool, resources: &[Resource], apply: bool, include_null: bool, limit: u32) -> Result<Value, String> {
    if !(1..=10000).contains(&limit) {
        return Err("default fill limit must be 1..10000 rows per field".into());
    }
    for resource in resources { resource.validate()?; }
    tokio::time::timeout(std::time::Duration::from_secs(60), fill_inner(pool, resources, apply, include_null, limit))
        .await.map_err(|_| "Default fill exceeded its 60 second deadline; fix: select one --resource and a smaller --limit".to_string())?
}

async fn fill_inner(pool: &PgPool, resources: &[Resource], apply: bool, include_null: bool, limit: u32) -> Result<Value, String> {
    crate::db::verify(pool, resources).await?;
    let mut tx = pool.begin().await.map_err(crate::db::safe)?;
    sqlx::raw_sql("SET LOCAL lock_timeout='2s'; SET LOCAL statement_timeout='30s'")
        .execute(&mut *tx).await.map_err(crate::db::safe)?;
    // Cooperates with db::migrate's exclusive lock; never repairs data against a changing recorded schema.
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1)").bind(crate::db::MIGRATION_LOCK)
        .execute(&mut *tx).await.map_err(crate::db::safe)?;
    let mut reports = Vec::new();
    let mut writes = 0u64;
    let mut more = false;
    for resource in resources {
        let recorded: Option<Value> = sqlx::query_scalar("SELECT fields FROM public._brain_resource_schemas WHERE name=$1")
            .bind(&resource.name).fetch_optional(&mut *tx).await.map_err(crate::db::safe)?;
        if recorded.as_ref() != Some(&resource.recorded_fields()) {
            return Err(format!("{}: the recorded contract changed; fix: run brain backend migrate or review it with brain backend approve-contract before filling defaults",resource.name));
        }
        // Resource and field identifiers were validated above; only the table identifier is interpolated.
        let table = format!("public._brain_res_{}", resource.name);
        let mut fields = Vec::new();
        for (name, field) in &resource.fields {
            let Some(value) = field.ty().zip(field.default.as_ref()).and_then(|(ty, value)| ty.normalize(value, field.required)).filter(|value| !value.is_null()) else { continue };
            let counts = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT count(*) FILTER (WHERE NOT(data ? $1)) AS absent, count(*) FILTER (WHERE data->$1='null'::jsonb) AS explicit_null FROM {table}")))
                .bind(name).fetch_one(&mut *tx).await.map_err(crate::db::safe)?;
            let absent: i64 = counts.get("absent");
            let nulls: i64 = counts.get("explicit_null");
            let eligible = absent + if include_null { nulls } else { 0 };
            let condition = "(NOT(data ? $1) OR ($3::boolean AND data->$1='null'::jsonb))";
            let updated = if apply && eligible > 0 {
                // Keyset selection is bounded and skips rows another writer owns instead of queuing behind them.
                let sql = format!("WITH chosen AS (SELECT id FROM {table} WHERE {condition} ORDER BY id LIMIT $4 FOR UPDATE SKIP LOCKED) UPDATE {table} AS target SET data=target.data||jsonb_build_object($1::text,$2::jsonb) FROM chosen WHERE target.id=chosen.id");
                sqlx::query(sqlx::AssertSqlSafe(sql)).bind(name).bind(&value).bind(include_null).bind(i64::from(limit))
                    .execute(&mut *tx).await.map_err(crate::db::safe)?.rows_affected()
            } else { 0 };
            let remaining: i64 = if apply {
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE NOT(data ? $1) OR ($2::boolean AND data->$1='null'::jsonb)")))
                    .bind(name).bind(include_null).fetch_one(&mut *tx).await.map_err(crate::db::safe)?
            } else { eligible };
            writes += updated;
            more |= remaining > 0;
            fields.push(json!({"field":name,"absentRows":absent,"nullRows":nulls,"eligibleRows":eligible,"batchLimit":limit,"updatedRows":updated,"remainingRows":remaining}));
        }
        reports.push(json!({"resource":resource.name,"fields":fields}));
    }
    tx.commit().await.map_err(crate::db::safe)?;
    Ok(json!({"schemaVersion":1,"applied":apply,"includeNull":include_null,"fieldWrites":writes,"hasMore":more,"resources":reports,
        "next":if !apply {"Review counts, then add --apply; explicit nulls are preserved unless --include-null is also set"} else if more {"Run the same command again for the next bounded batch; locked rows may remain"} else {"No eligible fields remain"}}))
}
