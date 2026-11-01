//! `brain edge plan`: what a CDN in front of this build may keep, read from the build itself and matching the headers the
//! server sends. It changes nothing: no file, no provider, no request.
use serde_json::{json, Value};
use std::path::Path;

/// Paths that are always answered by the running app; a stored copy would serve one visitor's answer to another.
const ALWAYS_ORIGIN: &[(&str, &str)] = &[
    ("api/*", "backend data, sessions and writes"),
    ("_brain/*", "server functions and backend routes"),
    ("s/*", "signed file links"),
    ("health*", "a deploy gate must reach the app"),
    ("sw.js", "must update as soon as a build ships"),
];

fn base_of(manifest: &Value) -> String {
    let base = manifest["base"].as_str().unwrap_or("/");
    if base.ends_with('/') { base.to_string() } else { format!("{base}/") }
}

/// The plan of one built manifest, in the order an edge should match the rules.
pub fn from_manifest(manifest: &Value) -> Value {
    let base = base_of(manifest);
    let mut rules = vec![json!({"path": format!("{base}_b/*"), "action": "cache", "seconds": 31536000, "immutable": true, "reason": "content-hashed files"})];
    rules.extend(ALWAYS_ORIGIN.iter().map(|(path, reason)| json!({"path": format!("{base}{path}"), "action": "origin", "reason": reason})));
    for route in manifest["routes"].as_array().into_iter().flatten() {
        let path = format!("{}{}", base, route["path"].as_str().unwrap_or("/").trim_start_matches('/'));
        match route["cache"].as_u64().filter(|n| *n > 0) {
            Some(n) => rules.push(json!({
                "path": path, "action": "cache", "seconds": n, "staleWhileRevalidate": n, "staleIfError": n,
                "bypassWhen": ["cookie", "authorization"], "reason": "<meta cache> page",
            })),
            None => rules.push(json!({"path": path, "action": "origin", "reason": "rendered per request"})),
        }
    }
    rules.push(json!({"path": format!("{base}*"), "action": "follow-origin", "reason": "other built files say how long to keep them"}));
    json!({"schemaVersion": 1, "build": manifest["build"], "rules": rules})
}

fn text(plan: &Value) -> String {
    let mut out = String::from("Edge plan of this build (nothing was changed)\n");
    for rule in plan["rules"].as_array().into_iter().flatten() {
        let action = match (rule["action"].as_str().unwrap_or(""), rule["seconds"].as_u64()) {
            ("cache", Some(n)) => format!("cache {n} s"),
            (other, _) => other.to_string(),
        };
        let bypass = if rule["bypassWhen"].is_array() { "; never for a request with a cookie or authorization" } else { "" };
        out.push_str(&format!("  {:<28} {:<20} {}{bypass}\n", rule["path"].as_str().unwrap_or(""), action, rule["reason"].as_str().unwrap_or("")));
    }
    out
}

/// The plan of the app at `root`, from its last build.
pub fn plan(root: &Path) -> Result<Value, String> {
    if !root.join("dist").is_dir() {
        return Err("no build to read: dist/ is missing; fix: run `brain build`, then `brain edge plan`".into());
    }
    // a site with no page rendered per request has no server manifest: its plan is the same with no route in it
    let manifest: Value = match std::fs::read_to_string(root.join("dist/_server/manifest.json")) {
        Ok(source) => serde_json::from_str(&source).map_err(|_| "dist/_server/manifest.json is not valid JSON; fix: run `brain build` again".to_string())?,
        Err(_) => std::fs::read_to_string(root.join("brain.json")).ok().and_then(|config| serde_json::from_str::<Value>(&config).ok()).map_or_else(|| json!({}), |config| json!({"base": config["base"]})),
    };
    Ok(from_manifest(&manifest))
}

/// `brain edge plan [--json]`
pub fn run(root: &Path, args: &[String], json_output: bool) -> Result<(), String> {
    if args.first().map(String::as_str) != Some("plan") {
        return Err("usage: brain edge plan [--json]".into());
    }
    let plan = plan(root)?;
    if json_output { println!("{plan}") } else { print!("{}", text(&plan)) }
    Ok(())
}
