//! `cargo run -p brain-min --example runtime -- <runtime-dir> <out-dir>`: core with the short map
//! (brain.min.js), minified sibling modules and a `brain.js` shim for `BRAIN_RUNTIME=<out>/brain.js`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (rt, out) = (std::path::Path::new(&a[1]), std::path::Path::new(&a[2]));
    std::fs::create_dir_all(out).unwrap();
    let core = std::fs::read_to_string(rt.join("brain.js")).unwrap();
    std::fs::write(out.join("brain.min.js"), brain_min::minify_runtime_core(&core).unwrap_or_else(|e| panic!("{e}"))).unwrap();
    std::fs::write(out.join("brain.js"), brain_min::runtime_test_shim("brain.min.js")).unwrap();
    let mut siblings: Vec<_> = std::fs::read_dir(rt).unwrap().flatten().map(|e| e.path()).filter(|p| {
        p.extension().is_some_and(|x| x == "js") && p.file_name().is_some_and(|n| n != "brain.js")
    }).collect();
    siblings.sort();
    for p in siblings {
        let f = p.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(&p).unwrap();
        std::fs::write(out.join(&f), brain_min::minify_module(&src, false).unwrap_or_else(|e| panic!("{f}: {e}"))).unwrap();
    }
}
