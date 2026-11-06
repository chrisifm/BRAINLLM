//! Bounded observation of the connected database and role. No user data, query text or persistent mutation.
use serde_json::{Value, json};
use sqlx::{ConnectOptions, Connection, PgConnection};
use std::time::Duration;

pub const MAX_LIMIT: u32 = 200;
const TOTAL_SECONDS: u64 = 20;
const SETTINGS: &str = "'work_mem','maintenance_work_mem','shared_buffers','effective_cache_size','max_connections','superuser_reserved_connections','reserved_connections','statement_timeout','lock_timeout','idle_in_transaction_session_timeout','idle_session_timeout','autovacuum','autovacuum_vacuum_scale_factor','autovacuum_analyze_scale_factor','track_counts','stats_fetch_consistency','max_parallel_workers_per_gather'";
const CONNECTIONS: &str = "SELECT jsonb_build_object('clientBackends',count(*),'active',count(*) FILTER(WHERE state='active'),'idle',count(*) FILTER(WHERE state='idle'),'idleInTransaction',count(*) FILTER(WHERE state='idle in transaction'),'waitingOnLock',count(*) FILTER(WHERE wait_event_type='Lock'),'auditConnectionExcluded',true) FROM pg_catalog.pg_stat_activity WHERE datid=(SELECT oid FROM pg_catalog.pg_database WHERE datname=current_database()) AND usesysid=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user) AND pid<>pg_backend_pid()";
const TABLES: &str = "SELECT coalesce(jsonb_agg(to_jsonb(s)), '[]'::jsonb) FROM (SELECT t.schemaname AS schema,t.relname AS \"table\",t.seq_scan,t.seq_tup_read,t.idx_scan,t.idx_tup_fetch,t.n_live_tup,t.n_dead_tup,t.n_mod_since_analyze,t.last_vacuum::text,t.last_autovacuum::text,t.last_analyze::text,t.last_autoanalyze::text,t.vacuum_count,t.autovacuum_count,pg_catalog.pg_total_relation_size(t.relid) AS total_bytes FROM pg_catalog.pg_stat_user_tables t JOIN pg_catalog.pg_class c ON c.oid=t.relid WHERE pg_catalog.has_schema_privilege(t.schemaname,'USAGE') AND pg_catalog.has_table_privilege(c.oid,'SELECT') ORDER BY t.n_dead_tup DESC,t.seq_scan DESC,t.relid LIMIT $1) s";
const INDEXES: &str = "SELECT coalesce(jsonb_agg(to_jsonb(s)), '[]'::jsonb) FROM (SELECT i.schemaname AS schema,i.relname AS \"table\",i.indexrelname AS \"index\",i.idx_scan,i.idx_tup_read,i.idx_tup_fetch,pg_catalog.pg_relation_size(i.indexrelid) AS bytes FROM pg_catalog.pg_stat_user_indexes i WHERE pg_catalog.has_schema_privilege(i.schemaname,'USAGE') AND pg_catalog.has_table_privilege(i.relid,'SELECT') ORDER BY i.idx_scan,i.relid,i.indexrelid LIMIT $1) s";
const SCOPE: &str = "SELECT jsonb_build_object('database',current_database(),'role',current_user,'databaseId',(SELECT oid::text FROM pg_catalog.pg_database WHERE datname=current_database()),'roleId',(SELECT oid::text FROM pg_catalog.pg_roles WHERE rolname=current_user),'serverVersion',current_setting('server_version_num'),'observedAt',transaction_timestamp()::text,'transactionReadOnly',current_setting('transaction_read_only')='on','statementTimeout',current_setting('statement_timeout'),'lockTimeout',current_setting('lock_timeout'),'idleTransactionTimeout',current_setting('idle_in_transaction_session_timeout'),'databaseConnectionLimit',(SELECT datconnlimit FROM pg_catalog.pg_database WHERE datname=current_database()),'roleConnectionLimit',(SELECT rolconnlimit FROM pg_catalog.pg_roles WHERE rolname=current_user))";

#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: &'static str,
    pub sqlstate: Option<String>,
}
impl Failure {
    fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            sqlstate: None,
        }
    }
    pub fn report(&self) -> Value {
        json!({"schemaVersion":1,"ok":false,"readOnly":true,"error":{"code":self.code,"message":self.message,"sqlstate":self.sqlstate},"queryTextCaptured":false,"valuesCaptured":false})
    }
}
fn state(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .filter(|c| {
            c.len() == 5
                && c.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        })
        .map(|c| c.into_owned())
}
fn failure(error: sqlx::Error) -> Failure {
    Failure {
        code: "DB_AUDIT_QUERY_UNAVAILABLE",
        message: "Database audit metadata is unavailable; inspect its safe SQLSTATE and the current role's catalog permissions",
        sqlstate: state(&error),
    }
}
fn unavailable(error: &sqlx::Error) -> Value {
    let code = state(error);
    let reason = match code.as_deref() {
        Some("42501") => "permission-denied",
        Some("42P01" | "42883") => "not-installed-or-unsupported",
        Some("55000") => "not-preloaded",
        Some("57014") => "statement-deadline",
        Some("55P03") => "lock-deadline",
        _ => "metadata-unavailable",
    };
    json!({"status":"unavailable","reason":reason,"sqlstate":code,"observations":Value::Null})
}
/// Catalog-derived names are always quoted as identifiers; they are never executable fragments.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
async fn section(conn: &mut PgConnection, sql: &str, limit: Option<u32>) -> Result<Value, Failure> {
    sqlx::raw_sql("SAVEPOINT brain_db_audit_section")
        .execute(&mut *conn)
        .await
        .map_err(failure)?;
    // Every dynamic fragment is either the fixed setting-name allowlist or a quoted extension namespace.
    let query = sqlx::query_scalar::<_, Value>(sqlx::AssertSqlSafe(sql));
    let result = if let Some(limit) = limit {
        query.bind(i64::from(limit) + 1).fetch_one(&mut *conn).await
    } else {
        query.fetch_one(&mut *conn).await
    };
    let report = match result {
        Ok(mut value) => {
            let mut truncated = false;
            if let (Some(limit), Some(rows)) = (limit, value.as_array_mut()) {
                truncated = rows.len() > limit as usize;
                rows.truncate(limit as usize);
            }
            json!({"status":"available","observations":value,"truncated":truncated})
        }
        Err(error) => {
            sqlx::raw_sql("ROLLBACK TO SAVEPOINT brain_db_audit_section")
                .execute(&mut *conn)
                .await
                .map_err(failure)?;
            unavailable(&error)
        }
    };
    sqlx::raw_sql("RELEASE SAVEPOINT brain_db_audit_section")
        .execute(&mut *conn)
        .await
        .map_err(failure)?;
    Ok(report)
}
fn setting<'a>(settings: &'a Value, name: &str) -> Option<&'a Value> {
    settings["observations"]
        .as_array()?
        .iter()
        .find(|row| row["name"] == name)
}
fn integer(settings: &Value, name: &str) -> Option<u64> {
    setting(settings, name)?["sourceResetValue"]
        .as_str()?
        .parse()
        .ok()
}
fn bytes(settings: &Value, name: &str) -> Option<u64> {
    let row = setting(settings, name)?;
    let n = integer(settings, name)?;
    n.checked_mul(match row["unit"].as_str().unwrap_or("") {
        "B" => 1,
        "kB" => 1024,
        "MB" => 1024 * 1024,
        "8kB" => 8 * 1024,
        _ => return None,
    })
}
fn capacity(scope: &Value, settings: &Value) -> Value {
    let max = integer(settings, "max_connections");
    let reserve = integer(settings, "superuser_reserved_connections")
        .unwrap_or(0)
        .saturating_add(integer(settings, "reserved_connections").unwrap_or(0));
    let capacity = max.map(|n| n.saturating_sub(reserve));
    let work = bytes(settings, "work_mem");
    let scenario = work
        .zip(max)
        .and_then(|(w, c)| w.checked_mul(c))
        .map(|n| n.to_string());
    json!({"maxConnections":max,"reservedConnections":reserve,"nonReservedCapacityUpperBound":capacity,
        "databaseConnectionLimit":scope["databaseConnectionLimit"],"roleConnectionLimit":scope["roleConnectionLimit"],
        "workMemBytesPerOperation":work,"oneOperationPerMaxConnectionScenarioBytes":scenario,
        "actualMemoryUsed":Value::Null,"oomCause":Value::Null,
        "interpretation":"work_mem times max_connections is only a one-operation-per-connection capacity scenario. It is neither observed RAM use nor a total memory upper bound: one query can have several operations or parallel workers. Current role/database limits are ceilings, not measured free cluster capacity."})
}
fn recommendations(scope: &Value, settings: &Value, tables: &Value) -> Vec<Value> {
    let role = quoted(scope["role"].as_str().unwrap_or(""));
    let db = quoted(scope["database"].as_str().unwrap_or(""));
    let mut out = Vec::new();
    for (name, value) in [
        ("statement_timeout", "5s"),
        ("lock_timeout", "1s"),
        ("idle_in_transaction_session_timeout", "10s"),
    ] {
        if integer(settings, name) == Some(0) {
            out.push(json!({"id":format!("review-{name}"),"scope":"current-role-in-current-database","requiresOperatorApproval":true,"applied":false,
                "reason":"The role's reset value has no deadline. Review this candidate against legitimate long-running work and test it before approving.",
                "sql":format!("ALTER ROLE {role} IN DATABASE {db} SET {name} = '{value}';")}));
        }
    }
    if tables["observations"].as_array().is_some_and(|rows| {
        rows.iter()
            .any(|r| r["n_dead_tup"].as_u64().unwrap_or(0) > 1000)
    }) {
        out.push(json!({"id":"review-table-maintenance","scope":"readable-tables-in-current-database","requiresOperatorApproval":true,"applied":false,
            "reason":"Some sampled tables have more than 1000 estimated dead tuples. Review vacuum/analyze history and workload before using the app's explicit maintenance workflow; the audit executes no maintenance."}));
    }
    out
}
async fn collect(url: &str, limit: u32, production: bool) -> Result<Value, Failure> {
    let options = crate::boot::require_verified_tls(url, production)
        .map_err(|_| {
            Failure::new(
                "DB_AUDIT_CONNECTION_POLICY",
                "Use a valid Postgres DATABASE_URL; remote production databases require sslmode=verify-full; connection details are never reported",
            )
        })?
        .application_name("brain:database-audit")
        .disable_statement_logging()
        .options([
            ("default_transaction_read_only", "on"),
            ("search_path", "pg_catalog"),
        ]);
    let mut conn=tokio::time::timeout(Duration::from_secs(3),PgConnection::connect_with(&options)).await
        .map_err(|_|Failure::new("DB_AUDIT_CONNECT_TIMEOUT","Database audit could not connect within three seconds; no connection details were captured"))?
        .map_err(|error|Failure {code:"DB_AUDIT_CONNECTION_UNAVAILABLE",message:"Database audit could not connect; check the configured app database and current role without logging DATABASE_URL",sqlstate:state(&error)})?;
    // Session defaults are read-only before any statement. Only transaction-local guard settings are changed.
    sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL statement_timeout='2s'; SET LOCAL lock_timeout='250ms'; SET LOCAL idle_in_transaction_session_timeout='5s';").execute(&mut conn).await.map_err(failure)?;
    let scope: Value = sqlx::query_scalar(SCOPE)
        .fetch_one(&mut conn)
        .await
        .map_err(failure)?;
    if scope["transactionReadOnly"] != true {
        return Err(Failure::new(
            "DB_AUDIT_READ_ONLY_REQUIRED",
            "Database audit refused to inspect metadata outside a read-only transaction",
        ));
    }
    let settings_sql = format!(
        "SELECT coalesce(jsonb_agg(to_jsonb(s)),'[]'::jsonb) FROM (SELECT name,reset_val AS \"sourceResetValue\",setting AS \"actualValue\",unit,context,CASE WHEN name IN ('statement_timeout','lock_timeout','idle_in_transaction_session_timeout') THEN 'audit-session' ELSE source END AS source,pending_restart, name IN ('statement_timeout','lock_timeout','idle_in_transaction_session_timeout') AS \"auditSessionOverride\" FROM pg_catalog.pg_settings WHERE name IN ({SETTINGS}) ORDER BY name LIMIT 32) s"
    );
    let settings = section(&mut conn, &settings_sql, None).await?;
    let configured_sql = format!(
        "SELECT coalesce(jsonb_agg(to_jsonb(s)),'[]'::jsonb) FROM (SELECT CASE WHEN x.setdatabase=0 THEN 'role' WHEN x.setrole=0 THEN 'database' ELSE 'role-in-database' END AS scope,split_part(v,'=',1) AS name,substring(v from position('=' in v)+1) AS value FROM pg_catalog.pg_db_role_setting x CROSS JOIN LATERAL unnest(x.setconfig) v WHERE x.setdatabase IN (0,(SELECT oid FROM pg_catalog.pg_database WHERE datname=current_database())) AND x.setrole IN (0,(SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)) AND (x.setdatabase<>0 OR x.setrole<>0) AND split_part(v,'=',1) IN ({SETTINGS}) ORDER BY scope,name LIMIT 64) s"
    );
    let configured = section(&mut conn, &configured_sql, None).await?;
    let connections = section(&mut conn, CONNECTIONS, None).await?;
    let tables = section(&mut conn, TABLES, Some(limit)).await?;
    let indexes = section(&mut conn, INDEXES, Some(limit)).await?;
    let extension = section(&mut conn, "SELECT jsonb_build_object('namespace',(SELECT n.nspname::text FROM pg_catalog.pg_extension e JOIN pg_catalog.pg_namespace n ON n.oid=e.extnamespace WHERE e.extname='pg_stat_statements'))", None).await?;
    let statements = if extension["status"] != "available" {
        extension
    } else if let Some(namespace) = extension["observations"]["namespace"].as_str() {
        let sql = format!(
            "SELECT coalesce(jsonb_agg(to_jsonb(s)),'[]'::jsonb) FROM (SELECT queryid::text AS \"queryId\",calls,total_exec_time,mean_exec_time,rows,shared_blks_hit,shared_blks_read,temp_blks_read,temp_blks_written FROM {}.pg_stat_statements(false) WHERE dbid=(SELECT oid FROM pg_catalog.pg_database WHERE datname=current_database()) AND userid=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user) ORDER BY total_exec_time DESC,queryid LIMIT $1) s",
            quoted(namespace)
        );
        section(&mut conn, &sql, Some(limit)).await?
    } else {
        json!({"status":"unavailable","reason":"extension-not-installed","observations":Value::Null})
    };
    // No commit, DDL, maintenance, stats reset, SET ROLE, or persistent ALTER is performed.
    sqlx::raw_sql("ROLLBACK")
        .execute(&mut conn)
        .await
        .map_err(failure)?;
    let _ = tokio::time::timeout(Duration::from_secs(1), conn.close()).await;
    let ok = [&settings, &configured, &connections, &tables, &indexes]
        .iter()
        .all(|v| v["status"] == "available");
    let report = json!({"schemaVersion":1,"ok":ok,"readOnly":true,"applied":false,"scope":scope,"limits":{"rowsPerSection":limit,"totalSeconds":TOTAL_SECONDS,"connectSeconds":3,"statementMs":2000,"lockMs":250},
        "settings":settings,"configuredOverrides":configured,"connections":connections,"tables":tables,"indexes":indexes,"statements":statements,
        "connectionBudget":capacity(&scope,&settings),"recommendations":recommendations(&scope,&settings,&tables),
        "queryTextCaptured":false,"valuesCaptured":false,"foreignIdentitiesCaptured":false,
        "coverage":{"tables":"Current database, current-role SELECT and schema USAGE privileges; owner identities and index definitions omitted","statements":"Current database ID and current user ID only; pg_stat_statements(false) never reads query text","statistics":"Cumulative counters and row/dead-tuple estimates; limited samples do not prove missing indexes, table health or a root cause","settings":"sourceResetValue is the connection's reset default; actualValue includes explicitly marked audit-session deadline overrides","persistentSettingsChanged":false}});
    if report.to_string().len() > 512 * 1024 {
        return Err(Failure::new(
            "DB_AUDIT_OUTPUT_LIMIT",
            "Database audit exceeded its output budget; retry with a smaller --limit",
        ));
    }
    Ok(report)
}
pub async fn audit(url: &str, limit: u32, production: bool) -> Result<Value, Failure> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(Failure::new(
            "DB_AUDIT_INVALID_LIMIT",
            "Database audit limit must be 1..200",
        ));
    }
    tokio::time::timeout(Duration::from_secs(TOTAL_SECONDS),collect(url,limit,production)).await.map_err(|_|Failure::new("DB_AUDIT_DEADLINE","Database audit exceeded its twenty-second deadline; no query text or values were captured"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_quoted_and_recommendations_cannot_escape_their_role_database_scope() {
        assert_eq!(
            quoted("a\"; ALTER SYSTEM RESET ALL;--"),
            "\"a\"\"; ALTER SYSTEM RESET ALL;--\""
        );
        let scope = json!({"role":"role\"quoted","database":"db\"quoted"});
        let settings =
            json!({"observations":[{"name":"statement_timeout","sourceResetValue":"0"}]});
        let proposals = recommendations(&scope, &settings, &json!({}));
        assert_eq!(
            proposals[0]["sql"],
            "ALTER ROLE \"role\"\"quoted\" IN DATABASE \"db\"\"quoted\" SET statement_timeout = '5s';"
        );
        assert_eq!(proposals[0]["applied"], false);
        assert_eq!(proposals[0]["requiresOperatorApproval"], true);
    }
    #[test]
    fn capacity_scenario_is_not_measured_memory_or_a_total_memory_bound() {
        let settings = json!({"observations":[{"name":"work_mem","sourceResetValue":"65536","unit":"kB"},{"name":"max_connections","sourceResetValue":"200"},{"name":"superuser_reserved_connections","sourceResetValue":"3"}]});
        let report = capacity(
            &json!({"databaseConnectionLimit":-1,"roleConnectionLimit":10}),
            &settings,
        );
        assert_eq!(
            report["oneOperationPerMaxConnectionScenarioBytes"],
            "13421772800"
        );
        assert!(report["actualMemoryUsed"].is_null() && report["oomCause"].is_null());
        assert_eq!(report["nonReservedCapacityUpperBound"], 197);
    }
    #[test]
    fn query_inventory_omits_user_values_and_cross_database_role_activity() {
        assert!(CONNECTIONS.contains("usesysid=(SELECT oid"));
        assert!(CONNECTIONS.contains("datid=(SELECT oid"));
        assert!(!CONNECTIONS.contains("query,") && !CONNECTIONS.contains("client_addr"));
        assert!(TABLES.contains("has_table_privilege") && INDEXES.contains("has_table_privilege"));
        assert!(!SETTINGS.contains("sourcefile") && !SETTINGS.contains("password"));
    }
}
