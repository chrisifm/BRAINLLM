//! HTTP glue for project API keys: the request gate for `/api` resource routes and the session-only key routes.
use super::{App, Error, Result};
use crate::api_keys::{self, Admission, Denied, EventFilter, NewKey, Principal, QuotaState};
use crate::auth::User;
use axum::{
    Json, Router,
    extract::{ConnectInfo, MatchedPath, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use uuid::Uuid;

tokio::task_local! { pub(super) static KEY_USER: User; }

/// The owner a verified key acts as for the current request, if any.
pub(super) fn key_user() -> Option<User> {
    KEY_USER.try_with(Clone::clone).ok()
}
/// `/api/{name}` or `/api/{name}/{id}` → resource name. Keys are accepted nowhere else.
fn resource_of(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/")?;
    let name = rest.split('/').next()?;
    (crate::contract::identifier(name) && rest.split('/').count() <= 2).then_some(name)
}
/// True when origin enforcement may be skipped: a non-ambient credential on a resource route.
pub(super) fn keyed_resource_request(req: &Request) -> bool {
    resource_of(req.uri().path()).is_some() && api_keys::presented(req.headers()).is_some()
}
fn denied(status: StatusCode, code: &'static str, fix: &'static str) -> Response {
    let mut response = Error::new(status, code, fix).into_response();
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"api-key\""),
        );
    }
    response
}
fn quota_headers(response: &mut Response, quota: &QuotaState) {
    let headers = response.headers_mut();
    for (name, value) in [
        ("x-quota-limit", quota.limit),
        ("x-quota-remaining", quota.remaining),
        ("x-quota-reset", quota.reset_seconds),
    ] {
        if let Ok(value) = HeaderValue::from_str(&value.to_string()) {
            headers.insert(name, value);
        }
    }
}

/// Runs for every request. Without a presented key it is a no-op.
pub(super) async fn gate(State(app): State<App>, req: Request, next: Next) -> Response {
    let Some(raw) = api_keys::presented(req.headers()).map(str::to_owned) else {
        return next.run(req).await;
    };
    let Some(keys) = app.api_keys.clone() else {
        return denied(
            StatusCode::UNAUTHORIZED,
            "API_KEYS_DISABLED",
            "This project does not enable API keys; declare apiKeys in server/api-keys.jsbrain or use a session",
        );
    };
    let path = req.uri().path().to_owned();
    let Some(resource) = resource_of(&path).map(str::to_owned) else {
        return denied(
            StatusCode::UNAUTHORIZED,
            "API_KEY_NOT_ACCEPTED",
            "API keys work only on /api resource routes; auth and key management require a signed-in session",
        );
    };
    let action = api_keys::action(req.method());
    let started = std::time::Instant::now();
    let method = req.method().as_str().to_owned();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| {
            if path.trim_end_matches('/').matches('/').count() >= 3 {
                "/api/{name}/{id}".into()
            } else {
                "/api/{name}".into()
            }
        });
    let query_keys: Vec<String> = req
        .uri()
        .query()
        .unwrap_or("")
        .split('&')
        .filter_map(|pair| pair.split('=').next())
        .filter(|k| !k.is_empty() && k.len() <= 48 && k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
        .take(16)
        .map(str::to_owned)
        .collect();
    let user_agent = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua.chars().take(200).collect::<String>());
    let client_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string());
    let trace = |key_id: Uuid, status: StatusCode, bytes: Option<i64>| api_keys::Event {
        key_id,
        rid: super::request_reference(),
        method: method.clone(),
        route: route.clone(),
        path: path.chars().take(512).collect(),
        resource: Some(resource.clone()),
        action: Some(action.to_owned()),
        status: status.as_u16() as i32,
        latency_ms: started.elapsed().as_millis().min(i32::MAX as u128) as i32,
        client_ip: client_ip.clone(),
        user_agent: user_agent.clone(),
        bytes_out: bytes,
        query_keys: query_keys.clone(),
    };
    let principal: Principal = match keys.authenticate(&raw).await {
        Err(error) => return Error::internal(error).into_response(),
        Ok(Err(Denied::Invalid)) => {
            return denied(
                StatusCode::UNAUTHORIZED,
                "API_KEY_INVALID",
                "Send a valid key as X-API-Key or Authorization: Bearer bk_live_<prefix>_<secret>",
            );
        }
        Ok(Err(Denied::Revoked(id))) => {
            keys.record(trace(id, StatusCode::UNAUTHORIZED, None));
            return denied(
                StatusCode::UNAUTHORIZED,
                "API_KEY_REVOKED",
                "This key was revoked or rotated; use its replacement or create a new key",
            );
        }
        Ok(Err(Denied::Expired(id))) => {
            keys.record(trace(id, StatusCode::UNAUTHORIZED, None));
            return denied(
                StatusCode::UNAUTHORIZED,
                "API_KEY_EXPIRED",
                "This key expired; rotate it or create a new key",
            );
        }
        Ok(Ok(principal)) => principal,
    };
    if !api_keys::allows(&principal.scopes, &resource, action) {
        keys.record(trace(principal.key_id, StatusCode::FORBIDDEN, None));
        return denied(
            StatusCode::FORBIDDEN,
            "API_KEY_SCOPE_DENIED",
            "The key lacks the resource:action scope for this request; create a key with the needed scope",
        );
    }
    let quota = match keys.admit(&principal).await {
        Err(error) => return Error::internal(error).into_response(),
        Ok(Admission::RateLimited) => {
            keys.record(trace(principal.key_id, StatusCode::TOO_MANY_REQUESTS, None));
            let mut response = denied(
                StatusCode::TOO_MANY_REQUESTS,
                "API_KEY_RATE_LIMITED",
                "Slow down: this key exceeded its per-minute limit; retry after the Retry-After seconds",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            return response;
        }
        Ok(Admission::QuotaExhausted(state)) => {
            keys.record(trace(principal.key_id, StatusCode::TOO_MANY_REQUESTS, None));
            let mut response = denied(
                StatusCode::TOO_MANY_REQUESTS,
                "API_KEY_QUOTA_EXHAUSTED",
                "The key used its daily or monthly quota; wait for X-Quota-Reset seconds or raise the quota",
            );
            quota_headers(&mut response, &state);
            if let Ok(value) = HeaderValue::from_str(&state.reset_seconds.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            return response;
        }
        Ok(Admission::Allowed(quota)) => quota,
    };
    let key_id = principal.key_id;
    let mut response = KEY_USER.scope(principal.user, next.run(req)).await;
    if let Some(quota) = &quota {
        quota_headers(&mut response, quota);
    }
    let bytes = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    keys.record(trace(key_id, response.status(), bytes));
    response
}

fn keys(app: &App) -> Result<&api_keys::Keys> {
    app.api_keys.as_ref().ok_or_else(Error::missing)
}
fn invalid(message: &'static str) -> Error {
    Error::validation(json!({ "body": message }))
}
/// Owner scope for key routes: admins act on any key, everyone else only on their own.
fn scope_of(user: &User) -> Option<Uuid> {
    (user.role != "admin").then_some(user.id)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CreateKey {
    name: String,
    scopes: Vec<String>,
    expires_in_days: Option<i64>,
    daily_quota: Option<i64>,
    monthly_quota: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListKeys {
    #[serde(default)]
    all: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Summary {
    days: Option<i64>,
}
async fn create(State(app): State<App>, h: HeaderMap, Json(body): Json<CreateKey>) -> Result<Json<Value>> {
    let keys = keys(&app)?;
    let user = app.required(&h).await?;
    match keys
        .create(
            user.id,
            NewKey {
                name: body.name,
                scopes: body.scopes,
                expires_in_days: body.expires_in_days,
                daily_quota: body.daily_quota,
                monthly_quota: body.monthly_quota,
            },
        )
        .await
        .map_err(Error::internal)?
    {
        Ok((key, secret)) => Ok(Json(json!({"key":key,"secret":secret}))),
        Err(message) => Err(invalid(message)),
    }
}
async fn list(State(app): State<App>, h: HeaderMap, Query(q): Query<ListKeys>) -> Result<Json<Value>> {
    let keys = keys(&app)?;
    let user = app.required(&h).await?;
    if q.all && user.role != "admin" {
        return Err(Error::forbidden());
    }
    let owner = if q.all { None } else { Some(user.id) };
    Ok(Json(json!({"items":keys.list(owner).await.map_err(Error::internal)?})))
}
async fn revoke(State(app): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Result<Json<Value>> {
    let keys = keys(&app)?;
    let user = app.required(&h).await?;
    if !keys.revoke(id, scope_of(&user)).await.map_err(Error::internal)? {
        return Err(Error::missing());
    }
    Ok(Json(json!({"ok":true})))
}
async fn rotate(State(app): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Result<Json<Value>> {
    let keys = keys(&app)?;
    let user = app.required(&h).await?;
    let (key, secret) = keys
        .rotate(id, scope_of(&user))
        .await
        .map_err(Error::internal)?
        .ok_or_else(Error::missing)?;
    Ok(Json(json!({"key":key,"secret":secret})))
}
async fn owned(app: &App, h: &HeaderMap, id: Uuid) -> Result<()> {
    let user = app.required(h).await?;
    keys(app)?
        .find(&id.to_string(), scope_of(&user))
        .await
        .map_err(Error::internal)?
        .map(|_| ())
        .ok_or_else(Error::missing)
}
async fn events(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(filter): Query<EventFilter>,
) -> Result<Json<Value>> {
    owned(&app, &h, id).await?;
    filter.validate().map_err(invalid)?;
    Ok(Json(keys(&app)?.events(id, &filter).await.map_err(Error::internal)?))
}
async fn summary(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<Summary>,
) -> Result<Json<Value>> {
    owned(&app, &h, id).await?;
    Ok(Json(
        keys(&app)?
            .summary(id, q.days.unwrap_or(7))
            .await
            .map_err(Error::internal)?,
    ))
}
/// Session-only management routes; the gate rejects any presented key here.
pub(super) fn routes() -> Router<App> {
    Router::new()
        .route("/_brain/keys", get(list).post(create))
        .route("/_brain/keys/{id}", axum::routing::delete(revoke))
        .route("/_brain/keys/{id}/rotate", post(rotate))
        .route("/_brain/keys/{id}/events", get(events))
        .route("/_brain/keys/{id}/summary", get(summary))
}
