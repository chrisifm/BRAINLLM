//! `brain guide`: task playbooks embedded in the binary, over the CLI (text and --json) and over MCP (`brain_guide`).
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

const TOPICS: &[&str] = &["start", "views", "data", "forms", "server", "seo", "a11y", "webmcp", "tests", "i18n", "performance"];

fn brain(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(std::env::temp_dir()).output().unwrap()
}

#[test]
fn guide_lists_every_topic_outside_an_app() {
    let o = brain(&["guide"]);
    assert!(o.status.success());
    let out = String::from_utf8_lossy(&o.stdout);
    for t in TOPICS {
        assert!(out.lines().any(|l| l.trim_start().starts_with(&format!("{t} "))), "missing `{t}` in:\n{out}");
    }
    let j = String::from_utf8_lossy(&brain(&["guide", "--json"]).stdout).to_string();
    assert!(j.contains("\"schemaVersion\":1") && j.contains("\"topic\":\"performance\""), "{j}");
}

#[test]
fn guide_seo_json_returns_the_playbook() {
    let o = brain(&["guide", "seo", "--json"]);
    assert!(o.status.success());
    let j = String::from_utf8_lossy(&o.stdout);
    assert!(j.contains("\"topic\":\"seo\"") && j.contains("target query") && j.contains("llms.txt") && j.contains("B1830"), "{j}");
    let text = String::from_utf8_lossy(&brain(&["guide", "seo"]).stdout).to_string();
    assert!(text.starts_with("# guide: seo"), "{text}");
}

#[test]
fn unknown_guide_is_a_usage_error_with_the_topics() {
    let o = brain(&["guide", "nope", "--json"]);
    assert_eq!(o.status.code(), Some(2));
    let j = String::from_utf8_lossy(&o.stdout);
    assert!(j.contains("unknown guide") && j.contains("start views data"), "{j}");
}

#[test]
fn help_lists_guide() {
    let out = String::from_utf8_lossy(&brain(&["--help"]).stdout).to_string();
    assert!(out.contains("brain guide [topic]"), "{out}");
}

#[test]
fn guide_over_mcp() {
    let mut p = Command::new(env!("CARGO_BIN_EXE_brain")).arg("mcp").current_dir(std::env::temp_dir()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
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
    let init = call("initialize", r#"{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}"#);
    assert!(init.contains("brain_guide"), "the server instructions point to brain_guide: {init}");
    let list = call("tools/list", "{}");
    assert!(list.contains("\"name\":\"brain_guide\""), "{list}");
    let all = call("tools/call", r#"{"name":"brain_guide","arguments":{}}"#);
    assert!(all.contains("webmcp") && !all.contains("\"isError\":true"), "{all}");
    let one = call("tools/call", r#"{"name":"brain_guide","arguments":{"topic":"webmcp"}}"#);
    assert!(one.contains("# guide: webmcp") && one.contains("tool-description"), "{one}");
    let bad = call("tools/call", r#"{"name":"brain_guide","arguments":{"topic":"nope"}}"#);
    assert!(bad.contains("\"isError\":true") && bad.contains("unknown guide"), "{bad}");
    drop(stdin);
    let _ = p.kill();
    let _ = p.wait();
}
