//! Doctor fixes mechanical errors but leaves judgment calls to the app author.
use std::process::{Command, Stdio};
#[test]
fn doctor_fix_applies_exact_edits_and_retains_unresolved_errors() {
    let dir = std::env::temp_dir().join(format!("brain-doctor-fix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_brain")).args(["new", "app"]).current_dir(&dir).output().unwrap();
    assert!(result.status.success());
    let app = dir.join("app");
    let path = app.join("comp/needs-review.brain");
    std::fs::write(&path, "<script>var n = 1</script><p>{n}</p><img src=\"/image.png\">\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_brain")).args(["doctor", "--fix", "--json"]).current_dir(&app).stdin(Stdio::null()).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report = brain_mcp::Json::parse(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(report.get("ok"), Some(&brain_mcp::Json::Bool(false)));
    assert!(report.get("fix").is_some(), "{report:?}");
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(source.contains("let n = 1") && !source.contains("var n"));
    assert!(!source.contains("alt="), "must not invent an image description");
    assert!(report.stringify().contains("B0401"));
    std::fs::remove_dir_all(dir).unwrap();
}

/// An intermittent failure is gone by the next run, so the report itself has to name it.
#[test]
fn doctor_names_the_tests_that_failed() {
    let dir = std::env::temp_dir().join(format!("brain-doctor-tests-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(Command::new(env!("CARGO_BIN_EXE_brain")).args(["new", "app"]).current_dir(&dir).output().unwrap().status.success());
    let app = dir.join("app");
    std::fs::write(app.join("comp/zz.test.brain"), "<script>\ntest('always fails', () => { expect(1).toBe(2) })\n</script>\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_brain")).args(["doctor", "--json"]).current_dir(&app).stdin(Stdio::null()).output().unwrap();
    let report = brain_mcp::Json::parse(&String::from_utf8(output.stdout).unwrap()).unwrap();
    let text = report.get("test").map(|t| t.stringify()).unwrap_or_default();
    // Without a browser the tests do not run, and there is nothing to name.
    if text.contains("\"passed\"") {
        assert!(text.contains("comp/zz.test.brain") && text.contains("always fails") && text.contains("expected 2, got 1"), "{text}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
