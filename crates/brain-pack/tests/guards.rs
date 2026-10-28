//! #208 (§22): views under a folder with `_guard.jsbrain` / `_middleware.jsbrain` are never static files; they go to
//! the server manifest with their guards (outermost first), the nearest `_error.brain` and the top-level `forbidden`
//! (views/403.brain). Through RUNTIME's real server.mjs: no session → 303 to the login, deny → 403 page, a failing
//! `load` → the folder's error page.
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-guards-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let files: &[(&str, &str)] = &[
        ("app.brain", "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>"),
        ("views/home.brain", "<meta seo title=\"Home\" description=\"Home\">\n<main><h1>Home</h1></main>"),
        ("views/login.brain", "<meta seo title=\"Login\" description=\"Login\">\n<main><h1>Login</h1></main>"),
        ("views/403.brain", "<meta seo title=\"Forbidden\" description=\"Forbidden\">\n<main><h1>No access: {props.reason}</h1></main>"),
        ("views/account/_guard.jsbrain", "export default (ctx) => (ctx.user ? true : { redirect: '/login?next=' + ctx.url.pathname })\n"),
        ("views/account/index.brain", "<meta seo title=\"Account\" description=\"Account\">\n<main><h1>Account</h1></main>"),
        ("views/admin/_guard.jsbrain", "export default () => ({ deny: 'admins only' })\n"),
        ("views/admin/index.brain", "<meta seo title=\"Admin\" description=\"Admin\">\n<main><h1>Admin</h1></main>"),
        ("views/shop/_error.brain", "<meta seo title=\"Error\" description=\"Error\">\n<main><h1>Shop failed: {props.message}</h1></main>"),
        ("views/shop/live.brain", "<meta seo title=\"Live\" description=\"Live\">\n<script>\nexport async function load() { throw Error('db down') }\n</script>\n<main><h1>Live</h1></main>"),
        ("views/shop/index.brain", "<meta seo title={`Shop`} description=\"Shop\">\n<script>\nexport async function load() { throw Error('db down') }\n</script>\n<main><h1>Shop</h1></main>"),
    ];
    for (f, c) in files {
        let p = d.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, c).unwrap();
    }
    d.canonicalize().unwrap()
}

fn http(port: u16, path: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    // skip informational responses (103 Early Hints): the status is the one of the final response
    let status = out.rfind("HTTP/1.1 ").map_or(out.as_str(), |i| &out[i..]).split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, out)
}

#[test]
fn guarded_routes_run_on_the_server() {
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
        post: None,
    };
    let views: Vec<(String, String)> = [("/", "views/home.brain"), ("/login", "views/login.brain"), ("/403", "views/403.brain"), ("/account", "views/account/index.brain"), ("/admin", "views/admin/index.brain"), ("/shop", "views/shop/index.brain"), ("/shop/live", "views/shop/live.brain")]
        .iter()
        .map(|(r, v)| (r.to_string(), v.to_string()))
        .collect();
    let out = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    assert!(!out.files.contains_key("account/index.html") && !out.files.contains_key("admin/index.html"), "a protected view is never a static file");
    let mf = &out.files["_server/manifest.json"];
    assert!(mf.contains("\"guards\":[\"_server/") && mf.contains("\"forbidden\":{") && mf.contains("\"error\":{"), "{mf}");
    assert!(!mf.contains("\"path\":\"!"), "templates are not routes: {mf}");
    // #283: the client router gets each folder guard as a chunk URL
    let home = &out.files["index.html"];
    let g = home.split("guards:{").nth(1).unwrap_or_else(|| panic!("no guards in router(): {home}")).split('}').next().unwrap();
    assert!(g.contains("\"/account\":\"/_b/") && g.contains("\"/admin\":\"/_b/"), "{g}");
    let acc = g.split("\"/account\":\"").nth(1).unwrap().split('"').next().unwrap();
    assert!(out.files[acc.trim_start_matches('/')].contains("export default") || out.files[acc.trim_start_matches('/')].contains("export{"), "the chunk exports the guard");
    assert!(!out.report.iter().any(|r| r.route.starts_with('!')));

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
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready).unwrap();
    assert!(ready.starts_with("brain:ready"), "{ready}");
    let (acc, acc_b) = http(port, "/account");
    let (adm, adm_b) = http(port, "/admin");
    let (shop, shop_b) = http(port, "/shop");
    let (home, _) = http(port, "/");
    let (live, live_b) = http(port, "/shop/live");
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(acc, 303, "{acc_b}");
    assert!(acc_b.to_ascii_lowercase().contains("location: /login?next=/account"), "{acc_b}");
    assert_eq!(adm, 403, "{adm_b}");
    assert!(adm_b.contains("No access: admins only"), "{adm_b}");
    assert_eq!(shop, 500, "{shop_b}");
    assert!(shop_b.contains("Shop failed: Something went wrong."), "{shop_b}");
    assert_eq!(home, 200);
    // a streamed route (static head): headers already left with 200, the folder's _error renders inside the region
    assert_eq!(live, 200, "{live_b}");
    assert!(live_b.contains("Shop failed:"), "{live_b}");
}
