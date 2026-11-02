//! `brain fix` runs where other people (and agents) are working: it writes only inside the target it was given,
//! leaves info-level normalizations alone without a target, and never overwrites a file that already had
//! uncommitted changes unless `--force`.
use std::path::Path;
use std::process::{Command, Stdio};

fn brain(dir: &Path, args: &[&str]) -> brain_mcp::Json {
    let out = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).arg("--json").current_dir(dir).stdin(Stdio::null()).output().unwrap();
    brain_mcp::Json::parse(&String::from_utf8_lossy(&out.stdout)).unwrap_or_else(|_| panic!("{args:?}: {}", String::from_utf8_lossy(&out.stderr)))
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"]).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap().status.success();
    assert!(ok, "git {args:?}");
}

// `#1d1d1f` is the `--fg` value: an error-level edit. `padding: 8px` is `--space-2`: an info-level normalization.
const ERR: &str = "<p class=\"a\">x</p>\n<style>.a { color: #1d1d1f; }</style>\n";
const INFO: &str = "<p class=\"a\">x</p>\n<style>.a { padding: 8px; }</style>\n";

#[test]
fn fix_respects_the_target_info_level_and_uncommitted_work() {
    let root = std::env::temp_dir().join(format!("brain-fix-scope-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    assert!(Command::new(env!("CARGO_BIN_EXE_brain")).args(["new", "app"]).current_dir(&root).stdin(Stdio::null()).output().unwrap().status.success());
    let app = root.join("app");
    let write = |f: &str, s: &str| std::fs::write(app.join(f), s).unwrap();
    let read = |f: &str| std::fs::read_to_string(app.join(f)).unwrap();
    write("comp/err.brain", ERR);
    write("comp/info.brain", INFO);
    write("comp/other.brain", ERR);
    write("comp/theirs.brain", INFO);
    git(&app, &["init", "-q"]);
    git(&app, &["add", "-A"]);
    git(&app, &["commit", "-q", "-m", "base"]);
    // Someone else is editing this one: its uncommitted change must survive.
    write("comp/theirs.brain", &ERR.replace("x</p>", "draft</p>"));

    // With a target: only the target is written, info included.
    let j = brain(&app, &["fix", "comp/info.brain"]);
    assert!(read("comp/info.brain").contains("var(--space-2)"), "{}", j.stringify());
    assert_eq!(read("comp/other.brain"), ERR, "outside the target");
    assert_eq!(read("comp/err.brain"), ERR, "outside the target");

    // Without a target: errors are fixed, info-level normalizations are not, a dirty file is skipped and named.
    write("comp/info.brain", INFO);
    write("comp/theirs.brain", &ERR.replace("x</p>", "draft</p>"));
    let j = brain(&app, &["fix"]);
    assert!(read("comp/err.brain").contains("var(--fg)") && read("comp/other.brain").contains("var(--fg)"), "{}", j.stringify());
    assert_eq!(read("comp/info.brain"), INFO, "info normalizations wait for a target");
    assert!(read("comp/theirs.brain").contains("#1d1d1f") && read("comp/theirs.brain").contains("draft"), "someone else's uncommitted file");
    let skipped = j.get("skipped").map(|s| s.stringify()).unwrap_or_default();
    assert!(skipped.contains("comp/theirs.brain") && skipped.contains("--force"), "{skipped}");

    // A file the run itself fixed in an earlier round is not "someone else's"; `--force` writes the dirty file too.
    let j = brain(&app, &["fix", "--force"]);
    assert!(read("comp/theirs.brain").contains("var(--fg)") && read("comp/theirs.brain").contains("draft"), "{}", j.stringify());
    std::fs::remove_dir_all(&root).ok();
}
