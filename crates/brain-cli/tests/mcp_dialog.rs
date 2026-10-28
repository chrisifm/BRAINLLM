//! QA #294: a modal dialog in the debug page (a dirty form's "Leave this page?", confirm, alert) used to block
//! `brain_eval` and, behind it, every other MCP tool forever. Now it is dismissed and reported in `dialogs`, and the
//! server keeps answering.
#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn a_dialog_never_blocks_the_mcp_server() {
    let tmp = std::env::temp_dir().join(format!("bdlg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "app"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("app");
    let mut p = Command::new(bin).arg("mcp").current_dir(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = p.stdin.take().unwrap();
    let mut out = BufReader::new(p.stdout.take().unwrap());
    let mut id = 0;
    let mut call = |method: &str, params: &str| -> String {
        id += 1;
        writeln!(stdin, "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":{params}}}").unwrap();
        loop {
            let mut l = String::new();
            assert!(out.read_line(&mut l).unwrap() > 0, "mcp closed");
            if l.contains(&format!("\"id\":{id},")) {
                return l;
            }
        }
    };
    call("initialize", r#"{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}"#);
    let tool = |call: &mut dyn FnMut(&str, &str) -> String, name: &str, args: &str| call("tools/call", &format!("{{\"name\":\"{name}\",\"arguments\":{args}}}"));
    assert!(tool(&mut call, "brain_run_start", r#"{"port":0}"#).contains("\\\"url\\\""));
    let nav = tool(&mut call, "brain_navigate", r#"{"route":"/about"}"#);
    if nav.contains("Chrome") && nav.contains("isError\":true") {
        eprintln!("skipped: no Chrome ({nav})");
        let _ = p.kill();
        return;
    }
    let t = Instant::now();
    let r = tool(&mut call, "brain_eval", r#"{"expr":"(async () => { const ok = confirm('Delete it?'); await new Promise(r => setTimeout(r, 100)); return ok })()"}"#);
    assert!(t.elapsed() < Duration::from_secs(20), "answered in {:?}", t.elapsed());
    assert!(r.contains("\\\"value\\\":false") && r.contains("confirm: Delete it?"), "{r}");
    // the server still answers, and the dialog is not reported twice
    let r = tool(&mut call, "brain_eval", r#"{"expr":"2+2"}"#);
    assert!(r.contains("\\\"value\\\":4") && !r.contains("dialogs"), "{r}");
    assert!(tool(&mut call, "brain_run_stop", "{}").contains("stopped"));
    let _ = p.kill();
    let _ = p.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}
