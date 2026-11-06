//! Lifecycle control selects one explicit app, independently of the caller's working directory.
#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!("brain-control-path-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir_all(root.join("app/views/nested")).unwrap();
        std::fs::write(root.join("app/tokens.cssbrain"), ":root {}\n").unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
fn cli(cwd: &Path, command: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).arg(command).args(args).arg("--json")
        .current_dir(cwd).stdin(Stdio::null()).output().unwrap()
}
fn socket(root: &Path) -> PathBuf {
    let full = root.join(".brain/run/brain.sock");
    if full.as_os_str().len() < 100 { full }
    else { PathBuf::from(format!("/tmp/brain-{}.sock", brain_pack::content_hash(&root.to_string_lossy()))) }
}
fn controlled(root: &Path, cwd: &Path, command: &str, args: &[&str]) {
    std::fs::create_dir_all(root.join(".brain/run")).unwrap();
    let path = socket(root);
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let expected = command.to_string();
    let controller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => return Err(format!("no control connection: {e}")),
            }
        };
        // BSD sockets inherit the listener's nonblocking mode.
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        stream.write_all(b"{\"ok\":true,\"selected\":\"target\"}\n").unwrap();
        if line.trim() == expected { Ok(()) } else { Err(line) }
    });
    let result = cli(cwd, command, args);
    let received = controller.join().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(result.status.success(), "{}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    assert!(received.is_ok(), "{received:?}");
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["selected"], "target");
}

#[test]
fn control_commands_resolve_relative_absolute_nested_and_symlink_targets() {
    let fixture = Fixture::new();
    let root = std::fs::canonicalize(fixture.0.join("app")).unwrap();
    let absolute = root.to_str().unwrap();
    std::os::unix::fs::symlink(&root, fixture.0.join("alias")).unwrap();
    for command in ["status", "reload", "stop"] {
        for path in ["app", "app/views/nested", "alias", absolute] {
            controlled(&root, &fixture.0, command, &[path]);
        }
        controlled(&root, &root.join("views/nested"), command, &[]);
    }
}

#[test]
fn invalid_explicit_targets_never_fall_back_to_the_current_project() {
    let fixture = Fixture::new();
    let root = fixture.0.join("app");
    for command in ["status", "reload", "stop"] {
        for (args, expected) in [
            (vec!["missing"], "does not exist"),
            (vec!["tokens.cssbrain"], "must be a directory"),
            (vec![".."], "not inside a BRAINLLM app"),
            (vec![".", "."], "at most one project directory"),
        ] {
            let result = cli(&root, command, &args);
            assert!(!result.status.success());
            let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            assert!(format!("{} {}", report["error"], report["fix"]).contains(expected), "{report}");
            assert!(!report["fix"].as_str().unwrap().is_empty(), "{report}");
            assert!(!String::from_utf8_lossy(&result.stdout).contains("no `brain start`"));
        }
    }
}
