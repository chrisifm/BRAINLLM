//! #151 (INTERNALS §37): one build, one page per locale. The default locale has no prefix, the others do (or the
//! view's translated slug); each page is prerendered in its language with `<html lang dir data-default-lang>`, and
//! a page that runs code gets a dictionary chunk with only the keys its modules read, loaded before it mounts.
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-i18n-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::create_dir_all(d.join("i18n")).unwrap();
    std::fs::write(d.join("brain.json"), "{\"name\":\"x\",\"i18n\":{\"default\":\"en\",\"locales\":[\"en\",\"es\",\"ar\"]}}").unwrap();
    std::fs::write(d.join("i18n/en.json"), "{\n  \"home.title\": \"Welcome\",\n  \"home.count\": { \"one\": \"{n} click\", \"other\": \"{n} clicks\" },\n  \"about.title\": \"About us\",\n  \"unused\": \"Never shipped\"\n}\n").unwrap();
    std::fs::write(d.join("i18n/es.json"), "{\n  \"home.title\": \"Welcome [es]\",\n  \"home.count\": { \"one\": \"{n} click [es]\", \"other\": \"{n} clicks [es]\" },\n  \"about.title\": \"About us [es]\",\n  \"unused\": \"Never [es]\"\n}\n").unwrap();
    std::fs::write(d.join("i18n/ar.json"), "{\n  \"home.title\": \"Welcome [ar]\",\n  \"home.count\": { \"one\": \"{n} click [ar]\", \"other\": \"{n} clicks [ar]\" },\n  \"about.title\": \"About us [ar]\",\n  \"unused\": \"x\"\n}\n").unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), "<meta seo title=\"Home\" description=\"Home\">\n<script>let n = sig(1)</script>\n<main><h1>{t('home.title')}</h1><button on:click={() => n++}>{t('home.count', { n })}</button></main>").unwrap();
    std::fs::write(d.join("views/about.brain"), "<meta seo title=\"About\" description=\"About\">\n<meta route es=\"/es/about-us\">\n<main><h1>{t('about.title')}</h1></main>").unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn one_page_per_locale_with_its_dictionary() {
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
    let views = vec![("/".to_string(), "views/home.brain".to_string()), ("/about".to_string(), "views/about.brain".to_string())];
    let out = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    let routes: Vec<&str> = out.pages.iter().map(|p| p.route.as_str()).collect();
    for r in ["/", "/about", "/es", "/es/about-us", "/ar", "/ar/about"] {
        assert!(routes.contains(&r), "{r} missing: {routes:?}");
    }
    let en = &out.files["index.html"];
    let es = &out.files["es/index.html"];
    assert!(en.contains("<h1>Welcome") && en.contains("lang=\"en\" dir=\"ltr\" data-default-lang=\"en\""), "{en}");
    assert!(es.contains("<h1>Welcome [es]") && es.contains("1 click [es]") && es.contains("lang=\"es\" dir=\"ltr\""), "prerendered in the es locale: {es}");
    assert!(out.files["es/about-us/index.html"].contains("<h1>About us [es]"), "translated slug");
    let ar = &out.files["ar/index.html"];
    assert!(ar.contains("dir=\"rtl\"") && ar.contains("Welcome [ar]"), "{ar}");
    // the interactive home loads its dictionary first; it has only the keys the view reads, in its locale
    let dict = es.split("<script type=\"module\">import\"").nth(1).expect("dictionary import").split('"').next().unwrap();
    assert!(dict.contains("/_b/i18n-home-es."), "{dict}");
    let d = &out.files[dict.trim_start_matches('/')];
    assert!(d.contains("\"home.title\":\"Welcome [es]\"") && d.contains("\"home.count\"") && !d.contains("unused") && !d.contains("about.title"), "{d}");
    let ard = out.files.iter().find(|(k, _)| k.starts_with("_b/i18n-home-ar.")).unwrap().1;
    assert!(ard.contains("Welcome [ar]") && ard.contains("add(\"ar\""), "{ard}");
    // the static about page ships no dictionary
    assert!(!out.files["es/about-us/index.html"].contains("i18n-"), "static page");
    assert!(!out.files.get("sw.js").is_some_and(|s| s.contains("/_b/i18n-")), "dictionaries are never precached");
    let _ = std::fs::remove_dir_all(&root);
}
