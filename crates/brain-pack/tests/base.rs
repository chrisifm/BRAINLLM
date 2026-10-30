//! #148: brain.json `"base": "/app/"` moves every URL the build writes under the base (pages, chunks and the imports
//! between them, links in HTML and in compiled templates, public/ copies, the worker registration, _headers and the
//! server manifest), while dist/ paths stay as they are. Verified in Chrome by serving dist/ under /app/.
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-base-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::create_dir_all(d.join("public")).unwrap();
    std::fs::write(d.join("brain.json"), "{\"name\":\"x\",\"base\":\"/app/\"}").unwrap();
    std::fs::write(d.join("public/logo.svg"), format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>", "<rect width=\"1\" height=\"1\"/>".repeat(40))).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title><link rel=\"icon\" href=\"/logo.svg\"></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), "<meta seo title=\"Home\" description=\"Home\">\n<script>let n = sig(0)</script>\n<main><h1>Home</h1><a href=\"/about\">About</a><button on:click={() => n++}>{n}</button></main>").unwrap();
    std::fs::write(d.join("views/about.brain"), "<meta seo title=\"About\" description=\"About\">\n<main><h1>About</h1><a href=\"/\">Home</a><form action=\"/save\" method=\"post\"></form></main>").unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn every_url_lives_under_the_base() {
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
        site: Some("https://x.dev".into()),
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
    assert!(!out.files.keys().chain(out.binary.keys()).any(|k| k.starts_with("app/")), "dist/ paths never carry the base");
    for page in ["index.html", "about/index.html"] {
        let html = &out.files[page];
        assert!(html.contains("<meta name=\"brain-base\" content=\"/app/\">"), "{html}");
        // every root-relative URL in an attribute is under /app/
        for attr in ["href=\"", "src=\"", "action=\""] {
            for (i, _) in html.match_indices(attr) {
                let v = &html[i + attr.len()..];
                if v.starts_with('/') {
                    assert!(v.starts_with("/app/") || v.starts_with("/app\""), "{attr}{} in {page}", &v[..v.find('"').unwrap_or(20).min(40)]);
                }
            }
        }
        if html.contains("<meta name=\"brain-zero-js\" content=\"1\">") {
            assert!(!brain_pack::cache::has_executable_scripts(html), "static page executes JS: {html}");
            assert!(!html.contains("serviceWorker.register"), "static page must not register a worker: {html}");
        } else {
            assert!(html.contains("register('/app/sw.js',{scope:'/app/',"), "{html}");
        }
    }
    assert!(out.files["index.html"].contains("href=\"/app/about\"") && out.files["about/index.html"].contains("action=\"/app/save\""));
    assert!(!out.files["index.html"].contains("/app/logo.svg") && out.files["index.html"].contains("rel=\"icon\" href=\"/app/_b/"), "the icon points at the hashed copy");
    // chunks import each other under the base, and the client template's link is prefixed too
    for (k, v) in out.files.iter().filter(|(k, _)| k.ends_with(".js") && k.starts_with("_b/")) {
        for (_, _, spec) in brain_pack::specifiers(v).unwrap() {
            assert!(!spec.starts_with("/_b/"), "{k} imports {spec}");
        }
    }
    assert!(out.files.iter().any(|(k, v)| k.starts_with("_b/") && v.contains("href=/app/about")), "compiled template link under the base");
    assert!(out.files["sw.js"].contains("base:\"/app/\""));
    // QA #21 F7: compressible binaries get their .gz too (the original and the hashed copy)
    assert!(out.binary.contains_key("logo.svg.gz") && out.binary.keys().any(|k| k.starts_with("_b/") && k.ends_with(".svg.gz")), "{:?}", out.binary.keys().collect::<Vec<_>>());
    assert!(out.files["about/index.html"].contains("<link rel=\"canonical\" href=\"https://x.dev/app/about\">"), "{}", out.files["about/index.html"]);
    assert!(out.files["_headers"].lines().filter(|l| l.starts_with('/')).all(|l| l.starts_with("/app/")), "{}", out.files["_headers"]);
    let _ = std::fs::remove_dir_all(&root);
}
