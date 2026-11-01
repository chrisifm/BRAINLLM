//! Origin declarations and origin refusals (B0740, B0743, B0744, B0748): the single place that says what a valid origin
//! looks like and what the operator should change. Nothing here widens a policy: refused stays refused. Production
//! responses never carry the allowlist; the server log does.
use std::sync::atomic::{AtomicU64, Ordering};

pub const PLATFORM_KEY: &str = "server/platform.jsbrain cors.origins";
pub const BRAIN_JSON_KEY: &str = "brain.json backend.origins";
pub const SOCKET_KEY: &str = "brain.json backend.sockets.allowedOrigins";
/// A refused origin is attacker-controlled text: bounded before it reaches a log or a dev response.
const SEEN_BYTES: usize = 256;
const LOGS_PER_MINUTE: u64 = 16;

/// Why a declared origin can never match what a browser sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub reason: &'static str,
    /// The exact replacement when the correction is unique (a wildcard has none: the author lists real origins).
    pub replacement: Option<String>,
    /// True when the current backend already refuses to start with this value; false for an entry that only never matches.
    pub fatal: bool,
}

/// `None` when `raw` is exactly the serialized origin a browser sends.
pub fn problem(raw: &str) -> Option<Problem> {
    let refuse = |reason, replacement| Some(Problem { reason, replacement, fatal: true });
    if raw.contains('*') {
        return refuse("a wildcard cannot be combined with credentials, and every allowed origin receives credentialed responses", None);
    }
    if raw.is_empty() || raw.len() > 2048 || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return refuse("an origin is a short string without spaces or control characters", None);
    }
    if !raw.contains("://") {
        let guess = format!("https://{}", raw.trim_end_matches('/'));
        let usable = !raw.contains('@') && url::Url::parse(&guess).is_ok_and(|u| u.host_str().is_some_and(|h| h.contains('.') || h == "localhost"));
        return refuse("the scheme is missing", usable.then_some(guess));
    }
    let Ok(url) = url::Url::parse(raw) else {
        return refuse("it does not parse as an http(s) origin", None);
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return refuse("only http and https origins exist for browser requests", None);
    }
    let clean = url.origin().ascii_serialization();
    if !url.username().is_empty() || url.password().is_some() {
        return refuse("origins carry no credentials", Some(clean));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return refuse("origins carry no path, query or fragment", Some(clean));
    }
    if raw == clean {
        return None;
    }
    let reason = if raw.ends_with('/') && raw.trim_end_matches('/') == clean {
        "browsers send origins without a trailing slash"
    } else {
        "browsers send lowercase hosts and omit the default port"
    };
    Some(Problem { reason, replacement: Some(clean), fatal: false })
}

/// One sentence with the seen value, the reason and the exact fix for `file key`.
pub fn declaration_message(raw: &str, key: &str) -> String {
    let shown = seen(raw);
    match problem(raw) {
        None => format!("B0744: {key} has the origin {shown:?}"),
        Some(p) => match p.replacement {
            Some(fixed) => format!("B0744: {key} has the origin {shown:?}: {}; fix: write {fixed:?}. Run brain explain B0744", p.reason),
            None => format!("B0744: {key} has the origin {shown:?}: {}; fix: list each exact origin such as \"https://shop.example\". Run brain explain B0744", p.reason),
        },
    }
}

/// B0748: the configured auth public URL and the `--origin` values are both named, so the operator sees the mismatch.
/// `origin` is `None` when the URL has no usable origin at all.
pub fn auth_public_url_message(url: &str, origin: Option<&str>, allowed: &[String]) -> String {
    // Only scheme, host and path are evidence: a query or fragment may hold a token.
    let url = seen(url.split(['?', '#']).next().unwrap_or(""));
    match origin {
        Some(origin) => format!(
            "B0748: Auth public base URL {url:?} resolves to {origin:?}, which is outside the allowed origins {allowed:?}; fix: pass --origin {origin} (one frontend) or list every frontend in {BRAIN_JSON_KEY}, or set PUBLIC_SITE_URL to a URL on an allowed origin; never a wildcard. Run brain explain B0748"
        ),
        None => format!(
            "B0748: Auth public base URL {url:?} is not a valid HTTP(S) URL; allowed origins are {allowed:?}; fix: set PUBLIC_SITE_URL to a full URL on one of them. Run brain explain B0748"
        ),
    }
}

/// Printable and bounded; a value that needed more cleaning than this is not useful evidence anyway.
pub fn seen(raw: &str) -> String {
    let start = raw.find("://").map_or(0, |at| at + 3);
    let authority = raw[start..].find(['/', '?', '#']).map_or(raw.len(), |at| start + at);
    let raw = match raw[start..authority].rfind('@') {
        // Userinfo may be a real credential pasted by mistake: its value is never evidence worth printing.
        Some(at) => format!("{}***{}", &raw[..start], &raw[start + at..]),
        None => raw.to_string(),
    };
    raw.chars().filter(|c| !c.is_control()).take(SEEN_BYTES).collect()
}

/// The closest allowed origin when the refusal is a near miss: same host on another scheme or port.
fn near_miss<'a>(origin: &str, allowed: &'a [String]) -> Option<&'a String> {
    let host = |value: &str| url::Url::parse(value).ok().and_then(|u| u.host_str().map(str::to_string));
    let wanted = host(origin)?;
    allowed.iter().find(|candidate| host(candidate).as_deref() == Some(wanted.as_str()))
}

/// The exact change for a refused request. It names a value to add, never a wildcard, and only when the sender is real.
pub fn refusal_fix(origin: Option<&str>, allowed: &[String]) -> String {
    let first = allowed.first().map_or("the site origin", String::as_str);
    let Some(origin) = origin else {
        return format!("This request has no Origin header. Browsers add it to mutations; a script must send `Origin: {first}` or use a project API key");
    };
    if origin == "null" {
        return "Origin `null` comes from a sandboxed frame or a file:// page and cannot be allowed. Serve the page from an http(s) origin".into();
    }
    if let Some(close) = near_miss(origin, allowed) {
        return format!("{origin} is close to the allowed {close}: open the page at {close}, or add exactly {origin:?} to {PLATFORM_KEY} if that address is really yours");
    }
    format!("If {origin} is your own frontend, add exactly {origin:?} to {PLATFORM_KEY} (or {BRAIN_JSON_KEY}). Do not add a wildcard")
}

/// The JSON detail of a refusal: full in server logs and development responses, absent from production responses.
pub fn detail(code: &str, origin: Option<&str>, allowed: &[String]) -> serde_json::Value {
    serde_json::json!({
        "brainCode": code,
        "seenOrigin": origin.map(seen),
        "allowedOrigins": allowed,
        "config": [PLATFORM_KEY, BRAIN_JSON_KEY, "PUBLIC_SITE_URL or --origin"],
        "fix": refusal_fix(origin, allowed),
        "explain": format!("brain explain {code}"),
    })
}

/// Bounded per minute: a refusal is reachable by anyone, a log line must not be cheaper than the request.
fn budget() -> bool {
    static WINDOW: AtomicU64 = AtomicU64::new(0);
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let minute = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 60);
    if WINDOW.swap(minute, Ordering::Relaxed) != minute {
        COUNT.store(0, Ordering::Relaxed);
    }
    COUNT.fetch_add(1, Ordering::Relaxed) < LOGS_PER_MINUTE
}

pub(crate) fn log_refusal(code: &str, phase: &str, method: &str, path: &str, origin: Option<&str>, allowed: &[String]) {
    if !budget() {
        return;
    }
    crate::log_line!(
        "{}",
        serde_json::json!({
            "schemaVersion": 1,
            "event": "origin-refused",
            "phase": phase,
            "method": method,
            "path": seen(path),
            "rid": crate::http::request_reference(),
            "detail": detail(code, origin, allowed),
        })
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn credentials_in_a_value_never_reach_a_message() {
        let allowed = vec!["https://shop.example.test".to_string()];
        for message in [
            declaration_message("https://secret:password@shop.example.test", BRAIN_JSON_KEY),
            declaration_message("secret:password@shop.example.test", BRAIN_JSON_KEY),
            auth_public_url_message("https://secret:password@blog.example.test/a?token=password#password", Some("https://blog.example.test"), &allowed),
            auth_public_url_message("secret:password@not a url", None, &allowed),
            seen("https://secret:password@shop.example.test"),
        ] {
            assert!(!message.contains("password") && !message.contains("secret"), "{message}");
        }
        assert_eq!(seen("/account/a@b"), "/account/a@b");
        assert_eq!(seen("https://shop.example.test"), "https://shop.example.test");
    }
    #[test]
    fn auth_public_url_refusal_names_the_url_the_origins_and_the_fix() {
        let allowed = vec!["https://admin.example.test".to_string(), "https://shop.example.test".to_string()];
        let message = auth_public_url_message("https://blog.example.test/app", Some("https://blog.example.test"), &allowed);
        for needle in ["B0748", "https://blog.example.test/app", "https://admin.example.test", "https://shop.example.test", "--origin https://blog.example.test", "PUBLIC_SITE_URL", "brain explain B0748"] {
            assert!(message.contains(needle), "{needle} missing from {message}");
        }
        let invalid = auth_public_url_message("not a url", None, &allowed);
        assert!(invalid.contains("B0748") && invalid.contains("not a url") && invalid.contains("https://shop.example.test"), "{invalid}");
    }
    use super::*;

    #[test]
    fn canonical_origins_have_no_problem() {
        for good in ["https://shop.example", "http://127.0.0.1:5173", "https://admin.shop.example:8443", "http://localhost:3000", "http://[::1]:5182"] {
            assert_eq!(problem(good), None, "{good}");
        }
    }

    #[test]
    fn each_malformed_shape_names_its_reason_and_exact_replacement() {
        let cases = [
            ("https://shop.example/", "trailing slash", Some("https://shop.example"), false),
            ("https://shop.example/admin", "no path", Some("https://shop.example"), true),
            ("shop.example", "scheme is missing", Some("https://shop.example"), true),
            ("https://user:pw@shop.example", "no credentials", Some("https://shop.example"), true),
            ("https://SHOP.example", "lowercase", Some("https://shop.example"), false),
            ("https://shop.example:443", "default port", Some("https://shop.example"), false),
            ("*", "wildcard", None, true),
            ("https://*.shop.example", "wildcard", None, true),
            ("ftp://shop.example", "http and https", None, true),
        ];
        for (raw, reason, replacement, fatal) in cases {
            let found = problem(raw).unwrap_or_else(|| panic!("{raw} must be reported"));
            assert!(found.reason.contains(reason), "{raw}: {}", found.reason);
            assert_eq!(found.replacement.as_deref(), replacement, "{raw}");
            assert_eq!(found.fatal, fatal, "{raw}");
        }
    }

    #[test]
    fn refusal_fix_never_suggests_a_wildcard_and_tells_near_misses_apart() {
        let allowed = vec!["https://shop.example".to_string()];
        let other = refusal_fix(Some("https://admin.shop.example"), &allowed);
        assert!(other.contains("\"https://admin.shop.example\"") && other.contains(PLATFORM_KEY), "{other}");
        assert!(other.contains("Do not add a wildcard"), "{other}");
        let port = refusal_fix(Some("https://shop.example:8443"), &allowed);
        assert!(port.contains("close to the allowed https://shop.example"), "{port}");
        let scheme = refusal_fix(Some("http://shop.example"), &allowed);
        assert!(scheme.contains("close to the allowed"), "{scheme}");
        assert!(refusal_fix(None, &allowed).contains("no Origin header"));
        assert!(refusal_fix(Some("null"), &allowed).contains("sandboxed"));
    }

    #[test]
    fn a_refused_origin_is_bounded_before_it_is_echoed() {
        let long = format!("https://{}.example\u{7}", "a".repeat(4000));
        let bounded = seen(&long);
        assert_eq!(bounded.len(), SEEN_BYTES);
        assert!(!bounded.contains('\u{7}'));
    }
}
