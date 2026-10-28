//! `cargo run -p brain-pack --example sizes -- <app> <file.brain>...`: bytes of one compiled component in production
//! as emitted by codegen, minified without the short runtime names, and minified with RUNTIME_MAP. Prints TSV
//! (gzip is measured by the caller).
use std::path::{Path, PathBuf};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(&a[1]).canonicalize().unwrap();
    let out = PathBuf::from("/tmp/brain-sizes");
    std::fs::create_dir_all(&out).unwrap();
    for rel in &a[2..] {
        let src = std::fs::read_to_string(root.join(rel)).unwrap();
        let opts = brain_codegen::Options { dev: false, hash_len: 4, root: Some(root.clone()), ..Default::default() };
        let o = brain_codegen::compile_with(&src, Path::new(rel), &opts).unwrap_or_else(|_| panic!("{rel} does not compile"));
        let plain = brain_min::minify(&o.js, &brain_min::Options { mangle: true, drop_log: true, ..Default::default() }).unwrap();
        let short = brain_min::minify_module(&o.js, true).unwrap();
        let stem = rel.replace('/', "_");
        for (kind, body) in [("raw", &o.js), ("min", &plain), ("min+map", &short)] {
            let f = out.join(format!("{stem}.{kind}.js"));
            std::fs::write(&f, body).unwrap();
            println!("{rel}\t{kind}\t{}\t{}", body.len(), f.display());
        }
    }
}
