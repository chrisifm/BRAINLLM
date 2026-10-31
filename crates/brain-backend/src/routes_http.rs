//! Serves application-declared routes: access, rate limits, signature checks, typed query and
//! path parameters, bounded bodies, then the project handler with a deadline.
use super::{App, ClientIp, Error, Result};
use crate::routes::{Access, Handlers, Limiter, RateKey, Rejection, Route, RouteFailure};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{DefaultBodyLimit, FromRequestParts, MatchedPath, Path, Request, State as Extract},
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{MethodFilter, on},
};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) struct State {
    routes: Vec<Arc<Route>>,
    handlers: Option<Handlers>,
    limiter: Limiter,
    /// Matched path → route deadline, for the outer request deadline.
    deadlines: HashMap<String, Duration>,
}

impl State {
    pub(super) fn declared(&self) -> Vec<Route> {
        self.routes.iter().map(|r| Route::clone(r)).collect()
    }
    pub(super) fn new(routes: &[Route], handlers: Option<Handlers>, roles: &[String]) -> std::result::Result<Self, String> {
        if routes.len() > crate::routes::MAX_ROUTES {
            return Err(format!("Declare at most {} routes", crate::routes::MAX_ROUTES));
        }
        let available = handlers.as_ref().map(|h| h.names()).unwrap_or_default();
        let mut seen = Vec::new();
        let mut deadlines = HashMap::new();
        for route in routes {
            if !available.contains(&route.name) {
                return Err(format!("route `{}` has no compiled handler; add server/routes/{}/handler.jsbrain", route.name, route.name));
            }
            if let Access::Roles(list) = &route.access {
                if let Some(role) = list.iter().find(|r| !roles.contains(r)) {
                    return Err(format!("route `{}` names the unconfigured role `{role}`; inspect server/auth/roles.jsbrain", route.name));
                }
            }
            // Same shape with different parameter names is the same path for the router.
            let shape: String = route.path.split('/').map(|s| if s.starts_with('{') { "{}" } else { s }).collect::<Vec<_>>().join("/");
            if seen.contains(&(shape.clone(), route.method.clone())) || routes.iter().filter(|r| r.name == route.name).count() > 1 {
                return Err(format!("route `{}` duplicates another route's method and path", route.name));
            }
            if routes.iter().any(|r| r.path != route.path && r.path.split('/').map(|s| if s.starts_with('{') { "{}" } else { s }).collect::<Vec<_>>().join("/") == shape) {
                return Err(format!("route `{}`: routes on the same path must use the same parameter names", route.name));
            }
            seen.push((shape, route.method.clone()));
            let deadline = deadlines.entry(route.path.clone()).or_insert(route.timeout);
            *deadline = (*deadline).max(route.timeout);
        }
        Ok(Self { routes: routes.iter().cloned().map(Arc::new).collect(), handlers, limiter: Limiter::default(), deadlines })
    }
    pub(super) fn deadline(&self, matched: &str) -> Option<Duration> {
        self.deadlines.get(matched).map(|d| *d + Duration::from_secs(1))
    }
    /// Signed routes authenticate the sender by signature, not by browser Origin.
    pub(super) fn signed(&self, method: &Method, matched: &str) -> bool {
        self.routes.iter().any(|r| r.verify.is_some() && &r.method == method && r.path == matched)
    }
}

pub(super) fn routes(state: &Arc<State>) -> Router<App> {
    let mut by_path: BTreeMap<String, axum::routing::MethodRouter<App>> = BTreeMap::new();
    for route in &state.routes {
        let filter = MethodFilter::try_from(route.method.clone()).expect("contract methods are routable");
        let (r, s) = (route.clone(), state.clone());
        let handler = on(filter, move |Extract(app): Extract<App>, req: Request| {
            let (r, s) = (r.clone(), s.clone());
            async move { serve(app, s, r, req).await.unwrap_or_else(IntoResponse::into_response) }
        })
        .layer(DefaultBodyLimit::max(route.body_limit));
        let entry = by_path.remove(&route.path);
        by_path.insert(route.path.clone(), match entry {
            Some(existing) => existing.merge(handler),
            None => handler,
        });
    }
    by_path.into_iter().fold(Router::new(), |router, (path, methods)| router.route(&path, methods))
}

fn error(status: StatusCode, code: &'static str, fix: &'static str) -> Error {
    Error::new(status, code, fix)
}

fn with_retry(mut response: Response, seconds: u64) -> Response {
    response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(seconds));
    response
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// One rate-limit hit: in process memory, or in Postgres when the deployment shares state across processes.
async fn limit(app: &App, state: &State, route: &str, subject: &str, rate: &crate::routes::Rate) -> Result<crate::cluster::Hit> {
    if app.cluster.shared {
        return crate::cluster::hit(&app.pool, &format!("route:{route}"), subject, rate.count, rate.window)
            .await
            .map_err(Error::internal);
    }
    Ok(state.limiter.hit(route, subject, rate, Instant::now()))
}

async fn serve(app: App, state: Arc<State>, route: Arc<Route>, req: Request) -> Result<Response> {
    let rid = super::request_reference();
    let client = req.extensions().get::<ClientIp>().map(|c| c.0.to_string());
    let (mut parts, body) = req.into_parts();
    let rate_limited = |retry: u64| with_retry(error(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED", "Too many requests to this route; retry after the Retry-After delay").into_response(), retry);
    if let Some(rate) = route.rate.as_ref().filter(|r| r.key == RateKey::Ip) {
        if let Err(retry) = limit(&app, &state, &route.name, client.as_deref().unwrap_or("unknown"), rate).await? {
            return Ok(rate_limited(retry));
        }
    }
    let user = match &route.access {
        Access::Public => app.user(&parts.headers).await.ok().flatten(),
        Access::SignedIn => Some(app.required(&parts.headers).await?),
        Access::Roles(roles) => {
            let user = app.required(&parts.headers).await?;
            if !roles.contains(&user.role) {
                return Err(Error::forbidden());
            }
            Some(user)
        }
    };
    if let (Some(rate), Some(user)) = (route.rate.as_ref().filter(|r| r.key == RateKey::User), user.as_ref()) {
        if let Err(retry) = limit(&app, &state, &route.name, &user.id.to_string(), rate).await? {
            return Ok(rate_limited(retry));
        }
    }
    // Path parameters, typed.
    let Path(raw_params) = <Path<BTreeMap<String, String>> as FromRequestParts<App>>::from_request_parts(&mut parts, &app)
        .await
        .map_err(|_| Error::missing())?;
    let mut params = Map::new();
    for (key, kind) in &route.params {
        let text = raw_params.get(key).ok_or_else(Error::missing)?;
        params.insert(key.clone(), crate::routes::typed(*kind, text).ok_or_else(Error::missing)?);
    }
    // Query, strictly declared.
    let mut query = Map::new();
    let mut problems = Map::new();
    for (key, value) in url::form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes()) {
        match route.query.get(key.as_ref()) {
            None => {
                problems.insert(key.chars().take(64).collect(), json!("unknown query parameter"));
            }
            Some(_) if query.contains_key(key.as_ref()) => {
                problems.insert(key.to_string(), json!("repeated query parameter"));
            }
            Some(param) => match crate::routes::typed(param.kind, &value) {
                Some(v) => {
                    query.insert(key.to_string(), v);
                }
                None => {
                    problems.insert(key.to_string(), json!(format!("expected {:?}", param.kind).to_lowercase()));
                }
            },
        }
        if problems.len() >= 32 {
            break;
        }
    }
    for (key, param) in &route.query {
        if param.required && !query.contains_key(key) && !problems.contains_key(key) {
            problems.insert(key.clone(), json!("required"));
        }
    }
    if !problems.is_empty() {
        return Err(Error::validation(json!({ "query": problems })));
    }
    // Body: bounded, signed over the exact bytes, then parsed.
    let raw = to_bytes(body, route.body_limit)
        .await
        .map_err(|_| error(StatusCode::PAYLOAD_TOO_LARGE, "BODY_TOO_LARGE", "Send a smaller body; this route declares its bodyLimit"))?;
    let mut verified = Value::Null;
    if let Some(verifier) = &route.verify {
        let header = parts.headers.get(verifier.header.as_str()).and_then(|v| v.to_str().ok());
        match verifier.verify(header, &raw, unix_now()) {
            Ok(timestamp) => verified = json!({"header": verifier.header, "timestamp": timestamp}),
            Err(rejection) => {
                eprintln!("{}", json!({"schemaVersion":1,"event":"route-signature-rejected","route":route.name,"reason":format!("{rejection:?}").to_lowercase(),"rid":rid,"valuesCaptured":false}));
                return Err(match rejection {
                    Rejection::Expired => error(StatusCode::UNAUTHORIZED, "SIGNATURE_EXPIRED", "The signature timestamp is outside the accepted tolerance; resend a fresh event"),
                    _ => error(StatusCode::UNAUTHORIZED, "SIGNATURE_INVALID", "Sign the exact raw body with the shared secret in the declared header"),
                });
            }
        }
    }
    let json_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next().is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json") || t.trim().ends_with("+json")));
    let parsed = if raw.is_empty() || (route.raw_body && !json_type) {
        Value::Null
    } else {
        if !route.raw_body && !json_type {
            return Err(error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON_REQUIRED", "Send Content-Type: application/json, or declare body: 'raw'"));
        }
        let value: Value = serde_json::from_slice(&raw).map_err(|_| error(StatusCode::BAD_REQUEST, "INVALID_JSON", "Send a valid JSON body"))?;
        if !crate::before_hooks::bounded(&value) || !crate::routes::prototype_safe(&value) {
            return Err(error(StatusCode::BAD_REQUEST, "INVALID_JSON", "Send a JSON body within depth and size limits and without __proto__, constructor or prototype keys"));
        }
        value
    };
    let (raw_body, encoding) = if route.raw_body {
        match std::str::from_utf8(&raw) {
            Ok(text) => (json!(text), json!("utf8")),
            Err(_) => (json!(crate::routes::encode_base64(&raw)), json!("base64")),
        }
    } else {
        (Value::Null, Value::Null)
    };
    let headers: Map<String, Value> = route
        .headers
        .iter()
        .filter_map(|name| parts.headers.get(name.as_str()).and_then(|v| v.to_str().ok()).map(|v| (name.clone(), json!(v))))
        .collect();
    let context = json!({
        "rid": rid,
        "route": route.name,
        "method": parts.method.as_str(),
        "path": parts.uri.path(),
        "params": params,
        "query": query,
        "headers": headers,
        "user": user.as_ref().map(|u| json!({"id": u.id, "role": u.role, "verified": u.verified})),
        "client": client,
        "body": parsed,
        "rawBody": raw_body,
        "rawBodyEncoding": encoding,
        "verified": verified,
    });
    let handlers = state.handlers.as_ref().ok_or_else(|| Error::internal("route handlers missing"))?;
    let outcome = match tokio::time::timeout(route.timeout, handlers.run(&route.name, context, route.timeout)).await {
        Ok(outcome) => outcome,
        Err(_) => Err(RouteFailure::Timeout),
    };
    match outcome {
        Ok(value) => Ok(respond(value)),
        Err(RouteFailure::Status(status, body)) => {
            let status = StatusCode::from_u16(status).ok().filter(|s| s.is_client_error()).unwrap_or(StatusCode::BAD_REQUEST);
            Ok((status, Json(body)).into_response())
        }
        Err(RouteFailure::Capacity) => Ok(with_retry(error(StatusCode::SERVICE_UNAVAILABLE, "OVERLOADED", "All route workers are busy; retry after the Retry-After delay").into_response(), 1)),
        Err(RouteFailure::Timeout) => {
            eprintln!("{}", json!({"schemaVersion":1,"event":"route-timeout","route":route.name,"timeoutMs":route.timeout.as_millis() as u64,"rid":rid}));
            Err(error(StatusCode::GATEWAY_TIMEOUT, "ROUTE_TIMEOUT", "The route handler did not finish within its timeout"))
        }
        Err(RouteFailure::Unavailable) => {
            eprintln!("{}", json!({"schemaVersion":1,"event":"route-failed","code":"ROUTE_FAILED","route":route.name,"location":{"file":format!("server/routes/{}/handler.jsbrain", route.name)},"rid":rid,"valuesCaptured":false}));
            Err(error(StatusCode::INTERNAL_SERVER_ERROR, "ROUTE_FAILED", "The route handler failed; inspect server diagnostics with the request reference"))
        }
    }
}

const RESPONSE_HEADERS: &[&str] = &["location", "cache-control", "etag", "retry-after", "link", "content-language"];

/// `{status, body, headers}` from the handler; any other value is a `200` JSON body.
fn respond(value: Value) -> Response {
    let envelope = value.as_object().filter(|m| !m.is_empty() && m.keys().all(|k| matches!(k.as_str(), "status" | "body" | "headers")));
    let Some(envelope) = envelope else {
        return Json(value).into_response();
    };
    let status = envelope
        .get("status")
        .and_then(Value::as_u64)
        .and_then(|s| u16::try_from(s).ok())
        .and_then(|s| StatusCode::from_u16(s).ok())
        .filter(|s| s.is_success() || s.is_redirection() || s.is_client_error())
        .unwrap_or(StatusCode::OK);
    let mut response = if status == StatusCode::NO_CONTENT || status == StatusCode::NOT_MODIFIED {
        status.into_response()
    } else {
        (status, Json(envelope.get("body").cloned().unwrap_or(Value::Null))).into_response()
    };
    if let Some(headers) = envelope.get("headers").and_then(Value::as_object) {
        for (name, value) in headers.iter().take(16) {
            let lower = name.to_ascii_lowercase();
            if !(RESPONSE_HEADERS.contains(&lower.as_str()) || lower.starts_with("x-")) || lower == "x-request-id" {
                continue;
            }
            if let (Ok(name), Some(Ok(value))) = (HeaderName::from_bytes(lower.as_bytes()), value.as_str().filter(|v| v.len() <= 1024).map(HeaderValue::from_str)) {
                response.headers_mut().insert(name, value);
            }
        }
    }
    response
}

/// Request deadline lookup for the outer middleware.
pub(super) fn matched(req: &Request) -> Option<&str> {
    req.extensions().get::<MatchedPath>().map(MatchedPath::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handler_envelopes_map_status_body_and_safe_headers() {
        let response = respond(json!({"status": 201, "body": {"id": 7}, "headers": {"Location": "/orders/7", "Set-Cookie": "x=1", "X-Shop": "a"}}));
        assert_eq!(response.status(), 201);
        assert_eq!(response.headers()["location"], "/orders/7");
        assert_eq!(response.headers()["x-shop"], "a");
        assert!(response.headers().get("set-cookie").is_none());
        assert_eq!(respond(json!({"status": 500})).status(), 200, "handlers cannot fake server errors");
        assert_eq!(respond(json!({"status": 204})).status(), 204);
        let plain = respond(json!({"items": []}));
        assert_eq!(plain.status(), 200);
        let body = to_bytes(plain.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], br#"{"items":[]}"#);
    }
}
