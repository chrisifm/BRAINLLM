//! HTTP entry points for declared domain writes. Child of `http`: the fixed lifecycle (limits, origin, auth,
//! rate limit, deadline) applies unchanged; every request runs in one Postgres transaction.
use super::{App, Error, Result, Strict, reveal, write_error};
use crate::domain::{BatchRequest, Change, Domain, Engine, Failure, UpsertRequest};
use super::Path;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Batch bodies are larger than single-record bodies; the per-resource item limit still applies.
const BATCH_BODY_BYTES: usize = 1024 * 1024;

pub(super) fn routes() -> Router<App> {
    Router::new()
        .route("/api/{name}/batch", post(batch).layer(DefaultBodyLimit::max(BATCH_BODY_BYTES)))
        .route("/api/{name}/upsert", post(upsert))
        .route("/api/{name}/{id}/actions/{action}", post(action))
        .route("/api/{name}/{id}/transitions/{transition}", post(transition))
        .route("/_brain/operations/{name}", post(operation))
}

pub(super) fn validate(
    domain: &Domain,
    resources: &BTreeMap<String, crate::Resource>,
    roles: &crate::auth_flows::RolesConfig,
) -> std::result::Result<(), String> {
    domain.validate(&resources.values().cloned().collect::<Vec<_>>())?;
    let grants = domain
        .resources
        .values()
        .flat_map(|d| {
            d.actions.values().flat_map(|a| a.roles.iter()).chain(
                d.transitions.iter().flat_map(|t| t.transitions.values().flat_map(|x| x.roles.iter())),
            )
        })
        .chain(domain.operations.values().flat_map(|o| o.roles.iter()));
    for role in grants {
        if role != "owner" && crate::ownership::path(role).is_none() && !roles.roles.contains(role) {
            return Err(format!("Domain contract names role `{role}` that is not declared in server/auth/roles.jsbrain"));
        }
    }
    Ok(())
}

async fn failure(app: &App, user: &crate::auth::User, error: Failure) -> Error {
    match error {
        Failure::Forbidden => Error::forbidden(),
        Failure::Missing(label) => {
            let mut e = Error::missing();
            e.fields = Some(json!({ label: "Not found" }));
            e
        }
        Failure::Denied { resource, id, label } => {
            let mut missing = Error::missing();
            missing.fields = Some(json!({ label: "Not found" }));
            let Some(r) = app.resources.get(&resource) else { return missing };
            super::write_denied(&app.pool, &app.resources, r, user, id, missing).await
        }
        Failure::Invalid(fields) => Error::validation(fields),
        Failure::Guard(fields) => {
            let mut e = Error::new(
                StatusCode::CONFLICT,
                "GUARD_FAILED",
                "The current record does not satisfy the declared conditions; reload it and decide again",
            );
            e.fields = Some(json!(fields));
            e
        }
        Failure::Database { resource, label, error } => match app.resources.get(&resource) {
            Some(r) => {
                let mut e = write_error(r, error);
                if let (Some(label), Some(Value::Object(fields))) = (label, e.fields.take()) {
                    e.fields = Some(Value::Object(fields.into_iter().map(|(k, v)| (format!("{label}.{k}"), v)).collect()));
                }
                e
            }
            None => Error::database(error),
        },
    }
}
fn notify(app: &App, changes: Vec<Change>) {
    for change in changes {
        app.sockets.changed(&change.resource, change.operation, &change.record);
    }
}
/// Resources with a mandatory before-hook keep their single-record endpoints so the hook cannot be bypassed.
fn hooked(r: &crate::Resource, ops: &[&str]) -> Result<()> {
    if ops.iter().any(|op| r.hooks.before(op).is_some()) {
        return Err(Error::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "BEFORE_HOOK_REQUIRES_SINGLE_WRITES",
            "This resource declares a mandatory before-hook; use the single-record create/update/delete endpoints",
        ));
    }
    Ok(())
}

/// Run `body` against one transaction; commit only when it succeeds, then notify live subscribers.
macro_rules! transactional {
    ($app:expr, $user:expr, |$engine:ident, $tx:ident| $body:expr) => {{
        let mut $tx = $app.pool.begin().await.map_err(|e| Error::database(e))?;
        let mut $engine = Engine::new(&$app.domain, &$app.resources, &$user);
        let outcome = $body;
        match outcome {
            Ok(value) => {
                $tx.commit().await.map_err(|e| Error::database(e))?;
                let changes = std::mem::take(&mut $engine.changes);
                notify(&$app, changes);
                Ok(value)
            }
            Err(error) => {
                drop($tx);
                Err(failure(&$app, &$user, error).await)
            }
        }
    }};
}

async fn action(
    State(app): State<App>,
    Path((name, id, action)): Path<(String, Uuid, String)>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    let user = app.required(&h).await?;
    let mut row = transactional!(app, user, |engine, tx| engine.action(&mut tx, &name, &action, id, &input).await)?;
    reveal(&app, r, &mut row, &user, false).await?;
    Ok(Json(row))
}
async fn transition(
    State(app): State<App>,
    Path((name, id, transition)): Path<(String, Uuid, String)>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    let user = app.required(&h).await?;
    let mut row = transactional!(app, user, |engine, tx| engine.transition(&mut tx, &name, &transition, id, &input).await)?;
    reveal(&app, r, &mut row, &user, false).await?;
    Ok(Json(row))
}
async fn batch(
    State(app): State<App>,
    Path(name): Path<String>,
    h: HeaderMap,
    Strict(request): Strict<BatchRequest>,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?.clone();
    let user = app.required(&h).await?;
    hooked(&r, match request.op.as_str() {
        "upsert" => &["create", "update"],
        "create" => &["create"],
        "update" => &["update"],
        _ => &["delete"],
    })?;
    let resources = app.resources.clone();
    let r = resources.get(&name).expect("resource exists");
    let mut out: Value = transactional!(app, user, |engine, tx| engine.batch(&mut tx, r, &request).await).map_err(|mut error: Error| {
        if error.code == "DATABASE_BUSY" && crate::platform::database_limit() == Some(crate::platform::DatabaseLimit::Deadline) {
            error.code = "BATCH_TOO_SLOW";
            error.fix = "Nothing was written: this batch did not finish within the database time limit, and the same batch will need that time again. Split it into smaller batches and send them one at a time".into();
        }
        error
    })?;
    if let Some(items) = out["items"].as_array_mut() {
        for row in items {
            reveal(&app, r, row, &user, false).await?;
        }
    }
    Ok(Json(out))
}
async fn upsert(
    State(app): State<App>,
    Path(name): Path<String>,
    h: HeaderMap,
    Strict(request): Strict<UpsertRequest>,
) -> Result<Json<Value>> {
    app.resource(&name)?;
    let resources = app.resources.clone();
    let r = resources.get(&name).ok_or_else(Error::missing)?;
    let user = app.required(&h).await?;
    hooked(r, &["create", "update"])?;
    let mut out: Value = transactional!(app, user, |engine, tx| engine
        .upsert_request(&mut tx, r, &request)
        .await
        .map(|(record, created)| json!({"record": record, "created": created})))?;
    reveal(&app, r, &mut out["record"], &user, false).await?;
    Ok(Json(out))
}
async fn operation(
    State(app): State<App>,
    Path(name): Path<String>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    if !app.domain.operations.contains_key(&name) {
        return Err(Error::missing());
    }
    let user = app.required(&h).await?;
    let mut out: Value = transactional!(app, user, |engine, tx| engine.operation(&mut tx, &name, &input).await)?;
    // Each step result is a row of its target resource and passes the same response rules as a single write.
    for step in &app.domain.operations[&name].steps {
        let target = step.kind().ok().map(|(_, t)| t.split('.').next().unwrap_or(t).to_string());
        if let (Some(r), Some(row)) = (target.and_then(|t| app.resources.get(&t)), out["result"].get_mut(&step.name)) {
            reveal(&app, r, row, &user, false).await?;
        }
    }
    Ok(Json(out))
}
