//! `cargo run -p brain-check --example check -- <app dir>`: parse + check every `.brain`/`.jsbrain` under a
//! directory and print the diagnostics as JSON lines (paths relative to it). Compiler-only; no CLI needed.

use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|e| e == "brain" || e == "jsbrain") {
            out.push(p);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let mut files = Vec::new();
    walk(&root, &mut files);
    for abs in files {
        let rel = abs.strip_prefix(&root).unwrap();
        let src = std::fs::read_to_string(&abs).unwrap();
        let diags = match brain_parse::parse(&src, rel) {
            Ok(c) => {
                let (ctx, _) = brain_check::load_ctx(&abs, &c);
                brain_check::check(&c, &ctx)
            }
            Err(d) => d,
        };
        for d in diags {
            println!("{} {}:{}:{} [{}] {}", d.code, d.file, d.line, d.col, d.level.as_str(), d.msg);
        }
    }
}
