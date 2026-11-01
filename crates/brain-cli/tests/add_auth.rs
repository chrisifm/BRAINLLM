//! `brain add auth` / `brain new --backend`: the whole accounts path, without overwriting, checks clean.
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn brain(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap()
}
fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}
fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("brain-add-auth-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn add_auth_writes_the_path_keeps_existing_files_and_checks_clean() {
    let base = workspace("add");
    assert!(brain(&base, &["new", "demo"]).status.success());
    let app = base.join("demo");
    let custom = "<main><h1>Mine</h1></main>\n";
    std::fs::create_dir_all(app.join("views")).unwrap();
    std::fs::write(app.join("views/signup.brain"), custom).unwrap();
    let first = brain(&app, &["add", "auth"]);
    assert!(first.status.success(), "{}", text(&first));
    for file in ["views/login.brain", "channels/session.jsbrain", "server/auth.jsbrain", "server/platform.jsbrain", ".brain_env"] {
        assert!(app.join(file).exists(), "{file} missing: {}", text(&first));
    }
    assert_eq!(std::fs::read_to_string(app.join("views/signup.brain")).unwrap(), custom, "an existing file is never overwritten");
    let env = std::fs::read_to_string(app.join(".brain_env")).unwrap();
    assert!(env.contains("BRAIN_BACKEND_URL=") && env.contains("DATABASE_URL="), "{env}");
    let again = brain(&app, &["add", "auth"]);
    assert!(again.status.success(), "a second run is harmless: {}", text(&again));
    assert_eq!(std::fs::read_to_string(app.join(".brain_env")).unwrap().matches("BRAIN_BACKEND_URL=").count(), env.matches("BRAIN_BACKEND_URL=").count(), "keys are not repeated");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn new_backend_starts_with_accounts_and_no_errors() {
    let base = workspace("new");
    let created = brain(&base, &["new", "demo", "--backend"]);
    assert!(created.status.success(), "{}", text(&created));
    let app = base.join("demo");
    assert!(app.join("views/signup.brain").exists());
    let check = brain(&app, &["check", "--json"]);
    let out = String::from_utf8_lossy(&check.stdout);
    assert!(!out.contains("\"severity\":\"error\""), "{out}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn run_backend_refuses_a_port_another_process_answers() {
    let base = workspace("port");
    assert!(brain(&base, &["new", "demo", "--backend"]).status.success());
    let app = base.join("demo");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let env = std::fs::read_to_string(app.join(".brain_env")).unwrap();
    let env: String = env.lines().map(|l| if l.starts_with("BRAIN_BACKEND_URL=") { format!("BRAIN_BACKEND_URL=http://127.0.0.1:{port}\n") } else { format!("{l}\n") }).collect();
    std::fs::write(app.join(".brain_env"), env).unwrap();
    let web = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let free = web.local_addr().unwrap().port().to_string();
    drop(web);
    let run = brain(&app, &["run", "--backend", "--port", &free, "--no-inspect", "--quiet"]);
    assert!(!run.status.success());
    assert!(text(&run).contains("B0763"), "{}", text(&run));
    let _ = std::fs::remove_dir_all(&base);
}
