//! Native log lines stay whole when several processes share one pipe, and nothing in the crate writes around the helper.
use std::path::Path;

const CHILD: &str = "BRAIN_LOG_LINES_CHILD";
const LINES: usize = 3000;

#[test]
fn log_lines_from_two_processes_never_interleave() {
    if std::env::var_os(CHILD).is_some() {
        for n in 0..LINES {
            let value = serde_json::json!({"schemaVersion":1,"event":"probe","n":n,"text":"x".repeat(120),"list":[1,2,3,4,5,6,7,8],"nested":{"a":true,"b":null}});
            brain_backend::log_line!("{value}");
        }
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let name = "log_lines_from_two_processes_never_interleave";
    let script = format!("\"$0\" --exact {name} --nocapture --test-threads=1 >/dev/null & \"$0\" --exact {name} --nocapture --test-threads=1 >/dev/null & wait");
    let out = std::process::Command::new("sh").args(["-c", &script]).arg(&exe).env(CHILD, "1").stdin(std::process::Stdio::null()).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stderr).unwrap();
    let mut seen = 0;
    for line in text.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
        assert_eq!(parsed["event"], "probe");
        seen += 1;
    }
    assert_eq!(seen, LINES * 2);
}

#[test]
fn nothing_in_the_crate_writes_a_log_line_around_the_helper() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        assert!(!text.contains(concat!("eprint", "ln!")), "{} writes around log_line!", entry.path().display());
    }
}
