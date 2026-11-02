//! `brain check` error corpus: each case in `corpus/check.txt` is a small app; the test runs the real command and compares
//! every B07xx, B14xx and B18xx finding, and every error of any family, with the expected code, file, line and column.
//! `@unreachable` names the codes `brain check` cannot produce from source files, with the reason: a new code in those
//! families must get a case or a reason, or this test fails.
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Default)]
struct Case {
    name: String,
    files: Vec<(String, String)>,
    expect: Vec<String>,
}

fn parse(src: &str) -> (Vec<Case>, Vec<(String, String)>) {
    let (mut cases, mut unreachable) = (Vec::<Case>::new(), vec![]);
    for line in src.lines() {
        if let Some(name) = line.strip_prefix("### ") {
            cases.push(Case { name: name.to_string(), ..Case::default() });
        } else if let Some(rest) = line.strip_prefix("@unreachable ") {
            let (code, why) = rest.split_once(' ').expect("@unreachable <code> <reason>");
            unreachable.push((code.to_string(), why.to_string()));
        } else if let Some(case) = cases.last_mut() {
            if let Some(path) = line.strip_prefix("@file ") {
                case.files.push((path.to_string(), String::new()));
            } else if let Some(want) = line.strip_prefix("@expect ") {
                case.expect.push(want.to_string());
            } else if let Some((_, body)) = case.files.last_mut() {
                body.push_str(line);
                body.push('\n');
            }
        }
    }
    (cases, unreachable)
}

fn in_scope(d: &Value) -> bool {
    let code = d["code"].as_str().unwrap_or("");
    code.starts_with("B07") || matches!(code, "B0808" | "B0809" | "B0810" | "B0811" | "B0812" | "B0813") || code.starts_with("B14") || code.starts_with("B18") || d["level"] == "error"
}

fn run(root: &Path, case: &Case) -> Vec<String> {
    let dir = root.join(case.name.replace(|c: char| !c.is_ascii_alphanumeric(), "-"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("brain.json"), "{ \"name\": \"Corpus\", \"csp\": { \"connectSrc\": [\"https://api.example.org\"] } }\n").unwrap();
    std::fs::write(dir.join("tokens.cssbrain"), ":root {\n}\n").unwrap();
    for (path, body) in &case.files {
        let at = dir.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, body).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_brain"))
        .args(["check", "--json"])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env_remove("BRAIN_BACKEND_URL")
        .env_remove("DATABASE_URL")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let report: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}: {text}", case.name));
    let diags = report.as_array().or(report["diagnostics"].as_array()).unwrap_or_else(|| panic!("{}: {text}", case.name));
    let mut got: Vec<String> = diags
        .iter()
        .filter(|d| in_scope(d))
        // A case with its own brain.json is about something else; only the case naming B0795 expects that warning.
        .filter(|d| d["code"] != "B0795" || case.expect.iter().any(|e| e.starts_with("B0795")))
        .map(|d| {
            assert!(d["msg"].as_str().is_some_and(|m| !m.is_empty()) && d["fix"].as_str().is_some_and(|f| !f.is_empty()), "{}: no msg or fix: {d}", case.name);
            format!("{} {} {}:{}", d["code"].as_str().unwrap(), d["file"].as_str().unwrap(), d["line"], d["col"])
        })
        .collect();
    got.sort();
    got
}

#[test]
fn every_case_reports_exactly_its_code_file_line_and_column() {
    let (cases, _) = parse(include_str!("corpus/check.txt"));
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("check-corpus-{}", std::process::id()));
    let mut bad = vec![];
    for case in &cases {
        let mut want = case.expect.clone();
        want.sort();
        let got = run(&root, case);
        if got != want {
            bad.push(format!("{}\n  want {want:?}\n  got  {got:?}", case.name));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(bad.is_empty(), "{} of {} cases differ:\n{}", bad.len(), cases.len(), bad.join("\n"));
}

#[test]
fn every_code_of_the_families_has_a_case_or_a_reason() {
    let (cases, unreachable) = parse(include_str!("corpus/check.txt"));
    let covered: BTreeSet<String> = cases.iter().flat_map(|c| c.expect.iter().map(|e| e.split(' ').next().unwrap().to_string())).collect();
    let excused: BTreeSet<String> = unreachable.iter().map(|(c, _)| c.clone()).collect();
    let mut missing = vec![];
    for (code, ..) in brain_parse::codes::CODES {
        if (code.starts_with("B07") || matches!(*code, "B0808" | "B0809" | "B0810" | "B0811" | "B0812" | "B0813") || code.starts_with("B14")) && !covered.contains(*code) && !excused.contains(*code) {
            missing.push(*code);
        }
    }
    assert!(missing.is_empty(), "no case and no @unreachable reason for {missing:?}");
    for code in &excused {
        assert!(!covered.contains(code), "{code} has a case, so it is not unreachable");
    }
}
