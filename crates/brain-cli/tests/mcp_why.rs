//! INTERNALS §43 over MCP (#200): with the dev server and the debug page open, `brain_why` names the signal write
//! (file.brain:line) behind a node's update and `brain_impact` lists the updates per interaction.
#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn why_and_interactions_over_mcp() {
    let tmp = std::env::temp_dir().join(format!("bwhy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "app"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("app");
    std::fs::write(
        dir.join("views/why.brain"),
        "<meta seo title=\"Why\" description=\"A page with a counter, used to ask the dev tools which signal write changed a node.\">\n\n<script>\n  import Counter from '../comp/counter.brain'\n</script>\n\n<main class=\"page\">\n  <h1>Why</h1>\n  <Counter start={1} />\n</main>\n",
    )
    .unwrap();
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
    let nav = tool(&mut call, "brain_navigate", r#"{"route":"/why"}"#);
    if nav.contains("Chrome") && nav.contains("isError\":true") {
        eprintln!("skipped: no Chrome ({nav})");
        let _ = p.kill();
        return;
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    tool(&mut call, "brain_click", r#"{"selector":"main button"}"#);
    std::thread::sleep(std::time::Duration::from_millis(400));
    let why = tool(&mut call, "brain_why", r#"{"selector":"main button"}"#);
    assert!(why.contains("comp/counter.brain:9") && why.contains("\\\"signal\\\":\\\"n\\\""), "{why}");
    let imp = tool(&mut call, "brain_impact", r#"{"last":1}"#);
    assert!(imp.contains("\\\"type\\\":\\\"click\\\"") && imp.contains("\\\"mutations\\\""), "{imp}");
    tool(&mut call, "brain_run_stop", "{}");
    let _ = p.kill();
    let _ = p.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}
