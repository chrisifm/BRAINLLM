//! #26: `"ext": "brain"` publishes `.brain` pages, `.jsbrain` chunks and `.cssbrain` sheets (INTERNALS §10, opt-in;
//! `web` stays the default), with every reference, the prerender, the worker precache and the host configs (types in
//! nginx and Caddy, explicit Content-Type in _headers) following the extensions.
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::write(d.join("brain.json"), "{\"name\":\"x\",\"ext\":\"brain\"}").unwrap();
    std::fs::write(d.join("tokens.cssbrain"), ":root{--fg:#111}").unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), "<meta seo title=\"Home\" description=\"Home\">\n<script>let n = sig(0)</script>\n<main><h1>Home</h1><a href=\"/about\">About</a><button on:click={() => n++}>{n}</button></main>").unwrap();
    std::fs::write(d.join("views/about.brain"), "<meta seo title=\"About\" description=\"About\">\n<main><h1>About</h1></main>").unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn own_extensions_everywhere() {
    let root = app();
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let core = brain_min::minify_runtime_core(&std::fs::read_to_string(rt.join("brain.js")).unwrap()).unwrap();
    let siblings: Vec<(String, String)> = std::fs::read_dir(&rt)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "js") && !p.ends_with("brain-dev.js") && !p.ends_with("brain-inspect.js"))
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
        cache: brain_pack::cache::CacheCfg { sw_template: std::fs::read_to_string(rt.join("sw/brain-sw.js")).ok(), ..Default::default() },
        server_mjs: None,
        progress: None,
        post: None,
    };
    let views = vec![("/".to_string(), "views/home.brain".to_string()), ("/about".to_string(), "views/about.brain".to_string())];
    let out = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    let keys: Vec<&String> = out.files.keys().collect();
    assert!(out.files.contains_key("index.brain") && out.files.contains_key("about/index.brain"), "{keys:?}");
    assert!(!keys.iter().any(|k| k.ends_with(".html") || (k.starts_with("_b/") && (k.ends_with(".js") || k.ends_with(".css")))), "{keys:?}");
    assert!(keys.iter().any(|k| k.ends_with(".jsbrain")) && keys.iter().any(|k| k.ends_with(".cssbrain")), "{keys:?}");
    let home = &out.files["index.brain"];
    assert!(home.contains("<h1>Home</h1>"), "prerendered with .jsbrain chunks: {home}");
    for (_, _, spec) in out.files.iter().filter(|(k, _)| k.ends_with(".jsbrain")).flat_map(|(_, v)| brain_pack::specifiers(v).unwrap()) {
        assert!(spec.ends_with(".jsbrain") && out.files.contains_key(spec.trim_start_matches('/')), "{spec}");
    }
    assert!(out.files["sw.js"].contains(".jsbrain\"") && !out.files["sw.js"].contains(".js\","), "{}", &out.files["sw.js"][..300]);
    let h = &out.files["_headers"];
    assert!(h.contains(".jsbrain\n  Content-Type: text/javascript; charset=utf-8") && h.contains("/index.brain\n  Content-Type: text/html; charset=utf-8") && h.contains("\n/about/\n  Content-Type: text/html"), "{h}");
    let n = &out.files["_brain/nginx.conf"];
    assert!(n.contains("types {\n    text/html brain;\n    text/javascript jsbrain;\n    text/css cssbrain;\n}") && n.contains("index index.brain;") && n.contains("$uri/index.brain"), "{n}");
    let c = &out.files["_brain/Caddyfile"];
    assert!(c.contains("@jsbrain path *.jsbrain") && c.contains("index index.brain") && c.contains("Content-Security-Policy"), "{c}");
    let _ = std::fs::remove_dir_all(&root);
}
