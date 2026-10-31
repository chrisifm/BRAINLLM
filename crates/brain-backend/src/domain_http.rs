//! HTTP entry points for declared domain writes. Child of `http`: the fixed lifecycle (limits, origin, auth,
//! rate limit, deadline) applies unchanged; every request runs in one Postgres transaction.
use super::{App, Error, Result, write_error};
use crate::domain::{BatchRequest, Change, Domain, Engine, Failure, UpsertRequest};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
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
        if role != "owner" && !roles.roles.contains(role) {
            return Err(format!("Domain contract names role `{role}` that is not declared in server/auth/roles.jsbrain"));
        }
    }
    Ok(())
}

fn failure(app: &App, error: Failure) -> Error {
    match error {
        Failure::Forbidden => Error::forbidden(),
        Failure::Missing(label) => {
            let mut e = Error::missing();
            e.fields = Some(json!({ label: "Not found" }));
            e
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
            None => Error::internal(crate::db::safe(error)),
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
        let mut $tx = $app.pool.begin().await.map_err(|e| Error::internal(crate::db::safe(e)))?;
        let mut $engine = Engine::new(&$app.domain, &$app.resources, &$user);
        let outcome = $body;
        match outcome {
            Ok(value) => {
                $tx.commit().await.map_err(|e| Error::internal(crate::db::safe(e)))?;
                let changes = std::mem::take(&mut $engine.changes);
                notify(&$app, changes);
                Ok(value)
            }
            Err(error) => {
                drop($tx);
                Err(failure(&$app, error))
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
    app.resource(&name)?;
    let user = app.required(&h).await?;
    transactional!(app, user, |engine, tx| engine.action(&mut tx, &name, &action, id, &input).await).map(Json)
}
async fn transition(
    State(app): State<App>,
    Path((name, id, transition)): Path<(String, Uuid, String)>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    app.resource(&name)?;
    let user = app.required(&h).await?;
    transactional!(app, user, |engine, tx| engine.transition(&mut tx, &name, &transition, id, &input).await).map(Json)
}
async fn batch(
    State(app): State<App>,
    Path(name): Path<String>,
    h: HeaderMap,
    Json(request): Json<BatchRequest>,
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
    transactional!(app, user, |engine, tx| engine.batch(&mut tx, r, &request).await).map(Json)
}
async fn upsert(
    State(app): State<App>,
    Path(name): Path<String>,
    h: HeaderMap,
    Json(request): Json<UpsertRequest>,
) -> Result<Json<Value>> {
    let resources = app.resources.clone();
    let r = resources.get(&name).ok_or_else(Error::missing)?;
    let user = app.required(&h).await?;
    hooked(r, &["create", "update"])?;
    transactional!(app, user, |engine, tx| engine
        .upsert_request(&mut tx, r, &request)
        .await
        .map(|(record, created)| json!({"record": record, "created": created})))
    .map(Json)
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
    transactional!(app, user, |engine, tx| engine.operation(&mut tx, &name, &input).await).map(Json)
}
