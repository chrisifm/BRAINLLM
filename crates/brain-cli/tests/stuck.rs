//! Stuck-loop protection (INTERNALS §25): only failed fix attempts count. Re-checking an untouched file (CI, fixture
//! suites) never marks a diagnostic stuck; two edits that leave the same error at the same place do (QA).
use std::process::{Command, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn only_changed_files_count_as_fix_attempts() {
    let tmp = std::env::temp_dir().join(format!("bstuck-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    brain(&tmp, &["new", "app"]);
    let dir = tmp.join("app");
    let f = dir.join("views/broken.brain");
    std::fs::write(&f, "<main>\n  <img src=\"/a.png\">\n</main>\n").unwrap();
    for _ in 0..4 {
        let out = brain(&dir, &["check", "--json"]);
        assert!(!out.contains("\"stuck\":true"), "an untouched file is not a failed fix: {out}");
    }
    for i in 0..2 {
        let mut src = std::fs::read_to_string(&f).unwrap();
        src.push_str(&format!("<!-- attempt {i} -->\n"));
        std::fs::write(&f, src).unwrap();
        let out = brain(&dir, &["check", "--json"]);
        assert_eq!(out.contains("\"stuck\":true"), i == 1, "attempt {i}: {out}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
