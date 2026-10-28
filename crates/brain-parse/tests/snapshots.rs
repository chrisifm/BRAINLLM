//! Snapshot tests: every valid `examples/**/*.brain` parses and its AST dump matches
//! `tests/snapshots/<path>.ast`. Regenerate with `UPDATE_SNAPSHOTS=1 cargo test -p brain-parse`.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn brain_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "invalid") {
                continue;
            }
            brain_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "brain" || e == "jsbrain") {
            out.push(p);
        }
    }
}

#[test]
fn examples_match_snapshots() {
    let root = root();
    let examples = root.join("examples");
    let mut files = Vec::new();
    brain_files(&examples, &mut files);
    assert!(!files.is_empty(), "no examples/*.brain found");
    let update = std::env::var("UPDATE_SNAPSHOTS").is_ok();
    let snap_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let mut failures = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&examples).unwrap();
        let src = std::fs::read_to_string(&f).unwrap();
        let got = match brain_parse::parse(&src, rel) {
            Ok(c) => brain_parse::dump::dump(&c),
            Err(ds) => {
                failures.push(format!("{} no parsea:\n{}", rel.display(), ds.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n")));
                continue;
            }
        };
        // `*.test.brain` are QA's test files: they must parse, but their AST is not snapshotted.
        if rel.to_string_lossy().ends_with(".test.brain") {
            continue;
        }
        let snap = snap_dir.join(rel.to_string_lossy().replace('/', "__") + ".ast");
        if update || !snap.exists() {
            std::fs::write(&snap, &got).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&snap).unwrap();
        if want != got {
            failures.push(format!("{} differs from {}:\n--- expected\n{want}\n--- got\n{got}", rel.display(), snap.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
