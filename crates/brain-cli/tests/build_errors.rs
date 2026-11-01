//! A build failure reports the same typed diagnostics as `brain check`, including those `brain check` cannot see.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn app(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("brain-build-errors-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("views")).unwrap();
    std::fs::write(dir.join("brain.json"), "{\"name\":\"t\"}\n").unwrap();
    std::fs::write(dir.join("app.brain"), "<slot />\n").unwrap();
    dir
}

fn build(dir: &Path) -> (Option<i32>, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_brain")).args(["build", "--json"]).current_dir(dir).stdin(std::process::Stdio::null()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let body = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    (out.status.code(), body)
}

const SEO: &str = "<meta seo title=\"Home title for readers\" description=\"Home description with enough words for the audit length rules.\" canonical=\"/\">\n";

#[test]
fn a_file_only_the_bundle_reaches_fails_with_its_location_and_real_fix() {
    let dir = app("bundle");
    std::fs::write(dir.join("views/index.brain"), format!("{SEO}<script>\n  import Hidden from '../.hidden/bad.brain'\n</script>\n<Hidden />\n")).unwrap();
    std::fs::create_dir_all(dir.join(".hidden")).unwrap();
    std::fs::write(dir.join(".hidden/bad.brain"), "<p>{missing}</p>\n").unwrap();
    let (code, body) = build(&dir);
    assert_eq!(code, Some(2));
    assert_eq!(body["phase"], "compile", "{body}");
    let d = &body["diagnostics"][0];
    assert_eq!((d["code"].as_str(), d["file"].as_str()), (Some("B0111"), Some(".hidden/bad.brain")), "{body}");
    assert!(d["line"].as_u64().unwrap() >= 1 && d["col"].as_u64().unwrap() >= 1, "{body}");
    assert!(!d["fix"].as_str().unwrap().is_empty() && d["fix"] != "run `brain check`", "{body}");
    assert!(d.get("generated").is_none(), "the file is the author's: {body}");
    assert!(body["error"].as_str().unwrap().contains("B0111"), "{body}");
    assert!(body["fix"].as_str().is_some_and(|f| !f.is_empty()), "{body}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_build_stopped_by_check_lists_every_error_typed() {
    let dir = app("check");
    std::fs::write(dir.join("views/index.brain"), "<h1>{one}</h1>\n<p>{two}</p>\n").unwrap();
    let (code, body) = build(&dir);
    assert_eq!(code, Some(2));
    assert_eq!(body["phase"], "check", "{body}");
    let codes: Vec<&str> = body["diagnostics"].as_array().unwrap().iter().map(|d| d["code"].as_str().unwrap()).collect();
    assert!(codes.iter().filter(|c| **c == "B0111").count() == 2, "{body}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failure_with_no_diagnostic_keeps_the_error_and_fix_pair() {
    let dir = app("plain");
    std::fs::write(dir.join("views/index.brain"), format!("{SEO}<h1>Hi</h1>\n")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_brain")).args(["build", "--json", "--site", "example.dev"]).current_dir(&dir).stdin(std::process::Stdio::null()).output().unwrap();
    let body: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(body["fix"], "`--site https://example.dev`", "{body}");
    assert_eq!(body["diagnostics"], serde_json::json!([]), "{body}");
    let _ = std::fs::remove_dir_all(&dir);
}
