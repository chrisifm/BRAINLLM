//! #128 (§35.5): `*.worker.jsbrain` and plain `*.worker.js` modules become their own chunks, referenced by hashed
//! URL from `new URL(…, import.meta.url)`; a plain JS worker gets a generated entry that serves its exports through
//! brain-worker.js; worker chunks are never modulepreloaded by the page.
use std::path::{Path, PathBuf};

const VIEW: &str = r#"<meta seo title="Work" description="Worker page">
<script>
import stats from '../lib/stats.worker.jsbrain'
import { twice } from '../lib/plain.worker.js'
let out = sig('idle')
const run = async () => { out = (await stats.mean([1, 2, 3])) + ' ' + (await twice(21)) }
</script>
<main><h1>Work</h1><button on:click={run}>Run</button><p>{out}</p></main>"#;

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-workers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/work.brain"), VIEW).unwrap();
    std::fs::write(d.join("lib/stats.worker.jsbrain"), "const mean = (xs) => xs.reduce((a, b) => a + b, 0) / xs.length\nexport default { mean }\n").unwrap();
    std::fs::write(d.join("lib/plain.worker.js"), "export const twice = (n) => n * 2\n").unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn worker_modules_are_chunks_with_hashed_urls() {
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
        shim: None,
        generator: String::new(),
        server_module: None,
        cache: Default::default(),
        server_mjs: None,
        progress: None,
        post: None,
    };
    let out = brain_pack::build(&input, &[("/work".to_string(), "views/work.brain".to_string())], &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    let (view_file, view) = out.files.iter().find(|(_, v)| brain_pack::worker_refs(v).is_ok_and(|r| r.len() == 2)).expect("view chunk with its two worker references");
    let urls: Vec<String> = brain_pack::worker_refs(view).unwrap().into_iter().map(|(_, _, u)| u).collect();
    assert_eq!(urls.len(), 2, "{view}");
    for u in &urls {
        assert!(u.starts_with("/_b/") && out.files.contains_key(u.trim_start_matches('/')), "unresolved worker URL {u}");
        let w = &out.files[u.trim_start_matches('/')];
        assert!(w.contains("serve"), "a worker chunk serves its exports: {w}");
    }
    // the plain JS worker: entry = import * + serve({...}) of the hashed module that holds `twice`
    let entry = urls.iter().map(|u| &out.files[u.trim_start_matches('/')]).find(|w| w.contains("import*as")).expect("generated entry");
    let inner = entry.split("from\"").nth(1).unwrap().split('"').next().unwrap();
    assert!(out.files[inner.trim_start_matches('/')].contains("*2"), "{entry}");
    let html = &out.files["work/index.html"];
    for u in &urls {
        assert!(!html.contains(&format!("modulepreload\" href=\"{u}")), "worker chunk preloaded: {html}");
    }
    assert!(html.contains(&format!("/{view_file}")), "{html}");
    let _ = std::fs::remove_dir_all(&root);
}
