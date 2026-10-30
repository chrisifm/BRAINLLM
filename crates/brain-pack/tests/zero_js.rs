//! Static routes execute no page JS, including in sites containing interactive/SSR routes.
use std::path::{Path, PathBuf};

struct App(PathBuf);
impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn build(mixed: bool, base: &str) -> (App, brain_pack::Output) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let app =
        App(std::env::temp_dir().join(format!("brain-zero-js-{}-{stamp}", std::process::id())));
    let root = &app.0;
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("brain.json"), format!("{{\"base\":\"{base}\"}}")).unwrap();
    std::fs::create_dir_all(root.join("views/items")).unwrap();
    std::fs::write(root.join("app.brain"),"<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Output</title></head><body><slot /></body></html>").unwrap();
    for (name, title) in [
        ("index", "Home"),
        ("about", "About"),
        ("_offline", "Offline"),
    ] {
        std::fs::write(root.join(format!("views/{name}.brain")),format!("<meta seo title=\"{title}\" description=\"{title} static page.\" robots=\"noindex\"><main><h1>{title}</h1><a href=\"/counter\">Counter</a></main>")).unwrap();
    }
    let mut views = vec![
        ("/".into(), "views/index.brain".into()),
        ("/about".into(), "views/about.brain".into()),
        ("/_offline".into(), "views/_offline.brain".into()),
    ];
    if mixed {
        std::fs::write(root.join("views/counter.brain"),"<meta seo title=\"Counter\" description=\"An interactive counter.\" robots=\"noindex\"><script>let n = sig(0)</script><main><h1>Counter</h1><button on:click={() => n++}>Count {n}</button><a href=\"/about\">About</a></main>").unwrap();
        std::fs::write(root.join("views/items/[id].brain"),"<meta seo title=\"Item\" description=\"An SSR item route.\" robots=\"noindex\"><script>let current = route()</script><main><h1>Item</h1><p>{current.params.id}</p></main>").unwrap();
        views.extend([
            ("/counter".into(), "views/counter.brain".into()),
            ("/items/:id".into(), "views/items/[id].brain".into()),
        ]);
    }
    if mixed {
        std::fs::create_dir_all(root.join("views/private")).unwrap();
        std::fs::create_dir_all(root.join("views/nested")).unwrap();
        std::fs::write(root.join("views/private/index.brain"),"<meta seo title=\"Private\" description=\"Guarded static content.\" robots=\"noindex\"><main><h1>Private</h1></main>").unwrap();
        std::fs::write(
            root.join("views/private/_guard.jsbrain"),
            "export default () => true",
        )
        .unwrap();
        std::fs::write(root.join("views/nested/index.brain"),"<meta seo title=\"Nested\" description=\"A static view inside an interactive layout.\" robots=\"noindex\"><main><h1>Nested</h1></main>").unwrap();
        std::fs::write(root.join("views/nested/_layout.brain"),"<script>let n = sig(0)</script><section><button on:click={() => n++}>Layout count {n}</button><slot /></section>").unwrap();
        views.extend([
            ("/private".into(), "views/private/index.brain".into()),
            ("/nested".into(), "views/nested/index.brain".into()),
        ]);
    }
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let siblings = std::fs::read_dir(&runtime)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "js")
                && !["brain.js", "brain-dev.js", "brain-inspect.js"]
                    .iter()
                    .any(|name| path.ends_with(name))
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(path).unwrap(),
            )
        })
        .collect();
    let compile = |source: &str, path: &Path| {
        let options = brain_codegen::Options {
            dev: false,
            root: Some(root.clone()),
            ..Default::default()
        };
        let output = brain_codegen::compile_with(source, path, &options)
            .map_err(|diags| format!("{}: {diags:?}", path.display()))?;
        Ok(brain_pack::Unit {
            js: output.js,
            css: output.css,
            server_js: output.server_js,
            server_fns: output.server_fns,
        })
    };
    let input = brain_pack::Input {
        root: root.clone(),
        compile: &compile,
        runtime_core: brain_min::minify_runtime_core(
            &std::fs::read_to_string(runtime.join("brain.js")).unwrap(),
        )
        .unwrap(),
        runtime_siblings: siblings,
        layer_order: String::new(),
        minify: true,
        site: None,
        shim: std::fs::read_to_string(runtime.join("shim/dom.js")).ok(),
        generator: String::new(),
        server_module: None,
        cache: brain_pack::cache::CacheCfg {
            base: base.into(),
            sw_template: std::fs::read_to_string(runtime.join("sw/brain-sw.js")).ok(),
            ..Default::default()
        },
        server_mjs: std::fs::read_to_string(runtime.join("server/server.mjs")).ok(),
        progress: None,
        post: None,
    };
    let output = brain_pack::build(
        &input,
        &views,
        &|view| {
            if view.starts_with("views/nested/") {
                vec!["views/nested/_layout.brain".into()]
            } else {
                vec![]
            }
        },
        &|_| vec![],
    )
    .unwrap();
    for page in &output.ledger {
        if let Some(report) = output.report.iter().find(|row| row.route == page.route) {
            assert_eq!(
                report.js_min, page.conservative_js.raw,
                "legacy and detailed reports must share route scanning"
            );
        }
        for asset in &page.assets {
            let raw = output.files[&asset.path].len();
            assert_eq!(asset.bytes.raw, raw, "ledger must use final emitted bytes");
            assert_eq!(
                asset.bytes.gzip,
                output
                    .binary
                    .get(&format!("{}.gz", asset.path))
                    .map_or(raw, |bytes| bytes.len().min(raw))
            );
            assert_eq!(
                asset.bytes.brotli,
                output
                    .binary
                    .get(&format!("{}.br", asset.path))
                    .map_or(raw, |bytes| bytes.len().min(raw))
            );
            assert!(
                asset
                    .source
                    .as_ref()
                    .is_none_or(|path| !Path::new(path).is_absolute()),
                "ledger cannot expose local absolute paths"
            );
        }
    }
    (app, output)
}
fn assert_static(output: &brain_pack::Output) {
    for path in ["index.html", "about/index.html", "_offline/index.html"] {
        let html = &output.files[path];
        assert!(
            html.contains("brain-zero-js"),
            "missing static proof: {path}: {html}"
        );
        assert!(
            !brain_pack::cache::has_executable_scripts(html),
            "static route executes JS: {path}: {html}"
        );
        assert!(!html.contains("serviceWorker.register"));
        let row = output
            .ledger
            .iter()
            .find(|row| {
                row.html.raw == html.len()
                    && row.route
                        == match path {
                            "index.html" => "/",
                            "about/index.html" => "/about",
                            _ => "/_offline",
                        }
            })
            .unwrap();
        assert_eq!(row.entry_js.raw, 0);
        assert_eq!(row.inline_js_bytes, 0);
    }
}
#[test]
fn static_only_site_and_offline_page_execute_zero_javascript() {
    let (_app, output) = build(false, "/");
    assert_static(&output);
}
#[test]
fn static_routes_in_mixed_site_use_full_navigation_and_skip_sw_registration() {
    let (_app, output) = build(true, "/app/");
    assert_static(&output);
    let html = &output.files["counter/index.html"];
    assert!(brain_pack::cache::has_executable_scripts(html));
    assert!(html.contains("serviceWorker.register"));
    assert!(
        html.contains("fullLoad:[\"/app\",\"/app/_offline\",\"/app/about\"]"),
        "{html}"
    );
    assert!(!html.contains("brain-zero-js"));
    for route in ["/private", "/nested"] {
        let page = output
            .pages
            .iter()
            .find(|page| page.route == route)
            .unwrap();
        let html = &output.files[&page.file];
        assert!(
            !html.contains("brain-zero-js"),
            "guards and interactive layouts cannot be classified static: {html}"
        );
        assert!(brain_pack::cache::has_executable_scripts(html));
    }
    let ssr = output
        .files
        .values()
        .find(|html| html.contains("<!--brain:ssr-->"))
        .expect("dynamic route must keep SSR template");
    assert!(!ssr.contains("brain-zero-js"));
    assert!(brain_pack::cache::has_executable_scripts(ssr));
}
