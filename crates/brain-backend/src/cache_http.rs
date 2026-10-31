//! HTTP caching for resource reads: validators (`ETag`, `If-None-Match` → 304), declared public caching for anonymous
//! reads (`server/cache.jsbrain`: `max-age`, `stale-while-revalidate`, `stale-if-error`) and conditional writes
//! (`If-Match` → 412) on top of the existing compare-and-swap revision check.
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

/// Largest response body hashed for a validator; larger bodies are sent without one.
const MAX_HASHED: usize = 2 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(86_400);
const MAX_STALE: Duration = Duration::from_secs(7 * 86_400);

/// One resource's declared public cache lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lifetime {
    pub max_age: Duration,
    pub stale_while_revalidate: Option<Duration>,
    pub stale_if_error: Option<Duration>,
}

impl Lifetime {
    pub fn header(&self) -> String {
        let mut out = format!("public, max-age={}", self.max_age.as_secs());
        if let Some(d) = self.stale_while_revalidate {
            out.push_str(&format!(", stale-while-revalidate={}", d.as_secs()));
        }
        if let Some(d) = self.stale_if_error {
            out.push_str(&format!(", stale-if-error={}", d.as_secs()));
        }
        out
    }
}

/// `server/cache.jsbrain`: resource name → lifetime. Applies only to anonymous `GET /api/<resource>[/<id>]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CachePolicy {
    pub resources: BTreeMap<String, Lifetime>,
}

/// Accepts `90`, `'30s'`, `'5m'`, `'2h'`, `'1d'`.
fn duration(value: &Value) -> Option<Duration> {
    if let Some(n) = value.as_u64() {
        return Some(Duration::from_secs(n));
    }
    let text = value.as_str()?.trim();
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (digits, unit) = text.split_at(split);
    let n: u64 = digits.parse().ok()?;
    let secs = match unit {
        "s" => n,
        "m" => n.checked_mul(60)?,
        "h" => n.checked_mul(3600)?,
        "d" => n.checked_mul(86_400)?,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

impl CachePolicy {
    /// Parses and validates the literal against the declared resources: each must grant `public` read.
    pub fn from_contract(value: &Value, resources: &[crate::Resource]) -> Result<Self, String> {
        let map = value.as_object().ok_or("cache.jsbrain must export an object such as { products: { maxAge: '1m' } }")?;
        if map.len() > 64 {
            return Err("cache.jsbrain declares at most 64 resources".into());
        }
        let mut out = BTreeMap::new();
        for (name, entry) in map {
            let Some(resource) = resources.iter().find(|r| &r.name == name) else {
                return Err(format!("cache.jsbrain names the unknown resource `{name}`"));
            };
            if !resource.policy.read.iter().any(|g| g == "public") {
                return Err(format!("cache.jsbrain: `{name}` is not publicly readable; only resources whose policy read list contains 'public' can be cached by shared caches"));
            }
            let entry = entry.as_object().ok_or_else(|| format!("cache.jsbrain: `{name}` must be an object such as {{ maxAge: '1m', staleWhileRevalidate: '10m' }}"))?;
            if let Some(key) = entry.keys().find(|k| !matches!(k.as_str(), "maxAge" | "staleWhileRevalidate" | "staleIfError")) {
                return Err(format!("cache.jsbrain: `{name}` has the unknown key `{key}`; use maxAge, staleWhileRevalidate, staleIfError"));
            }
            let max_age = entry
                .get("maxAge")
                .and_then(duration)
                .filter(|d| *d <= MAX_AGE)
                .ok_or_else(|| format!("cache.jsbrain: `{name}`.maxAge is required, as '30s', '5m', '2h' or '1d', at most 1d"))?;
            let stale = |key: &str| -> Result<Option<Duration>, String> {
                match entry.get(key) {
                    None => Ok(None),
                    Some(v) => duration(v).filter(|d| *d <= MAX_STALE).map(Some).ok_or_else(|| format!("cache.jsbrain: `{name}`.{key} must be a duration such as '10m', at most 7d")),
                }
            };
            out.insert(name.clone(), Lifetime { max_age, stale_while_revalidate: stale("staleWhileRevalidate")?, stale_if_error: stale("staleIfError")? });
        }
        Ok(Self { resources: out })
    }
    pub fn schema(&self) -> Value {
        json!(self.resources.iter().map(|(name, l)| (name.clone(), json!({
            "maxAge": l.max_age.as_secs(),
            "staleWhileRevalidate": l.stale_while_revalidate.map(|d| d.as_secs()),
            "staleIfError": l.stale_if_error.map(|d| d.as_secs()),
            "cacheControl": l.header(),
        }))).collect::<serde_json::Map<_, _>>())
    }
}

/// Marks a response whose `Cache-Control` is the declared public lifetime instead of `no-store`.
#[derive(Clone)]
pub(crate) struct Declared(pub HeaderValue);

/// Strong validator of one record revision as one reader sees it: id, Postgres row revision and the visible keys.
pub(crate) fn record_etag(record: &Value, revision: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(record["id"].as_str().unwrap_or_default().as_bytes());
    hash.update(b"\0");
    hash.update(revision.as_bytes());
    if let Some(data) = record["data"].as_object() {
        for key in data.keys() {
            hash.update(b"\0");
            hash.update(key.as_bytes());
        }
    }
    let digest = hash.finalize();
    format!("\"r{}\"", digest.iter().take(12).map(|b| format!("{b:02x}")).collect::<String>())
}

fn weak_etag(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    format!("W/\"b{}\"", digest.iter().take(12).map(|b| format!("{b:02x}")).collect::<String>())
}

fn opaque(tag: &str) -> &str {
    tag.trim().trim_start_matches("W/")
}

/// Weak comparison (RFC 9110 §13.1.2): `*` or any listed tag equal after removing `W/`.
fn none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers.get_all(header::IF_NONE_MATCH).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(',')).any(|t| t.trim() == "*" || opaque(t) == opaque(etag))
}

/// Strong comparison for `If-Match` (RFC 9110 §13.1.1): a weak tag never matches.
pub(crate) fn if_match(headers: &HeaderMap, etag: &str) -> Result<(), super::Error> {
    let values: Vec<&str> = headers.get_all(header::IF_MATCH).iter().filter_map(|v| v.to_str().ok()).collect();
    if values.is_empty() {
        return Ok(());
    }
    let matched = values.iter().flat_map(|v| v.split(',')).map(str::trim).any(|t| t == "*" || (!t.starts_with("W/") && t == etag));
    if matched {
        Ok(())
    } else {
        Err(super::Error::new(StatusCode::PRECONDITION_FAILED, "PRECONDITION_FAILED", "The record changed since you read it; GET it again and retry with the new ETag in If-Match"))
    }
}

fn anonymous(headers: &HeaderMap) -> bool {
    !headers.contains_key(header::COOKIE) && !headers.contains_key(header::AUTHORIZATION) && !headers.contains_key("x-api-key")
}

/// `/api/<name>` or `/api/<name>/<uuid>` → resource name.
fn resource_read(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/")?;
    let mut parts = rest.split('/');
    let name = parts.next().filter(|n| crate::contract::identifier(n))?;
    match (parts.next(), parts.next()) {
        (None, _) => Some(name),
        (Some(id), None) if uuid::Uuid::parse_str(id).is_ok() => Some(name),
        _ => None,
    }
}

/// Adds validators and 304 answers to successful `GET /api/...` responses and marks declared public reads.
pub(crate) async fn conditional(State(policy): State<Arc<CachePolicy>>, req: Request, next: Next) -> Response {
    if req.method() != Method::GET || !req.uri().path().starts_with("/api/") {
        return next.run(req).await;
    }
    let headers = req.headers().clone();
    let lifetime = resource_read(req.uri().path())
        .filter(|_| anonymous(&headers))
        .and_then(|name| policy.resources.get(name))
        .map(Lifetime::header);
    let response = next.run(req).await;
    if response.status() != StatusCode::OK {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let (etag, body) = match parts.headers.get(header::ETAG).and_then(|v| v.to_str().ok()).map(str::to_owned) {
        Some(etag) => (etag, body),
        None => match to_bytes(body, MAX_HASHED).await {
            Ok(bytes) => (weak_etag(&bytes), Body::from(bytes)),
            Err(_) => return super::Error::internal("response body exceeded the validator budget").into_response(),
        },
    };
    let Ok(value) = HeaderValue::from_str(&etag) else {
        return Response::from_parts(parts, body);
    };
    parts.headers.insert(header::ETAG, value);
    parts.headers.append(header::VARY, HeaderValue::from_static("Cookie, Authorization, X-API-Key"));
    if let Some(cache) = lifetime.and_then(|l| HeaderValue::from_str(&l).ok()) {
        parts.extensions.insert(Declared(cache));
    }
    if none_match(&headers, &etag) {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(header::CONTENT_TYPE);
        parts.headers.remove(header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::empty());
    }
    Response::from_parts(parts, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resource(read: &[&str]) -> crate::Resource {
        serde_json::from_value(json!({"name":"products","fields":{"title":{"kind":"str","required":true}},"policy":{"read":read,"create":["admin"],"update":["admin"],"delete":["admin"]}})).unwrap()
    }
    #[test]
    fn policy_requires_public_read_and_bounded_durations() {
        let ok = CachePolicy::from_contract(&json!({"products": {"maxAge": "1m", "staleWhileRevalidate": "10m", "staleIfError": "1d"}}), &[resource(&["public"])]).unwrap();
        assert_eq!(ok.resources["products"].header(), "public, max-age=60, stale-while-revalidate=600, stale-if-error=86400");
        assert!(CachePolicy::from_contract(&json!({"products": {"maxAge": "1m"}}), &[resource(&["user"])]).unwrap_err().contains("not publicly readable"));
        assert!(CachePolicy::from_contract(&json!({"orders": {"maxAge": "1m"}}), &[resource(&["public"])]).is_err());
        assert!(CachePolicy::from_contract(&json!({"products": {"maxAge": "2d"}}), &[resource(&["public"])]).is_err());
        assert!(CachePolicy::from_contract(&json!({"products": {"ttl": "1m"}}), &[resource(&["public"])]).is_err());
    }
    #[test]
    fn validators_compare_like_rfc_9110() {
        let mut h = HeaderMap::new();
        h.insert(header::IF_NONE_MATCH, HeaderValue::from_static("\"x\", W/\"b1\""));
        assert!(none_match(&h, "W/\"b1\""));
        assert!(none_match(&h, "\"x\""));
        assert!(!none_match(&h, "\"y\""));
        let mut h = HeaderMap::new();
        h.insert(header::IF_MATCH, HeaderValue::from_static("W/\"r1\""));
        assert!(if_match(&h, "\"r1\"").is_err(), "weak tags never satisfy If-Match");
        h.insert(header::IF_MATCH, HeaderValue::from_static("\"r1\""));
        assert!(if_match(&h, "\"r1\"").is_ok());
        assert!(if_match(&HeaderMap::new(), "\"r1\"").is_ok());
        assert_eq!(resource_read("/api/products"), Some("products"));
        assert_eq!(resource_read("/api/products/0b0c4f5e-8a43-4a3c-9b8e-2b7c1f0c7a10"), Some("products"));
        assert_eq!(resource_read("/api/products/batch"), None);
    }
}
