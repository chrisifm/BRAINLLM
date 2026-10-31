//! Postgres only. Identifiers are contract-validated; request values are always bound.
use crate::Resource;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::time::Duration;
use uuid::Uuid;

const MIGRATIONS: &[(&str, &str)] = &[
    ("backend", include_str!("../migrations/0001_backend.sql")),
    ("jobs", include_str!("../migrations/0002_jobs.sql")),
    (
        "foundation",
        include_str!("../migrations/0003_foundation.sql"),
    ),
    ("links", include_str!("../migrations/0004_links.sql")),
    (
        "auth_flows",
        include_str!("../migrations/0005_auth_flows.sql"),
    ),
    ("webhooks", include_str!("../migrations/0006_webhooks.sql")),
    (
        "search_empty_fields",
        include_str!("../migrations/0007_search_empty_fields.sql"),
    ),
    ("endpoints_platform", include_str!("../migrations/0008_endpoints_platform.sql")),
];
fn checksum(sql: &str) -> String {
    Sha256::digest(sql.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub async fn connect(url: &str) -> Result<PgPool, String> {
    PgPoolOptions::new().max_connections(8).acquire_timeout(Duration::from_secs(5))
        .after_connect(|conn, _| Box::pin(async move {
            // Keep pg_catalog implicit: it resolves first, while unqualified CREATE targets public.
            sqlx::raw_sql("SET search_path=public; SET statement_timeout='5s'; SET lock_timeout='2s'; SET idle_in_transaction_session_timeout='10s';").execute(conn).await?;
            Ok(())
        })).connect(url).await.map_err(|_| "Cannot connect to Postgres; check DATABASE_URL (credentials are not logged)".into())
}
fn drift(object: &str) -> String {
    format!(
        "BDB_SCHEMA_DRIFT: {object}; review schema and run the explicit backend migration; no rows were removed"
    )
}
async fn exists(conn: &mut sqlx::PgConnection, name: &str) -> Result<bool, String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass($1)::text")
        .bind(format!("public.{name}"))
        .fetch_one(conn)
        .await
        .map(|x| x.is_some())
        .map_err(safe)
}
// Baseline adoption verifies column types/nullability, keys and referential actions before recording history.
async fn columns(
    conn: &mut sqlx::PgConnection,
    table: &str,
    expected: &[(&str, &str, bool)],
    exact: bool,
) -> Result<(), String> {
    let rows = sqlx::query("SELECT a.attname,format_type(a.atttypid,a.atttypmod) AS kind,a.attnotnull FROM pg_attribute a WHERE a.attrelid=to_regclass($1) AND a.attnum>0 AND NOT a.attisdropped").bind(format!("public.{table}")).fetch_all(&mut *conn).await.map_err(safe)?;
    if exact && rows.len() != expected.len() {
        return Err(drift(table));
    }
    for (name, kind, nonnull) in expected {
        if !rows.iter().any(|r| {
            r.get::<String, _>("attname") == *name
                && r.get::<String, _>("kind") == *kind
                && r.get::<bool, _>("attnotnull") == *nonnull
        }) {
            return Err(drift(table));
        }
    }
    Ok(())
}
async fn constraint(
    conn: &mut sqlx::PgConnection,
    table: &str,
    kind: &str,
    columns: &str,
    reference: Option<&str>,
    delete_action: Option<&str>,
) -> Result<(), String> {
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_constraint c WHERE c.conrelid=to_regclass($1) AND c.contype::text=$2 AND (SELECT string_agg(a.attname,',' ORDER BY k.n) FROM unnest(c.conkey) WITH ORDINALITY k(num,n) JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.num)=$3 AND ($4::text IS NULL OR c.confrelid=to_regclass($4)) AND ($5::text IS NULL OR c.confdeltype::text=$5) AND c.convalidated)").bind(format!("public.{table}")).bind(kind).bind(columns).bind(reference.map(|s|format!("public.{s}"))).bind(delete_action).fetch_one(conn).await.map_err(safe)?;
    if valid { Ok(()) } else { Err(drift(table)) }
}
async fn baseline(
    conn: &mut sqlx::PgConnection,
    version: usize,
    exact: bool,
) -> Result<(), String> {
    let specs: Vec<(&str, Vec<(&str, &str, bool)>)> = if version == 1 {
        vec![
            (
                "_brain_users",
                vec![
                    ("id", "uuid", true),
                    ("email", "text", true),
                    ("password_hash", "text", true),
                    ("role", "text", true),
                    ("verified", "boolean", true),
                    ("created_at", "timestamp with time zone", true),
                ],
            ),
            (
                "_brain_sessions",
                vec![
                    ("token_hash", "text", true),
                    ("user_id", "uuid", true),
                    ("expires_at", "timestamp with time zone", true),
                ],
            ),
            (
                "_brain_auth_tokens",
                vec![
                    ("token_hash", "text", true),
                    ("user_id", "uuid", true),
                    ("purpose", "text", true),
                    ("expires_at", "timestamp with time zone", true),
                ],
            ),
            (
                "_brain_auth_events",
                vec![
                    ("id", "bigint", true),
                    ("user_id", "uuid", false),
                    ("event", "text", true),
                    ("created_at", "timestamp with time zone", true),
                ],
            ),
            (
                "_brain_uploads",
                vec![
                    ("id", "uuid", true),
                    ("user_id", "uuid", true),
                    ("media_type", "text", true),
                    ("bytes", "bigint", true),
                    ("created_at", "timestamp with time zone", true),
                ],
            ),
            (
                "_brain_resource_schemas",
                vec![("name", "text", true), ("fields", "jsonb", true)],
            ),
            (
                "_brain_login_limits",
                vec![
                    ("key", "text", true),
                    ("attempts", "integer", true),
                    ("expires_at", "timestamp with time zone", true),
                ],
            ),
        ]
    } else {
        vec![(
            "_brain_jobs",
            vec![
                ("id", "uuid", true),
                ("queue", "text", true),
                ("payload", "jsonb", true),
                ("state", "text", true),
                ("attempts", "integer", true),
                ("max_attempts", "integer", true),
                ("available_at", "timestamp with time zone", true),
                ("lease_until", "timestamp with time zone", false),
                ("lease_token", "uuid", false),
                ("idempotency_key", "text", false),
                ("last_error", "text", false),
                ("created_at", "timestamp with time zone", true),
                ("finished_at", "timestamp with time zone", false),
            ],
        )]
    };
    for (table, cols) in specs {
        if !exists(conn, table).await? {
            if exact {
                continue;
            } else {
                return Err(drift(table));
            }
        }
        columns(conn, table, &cols, exact).await?;
        let pk = match table {
            "_brain_sessions" | "_brain_auth_tokens" => "token_hash",
            "_brain_resource_schemas" => "name",
            "_brain_login_limits" => "key",
            _ => "id",
        };
        constraint(conn, table, "p", pk, None, None).await?;
        if table == "_brain_users" {
            constraint(conn, table, "u", "email", None, None).await?;
            let verified_default:Option<String>=sqlx::query_scalar("SELECT pg_get_expr(d.adbin,d.adrelid) FROM pg_attrdef d JOIN pg_attribute a ON a.attrelid=d.adrelid AND a.attnum=d.adnum WHERE d.adrelid='public._brain_users'::regclass AND a.attname='verified'").fetch_optional(&mut *conn).await.map_err(safe)?;
            if verified_default.as_deref() != Some("false") {
                return Err(drift("_brain_users.verified default"));
            }
            let role_check:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_constraint WHERE conrelid='public._brain_users'::regclass AND contype='c' AND convalidated AND replace(pg_get_constraintdef(oid),' ','')='CHECK((role=ANY(ARRAY[''admin''::text,''company''::text,''candidate''::text])))')").fetch_one(&mut *conn).await.map_err(safe)?;
            let upgraded: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM public._brain_migrations WHERE version=5)",
            )
            .fetch_one(&mut *conn)
            .await
            .map_err(safe)?;
            if upgraded {
                // A replacement role constraint is accepted only after the entire immutable ledger is verified.
                if !ledger(&mut *conn).await?.contains(&5) || role_check {
                    return Err(drift("registered role migration"));
                }
                columns(
                    &mut *conn,
                    "_brain_auth_roles",
                    &[("role", "text", true)],
                    true,
                )
                .await?;
                constraint(&mut *conn, "_brain_auth_roles", "p", "role", None, None).await?;
                constraint(
                    &mut *conn,
                    "_brain_users",
                    "f",
                    "role",
                    Some("_brain_auth_roles"),
                    Some("r"),
                )
                .await?;
                let builtins:i64=sqlx::query_scalar("SELECT count(*) FROM public._brain_auth_roles WHERE role IN('admin','company','candidate','user')").fetch_one(&mut *conn).await.map_err(safe)?;
                if builtins != 4 {
                    return Err(drift("registered built-in roles"));
                }
                columns(
                    &mut *conn,
                    "_brain_auth_tokens",
                    &[
                        ("email_bound", "text", false),
                        ("auth_version", "bigint", false),
                        ("consumed_at", "timestamp with time zone", false),
                        ("activated_at", "timestamp with time zone", false),
                        ("created_at", "timestamp with time zone", true),
                    ],
                    false,
                )
                .await?;
            } else if !role_check {
                return Err(drift("_brain_users.role constraint"));
            }
        }
        if matches!(
            table,
            "_brain_sessions" | "_brain_auth_tokens" | "_brain_uploads" | "_brain_auth_events"
        ) {
            constraint(
                conn,
                table,
                "f",
                "user_id",
                Some("_brain_users"),
                Some(if table == "_brain_auth_events" {
                    "n"
                } else {
                    "c"
                }),
            )
            .await?;
        }
        if table == "_brain_jobs" {
            constraint(conn, table, "u", "queue,idempotency_key", None, None).await?;
        }
    }
    Ok(())
}
async fn resource_shape(conn: &mut sqlx::PgConnection, r: &Resource) -> Result<(), String> {
    let table = table(r);
    columns(
        conn,
        &table,
        &[
            ("id", "uuid", true),
            ("owner", "uuid", true),
            ("data", "jsonb", true),
            ("created_at", "timestamp with time zone", true),
            ("updated_at", "timestamp with time zone", true),
            ("search_text", "text", true),
            ("search_vector", "tsvector", true),
        ],
        false,
    )
    .await?;
    constraint(conn, &table, "p", "id", None, None).await?;
    constraint(conn, &table, "f", "owner", Some("_brain_users"), Some("a")).await?;
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_trigger WHERE tgrelid=to_regclass($1) AND tgname='_brain_search' AND tgenabled='O' AND tgfoid='public._brain_search_update()'::regprocedure AND tgargs=$2)").bind(format!("public.{table}")).bind(r.search.iter().flat_map(|f|f.as_bytes().iter().copied().chain(std::iter::once(0))).collect::<Vec<_>>()).fetch_one(&mut *conn).await.map_err(safe)?;
    if !valid {
        return Err(drift(&format!("{table} search trigger")));
    }
    for (name, _) in r.sql_checks() {
        if !check_exists(conn, &table, &name).await? {
            return Err(drift(&format!("{table} check constraint {name}")));
        }
    }
    for (name, _) in r.unique_sets() {
        if !unique_index_exists(conn, &table, &name).await? {
            return Err(drift(&format!("{table} unique index {name}")));
        }
    }
    Ok(())
}
async fn check_exists(conn: &mut sqlx::PgConnection, table: &str, name: &str) -> Result<bool, String> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_constraint WHERE conrelid=to_regclass($1) AND conname=$2 AND contype='c' AND convalidated)")
        .bind(format!("public.{table}"))
        .bind(name)
        .fetch_one(conn)
        .await
        .map_err(safe)
}
async fn unique_index_exists(conn: &mut sqlx::PgConnection, table: &str, name: &str) -> Result<bool, String> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid WHERE i.indrelid=to_regclass($1) AND c.relname=$2 AND i.indisunique AND i.indisvalid AND i.indpred IS NULL)")
        .bind(format!("public.{table}"))
        .bind(name)
        .fetch_one(conn)
        .await
        .map_err(safe)
}
/// Exact-type CHECK constraints and unique indexes. Legacy-only contracts produce none, so tables migrated
/// by earlier releases are untouched. Existing rows that violate a new constraint abort the whole migration.
async fn install_constraints(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    r: &Resource,
) -> Result<(), String> {
    let table = table(r);
    for (name, expr) in r.sql_checks() {
        if check_exists(tx, &table, &name).await? {
            continue;
        }
        // Audited SQL: identifiers are validated; literals come from restricted canonical alphabets.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {table} ADD CONSTRAINT {name} CHECK ({expr})"
        )))
        .execute(&mut **tx)
        .await
        .map_err(|error| {
            safe(error);
            format!("{}: existing rows violate a declared type or check constraint; review the data before migrating; nothing was changed", r.name)
        })?;
    }
    for (name, fields) in r.unique_sets() {
        if unique_index_exists(tx, &table, &name).await? {
            continue;
        }
        let columns = fields
            .iter()
            .map(|f| format!("(data->>'{f}')"))
            .collect::<Vec<_>>()
            .join(",");
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE UNIQUE INDEX {name} ON {table} ({columns})"
        )))
        .execute(&mut **tx)
        .await
        .map_err(|error| {
            safe(error);
            format!("{}: existing rows contain duplicates for a declared unique constraint; review the data before migrating; nothing was changed", r.name)
        })?;
    }
    Ok(())
}
/// Field names covered by the declared constraint or unique index named in a database error.
pub fn constraint_fields(r: &Resource, error: &sqlx::Error) -> Option<(&'static str, Vec<String>)> {
    let db = error.as_database_error()?;
    let name = db.constraint()?;
    match db.code()?.as_ref() {
        "23505" => r
            .unique_sets()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, fields)| ("unique", fields)),
        "23514" => r
            .sql_checks()
            .into_iter()
            .any(|(n, _)| n == name)
            .then(|| ("check", Vec::new())),
        _ => None,
    }
}
async fn foundation(conn: &mut sqlx::PgConnection) -> Result<(), String> {
    for (table, cols) in [
        (
            "_brain_users",
            vec![
                ("auth_version", "bigint", true),
                ("disabled_at", "timestamp with time zone", false),
                ("updated_at", "timestamp with time zone", true),
            ],
        ),
        (
            "_brain_sessions",
            vec![
                ("session_id", "uuid", true),
                ("created_at", "timestamp with time zone", true),
                ("last_used_at", "timestamp with time zone", true),
                ("revoked_at", "timestamp with time zone", false),
            ],
        ),
        (
            "_brain_identities",
            vec![
                ("provider", "text", true),
                ("subject", "text", true),
                ("user_id", "uuid", true),
                ("issuer", "text", true),
                ("verified_email", "text", false),
            ],
        ),
        (
            "_brain_refresh_families",
            vec![
                ("id", "uuid", true),
                ("session_id", "uuid", false),
                ("user_id", "uuid", true),
                ("auth_version", "bigint", true),
                ("expires_at", "timestamp with time zone", true),
                ("revoked_at", "timestamp with time zone", false),
            ],
        ),
        (
            "_brain_refresh_tokens",
            vec![
                ("token_hash", "text", true),
                ("family_id", "uuid", true),
                ("parent_hash", "text", false),
                ("replacement_hash", "text", false),
                ("expires_at", "timestamp with time zone", true),
                ("used_at", "timestamp with time zone", false),
            ],
        ),
        (
            "_brain_auth_challenges",
            vec![
                ("token_hash", "text", true),
                ("purpose", "text", true),
                ("session_id", "uuid", false),
                ("expires_at", "timestamp with time zone", true),
                ("consumed_at", "timestamp with time zone", false),
            ],
        ),
        (
            "_brain_media_renditions",
            vec![
                ("upload_id", "uuid", true),
                ("version", "integer", true),
                ("preset", "text", true),
                ("provider", "text", true),
                ("object_key", "text", true),
                ("generation", "text", true),
                ("bytes", "bigint", true),
                ("sha256", "text", true),
            ],
        ),
        (
            "_brain_media_deletions",
            vec![
                ("id", "bigint", true),
                ("upload_id", "uuid", true),
                ("provider", "text", true),
                ("object_key", "text", true),
                ("generation", "text", true),
                ("attempts", "integer", true),
                ("completed_at", "timestamp with time zone", false),
            ],
        ),
        (
            "_brain_resource_schemas",
            vec![("search_fields", "jsonb", true)],
        ),
    ] {
        columns(conn, table, &cols, false).await?;
    }
    constraint(
        conn,
        "_brain_identities",
        "p",
        "provider,subject",
        None,
        None,
    )
    .await?;
    constraint(conn, "_brain_refresh_tokens", "p", "token_hash", None, None).await?;
    constraint(
        conn,
        "_brain_refresh_tokens",
        "u",
        "parent_hash",
        None,
        None,
    )
    .await?;
    constraint(
        conn,
        "_brain_refresh_tokens",
        "f",
        "family_id",
        Some("_brain_refresh_families"),
        Some("c"),
    )
    .await?;
    constraint(
        conn,
        "_brain_refresh_families",
        "f",
        "session_id",
        Some("_brain_sessions"),
        Some("n"),
    )
    .await?;
    let extensions:i64=sqlx::query_scalar("SELECT count(*) FROM pg_extension e JOIN pg_namespace n ON n.oid=e.extnamespace WHERE e.extname IN ('pg_trgm','unaccent') AND n.nspname='public'").fetch_one(&mut *conn).await.map_err(safe)?;
    if extensions != 2 {
        return Err(drift("required pg_trgm/unaccent extensions in public"));
    }
    let triggers:i64=sqlx::query_scalar("SELECT count(*) FROM pg_trigger WHERE NOT tgisinternal AND tgenabled='O' AND ((tgrelid='public._brain_uploads'::regclass AND tgname='_brain_upload_delete' AND tgfoid='public._brain_upload_tombstone()'::regprocedure) OR (tgrelid='public._brain_media_renditions'::regclass AND tgname='_brain_rendition_delete' AND tgfoid='public._brain_rendition_tombstone()'::regprocedure))").fetch_one(conn).await.map_err(safe)?;
    if triggers != 2 {
        return Err(drift("media deletion outbox triggers"));
    }
    Ok(())
}
async fn ledger(conn: &mut sqlx::PgConnection) -> Result<Vec<i32>, String> {
    if !exists(conn, "_brain_migrations").await? {
        return Err(drift("_brain_migrations"));
    }
    columns(
        conn,
        "_brain_migrations",
        &[
            ("version", "integer", true),
            ("name", "text", true),
            ("checksum", "text", true),
            ("applied_at", "timestamp with time zone", true),
        ],
        true,
    )
    .await?;
    constraint(conn, "_brain_migrations", "p", "version", None, None).await?;
    let rows =
        sqlx::query("SELECT version,name,checksum FROM public._brain_migrations ORDER BY version")
            .fetch_all(conn)
            .await
            .map_err(safe)?;
    let mut installed = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let version: i32 = row.get("version");
        if version != (i + 1) as i32 || version as usize > MIGRATIONS.len() {
            return Err(drift("migration version"));
        }
        let (name, sql) = MIGRATIONS[i];
        if row.get::<String, _>("name") != name || row.get::<String, _>("checksum") != checksum(sql)
        {
            return Err(drift("migration checksum"));
        }
        installed.push(version);
    }
    Ok(installed)
}
/// Explicit transactional migrations; history, schema drift and resource contracts fail closed.
pub async fn migrate(pool: &PgPool, resources: &[Resource]) -> Result<(), String> {
    if resources.len() > 128 {
        return Err("backend migration supports at most 128 resources per invocation".into());
    }
    for r in resources {
        r.validate()?;
    }
    let mut tx = pool.begin().await.map_err(safe)?;
    sqlx::raw_sql("SET LOCAL statement_timeout='60s'; SET LOCAL lock_timeout='5s'; SET LOCAL idle_in_transaction_session_timeout='60s';").execute(&mut *tx).await.map_err(safe)?;
    sqlx::query("SELECT pg_advisory_xact_lock(728196410)")
        .execute(&mut *tx)
        .await
        .map_err(safe)?;
    sqlx::raw_sql("CREATE TABLE IF NOT EXISTS public._brain_migrations(version integer PRIMARY KEY,name text NOT NULL,checksum text NOT NULL,applied_at timestamptz NOT NULL DEFAULT now())").execute(&mut *tx).await.map_err(safe)?;
    let installed = ledger(&mut tx).await?;
    for (index, (name, sql)) in MIGRATIONS.iter().enumerate() {
        if index < installed.len() {
            continue;
        }
        if index < 2 {
            baseline(&mut tx, index + 1, true).await?;
        }
        sqlx::raw_sql(*sql)
            .execute(&mut *tx)
            .await
            .map_err(|_| drift(name))?;
        if index < 2 {
            baseline(&mut tx, index + 1, false).await?;
        }
        sqlx::query("INSERT INTO public._brain_migrations(version,name,checksum) VALUES($1,$2,$3)")
            .bind((index + 1) as i32)
            .bind(name)
            .bind(checksum(sql))
            .execute(&mut *tx)
            .await
            .map_err(safe)?;
    }
    for r in resources {
        let fields = r.recorded_fields();
        let old =
            sqlx::query("SELECT fields,search_fields FROM _brain_resource_schemas WHERE name=$1")
                .bind(&r.name)
                .fetch_optional(&mut *tx)
                .await
                .map_err(safe)?;
        if old.as_ref().is_some_and(|row| {
            row.get::<Value, _>("fields") != fields
                || row.get::<Value, _>("search_fields") != json!(r.search)
        }) {
            return Err(drift(&format!("resource {} contract", r.name)));
        }
        let table = table(r);
        // The legacy generated `search` column is preserved for existing data but removed from indexing/use.
        // Audited SQL: table/field identifiers pass Resource::validate; all request values remain bound.
        let ddl = format!(
            "CREATE TABLE IF NOT EXISTS {table}(id uuid PRIMARY KEY,owner uuid NOT NULL REFERENCES _brain_users(id),data jsonb NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now()); ALTER TABLE {table} ADD COLUMN IF NOT EXISTS search_text text NOT NULL DEFAULT ''; ALTER TABLE {table} ADD COLUMN IF NOT EXISTS search_vector tsvector NOT NULL DEFAULT ''::tsvector; DROP INDEX IF EXISTS {table}_search; CREATE INDEX IF NOT EXISTS {table}_fts ON {table} USING gin(search_vector); CREATE INDEX IF NOT EXISTS {table}_trgm ON {table} USING gin(search_text public.gin_trgm_ops); CREATE INDEX IF NOT EXISTS {table}_chronological ON {table}(created_at DESC,id DESC); CREATE INDEX IF NOT EXISTS {table}_owner_chronological ON {table}(owner,created_at DESC,id DESC);"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
            .execute(&mut *tx)
            .await
            .map_err(safe)?;
        let args = r
            .search
            .iter()
            .map(|f| format!("'{f}'"))
            .collect::<Vec<_>>()
            .join(",");
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TRIGGER IF EXISTS _brain_search ON {table}; CREATE TRIGGER _brain_search BEFORE INSERT OR UPDATE OF data ON {table} FOR EACH ROW EXECUTE FUNCTION public._brain_search_update({args});"))).execute(&mut *tx).await.map_err(safe)?;
        sqlx::query("INSERT INTO _brain_resource_schemas(name,fields,search_fields) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(&r.name).bind(fields).bind(json!(r.search)).execute(&mut *tx).await.map_err(safe)?;
        install_constraints(&mut tx, r).await?;
        resource_shape(&mut tx, r).await?;
    }
    foundation(&mut tx).await?;
    tx.commit().await.map_err(safe)
}
fn table(r: &Resource) -> String {
    // Resource is validated at service startup; keep this assertion next to every dynamic SQL identifier.
    assert!(crate::contract::identifier(&r.name));
    format!("_brain_res_{}", r.name)
}
fn failure_metadata(error: &sqlx::Error) -> Value {
    // SQLSTATE is a finite five-character identifier. Never format an error, SQL text, URL or bound value.
    let code = error
        .as_database_error()
        .and_then(|e| e.code())
        .filter(|code| {
            code.len() == 5
                && code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        })
        .map(|code| code.into_owned());
    let category = match code.as_deref() {
        Some("57014") => "statement deadline",
        Some("55P03") => "lock deadline",
        Some("23505") => "unique constraint",
        Some("23503") => "reference constraint",
        Some("23514") => "check constraint",
        Some("42501") => "database privilege",
        _ => "database operation",
    };
    json!({"schemaVersion":1,"event":"database-operation-failed","rid":crate::http::request_reference(),"sqlstate":code,"category":category,"private":true,"valuesCaptured":false})
}
pub fn safe(error: sqlx::Error) -> String {
    let metadata = failure_metadata(&error);
    eprintln!("{metadata}");
    format!(
        "Postgres {} failed; inspect sanitized server diagnostics",
        metadata["category"]
            .as_str()
            .unwrap_or("database operation")
    )
}

/// Public record columns. Timestamps are rendered by Postgres in UTC as `YYYY-MM-DDTHH:MM:SS.sssZ`
/// (the JavaScript `toISOString` form), so no date library or local time zone enters the response.
#[macro_export]
#[doc(hidden)]
macro_rules! record_columns {
    () => {
        "id,owner,data,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_at_utc,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS updated_at_utc"
    };
}
pub(crate) fn record(row: sqlx::postgres::PgRow) -> Value {
    json!({"id":row.get::<Uuid,_>("id"),"owner":row.get::<Uuid,_>("owner"),"data":row.get::<Value,_>("data"),"created_at":row.get::<String,_>("created_at_utc"),"updated_at":row.get::<String,_>("updated_at_utc")})
}
fn query(
    prefix: &'static str,
    r: &Resource,
    suffix: &'static str,
) -> sqlx::QueryBuilder<sqlx::Postgres> {
    let mut q = sqlx::QueryBuilder::new(prefix);
    // Only this validated identifier enters SQL text. Request data below always uses bind parameters.
    q.push(table(r)).push(suffix);
    q
}
pub async fn list(
    pool: &PgPool,
    r: &Resource,
    owner: Option<Uuid>,
    search: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<Value>, sqlx::Error> {
    crate::search::list(pool, r, owner, search, limit, offset, None).await
}
pub async fn get(pool: &PgPool, r: &Resource, id: Uuid) -> Result<Option<Value>, sqlx::Error> {
    query(concat!("SELECT ", record_columns!(), " FROM "), r, " WHERE id=$1")
        .build()
        .bind(id)
        .fetch_optional(pool)
        .await
        .map(|r| r.map(record))
}
async fn hook(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    r: &Resource,
    operation: &str,
    record: &Value,
) -> Result<(), sqlx::Error> {
    if let Some(queue) = r.hooks.queue(operation) {
        // Mutation rollback also removes the event; queue overload rolls back the mutation rather than losing work.
        let payload = json!({"resource":r.name,"operation":operation,"record":record});
        crate::jobs::enqueue_tx(tx, queue, payload, None, 3)
            .await
            .map_err(|_| sqlx::Error::Protocol("Resource hook could not be enqueued".into()))?;
    }
    Ok(())
}
pub async fn create(
    pool: &PgPool,
    r: &Resource,
    owner: Uuid,
    data: Value,
) -> Result<Value, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let row = query(
        "INSERT INTO ",
        r,
        concat!("(id,owner,data) VALUES ($1,$2,$3) RETURNING ", record_columns!()),
    )
    .build()
    .bind(Uuid::new_v4())
    .bind(owner)
    .bind(data)
    .fetch_one(&mut *tx)
    .await
    .map(record)?;
    hook(&mut tx, r, "create", &row).await?;
    tx.commit().await?;
    Ok(row)
}
pub async fn update(
    pool: &PgPool,
    r: &Resource,
    id: Uuid,
    owner: Option<Uuid>,
    data: Value,
) -> Result<Option<Value>, sqlx::Error> {
    update_checked(pool, r, id, owner, data, None).await
}
pub async fn update_checked(
    pool: &PgPool,
    r: &Resource,
    id: Uuid,
    owner: Option<Uuid>,
    data: Value,
    expected: Option<&str>,
) -> Result<Option<Value>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let row=query("UPDATE ",r,concat!(" SET data=$2,updated_at=now() WHERE id=$1 AND ($3::uuid IS NULL OR owner=$3) AND ($4::text IS NULL OR xmin::text=$4) RETURNING ",record_columns!())).build()
        .bind(id).bind(data).bind(owner).bind(expected).fetch_optional(&mut *tx).await?.map(record);
    if let Some(row) = &row {
        hook(&mut tx, r, "update", row).await?;
    }
    tx.commit().await?;
    Ok(row)
}
pub async fn delete(
    pool: &PgPool,
    r: &Resource,
    id: Uuid,
    owner: Option<Uuid>,
) -> Result<bool, sqlx::Error> {
    delete_checked(pool, r, id, owner, None).await
}
pub async fn delete_checked(
    pool: &PgPool,
    r: &Resource,
    id: Uuid,
    owner: Option<Uuid>,
    expected: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let row = query(
        "DELETE FROM ",
        r,
        concat!(" WHERE id=$1 AND ($2::uuid IS NULL OR owner=$2) AND ($3::text IS NULL OR xmin::text=$3) RETURNING ", record_columns!()),
    )
    .build()
    .bind(id)
    .bind(owner)
    .bind(expected)
    .fetch_optional(&mut *tx)
    .await?
    .map(record);
    if let Some(row) = &row {
        hook(&mut tx, r, "delete", row).await?;
    }
    tx.commit().await?;
    Ok(row.is_some())
}

/// Serving never creates or rewrites schema. A stale runtime contract fails before opening HTTP.
pub async fn verify(pool: &PgPool, resources: &[Resource]) -> Result<(), String> {
    let installed: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('_brain_resource_schemas')::text")
            .fetch_one(pool)
            .await
            .map_err(safe)?;
    if installed.is_none() {
        return Err(
            "Backend schema is not installed; run brain backend migrate for this environment"
                .into(),
        );
    }
    let mut conn = pool.acquire().await.map_err(safe)?;
    if ledger(&mut conn).await?.len() != MIGRATIONS.len() {
        return Err(drift("pending foundation migrations"));
    }
    baseline(&mut conn, 1, false).await?;
    baseline(&mut conn, 2, false).await?;
    foundation(&mut conn).await?;
    columns(
        &mut conn,
        "_brain_webhook_events",
        &[
            ("id", "uuid", true),
            ("direction", "text", true),
            ("endpoint", "text", true),
            ("event_id", "uuid", true),
            ("job_id", "uuid", true),
            ("queue", "text", true),
            ("key_id", "text", true),
            ("nonce", "bytea", true),
            ("ciphertext", "bytea", true),
            ("created_at", "timestamp with time zone", true),
        ],
        true,
    )
    .await?;
    columns(
        &mut conn,
        "_brain_webhook_attempts",
        &[
            ("event_id", "uuid", true),
            ("attempt", "integer", true),
            ("status", "integer", false),
            ("code", "text", true),
            ("elapsed_ms", "integer", true),
            ("finished_at", "timestamp with time zone", true),
        ],
        true,
    )
    .await?;
    constraint(&mut conn, "_brain_webhook_events", "p", "id", None, None).await?;
    constraint(
        &mut conn,
        "_brain_webhook_events",
        "u",
        "job_id",
        None,
        None,
    )
    .await?;
    constraint(
        &mut conn,
        "_brain_webhook_events",
        "u",
        "direction,endpoint,event_id",
        None,
        None,
    )
    .await?;
    constraint(
        &mut conn,
        "_brain_webhook_events",
        "u",
        "key_id,nonce",
        None,
        None,
    )
    .await?;
    constraint(
        &mut conn,
        "_brain_webhook_events",
        "f",
        "job_id",
        Some("_brain_jobs"),
        Some("c"),
    )
    .await?;
    constraint(
        &mut conn,
        "_brain_webhook_attempts",
        "p",
        "event_id,attempt",
        None,
        None,
    )
    .await?;
    constraint(
        &mut conn,
        "_brain_webhook_attempts",
        "f",
        "event_id",
        Some("_brain_webhook_events"),
        Some("c"),
    )
    .await?;
    for r in resources {
        r.validate()?;
        resource_shape(&mut conn, r).await?;
        let stored =
            sqlx::query("SELECT fields,search_fields FROM _brain_resource_schemas WHERE name=$1")
                .bind(&r.name)
                .fetch_optional(pool)
                .await
                .map_err(safe)?;
        if stored.as_ref().is_none_or(|row| {
            row.get::<Value, _>("fields") != r.recorded_fields()
                || row.get::<Value, _>("search_fields") != json!(r.search)
        }) {
            return Err(format!(
                "{}: database schema differs from schema.jsbrain; review migrations before serving",
                r.name
            ));
        }
    }
    Ok(())
}

/// Explicit finite retention. No row contents are returned; used refresh digests survive family expiry.
pub async fn cleanup(pool: &PgPool, limit: i64) -> Result<Value, String> {
    if !(1..=1000).contains(&limit) {
        return Err("cleanup limit must be 1..1000".into());
    }
    let mut tx = pool.begin().await.map_err(safe)?;
    let mut remaining = limit;
    let mut counts = serde_json::Map::new();
    let tasks = [
        (
            "sessions",
            "_brain_sessions",
            "token_hash",
            "expires_at<now() AND NOT EXISTS(SELECT 1 FROM _brain_refresh_families f WHERE f.session_id=_brain_sessions.session_id)",
        ),
        (
            "tokens",
            "_brain_auth_tokens",
            "token_hash",
            "expires_at<now()",
        ),
        (
            "loginLimits",
            "_brain_login_limits",
            "key",
            "expires_at<now()",
        ),
        (
            "challenges",
            "_brain_auth_challenges",
            "token_hash",
            "expires_at<now()",
        ),
        (
            "refreshTokens",
            "_brain_refresh_tokens",
            "token_hash",
            "EXISTS(SELECT 1 FROM _brain_refresh_families f WHERE f.id=_brain_refresh_tokens.family_id AND f.expires_at<now())",
        ),
        (
            "refreshFamilies",
            "_brain_refresh_families",
            "id",
            "expires_at<now() AND NOT EXISTS(SELECT 1 FROM _brain_refresh_tokens t WHERE t.family_id=_brain_refresh_families.id)",
        ),
        (
            "authEvents",
            "_brain_auth_events",
            "id",
            "created_at<now()-interval '90 days'",
        ),
        (
            "mediaDeletions",
            "_brain_media_deletions",
            "id",
            "completed_at<now()-interval '90 days'",
        ),
    ];
    for (label, table, key, predicate) in tasks {
        // Audited SQL: this table/key/predicate tuple is a fixed internal allowlist, never caller text.
        let count = if remaining == 0 {
            0
        } else {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE {key} IN(SELECT {key} FROM {table} WHERE {predicate} ORDER BY {key} FOR UPDATE SKIP LOCKED LIMIT $1)" ))).bind(remaining).execute(&mut *tx).await.map_err(safe)?.rows_affected()
        };
        remaining -= count as i64;
        counts.insert(label.into(), json!(count));
    }
    tx.commit().await.map_err(safe)?;
    Ok(json!({"counts":counts,"deleted":limit-remaining,"limit":limit,"possiblyMore":remaining==0}))
}

#[derive(Clone, Debug)]
pub struct MediaDeletion {
    pub id: i64,
    pub upload_id: Uuid,
    pub provider: String,
    pub object_key: String,
    pub generation: String,
    pub lease: Uuid,
}
/// Claims only persisted owned object identities. The storage worker must enforce its configured root/prefix.
pub async fn claim_media_deletions(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<MediaDeletion>, String> {
    if !(1..=100).contains(&limit) {
        return Err("media deletion batch must be 1..100".into());
    }
    let token = Uuid::new_v4();
    let rows=sqlx::query("WITH next AS(SELECT id FROM _brain_media_deletions WHERE completed_at IS NULL AND attempts<10 AND (lease_until IS NULL OR lease_until<now()) ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT $1) UPDATE _brain_media_deletions d SET attempts=attempts+1,lease_until=now()+interval '30 seconds',lease_token=$2 FROM next WHERE d.id=next.id RETURNING d.id,d.upload_id,d.provider,d.object_key,d.generation").bind(limit).bind(token).fetch_all(pool).await.map_err(safe)?;
    Ok(rows
        .into_iter()
        .map(|r| MediaDeletion {
            id: r.get("id"),
            upload_id: r.get("upload_id"),
            provider: r.get("provider"),
            object_key: r.get("object_key"),
            generation: r.get("generation"),
            lease: token,
        })
        .collect())
}
pub async fn ack_media_deletion(pool: &PgPool, item: &MediaDeletion) -> Result<bool, String> {
    sqlx::query("UPDATE _brain_media_deletions SET completed_at=now(),lease_until=NULL,lease_token=NULL WHERE id=$1 AND lease_token=$2 AND lease_until>now() AND completed_at IS NULL").bind(item.id).bind(item.lease).execute(pool).await.map(|r|r.rows_affected()==1).map_err(safe)
}

/// Explicit finite backfill; callers repeat using nextAfter. No unbounded UPDATE is hidden in migration.
pub async fn rebuild_search(
    pool: &PgPool,
    r: &Resource,
    limit: i64,
    after: Option<Uuid>,
) -> Result<Value, String> {
    r.validate()?;
    if !(1..=1000).contains(&limit) {
        return Err("search rebuild limit must be 1..1000".into());
    }
    let mut tx = pool.begin().await.map_err(safe)?;
    resource_shape(&mut tx, r).await?;
    let stored: Value =
        sqlx::query_scalar("SELECT search_fields FROM _brain_resource_schemas WHERE name=$1")
            .bind(&r.name)
            .fetch_one(&mut *tx)
            .await
            .map_err(safe)?;
    if stored != json!(r.search) {
        return Err(drift("search rebuild contract"));
    }
    let table = table(r);
    // Audited SQL: Resource::validate and table() guard the only interpolated identifier; cursor/limit are bound.
    let rows=sqlx::query(sqlx::AssertSqlSafe(format!("WITH batch AS(SELECT id FROM {table} WHERE ($1::uuid IS NULL OR id>$1) ORDER BY id FOR UPDATE LIMIT $2) UPDATE {table} t SET data=t.data FROM batch WHERE t.id=batch.id RETURNING t.id"))).bind(after).bind(limit).fetch_all(&mut *tx).await.map_err(safe)?;
    let ids: Vec<Uuid> = rows.into_iter().map(|r| r.get("id")).collect();
    tx.commit().await.map_err(safe)?;
    Ok(
        json!({"updated":ids.len(),"limit":limit,"nextAfter":ids.iter().max(),"possiblyMore":ids.len()==limit as usize}),
    )
}

#[cfg(test)]
mod private_failure_tests {
    use super::*;
    #[test]
    fn driver_messages_and_values_never_enter_private_metadata_or_public_error() {
        let metadata = failure_metadata(&sqlx::Error::Protocol(
            "synthetic-private-driver-message".into(),
        ));
        assert_eq!(metadata["event"], "database-operation-failed");
        assert_eq!(metadata["valuesCaptured"], false);
        assert_eq!(metadata["sqlstate"], Value::Null);
        assert!(
            !metadata
                .to_string()
                .contains("synthetic-private-driver-message")
        );
        let public = safe(sqlx::Error::Protocol(
            "synthetic-private-driver-message".into(),
        ));
        assert_eq!(
            public,
            "Postgres database operation failed; inspect sanitized server diagnostics"
        );
    }
}

/// Internal optimistic revision: never expose xmin as a durable public resource version.
pub async fn get_version(
    pool: &PgPool,
    r: &Resource,
    id: Uuid,
) -> Result<Option<(Value, String)>, sqlx::Error> {
    query(
        concat!("SELECT ", record_columns!(), ",xmin::text AS revision FROM "),
        r,
        " WHERE id=$1",
    )
    .build()
    .bind(id)
    .fetch_optional(pool)
    .await
    .map(|row| {
        row.map(|row| {
            let revision = row.get::<String, _>("revision");
            (record(row), revision)
        })
    })
}
