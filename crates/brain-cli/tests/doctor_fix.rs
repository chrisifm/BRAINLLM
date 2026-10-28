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
