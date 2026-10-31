//! Application-declared platform settings: file buckets plus the HTTP edge (trusted proxies,
//! extra CORS origins, backpressure and shutdown budgets). Everything is optional; the defaults
//! keep the behavior of a backend without `server/platform.jsbrain`.
use crate::files::Bucket;
use ipnet::IpNet;
use serde_json::Value;
use std::{cell::Cell, net::IpAddr, time::Duration};

/// Upper bound for declared trusted proxy ranges.
pub const MAX_TRUSTED_PROXIES: usize = 64;
/// Default concurrent request cap before the backend answers `503 OVERLOADED`.
pub const DEFAULT_MAX_IN_FLIGHT: usize = 512;
/// Default bounded drain after a shutdown signal.
pub const DEFAULT_DRAIN: Duration = Duration::from_secs(25);

#[derive(Clone, Default)]
pub struct Platform {
    pub buckets: Vec<Bucket>,
    pub edge: Edge,
    /// Application-declared routes and the executor that runs their handlers.
    pub routes: Vec<crate::routes::Route>,
    pub handlers: Option<crate::routes::Handlers>,
}

/// HTTP edge settings read from the `server/platform.jsbrain` literal.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// Proxies whose `X-Forwarded-For` hops are trusted. Empty: the TCP peer is the client.
    pub trust_proxy: Vec<IpNet>,
    /// Extra browser origins allowed for CORS and mutations, on top of the site origin.
    pub cors_origins: Vec<String>,
    /// `Access-Control-Max-Age` for preflight responses.
    pub cors_max_age: Option<Duration>,
    pub max_in_flight: usize,
    /// Time readiness reports `DRAINING` before the listener stops accepting connections.
    pub ready_delay: Duration,
    /// Upper bound for in-flight requests to finish after the listener closes.
    pub drain: Duration,
}

impl Default for Edge {
    fn default() -> Self {
        Self {
            trust_proxy: Vec::new(),
            cors_origins: Vec::new(),
            cors_max_age: None,
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
            ready_delay: Duration::ZERO,
            drain: DEFAULT_DRAIN,
        }
    }
}

fn object<'a>(value: &'a Value, at: &str, keys: &[&str]) -> Result<&'a serde_json::Map<String, Value>, String> {
    let map = value
        .as_object()
        .ok_or_else(|| format!("{at} must be an object literal"))?;
    if let Some(unknown) = map.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("{at} has unknown key `{unknown}`; allowed: {}", keys.join(", ")));
    }
    Ok(map)
}

/// Parses `500ms`, `5s`, `2m` or a bare integer of milliseconds.
pub fn parse_duration(value: &Value) -> Option<Duration> {
    if let Some(ms) = value.as_u64() {
        return Some(Duration::from_millis(ms));
    }
    let text = value.as_str()?.trim().to_ascii_lowercase();
    let (digits, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = digits.parse().ok()?;
    match unit.trim() {
        "ms" => Some(Duration::from_millis(n)),
        "s" => n.checked_mul(1000).map(Duration::from_millis),
        "m" | "min" => n.checked_mul(60_000).map(Duration::from_millis),
        _ => None,
    }
}

fn bounded(value: Option<&Value>, at: &str, max: Duration, default: Duration) -> Result<Duration, String> {
    let Some(value) = value else { return Ok(default) };
    let duration = parse_duration(value).ok_or_else(|| format!("{at} must be a duration such as '5s' or 500 (milliseconds)"))?;
    if duration > max {
        return Err(format!("{at} must be at most {}s", max.as_secs()));
    }
    Ok(duration)
}

/// Parses `127.0.0.1`, `10.0.0.0/8` or `::1/128`. Catch-all ranges are refused: trusting every
/// peer would let any client choose its own rate-limit key.
pub fn parse_proxy(text: &str) -> Result<IpNet, String> {
    let net = match text.parse::<IpNet>() {
        Ok(net) => net,
        Err(_) => IpNet::from(
            text.parse::<IpAddr>()
                .map_err(|_| format!("trustProxy entry `{text}` must be an IP address or CIDR range"))?,
        ),
    };
    if net.prefix_len() == 0 {
        return Err(format!("trustProxy entry `{text}` trusts every address; list your proxy ranges instead"));
    }
    Ok(net.trunc())
}

impl Edge {
    /// Reads the `server/platform.jsbrain` literal. Unknown keys are rejected so typos never
    /// silently fall back to defaults.
    pub fn from_contract(value: &Value) -> Result<Self, String> {
        let map = object(value, "platform", &["trustProxy", "cors", "limits", "shutdown"])?;
        let mut edge = Edge::default();
        if let Some(list) = map.get("trustProxy") {
            let list = list.as_array().ok_or("platform.trustProxy must be an array of IPs or CIDR ranges")?;
            if list.len() > MAX_TRUSTED_PROXIES {
                return Err(format!("platform.trustProxy accepts at most {MAX_TRUSTED_PROXIES} entries"));
            }
            for entry in list {
                let text = entry.as_str().ok_or("platform.trustProxy entries must be strings")?;
                let net = parse_proxy(text)?;
                if !edge.trust_proxy.contains(&net) {
                    edge.trust_proxy.push(net);
                }
            }
        }
        if let Some(cors) = map.get("cors") {
            let cors = object(cors, "platform.cors", &["origins", "maxAge"])?;
            if let Some(origins) = cors.get("origins") {
                let origins = origins.as_array().ok_or("platform.cors.origins must be an array of origins")?;
                if origins.len() > 32 {
                    return Err("platform.cors.origins accepts at most 32 origins".into());
                }
                for origin in origins {
                    let origin = origin.as_str().ok_or("platform.cors.origins entries must be strings")?;
                    if !edge.cors_origins.iter().any(|known| known == origin) {
                        edge.cors_origins.push(origin.to_string());
                    }
                }
            }
            if let Some(age) = cors.get("maxAge") {
                let secs = age.as_u64().filter(|s| *s <= 86_400).ok_or("platform.cors.maxAge must be whole seconds in 0..86400")?;
                edge.cors_max_age = Some(Duration::from_secs(secs));
            }
        }
        if let Some(limits) = map.get("limits") {
            let limits = object(limits, "platform.limits", &["maxInFlight"])?;
            if let Some(cap) = limits.get("maxInFlight") {
                edge.max_in_flight = cap
                    .as_u64()
                    .filter(|n| (1..=100_000).contains(n))
                    .ok_or("platform.limits.maxInFlight must be an integer in 1..100000")? as usize;
            }
        }
        if let Some(shutdown) = map.get("shutdown") {
            let shutdown = object(shutdown, "platform.shutdown", &["readyDelay", "drain"])?;
            edge.ready_delay = bounded(shutdown.get("readyDelay"), "platform.shutdown.readyDelay", Duration::from_secs(60), Duration::ZERO)?;
            edge.drain = bounded(shutdown.get("drain"), "platform.shutdown.drain", Duration::from_secs(120), DEFAULT_DRAIN)?;
            if edge.drain.is_zero() {
                return Err("platform.shutdown.drain must be greater than zero".into());
            }
        }
        Ok(edge)
    }
}

tokio::task_local! {
    /// Set when a request's database work waited longer than the pool acquire budget.
    static POOL_SATURATED: Cell<bool>;
}

/// Runs `work` while recording pool saturation; returns whether the pool timed out during it.
pub(crate) async fn watch_pool<F: std::future::Future>(work: F) -> (F::Output, bool) {
    POOL_SATURATED
        .scope(Cell::new(false), async move {
            let out = work.await;
            (out, POOL_SATURATED.with(Cell::get))
        })
        .await
}

/// Called by sanitized database error handling; a no-op outside an HTTP request.
pub(crate) fn note_database_error(error: &sqlx::Error) {
    if matches!(error, sqlx::Error::PoolTimedOut) {
        let _ = POOL_SATURATED.try_with(|flag| flag.set(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edge_contract_parses_defaults_and_rejects_unsafe_values() {
        assert_eq!(Edge::from_contract(&json!({})).unwrap(), Edge::default());
        let edge = Edge::from_contract(&json!({
            "trustProxy": ["127.0.0.1", "10.0.0.0/8", "::1/128", "10.1.2.3/8"],
            "cors": {"origins": ["https://admin.shop.example"], "maxAge": 600},
            "limits": {"maxInFlight": 64},
            "shutdown": {"readyDelay": "5s", "drain": 30000}
        }))
        .unwrap();
        assert_eq!(edge.trust_proxy.len(), 3, "equal ranges collapse after truncation");
        assert_eq!(edge.cors_max_age, Some(Duration::from_secs(600)));
        assert_eq!(edge.max_in_flight, 64);
        assert_eq!(edge.ready_delay, Duration::from_secs(5));
        assert_eq!(edge.drain, Duration::from_secs(30));
        for bad in [
            json!({"trustProxy": ["0.0.0.0/0"]}),
            json!({"trustProxy": ["::/0"]}),
            json!({"trustProxy": ["proxy.internal"]}),
            json!({"trustProxy": 2}),
            json!({"cors": {"maxAge": 90000}}),
            json!({"limits": {"maxInFlight": 0}}),
            json!({"shutdown": {"drain": "0s"}}),
            json!({"shutdown": {"readyDelay": "2h"}}),
            json!({"trustProxies": []}),
        ] {
            assert!(Edge::from_contract(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn durations_accept_units_and_milliseconds() {
        assert_eq!(parse_duration(&json!("500ms")), Some(Duration::from_millis(500)));
        assert_eq!(parse_duration(&json!("5s")), Some(Duration::from_secs(5)));
        assert_eq!(parse_duration(&json!("2m")), Some(Duration::from_secs(120)));
        assert_eq!(parse_duration(&json!(250)), Some(Duration::from_millis(250)));
        assert_eq!(parse_duration(&json!("5 hours")), None);
        assert_eq!(parse_duration(&json!("s")), None);
    }

    #[tokio::test]
    async fn pool_timeouts_are_recorded_only_inside_the_watched_request() {
        note_database_error(&sqlx::Error::PoolTimedOut);
        let ((), saturated) = watch_pool(async { note_database_error(&sqlx::Error::PoolTimedOut) }).await;
        assert!(saturated);
        let ((), saturated) = watch_pool(async { note_database_error(&sqlx::Error::RowNotFound) }).await;
        assert!(!saturated);
    }
}
