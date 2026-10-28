//! TOOLCHAIN defect: a view that uses the ui/ kit (`<Button>`, no import: `/@brain/ui/button.brain`) builds, the kit
//! component is compiled like an app component and ships as its own `/_b/` chunk with its CSS.
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-kit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), "<meta seo title=\"Home\" description=\"Home\">\n<script>let n = sig(0)</script>\n<main><h1>Home</h1><Button on:click={() => n++}>Add {n}</Button></main>").unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn kit_components_are_bundled() {
    let root = app();
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let core = brain_min::minify_runtime_core(&std::fs::read_to_string(rt.join("brain.js")).unwrap()).unwrap();
    let mut siblings: Vec<(String, String)> = std::fs::read_dir(&rt)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "js") && !p.ends_with("brain-dev.js") && !p.ends_with("brain-inspect.js"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    for e in std::fs::read_dir(rt.join("ui")).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "brain") {
            siblings.push((format!("ui/{}", p.file_name().unwrap().to_string_lossy()), std::fs::read_to_string(&p).unwrap()));
        }
    }
    let r2 = root.clone();
    let compile = move |src: &str, rel: &Path| -> Result<brain_pack::Unit, String> {
        let opts = brain_codegen::Options { dev: false, root: Some(r2.clone()), ..Default::default() };
        let o = brain_codegen::compile_with(src, rel, &opts).map_err(|d| format!("{}: {:?}", rel.display(), d.iter().map(|x| &x.msg).collect::<Vec<_>>()))?;
        Ok(brain_pack::Unit { js: o.js, css: o.css, server_js: o.server_js, server_fns: o.server_fns })
    };
    let no_kit: Vec<(String, String)> = siblings.iter().filter(|(n, _)| !n.starts_with("ui/")).cloned().collect();
    let input = brain_pack::Input {
        root: root.clone(),
        compile: &compile,
        runtime_core: core.clone(),
        runtime_siblings: siblings,
        layer_order: String::new(),
        minify: true,
        site: None,
        shim: None,
        generator: String::new(),
        server_module: None,
        cache: Default::default(),
        server_mjs: None,
        progress: None,
        post: None,
    };
    let out = brain_pack::build(&input, &[("/".to_string(), "views/home.brain".to_string())], &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    let html = &out.files["index.html"];
    assert!(!out.files.values().any(|v| v.contains("/@brain/")), "every runtime/kit import resolved to /_b/");
    let preloads = html.matches("modulepreload").count();
    assert!(preloads >= 3, "core + view + kit chunk: {html}");
    // without the kit in runtime_siblings the build names what is missing (the path above really ran)
    let bare = brain_pack::Input { runtime_core: core, runtime_siblings: no_kit, ..input };
    let err = brain_pack::build(&bare, &[("/".to_string(), "views/home.brain".to_string())], &|_| vec![], &|_| vec![]).err().unwrap_or_default();
    assert!(err.contains("kit component `ui/button.brain` is not embedded"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}
