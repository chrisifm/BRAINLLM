//! The command surface every command shares: `--help` never acts, a closed pipe never panics, and `brain add` lists
//! its kinds from the table it dispatches on. The commands are read from `main.rs`, so a new one is covered by itself.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_brain");

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("brain-surface-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every first-level command `main` dispatches on.
fn commands() -> Vec<String> {
    let source = include_str!("../src/main.rs");
    let body = &source[source.find("let res = match args.first()").unwrap()..source.find("Some(other) => Err(format!(\"unknown command").unwrap()];
    let mut found = Vec::new();
    for arm in body.lines().filter(|l| l.starts_with("        Some(") && l.contains("=>")) {
        for piece in arm.split("Some(").skip(1) {
            let piece = piece.trim_start_matches(|c: char| !c.is_ascii_lowercase() && c != '"' );
            let start = piece.find('"');
            let mut rest = &piece[start.unwrap_or(0)..];
            while let Some(name) = rest.strip_prefix('"').and_then(|r| r.split('"').next()) {
                found.push(name.to_string());
                let after = &rest[name.len() + 2..];
                let Some(next) = after.strip_prefix(" | ") else { break };
                rest = next;
            }
        }
    }
    found.retain(|c| !c.starts_with('-') && c != "help");
    found.sort();
    found.dedup();
    assert!(found.len() > 40, "the command walk lost the dispatch table: {found:?}");
    found
}

fn snapshot(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            if entry.path().is_dir() { stack.push(entry.path()) }
            out.push(entry.path());
        }
    }
    out.sort();
    out
}

fn help(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN).args(args).current_dir(dir).env("HOME", dir).env("XDG_CONFIG_HOME", dir).stdin(Stdio::null()).output().unwrap()
}

#[test]
fn help_on_every_command_prints_usage_and_touches_nothing() {
    let dir = temp("help");
    for command in commands() {
        for flag in ["--help", "-h"] {
            let out = help(&dir, &[&command, flag]);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success(), "`brain {command} {flag}` failed: {}{}", stdout, String::from_utf8_lossy(&out.stderr));
            assert!(stdout.contains("brain"), "`brain {command} {flag}` printed no usage: {stdout}");
            assert_eq!(snapshot(&dir), Vec::<PathBuf>::new(), "`brain {command} {flag}` created files");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_deeper_in_the_arguments_still_acts_on_nothing() {
    let dir = temp("deep");
    for args in [&["new", "demo", "--help"][..], &["add", "view", "x", "--help"], &["backend", "migrate", "--help"], &["theme", "glass", "-h"], &["jobs", "work", "--help"], &["build", "--report", "--help"], &["help", "build"]] {
        let out = help(&dir, args);
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("brain"), "{args:?}");
        assert_eq!(snapshot(&dir), Vec::<PathBuf>::new(), "{args:?} created files");
    }
    let usage = String::from_utf8_lossy(&help(&dir, &["build", "--help"]).stdout).to_string();
    assert!(usage.contains("brain build") && !usage.contains("brain seo"), "help is for one command: {usage}");
    let json = String::from_utf8_lossy(&help(&dir, &["build", "--help", "--json"]).stdout).to_string();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value["schemaVersion"] == 1 && value["command"] == "build" && value["usage"].as_str().unwrap().contains("brain build"), "{json}");
    // an unknown command stays an error
    assert_eq!(help(&dir, &["nosuchcommand", "--help"]).status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_closed_pipe_ends_every_command_quietly() {
    let dir = temp("pipe");
    let new = Command::new(BIN).args(["new", "app"]).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    assert!(new.status.success());
    let app = dir.join("app");
    let cases: &[&[&str]] = &[
        &["--help"], &["help"], &["--version"], &["--version", "--json"], &["build", "--help"], &["spec"], &["spec", "--all"], &["spec", "--json"],
        &["guide"], &["guide", "--json"], &["schema"], &["theme", "--list"], &["map"], &["map", "--json"], &["check"], &["check", "--json"],
        &["explain", "B0111"], &["explain", "B0111", "--json"], &["explain", "B9999"], &["i18n"], &["i18n", "--json"], &["card", "counter"], &["seo", "audit"],
        &["find", "counter"], &["symbols", "comp/counter.brain"], &["lib", "list"], &["quality"], &["history"], &["graph"], &["upgrade", "--dry-run", "--no-doctor"],
        &["nosuchcommand"], &["nosuchcommand", "--json"], &["add"], &["e2e", "probe"], &["feedback", "query"], &["manifest", "query"], &["docs", "--check"],
    ];
    for args in cases {
        // the read end is closed before the child starts, so its first write hits EPIPE however little it prints
        let (read, write) = std::io::pipe().unwrap();
        drop(read);
        let out = Command::new(BIN).args(*args).current_dir(&app).stdin(Stdio::null()).stdout(Stdio::from(write)).stderr(Stdio::piped()).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("internal brain error") && !stderr.contains("panicked") && !stderr.contains("Broken pipe"), "{args:?}: {stderr}");
        assert!(!matches!(out.status.code(), Some(3 | 101)), "{args:?}: {:?} {stderr}", out.status.code());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn add_usage_lists_every_kind_it_dispatches_on() {
    let dir = temp("add");
    let out = help(&dir, &["add"]);
    let usage = String::from_utf8_lossy(&out.stderr).to_string();
    for kind in ["view", "comp", "ui", "layout", "lib", "test", "channel", "service", "api", "type", "guard", "server", "hook", "resource", "job", "collection", "locale", "roles"] {
        assert!(usage.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == kind), "`brain add` usage omits `{kind}`: {usage}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
