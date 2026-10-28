//! #181 (QA #124): a view that reads data while rendering is prerendered in its loading state (the build has no
//! origin: a real fetch would freeze an error into the page) and mounts on load so the client fetches; `public/`
//! paths named in JS point at the hashed copy, so the chunk's name changes with the asset.
use std::path::{Path, PathBuf};

const DATA: &str = r#"<meta seo title="Data" description="Data page">
<script>
let d = resource(() => fetch('/data.json').then(r => r.json()))
let logo = '/og.png'
</script>
<main><h1>Data</h1><img src="/og.png" alt="Static" width="10" height="10"><img src={logo} alt="Logo" width="10" height="10">{#await d}<p id="st">Loading</p>{:then v}<p id="st">Value {v.n}</p>{:catch e}<p id="st">Failed</p>{/await}</main>"#;

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-data-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::create_dir_all(d.join("public")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/data.brain"), DATA).unwrap();
    std::fs::write(d.join("public/og.png"), [137u8, 80, 78, 71, 1]).unwrap();
    std::fs::write(d.join("public/data.json"), "{\"n\":42}").unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn data_reads_stay_loading_and_public_paths_are_hashed_in_js() {
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
    let views = vec![("/data".to_string(), "views/data.brain".to_string())];
    let out = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    let html = &out.files["data/index.html"];
    assert!(html.contains("<p id=\"st\">Loading</p>") && !html.contains("Failed"), "prerender froze the fetch result: {html}");
    assert!(html.contains("e.$d=m(") && !html.contains("import{h}"), "a data page mounts on load, not as an island: {html}");
    let og = brain_pack::cache::hashed_url("/og.png", &[137u8, 80, 78, 71, 1]);
    let js_names_og = out.files.iter().any(|(k, v)| k.ends_with(".js") && v.contains(&og));
    let js_names_raw = out.files.iter().any(|(k, v)| k.ends_with(".js") && (v.contains("\"/og.png\"") || v.contains("=/og.png")));
    assert!(js_names_og && !js_names_raw, "public/ path in JS not rewritten");
    assert!(out.binary.contains_key(og.trim_start_matches('/')) && out.binary.contains_key("og.png"));
    let _ = std::fs::remove_dir_all(&root);
}
