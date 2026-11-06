//! `brain edge plan`: what a CDN in front of this build may keep, read from the build itself and matching the headers the
//! server sends. It changes nothing: no file, no provider, no request.
use serde_json::{Value, json};
use std::path::Path;
#[path = "edge_audit.rs"]
mod audit;
#[path = "edge_cloudflare.rs"]
mod cloudflare;

/// Paths that are always answered by the running app; a stored copy would serve one visitor's answer to another.
const ALWAYS_ORIGIN: &[(&str, &str)] = &[
    ("api/*", "backend data, sessions and writes"),
    ("_brain/*", "server functions and backend routes"),
    ("_r/*", "server function calls"),
    ("s/*", "signed file links"),
    ("health*", "a deploy gate must reach the app"),
    ("sw.js", "must update as soon as a build ships"),
];

fn base_of(manifest: &Value) -> String {
    let base = manifest["base"].as_str().unwrap_or("/");
    if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    }
}

/// The plan of one built manifest, in the order an edge should match the rules.
pub fn from_manifest(manifest: &Value) -> Value {
    let base = base_of(manifest);
    let mut rules = vec![
        json!({"path": format!("{base}_b/*"), "action": "cache", "seconds": 31536000, "immutable": true, "reason": "content-hashed files"}),
    ];
    rules.extend(ALWAYS_ORIGIN.iter().map(|(path, reason)| json!({"path": format!("{base}{path}"), "action": "origin", "reason": reason})));
    for route in manifest["routes"].as_array().into_iter().flatten() {
        let path = format!(
            "{}{}",
            base,
            route["path"]
                .as_str()
                .unwrap_or("/")
                .trim_start_matches('/')
        );
        match route["cache"].as_u64().filter(|n| *n > 0) {
            Some(n) => rules.push(json!({
                "path": path, "action": "cache", "seconds": n, "staleWhileRevalidate": n, "staleIfError": n,
                "bypassWhen": ["cookie", "authorization"], "reason": "<meta cache> page",
                "requiresCacheKey": ["Accept"], "providerSafeDefault": "bypass-unless-Accept-cache-key-proven",
            })),
            None => rules.push(json!({"path": path, "action": "origin", "reason": "rendered per request"})),
        }
    }
    rules.push(json!({"path": format!("{base}*"), "action": "follow-origin", "reason": "other built files say how long to keep them"}));
    json!({"schemaVersion": 1, "build": manifest["build"], "base":base,"rules": rules})
}

fn text(plan: &Value) -> String {
    let mut out = String::from("Edge plan of this build (nothing was changed)\n");
    for rule in plan["rules"].as_array().into_iter().flatten() {
        let action = match (
            rule["action"].as_str().unwrap_or(""),
            rule["seconds"].as_u64(),
        ) {
            ("cache", Some(n)) => format!("cache {n} s"),
            (other, _) => other.to_string(),
        };
        let bypass = if rule["bypassWhen"].is_array() {
            "; never for a request with a cookie or authorization"
        } else {
            ""
        };
        out.push_str(&format!(
            "  {:<28} {:<20} {}{bypass}\n",
            rule["path"].as_str().unwrap_or(""),
            action,
            rule["reason"].as_str().unwrap_or("")
        ));
    }
    out
}

/// The plan of the app at `root`, from its last build.
pub fn plan(root: &Path) -> Result<Value, String> {
    if !root.join("dist").is_dir() {
        return Err(
            "no build to read: dist/ is missing; fix: run `brain build`, then `brain edge plan`"
                .into(),
        );
    }
    // a site with no page rendered per request has no server manifest: its plan is the same with no route in it
    let config: Value = if root.join("brain.json").exists() {
        serde_json::from_str(&audit::read(&root.join("brain.json"), 2 * 1024 * 1024)?)
            .map_err(|_| "brain.json is not valid JSON")?
    } else {
        json!({})
    };
    let manifest: Value = if root.join("dist/_server/manifest.json").exists() {
        serde_json::from_str(&audit::read(
            &root.join("dist/_server/manifest.json"),
            2 * 1024 * 1024,
        )?)
        .map_err(|_| {
            "dist/_server/manifest.json is not valid JSON; fix: run `brain build` again".to_string()
        })?
    } else {
        json!({"base": config["base"]})
    };
    let mut plan = from_manifest(&manifest);
    // The built host policy is authoritative, including custom staticOutput directories.
    let assets = audit::assets(root, &base_of(&manifest))?;
    if !assets.is_empty() {
        let rules = plan["rules"].as_array_mut().unwrap();
        if assets
            .iter()
            .any(|path| !path.starts_with(&format!("{}_b/", base_of(&manifest))))
        {
            rules.retain(|rule| rule["immutable"] != true);
        }
        for path in &assets {
            rules.insert(rules.len()-1,json!({"path":path,"action":"cache","seconds":31536000,"immutable":true,"reason":"built immutable file"}));
        }
    }
    plan["immutableAssets"] = json!(assets);
    let custom = config["staticOutput"].as_object().is_some_and(|fields| {
        ["js", "css", "images", "assets", "fonts"]
            .iter()
            .any(|key| {
                fields
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|v| v != "_b")
            })
    });
    if custom {
        plan["rules"]
            .as_array_mut()
            .unwrap()
            .retain(|r| r["path"] != format!("{}_b/*", base_of(&manifest)));
        if plan["immutableAssets"].as_array().unwrap().is_empty() {
            for key in ["js", "css", "images", "assets", "fonts"] {
                if let Some(folder) = config["staticOutput"][key].as_str() {
                    let path = format!("{}{folder}/", base_of(&manifest));
                    if audit::safe_path(&path) && !audit::protected(&path) {
                        plan["rules"].as_array_mut().unwrap().push(json!({"path":format!("{path}*"),"action":"follow-origin","reason":"custom asset destination; no emitted immutable host policy available"}));
                    }
                }
            }
        }
    }
    Ok(plan)
}

fn evaluate(root: &Path, args: &[String]) -> Result<(Value, audit::Options), String> {
    let options = audit::Options::parse(args)?;
    let plan = plan(root)?;
    let result = match (options.command.as_str(), options.provider.as_deref()) {
        ("plan", None) => plan,
        ("check" | "audit", None) => audit::run(&plan, options.origin.as_deref().ok_or("edge audit requires --url <origin>")?)?,
        ("plan" | "audit" | "check" | "apply", Some("cloudflare")) => cloudflare::run(&plan, &options)?,
        _ => return Err("usage: brain edge plan|check|audit|apply [--url <origin>] [--provider cloudflare] [--request <file>] [--json]".into()),
    };
    Ok((result, options))
}
/// One structured result, including a nonzero audit verdict; the caller must not append a second JSON error.
pub fn command(
    root: Result<std::path::PathBuf, String>,
    args: &[String],
    json_output: bool,
) -> (u8, String) {
    match root.and_then(|root| evaluate(&root, args)) {
        Ok((result, options)) => {
            let code = if result["ok"] == false { 2 } else { 0 };
            let output = if json_output || options.command != "plan" || options.provider.is_some() {
                result.to_string()
            } else {
                text(&result)
            };
            (code, output)
        }
        Err(error) => (
            2,
            if json_output {
                crate::error_json(&error)
            } else {
                format!("error: {error}")
            },
        ),
    }
}
