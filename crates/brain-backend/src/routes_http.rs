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
    /// Signed and internal routes authenticate the sender by signature or secret, not by browser Origin.
    pub(super) fn signed(&self, method: &Method, matched: &str) -> bool {
        self.routes.iter().any(|r| (r.verify.is_some() || matches!(r.access, Access::Internal(_))) && &r.method == method && r.path == matched)
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
    // Fails closed before anything looks at the request: a sender keeps retrying and no handler runs unverified.
    if route.disabled {
        return Ok(with_retry(error(StatusCode::SERVICE_UNAVAILABLE, "ROUTE_DISABLED", "This route's secret is not configured on the server; retry after the Retry-After delay").into_response(), 300));
    }
    let rid = super::request_reference();
    let client = req.extensions().get::<ClientIp>().map(|c| c.0.to_string());
    let (mut parts, body) = req.into_parts();
    let rate_limited = |retry: u64| with_retry(error(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED", "Too many requests to this route; retry after the Retry-After delay").into_response(), retry);
    if let Some(rate) = route.rate.as_ref().filter(|r| r.key == RateKey::Ip) {
        if let Err(retry) = limit(&app, &state, &route.name, client.as_deref().unwrap_or("unknown"), rate).await? {
            return Ok(rate_limited(retry));
        }
    }
    if matches!(route.access, Access::Public) && parts.method != axum::http::Method::GET {
        app.require_captcha(&format!("route:{}", route.name), &parts.headers, parts.extensions.get::<ClientIp>().map(|c| c.0)).await?;
    }
    let user = match &route.access {
        Access::Public => app.user(&parts.headers).await.ok().flatten(),
        Access::SignedIn => Some(app.required(&parts.headers).await?),
        Access::Internal(internal) => {
            let allowed = parts.headers.get(&internal.header).and_then(|v| v.to_str().ok()).is_some_and(|v| internal.accepts(v));
            // Loopback means the connection itself and no proxy in front of it speaking for someone else.
            let local = !internal.loopback
                || (parts.extensions.get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().is_some_and(|c| c.0.ip().is_loopback())
                    && parts.extensions.get::<ClientIp>().is_none_or(|c| c.0.is_loopback())
                    && !parts.headers.contains_key("x-forwarded-for")
                    && !parts.headers.contains_key("forwarded"));
            if !allowed || !local {
                return Err(Error::forbidden());
            }
            None
        }
        Access::Roles(roles) => {
            let user = app.required(&parts.headers).await?;
            if !roles.contains(&user.role) {
                return Err(Error::forbidden());
            }
            Some(user)
        }
    };
    if let Some(seconds) = route.reauth {
        app.require_recent_auth(&parts.headers, seconds).await?;
    }
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
                crate::log_line!("{}", json!({"schemaVersion":1,"event":"route-signature-rejected","route":route.name,"reason":format!("{rejection:?}").to_lowercase(),"rid":rid,"valuesCaptured":false}));
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
    let mut context = context;
    if let Some(who) = &route.run_as {
        context["$runAs"] = json!(who);
    }
    let outcome = match tokio::time::timeout(route.timeout, handlers.run(&route.name, context, route.timeout)).await {
        Ok(outcome) => outcome,
        Err(_) => Err(RouteFailure::Timeout),
    };
    match outcome {
        Ok(value) => respond(value).ok_or_else(|| invalid_status(&route.name, &rid)),
        Err(RouteFailure::Status(status, body)) => {
            let Some(status) = failure_status(status) else {
                return Err(invalid_status(&route.name, &rid));
            };
            Ok((status, Json(body)).into_response())
        }
        Err(RouteFailure::Capacity) => Ok(with_retry(error(StatusCode::SERVICE_UNAVAILABLE, "OVERLOADED", "All route workers are busy; retry after the Retry-After delay").into_response(), 1)),
        Err(RouteFailure::Timeout) => {
            crate::db::diagnostic(&json!({"schemaVersion":1,"event":"route-timeout","route":route.name,"timeoutMs":route.timeout.as_millis() as u64,"rid":rid}));
            Err(error(StatusCode::GATEWAY_TIMEOUT, "ROUTE_TIMEOUT", "The route handler did not finish within its timeout"))
        }
        Err(failure @ (RouteFailure::Unavailable | RouteFailure::Failed(_))) => {
            let cause = match &failure {
                RouteFailure::Failed(cause) => Some(cause),
                _ => None,
            };
            crate::db::diagnostic(&route_failed(&route.name, &rid, cause));
            Err(error(StatusCode::INTERNAL_SERVER_ERROR, "ROUTE_FAILED", "The route handler failed; inspect server diagnostics with the request reference"))
        }
    }
}

/// The `route-failed` line. A thrown error adds its class, the backend client's `status`, `code` and field names, and the
/// app source line; each value is re-checked here so a sidecar can never put free text in the log.
fn route_failed(route: &str, rid: &impl serde::Serialize, cause: Option<&Value>) -> Value {
    let word = |v: &Value, max: usize| v.as_str().filter(|s| !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')).map(|s| json!(s));
    let mut location = json!({"file": format!("server/routes/{route}/handler.jsbrain")});
    let mut line = json!({"schemaVersion":1,"event":"route-failed","code":"ROUTE_FAILED","route":route,"rid":rid,"valuesCaptured":false});
    if let Some(cause) = cause {
        if let Some(name) = word(&cause["name"], 64) {
            line["error"] = name;
        }
        if let Some(backend) = cause.get("backend").filter(|b| b.is_object()) {
            let fields: Vec<Value> = backend["fields"].as_array().into_iter().flatten().filter_map(|f| f.as_str().filter(|s| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_$.".contains(&b))).map(|s| json!(s))).take(16).collect();
            line["backend"] = json!({
                "status": backend["status"].as_u64().filter(|s| (100..=599).contains(s)),
                "code": word(&backend["code"], 64),
                "fields": fields,
            });
        }
        let file = cause["location"]["file"].as_str().filter(|f| f.starts_with("server/") && f.ends_with(".jsbrain") && f.len() <= 200 && !f.contains("..") && f.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-./".contains(&b)));
        if let (Some(file), Some(at)) = (file, cause["location"]["line"].as_u64().filter(|n| (1..=9_999_999).contains(n))) {
            location = json!({"file": file, "line": at});
        }
    }
    line["location"] = location;
    line
}

const RESPONSE_HEADERS: &[&str] = &["location", "cache-control", "etag", "retry-after", "link", "content-language"];

fn failure_status(status: u16) -> Option<StatusCode> {
    StatusCode::from_u16(status).ok().filter(|s| s.is_client_error() || s.is_server_error())
}
fn invalid_status(route: &str, rid: &impl serde::Serialize) -> Error {
    crate::db::diagnostic(&json!({"schemaVersion":1,"event":"route-invalid-status","code":"ROUTE_INVALID_STATUS","route":route,"rid":rid,"valuesCaptured":false}));
    error(StatusCode::INTERNAL_SERVER_ERROR, "ROUTE_INVALID_STATUS", "The route handler returned an invalid status; return an integer from 200 to 599, excluding 1xx")
}

/// `{status, body, headers}` from the handler; any other value is a `200` JSON body. The handler's status is sent as
/// written, 5xx included, because a webhook provider reads the status to decide whether to redeliver. `None`: the
/// envelope names a status no response can have. A lone `{status: 'open'}` is ordinary data, not an envelope.
/// `{ file: { base64, type, name } }`, optionally with `status` and `headers`: an attachment built by the handler.
/// It gets the same download headers as a stored file, so handler output never renders in the site's origin.
fn file_response(file: &Value) -> Option<Response> {
    let spec = file.as_object().filter(|m| m.keys().all(|k| matches!(k.as_str(), "base64" | "type" | "name")))?;
    let bytes = crate::routes::base64(spec.get("base64")?.as_str()?)?;
    let name = spec.get("name")?.as_str().filter(|n| !n.is_empty() && n.len() <= 255)?;
    let kind = spec.get("type").and_then(Value::as_str).unwrap_or("application/octet-stream");
    let (top, sub) = kind.split_once('/')?;
    let token = |t: &str| !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b));
    if !token(top) || !token(sub) {
        return None;
    }
    let mut response = bytes.into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_str(kind).ok()?);
    response.headers_mut().insert(header::CONTENT_DISPOSITION, super::files_http::disposition("attachment", name));
    response.headers_mut().insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; sandbox"));
    Some(response)
}

fn apply_headers(response: &mut Response, headers: Option<&Value>) {
    if let Some(headers) = headers.and_then(Value::as_object) {
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
}

fn respond(value: Value) -> Option<Response> {
    let wants_file = value.as_object().is_some_and(|m| {
        m.keys().all(|k| matches!(k.as_str(), "status" | "headers" | "file")) && m.get("file").is_some_and(|f| f.get("base64").is_some())
    });
    if wants_file {
        let mut response = file_response(&value["file"])?;
        if let Some(status) = value.get("status") {
            *response.status_mut() = status.as_u64().and_then(|s| u16::try_from(s).ok()).and_then(|s| StatusCode::from_u16(s).ok()).filter(StatusCode::is_success)?;
        }
        apply_headers(&mut response, value.get("headers"));
        return Some(response);
    }
    let envelope = value.as_object().filter(|m| !m.is_empty() && m.keys().all(|k| matches!(k.as_str(), "status" | "body" | "headers")));
    let envelope = match envelope {
        Some(m) if m.len() == 1 && m.get("status").is_some_and(|s| !s.is_number()) => None,
        other => other,
    };
    let Some(envelope) = envelope else {
        return Some(Json(value).into_response());
    };
    let status = match envelope.get("status") {
        None => StatusCode::OK,
        Some(raw) => raw
            .as_u64()
            .and_then(|s| u16::try_from(s).ok())
            .and_then(|s| StatusCode::from_u16(s).ok())
            .filter(|s| !s.is_informational() && s.as_u16() <= 599)?,
    };
    let mut response = if status == StatusCode::NO_CONTENT || status == StatusCode::NOT_MODIFIED {
        status.into_response()
    } else {
        (status, Json(envelope.get("body").cloned().unwrap_or(Value::Null))).into_response()
    };
    apply_headers(&mut response, envelope.get("headers"));
    Some(response)
}

/// Request deadline lookup for the outer middleware.
pub(super) fn matched(req: &Request) -> Option<&str> {
    req.extensions().get::<MatchedPath>().map(MatchedPath::as_str)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_route_failure_logs_only_the_checked_shape_of_its_cause() {
        let plain = super::route_failed("pay", &"r1", None);
        assert_eq!(plain["location"], serde_json::json!({"file": "server/routes/pay/handler.jsbrain"}));
        assert!(plain.get("error").is_none() && plain.get("backend").is_none());
        let cause = serde_json::json!({"name": "Error", "backend": {"status": 503, "code": "BACKEND_UNAVAILABLE", "fields": ["card", "card number 4111"]}, "location": {"file": "server/lib/pay.jsbrain", "line": 12}});
        let line = super::route_failed("pay", &"r1", Some(&cause));
        assert_eq!(line["error"], "Error");
        assert_eq!(line["backend"], serde_json::json!({"status": 503, "code": "BACKEND_UNAVAILABLE", "fields": ["card"]}));
        assert_eq!(line["location"], serde_json::json!({"file": "server/lib/pay.jsbrain", "line": 12}));
        let hostile = serde_json::json!({"name": "secret value", "backend": {"status": 9000, "code": "has space"}, "location": {"file": "/etc/passwd", "line": 1}});
        let line = super::route_failed("pay", &"r1", Some(&hostile));
        assert!(!line.to_string().contains("secret") && !line.to_string().contains("passwd") && !line.to_string().contains("space"), "{line}");
        assert_eq!(line["backend"], serde_json::json!({"status": null, "code": null, "fields": []}));
    }

    use super::*;

    #[tokio::test]
    async fn a_handler_returns_a_file_only_as_a_sandboxed_attachment() {
        let response = respond(json!({"file": {"base64": "aGk=", "type": "text/html", "name": "report €.html"}, "headers": {"content-disposition": "inline", "x-shop": "a"}})).unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "text/html");
        assert!(response.headers()["content-disposition"].to_str().unwrap().starts_with("attachment;"), "the handler cannot make it inline");
        assert_eq!(response.headers()["x-shop"], "a");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert!(response.headers()["content-security-policy"].to_str().unwrap().contains("default-src 'none'"));
        assert_eq!(axum::body::to_bytes(response.into_body(), 64).await.unwrap(), "hi".as_bytes());
        for bad in [json!({"file": {"base64": "***", "name": "a"}}), json!({"file": {"base64": "aGk="}}), json!({"file": {"base64": "aGk=", "name": "a", "type": "text/html\r\nx: y"}}), json!({"status": 404, "file": {"base64": "aGk=", "name": "a"}})] {
            assert!(respond(bad.clone()).is_none(), "{bad}");
        }
        let plain = respond(json!({"file": "a.pdf", "status": 200})).unwrap();
        assert_eq!(axum::body::to_bytes(plain.into_body(), 64).await.unwrap(), r#"{"file":"a.pdf","status":200}"#.as_bytes(), "data with a `file` key stays data");
    }

    #[tokio::test]
    async fn handler_envelopes_map_status_body_and_safe_headers() {
        let response = respond(json!({"status": 201, "body": {"id": 7}, "headers": {"Location": "/orders/7", "Set-Cookie": "x=1", "X-Shop": "a"}})).unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(response.headers()["location"], "/orders/7");
        assert_eq!(response.headers()["x-shop"], "a");
        assert!(response.headers().get("set-cookie").is_none());
        // A handler's 5xx reaches the client: a webhook provider must see the failure and redeliver.
        for status in [500, 502, 503, 504, 599] {
            assert_eq!(respond(json!({"status": status, "body": {"retry": true}})).unwrap().status(), status);
        }
        for invalid in [json!(0), json!(99), json!(101), json!(600), json!(70000), json!(1.5), json!(-1), json!("500")] {
            assert!(respond(json!({"status": invalid, "body": null})).is_none(), "{invalid}");
        }
        let data = respond(json!({"status": "open"})).unwrap();
        assert_eq!(data.status(), 200);
        assert_eq!(&to_bytes(data.into_body(), 64).await.unwrap()[..], br#"{"status":"open"}"#);
        assert_eq!(respond(json!({"status": 204})).unwrap().status(), 204);
        assert_eq!(failure_status(503), Some(StatusCode::SERVICE_UNAVAILABLE));
        assert_eq!(failure_status(200), None);
        let plain = respond(json!({"items": []})).unwrap();
        assert_eq!(plain.status(), 200);
        let body = to_bytes(plain.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], br#"{"items":[]}"#);
    }
}
