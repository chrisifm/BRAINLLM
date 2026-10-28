//! QA #172: (S13) the client router is a graph module, so its import of the core points at the hashed core chunk;
//! (S22) a view without handlers that reads a live primitive (`online()`) hydrates; (S23) `api/_http.jsbrain` runs
//! on every interactive page before it mounts, so its headers reach every api() call.
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-qa172-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::create_dir_all(d.join("api")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><meta http base=\"/api\" timeout=\"4s\" retry=\"2\" login=\"/login\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), "<meta seo title=\"Home\" description=\"Home\">\n<main><h1>Home</h1><a href=\"/about\">About</a><output>{online() ? 'online' : 'offline'}</output></main>\n<style>\n  main { padding: var(--space-4); }\n</style>").unwrap();
    std::fs::write(d.join("views/about.brain"), "<meta seo title=\"About\" description=\"About\">\n<main><h1>About</h1><a href=\"/\">Home</a></main>").unwrap();
    std::fs::write(d.join("views/deb.brain"), "<meta seo title=\"Deb\" description=\"Deb\">\n<script>\nimport { debounce } from '/@brain/brain-env.js'\nlet q = sig('')\nconst set = debounce((v) => { q = v }, 100)\n</script>\n<main><h1>Deb</h1><input aria-label=\"Q\" on:input={(e) => set(e.target.value)} /><p>{q}</p></main>").unwrap();
    std::fs::write(d.join("views/feed.brain"), "<meta seo title=\"Feed\" description=\"Feed\">\n<script>\nimport { mounted } from '/@brain/brain-env.js'\nlet n = sig(0)\nmounted(() => { n = 1 })\n</script>\n<main><h1>Feed</h1><p>{n}</p></main>").unwrap();
    std::fs::write(d.join("api/_http.jsbrain"), "export default http({ headers: () => ({ 'x-k': '1' }) })\n").unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn router_live_primitives_and_http_setup() {
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
        cache: Default::default(),
        server_mjs: None,
        progress: None,
        post: None,
    };
    let views = vec![
        ("/".to_string(), "views/home.brain".to_string()),
        ("/about".to_string(), "views/about.brain".to_string()),
        ("/deb".to_string(), "views/deb.brain".to_string()),
        ("/feed".to_string(), "views/feed.brain".to_string()),
    ];
    let out = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    // S13: every import of every chunk resolves to a file of dist/ (no `./brain.js` left in the router)
    for (k, v) in out.files.iter().filter(|(k, _)| k.starts_with("_b/") && k.ends_with(".js")) {
        for (_, _, spec) in brain_pack::specifiers(v).unwrap() {
            assert!(out.files.contains_key(spec.trim_start_matches('/')), "{k} imports {spec}, which dist/ does not have");
        }
    }
    let home = &out.files["index.html"];
    let router = home.split("import{router}from\"").nth(1).expect("router tag").split('"').next().unwrap();
    assert!(out.files.contains_key(router.trim_start_matches('/')), "{home}");
    // S22: online() makes the page load its code; a plain page still ships none of its own
    assert!(home.contains("e.$d=m(") && !home.contains("type=\"b/state\""), "a view reading online() mounts on load, not as an island: {home}");
    assert!(!out.files["about/index.html"].contains("e.$d=m("), "a plain page does not mount");
    // QA #21 F4: mounted() runs without a user event → mount on load; a pure helper (debounce) keeps the island
    assert!(out.files["feed/index.html"].contains("e.$d=m("), "{}", out.files["feed/index.html"]);
    assert!(out.files["deb/index.html"].contains("import{h}") && !out.files["deb/index.html"].contains("e.$d=m("), "{}", out.files["deb/index.html"]);
    // S23: the HTTP setup is imported first by the interactive page
    let http = out.files.iter().find(|(_, v)| v.contains("x-k")).map(|(k, _)| format!("/{k}")).expect("http setup chunk");
    let first_script = home.split("<script type=\"module\">").nth(1).unwrap();
    // S19/S20: app.brain's <meta http> policy first (its module calls http({…})), then api/_http.jsbrain
    let app = out.files.iter().find(|(_, v)| v.contains("base:\"/api\"") || v.contains("base: \"/api\"")).map(|(k, _)| format!("/{k}")).expect("app.brain policy chunk");
    assert!(first_script.starts_with(&format!("import\"{app}\";import\"{http}\";")), "{home}");
    for (k, v) in out.files.iter().filter(|(k, _)| k.ends_with(".html")) {
        assert!(!v.contains("<meta http"), "{k} keeps <meta http>");
    }
    // #35: the view's small stylesheet is inline inside the region (the router swaps it with the view), static
    // pages are listed for prerender on hover, and _headers carries the page's preloads as `Link` (early hints)
    let region = home.split("<div id=\"brain-view\">").nth(1).unwrap();
    assert!(region.starts_with("<style>"), "{home}");
    assert!(home.contains("<script type=\"speculationrules\">") && home.contains("\"href_matches\":[\"/\",\"/about\",\"/deb\",\"/feed\"]"), "{home}");
    let h = &out.files["_headers"];
    let root_rule = h.split("\n/\n").nth(1).unwrap().split("\n/").next().unwrap();
    assert!(root_rule.contains("  Link: <") && root_rule.contains("rel=modulepreload; integrity=\"sha384-"), "the preload carries the SRI of the tag, or the browser downloads twice (QA #21 F5): {h}");
    let _ = std::fs::remove_dir_all(&root);
}
