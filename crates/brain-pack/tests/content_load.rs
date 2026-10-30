//! #273: a static route whose `load` only reads `brain:content` (the index `brain add collection` writes) runs its
//! load at build time and is prerendered: a static file with the entries, no SSR route, and it does not turn on
//! hydration for every page. A `load` that reads the request stays on the server.
mod support;
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-content-load-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views/notes")).unwrap();
    std::fs::create_dir_all(d.join(".brain/build/server/_rt")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), "<meta seo title=\"Home\" description=\"Home\">\n<main><h1>Home</h1><a href=\"/notes\">Notes</a></main>").unwrap();
    std::fs::write(
        d.join("views/notes/index.brain"),
        "<meta seo title=\"Notes\" description=\"Notes\">\n<script>\nimport { page } from 'brain:content'\nexport async function load() { return { items: page('notes', 1) } }\n</script>\n<main><h1>Notes</h1><ul>{#each props.items as n (n)}<li>{n}</li>{/each}</ul></main>",
    )
    .unwrap();
    std::fs::write(d.join("views/me.brain"), "<meta seo title=\"Me\" description=\"Me\">\n<script>\nexport async function load(ctx) { return { who: ctx.user } }\n</script>\n<main><h1>Me</h1></main>").unwrap();
    std::fs::write(d.join(".brain/build/server/_rt/content.mjs"), "export const page = (col, n) => ['first note', 'second note']\n").unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn collection_index_is_prerendered() {
    let node = std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into());
    if std::process::Command::new(&node).arg("-v").output().is_err() {
        eprintln!("skipped: no Node");
        return;
    }
    let root = app();
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let Ok(server_mjs) = std::fs::read_to_string(rt.join("server/server.mjs")) else { return };
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
    // what the CLI does for build-time evaluation: the server code, `brain:content` pointing at the generated module
    let r3 = root.clone();
    let c2 = compile.clone();
    let server_module = move |rel: &str| -> Result<(String, String), String> {
        let src = std::fs::read_to_string(r3.join(rel)).map_err(|e| e.to_string())?;
        let u = c2(&src, Path::new(rel))?;
        let content = format!("file://{}", r3.join(".brain/build/server/_rt/content.mjs").display());
        let code = brain_pack::rewrite_imports(&u.server_js, &mut |s| if s == "brain:content" { Ok(Some(content.clone())) } else { support::runtime_specifier(s) })?;
        let out = r3.join(".brain/build/server").join(format!("{}.mjs", rel.replace('/', "_")));
        std::fs::write(&out, &code).map_err(|e| e.to_string())?;
        Ok((format!("file://{}", out.display()), code))
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
        server_module: Some(&server_module),
        cache: Default::default(),
        server_mjs: Some(server_mjs),
        progress: None,
        post: None,
    };
    let views = vec![("/".to_string(), "views/home.brain".to_string()), ("/notes".to_string(), "views/notes/index.brain".to_string()), ("/me".to_string(), "views/me.brain".to_string())];
    let out = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    let notes = out.files.get("notes/index.html").unwrap_or_else(|| panic!("no static page: {:?} {:?}", out.files.keys().collect::<Vec<_>>(), out.warnings));
    assert!(notes.contains("<li>first note") && notes.contains("<li>second note"), "{notes}");
    let mf = &out.files["_server/manifest.json"];
    assert!(!mf.contains("\"path\":\"/notes\"") && mf.contains("\"path\":\"/me\""), "only the request-reading load is SSR: {mf}");
    // /notes renders at build: no props in a server template, no SSR page for it
    assert!(!out.files.keys().any(|k| k.starts_with("_server/pages/") && out.files[k].contains("<h1>Notes")), "no server template for /notes");
    let _ = std::fs::remove_dir_all(&root);
}
