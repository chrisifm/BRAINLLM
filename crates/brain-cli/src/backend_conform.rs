//! Anonymous, read-only observations of the native JSON response contract, never a write-path proof.
use reqwest::{Method, Url, Client};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::{Duration, Instant}};
const BODY: u64 = 256 * 1024;

struct Options { origin: Url, paths: Vec<String> }
impl Options {
    fn parse(args: &[String]) -> Result<Self, &'static str> {
        let (mut origin, mut paths, mut seen_json) = (None, BTreeSet::new(), false);
        let mut i = 0;
        while i < args.len() {
            let (key, inline) = args[i].split_once('=').map_or((args[i].as_str(), None), |(key, value)| (key, Some(value)));
            if key == "--json" && inline.is_none() && !seen_json { seen_json = true; i += 1; continue; }
            if !matches!(key, "--url" | "--path") { return Err("use --url <origin>, at most eight --path /api/example options, and --json"); }
            let value = if let Some(value) = inline { value } else {
                i += 1; args.get(i).filter(|v| !v.starts_with('-')).map(String::as_str).ok_or("option requires a value")?
            };
            if key == "--url" {
                if origin.is_some() { return Err("--url must occur once"); }
                let url = Url::parse(value).map_err(|_| "--url must be an HTTP(S) origin without credentials, query or fragment")?;
                if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty()
                    || url.password().is_some() || url.path() != "/" || url.query().is_some() || url.fragment().is_some()
                    || value.bytes().any(|c| c <= b' ' || c == 127) {
                    return Err("--url must be an HTTP(S) origin without credentials, path, query or fragment");
                }
                origin = Some(url);
            } else {
                if !path(value) { return Err("--path must be a concrete same-origin path without query, fragment, traversal, escapes or credentials"); }
                if !paths.insert(value.to_owned()) || paths.len() > 8 { return Err("supply at most eight distinct --path options"); }
            }
            i += 1;
        }
        if paths.is_empty() { paths.extend(["/health".to_owned(), "/api/brain_conformance_missing".to_owned()]); }
        Ok(Self { origin: origin.ok_or("--url is required; no request is made without an explicit target")?, paths: paths.into_iter().collect() })
    }
}
fn path(value: &str) -> bool {
    value.starts_with('/') && value.len() <= 512 && !value.contains("//")
        && value.bytes().all(|c| c.is_ascii_alphanumeric() || b"/-_.~".contains(&c))
        && !value.split('/').any(|part| matches!(part, "." | ".."))
}
fn json_mime(value: &str) -> bool {
    let mime = value.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    mime == "application/json" || mime.starts_with("application/") && mime.ends_with("+json")
}
async fn observe(client: &Client, origin: &Url, path: &str, method: Method, deadline: Instant) -> Result<Value, &'static str> {
    let remaining = deadline.checked_duration_since(Instant::now()).ok_or("total observation deadline exceeded")?;
    let url = origin.join(path).map_err(|_| "cannot resolve the selected path")?;
    let mut response = client.request(method.clone(), url).header("accept", "application/json")
        .header("accept-encoding", "identity").timeout(remaining.min(Duration::from_secs(3))).send().await
        .map_err(|_| "HTTP observation failed or timed out; no response values were recorded")?;
    let status = response.status().as_u16();
    let content_type = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("");
    let is_json = json_mime(content_type);
    let mut findings = Vec::new();
    if (300..400).contains(&status) { findings.push("redirect-not-followed"); }
    if matches!(status, 405) && !response.headers().contains_key("allow") { findings.push("method-rejection-missing-allow"); }
    if status == 429 && !response.headers().contains_key("retry-after") { findings.push("rate-limit-missing-retry-after"); }
    if method == Method::HEAD {
        return Ok(json!({"path":path,"method":"HEAD","status":status,"jsonContentType":is_json,"findings":findings}));
    }
    if response.content_length().is_some_and(|bytes| bytes > BODY) { return Err("response exceeds the 256 KiB observation bound; this response remains unverified"); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "cannot read bounded response; this response remains unverified")? {
        if bytes.len().saturating_add(chunk.len()) as u64 > BODY { return Err("response exceeds the 256 KiB observation bound; this response remains unverified"); }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() as u64 > BODY { return Err("response exceeds the 256 KiB observation bound; this response remains unverified"); }
    let mut has_stable_error_code = false;
    if !matches!(status, 204 | 205 | 304) {
        if !is_json { findings.push("explicit-json-request-received-non-json"); }
        let body = serde_json::from_slice::<Value>(&bytes);
        if let Ok(body) = body {
            if status >= 400 {
                let error = body.get("error");
                let stable_code = error.and_then(|v| v.get("code")).and_then(Value::as_str)
                    .filter(|v| !v.is_empty() && v.len() <= 64 && v.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_'));
                if stable_code.is_none() || !error.and_then(|v| v.get("message")).is_some_and(Value::is_string) {
                    findings.push("json-error-missing-stable-code-or-message");
                }
                has_stable_error_code = stable_code.is_some();
            }
        } else { findings.push("unreadable-json-body"); }
    }
    Ok(json!({"path":path,"method":"GET","status":status,"jsonContentType":is_json,"hasStableErrorCode":has_stable_error_code,"findings":findings}))
}
async fn report(args: &[String]) -> Result<(i32, Value), &'static str> {
        let options = Options::parse(args)?;
        let client = crate::http_observer::client(Client::builder()).no_proxy().retry(reqwest::retry::never()).redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2)).timeout(Duration::from_secs(3)).build().map_err(|_| "cannot construct HTTP observation client")?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut observations = Vec::new();
        for path in &options.paths {
            let mut get = observe(&client, &options.origin, path, Method::GET, deadline).await?;
            let mut head = observe(&client, &options.origin, path, Method::HEAD, deadline).await?;
            // This checks a paired observation, not a claim that concurrent mutable data cannot change.
            if get["status"] != head["status"] { head["findings"].as_array_mut().unwrap().push(json!("head-get-status-mismatch")); }
            if get["jsonContentType"] != head["jsonContentType"] { head["findings"].as_array_mut().unwrap().push(json!("head-get-content-type-mismatch")); }
            get["bodyValuesCaptured"] = json!(false);
            observations.extend([get, head]);
        }
        let ok = observations.iter().all(|row| row["findings"].as_array().is_some_and(Vec::is_empty));
        Ok((if ok {0} else {1}, json!({"schemaVersion":1,"ok":ok,"scope":"observed anonymous GET/HEAD JSON response contract; not write-path, authorization, live-app adoption or full HTTP standards proof","accept":"application/json","credentialsSent":false,"redirectsFollowed":false,"observations":observations})))
}
pub fn run(args: &[String]) -> (i32, String) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(_) => return (2, json!({"schemaVersion":1,"ok":false,"error":"cannot construct observation runtime"}).to_string()),
    };
    let result = runtime.block_on(report(args));
    runtime.shutdown_timeout(Duration::from_millis(100));
    match result {
        Ok((code, report)) => (code, report.to_string()),
        Err(message) => (2, json!({"schemaVersion":1,"ok":false,"error":message,"fix":"use an explicit origin and concrete read-only JSON paths; resolve the reported input or observation bound before relying on this report"}).to_string()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refused_targets_and_paths_never_reach_the_network() {
        let mut credentialed = Url::parse("https://example.test").unwrap();
        credentialed.set_username("synthetic").unwrap();
        credentialed.set_password(Some("synthetic")).unwrap();
        assert!(Options::parse(&["--url".into(), credentialed.to_string()]).is_err());
        for origin in ["https://example.test/path", "https://example.test/?secret=x", "file:///tmp/example", "https://example.test/\n"] {
            assert!(Options::parse(&["--url".into(), origin.into()]).is_err());
        }
        for bad in ["//foreign.test", "/../private", "/api/%2fother", "/api/a?secret=x", "/api/a#fragment", "/api/\\other"] { assert!(!path(bad)); }
        assert!(path("/api/items/example-id"));
    }
}
