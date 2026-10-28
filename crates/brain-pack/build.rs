//! Fingerprint of the whole toolchain (bundler, minifier, embedded runtime and the compiler crates, the same set as
//! `COMPILER_VERSION`) for `BRAIN_PACK_FP`: the build cache in `.brain/cache/` keys on it, so any toolchain change
//! recomputes min, gzip and prerender output instead of serving stale entries (INTERNALS §28).

use std::path::Path;

fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "node_modules" || n == "test" || n == "tests") {
                continue;
            }
            walk(&p, files);
        } else if p.extension().is_some_and(|x| x == "rs" || x == "js") {
            files.push(p);
        }
    }
}

fn main() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for d in ["brain-pack/src", "brain-min/src", "../runtime", "brain-lex/src", "brain-parse/src", "brain-check/src", "brain-codegen/src", "brain-css/src"] {
        let src = here.join("..").join(d);
        println!("cargo:rerun-if-changed={}", src.display());
        walk(&src, &mut files);
    }
    files.sort();
    let mut h: u64 = 0xcbf29ce484222325;
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        for b in std::fs::read(f).unwrap_or_default().into_iter().chain([0xff]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    println!("cargo:rustc-env=BRAIN_PACK_FP={h:016x}");
}
