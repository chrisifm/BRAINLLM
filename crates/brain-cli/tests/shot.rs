//! Visual regression exercises real Chrome; no image-library or synthetic pixel comparator.
#![cfg(unix)]
use brain_mcp::Json;
use std::path::{Path,PathBuf};
use std::process::{Command,Stdio};

struct App(PathBuf);
impl Drop for App { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
fn run(root:&Path,args:&[&str])->(bool,String) {
    let child=Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let prefix=format!("brain-test-{}-",child.id());
    let out=child.wait_with_output().unwrap();
    let leaked=std::fs::read_dir(std::env::temp_dir()).unwrap().filter_map(Result::ok).any(|e|e.file_name().to_string_lossy().starts_with(&prefix));
    assert!(!leaked,"owned Chrome profile was not removed: {prefix}");
    (out.status.success(),String::from_utf8_lossy(&out.stdout).into_owned()+&String::from_utf8_lossy(&out.stderr))
}
#[test]
fn real_png_baseline_mismatch_budget_text_and_owned_browser_cleanup() {
    let work=std::env::temp_dir().join(format!("brain-shot-integration-{}",std::process::id()));
    std::fs::create_dir_all(&work).unwrap();let app=App(work);
    let(ok,out)=run(&app.0,&["new","app"]);assert!(ok,"{out}");let root=app.0.join("app");
    std::fs::write(root.join("views/home.brain"),r#"<meta seo title="Visual test" description="A stable visual regression test page." canonical="/">
<h1>Visual baseline</h1>"#).unwrap();
    let args=["shot","/","--width","320","--height","240","--baseline","home","--json"];
    let mut update=args.to_vec();update.push("--update-baseline");
    let(ok,out)=run(&root,&update);
    if out.contains("Chrome not found") {eprintln!("skipped: Chrome not installed");return;}
    assert!(ok,"{out}");assert!(out.contains("Visual baseline")&&out.contains("\"updated\":true"),"{out}");
    let(ok,out)=run(&root,&args);assert!(ok&&out.contains("\"ratio\":0"),"{out}");
    std::fs::write(root.join("views/home.brain"),r#"<meta seo title="Visual test" description="A stable visual regression test page." canonical="/">
<h1>A changed visual baseline</h1>"#).unwrap();
    let(ok,out)=run(&root,&args);assert!(!ok&&out.contains("diffFile"),"{out}");
    let value=Json::parse(&out).unwrap();
    let Json::Arr(shots)=value.get("shots").unwrap() else {panic!("missing shots")};
    let text=shots[0].get("textFile").unwrap().as_str().unwrap();
    assert!(std::fs::read_to_string(text).unwrap().contains("A changed visual baseline"));
    let diff=shots[0].get("comparison").unwrap().get("diffFile").unwrap().as_str().unwrap();assert!(std::fs::metadata(diff).unwrap().len()>0);
    let mut tolerant=args.to_vec();tolerant.extend(["--max-diff","1"]);
    let(ok,out)=run(&root,&tolerant);assert!(ok,"{out}");
}

#[test]
fn mcp_baselines_are_explicit_and_managed_browser_stops_cleanly() {
    use std::io::Write;
    let work=std::env::temp_dir().join(format!("brain-shot-mcp-{}",std::process::id()));
    std::fs::create_dir_all(&work).unwrap();let app=App(work);
    assert!(run(&app.0,&["new","app"]).0);let root=app.0.join("app");
    let mut child=Command::new(env!("CARGO_BIN_EXE_brain")).arg("mcp").current_dir(&root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let pid=child.id();let mut input=child.stdin.take().unwrap();
    for message in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"shot-test","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"brain_run_start","arguments":{"port":0}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"brain_shot","arguments":{"route":"/about","width":320,"height":240,"baseline":"about","updateBaseline":true}}}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"brain_shot","arguments":{"route":"/about","width":320,"height":240,"baseline":"about","maxDiff":0}}}"#,
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"brain_run_stop","arguments":{}}}"#,
    ] {writeln!(input,"{message}").unwrap();}
    drop(input);let output=child.wait_with_output().unwrap();let out=String::from_utf8_lossy(&output.stdout);
    if out.contains("Chrome not found"){eprintln!("skipped: Chrome not installed");return;}
    assert!(output.status.success()&&!out.contains("\"isError\":true"),"{out}");
    assert!(out.contains("\"updated\":true")&&out.contains("\"ratio\":0")&&out.contains("textFile"),"{out}");
    assert!(root.join("baselines/about-320x240.png").exists());
    let prefix=format!("brain-test-{pid}-");
    assert!(!std::fs::read_dir(std::env::temp_dir()).unwrap().filter_map(Result::ok).any(|e|e.file_name().to_string_lossy().starts_with(&prefix)),"managed Chrome profile leaked");
}
