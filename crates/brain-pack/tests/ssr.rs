//! #51: views with `load`, dynamic routes without `paths()` and `server:` functions ship as dist/server.mjs +
//! dist/_server/ (manifest v1, page templates with markers, server modules, RPC map). The test builds an app, starts
//! RUNTIME's real server.mjs on that dist and checks SSR and RPC over HTTP.
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

const USER: &str = r#"<meta seo title={`User ${props.user.name}`} description="A user">
<script>
import fmt from '../../lib/fmt.jsbrain'
export async function load(ctx) { return { user: { id: ctx.params.id, name: fmt(ctx.params.id) } } }
server: async function rename(ctx, name) { if (!name) throw Object.assign(Error('Invalid input'), { errors: { name: 'required' } }); return fmt(name) }
let user = sig(props.user)
const go = async () => { user = { ...user, name: await rename('x') } }
</script>
<main><h1>{user.name}</h1><button on:click={go}>Rename</button></main>"#;

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-ssr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views/users")).unwrap();
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), "<meta seo title=\"Home\" description=\"Static home\">\n<main><h1>Home</h1></main>").unwrap();
    std::fs::write(d.join("views/users/[id].brain"), USER).unwrap();
    std::fs::write(d.join("views/users/_loading.brain"), "<main><p>Loading user…</p></main>").unwrap();
    std::fs::write(d.join("views/slow.brain"), "<meta seo title=\"Slow\" description=\"A slow page\">\n<meta cache=\"30\">\n<script>\nexport async function load() { return { n: 1 } }\n</script>\n<main><h1>Slow {props.n}</h1></main>").unwrap();
    std::fs::write(d.join("lib/fmt.jsbrain"), "export default (s) => 'User ' + String(s).toUpperCase()\n").unwrap();
    d.canonicalize().unwrap()
}

fn http(port: u16, req: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    // skip informational responses (103 Early Hints): the status is the one of the final response
    let last = out.rfind("HTTP/1.1 ").map_or(out.as_str(), |i| &out[i..]);
    let status = last.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, out)
}

#[test]
fn ssr_and_rpc_through_the_real_server() {
    let node = std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into());
    if std::process::Command::new(&node).arg("-v").output().is_err() {
        eprintln!("skipped: no Node");
        return;
    }
    let root = app();
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let Ok(server_mjs) = std::fs::read_to_string(rt.join("server/server.mjs")) else {
        eprintln!("skipped: runtime/server/server.mjs missing");
        return;
    };
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
    // the CLI's post step passes brain-seo's redirect table for server.mjs (#219)
    let post = |_: &mut [brain_pack::Page], _: &[(String, u64)]| -> Result<brain_pack::PostOut, String> {
        Ok(brain_pack::PostOut { files: vec![("_server/redirects.json".into(), "{\"exact\":{\"/old\":[\"/\",301]},\"patterns\":[]}".into())], ..Default::default() })
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
        server_mjs: Some(server_mjs),
        progress: None,
        post: Some(&post),
    };
    let views = vec![("/".to_string(), "views/home.brain".to_string()), ("/users/:id".to_string(), "views/users/[id].brain".to_string()), ("/slow".to_string(), "views/slow.brain".to_string())];
    let out = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));

    // shape of dist/
    // form posts: server.mjs loads _server/brain-form.js, which imports ./brain.js and ./brain-schema.js (raw names)
    for m in ["_server/brain-form.js", "_server/brain-schema.js", "_server/brain.js"] {
        assert!(out.files.contains_key(m), "{m} missing: {:?}", out.files.keys().filter(|k| k.starts_with("_server/")).collect::<Vec<_>>());
    }
    assert!(out.files["_server/brain.js"].contains("export const sig"), "the server copy of the core keeps its export names");
    let mf = &out.files["_server/manifest.json"];
    assert!(mf.contains("\"v\":1") && mf.contains(&format!("\"build\":\"{}\"", out.signature)) && mf.contains("\"csp\":\"default-src 'self'") && mf.contains("\"core\":\"/_b/") && mf.contains("\"path\":\"/users/:id\",\"rx\":\"^/users/(?<id>") && mf.contains("\"page\":\"_server/pages/"), "{mf}");
    assert!(mf.contains("\"redirects\":{\"exact\":{\"/old\":[\"/\",301]}") && !out.files.contains_key("_server/redirects.json"), "{mf}");
    assert!(mf.contains("\"rpc\":{\"") && out.files.contains_key("server.mjs") && out.files.contains_key("_server/dom.js"), "{mf}");
    // #235/#232: the nearest _loading chunk, `<meta cache>`, streaming only with a static head, preloads for 103
    let route_of = |path: &str| mf.split(&format!("{{\"path\":\"{path}\"")).nth(1).unwrap().split("},{\"path\"").next().unwrap().to_string();
    let user = route_of("/users/:id");
    let slow = route_of("/slow");
    assert!(user.contains("\"loading\":\"/_b/") && !user.contains("\"stream\"") && !user.contains("\"cache\""), "{user}");
    assert!(slow.contains("\"cache\":30") && slow.contains("\"stream\":true") && !slow.contains("\"loading\""), "{slow}");
    assert!(user.contains("\"links\":[\"</_b/") && user.contains("integrity=\\\"sha384-") && !mf.contains('\u{1}'), "{user}");
    let home = &out.files["index.html"];
    assert!(home.contains(",core:\"/_b/") && home.contains("loading:[{rx:\"^/users/"), "the router knows which loading view to show: {home}");
    let page = out.files.iter().find(|(k, v)| k.starts_with("_server/pages/") && v.contains("User")).map(|(_, v)| v).unwrap_or_else(|| out.files.iter().find(|(k, _)| k.starts_with("_server/pages/")).unwrap().1);
    assert!(page.contains("<!--brain:ssr-->") && page.contains("<script type=\"application/json\" id=\"brain-props\"><!--brain:props--></script>"), "{page}");
    assert!(page.contains("P=JSON.parse(document.getElementById('brain-props').textContent)"), "props read before the region is cleared: {page}");
    assert!(out.files.contains_key("index.html") && !out.files.keys().any(|k| k.starts_with("users/")), "SSR routes have no static page");
    assert!(!out.binary.keys().any(|k| k.starts_with("_server/") || k == "server.mjs.gz"), "server code is never precompressed");
    let srv_mods: Vec<&String> = out.files.keys().filter(|k| k.starts_with("_server/") && k.ends_with(".mjs")).collect();
    assert!(srv_mods.len() >= 2, "view + lib/fmt server modules: {srv_mods:?}");

    // run it
    let dist = root.join("dist");
    for (p, c) in &out.files {
        let f = dist.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut child = std::process::Command::new(&node)
        .arg(dist.join("server.mjs"))
        .env("PORT", port.to_string())
        .env("HOST", "127.0.0.1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready).unwrap();
    assert!(ready.starts_with("brain:ready"), "{ready}");

    let (st, body) = http(port, "GET /users/ada HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    let (st_home, home) = http(port, "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    let rpc_hash = mf.split("\"rpc\":{\"").nth(1).unwrap().split('"').next().unwrap().to_string();
    let call = |args: &str| http(port, &format!("POST /_r/{rpc_hash}/rename HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nX-Brain-RPC: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{args}", args.len()));
    let (st_ok, ok) = call("[\"bo\"]");
    let (st_bad, bad) = call("[\"\"]");
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(st, 200, "{body}");
    assert!(body.contains("<h1>User ADA") && body.contains("\"name\":\"User ADA\""), "rendered body + props: {body}");
    assert!(body.contains("<title>User User ADA</title>"), "title from View.seo(props): {body}");
    assert_eq!(st_home, 200, "{home}");
    assert!(home.contains("<h1>Home"), "{home}");
    assert_eq!(st_ok, 200, "{ok}");
    assert!(ok.contains("\"User BO\""), "chunked body: {ok}");
    assert_eq!(st_bad, 422, "{bad}");
    assert!(bad.contains("\"errors\":{\"name\":\"required\"}"), "{bad}");
}
