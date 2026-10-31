//! `brain trace` (INTERNALS §49): reads what `brain run` captured in `.brain/trace.db`. The store lives in brain-serve;
//! this file turns arguments into one store query.

use std::io::Write;
use std::path::Path;

/// `args` as the store takes them. The MCP tool builds the same object.
pub(crate) fn query_args(args: &[String]) -> Result<String, String> {
    let mut query = serde_json::Map::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        match name {
            "--json" => { query.insert("json".into(), true.into()); }
            "--failed" => { query.insert("failed".into(), true.into()); }
            "--since" | "--module" | "--limit" | "--app" => {
                let value = inline.or_else(|| rest.next().cloned()).ok_or_else(|| format!("{name} needs a value; fix: `brain trace {name} <value>`"))?;
                if name != "--app" { query.insert(name.trim_start_matches("--").into(), value.into()); }
            }
            "issues" => { query.insert("mode".into(), "issues".into()); }
            id if !id.starts_with('-') && !query.contains_key("requestId") => { query.insert("requestId".into(), id.into()); }
            other => return Err(format!("unknown argument `{other}`; fix: `brain trace [<requestId>|issues] [--since 5m] [--failed] [--module m] [--limit n] [--json]`")),
        }
    }
    if let Some(id) = query.get("requestId").and_then(|id| id.as_str()) && id.len() < 8 {
        return Err(format!("`{id}` is too short for a request id; fix: at least its first 8 characters"));
    }
    Ok(serde_json::Value::Object(query).to_string())
}

/// A closed pipe (`brain trace | head`) is a normal way to stop reading.
pub(crate) fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let answer = brain_serve::trace::query(root, &query_args(args)?)?;
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(answer.as_bytes()).and_then(|()| stdout.flush()) {
        Err(error) if error.kind() != std::io::ErrorKind::BrokenPipe => Err(error.to_string()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::query_args;

    fn parsed(args: &[&str]) -> serde_json::Value {
        serde_json::from_str(&query_args(&args.iter().map(ToString::to_string).collect::<Vec<_>>()).unwrap()).unwrap()
    }

    #[test]
    fn arguments_become_one_store_query() {
        assert_eq!(parsed(&["--since", "5m", "--failed", "--module=browser", "--json"]), serde_json::json!({ "since": "5m", "failed": true, "module": "browser", "json": true }));
        assert_eq!(parsed(&["issues"]), serde_json::json!({ "mode": "issues" }));
        assert_eq!(parsed(&["0123abcd-0123"]), serde_json::json!({ "requestId": "0123abcd-0123" }));
    }

    #[test]
    fn a_bad_argument_names_its_fix() {
        for bad in [&["--bogus"][..], &["--since"], &["abc"]] {
            let error = query_args(&bad.iter().map(ToString::to_string).collect::<Vec<_>>()).unwrap_err();
            assert!(error.contains("fix:"), "{error}");
        }
    }
}
