//! #114 (INTERNALS §32): a static build evaluates `paths()` (one page per param), build-time `load` on those routes,
//! `seo(props)` and the view's `<head>` block, and a view without `load` is not mistaken for a server view.
use std::path::{Path, PathBuf};

const HOME: &str = r#"<meta seo title="Home" description="Static home" canonical="/">
<main><h1>Home</h1></main>"#;

const POST: &str = r#"<meta seo title={`Post ${props.id}`} description="One post" canonical={`/post/${props.id}`} keyword="post" jsonld={{ '@type': 'Article', headline: props.id }}>
<script>
import { route } from '/@brain/brain-env.js'
const r = route()
export async function paths() { return [{ id: 'a' }, { id: 'b c' }] }
export async function load(ctx) { return { id: ctx.params.id } }
</script>
<head>
  <link rel="alternate" hreflang="es" href={`/es/post/${props.id}`} />
  <meta name="description" content="from the head block" />
</head>
<main><h1>Post {props.id}</h1><p id="rp">{r.params.id}</p></main>"#;

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-seo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views/post")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"description\" content=\"app\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), HOME).unwrap();
    std::fs::write(d.join("views/post/[id].brain"), POST).unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn paths_seo_and_head_reach_every_page() {
    if std::process::Command::new(std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into())).arg("-v").output().is_err() {
        eprintln!("skipped: no Node");
        return;
    }
    let root = app();
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let core = brain_min::minify_runtime_core(&std::fs::read_to_string(rt.join("brain.js")).unwrap()).unwrap();
    let r2 = root.clone();
    let compile = move |src: &str, rel: &Path| -> Result<brain_pack::Unit, String> {
        let opts = brain_codegen::Options { dev: false, root: Some(r2.clone()), ..Default::default() };
        let o = brain_codegen::compile_with(src, rel, &opts).map_err(|d| format!("{}: {:?}", rel.display(), d.iter().map(|x| &x.msg).collect::<Vec<_>>()))?;
        Ok(brain_pack::Unit { js: o.js, css: o.css, server_js: o.server_js, server_fns: o.server_fns })
    };
    // the views here import nothing, so the server module is the server_js as is
    let r3 = root.clone();
    let server = move |rel: &str| -> Result<(String, String), String> {
        let src = std::fs::read_to_string(r3.join(rel)).map_err(|e| e.to_string())?;
        let opts = brain_codegen::Options { dev: false, root: Some(r3.clone()), ..Default::default() };
        let o = brain_codegen::compile_with(&src, Path::new(rel), &opts).map_err(|_| "compile".to_string())?;
        let f = r3.join(".brain/test-server").join(format!("{}.mjs", rel.replace('/', "_")));
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, &o.server_js).unwrap();
        Ok((format!("file://{}", f.to_string_lossy().replace('[', "%5B").replace(']', "%5D")), o.server_js))
    };
    let input = brain_pack::Input {
        root: root.clone(),
        compile: &compile,
        runtime_core: core,
        runtime_siblings: std::fs::read_dir(&rt)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "js") && !p.ends_with("brain.js") && !p.ends_with("brain-dev.js") && !p.ends_with("brain-inspect.js"))
            .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
            .collect(),
        layer_order: String::new(),
        minify: true,
        site: Some("https://example.dev".into()),
        shim: std::fs::read_to_string(rt.join("shim/dom.js")).ok(),
        generator: String::new(),
        server_module: Some(&server),
        cache: Default::default(),
        server_mjs: None,
        progress: None,
        post: None,
    };
    let views = vec![("/".to_string(), "views/home.brain".to_string()), ("/post/:id".to_string(), "views/post/[id].brain".to_string())];
    let seo_of = |v: &str| -> Vec<(String, String)> {
        match v {
            "views/home.brain" => vec![("seo".into(), String::new()), ("title".into(), "Home".into()), ("description".into(), "Static home".into()), ("canonical".into(), "/".into()), ("twitter".into(), "@home".into())],
            "app.brain" => vec![("twitter".into(), "@app".into())],
            _ => vec![],
        }
    };
    let out = brain_pack::build(&input, &views, &|_| vec![], &seo_of).unwrap_or_else(|e| panic!("{e}"));
    let routes: Vec<&str> = out.pages.iter().map(|p| p.route.as_str()).collect();
    assert_eq!(routes, ["/", "/post/a", "/post/b%20c"], "{:?}", out.warnings);

    let home = &out.files["index.html"];
    assert!(home.contains("<title>Home</title>") && !home.contains("<title>App</title>"), "{home}");
    assert_eq!(home.matches("name=\"description\"").count(), 1, "{home}");

    let a = &out.files["post/a/index.html"];
    assert!(a.contains("<title>Post a</title>"), "{a}\nwarnings: {:?}", out.warnings);
    assert!(a.contains("<h1>Post a"), "build-time load reached the view: {a}");
    assert!(a.contains("<link rel=\"canonical\" href=\"https://example.dev/post/a\">"), "{a}");
    assert!(a.contains("\"@context\":\"https://schema.org\",\"@type\":\"Article\",\"headline\":\"a\""), "{a}");
    assert!(a.contains("<link rel=\"alternate\" hreflang=\"es\" href=\"/es/post/a\">"), "{a}");
    assert!(a.contains("content=\"from the head block\"") && a.matches("name=\"description\"").count() == 1, "{a}");
    assert!(!a.contains("keyword"), "keyword is audit-only: {a}");

    // route() during the prerender and in the browser: the same params (RUNTIME #169)
    assert!(a.contains("<meta name=\"brain-params\" content='{\"id\":\"a\"}'>") && a.contains("<p id=\"rp\">a<"), "{a}");
    assert!(!home.contains("brain-params"), "pages without params carry no meta");
    // `<meta seo twitter>`: app.brain's default on every page, the view's value wins (QA conformance 165)
    assert!(a.contains("<meta name=\"twitter:site\" content=\"@app\">"), "{a}");
    assert!(home.contains("<meta name=\"twitter:site\" content=\"@home\">") && !home.contains("@app"), "{home}");
    let bc = &out.files["post/b%20c/index.html"];
    assert!(bc.contains("<p id=\"rp\">b c<"), "{bc}");
    assert!(bc.contains("<title>Post b c</title>") && bc.contains("href=\"https://example.dev/post/b%20c\""), "{bc}");
    let _ = std::fs::remove_dir_all(&root);
}
