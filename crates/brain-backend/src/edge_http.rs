//! HTTP edge: real client IP behind trusted proxies, in-flight backpressure, pool-saturation
//! mapping and the split liveness/readiness probes.
use super::{App, Error};
use crate::platform::{Edge, RequestLog};
use axum::{
    Extension, Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use ipnet::IpNet;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower_governor::{GovernorError, key_extractor::KeyExtractor};

/// The resolved client address, inserted into every request's extensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

pub(super) struct Lifecycle {
    draining: AtomicBool,
    in_flight: AtomicUsize,
    max_in_flight: usize,
}

/// Lets the owner of the listener mark the process as draining.
#[derive(Clone)]
pub struct EdgeControl(Arc<Lifecycle>);
impl EdgeControl {
    /// Readiness answers `503 DRAINING` from now on; liveness keeps answering.
    pub fn drain(&self) {
        self.0.draining.store(true, Ordering::SeqCst);
    }
    pub fn draining(&self) -> bool {
        self.0.draining.load(Ordering::SeqCst)
    }
    /// Requests currently inside the application (probes excluded).
    pub fn in_flight(&self) -> usize {
        self.0.in_flight.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub(super) struct EdgeState {
    trusted: Arc<Vec<IpNet>>,
    lifecycle: Arc<Lifecycle>,
    log: LogPolicy,
}

/// Request logging policy, attached to each response for the outer request-id layer.
#[derive(Clone, Copy, Debug)]
pub(super) struct LogPolicy {
    mode: RequestLog,
    slow: Option<Duration>,
    probe: bool,
}

/// Which line, if any, a finished request produces. Probes never log on success.
pub(super) fn request_event(policy: Option<&LogPolicy>, status: StatusCode, elapsed: Duration) -> Option<&'static str> {
    let failed = status.is_client_error() || status.is_server_error();
    let Some(policy) = policy else { return failed.then_some("backend-request-failed") };
    match policy.mode {
        RequestLog::Off => None,
        _ if failed => Some("backend-request-failed"),
        _ if policy.slow.is_some_and(|slow| elapsed >= slow) => Some("backend-request-slow"),
        RequestLog::All if !policy.probe => Some("backend-request"),
        _ => None,
    }
}
impl EdgeState {
    pub(super) fn new(edge: &Edge) -> (Self, EdgeControl) {
        let lifecycle = Arc::new(Lifecycle {
            draining: AtomicBool::new(false),
            in_flight: AtomicUsize::new(0),
            max_in_flight: edge.max_in_flight,
        });
        (
            Self {
                trusted: Arc::new(edge.trust_proxy.clone()),
                lifecycle: lifecycle.clone(),
                log: LogPolicy { mode: edge.request_log, slow: edge.slow, probe: false },
            },
            EdgeControl(lifecycle),
        )
    }
}

fn hop(text: &str) -> Option<IpAddr> {
    let text = text.trim().trim_matches('"');
    text.parse::<IpAddr>()
        .ok()
        .or_else(|| text.parse::<SocketAddr>().ok().map(|a| a.ip()))
        .map(|ip| ip.to_canonical())
}

/// The client is the right-most `X-Forwarded-For` hop that is not a trusted proxy, and only when
/// the TCP peer itself is trusted. Hops left of the first untrusted address are client-supplied
/// and never consulted, so a client cannot pick its own address. An unreadable hop stops the walk
/// at the last trusted address.
pub fn resolve_client(peer: IpAddr, headers: &HeaderMap, trusted: &[IpNet]) -> IpAddr {
    let peer = peer.to_canonical();
    let is_trusted = |ip: &IpAddr| trusted.iter().any(|net| net.contains(ip));
    if trusted.is_empty() || !is_trusted(&peer) {
        return peer;
    }
    let mut hops: Vec<&str> = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        let Ok(value) = value.to_str() else { return peer };
        hops.extend(value.split(','));
    }
    let mut client = peer;
    for text in hops.iter().rev() {
        let Some(ip) = hop(text) else { return client };
        client = ip;
        if !is_trusted(&ip) {
            return ip;
        }
    }
    client
}

/// Rate-limit key: the resolved client IP (falls back to the TCP peer).
#[derive(Clone, Copy, Debug)]
pub(super) struct ClientKey;
impl KeyExtractor for ClientKey {
    type Key = IpAddr;
    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<IpAddr, GovernorError> {
        req.extensions()
            .get::<ClientIp>()
            .map(|c| c.0)
            .or_else(|| req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip()))
            .ok_or(GovernorError::UnableToExtractKey)
    }
}

fn probe(path: &str) -> bool {
    matches!(path, "/health" | "/health/live" | "/health/ready")
}

fn overloaded(retry_after: u64) -> Response {
    let mut response = Error::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "OVERLOADED",
        "The server is at capacity; retry after the Retry-After delay",
    )
    .into_response();
    response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
    response
}

struct Slot(Arc<Lifecycle>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) async fn edge(State(state): State<EdgeState>, mut req: Request, next: Next) -> Response {
    if let Some(peer) = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip()) {
        let client = resolve_client(peer, req.headers(), &state.trusted);
        req.extensions_mut().insert(ClientIp(client));
    }
    req.extensions_mut().insert(state.lifecycle.clone());
    let is_probe = probe(req.uri().path());
    let mut response = admit(&state, is_probe, req, next).await;
    response.extensions_mut().insert(LogPolicy { probe: is_probe, ..state.log });
    response
}
async fn admit(state: &EdgeState, is_probe: bool, req: Request, next: Next) -> Response {
    if is_probe {
        return next.run(req).await;
    }
    let lifecycle = state.lifecycle.clone();
    if lifecycle.in_flight.fetch_add(1, Ordering::SeqCst) >= lifecycle.max_in_flight {
        lifecycle.in_flight.fetch_sub(1, Ordering::SeqCst);
        return overloaded(1);
    }
    let _slot = Slot(lifecycle);
    let (response, saturated) = crate::platform::watch_pool(next.run(req)).await;
    if saturated && response.status() == StatusCode::INTERNAL_SERVER_ERROR {
        return overloaded(2);
    }
    response
}

pub(super) async fn live() -> Json<Value> {
    Json(json!({"ok":true,"schemaVersion":1,"status":"live"}))
}

pub(super) async fn ready(
    State(app): State<App>,
    Extension(lifecycle): Extension<Arc<Lifecycle>>,
) -> Response {
    if lifecycle.draining.load(Ordering::SeqCst) {
        let mut response = Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "DRAINING",
            "This instance is shutting down; route new traffic to another instance",
        )
        .into_response();
        response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(5u64));
        return response;
    }
    if lifecycle.in_flight.load(Ordering::SeqCst) >= lifecycle.max_in_flight {
        return overloaded(1);
    }
    match tokio::time::timeout(Duration::from_secs(1), sqlx::query("SELECT 1").execute(&app.pool)).await {
        Ok(Ok(_)) => Json(json!({"ok":true,"schemaVersion":1,"status":"ready"})).into_response(),
        Ok(Err(sqlx::Error::PoolTimedOut)) | Err(_) => overloaded(2),
        Ok(Err(_)) => Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "DATABASE_UNAVAILABLE",
            "Check Postgres connectivity",
        )
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in values {
            map.append("x-forwarded-for", value.parse().unwrap());
        }
        map
    }
    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn request_lines_follow_the_declared_policy() {
        let ms = Duration::from_millis;
        let policy = |mode, slow, probe| LogPolicy { mode, slow, probe };
        assert_eq!(request_event(None, StatusCode::OK, ms(5)), None);
        assert_eq!(request_event(None, StatusCode::NOT_FOUND, ms(5)), Some("backend-request-failed"));
        let errors = policy(RequestLog::Errors, Some(ms(500)), false);
        assert_eq!(request_event(Some(&errors), StatusCode::OK, ms(5)), None);
        assert_eq!(request_event(Some(&errors), StatusCode::OK, ms(900)), Some("backend-request-slow"));
        assert_eq!(request_event(Some(&errors), StatusCode::BAD_GATEWAY, ms(5)), Some("backend-request-failed"));
        let all = policy(RequestLog::All, None, false);
        assert_eq!(request_event(Some(&all), StatusCode::OK, ms(5)), Some("backend-request"));
        assert_eq!(request_event(Some(&policy(RequestLog::All, None, true)), StatusCode::OK, ms(5)), None);
        assert_eq!(request_event(Some(&policy(RequestLog::Off, None, false)), StatusCode::INTERNAL_SERVER_ERROR, ms(5)), None);
    }

    #[test]
    fn client_ip_walks_forwarded_hops_only_through_trusted_proxies() {
        let trusted = vec!["127.0.0.1/32".parse().unwrap(), "10.0.0.0/8".parse().unwrap()];
        let local = ip("127.0.0.1");
        // No trust list: today's behavior, the TCP peer.
        assert_eq!(resolve_client(local, &headers(&["203.0.113.9"]), &[]), local);
        // Untrusted peer: forwarded headers are ignored.
        assert_eq!(resolve_client(ip("198.51.100.4"), &headers(&["203.0.113.9"]), &trusted), ip("198.51.100.4"));
        // Behind nginx on loopback.
        assert_eq!(resolve_client(local, &headers(&["203.0.113.9"]), &trusted), ip("203.0.113.9"));
        // A spoofed left-most hop is never used.
        assert_eq!(resolve_client(local, &headers(&["1.1.1.1, 203.0.113.9"]), &trusted), ip("203.0.113.9"));
        // Two trusted proxies, and repeated headers in order.
        assert_eq!(resolve_client(local, &headers(&["1.1.1.1, 203.0.113.9, 10.2.3.4"]), &trusted), ip("203.0.113.9"));
        assert_eq!(resolve_client(local, &headers(&["1.1.1.1", "203.0.113.9", "10.2.3.4"]), &trusted), ip("203.0.113.9"));
        // Garbage at the right stops at the last trusted address.
        assert_eq!(resolve_client(local, &headers(&["203.0.113.9, nonsense"]), &trusted), local);
        // Only trusted hops: the left-most one.
        assert_eq!(resolve_client(local, &headers(&["10.9.9.9, 10.2.3.4"]), &trusted), ip("10.9.9.9"));
        // IPv4-mapped peers and hops with ports.
        assert_eq!(resolve_client(ip("::ffff:127.0.0.1"), &headers(&["203.0.113.9:4711"]), &trusted), ip("203.0.113.9"));
        assert_eq!(resolve_client(ip("::1"), &headers(&["2001:db8::1"]), &["::1/128".parse().unwrap()]), ip("2001:db8::1"));
        // No header: the peer.
        assert_eq!(resolve_client(local, &HeaderMap::new(), &trusted), local);
    }
}
