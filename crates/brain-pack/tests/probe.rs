//! #284 (INTERNALS §30.2): interactive pages are stepped by RUNTIME's runaway probe after the prerender; a list that
//! grows with no input fails the build with B0907 naming the component file, a bounded clock does not.
use std::path::{Path, PathBuf};

fn app(tag: &str, view: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-probe-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), view).unwrap();
    d.canonicalize().unwrap()
}

fn build(root: &Path) -> Option<Result<brain_pack::Output, String>> {
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
        server_mjs: None,
        progress: None,
        post: None,
    };
    let views = vec![("/".to_string(), "views/home.brain".to_string())];
    Some(brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]))
}

#[test]
fn runaway_list_fails_the_build() {
    let root = app(
        "grow",
        "<meta seo title=\"Home\" description=\"Home\">\n<script>\n  let rows = sig([])\n  const grow = list => [...list, list.length]\n  fx(() => {\n    const id = setInterval(() => { rows = grow(rows) }, 100)\n    return () => clearInterval(id)\n  })\n</script>\n<main><h1>Home</h1><ul>{#each rows as r (r)}<li>{r}</li>{/each}</ul></main>",
    );
    let Some(r) = build(&root) else { return };
    let e = r.err().expect("a list growing with no input must fail the build");
    assert!(e.contains("B0907") && e.contains("views/home.brain"), "{e}");
}

#[test]
fn bounded_clock_passes() {
    let root = app(
        "clock",
        "<meta seo title=\"Home\" description=\"Home\">\n<script>\n  let n = sig(0)\n  fx(() => {\n    const id = setInterval(() => { n++ }, 1000)\n    return () => clearInterval(id)\n  })\n</script>\n<main><h1>Home</h1><p>{n}</p></main>",
    );
    let Some(r) = build(&root) else { return };
    let out = r.unwrap_or_else(|e| panic!("{e}"));
    assert!(out.files.contains_key("index.html"));
}
