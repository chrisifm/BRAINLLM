//! Fingerprint of the compiler sources (lex, parse, check, codegen, css) for `blocks::COMPILER_VERSION`: any change
//! to the compiler invalidates every persistent cache entry (INTERNALS §28), not only a version bump.

use std::path::Path;

fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, files);
        } else if p.extension().is_some_and(|x| matches!(x.to_str(), Some("rs" | "js" | "brain"))) {
            files.push(p);
        }
    }
}

fn main() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for c in ["brain-lex", "brain-parse", "brain-check", "brain-codegen", "brain-css"] {
        let src = here.join("..").join(c).join("src");
        println!("cargo:rerun-if-changed={}", src.display());
        walk(&src, &mut files);
        for name in ["Cargo.toml", "build.rs"] {
            let p = here.join("..").join(c).join(name);
            if p.is_file() { files.push(p); }
        }
    }
    // Checker-generated primitive exports and kit hashes depend on these embedded files too.
    let runtime = here.join("../../runtime");
    println!("cargo:rerun-if-changed={}", runtime.display());
    if let Ok(entries) = std::fs::read_dir(&runtime) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "js") { files.push(p); }
        }
    }
    walk(&runtime.join("ui"), &mut files);
    files.push(here.join("../../Cargo.toml"));
    files.push(here.join("../../Cargo.lock"));
    files.sort();
    let mut h: u64 = 0xcbf29ce484222325;
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        // Include relative names (renames matter), but no build-machine path or clock.
        let relative = f.strip_prefix(here.join("../..")).unwrap_or(f).to_string_lossy();
        // All paths above are constructed from `here`; normalize the prefix text without canonicalizing symlinks.
        let relative = relative.strip_prefix(&format!("{}/", here.display())).unwrap_or(&relative);
        for b in relative.bytes().chain([0xff]).chain(std::fs::read(f).expect("compiler fingerprint input")).chain([0xff]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    println!("cargo:rustc-env=BRAIN_COMPILER_FP={h:016x}");
}
