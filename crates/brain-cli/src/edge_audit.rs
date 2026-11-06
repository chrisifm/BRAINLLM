//! Bounded anonymous HTTP observations. Cache status is evidence, never a cold-request verdict.
use serde_json::{Value, json};
use std::{collections::BTreeSet, io::Read, path::Path, time::Duration};

pub(super) struct Options {
    pub command: String,
    pub origin: Option<String>,
    pub provider: Option<String>,
    pub request: Option<String>,
}
impl Options {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut out = Self {
            command: args.first().cloned().unwrap_or_default(),
            origin: None,
            provider: None,
            request: None,
        };
        let mut i = 1;
        while i < args.len() {
            let (name, inline) = args[i]
                .split_once('=')
                .map_or((args[i].as_str(), None), |(a, b)| (a, Some(b)));
            if name == "--json" && inline.is_none() {
                i += 1;
                continue;
            }
            let slot = match name {
                "--url" => &mut out.origin,
                "--provider" => &mut out.provider,
                "--request" => &mut out.request,
                _ => {
                    return Err(
                        "unknown edge option; fix: use --url, --provider, --request or --json"
                            .into(),
                    );
                }
            };
            if slot.is_some() {
                return Err("edge options must be supplied once".into());
            }
            let value = if let Some(value) = inline {
                value.to_string()
            } else {
                i += 1;
                args.get(i)
                    .filter(|v| !v.starts_with('-'))
                    .cloned()
                    .ok_or("edge option requires a value")?
            };
            *slot = Some(value);
            i += 1;
        }
        if !matches!(out.command.as_str(), "plan" | "check" | "audit" | "apply")
            || out.provider.as_deref().is_some_and(|v| v != "cloudflare")
            || out.request.is_some() && out.command != "apply"
        {
            return Err("usage: brain edge plan|check|audit|apply [--url <origin>] [--provider cloudflare] [--request <file>] [--json]".into());
        }
        Ok(out)
    }
}
pub(super) fn origin(input: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(input).map_err(|_| "edge --url must be an http(s) origin")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("edge --url must contain only scheme, host and optional port, without credentials, path or query".into());
    }
    Ok(url)
}
pub(super) fn read(path: &Path, limit: u64) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "cannot read edge input file")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit {
        return Err("edge input must be a bounded regular file".into());
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|_| "cannot open edge input file")?
        .take(limit + 1)
        .read_to_string(&mut text)
        .map_err(|_| "edge input must be UTF-8")?;
    if text.len() as u64 > limit {
        return Err("edge input exceeds its read limit".into());
    }
    Ok(text)
}
/// Conservative URL paths: no encoded traversal, query, fragment, expression syntax or reserved endpoints.
pub(super) fn safe_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 512
        && !path.contains("//")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.~".contains(&b))
        && !path.split('/').any(|s| matches!(s, "." | ".."))
}
pub(super) fn protected(path: &str) -> bool {
    path.split('/').any(|s| {
        matches!(s, "api" | "_r" | "_brain" | "_server" | "s")
            || s.starts_with("health")
            || s == "sw.js"
    })
}
fn files(
    directory: &Path,
    prefix: &str,
    out: &mut BTreeSet<String>,
    depth: usize,
    visited: &mut usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("edge asset inventory exceeds 16 directory levels".into());
    }
    if out.len() >= 1024 {
        return Err("edge asset inventory exceeds 1024 files; fix: reduce the build or audit a smaller build".into());
    }
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory).map_err(|_| "cannot read built asset directory")? {
        *visited += 1;
        if *visited > 4096 {
            return Err("edge asset inventory exceeds 4096 directory entries".into());
        }
        let entry = entry.map_err(|_| "cannot read built asset entry")?;
        let kind = entry
            .file_type()
            .map_err(|_| "cannot read built asset type")?;
        if kind.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let path = format!("{prefix}/{name}");
        if kind.is_dir() {
            files(&entry.path(), &path, out, depth + 1, visited)?;
        } else if kind.is_file()
            && safe_path(&path)
            && !protected(&path)
            && !path.ends_with(".br")
            && !path.ends_with(".gz")
            && !path.ends_with(".zst")
        {
            out.insert(path);
        }
        if out.len() > 1024 {
            return Err("edge asset inventory exceeds 1024 files".into());
        }
    }
    Ok(())
}
pub(super) fn assets(root: &Path, base: &str) -> Result<Vec<String>, String> {
    let mut out = BTreeSet::new();
    let policy = root.join("dist/_headers");
    if !policy.exists() {
        return Ok(Vec::new());
    }
    let mut path = "";
    let source = read(&policy, 2 * 1024 * 1024)?;
    for line in source.lines() {
        if line.starts_with('/') {
            path = line.trim();
        } else if line
            .trim()
            .to_ascii_lowercase()
            .starts_with("cache-control:")
            && line
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("immutable"))
        {
            if path == "/_b/*" {
                if std::fs::symlink_metadata(root.join("dist/_b"))
                    .is_ok_and(|m| m.file_type().is_symlink())
                {
                    return Err("built asset directory must not be a symlink".into());
                }
                files(&root.join("dist/_b"), "/_b", &mut out, 0, &mut 0)?;
            } else if safe_path(path) && !protected(path) {
                let file = root.join("dist").join(path.trim_start_matches('/'));
                let real = file.canonicalize().ok();
                let dist = root.join("dist").canonicalize().ok();
                if real
                    .as_ref()
                    .zip(dist.as_ref())
                    .is_some_and(|(f, d)| f.starts_with(d) && f.is_file())
                {
                    out.insert(path.to_string());
                }
            }
        }
    }
    if out.len() > 1024 {
        return Err("edge asset inventory exceeds 1024 files".into());
    }
    Ok(out
        .into_iter()
        .map(|path| format!("{}{}", base, path.trim_start_matches('/')))
        .collect())
}
fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get_all(name)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join(", ")
}
fn tokens(value: &str) -> BTreeSet<String> {
    value
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .collect()
}
fn ttl(value: &str, directive: &str) -> Option<u64> {
    value.split(',').find_map(|s| {
        let (k, v) = s.trim().split_once('=')?;
        k.eq_ignore_ascii_case(directive)
            .then(|| v.trim_matches('"').parse().ok())
            .flatten()
    })
}
fn assess(kind: &str, seconds: u64, observations: &[Value]) -> Vec<&'static str> {
    let mut errors = BTreeSet::new();
    for item in observations {
        if item["status"]
            .as_u64()
            .is_none_or(|s| !(200..300).contains(&s))
        {
            errors.insert("unexpected-http-status");
        }
        let cc = item["cacheControl"].as_str().unwrap_or("");
        let directives = tokens(cc);
        let vary = tokens(item["vary"].as_str().unwrap_or(""));
        let ct = item["contentType"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase();
        let private = directives.contains("private")
            || directives.contains("no-store")
            || item["setCookie"] == true;
        let hit = matches!(
            item["cacheStatus"].as_str().unwrap_or(""),
            "HIT" | "STALE" | "UPDATING"
        ) || item["age"]
            .as_str()
            .unwrap_or("")
            .parse::<u64>()
            .is_ok_and(|n| n > 0);
        if private && hit {
            errors.insert("private-response-shared-cache");
        }
        if kind == "immutable" {
            if private
                || !directives.contains("immutable")
                || ttl(cc, "max-age").unwrap_or(0) < 31536000
            {
                errors.insert("immutable-cache-control-mismatch");
            }
            if ct.contains("html") || ct.contains("markdown") {
                errors.insert("asset-returned-document");
            }
        } else {
            if item["accept"] == "text/html" && !ct.contains("text/html") {
                errors.insert("html-representation-mismatch");
            }
            if item["accept"] == "text/markdown" && !ct.contains("text/markdown") {
                errors.insert("markdown-representation-mismatch");
            }
            if !vary.contains("accept") && !vary.contains("*") {
                errors.insert("missing-vary-accept");
            }
            if seconds > 0
                && !private
                && ttl(cc, "s-maxage")
                    .or_else(|| ttl(cc, "max-age"))
                    .unwrap_or(0)
                    != seconds
            {
                errors.insert("declared-page-cache-mismatch");
            }
            if seconds == 0
                && !private
                && !directives.contains("no-cache")
                && ttl(cc, "max-age") != Some(0)
            {
                errors.insert("undeclared-page-cache");
            }
        }
    }
    errors.into_iter().collect()
}
pub(super) fn run(plan: &Value, input: &str) -> Result<Value, String> {
    let origin = origin(input)?;
    let runtime = crate::http_observer::Runtime::new()?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), observe(plan, origin))
            .await
            .map_err(|_| "edge audit exceeded its 30 second total deadline".to_string())?
    })
}
async fn observe(plan: &Value, origin: reqwest::Url) -> Result<Value, String> {
    let client = crate::http_observer::client(reqwest::Client::builder())
        .no_proxy()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .build()
        .map_err(|_| "cannot create edge HTTP client")?;
    let rules = plan["rules"].as_array().ok_or("edge plan has no rules")?;
    let pages: Vec<_> = rules
        .iter()
        .filter(|r| {
            r["immutable"] != true
                && matches!(r["action"].as_str(), Some("cache" | "origin"))
                && r["path"]
                    .as_str()
                    .is_some_and(|p| safe_path(p) && !protected(p))
        })
        .collect();
    let assets: Vec<_> = plan["immutableAssets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut targets: Vec<_> = pages
        .iter()
        .take(8)
        .map(|r| {
            (
                r["path"].as_str().unwrap().to_string(),
                "page",
                r["seconds"].as_u64().unwrap_or(0),
            )
        })
        .collect();
    if targets.is_empty() {
        targets.push((plan["base"].as_str().unwrap_or("/").into(), "page", 0));
    }
    targets.extend(
        assets
            .iter()
            .take(4)
            .map(|p| (p.to_string(), "immutable", 31536000)),
    );
    let mut results = Vec::new();
    let mut ok = true;
    for (path, kind, seconds) in targets {
        if !safe_path(&path) || protected(&path) {
            continue;
        }
        let mut observations = Vec::new();
        let accepts: &[&str] = if kind == "page" {
            &["text/html", "text/markdown"]
        } else {
            &["*/*"]
        };
        for accept in accepts {
            for repetition in 1..=2 {
                let url = origin.join(&path).map_err(|_| "invalid built edge path")?;
                let response = client
                    .get(url)
                    .header("Accept", *accept)
                    .header("Accept-Encoding", "identity")
                    .header("User-Agent", "BRAINLLM-edge-audit")
                    .send()
                    .await
                    .map_err(|_| "edge HTTP request failed; fix: check the origin is reachable")?;
                observations.push(json!({"accept":accept,"repetition":repetition,"status":response.status().as_u16(),"cacheControl":header(&response,"cache-control"),"cacheStatus":header(&response,"cf-cache-status"),"age":header(&response,"age"),"vary":header(&response,"vary"),"contentType":header(&response,"content-type"),"setCookie":response.headers().contains_key("set-cookie")}));
                // Do not retain bodies or cookies; dropping the response cancels any unbounded body.
            }
        }
        let errors = assess(kind, seconds, &observations);
        ok &= errors.is_empty();
        let cached = observations.iter().any(|r| {
            r["cacheStatus"] == "HIT"
                || r["age"]
                    .as_str()
                    .is_some_and(|a| a.parse::<u64>().is_ok_and(|n| n > 0))
        });
        results.push(json!({"path":path,"kind":kind,"declaredSeconds":seconds,"observations":observations,"errors":errors,"cacheObservation":if cached {"shared-cache-observed"} else {"not-observed-in-two-requests"}}));
    }
    Ok(
        json!({"schemaVersion":1,"ok":ok,"origin":origin.origin().ascii_serialization(),"coverage":{"pageLimit":8,"assetLimit":4,"declaredPages":pages.len(),"declaredAssets":assets.len(),"fallbackHomeSampled":pages.is_empty(),"routePatternsRequiringParametersTested":false,"headersOnly":true,"requestsPerRepresentation":2,"anonymousOnly":true,"privateAuthenticatedPathsTested":false,"providerConfigurationVerified":false,"cacheHitRequired":false},"results":results}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_rejects_credentials_and_paths() {
        for value in [
            "https://user@example.com",
            "https://example.com/a",
            "https://example.com?secret=x",
            "file:///tmp/a",
        ] {
            assert!(origin(value).is_err());
        }
        assert!(origin("http://127.0.0.1:1234").is_ok());
    }
    #[test]
    fn cold_cache_is_not_a_failure_but_missing_vary_is() {
        let mut item = json!({"status":200,"cacheControl":"public, s-maxage=60","cacheStatus":"DYNAMIC","contentType":"text/html","accept":"text/html","vary":"Accept, Accept-Encoding","setCookie":false});
        assert!(assess("page", 60, &[item.clone(), item.clone()]).is_empty());
        item["vary"] = json!("Accept-Encoding");
        assert_eq!(assess("page", 60, &[item]), vec!["missing-vary-accept"]);
    }
    #[test]
    fn private_hit_is_a_failure_without_recording_cookie_values() {
        let item = json!({"status":200,"cacheControl":"private, no-store","cacheStatus":"HIT","contentType":"text/html","accept":"text/html","vary":"Accept","setCookie":true});
        assert_eq!(
            assess("page", 0, &[item]),
            vec!["private-response-shared-cache"]
        );
    }
    #[test]
    fn protected_paths_and_expression_characters_are_never_assets() {
        for p in [
            "/api/file.js",
            "/app/_r/x",
            "/s/abc",
            "/healthz",
            "/app/sw.js",
        ] {
            assert!(protected(p));
        }
        for p in ["//other/path", "/a/../b", "/a%2fb", "/a\"b"] {
            assert!(!safe_path(p));
        }
    }
}
