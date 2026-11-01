//! Audit log: append-only record of mutations on resources that declare `audit: true`, plus every organization and
//! membership change. A database trigger writes it, so no write path can skip it; it stores field names and never
//! values. The actor is whoever `App::user` resolved for this request, handed to the trigger through settings local to
//! the writing transaction. A write outside a request is recorded with `actor_kind: server`.
use crate::Resource;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub const MAX_PAGE: i64 = 100;

tokio::task_local! { static ACTOR: Arc<Mutex<Option<Uuid>>>; }

/// Runs one request with an empty actor cell that authentication fills in.
pub async fn scope<F: std::future::Future>(body: F) -> F::Output {
    ACTOR.scope(Arc::new(Mutex::new(None)), body).await
}
/// Records the identity this request authenticated as.
pub fn identified(user: Uuid) {
    let _ = ACTOR.try_with(|cell| {
        if let Ok(mut held) = cell.lock() {
            *held = Some(user);
        }
    });
}
fn actor() -> Option<Uuid> {
    ACTOR.try_with(|cell| cell.lock().ok().and_then(|held| *held)).ok().flatten()
}
fn kind(actor: Option<Uuid>) -> &'static str {
    if crate::api_keys::system_active() {
        "system_key"
    } else if actor.is_some() {
        "user"
    } else {
        "server"
    }
}
/// Hands the request identity to the triggers of this transaction. Settings end with it.
pub async fn mark(tx: &mut sqlx::PgConnection) -> Result<(), sqlx::Error> {
    let who = actor();
    sqlx::query("SELECT set_config('brain.actor',$1,true),set_config('brain.actor_kind',$2,true),set_config('brain.request',$3,true)")
        .bind(who.map(|id| id.to_string()).unwrap_or_default())
        .bind(kind(who))
        .bind(crate::http::request_reference().unwrap_or_default())
        .execute(tx)
        .await
        .map(|_| ())
}
/// One organization or membership event, inside the transaction that made the change.
pub async fn organization_event(tx: &mut sqlx::PgConnection, actor: Uuid, org: Uuid, operation: &str, subject: Option<Uuid>, fields: &[&str]) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO _brain_audit_log(actor,actor_kind,org,resource,record,operation,fields,request) VALUES($1,'user',$2,'organization',$3,$4,$5,$6)")
        .bind(actor)
        .bind(org)
        .bind(subject)
        .bind(operation)
        .bind(fields.iter().map(|f| f.to_string()).collect::<Vec<_>>())
        .bind(crate::http::request_reference())
        .execute(tx)
        .await
        .map(|_| ())
}
fn trigger_arg(r: &Resource) -> &str {
    crate::organizations::tenant(r).unwrap_or("")
}
pub(crate) async fn install(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, r: &Resource) -> Result<(), String> {
    let table = crate::relations::table(&r.name);
    let create = if r.policy.audit {
        format!(" CREATE TRIGGER _brain_audit AFTER INSERT OR UPDATE OR DELETE ON {table} FOR EACH ROW EXECUTE FUNCTION public._brain_audit_row('{}');", trigger_arg(r))
    } else {
        String::new()
    };
    // Audited SQL: table and tenant field are validated contract identifiers.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TRIGGER IF EXISTS _brain_audit ON {table};{create}"))).execute(&mut **tx).await.map_err(crate::db::safe)?;
    Ok(())
}
/// Serving refuses an audited resource whose trigger is missing or disabled.
pub(crate) async fn verify(conn: &mut sqlx::PgConnection, r: &Resource) -> Result<(), String> {
    if !r.policy.audit {
        return Ok(());
    }
    let table = crate::relations::table(&r.name);
    let args: Vec<u8> = trigger_arg(r).bytes().chain(std::iter::once(0)).collect();
    let present: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_trigger WHERE tgrelid=to_regclass($1) AND tgname='_brain_audit' AND tgenabled='O' AND tgfoid='public._brain_audit_row()'::regprocedure AND tgargs=$2)")
        .bind(format!("public.{table}"))
        .bind(args)
        .fetch_one(&mut *conn)
        .await
        .map_err(crate::db::safe)?;
    if !present {
        return Err(format!("BDB_SCHEMA_DRIFT: {table} audit trigger; review schema and run the explicit backend migration; no rows were removed"));
    }
    Ok(())
}
/// One newest-first page. `org` None reads entries outside any organization: a global role has no reach into tenants.
pub async fn page(pool: &PgPool, org: Option<Uuid>, resource: Option<&str>, before: Option<i64>, limit: i64) -> Result<Value, sqlx::Error> {
    let limit = limit.clamp(1, MAX_PAGE);
    let rows = sqlx::query("SELECT id,to_jsonb(at)#>>'{}' AS at,actor,actor_kind,org,resource,record,operation,fields,request FROM _brain_audit_log WHERE org IS NOT DISTINCT FROM $1 AND ($2::text IS NULL OR resource=$2) AND ($3::bigint IS NULL OR id<$3) ORDER BY id DESC LIMIT $4")
        .bind(org)
        .bind(resource)
        .bind(before)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?;
    let more = rows.len() as i64 > limit;
    let items: Vec<Value> = rows.iter().take(limit as usize).map(|row| json!({
        "id": row.get::<i64, _>("id"),
        "at": row.get::<String, _>("at"),
        "actor": row.get::<Option<Uuid>, _>("actor"),
        "actor_kind": row.get::<String, _>("actor_kind"),
        "org": row.get::<Option<Uuid>, _>("org"),
        "resource": row.get::<String, _>("resource"),
        "record": row.get::<Option<Uuid>, _>("record"),
        "operation": row.get::<String, _>("operation"),
        "fields": row.get::<Vec<String>, _>("fields"),
        "request": row.get::<Option<String>, _>("request"),
    })).collect();
    let next = if more { items.last().map(|i| i["id"].clone()) } else { None };
    Ok(json!({"items": items, "next": next}))
}
