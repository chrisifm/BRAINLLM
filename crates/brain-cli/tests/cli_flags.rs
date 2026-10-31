//! Every command refuses a flag it does not know, and `brain test` is never green without running a test.
#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn brain(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::null()).env_remove("BRAIN_VIEWER").output().unwrap()
}

fn app(name: &str) -> PathBuf {
    let tmp = std::env::temp_dir().join(format!("brain-flags-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    tmp.join("app")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn every_command_refuses_an_unknown_flag_before_acting() {
    let dir = app("table");
    const FLAG: &str = "--definitely-unknown";
    for words in [
        "new other", "add view other", "add comp other", "check", "fix", "doctor", "build", "run", "dev", "test", "explain B0302", "map", "card counter",
        "split counter div Other", "spec", "guide", "schema", "theme --list", "i18n", "seo audit", "stop", "reload", "status", "version", "find x", "symbols comp/counter.brain",
        "refs x", "lib list", "history", "integration list", "deps analyze", "feedback query", "quality", "deploy config pm2", "design check", "manifest query",
        "backend schema", "backend serve", "backend migrate", "jobs stats", "analytics report", "media list", "e2e probe", "e2e run", "changes list", "graph",
        "context x", "why x", "preflight", "recipe list", "risk x", "edit", "shot", "preview", "start", "docs", "upgrade", "mcp", "agent setup",
    ] {
        let mut args: Vec<&str> = words.split(' ').collect();
        args.push(FLAG);
        let out = brain(&dir, &args);
        assert_eq!(out.status.code(), Some(2), "brain {words} {FLAG} must exit 2: {}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
    }
    assert!(!dir.join("other").exists() && !dir.join("views/other.brain").exists() && !dir.join("comp/other.brain").exists(), "a refused command created files");
    assert!(!dir.join("dist").exists(), "a refused build wrote dist/");
    let json = brain(&dir, &["test", "--port", "5280", "--json"]);
    let text = String::from_utf8_lossy(&json.stdout);
    assert!(text.contains("\"error\"") && text.contains("--port") && text.contains("\"fix\""), "{text}");
}

#[test]
fn every_flag_the_usage_documents_is_accepted() {
    let dir = app("usage");
    // a documented flag reaches the command: whatever it answers, it is not the unknown-flag refusal
    for words in ["test --timeout 1 no-such-path", "test --heap no-such-path", "map --limit 1", "spec --topic tests", "build --site https://example.dev --report", "doctor --fix no-such-path", "i18n --missing", "theme --list", "seo audit --since 2026-01-01"] {
        let args: Vec<&str> = words.split(' ').collect();
        let out = brain(&dir, &args);
        assert!(!stderr(&out).contains("unknown flag"), "brain {words}: {}", stderr(&out));
    }
}

#[test]
fn a_test_run_selecting_nothing_is_not_green() {
    let dir = app("selection");
    std::fs::create_dir_all(dir.join("empty")).unwrap();
    std::fs::write(dir.join("comp/none.test.brain"), "<script>\n</script>\n").unwrap();
    for (args, needle) in [
        (&["test", "--port", "5280"][..], "--port"),
        (&["test", "5280"], "does not exist"),
        (&["test", "comp/counter.brain"], "not a test file"),
        (&["test", "empty"], "no *.test.brain"),
        (&["test", "comp/counter.test.brain", "comp/none.test.brain"], "one path"),
        (&["test", "comp/none.test.brain"], "no test"),
        (&["test", "--timeout", "soon"], "--timeout"),
    ] {
        let out = brain(&dir, args);
        assert_eq!(out.status.code(), Some(2), "brain {args:?}: {}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
        assert!(stderr(&out).contains(needle), "brain {args:?}: {}", stderr(&out));
    }
    let selected = brain(&dir, &["test", "comp/counter.test.brain"]);
    assert!(selected.status.success(), "{}{}", String::from_utf8_lossy(&selected.stdout), stderr(&selected));
    assert!(String::from_utf8_lossy(&selected.stdout).contains("counter.test.brain"));
}

/// A native-socket app whose declared origin is the port `brain run` holds.
#[cfg(unix)]
fn socket_app(name: &str) -> (PathBuf, u16) {
    let dir = app(name);
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let origin = format!("http://127.0.0.1:{port}");
    let manifest = std::fs::read_to_string(dir.join("brain.json")).unwrap();
    std::fs::write(dir.join("brain.json"), manifest.replacen('{', &format!(r#"{{"backend":{{"origins":["{origin}"],"sockets":{{"allowedOrigins":["{origin}"]}}}},"#), 1)).unwrap();
    let env = std::fs::read_to_string(dir.join(".brain_env")).unwrap();
    std::fs::write(dir.join(".brain_env"), env.replacen("http://localhost:5180", &origin, 1).replacen("[development]", "[development]\nBRAIN_BACKEND_URL=http://127.0.0.1:9", 1)).unwrap();
    (dir, port)
}

#[test]
fn tests_run_against_the_apps_own_dev_server_on_the_declared_port() {
    let (dir, port) = socket_app("reuse");
    let mut dev = Command::new(env!("CARGO_BIN_EXE_brain")).args(["run", "--port", &port.to_string(), "--no-inspect"]).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    for _ in 0..200 {
        if dir.join(".brain/dev.json").exists() && std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() { break }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let tested = brain(&dir, &["test", "comp/counter.test.brain"]);
    let doctor = brain(&dir, &["doctor", "--json"]);
    let _ = dev.kill();
    let _ = dev.wait();
    assert!(tested.status.success(), "{}{}", String::from_utf8_lossy(&tested.stdout), stderr(&tested));
    assert!(stderr(&tested).contains("brain run"), "the reuse is stated: {}", stderr(&tested));
    let report = String::from_utf8_lossy(&doctor.stdout);
    assert!(!report.contains("cannot listen") && !report.contains("could not start the test server"), "{report}");
}

#[test]
fn another_process_on_the_declared_port_is_named_without_suggesting_a_flag_test_lacks() {
    let (dir, port) = socket_app("foreign");
    let _held = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let out = brain(&dir, &["test"]);
    assert_eq!(out.status.code(), Some(2));
    let text = stderr(&out);
    assert!(text.contains(&port.to_string()) && text.contains("lsof") && text.contains("PUBLIC_SITE_URL"), "{text}");
    assert!(!text.contains("--port"), "brain test has no --port: {text}");
}

#[test]
fn a_doctor_whose_tests_did_not_run_does_not_claim_no_errors() {
    let (dir, port) = socket_app("contradiction");
    let _held = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let out = brain(&dir, &["doctor"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("did not run"), "{text}");
    assert!(!text.contains("none captured"), "{text}");
}
