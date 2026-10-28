//! AST edits validate a private snapshot before changing source and never overwrite other components.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
struct App(PathBuf);
impl App {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("brain-edit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let (ok, out) = brain(&dir, &["new", "app"]); assert!(ok, "{out}");
        Self(dir)
    }
    fn root(&self) -> PathBuf { self.0.join("app") }
    fn read(&self, path: &str) -> String { std::fs::read_to_string(self.root().join(path)).unwrap() }
}
impl Drop for App { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn brain(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    (output.status.success(), String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr))
}
#[test]
fn typed_props_add_remove_and_invalid_edits_leave_source_intact() {
    let app = App::new("props");
    let original = app.read("comp/counter.brain");
    let (ok, out) = brain(&app.root(), &["edit", "add-prop", "comp/counter.brain", "hint:str?", "--json"]);
    assert!(ok && out.contains("\"checked\":true"), "{out}");
    assert!(app.read("comp/counter.brain").contains("hint:str?"));
    let (ok, out) = brain(&app.root(), &["edit", "remove-prop", "comp/counter.brain", "hint", "--json"]);
    assert!(ok, "{out}");
    let current = app.read("comp/counter.brain");
    assert_eq!(original, current);
    for args in [vec!["edit", "remove-prop", "comp/counter.brain", "start"], vec!["edit", "add-prop", "comp/counter.brain", "start:num"], vec!["edit", "add-prop", "comp/counter.brain", "a:num b:num"]] {
        let (ok, out) = brain(&app.root(), &args); assert!(!ok, "{out}");
        assert_eq!(app.read("comp/counter.brain"), current);
        assert!(std::fs::File::open(app.root().join(".brain/edit.lock")).unwrap().try_lock().is_ok());
    }
}
#[test]
fn move_and_rename_update_resolved_imports_preserving_local_aliases() {
    let app = App::new("move");
    let counter = app.read("comp/counter.brain");
    let (ok, out) = brain(&app.root(), &["edit", "move", "comp/counter.brain", "comp/nested/counter.brain", "--json"]);
    assert!(ok, "{out}");
    assert!(!app.root().join("comp/counter.brain").exists());
    assert_eq!(app.read("comp/nested/counter.brain"), counter);
    let (ok, out) = brain(&app.root(), &["check", "--json"]); assert!(ok, "{out}");
    let (ok, out) = brain(&app.root(), &["edit", "rename", "comp/nested/counter.brain", "ClickCount", "--json"]);
    assert!(ok, "{out}");
    assert!(app.root().join("comp/nested/click-count.brain").exists());
    let (ok, out) = brain(&app.root(), &["check", "--json"]); assert!(ok, "{out}");
}
#[test]
fn extract_uses_checked_split_and_destinations_are_protected() {
    let app = App::new("extract");
    std::fs::write(app.root().join("comp/panel.brain"), "<section><p class=\"note\">Hello</p></section>\n").unwrap();
    let (ok, out) = brain(&app.root(), &["edit", "extract", "comp/panel.brain", "p.note", "NoteLine", "--json"]);
    assert!(ok, "{out}");
    assert!(app.read("comp/panel.brain").contains("NoteLine"));
    let before = app.read("comp/panel.brain");
    let destination = app.read("comp/note-line.brain");
    for dest in ["comp/note-line.brain", "../outside.brain", "views/panel.brain"] {
        let (ok, out) = brain(&app.root(), &["edit", "move", "comp/panel.brain", dest]); assert!(!ok, "{out}");
        assert_eq!(app.read("comp/panel.brain"), before);
        assert_eq!(app.read("comp/note-line.brain"), destination);
    }
    #[cfg(unix)] {
        std::os::unix::fs::symlink(app.0.join("outside"), app.root().join("comp/escape")).unwrap();
        let (ok, out) = brain(&app.root(), &["edit", "move", "comp/panel.brain", "comp/escape/panel.brain"]);
        assert!(!ok, "{out}");
        assert!(!app.0.join("outside").exists());
        assert_eq!(app.read("comp/panel.brain"), before);
    }
}
#[test]
fn mcp_ast_edits_preserve_legacy_text_edits_and_undo() {
    use std::io::Write;
    let app = App::new("mcp");
    let before = app.read("comp/counter.brain");
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain")).arg("mcp").current_dir(app.root()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    for message in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"edit-test","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"brain_edit","arguments":{"op":"add-prop","path":"comp/counter.brain","declaration":"hint:str?"}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"brain_edit","arguments":{"path":"comp/counter.brain","old":"'Clicks'","new":"'Taps'"}}}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"brain_undo","arguments":{"steps":2,"confirm":true}}}"#,
    ] { writeln!(input, "{message}").unwrap(); }
    drop(input);
    let output = child.wait_with_output().unwrap();
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{out}");
    assert!(!out.contains("\"isError\":true"), "{out}");
    assert!(out.contains("\"checked\":true") && out.contains("\"replaced\":1"), "{out}");
    assert_eq!(app.read("comp/counter.brain"), before);
}

#[test]
fn failed_commit_rolls_back_and_lock_protects_another_edit() {
    let app = App::new("rollback");
    let original = app.read("comp/counter.brain");
    let caller = app.read("views/home.brain");
    std::fs::create_dir_all(app.root().join(".brain")).unwrap();
    std::fs::write(app.root().join(".brain/edit.lock"), "owned by another edit").unwrap();
    let lock = std::fs::File::open(app.root().join(".brain/edit.lock")).unwrap();
    lock.try_lock().unwrap();
    let (ok, out) = brain(&app.root(), &["edit", "add-prop", "comp/counter.brain", "hint:str?"]);
    assert!(!ok && out.contains("another edit"), "{out}");
    assert_eq!(app.read(".brain/edit.lock"), "owned by another edit");
    assert_eq!(app.read("comp/counter.brain"), original);
    drop(lock);
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        // Stage files remain writable; the live destination refuses the final commit.
        // The earlier source deletion must be rolled back.
        let destination = app.root().join("comp/nested");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o555)).unwrap();
        let (ok, out) = brain(&app.root(), &["edit", "move", "comp/counter.brain", "comp/nested/counter.brain"]);
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!ok && out.contains("edit write failed"), "{out}");
        assert_eq!(app.read("comp/counter.brain"), original);
        assert_eq!(app.read("views/home.brain"), caller);
        assert!(!destination.join("counter.brain").exists());
    }
    assert!(std::fs::File::open(app.root().join(".brain/edit.lock")).unwrap().try_lock().is_ok());
}
