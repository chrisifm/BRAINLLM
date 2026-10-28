//! #78: every code the compiler emits has exactly one catalog row, in English.

use std::collections::HashSet;
use std::path::Path;

#[test]
fn catalog_covers_every_emitted_code_once() {
    let mut seen = HashSet::new();
    for (code, title, why, ok) in brain_check::CATALOG.iter() {
        assert!(seen.insert(*code), "duplicate {code}");
        assert!(code.len() == 5 && code.starts_with('B'), "{code}");
        assert!(!title.is_empty() && !why.is_empty() && !ok.is_empty(), "{code}");
    }
    let mut sorted: Vec<&str> = brain_check::CATALOG.iter().map(|r| r.0).collect();
    let before = sorted.clone();
    sorted.sort();
    assert_eq!(before, sorted, "keep CATALOG sorted by code");
    // Every literal code in the compiler sources and every QA fixture code is in the catalog.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sources = Vec::new();
    // #185: every crate and the runtime (one table for Rust and JS); B18xx belong to brain-seo's own table.
    for dir in ["crates/brain-lex/src", "crates/brain-parse/src", "crates/brain-check/src", "crates/brain-css/src", "crates/brain-codegen/src", "crates/brain-cli/src", "crates/brain-serve/src", "crates/brain-pack/src", "crates/brain-min/src", "crates/brain-mcp/src", "runtime", "examples/invalid"] {
        let mut stack = vec![root.join(dir)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().is_some_and(|n| n == "node_modules" || n == "test") {
                    continue;
                } else if p.extension().is_some_and(|x| x == "rs" || x == "json" || x == "js") && !p.ends_with("catalog.rs") && !p.ends_with("codes.rs") {
                    sources.push(std::fs::read_to_string(&p).unwrap());
                }
            }
        }
    }
    for src in sources {
        for q in ['"', '\'', '`'] {
            for (i, _) in src.match_indices(&format!("{q}B")) {
                let code = &src[i + 1..(i + 6).min(src.len())];
                if code.len() == 5 && code[1..].chars().all(|c| c.is_ascii_digit()) && src[i + 6..].starts_with(q) && !code.starts_with("B18") {
                    assert!(seen.contains(code), "{code} is emitted but missing from CATALOG");
                }
            }
        }
    }
    assert_eq!(brain_check::explain("B0302").unwrap().1, "Unknown design token");
}

#[test]
fn default_levels_come_from_the_single_table() {
    // #185: `Diag::new` takes its level from brain_lex::codes::CODES (no second list).
    use brain_parse::{codes::default_level, Severity};
    assert_eq!(default_level("B0201"), Severity::Warning);
    assert_eq!(default_level("B0722"), Severity::Info);
    assert_eq!(default_level("B0702"), Severity::Error);
    let d = brain_parse::Diag::new("B1005", "x.brain", Default::default(), "m", "f");
    assert!(d.is_warning() && !d.is_info());
}
