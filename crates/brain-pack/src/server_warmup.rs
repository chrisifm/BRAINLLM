//! Opt-in read-only startup data warmup. Syntax validation never fetches or runs application code.
use brain_mcp::Json;
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings { pub paths: Vec<String>, pub timeout_ms: u64, pub abort: bool }
const FIX: &str = "fix: declare up to eight exact public cached SSR paths, timeoutMs from 100 to 10000, and onError 'continue' or 'abort'; omit server.warmup to disable it";
fn error() -> String { format!("brain.json server.warmup is invalid; {FIX}") }

pub fn safe_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 256 || !path.starts_with('/') || path.contains("//") || path.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '?' | '#')) { return false; }
    let bytes = path.as_bytes();
    let mut decoded = Vec::new(); let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let Some(pair) = bytes.get(at + 1..at + 3) else { return false };
            let hex = |byte: u8| (byte as char).to_digit(16);
            let (Some(hi), Some(lo)) = (hex(pair[0]), hex(pair[1])) else { return false };
            let byte = (hi * 16 + lo) as u8;
            if matches!(byte, b'/' | b'\\') { return false; }
            decoded.push(byte); at += 3;
        } else { decoded.push(bytes[at]); at += 1; }
    }
    let Ok(decoded) = std::str::from_utf8(&decoded) else { return false };
    if decoded.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '?' | '#')) || decoded.split('/').any(|part| matches!(part, "." | "..")) { return false; }
    url::Url::parse(&format!("http://warmup.invalid{path}")).is_ok_and(|url| url.path() == path && url.query().is_none() && url.fragment().is_none())
}
pub fn parse(value: &Json) -> Result<Settings, String> {
    let Json::Obj(entries) = value else { return Err(error()) };
    if entries.iter().any(|(key,_)| !matches!(key.as_str(), "paths" | "timeoutMs" | "onError")) { return Err(error()); }
    let Some(Json::Arr(paths)) = value.get("paths") else { return Err(error()) };
    if paths.is_empty() || paths.len() > 8 { return Err(error()); }
    let paths: Vec<String> = paths.iter().map(|path| path.as_str().filter(|path| safe_path(path)).map(str::to_owned).ok_or_else(error)).collect::<Result<_,_>>()?;
    if paths.iter().collect::<BTreeSet<_>>().len() != paths.len() { return Err(error()); }
    let timeout_ms = match value.get("timeoutMs") { None => 8000, Some(Json::Num(n)) if n.fract() == 0.0 && *n >= 100.0 && *n <= 10000.0 => *n as u64, _ => return Err(error()) };
    let abort = match value.get("onError") { None => false, Some(Json::Str(value)) if value == "continue" => false, Some(Json::Str(value)) if value == "abort" => true, _ => return Err(error()) };
    Ok(Settings {paths, timeout_ms, abort})
}
pub fn read(root: &Path) -> Result<Option<Settings>, String> {
    let Ok(source) = std::fs::read_to_string(root.join("brain.json")) else { return Ok(None) };
    let Ok(config) = Json::parse(&source) else { return Ok(None) };
    config.get("server").and_then(|server|server.get("warmup")).map(parse).transpose()
}
impl Settings {
    pub fn to_json(&self) -> String {
        format!("{{\"paths\":[{}],\"timeoutMs\":{},\"onError\":\"{}\"}}", self.paths.iter().map(|path|Json::str(path).stringify()).collect::<Vec<_>>().join(","), self.timeout_ms, if self.abort {"abort"} else {"continue"})
    }
    /// Match only exact public cached SSR routes, after base and locale expansion, with a known site host.
    pub fn validate_manifest(&self, manifest: &str, site: Option<&str>) -> Result<String, String> {
        let value = Json::parse(manifest).map_err(|_|error())?;
        let site = value.get("site").and_then(Json::as_str).or(site).ok_or_else(||format!("Startup data warmup requires a configured site origin; fix: set site or PUBLIC_SITE_URL before opting in"))?;
        let parsed = url::Url::parse(site).map_err(|_|error())?;
        let host = parsed.host_str().unwrap_or("");
        if site.chars().any(|c| c.is_control() || c.is_whitespace()) || !matches!(parsed.scheme(), "http" | "https") || host.is_empty() || host.len()>253 || parsed.origin().ascii_serialization().len()>268 || !parsed.username().is_empty() || parsed.password().is_some() || parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() || matches!(host,"example.com"|"example.net"|"example.org"|"example"|"invalid"|"test") || [".example.com",".example.net",".example.org",".example",".invalid",".test"].iter().any(|suffix|host.ends_with(suffix)) { return Err(error()); }
        let base = value.get("base").and_then(Json::as_str).unwrap_or("/");
        let Some(Json::Arr(routes)) = value.get("routes") else { return Err(error()) };
        for path in &self.paths {
            let local = if base == "/" { path.clone() } else { format!("/{}",path.strip_prefix(base).ok_or_else(error)?) };
            if matches!(local.split('/').nth(1),Some("api"|"_brain"|"_r"|"_server"|"s"|"_health"|"health")) { return Err(error()); }
            let always = value.get("redirects").and_then(|redirects|redirects.get("slash")).and_then(Json::as_str)==Some("always");
            let route = routes.iter().find(|route|route.get("path").and_then(Json::as_str).is_some_and(|path| {
                let final_segment = path.rsplit('/').next().unwrap_or("");
                let canonical = if always && path != "/" && !path.ends_with('/') && !final_segment.contains('.') {format!("{path}/")} else {path.to_string()};
                canonical == local
            })).ok_or_else(error)?;
            let empty = |key: &str| match route.get(key) { None => true, Some(Json::Arr(items)) => items.is_empty(), _ => false };
            if route.get("server").and_then(Json::as_str).is_none_or(str::is_empty) || local.contains([':','*','[',']']) || !matches!(route.get("cache"),Some(Json::Num(n)) if n.is_finite() && n.fract()==0.0 && *n > 0.0) || matches!(route.get("forms"),Some(Json::Bool(true))) || !empty("guards") || !empty("middleware") { return Err(error()); }
        }
        Ok(site.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings(source: &str) -> Result<Settings,String> { parse(&Json::parse(source).unwrap()) }
    #[test]
    fn settings_are_bounded_opt_in_and_never_describe_credentials_or_writes() {
        let config=settings(r#"{"paths":["/data"]}"#).unwrap(); assert_eq!(config.timeout_ms,8000); assert!(!config.abort);
        for bad in [r#"{"paths":[]}"#,r#"{"paths":["/data","/data"]}"#,r#"{"paths":["/data"],"timeoutMs":10001}"#,r#"{"paths":["/data"],"timeoutMs":1.5}"#,r#"{"paths":["/data"],"headers":{"Cookie":"synthetic"}}"#,r#"{"paths":["/data"],"method":"POST"}"#,r#"{"paths":["/data"],"onError":"ignore"}"#] { assert!(settings(bad).is_err(),"{bad}"); }
        assert_eq!(Json::parse(&config.to_json()).unwrap().get("paths"),Some(&Json::Arr(vec![Json::str("/data")])));
    }
    #[test]
    fn paths_cannot_escape_their_exact_base_or_smuggle_query_credentials() {
        for bad in ["//host/path","/a//b","/a/../b","/a/%2e%2e/b","/a%2fb","/a%5cb","/data?token=synthetic","/data#token","/a\\b","/a%00","/a%3fkey","/a%zz"] { assert!(!safe_path(bad),"{bad}"); }
        for path in ["/","/data","/app/en/data"] { assert!(safe_path(path)); }
    }
    #[test]
    fn only_cached_public_ssr_routes_of_the_exact_base_and_locale_are_eligible() {
        let config=settings(r#"{"paths":["/app/en/data"],"onError":"abort"}"#).unwrap();
        let manifest=r#"{"base":"/app/","site":"https://public.localhost","routes":[{"path":"/en/data","server":"data.mjs","cache":60}]}"#;
        assert_eq!(config.validate_manifest(manifest,None).unwrap(),"https://public.localhost");
        for name in ["api","_brain","_r","_server","s","_health","health"] {
            let path=format!("/{name}/data");
            let blocked=settings(&format!("{{\"paths\":[{}]}}",Json::str(&path).stringify())).unwrap();
            let manifest=format!("{{\"site\":\"http://localhost\",\"routes\":[{{\"path\":{},\"server\":\"data.mjs\",\"cache\":60}}]}}",Json::str(&path).stringify());
            assert!(blocked.validate_manifest(&manifest,None).is_err());
        }
        let mut credentialed=url::Url::parse("https://public.localhost").unwrap();
        credentialed.set_username("synthetic-user").unwrap(); credentialed.set_password(Some("synthetic-placeholder")).unwrap();
        for bad in [manifest.replace("/en/data","/fr/data"),manifest.replace("\"cache\":60","\"cache\":0"),manifest.replace("\"cache\":60","\"cache\":60,\"guards\":[\"guard.mjs\"]"),manifest.replace("\"cache\":60","\"cache\":60,\"middleware\":[\"middleware.mjs\"]"),manifest.replace("\"server\":\"data.mjs\"","\"server\":null"),manifest.replace("https://public.localhost",credentialed.as_str()),manifest.replace("https://public.localhost","https://example.com")] { assert!(config.validate_manifest(&bad,None).is_err()); }
    }
}
