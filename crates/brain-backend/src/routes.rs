//! Application-declared HTTP routes (`server/routes/<name>/route.jsbrain`) and their inbound
//! signature verifiers. The handler is trusted project code run by an executor; this module owns
//! the contract, validation, signatures and rate limits around it.
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const MAX_ROUTES: usize = 128;
pub const DEFAULT_BODY_LIMIT: usize = 64 * 1024;
/// JSON and raw bodies are buffered for signature checks and parsing; bigger payloads are files.
pub const MAX_BODY_LIMIT: usize = 1024 * 1024;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const RESERVED: &[&str] = &["/_brain", "/api", "/health", "/s"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Public,
    SignedIn,
    Roles(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Str,
    Int,
    Bool,
    Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub kind: Kind,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateKey {
    Ip,
    User,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rate {
    pub count: u32,
    pub window: Duration,
    pub key: RateKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Hex,
    Base64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// `t=<unix seconds>,v1=<hex>[,v1=<hex>]` over `<t>.<raw body>`.
    Timestamped,
    /// `<prefix><digest>` over the raw body, e.g. `sha256=<hex>`.
    Hmac { prefix: String, encoding: Encoding },
}

#[derive(Clone)]
pub struct Verifier {
    pub scheme: Scheme,
    pub header: String,
    /// Several secrets allow rotation; any one may match.
    secrets: Vec<ring::hmac::Key>,
    pub tolerance: Duration,
}
impl std::fmt::Debug for Verifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Verifier").field("scheme", &self.scheme).field("header", &self.header).field("secrets", &self.secrets.len()).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    Missing,
    Malformed,
    Expired,
    Mismatch,
}

impl Verifier {
    pub fn new(scheme: Scheme, header: &str, secrets: &[Vec<u8>], tolerance: Duration) -> Self {
        Self {
            scheme,
            header: header.to_ascii_lowercase(),
            secrets: secrets.iter().map(|s| ring::hmac::Key::new(ring::hmac::HMAC_SHA256, s)).collect(),
            tolerance,
        }
    }
    fn check(&self, message: &[u8], digest: &[u8]) -> bool {
        self.secrets.iter().any(|key| ring::hmac::verify(key, message, digest).is_ok())
    }
    /// Constant-time verification of `value` (the signature header) over `raw`. `now` is unix seconds.
    pub fn verify(&self, value: Option<&str>, raw: &[u8], now: u64) -> Result<Option<u64>, Rejection> {
        let value = value.map(str::trim).filter(|v| !v.is_empty()).ok_or(Rejection::Missing)?;
        if value.len() > 2048 {
            return Err(Rejection::Malformed);
        }
        match &self.scheme {
            Scheme::Timestamped => {
                let mut timestamp = None;
                let mut digests = Vec::new();
                for part in value.split(',') {
                    let (k, v) = part.trim().split_once('=').ok_or(Rejection::Malformed)?;
                    match k {
                        "t" => timestamp = Some(v.parse::<u64>().map_err(|_| Rejection::Malformed)?),
                        "v1" => digests.push(hex(v).ok_or(Rejection::Malformed)?),
                        _ => {}
                    }
                }
                let timestamp = timestamp.ok_or(Rejection::Malformed)?;
                if digests.is_empty() {
                    return Err(Rejection::Malformed);
                }
                if now.abs_diff(timestamp) > self.tolerance.as_secs() {
                    return Err(Rejection::Expired);
                }
                let mut message = timestamp.to_string().into_bytes();
                message.push(b'.');
                message.extend_from_slice(raw);
                if digests.iter().any(|d| self.check(&message, d)) {
                    Ok(Some(timestamp))
                } else {
                    Err(Rejection::Mismatch)
                }
            }
            Scheme::Hmac { prefix, encoding } => {
                let encoded = value.strip_prefix(prefix.as_str()).ok_or(Rejection::Malformed)?;
                let digest = match encoding {
                    Encoding::Hex => hex(encoded),
                    Encoding::Base64 => base64(encoded),
                }
                .ok_or(Rejection::Malformed)?;
                if self.check(raw, &digest) { Ok(None) } else { Err(Rejection::Mismatch) }
            }
        }
    }
}

fn hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || text.len() > 256 {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

fn base64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Standard base64 with padding, for raw bodies that are not UTF-8.
pub fn encode_base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct Route {
    pub name: String,
    pub method: axum::http::Method,
    /// Axum path syntax: `/payments/{id}`.
    pub path: String,
    pub access: Access,
    pub raw_body: bool,
    pub body_limit: usize,
    pub timeout: Duration,
    pub rate: Option<Rate>,
    pub verify: Option<Verifier>,
    pub params: BTreeMap<String, Kind>,
    pub query: BTreeMap<String, Param>,
    pub headers: Vec<String>,
}

fn object<'a>(value: &'a Value, at: &str, keys: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let map = value.as_object().ok_or_else(|| format!("{at} must be an object literal"))?;
    if let Some(unknown) = map.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("{at} has unknown key `{unknown}`; allowed: {}", keys.join(", ")));
    }
    Ok(map)
}

fn kind(text: &str) -> Option<Kind> {
    Some(match text {
        "str" => Kind::Str,
        "int" => Kind::Int,
        "bool" => Kind::Bool,
        "uuid" => Kind::Uuid,
        _ => return None,
    })
}

fn segment_ok(seg: &str) -> bool {
    !seg.is_empty() && seg.len() <= 64 && seg.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_' || b == b'.')
        && seg != "." && seg != ".."
}

/// Parses `'60/min'`, `'10/s'`, `'1000/hour'` with an optional `' per user'` suffix.
pub fn parse_rate(text: &str) -> Option<Rate> {
    let text = text.trim().to_ascii_lowercase();
    let (spec, key) = match text.strip_suffix(" per user") {
        Some(spec) => (spec.trim(), RateKey::User),
        None => (text.strip_suffix(" per ip").unwrap_or(&text).trim(), RateKey::Ip),
    };
    let (count, unit) = spec.split_once('/')?;
    let count: u32 = count.trim().parse().ok().filter(|n| (1..=100_000).contains(n))?;
    let window = match unit.trim() {
        "s" | "sec" | "second" => Duration::from_secs(1),
        "min" | "minute" => Duration::from_secs(60),
        "h" | "hour" => Duration::from_secs(3600),
        "day" => Duration::from_secs(86_400),
        _ => return None,
    };
    Some(Rate { count, window, key })
}

impl Route {
    /// Reads a route contract. `env` resolves secret names; secret values never enter errors.
    pub fn from_contract(name: &str, value: &Value, env: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        if !crate::contract::identifier(name) || name.len() > 48 {
            return Err(format!("route `{name}`: the folder name must be a lowercase identifier"));
        }
        let at = format!("route `{name}`");
        let map = object(value, &at, &["method", "path", "auth", "body", "bodyLimit", "timeout", "rate", "verify", "params", "query", "headers"])?;
        let method = match map.get("method").and_then(Value::as_str).unwrap_or("GET").to_ascii_uppercase().as_str() {
            "GET" => axum::http::Method::GET,
            "POST" => axum::http::Method::POST,
            "PUT" => axum::http::Method::PUT,
            "PATCH" => axum::http::Method::PATCH,
            "DELETE" => axum::http::Method::DELETE,
            _ => return Err(format!("{at}: method must be GET, POST, PUT, PATCH or DELETE")),
        };
        let path = map.get("path").and_then(Value::as_str).ok_or_else(|| format!("{at}: path is required, e.g. '/payments/webhook'"))?;
        if !path.starts_with('/') || path.len() > 200 || path.ends_with('/') {
            return Err(format!("{at}: path must start with '/' and not end with '/'"));
        }
        if RESERVED.iter().any(|r| path == *r || path.starts_with(&format!("{r}/"))) {
            return Err(format!("{at}: path must not use the reserved prefixes {}", RESERVED.join(", ")));
        }
        let mut params = BTreeMap::new();
        for seg in path[1..].split('/') {
            if let Some(param) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                let named = param.len() <= 32 && param.starts_with(|c: char| c.is_ascii_lowercase()) && param.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
                if !named || params.insert(param.to_string(), Kind::Str).is_some() {
                    return Err(format!("{at}: path parameter `{seg}` must be a unique lowercase identifier"));
                }
            } else if !segment_ok(seg) {
                return Err(format!("{at}: path segment `{seg}` may use a-z, 0-9, '-', '_' and '.'"));
            }
        }
        if let Some(declared) = map.get("params") {
            for (key, kind_text) in object(declared, &format!("{at}.params"), &params.keys().map(String::as_str).collect::<Vec<_>>())? {
                let parsed = kind_text.as_str().and_then(kind).ok_or_else(|| format!("{at}.params.{key} must be 'str', 'int', 'bool' or 'uuid'"))?;
                params.insert(key.clone(), parsed);
            }
        }
        let access = match map.get("auth") {
            None => Access::SignedIn,
            Some(Value::String(s)) if s == "public" => Access::Public,
            Some(Value::String(s)) if s == "user" => Access::SignedIn,
            Some(Value::Array(roles)) if !roles.is_empty() && roles.len() <= 16 => Access::Roles(
                roles.iter().map(|r| r.as_str().filter(|r| crate::contract::identifier(r)).map(str::to_owned)).collect::<Option<Vec<_>>>()
                    .ok_or_else(|| format!("{at}.auth roles must be identifiers"))?,
            ),
            _ => return Err(format!("{at}.auth must be 'public', 'user' or a list of roles")),
        };
        let raw_body = match map.get("body").and_then(Value::as_str) {
            None | Some("json") => false,
            Some("raw") => true,
            _ => return Err(format!("{at}.body must be 'json' or 'raw'")),
        };
        let body_limit = match map.get("bodyLimit") {
            None => DEFAULT_BODY_LIMIT,
            Some(v) => crate::files::parse_size(v).ok().filter(|n| *n > 0 && *n <= MAX_BODY_LIMIT as u64).ok_or_else(|| format!("{at}.bodyLimit must be at most 1mb"))? as usize,
        };
        let timeout = match map.get("timeout") {
            None => DEFAULT_TIMEOUT,
            Some(v) => crate::platform::parse_duration(v).filter(|d| !d.is_zero() && *d <= MAX_TIMEOUT).ok_or_else(|| format!("{at}.timeout must be a duration up to 30s"))?,
        };
        let rate = match map.get("rate") {
            None => None,
            Some(v) => Some(v.as_str().and_then(parse_rate).ok_or_else(|| format!("{at}.rate must look like '60/min' or '10/min per user'"))?),
        };
        if matches!(rate, Some(Rate { key: RateKey::User, .. })) && access == Access::Public {
            return Err(format!("{at}: a per-user rate needs auth 'user' or roles"));
        }
        let mut query = BTreeMap::new();
        if let Some(declared) = map.get("query") {
            let declared = declared.as_object().ok_or_else(|| format!("{at}.query must map names to kinds"))?;
            if declared.len() > 32 {
                return Err(format!("{at}.query declares at most 32 parameters"));
            }
            for (key, spec) in declared {
                let text = spec.as_str().unwrap_or_default();
                let (text, required) = text.strip_suffix('!').map_or((text, false), |t| (t, true));
                let parsed = kind(text).filter(|_| key.len() <= 32 && key.starts_with(|c: char| c.is_ascii_lowercase()) && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')).ok_or_else(|| format!("{at}.query.{key} must be 'str', 'int', 'bool' or 'uuid' (suffix '!' when required)"))?;
                query.insert(key.clone(), Param { kind: parsed, required });
            }
        }
        let mut headers = Vec::new();
        if let Some(list) = map.get("headers") {
            for h in list.as_array().ok_or_else(|| format!("{at}.headers must be a list of header names"))? {
                let h = h.as_str().map(str::to_ascii_lowercase).filter(|h| !h.is_empty() && h.len() <= 64 && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
                    .ok_or_else(|| format!("{at}.headers entries must be header names"))?;
                if matches!(h.as_str(), "cookie" | "authorization" | "proxy-authorization") {
                    return Err(format!("{at}.headers must not forward `{h}`; the backend authenticates the request"));
                }
                if !headers.contains(&h) {
                    headers.push(h);
                }
            }
            if headers.len() > 16 {
                return Err(format!("{at}.headers forwards at most 16 headers"));
            }
        }
        let verify = map.get("verify").map(|v| verifier(&at, v, env)).transpose()?;
        if verify.is_some() && method == axum::http::Method::GET {
            return Err(format!("{at}: signed routes receive a body; use POST, PUT or PATCH"));
        }
        Ok(Route { name: name.into(), method, path: path.into(), access, raw_body, body_limit, timeout, rate, verify, params, query, headers })
    }
    pub fn schema(&self) -> Value {
        json!({
            "name": self.name, "method": self.method.as_str(), "path": self.path,
            "auth": match &self.access { Access::Public => json!("public"), Access::SignedIn => json!("user"), Access::Roles(r) => json!(r) },
            "body": if self.raw_body { "raw" } else { "json" }, "bodyLimit": self.body_limit, "timeoutMs": self.timeout.as_millis() as u64,
            "rate": self.rate.as_ref().map(|r| json!({"count": r.count, "windowSeconds": r.window.as_secs(), "per": if r.key == RateKey::User { "user" } else { "ip" }})),
            "signed": self.verify.as_ref().map(|v| json!({"header": v.header, "scheme": match v.scheme { Scheme::Timestamped => "timestamped", Scheme::Hmac { .. } => "hmac" }})),
            "params": self.params.iter().map(|(k, v)| (k.clone(), json!(format!("{v:?}").to_lowercase()))).collect::<Map<_, _>>(),
            "query": self.query.iter().map(|(k, p)| (k.clone(), json!({"kind": format!("{:?}", p.kind).to_lowercase(), "required": p.required}))).collect::<Map<_, _>>(),
            "headers": self.headers,
        })
    }
}

fn verifier(at: &str, value: &Value, env: &dyn Fn(&str) -> Option<String>) -> Result<Verifier, String> {
    let at = format!("{at}.verify");
    let map = object(value, &at, &["scheme", "header", "secret", "tolerance", "prefix", "encoding"])?;
    let header = map.get("header").and_then(Value::as_str).ok_or_else(|| format!("{at}.header names the signature header"))?;
    let names: Vec<&str> = match map.get("secret") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(list)) if !list.is_empty() && list.len() <= 4 => list.iter().map(|v| v.as_str().ok_or_else(|| format!("{at}.secret entries are environment variable names"))).collect::<Result<_, _>>()?,
        _ => return Err(format!("{at}.secret is the environment variable name holding the signing secret (or up to 4 for rotation)")),
    };
    let mut secrets = Vec::new();
    for name in names {
        if !name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') || name.is_empty() {
            return Err(format!("{at}.secret must be an environment variable name like PAYMENTS_WEBHOOK_SECRET, never the secret itself"));
        }
        let secret = env(name).filter(|s| s.len() >= 16).ok_or_else(|| format!("{at}: set {name} (at least 16 bytes) in the environment or .brain_env"))?;
        secrets.push(secret.into_bytes());
    }
    let scheme = match map.get("scheme").and_then(Value::as_str).unwrap_or("hmac") {
        "timestamped" => {
            if map.contains_key("prefix") || map.contains_key("encoding") {
                return Err(format!("{at}: prefix and encoding apply to the 'hmac' scheme"));
            }
            Scheme::Timestamped
        }
        "hmac" => Scheme::Hmac {
            prefix: map.get("prefix").and_then(Value::as_str).unwrap_or("").to_string(),
            encoding: match map.get("encoding").and_then(Value::as_str).unwrap_or("hex") {
                "hex" => Encoding::Hex,
                "base64" => Encoding::Base64,
                _ => return Err(format!("{at}.encoding must be 'hex' or 'base64'")),
            },
        },
        _ => return Err(format!("{at}.scheme must be 'timestamped' (t=…,v1=…) or 'hmac' (e.g. sha256=…)")),
    };
    let tolerance = match map.get("tolerance") {
        None => Duration::from_secs(300),
        Some(v) => crate::platform::parse_duration(v).filter(|d| d.as_secs() >= 1 && d.as_secs() <= 3600).ok_or_else(|| format!("{at}.tolerance must be between 1s and 1h"))?,
    };
    Ok(Verifier::new(scheme, header, &secrets, tolerance))
}

/// Rejects keys that reach JavaScript prototypes, at any depth.
pub fn prototype_safe(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().all(|(k, v)| !matches!(k.as_str(), "__proto__" | "constructor" | "prototype") && prototype_safe(v)),
        Value::Array(items) => items.iter().all(prototype_safe),
        _ => true,
    }
}

/// Validates query parameters against the declaration; unknown parameters are rejected.
pub fn typed(kind: Kind, text: &str) -> Option<Value> {
    match kind {
        Kind::Str => (text.len() <= 1024).then(|| json!(text)),
        Kind::Int => text.parse::<i64>().ok().map(|n| json!(n)),
        Kind::Bool => match text {
            "true" | "1" => Some(json!(true)),
            "false" | "0" => Some(json!(false)),
            _ => None,
        },
        Kind::Uuid => uuid::Uuid::parse_str(text).ok().map(|u| json!(u.to_string())),
    }
}

/// Fixed-window counters per (route, key). Memory is bounded by pruning expired windows.
#[derive(Default)]
pub struct Limiter {
    windows: Mutex<HashMap<(String, String), (Instant, u32)>>,
}
impl Limiter {
    /// Returns `Err(seconds until reset)` when the key is over its budget.
    pub fn hit(&self, route: &str, key: &str, rate: &Rate, now: Instant) -> Result<(), u64> {
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if windows.len() > 50_000 {
            windows.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(86_400));
            if windows.len() > 50_000 {
                windows.clear();
            }
        }
        let entry = windows.entry((route.to_string(), key.to_string())).or_insert((now, 0));
        if now.duration_since(entry.0) >= rate.window {
            *entry = (now, 0);
        }
        if entry.1 >= rate.count {
            return Err((rate.window.saturating_sub(now.duration_since(entry.0))).as_secs().max(1));
        }
        entry.1 += 1;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum RouteFailure {
    /// The handler rejected the request with a 4xx status and a JSON body.
    Status(u16, Value),
    Capacity,
    Timeout,
    Unavailable,
}

/// Runs project route handlers. Implemented by the CLI with private Node sidecars.
pub trait RouteHandlers: Send + Sync {
    fn names(&self) -> Vec<String>;
    fn run(&self, name: &str, context: Value, timeout: Duration) -> Pin<Box<dyn Future<Output = Result<Value, RouteFailure>> + Send + '_>>;
}
pub type Handlers = Arc<dyn RouteHandlers>;

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        (name == "PAYMENTS_WEBHOOK_SECRET").then(|| "whsec_synthetic_test_secret_0001".into())
    }
    fn sign(secret: &[u8], message: &[u8]) -> Vec<u8> {
        ring::hmac::sign(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret), message).as_ref().to_vec()
    }
    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn route_contracts_validate_paths_auth_limits_and_secrets() {
        let route = Route::from_contract("payment_events", &json!({
            "method": "post", "path": "/payments/{provider}/events", "auth": "public", "body": "raw", "bodyLimit": "256kb",
            "timeout": "15s", "rate": "120/min", "params": {"provider": "str"},
            "verify": {"scheme": "timestamped", "header": "X-Payment-Signature", "secret": "PAYMENTS_WEBHOOK_SECRET"}
        }), &env).unwrap();
        assert_eq!(route.body_limit, 256 * 1024);
        assert_eq!(route.timeout, Duration::from_secs(15));
        assert_eq!(route.verify.as_ref().unwrap().header, "x-payment-signature");
        assert!(!format!("{route:?}").contains("whsec"), "secrets never print");
        let defaults = Route::from_contract("orders_export", &json!({"path": "/orders/export", "query": {"page": "int", "from": "str!"}}), &env).unwrap();
        assert_eq!(defaults.access, Access::SignedIn);
        assert_eq!(defaults.method, axum::http::Method::GET);
        assert!(defaults.query["from"].required);
        for bad in [
            json!({"path": "/api/x"}),
            json!({"path": "/_brain/x"}),
            json!({"path": "/health"}),
            json!({"path": "no-slash"}),
            json!({"path": "/a/../b"}),
            json!({"path": "/a/{Id}"}),
            json!({"path": "/a", "method": "TRACE"}),
            json!({"path": "/a", "bodyLimit": "2mb"}),
            json!({"path": "/a", "timeout": "1m"}),
            json!({"path": "/a", "rate": "lots"}),
            json!({"path": "/a", "auth": "public", "rate": "10/min per user"}),
            json!({"path": "/a", "headers": ["cookie"]}),
            json!({"path": "/a", "method": "POST", "verify": {"header": "x-sig", "secret": "whsec_literal_secret_value"}}),
            json!({"path": "/a", "method": "POST", "verify": {"header": "x-sig", "secret": "MISSING_SECRET"}}),
            json!({"path": "/a", "verify": {"header": "x-sig", "secret": "PAYMENTS_WEBHOOK_SECRET"}}),
            json!({"path": "/a", "handler": "x"}),
        ] {
            assert!(Route::from_contract("r", &bad, &env).is_err(), "{bad}");
        }
    }

    #[test]
    fn timestamped_signatures_check_time_rotation_and_bytes() {
        let secret = b"whsec_synthetic_test_secret_0001";
        let v = Verifier::new(Scheme::Timestamped, "x-sig", &[b"old_rotated_secret_value".to_vec(), secret.to_vec()], Duration::from_secs(300));
        let body = br#"{"id":"evt_1","type":"payment.succeeded"}"#;
        let now = 1_800_000_000;
        let digest = to_hex(&sign(secret, format!("{now}.").as_bytes().iter().chain(body).copied().collect::<Vec<_>>().as_slice()));
        let header = format!("t={now},v1=deadbeef,v1={digest}");
        assert_eq!(v.verify(Some(&header), body, now + 10), Ok(Some(now)));
        assert_eq!(v.verify(Some(&header), b"{\"id\":\"evt_2\"}", now), Err(Rejection::Mismatch));
        assert_eq!(v.verify(Some(&header), body, now + 301), Err(Rejection::Expired));
        assert_eq!(v.verify(Some(&format!("t={},v1={digest}", now + 1)), body, now), Err(Rejection::Mismatch), "timestamp is signed");
        assert_eq!(v.verify(None, body, now), Err(Rejection::Missing));
        assert_eq!(v.verify(Some("v1=abcd"), body, now), Err(Rejection::Malformed));
        assert_eq!(v.verify(Some("t=x,v1=ab"), body, now), Err(Rejection::Malformed));
    }

    #[test]
    fn prefixed_hmac_signatures_accept_hex_and_base64() {
        let secret = b"whsec_synthetic_test_secret_0001";
        let body = b"payload";
        let digest = sign(secret, body);
        let hex = Verifier::new(Scheme::Hmac { prefix: "sha256=".into(), encoding: Encoding::Hex }, "x-hub-signature-256", &[secret.to_vec()], Duration::from_secs(300));
        assert_eq!(hex.verify(Some(&format!("sha256={}", to_hex(&digest))), body, 0), Ok(None));
        assert_eq!(hex.verify(Some(&to_hex(&digest)), body, 0), Err(Rejection::Malformed));
        assert_eq!(hex.verify(Some(&format!("sha256={}", to_hex(&digest))), b"payload!", 0), Err(Rejection::Mismatch));
        let b64 = Verifier::new(Scheme::Hmac { prefix: String::new(), encoding: Encoding::Base64 }, "x-signature", &[secret.to_vec()], Duration::from_secs(300));
        assert_eq!(b64.verify(Some(&encode_base64(&digest)), body, 0), Ok(None));
        assert_eq!(base64(&encode_base64(b"any bytes \x00\xff")).unwrap(), b"any bytes \x00\xff");
    }

    #[test]
    fn rates_query_values_and_prototype_keys() {
        assert_eq!(parse_rate("10/min per user"), Some(Rate { count: 10, window: Duration::from_secs(60), key: RateKey::User }));
        assert_eq!(parse_rate("5/s").unwrap().key, RateKey::Ip);
        assert_eq!(parse_rate("0/min"), None);
        let limiter = Limiter::default();
        let rate = parse_rate("2/min").unwrap();
        let t = Instant::now();
        assert!(limiter.hit("r", "a", &rate, t).is_ok());
        assert!(limiter.hit("r", "a", &rate, t).is_ok());
        assert!(limiter.hit("r", "a", &rate, t).is_err());
        assert!(limiter.hit("r", "b", &rate, t).is_ok());
        assert!(limiter.hit("r", "a", &rate, t + Duration::from_secs(61)).is_ok());
        assert_eq!(typed(Kind::Int, "42"), Some(json!(42)));
        assert_eq!(typed(Kind::Int, "4x"), None);
        assert_eq!(typed(Kind::Bool, "true"), Some(json!(true)));
        assert!(prototype_safe(&json!({"a": [{"b": 1}]})));
        assert!(!prototype_safe(&json!({"a": [{"__proto__": {"admin": true}}]})));
        assert!(!prototype_safe(&json!({"constructor": {"prototype": 1}})));
    }
}
