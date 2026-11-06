//! `brain db audit`: current app configuration, one read-only database audit, no apply path.
use serde_json::{Value, json};
use std::{path::Path, process::ExitCode, time::Duration};
const USAGE: &str = "usage: brain db audit [--env development] [--limit 50] [--json]; read-only current database/role observations and review proposals, never --apply";
#[derive(Debug, PartialEq)]
struct Options {
    environment: String,
    limit: u32,
    json: bool,
}
fn options(args: &[String]) -> Result<Options, &'static str> {
    if args.first().map(String::as_str) != Some("audit") {
        return Err(USAGE);
    }
    let mut out = Options {
        environment: "development".into(),
        limit: 50,
        json: false,
    };
    let (mut env_seen, mut limit_seen) = (false, false);
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--json" if !out.json => out.json = true,
            "--env" if !env_seen => {
                i += 1;
                let value = args.get(i).ok_or(USAGE)?;
                if value.is_empty()
                    || value.len() > 64
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    || !value.as_bytes()[0].is_ascii_alphabetic()
                {
                    return Err(USAGE);
                }
                out.environment = value.clone();
                env_seen = true;
            }
            "--limit" if !limit_seen => {
                i += 1;
                let value = args.get(i).ok_or(USAGE)?;
                if !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(USAGE);
                }
                out.limit = value.parse().map_err(|_| USAGE)?;
                if !(1..=brain_backend::database_audit::MAX_LIMIT).contains(&out.limit) {
                    return Err(USAGE);
                }
                limit_seen = true;
            }
            _ => return Err(USAGE),
        }
        i += 1;
    }
    Ok(out)
}
fn error(code: &str, message: &str) -> Value {
    json!({"schemaVersion":1,"ok":false,"readOnly":true,"error":{"code":code,"message":message},"queryTextCaptured":false,"valuesCaptured":false})
}
struct Runtime(Option<tokio::runtime::Runtime>);
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}
fn report(root: &Path, options: &Options) -> Result<Value, Value> {
    let url=crate::env::value(root,&options.environment,"DATABASE_URL").ok_or_else(||error("DB_AUDIT_DATABASE_REQUIRED","Configure DATABASE_URL for this app and selected environment; the audit never searches another project"))?;
    let runtime = Runtime(Some(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| {
                error(
                    "DB_AUDIT_RUNTIME_UNAVAILABLE",
                    "Cannot initialize the bounded database audit runtime",
                )
            })?,
    ));
    let mut value = runtime
        .0
        .as_ref()
        .unwrap()
        .block_on(brain_backend::database_audit::audit(
            &url,
            options.limit,
            options.environment == "production",
        ))
        .map_err(|e| e.report())?;
    value["environment"] = json!(options.environment);
    value["configurationSource"] = json!(
        "Current app DATABASE_URL: process environment, selected .brain_env section, then shared values; connection details omitted"
    );
    Ok(value)
}
fn text(value: &Value) -> String {
    if let Some(error) = value.get("error") {
        let state = error["sqlstate"]
            .as_str()
            .map(|value| format!(" (SQLSTATE {value})"))
            .unwrap_or_default();
        return format!(
            "Database audit unavailable: {}{} — {}\n",
            error["code"].as_str().unwrap_or("DB_AUDIT_UNAVAILABLE"),
            state,
            error["message"]
                .as_str()
                .unwrap_or("Metadata is unavailable")
        );
    }
    let mut out = format!(
        "Read-only database audit\nDatabase: {}\nRole: {}\n",
        value["scope"]["database"], value["scope"]["role"]
    );
    for name in [
        "settings",
        "configuredOverrides",
        "connections",
        "tables",
        "indexes",
        "statements",
    ] {
        let section = &value[name];
        let status = section["status"].as_str().unwrap_or("unavailable");
        let count = section["observations"]
            .as_array()
            .map(|rows| {
                format!(
                    ", {} sampled row(s){}",
                    rows.len(),
                    if section["truncated"] == true {
                        "; truncated"
                    } else {
                        ""
                    }
                )
            })
            .unwrap_or_default();
        let reason = section["reason"]
            .as_str()
            .map(|r| format!("; {r}"))
            .unwrap_or_default();
        out.push_str(&format!("  {name}: {status}{count}{reason}\n"));
    }
    out.push_str("No query text, application values or foreign identities captured. No persistent settings or schema changed.\n");
    out.push_str("work_mem times max_connections is a capacity scenario, not actual RAM, a total memory bound or proof of OOM.\n");
    for item in value["recommendations"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "Review proposal {} (operator approval required): {}\n",
            item["id"], item["reason"]
        ));
        if item["sql"].is_string() {
            out.push_str(&format!("  {}\n", item["sql"]));
        }
    }
    out
}
fn emit(value: &Value, json_output: bool) {
    if json_output {
        println!("{value}")
    } else {
        print!("{}", text(value))
    }
}
/// Own the exit code so partial observations remain one JSON document rather than a report plus a second error.
pub fn entry(args: &[String]) -> ExitCode {
    let json_output = args.iter().any(|arg| arg == "--json");
    let options = match options(args) {
        Ok(options) => options,
        Err(message) => {
            emit(&error("DB_AUDIT_ARGUMENTS", message), json_output);
            return ExitCode::from(2);
        }
    };
    let root = match crate::app_root() {
        Ok(root) => root,
        Err(_) => {
            emit(
                &error(
                    "DB_AUDIT_APP_REQUIRED",
                    "Run brain db audit from a BRAINLLM app with its own configured DATABASE_URL",
                ),
                json_output,
            );
            return ExitCode::from(2);
        }
    };
    match report(&root, &options) {
        Ok(value) => {
            emit(&value, options.json);
            if value["ok"] == true {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(value) => {
            let code = if value["error"]["code"] == "DB_AUDIT_DATABASE_REQUIRED" {
                2
            } else {
                1
            };
            emit(&value, options.json);
            ExitCode::from(code)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(value: &[&str]) -> Vec<String> {
        value.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn audit_arguments_are_bounded_read_only_and_reject_ambiguous_or_mutating_options() {
        assert_eq!(
            options(&args(&["audit"])).unwrap(),
            Options {
                environment: "development".into(),
                limit: 50,
                json: false
            }
        );
        assert_eq!(
            options(&args(&[
                "audit",
                "--limit",
                "1",
                "--json",
                "--env",
                "production"
            ]))
            .unwrap()
            .limit,
            1
        );
        for bad in [
            args(&["apply"]),
            args(&["audit", "--apply"]),
            args(&["audit", "--url", "postgres://example"]),
            args(&["audit", "--limit", "0"]),
            args(&["audit", "--limit", "201"]),
            args(&["audit", "--limit", "99999999999999999999999"]),
            args(&["audit", "--env", "../other"]),
            args(&["audit", "--env", "production", "--env", "development"]),
            args(&["audit", "--json", "--json"]),
        ] {
            assert!(options(&bad).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn text_output_quotes_catalog_identifiers_and_does_not_announce_optional_sections_as_measured()
    {
        let output = text(
            &json!({"scope":{"database":"db\nname","role":"role\u{1b}name"},"statements":{"status":"unavailable","reason":"permission-denied"}}),
        );
        assert!(!output.contains('\u{1b}'));
        assert!(output.contains("statements: unavailable; permission-denied"));
        assert!(output.contains("not actual RAM"));
    }
}
