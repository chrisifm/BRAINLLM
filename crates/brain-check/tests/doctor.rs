//! #36 (INTERNALS §18): app-wide overengineering findings on QA's doctor-app fixture.

use std::path::Path;

#[test]
fn doctor_app_matches_expected() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/fixtures-pending");
    let root = base.join("doctor-app");
    let Ok(expected) = std::fs::read_to_string(root.join("doctor.expected.json")) else { return };
    let got = brain_check::doctor::doctor(&root);
    let got: Vec<String> = got.iter().map(|d| format!("{} {} doctor-app/{} {}:{}", d.code, d.level.as_str(), d.file, d.line, d.col)).collect();
    // Expected entries, read without a JSON library: code, level, file, line, col in that order per object.
    let field = |obj: &str, k: &str| -> String {
        let i = obj.find(&format!("\"{k}\":")).unwrap_or(0) + k.len() + 3;
        let rest = obj[i..].trim_start();
        let end = if rest.starts_with('"') { rest[1..].find('"').map(|e| e + 2).unwrap_or(rest.len()) } else { rest.find([',', '\n', '}']).unwrap_or(rest.len()) };
        rest[..end].trim().trim_matches('"').to_string()
    };
    let want: Vec<String> = expected
        .split("\"code\"")
        .skip(1)
        .map(|o| format!("\"code\"{o}"))
        .map(|o| format!("{} {} {} {}:{}", field(&o, "code"), field(&o, "level"), field(&o, "file"), field(&o, "line"), field(&o, "col")))
        .collect();
    let mut g = got.clone();
    let mut w = want.clone();
    g.sort();
    w.sort();
    assert_eq!(g, w);
}
