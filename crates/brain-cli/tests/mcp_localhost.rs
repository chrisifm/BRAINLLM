//! `brain_run_start` reports, and its browser opens, the origin the socket allow-list accepts: an app that declares only
//! a `localhost` origin must not be sent to 127.0.0.1, where its live sockets are refused (B0743).
#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn run_start_reports_the_declared_localhost_origin() {
    let tmp = std::env::temp_dir().join(format!("bmcplocal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "app"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("app");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let origin = format!("http://localhost:{port}");
    let cfg = std::fs::read_to_string(dir.join("brain.json")).unwrap();
    std::fs::write(dir.join("brain.json"), cfg.trim_end().trim_end_matches('}').trim_end().to_string() + &format!(",\n  \"backend\": {{ \"origins\": [\"{origin}\"], \"sockets\": {{ \"allowedOrigins\": [\"{origin}\"] }} }}\n}}\n")).unwrap();
    std::fs::write(dir.join(".brain_env"), format!("[development]\nPUBLIC_SITE_URL={origin}\nBRAIN_BACKEND_URL=http://127.0.0.1:9\nBRAIN_SECRET=123b763507a73ff55d208f80e7d4aaa43fa61c602141941c\n")).unwrap();
    let mut p = Command::new(bin).arg("mcp").current_dir(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = p.stdin.take().unwrap();
    let mut out = BufReader::new(p.stdout.take().unwrap());
    let mut send = |id: u32, method: &str, params: &str| -> String {
        writeln!(stdin, "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":{params}}}").unwrap();
        loop {
            let mut l = String::new();
            assert!(out.read_line(&mut l).unwrap() > 0, "mcp closed");
            if l.contains(&format!("\"id\":{id},")) {
                return l;
            }
        }
    };
    send(1, "initialize", r#"{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}"#);
    let args = format!("{{\"name\":\"brain_run_start\",\"arguments\":{{\"port\":{port}}}}}");
    let first = send(2, "tools/call", &args);
    assert!(first.contains(&origin), "first start: {first}");
    let again = send(3, "tools/call", &args);
    assert!(again.contains(&origin), "already running: {again}");
    let _ = p.kill();
    let _ = std::fs::remove_dir_all(&tmp);
}
