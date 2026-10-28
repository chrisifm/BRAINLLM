//! RUNTIME #299 in real Chrome: a dirty `form()` whose submit navigates once the data is accepted leaves without the
//! "Leave this page?" prompt (router confirm or beforeunload); an unsaved form still asks.
#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn submit_that_navigates_does_not_ask_to_leave() {
    let tmp = std::env::temp_dir().join(format!("bsubnav-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "app"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("app");
    std::fs::write(
        dir.join("views/new.brain"),
        "<meta seo title=\"New product\" description=\"Create a product\">\n<script>\n  let f = form({ name: 'str!' }, { submit: async (v) => { await new Promise(r => setTimeout(r, 50)); navigate('/about') } })\n</script>\n<main><h1>New product</h1><form on:submit={f.submit}><label for=\"name\">Name</label><input id=\"name\" name=\"name\" bind:value={f.v.name} /><button type=\"submit\">Save</button></form></main>\n",
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
    let nav = tool(&mut call, "brain_navigate", r#"{"route":"/new"}"#);
    if nav.contains("Chrome") && nav.contains("isError\":true") {
        eprintln!("skipped: no Chrome ({nav})");
        let _ = p.kill();
        return;
    }
    let check = tool(&mut call, "brain_check", "{}");
    assert!(!check.contains("\\\"level\\\":\\\"error\\\""), "{check}");
    let typed = tool(&mut call, "brain_type", r#"{"selector":"input[name=name]","text":"Lamp"}"#);
    assert!(!typed.contains("isError\":true"), "{typed}");
    let r = tool(&mut call, "brain_click", r#"{"selector":"button[type=submit]"}"#);
    assert!(!r.contains("dialogs"), "a submit that navigates must not ask: {r}");
    let mut path = String::new();
    for _ in 0..40 {
        path = tool(&mut call, "brain_eval", r#"{"expr":"location.pathname"}"#);
        if path.contains("/about") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(path.contains("/about"), "{path}");
    let r = tool(&mut call, "brain_eval", r#"{"expr":"1"}"#);
    assert!(!r.contains("dialogs"), "{r}");
    // control: an unsaved form still asks before a client navigation
    tool(&mut call, "brain_navigate", r#"{"route":"/new"}"#);
    tool(&mut call, "brain_type", r#"{"selector":"input[name=name]","text":"Desk"}"#);
    let r = tool(&mut call, "brain_eval", r#"{"expr":"(globalThis.__brainNavigate || ((u) => location.assign(u)))('/about'), 1"}"#);
    let later = tool(&mut call, "brain_eval", r#"{"expr":"location.pathname"}"#);
    assert!(r.contains("dialogs") || later.contains("dialogs") || later.contains("/new"), "{r} {later}");
    assert!(tool(&mut call, "brain_run_stop", "{}").contains("stopped"));
    let _ = p.kill();
    let _ = p.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}
