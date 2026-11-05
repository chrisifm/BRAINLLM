//! `brain release-check --env production [--url <origin>] [--json]`: one exit code for "can this go out". It composes the
//! gates that already exist (doctor, secrets, a build for that environment, the SEO audit and, with `--url`, the live checks) and
//! never decides anything they do not. Each command runs as its own process, so one step cannot disturb the next.
use std::path::Path;
use std::process::{Command, Stdio};

struct Step {
    name: &'static str,
    ok: bool,
    detail: Vec<String>,
}

/// A failed command says why in its last lines; a passing one needs no space in the report.
fn command(name: &'static str, root: &Path, args: &[&str]) -> Step {
    let Ok(exe) = std::env::current_exe() else { return Step { name, ok: false, detail: vec!["could not find this binary to run the step".into()] } };
    match Command::new(exe).args(args).current_dir(root).env("NO_COLOR", "1").stdin(Stdio::null()).output() {
        Ok(out) if out.status.success() => Step { name, ok: true, detail: Vec::new() },
        Ok(out) => {
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
            Step { name, ok: false, detail: lines[lines.len().saturating_sub(20)..].iter().map(|line| line.to_string()).collect() }
        }
        Err(error) => Step { name, ok: false, detail: vec![format!("could not run `brain {}`: {error}", args.join(" "))] },
    }
}

fn secrets(root: &Path, env: &str) -> Step {
    let detail: Vec<String> = crate::backend::secret_findings(root, env).into_iter().map(|(file, message, disables)| format!("{}{file}: {message}", if disables { "B0790 " } else { "" })).collect();
    Step { name: "secrets", ok: detail.is_empty(), detail }
}

/// Built here, for this environment: doctor validates isolated output; an existing dist/ proves nothing about
/// which environment it holds. This explicit deployment build remains mandatory.
fn built_output(root: &Path, env: &str) -> Step {
    let mut step = command("build", root, &["build", "--env", env, "--progress=off"]);
    if step.ok {
        step.detail = crate::asset_check::check(&root.join("dist"));
    }
    step
}

pub fn run(root: &Path, args: &[String]) -> (i32, String) {
    let json = args.iter().any(|a| a == "--json");
    let env = crate::flag(args, "--env").unwrap_or_else(|| "production".into());
    let url = crate::flag(args, "--url").map(|u| u.trim_end_matches('/').to_string());
    if url.as_deref().is_some_and(|u| !(u.starts_with("http://") || u.starts_with("https://"))) {
        return (2, "usage: brain release-check [--env production] [--url <https://origin>] [--json]; fix: --url must start with http:// or https://".into());
    }
    let mut steps = vec![command("doctor", root, &["doctor"]), secrets(root, &env), built_output(root, &env)];
    if steps[2].ok {
        steps.push(command("seo", root, &["seo", "audit"]));
    }
    if let Some(origin) = &url {
        steps.push(command("deploy", root, &["deploy", "check", "--url", origin]));
        steps.push(command("seo-live", root, &["seo", "--url", origin]));
    }
    let ok = steps.iter().all(|step| step.ok);
    let text = if json {
        let list: Vec<_> = steps.iter().map(|s| serde_json::json!({"name":s.name,"ok":s.ok,"detail":s.detail})).collect();
        serde_json::json!({"schemaVersion":1,"ok":ok,"env":env,"url":url,"steps":list}).to_string()
    } else {
        let mut out = String::new();
        for step in &steps {
            out.push_str(&format!("{} {}\n", if step.ok { "ok  " } else { "FAIL" }, step.name));
            step.detail.iter().for_each(|line| out.push_str(&format!("     {line}\n")));
        }
        out.push_str(&format!("release-check {env}: {}", if ok { "OK" } else { "not ready" }));
        out
    };
    (if ok { 0 } else { 1 }, text)
}
