//! `brain add auth` and `brain new <app> --backend`: one path from nothing to working accounts on the native backend.
//! Writes what is missing and keeps everything that exists, so it is safe on a real app. It never migrates, starts or
//! creates anything outside the app folder: those stay explicit commands, and the report names them in order.
use serde_json::{Value, json};
use std::path::Path;

/// The local development backend, on the port `brain backend serve` uses by default.
pub const DEFAULT_BACKEND_URL: &str = "http://127.0.0.1:5182";

const LOGIN: &str = include_str!("templates/new/views/login.brain");
/// The generated sign-in page gets its sign-up link only here: an app without accounts has no such route.
const SIGNUP_LINK: &str = "      <Button type=\"submit\" busy={f.busy}>Sign in</Button>\n    </form>\n";
const SIGNUP_LINK_NEW: &str = "      <Button type=\"submit\" busy={f.busy}>Sign in</Button>\n    </form>\n    <p>No account yet? <a href=\"/signup\">Create one</a></p>\n";

fn files() -> Vec<(&'static str, String)> {
    let basics: std::collections::BTreeMap<&str, String> = crate::design::new_basics().into_iter().collect();
    let mut out = vec![
        ("views/signup.brain", include_str!("templates/new/views/signup.brain").to_string()),
        ("views/signup.test.brain", include_str!("templates/new/views/signup.test.brain").to_string()),
        ("views/login.brain", LOGIN.replace(SIGNUP_LINK, SIGNUP_LINK_NEW)),
        ("views/login.test.brain", include_str!("templates/new/views/login.test.brain").to_string()),
        ("views/account/index.brain", include_str!("templates/new/views/account/index.brain").to_string()),
        ("views/account/_guard.jsbrain", include_str!("templates/new/views/account/_guard.jsbrain").to_string()),
        ("channels/session.jsbrain", include_str!("templates/new/channels/session.jsbrain").to_string()),
        ("server/auth.jsbrain", include_str!("templates/new/server/auth.jsbrain").to_string()),
        // `brain run` and `brain start` reach the backend from this machine, so each visitor keeps their own rate limit.
        ("server/platform.jsbrain", "<meta server>\nexport default { trustProxy: ['127.0.0.1', '::1'] }\n".to_string()),
    ];
    if let Some(button) = basics.get("ui/button.brain") {
        out.push(("ui/button.brain", button.clone()));
    }
    out
}

fn database_name(root: &Path) -> String {
    let name = root.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let clean: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("{}_dev", if clean.is_empty() { "app" } else { &clean })
}

/// Writes the account files and environment keys that are missing; returns what happened and what to run next.
pub fn add(root: &Path) -> Result<Value, String> {
    let (mut written, mut kept) = (vec![], vec![]);
    for (rel, body) in files() {
        let path = root.join(rel);
        if path.symlink_metadata().is_ok() {
            // An untouched generated sign-in page gains the link; an edited one is the author's.
            if rel == "views/login.brain" && std::fs::read_to_string(&path).is_ok_and(|s| s == LOGIN) {
                std::fs::write(&path, &body).map_err(|e| format!("could not write {rel}: {e}"))?;
                written.push(rel.to_string());
                continue;
            }
            kept.push(rel.to_string());
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|e| format!("could not create the folder of {rel}: {e}"))?;
        std::fs::write(&path, body).map_err(|e| format!("could not write {rel}: {e}"))?;
        written.push(rel.to_string());
    }
    let database = database_name(root);
    let mut env = vec![];
    for (key, section, value) in [
        ("DATABASE_URL", "development", format!("postgres://127.0.0.1:5432/{database}")),
        ("BRAIN_BACKEND_URL", "development", DEFAULT_BACKEND_URL.to_string()),
        ("BRAIN_BACKEND_URL", "production", DEFAULT_BACKEND_URL.to_string()),
    ] {
        if !declared(root, section, key) {
            crate::env::set(root, key, section, &value)?;
            env.push(format!("[{section}] {key}"));
        }
    }
    Ok(json!({
        "schemaVersion": 1,
        "written": written,
        "kept": kept,
        "env": env,
        "next": [
            "brain backend migrate --create-db   # creates the local database if missing, then the tables",
            "brain run --backend                 # the app and the backend, one command",
            "open /signup, create an account, then `brain backend bootstrap-admin --email <address>` for the first administrator",
            "production: set DATABASE_URL and BRAIN_JWT_SECRET in the host; see `brain guide backend`",
        ],
    }))
}

/// Whether `.brain_env` names `key` in `[section]`, with or without a value; `env::set` would replace it otherwise.
fn declared(root: &Path, section: &str, key: &str) -> bool {
    let src = std::fs::read_to_string(root.join(".brain_env")).unwrap_or_default();
    let mut current = "";
    for line in src.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            current = line.trim_matches(['[', ']']);
        } else if current == section && line.split_once('=').is_some_and(|(k, _)| k.trim().trim_end_matches('?') == key) {
            return true;
        }
    }
    false
}

/// The human form of the report `add` returns.
pub fn print(report: &Value) {
    let list = |key: &str| report[key].as_array().cloned().unwrap_or_default().into_iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>();
    for f in list("written") {
        println!("+ {f}");
    }
    for f in list("kept") {
        println!("= {f} (kept)");
    }
    for e in list("env") {
        println!("+ .brain_env {e}");
    }
    println!("\nnext:");
    for n in list("next") {
        println!("  {n}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("brain-auth-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    #[test]
    fn the_replaced_login_anchor_exists_in_the_template() {
        assert!(LOGIN.contains(SIGNUP_LINK), "the sign-up link anchor moved in views/login.brain");
    }

    #[test]
    fn a_new_app_gets_accounts_and_passes_the_checker() {
        let s = scratch("new");
        let app = s.0.join("demo-app");
        crate::scaffold::new_app(&app).unwrap();
        let report = add(&app).unwrap();
        let written: Vec<&str> = report["written"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        assert!(written.contains(&"views/signup.brain") && written.contains(&"views/login.brain"), "{written:?}");
        assert!(std::fs::read_to_string(app.join("views/login.brain")).unwrap().contains("/signup"));
        let env = std::fs::read_to_string(app.join(".brain_env")).unwrap();
        assert!(env.contains("postgres://127.0.0.1:5432/demo_app_dev"), "{env}");
        assert!(env.contains("BRAIN_BACKEND_URL=http://127.0.0.1:5182"), "{env}");
        let example = std::fs::read_to_string(app.join(".brain_env.example")).unwrap();
        assert!(example.contains("DATABASE_URL=") && !example.contains("postgres://"), "values stay out of the example: {example}");
    }

    #[test]
    fn nothing_that_exists_is_overwritten_and_a_second_run_changes_nothing() {
        let s = scratch("keep");
        let app = s.0.join("demo");
        crate::scaffold::new_app(&app).unwrap();
        std::fs::write(app.join("views/login.brain"), "<main><h1>Mine</h1></main>\n").unwrap();
        crate::env::set(&app, "DATABASE_URL", "development", "postgres://db.internal/mine").unwrap();
        add(&app).unwrap();
        assert_eq!(std::fs::read_to_string(app.join("views/login.brain")).unwrap(), "<main><h1>Mine</h1></main>\n");
        assert!(std::fs::read_to_string(app.join(".brain_env")).unwrap().contains("postgres://db.internal/mine"));
        let again = add(&app).unwrap();
        assert_eq!(again["written"], json!([]));
        assert_eq!(again["env"], json!([]));
    }
}
