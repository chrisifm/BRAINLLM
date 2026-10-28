//! `brain theme <preset>` (§27): the app's tokens.cssbrain becomes the runtime's preset byte for byte.
use std::process::{Command, Stdio};

#[test]
fn theme_writes_the_preset_and_lists_them() {
    let tmp = std::env::temp_dir().join(format!("btheme-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    let run = |dir: &std::path::Path, args: &[&str]| Command::new(bin).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    assert!(run(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    let list = String::from_utf8(run(&dir, &["theme", "--list"]).stdout).unwrap();
    for p in ["glass", "dark", "paper", "contrast"] {
        assert!(list.lines().any(|l| l == p), "{list}");
    }
    assert!(run(&dir, &["theme", "dark"]).status.success());
    let want = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/themes/dark.cssbrain")).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("tokens.cssbrain")).unwrap(), want);
    assert!(run(&dir, &["check"]).status.success());
    assert!(!run(&dir, &["theme", "neon"]).status.success());
    let _ = std::fs::remove_dir_all(&tmp);
}
