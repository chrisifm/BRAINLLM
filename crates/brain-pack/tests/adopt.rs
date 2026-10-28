//! #287 (RUNTIME #285, COMPILER #143): with codegen in adoption mode the manifest says `adopt`, server.mjs gets
//! `_server/brain-hydrate.js`, the prerendered HTML is stamped (`data-hk`, `<!--[-->…<!--]-->`) and brain-hydrate.js
//! ships in _b/ for the islands. Without the mode nothing changes.
use std::path::{Path, PathBuf};

fn app(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-adopt-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(
        d.join("views/home.brain"),
        "<meta seo title=\"Home\" description=\"Home\">\n<script>\n  let n = sig(0)\n</script>\n<main><h1>Home</h1><button onclick={() => n++}>Add</button>{#if n > 1}<p>many</p>{/if}<p>{n}</p></main>",
    )
    .unwrap();
    // a request-reading load makes the app a server app: it gets server.mjs and _server/manifest.json
    std::fs::write(d.join("views/me.brain"), "<meta seo title=\"Me\" description=\"Me\">\n<script>\nexport async function load(ctx) { return { who: ctx.user } }\n</script>\n<main><h1>Me</h1></main>").unwrap();
    d.canonicalize().unwrap()
}

fn build(root: &Path, adopt: bool) -> Option<Result<brain_pack::Output, String>> {
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
        let opts = brain_codegen::Options { dev: false, adopt, root: Some(r2.clone()), ..Default::default() };
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
    let views = vec![("/".to_string(), "views/home.brain".to_string()), ("/me".to_string(), "views/me.brain".to_string())];
    Some(brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]))
}

#[test]
fn adopt_mode_stamps_and_ships_the_hydrator() {
    let Some(r) = build(&app("on"), true) else { return };
    let out = r.unwrap_or_else(|e| panic!("{e}"));
    let mf = &out.files["_server/manifest.json"];
    assert!(mf.contains("\"adopt\":true"), "{mf}");
    assert!(out.files.contains_key("_server/brain-hydrate.js"));
    let page = out.files.get("index.html").unwrap_or_else(|| panic!("{:?} {:?}", out.files.keys().collect::<Vec<_>>(), out.warnings));
    assert!(page.contains("data-hk=\"") && page.contains("<!--[-->"), "{page} {:?}", out.warnings);
    let hydrate = out.files.iter().find(|(k, v)| k.starts_with("_b/") && v.contains("data-hk")).map(|(k, _)| k.clone());
    assert!(hydrate.is_some(), "brain-hydrate.js is in _b/");
}

#[test]
fn default_mode_unchanged() {
    let Some(r) = build(&app("off"), false) else { return };
    let out = r.unwrap_or_else(|e| panic!("{e}"));
    assert!(!out.files["_server/manifest.json"].contains("\"adopt\""));
    assert!(out.files["index.html"].contains("<h1>Home</h1>"), "prerendered");
    assert!(!out.files["index.html"].contains("data-hk"));
    // #306: without adoption brain-hydrate.js is not shipped (brain-island loses its static import)
    assert!(!out.files.iter().any(|(k, v)| k.starts_with("_b/") && v.contains("export function adopter")), "brain-hydrate.js shipped without adopt");
}
