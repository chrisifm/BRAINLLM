//! QA (defect sent to TOOLCHAIN, no id): a `brain_eval` whose expression navigates while it is pending (a submit that
//! calls navigate(), a `location` change) used to hang the MCP server. It must answer within the deadline with the
//! navigation explained, and the server keeps answering.
#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn navigation_during_eval_never_hangs_the_mcp_server() {
    let tmp = std::env::temp_dir().join(format!("bevnav-{}", std::process::id()));
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
    for expr in [
        // a full navigation destroys the context the expression runs in
        r#"(async () => { location.href = '/'; await new Promise(r => setTimeout(r, 5000)); return 1 })()"#,
        // the router's client navigation keeps it
        r#"(async () => { history.pushState({}, '', '/'); dispatchEvent(new PopStateEvent('popstate')); await new Promise(r => setTimeout(r, 200)); return 2 })()"#,
    ] {
        let t = Instant::now();
        let r = tool(&mut call, "brain_eval", &format!("{{\"expr\":{}}}", json_str(expr)));
        assert!(t.elapsed() < Duration::from_secs(40), "answered in {:?}: {r}", t.elapsed());
        assert!(r.contains("navigat") || r.contains("\\\"value\\\":2"), "{r}");
        // the server still answers
        let r = tool(&mut call, "brain_eval", r#"{"expr":"2+2"}"#);
        assert!(r.contains("\\\"value\\\":4"), "{r}");
    }
    assert!(tool(&mut call, "brain_run_stop", "{}").contains("stopped"));
    let _ = p.kill();
    let _ = p.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            _ => o.push(c),
        }
    }
    o.push('"');
    o
}
