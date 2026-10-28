//! #201 (§43): Δ bytes gz per view after a save. Editing one view changes only its row; a view with `load` and a
//! `server:` function is measured even though the measuring pipeline has no server runtime; the second
//! measurement reuses the build caches.
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-impact-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views/users")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), "<meta seo title=\"Home\" description=\"Home\">\n<script>let n = sig(0)</script>\n<main><h1>Home</h1><button on:click={() => n++}>{n}</button></main>").unwrap();
    std::fs::write(d.join("views/about.brain"), "<meta seo title=\"About\" description=\"About\">\n<main><h1>About</h1></main>").unwrap();
    std::fs::write(
        d.join("views/users/[id].brain"),
        "<meta seo title=\"User\" description=\"User\">\n<script>\nexport async function load(ctx) { return { id: ctx.params.id } }\nserver: async function ping(ctx) { return 1 }\nlet u = sig(props.id)\n</script>\n<main><h1>{u}</h1><button on:click={() => ping()}>Ping</button></main>",
    )
    .unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn one_edit_changes_one_view() {
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
        ("/users/:id".to_string(), "views/users/[id].brain".to_string()),
    ];
    let a = brain_pack::impact::measure(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.len(), 3, "{a:?}");
    assert!(a["views/home.brain"].js_gz > 0 && a["views/users/[id].brain"].js_gz > 0 && a["views/users/[id].brain"].route == "/users/:id", "{a:?}");
    std::fs::write(root.join("views/about.brain"), "<meta seo title=\"About\" description=\"About\">\n<main><h1>About us</h1><p>We build things for people who build things.</p></main>").unwrap();
    let t = std::time::Instant::now();
    let b = brain_pack::impact::measure(&input, &views, &|_| vec![], &|_| vec![]).unwrap();
    let took = t.elapsed();
    // #200: the page HTML of each measured view, prerendered for static ones, for the SEO/a11y deltas
    assert!(b["views/about.brain"].html.contains("We build things") && b["views/about.brain"].html.contains("<h1>About us"), "{}", b["views/about.brain"].html);
    let d = brain_pack::impact::diff(&a, &b);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].view, "views/about.brain");
    assert!(d[0].delta() > 0 && d[0].html > 0 && d[0].js == 0, "a static view changes its prerendered HTML only: {d:?}");
    assert!(took.as_millis() < 2000, "measuring after one edit took {took:?}");
    let _ = std::fs::remove_dir_all(&root);
}
