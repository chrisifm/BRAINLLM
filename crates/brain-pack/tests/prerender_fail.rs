//! QA #176: a component that throws while the build prerenders its page shows its fallback in the HTML (brain-guard,
//! §42) and the build reports it as a warning naming the component, so the author learns about it.
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-fail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::create_dir_all(d.join("comp")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("comp/inner.brain"), "<script>\nconst boom = () => { throw Error('inner exploded') }\nlet v = sig(boom())\n</script>\n<p>{v}</p>").unwrap();
    std::fs::write(d.join("views/pre.brain"), "<meta seo title=\"Pre\" description=\"Pre\">\n<script>\nimport Inner from '../comp/inner.brain'\n</script>\n<main><h1>Pre</h1><Inner /></main>").unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn a_failing_component_is_reported() {
    let node = std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into());
    if std::process::Command::new(&node).arg("-v").output().is_err() {
        eprintln!("skipped: no Node");
        return;
    }
    let root = app();
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let core = brain_min::minify_runtime_core(&std::fs::read_to_string(rt.join("brain.js")).unwrap()).unwrap();
    let siblings: Vec<(String, String)> = std::fs::read_dir(&rt)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "js") && !p.ends_with("brain.js") && !p.ends_with("brain-dev.js") && !p.ends_with("brain-inspect.js"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    let r2 = root.clone();
    let compile = move |src: &str, rel: &Path| -> Result<brain_pack::Unit, String> {
        let opts = brain_codegen::Options { dev: false, root: Some(r2.clone()), ..Default::default() };
        let o = brain_codegen::compile_with(src, rel, &opts).map_err(|d| format!("{}: {:?}", rel.display(), d.iter().map(|x| &x.msg).collect::<Vec<_>>()))?;
        Ok(brain_pack::Unit { js: o.js, css: o.css, server_js: o.server_js, server_fns: o.server_fns })
    };
    let input = brain_pack::Input {
        root: root.clone(),
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
    let views = vec![("/pre".to_string(), "views/pre.brain".to_string())];
    let out = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    let html = &out.files["pre/index.html"];
    assert!(html.contains("<h1>Pre</h1>"), "the page still renders: {html}");
    assert!(
        out.warnings.iter().any(|w| w.contains("component failed during prerender of /pre") && w.contains("inner") && w.contains("inner exploded")),
        "{:?}",
        out.warnings
    );
    let _ = std::fs::remove_dir_all(&root);
}
