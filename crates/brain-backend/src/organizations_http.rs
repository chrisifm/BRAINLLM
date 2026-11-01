//! Organization and membership endpoints. Child of `http`. A non-member gets the same 404 as for an organization that
//! does not exist; there is no lookup by email, so these routes cannot be used to discover accounts.
use super::{App, Error, Path, Result};
use crate::{auth, http::User};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

pub(super) fn routes() -> Router<App> {
    Router::new()
        .route("/_brain/orgs", get(mine).post(create))
        .route("/_brain/orgs/invites/accept", post(accept))
        .route("/_brain/orgs/{id}/members", get(members))
        .route("/_brain/orgs/{id}/members/{user}", patch(change_role).delete(remove))
        .route("/_brain/orgs/{id}/invites", post(invite))
        .route("/_brain/orgs/{id}/audit", get(org_audit))
        .route("/_brain/audit", get(global_audit))
}
fn db_error(error: sqlx::Error) -> Error {
    Error::database(error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewOrg {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleBody {
    role: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptBody {
    token: String,
}
async fn role_of(app: &App, org: Uuid, user: Uuid) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT role FROM _brain_org_members WHERE org_id=$1 AND user_id=$2").bind(org).bind(user).fetch_optional(&app.pool).await.map_err(db_error)
}
/// The caller's role here; everyone else sees a missing organization.
async fn member(app: &App, org: Uuid, u: &User) -> Result<String> {
    role_of(app, org, u.id).await?.ok_or_else(Error::missing)
}
/// A manager's role; a plain member learns it exists (they already do) and is refused.
async fn manager(app: &App, org: Uuid, u: &User) -> Result<String> {
    let role = member(app, org, u).await?;
    if !app.organizations.manages(&role) {
        return Err(Error::forbidden());
    }
    Ok(role)
}
fn known_role(app: &App, role: &str) -> Result<()> {
    if app.organizations.rank(role).is_none() {
        return Err(Error::validation(json!({"role": format!("Use one of: {}", app.organizations.roles.join(", "))})));
    }
    Ok(())
}
fn outranked() -> Error {
    Error::new(StatusCode::FORBIDDEN, "ORG_ROLE_DENIED", "You can only grant or change membership roles at or below your own")
}
fn last_founder() -> Error {
    Error::new(StatusCode::CONFLICT, "ORG_LAST_OWNER", "An organization keeps at least one member with its top role; give that role to someone else first")
}
async fn create(State(app): State<App>, h: HeaderMap, Json(input): Json<NewOrg>) -> Result<Json<Value>> {
    let u = app.required(&h).await?;
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(Error::validation(json!({"name": "Use 1 to 200 characters"})));
    }
    let id = Uuid::new_v4();
    let role = app.organizations.founder().to_string();
    let mut tx = app.pool.begin().await.map_err(db_error)?;
    sqlx::query("INSERT INTO _brain_orgs(id,name,created_by) VALUES($1,$2,$3)").bind(id).bind(name).bind(u.id).execute(&mut *tx).await.map_err(db_error)?;
    sqlx::query("INSERT INTO _brain_org_members(org_id,user_id,role) VALUES($1,$2,$3)").bind(id).bind(u.id).bind(&role).execute(&mut *tx).await.map_err(db_error)?;
    crate::audit::organization_event(&mut tx, u.id, id, "org_created", Some(id), &[]).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"id": id, "name": name, "role": role})))
}
async fn mine(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>> {
    let u = app.required(&h).await?;
    let rows = sqlx::query("SELECT o.id,o.name,m.role FROM _brain_org_members m JOIN _brain_orgs o ON o.id=m.org_id WHERE m.user_id=$1 ORDER BY o.created_at,o.id LIMIT 200").bind(u.id).fetch_all(&app.pool).await.map_err(db_error)?;
    let items: Vec<Value> = rows.iter().map(|r| json!({"id": r.get::<Uuid, _>("id"), "name": r.get::<String, _>("name"), "role": r.get::<String, _>("role")})).collect();
    Ok(Json(json!({"items": items})))
}
async fn members(State(app): State<App>, Path(org): Path<Uuid>, h: HeaderMap) -> Result<Json<Value>> {
    let u = app.required(&h).await?;
    member(&app, org, &u).await?;
    let rows = sqlx::query("SELECT user_id,role FROM _brain_org_members WHERE org_id=$1 ORDER BY created_at,user_id LIMIT 1000").bind(org).fetch_all(&app.pool).await.map_err(db_error)?;
    let items: Vec<Value> = rows.iter().map(|r| json!({"user": r.get::<Uuid, _>("user_id"), "role": r.get::<String, _>("role")})).collect();
    Ok(Json(json!({"items": items})))
}
/// Locks the organization so concurrent changes cannot both remove its last top-role member.
async fn change(app: &App, org: Uuid, target: Uuid, next: Option<&str>, actor: &User, actor_role: &str) -> Result<()> {
    let founder = app.organizations.founder();
    let mut tx = app.pool.begin().await.map_err(db_error)?;
    let _: Uuid = sqlx::query_scalar("SELECT id FROM _brain_orgs WHERE id=$1 FOR UPDATE").bind(org).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or_else(Error::missing)?;
    let current: String = sqlx::query_scalar("SELECT role FROM _brain_org_members WHERE org_id=$1 AND user_id=$2").bind(org).bind(target).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or_else(Error::missing)?;
    // Re-read under the lock: a demotion that committed first must not leave this actor acting on the old rank.
    let live: Option<String> = sqlx::query_scalar("SELECT role FROM _brain_org_members WHERE org_id=$1 AND user_id=$2").bind(org).bind(actor.id).fetch_optional(&mut *tx).await.map_err(db_error)?;
    let live = live.ok_or_else(Error::missing)?;
    let leaving_self = next.is_none() && target == actor.id;
    if !leaving_self {
        if live != actor_role && !app.organizations.manages(&live) {
            return Err(Error::forbidden());
        }
        if !app.organizations.outranks_or_equals(&live, &current) || next.is_some_and(|role| !app.organizations.outranks_or_equals(&live, role)) {
            return Err(outranked());
        }
    }
    if current == founder && next != Some(founder) {
        let others: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_org_members WHERE org_id=$1 AND role=$2 AND user_id<>$3").bind(org).bind(founder).bind(target).fetch_one(&mut *tx).await.map_err(db_error)?;
        if others == 0 {
            return Err(last_founder());
        }
    }
    match next {
        Some(role) => sqlx::query("UPDATE _brain_org_members SET role=$3 WHERE org_id=$1 AND user_id=$2").bind(org).bind(target).bind(role).execute(&mut *tx).await.map_err(db_error)?,
        None => sqlx::query("DELETE FROM _brain_org_members WHERE org_id=$1 AND user_id=$2").bind(org).bind(target).execute(&mut *tx).await.map_err(db_error)?,
    };
    let event = if next.is_some() { "member_role_changed" } else { "member_removed" };
    crate::audit::organization_event(&mut tx, actor.id, org, event, Some(target), &["role"]).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)
}
async fn change_role(State(app): State<App>, Path((org, target)): Path<(Uuid, Uuid)>, h: HeaderMap, Json(input): Json<RoleBody>) -> Result<Json<Value>> {
    let u = app.required(&h).await?;
    let actor = manager(&app, org, &u).await?;
    known_role(&app, &input.role)?;
    change(&app, org, target, Some(&input.role), &u, &actor).await?;
    Ok(Json(json!({"user": target, "role": input.role})))
}
async fn remove(State(app): State<App>, Path((org, target)): Path<(Uuid, Uuid)>, h: HeaderMap) -> Result<Json<Value>> {
    let u = app.required(&h).await?;
    let actor = if target == u.id { member(&app, org, &u).await? } else { manager(&app, org, &u).await? };
    change(&app, org, target, None, &u, &actor).await?;
    Ok(Json(json!({"ok": true})))
}
async fn invite(State(app): State<App>, Path(org): Path<Uuid>, h: HeaderMap, Json(input): Json<RoleBody>) -> Result<Json<Value>> {
    let u = app.required(&h).await?;
    let actor = manager(&app, org, &u).await?;
    known_role(&app, &input.role)?;
    if !app.organizations.outranks_or_equals(&actor, &input.role) {
        return Err(outranked());
    }
    let token = auth::token();
    let mut tx = app.pool.begin().await.map_err(db_error)?;
    crate::audit::organization_event(&mut tx, u.id, org, "invite_created", None, &["role"]).await.map_err(db_error)?;
    let expires: String = sqlx::query_scalar("INSERT INTO _brain_org_invites(token_hash,org_id,role,created_by,expires_at) VALUES($1,$2,$3,$4,now()+make_interval(days=>$5)) RETURNING to_jsonb(expires_at)#>>'{}'")
        .bind(auth::digest(&token)).bind(org).bind(&input.role).bind(u.id).bind(crate::organizations::INVITE_DAYS)
        .fetch_one(&mut *tx).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"token": token, "role": input.role, "expires_at": expires})))
}
async fn accept(State(app): State<App>, h: HeaderMap, Json(input): Json<AcceptBody>) -> Result<Json<Value>> {
    let u = app.required(&h).await?;
    let invalid = || Error::new(StatusCode::BAD_REQUEST, "ORG_INVITE_INVALID", "Ask a manager of the organization for a new invitation; this one is unknown, used or expired");
    let mut tx = app.pool.begin().await.map_err(db_error)?;
    let row = sqlx::query("UPDATE _brain_org_invites SET accepted_at=now(),accepted_by=$2 WHERE token_hash=$1 AND accepted_at IS NULL AND expires_at>now() RETURNING org_id,role")
        .bind(auth::digest(&input.token)).bind(u.id).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or_else(invalid)?;
    let (org, role): (Uuid, String) = (row.get("org_id"), row.get("role"));
    // An existing member keeps their role: an invitation never changes it, up or down.
    sqlx::query("INSERT INTO _brain_org_members(org_id,user_id,role) VALUES($1,$2,$3) ON CONFLICT (org_id,user_id) DO NOTHING").bind(org).bind(u.id).bind(&role).execute(&mut *tx).await.map_err(db_error)?;
    crate::audit::organization_event(&mut tx, u.id, org, "member_added", Some(u.id), &["role"]).await.map_err(db_error)?;
    let held: String = sqlx::query_scalar("SELECT role FROM _brain_org_members WHERE org_id=$1 AND user_id=$2").bind(org).bind(u.id).fetch_one(&mut *tx).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"id": org, "role": held})))
}
struct AuditQuery {
    resource: Option<String>,
    before: Option<i64>,
    limit: i64,
}
fn audit_query(pairs: &[(String, String)]) -> Result<AuditQuery> {
    let mut out = AuditQuery { resource: None, before: None, limit: 50 };
    for (key, value) in pairs {
        let bad = || Error::validation(json!({key.as_str(): "Use resource, before (an entry id) or limit from 1 to 100"}));
        match key.as_str() {
            "resource" if value.len() <= 64 => out.resource = Some(value.clone()),
            "before" => out.before = Some(value.parse().map_err(|_| bad())?),
            "limit" => {
                out.limit = value.parse().map_err(|_| bad())?;
                if !(1..=crate::audit::MAX_PAGE).contains(&out.limit) {
                    return Err(bad());
                }
            }
            _ => return Err(bad()),
        }
    }
    Ok(out)
}
/// Managers read their own organization's log; everyone else gets the answer for a missing organization or a denial.
async fn org_audit(State(app): State<App>, Path(org): Path<Uuid>, h: HeaderMap, Query(pairs): Query<Vec<(String, String)>>) -> Result<Json<Value>> {
    let u = app.required(&h).await?;
    manager(&app, org, &u).await?;
    let q = audit_query(&pairs)?;
    crate::audit::page(&app.pool, Some(org), q.resource.as_deref(), q.before, q.limit).await.map(Json).map_err(db_error)
}
/// Administrators read entries outside any organization; tenant entries stay with their organization.
async fn global_audit(State(app): State<App>, h: HeaderMap, Query(pairs): Query<Vec<(String, String)>>) -> Result<Json<Value>> {
    app.admin(&h).await?;
    let q = audit_query(&pairs)?;
    crate::audit::page(&app.pool, None, q.resource.as_deref(), q.before, q.limit).await.map(Json).map_err(db_error)
}
