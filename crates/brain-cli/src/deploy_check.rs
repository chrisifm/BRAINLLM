//! `brain deploy check --url <origin>`: what only the live server shows about the proxy in front of it. Headers set
//! twice, a proxy too old for 103 responses, a CDN the rate limiter does not know about, and a dead health endpoint.
//! Reads only; it fetches `/` and `/health`.
use std::path::Path;

/// Headers a browser takes one value of, or combines into the strictest: set in the proxy and the app, one policy breaks.
const SINGLE_VALUE: &[&str] = &["content-security-policy", "strict-transport-security", "x-frame-options", "x-content-type-options", "referrer-policy", "permissions-policy", "cross-origin-opener-policy"];
/// Response headers only a CDN or another hop adds.
const HOPS: &[&str] = &["cf-ray", "x-amz-cf-id", "x-served-by", "x-vercel-id", "fly-request-id", "via"];

pub struct Live {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl Live {
    fn values(&self, name: &str) -> Vec<&str> {
        self.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).collect()
    }
}

#[derive(Debug)]
pub struct Finding {
    pub code: &'static str,
    pub error: bool,
    pub msg: String,
    pub fix: String,
}

/// What the app declares that the responses are read against.
#[derive(Default)]
pub struct Declared {
    pub early_hints: bool,
    pub native_backend: bool,
    pub public_proxies: usize,
}

pub fn declared(root: &Path) -> Declared {
    let early_hints = std::fs::read_to_string(root.join("brain.json")).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).is_some_and(|v| v["server"]["earlyHints"] == serde_json::json!(true));
    let native_backend = root.join("server/resources").is_dir() || root.join("server/auth").is_dir() || root.join("server/routes").is_dir();
    Declared { early_hints, native_backend, public_proxies: crate::platform_config::public_proxies(root) }
}

/// nginx below 1.29 fails on a 103 response from upstream. `None` when the header hides the version.
fn nginx_below_1_29(server: &str) -> Option<bool> {
    let version = server.trim().strip_prefix("nginx/")?;
    let mut parts = version.split('.').map_while(|p| p.parse::<u32>().ok());
    Some((parts.next()?, parts.next()?) < (1, 29))
}

pub fn findings(declared: &Declared, page: &Live, health: &Live) -> Vec<Finding> {
    let mut out = Vec::new();
    for name in SINGLE_VALUE {
        let found = page.values(name);
        if found.len() > 1 {
            out.push(Finding { code: "B1857", error: false, msg: format!("`{name}` is sent {} times: {}", found.len(), found.join(" | ")), fix: "set this header in one place only; the nginx file `brain deploy config nginx` generates leaves them to the app".into() });
        }
    }
    if declared.early_hints {
        let server = page.values("server").first().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
        let (old, known) = match nginx_below_1_29(&server) {
            Some(old) => (old, true),
            None => (server == "nginx", false),
        };
        if old {
            let which = if known { format!("`{server}`, below 1.29,") } else { "nginx with a hidden version".to_string() };
            out.push(Finding { code: "B1891", error: false, msg: format!("brain.json `server.earlyHints` is on and {which} answers here; nginx below 1.29 fails on a 103 response"), fix: "leave `server.earlyHints` out (the default) or upgrade nginx to 1.29 or later".into() });
        }
    }
    if declared.native_backend && declared.public_proxies == 0 {
        if let Some(hop) = HOPS.iter().find(|name| !page.values(name).is_empty()) {
            out.push(Finding { code: "B1892", error: false, msg: format!("responses pass through another hop (`{hop}`) and `server/platform.jsbrain` declares no `trustPublicProxy`, so behind it every visitor can share one rate-limit and lockout key"), fix: "add the CDN's published ranges to `trustPublicProxy` and to `trustProxy`, and keep both as narrow as they publish".into() });
        }
    }
    if health.status != 200 {
        out.push(Finding { code: "B1893", error: true, msg: format!("`/health` answers {} here, so a load balancer takes this server out", health.status), fix: "route `/health` to the backend or the static server; the framework owns it, and `brain deploy config nginx` writes that location".into() });
    }
    out
}

fn fetch(runtime: &tokio::runtime::Runtime, client: &reqwest::Client, url: &str) -> Result<Live, String> {
    runtime.block_on(async {
        let response = client.get(url).send().await.map_err(|e| format!("{url}: {e}"))?;
        let status = response.status().as_u16();
        let headers = response.headers().iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
        Ok(Live { status, headers })
    })
}

/// Findings of a live origin, or why it could not be read.
pub fn inspect(root: Option<&Path>, origin: &str) -> Result<Vec<Finding>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).redirect(reqwest::redirect::Policy::limited(3)).build().map_err(|e| e.to_string())?;
    let page = fetch(&runtime, &client, &format!("{origin}/"))?;
    let health = fetch(&runtime, &client, &format!("{origin}/health"))?;
    Ok(findings(&root.map(declared).unwrap_or_default(), &page, &health))
}

pub fn as_json(findings: &[Finding]) -> serde_json::Value {
    serde_json::json!(findings.iter().map(|f| serde_json::json!({"code":f.code,"level":if f.error {"error"} else {"warning"},"msg":f.msg,"fix":f.fix})).collect::<Vec<_>>())
}

pub fn run(root: Option<&Path>, args: &[String]) -> (i32, String) {
    let json = args.iter().any(|a| a == "--json");
    let Some(origin) = crate::flag(args, "--url").map(|u| u.trim_end_matches('/').to_string()).filter(|u| u.starts_with("http://") || u.starts_with("https://")) else {
        return (2, "usage: brain deploy check --url <https://origin> [--json]; fix: name the origin the public reaches".into());
    };
    let found = match inspect(root, &origin) {
        Ok(found) => found,
        Err(error) => return (2, if json { serde_json::json!({"schemaVersion":1,"ok":false,"error":error}).to_string() } else { format!("error: {error}") }),
    };
    let ok = !found.iter().any(|f| f.error);
    let text = if json {
        serde_json::json!({"schemaVersion":1,"ok":ok,"origin":origin,"findings":as_json(&found)}).to_string()
    } else if found.is_empty() {
        format!("deploy check: {origin} OK")
    } else {
        found.iter().map(|f| format!("{}: {}: {}\n  fix: {}", if f.error { "error" } else { "warning" }, f.code, f.msg, f.fix)).collect::<Vec<_>>().join("\n")
    };
    (if ok { 0 } else { 1 }, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn live(status: u16, headers: &[(&str, &str)]) -> Live {
        Live { status, headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }
    fn codes(declared: &Declared, page: &Live, health: &Live) -> Vec<&'static str> {
        findings(declared, page, health).iter().map(|f| f.code).collect()
    }
    #[test]
    fn a_clean_origin_has_no_findings_and_each_fault_has_its_code() {
        let up = live(200, &[]);
        let plain = Declared::default();
        assert!(codes(&plain, &live(200, &[("x-frame-options", "DENY"), ("server", "nginx/1.24.0")]), &up).is_empty());
        assert_eq!(codes(&plain, &live(200, &[("x-frame-options", "DENY"), ("X-Frame-Options", "SAMEORIGIN")]), &up), ["B1857"]);
        assert_eq!(codes(&plain, &live(200, &[]), &live(502, &[])), ["B1893"]);
        let hints = Declared { early_hints: true, ..Default::default() };
        assert_eq!(codes(&hints, &live(200, &[("server", "nginx/1.24.0")]), &up), ["B1891"]);
        assert_eq!(codes(&hints, &live(200, &[("server", "nginx")]), &up), ["B1891"]);
        assert!(codes(&hints, &live(200, &[("server", "nginx/1.29.1")]), &up).is_empty());
        assert!(codes(&hints, &live(200, &[("server", "cloudflare")]), &up).is_empty());
        let native = Declared { native_backend: true, ..Default::default() };
        assert_eq!(codes(&native, &live(200, &[("cf-ray", "abc")]), &up), ["B1892"]);
        assert!(codes(&Declared { public_proxies: 1, ..native }, &live(200, &[("cf-ray", "abc")]), &up).is_empty());
    }
}
