//! Fingerprint of the whole toolchain (bundler, minifier, embedded runtime and the compiler crates, the same set as
//! `COMPILER_VERSION`) for `BRAIN_PACK_FP`: the build cache in `.brain/cache/` keys on it, so any toolchain change
//! recomputes min, gzip and prerender output instead of serving stale entries (INTERNALS §28).

use std::path::Path;
#[path = "src/sha.rs"]
pub mod fingerprint_sha;

fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .is_some_and(|n| n == "node_modules" || n == "test" || n == "tests")
            {
                continue;
            }
            walk(&p, files);
        } else if p.extension().is_some_and(|x| matches!(x.to_str(), Some("rs" | "js" | "mjs" | "brain" | "cssbrain" | "json"))) {
            files.push(p);
        }
    }
}

fn main() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for d in [
        "brain-pack/src",
        "brain-min/src",
        "../runtime",
        "brain-lex/src",
        "brain-parse/src",
        "brain-check/src",
        "brain-codegen/src",
        "brain-css/src",
    ] {
        let src = here.join("..").join(d);
        println!("cargo:rerun-if-changed={}", src.display());
        let mut local = Vec::new();
        walk(&src, &mut local);
        for file in local {
            let label = format!(
                "{d}/{}",
                file.strip_prefix(&src)
                    .expect("fingerprint path inside source")
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            files.push((label, file));
        }
    }
    for (label, file) in [
        ("pack-manifest", here.join("Cargo.toml")),
        ("pack-build", here.join("build.rs")),
        ("workspace-lock", here.join("../../Cargo.lock")),
    ] {
        files.push((label.to_string(), file));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut identity = Vec::new();
    for (label, file) in files {
        println!("cargo:rerun-if-changed={}", file.display());
        identity.extend_from_slice(&(label.len() as u64).to_be_bytes());
        identity.extend_from_slice(label.as_bytes());
        identity.extend_from_slice(&fingerprint_sha::sha256(
            &std::fs::read(file).expect("toolchain fingerprint input"),
        ));
    }
    let hash = fingerprint_sha::sha256(&identity)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    println!("cargo:rustc-env=BRAIN_PACK_FP={hash}");
}
