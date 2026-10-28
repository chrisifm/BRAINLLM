//! INTERNALS §20 in `brain build` (#47): the native linter runs on the whole app; errors stop the build with a
//! summary, warnings are listed with their count, and `brain.json "lint"` lowers or turns off quality rules only.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn build_lists_warnings_and_stops_on_errors_with_a_summary() {
    let tmp = std::env::temp_dir().join(format!("bblint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    // a warning (B0130) in a component nobody imports still shows up: the build lints the app, like `brain check`
    std::fs::write(dir.join("comp/warn.brain"), "<script>\n  let el = ref()\n  mounted(() => { log(el.v) })\n</script>\n\n<p bind:ref={el}>Warn</p>\n").unwrap();
    let o = brain(&dir, &["build"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("warning: B0130 comp/warn.brain:3:") && t.contains("1 warning ") || t.contains("1 warning\n") || t.contains("1 warning ·"), "{t}");
    // an error anywhere stops it: every error with file:line:col and fix, then the counts
    std::fs::write(dir.join("comp/bad.brain"), "<script>\n  let x = \n</script>\n<p>{x}</p>\n").unwrap();
    let o = brain(&dir, &["build"]);
    let t = text(&o);
    assert!(!o.status.success());
    assert!(t.contains("B0002 comp/bad.brain:3:1") && t.contains("; fix: ") && t.contains("build stopped: 1 error(s), 1 warning(s)"), "{t}");
    std::fs::remove_file(dir.join("comp/bad.brain")).unwrap();
    // "lint": a quality rule can be turned off; an override outside B09–B13 is ignored
    let cfg = std::fs::read_to_string(dir.join("brain.json")).unwrap();
    let with = |lint: &str| cfg.trim_end().trim_end_matches('}').trim_end().to_string() + &format!(",\n  \"lint\": {lint}\n}}\n");
    std::fs::write(dir.join("comp/loop.brain"), "<script>\n  let n = sig(0)\n  fx(() => { n = 1 })\n</script>\n<p>{n}</p>\n").unwrap();
    let base = text(&brain(&dir, &["check", "--json"]));
    let code = ["B1004", "B0901", "B1304"].into_iter().find(|c| base.contains(c));
    if let Some(code) = code {
        std::fs::write(dir.join("brain.json"), with(&format!("{{\"{code}\": \"off\", \"B0130\": \"off\"}}"))).unwrap();
        let t = text(&brain(&dir, &["check", "--json"]));
        assert!(!t.contains(code), "{code} turned off: {t}");
        assert!(t.contains("B0130"), "B0130 is not a quality rule: the override is ignored: {t}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
