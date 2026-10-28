//! examples/ must check clean; every examples/invalid/<case>.brain must produce exactly the Diags
//! of <case>.expected.json (code, file, line, col equal; msg and fix non-empty).

use std::path::{Path, PathBuf};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").canonicalize().unwrap()
}

fn brain_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            brain_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "brain") && !p.to_string_lossy().ends_with(".test.brain") {
            out.push(p);
        }
    }
}

fn run(abs: &Path) -> Vec<brain_parse::Diag> {
    let rel = abs.strip_prefix(examples()).unwrap();
    let src = std::fs::read_to_string(abs).unwrap();
    match brain_parse::parse(&src, rel) {
        Ok(c) => {
            let (ctx, _) = brain_check::load_ctx(abs, &c);
            brain_check::check(&c, &ctx)
        }
        Err(d) => d,
    }
}

/// Minimal reader for the flat objects of SPEC §5.
fn expected(json: &str) -> Vec<(String, String, u32, u32)> {
    let field = |obj: &str, k: &str| -> String {
        let i = obj.find(&format!("\"{k}\"")).unwrap_or_else(|| panic!("missing {k}")) + k.len() + 2;
        let rest = obj[i..].trim_start().strip_prefix(':').unwrap().trim_start();
        if let Some(r) = rest.strip_prefix('"') {
            let mut out = String::new();
            let mut esc = false;
            for c in r.chars() {
                if esc {
                    out.push(c);
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    break;
                } else {
                    out.push(c);
                }
            }
            out
        } else {
            rest.chars().take_while(|c| c.is_ascii_digit()).collect()
        }
    };
    json.match_indices("\"code\"")
        .map(|(i, _)| &json[i..])
        .map(|o| {
            assert!(!field(o, "msg").is_empty() && !field(o, "fix").is_empty());
            (field(o, "code"), field(o, "file"), field(o, "line").parse().unwrap(), field(o, "col").parse().unwrap())
        })
        .collect()
}

#[test]
fn valid_examples_are_clean() {
    let mut files = Vec::new();
    brain_files(&examples(), &mut files);
    let mut bad = Vec::new();
    for f in files.iter().filter(|f| !f.to_string_lossy().contains("/invalid/")) {
        let d = run(f);
        if !d.is_empty() {
            bad.push(d.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn invalid_fixtures_match_expected() {
    let mut files = Vec::new();
    brain_files(&examples().join("invalid"), &mut files);
    assert!(files.len() >= 13, "missing fixtures in examples/invalid");
    let mut bad = Vec::new();
    for f in files {
        let exp_path = f.with_extension("expected.json");
        let want = expected(&std::fs::read_to_string(&exp_path).unwrap());
        let got: Vec<_> = run(&f).into_iter().map(|d| {
            assert!(!d.msg.is_empty() && !d.fix.is_empty(), "empty msg/fix: {d}");
            (d.code.to_string(), d.file.clone(), d.line, d.col)
        }).collect();
        if got != want {
            bad.push(format!("{}\n  esperado {want:?}\n  obtenido {got:?}", f.display()));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
