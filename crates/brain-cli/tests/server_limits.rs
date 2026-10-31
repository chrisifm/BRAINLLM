//! brain.json `server` (INTERNALS §11.1): `brain check` rejects an invalid value with B0707 and names the fix; a valid
//! one passes and reaches the server manifest, where server.mjs enforces it.
use std::process::{Command, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr))
}

fn with_server(dir: &std::path::Path, server: &str) {
    let json = std::fs::read_to_string(dir.join("brain.json")).unwrap();
    let json = json.split_once("\"server\":").map_or(json.clone(), |(head, _)| head.trim_end().trim_end_matches(',').to_string() + "\n}\n");
    let body = json.trim_end().strip_suffix('}').unwrap().trim_end();
    std::fs::write(dir.join("brain.json"), format!("{body},\n  \"server\": {server}\n}}\n")).unwrap();
}

#[test]
fn check_validates_the_server_setting_and_build_writes_it_to_the_manifest() {
    let tmp = std::env::temp_dir().join(format!("bsrv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).0);
    let dir = tmp.join("app");
    with_server(&dir, "{ \"maxBodyKB\": 70000 }");
    let (ok, chk) = brain(&dir, &["check"]);
    assert!(!ok && chk.contains("B0707") && chk.contains("brain.json") && chk.contains("server.maxBodyKB") && chk.contains("65536"), "{chk}");
    with_server(&dir, "{ \"maxBodyMB\": 8 }");
    let (ok, chk) = brain(&dir, &["check"]);
    assert!(!ok && chk.contains("B0707") && chk.contains("server.maxBodyMB"), "{chk}");
    with_server(&dir, "{ \"maxBodyKB\": 8192, \"timeoutMs\": 30000, \"rpcPerMinute\": 600 }");
    let (ok, chk) = brain(&dir, &["check"]);
    assert!(ok && !chk.contains("B0707"), "{chk}");
    let (ok, b) = brain(&dir, &["build", "--progress=off"]);
    assert!(ok, "{b}");
    let mf = std::fs::read_to_string(dir.join("dist/_server/manifest.json")).unwrap();
    assert!(mf.contains("\"server\":{\"maxBodyKB\":8192,\"timeoutMs\":30000,\"rpcPerMinute\":600}"), "{mf}");
    let _ = std::fs::remove_dir_all(&tmp);
}
