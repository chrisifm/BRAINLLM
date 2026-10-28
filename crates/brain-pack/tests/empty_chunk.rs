//! QA #292: a module whose only import was dropped by the minifier is neither preloaded nor written as an empty
//! chunk. Built on the frozen benchmark app (bench/build/app), where it showed up as a 0-byte `_b/` file.
use std::path::{Path, PathBuf};

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let n = e.file_name();
        if n == "dist" || n == ".brain" {
            continue;
        }
        if e.path().is_dir() { copy(&e.path(), &to.join(&n)) } else { std::fs::copy(e.path(), to.join(&n)).unwrap(); }
    }
}

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-empty-chunk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    copy(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/build/app"), &d);
    d.canonicalize().unwrap()
}

fn build(root: &Path, views: Vec<(String, String)>) -> Option<Result<brain_pack::Output, String>> {
    let node = std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into());
    if std::process::Command::new(&node).arg("-v").output().is_err() {
        eprintln!("skipped: no Node");
        return None;
    }
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let probe = std::fs::read_to_string(rt.join("shim/probe.js")).ok()?;
    let core = brain_min::minify_runtime_core(&std::fs::read_to_string(rt.join("brain.js")).unwrap()).unwrap();
    let mut siblings: Vec<(String, String)> = std::fs::read_dir(&rt)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "js") && !p.ends_with("brain-dev.js") && !p.ends_with("brain-inspect.js"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    siblings.push(("shim/probe.js".into(), probe));
    let r2 = root.to_path_buf();
    let compile = move |src: &str, rel: &Path| -> Result<brain_pack::Unit, String> {
        let opts = brain_codegen::Options { dev: false, root: Some(r2.clone()), ..Default::default() };
        let o = brain_codegen::compile_with(src, rel, &opts).map_err(|d| format!("{}: {:?}", rel.display(), d.iter().map(|x| &x.msg).collect::<Vec<_>>()))?;
        Ok(brain_pack::Unit { js: o.js, css: o.css, server_js: o.server_js, server_fns: o.server_fns })
    };
    let input = brain_pack::Input {
        root: root.to_path_buf(),
        compile: &compile,
        runtime_core: core,
        runtime_siblings: siblings,
        layer_order: String::new(),
        minify: true,
        site: None,
        shim: std::fs::read_to_string(rt.join("shim/dom.js")).ok(),
        generator: String::new(),
        server_module: None,
        cache: Default::default(),
        server_mjs: std::fs::read_to_string(rt.join("server/server.mjs")).ok(),
        progress: None,
        post: None,
    };
    Some(brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]))
}

#[test]
fn no_empty_chunk_no_dead_preload() {
    let root = app();
    let Some(r) = build(&root, vec![("/dashboard".to_string(), "views/dashboard.brain".to_string())]) else { return };
    let out = r.unwrap_or_else(|e| panic!("{e}"));
    for (k, v) in &out.files {
        assert!(!(k.starts_with("_b/") && v.trim().is_empty()), "empty chunk {k}");
    }
    for (k, v) in out.files.iter().filter(|(k, _)| k.ends_with(".html")) {
        for part in v.split("rel=\"modulepreload\" href=\"").skip(1) {
            let url = part.split('"').next().unwrap();
            assert!(out.files.contains_key(url.trim_start_matches('/')), "{k} preloads {url}, which is not in the build");
        }
    }
}
