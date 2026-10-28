//! #155 (§38, HTML first): a view whose forms post to a `server:` function (`export const $forms`) and has no `load`
//! is prerendered for GET and also listed in the server manifest (`"forms": true`) with its page template, so
//! dist/server.mjs answers a form-encoded POST without JavaScript: 303 on success, 422 with the errors, 403 without
//! a same origin, 415 for another body type.
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

const CONTACT: &str = r#"<meta seo title="Contact" description="Contact form">
<script>
server: async function save(ctx, data) { if (data.name === 'taken') throw Object.assign(Error('Invalid input'), { errors: { name: 'Already taken' } }); return { redirect: '/' } }
const go = async () => { await save({ name: 'x' }) }
</script>
<main><h1>Contact</h1><button on:click={go}>Send</button></main>"#;

fn app() -> PathBuf {
    let d = std::env::temp_dir().join(format!("brain-pack-forms-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("views")).unwrap();
    std::fs::write(d.join("app.brain"), "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>App</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("views/home.brain"), "<meta seo title=\"Home\" description=\"Static home\">\n<main><h1>Home</h1></main>").unwrap();
    std::fs::write(d.join("views/contact.brain"), CONTACT).unwrap();
    d.canonicalize().unwrap()
}

fn http(port: u16, req: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    // skip informational responses (103 Early Hints): the status is the one of the final response
    let status = out.rfind("HTTP/1.1 ").map_or(out.as_str(), |i| &out[i..]).split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, out)
}

#[test]
fn form_pages_are_static_for_get_and_served_for_post() {
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
        let mut server_js = o.server_js;
        // what COMPILER #154 emits for a `<form>` bound to `save` (contract with RUNTIME #156)
        if rel.ends_with("contact.brain") && !server_js.contains("$forms") {
            server_js.push_str("\nexport const $forms = { save: { name: 'str!' } }\n");
        }
        Ok(brain_pack::Unit { js: o.js, css: o.css, server_js, server_fns: o.server_fns })
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
    let views = vec![("/".to_string(), "views/home.brain".to_string()), ("/contact".to_string(), "views/contact.brain".to_string())];
    let out = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    assert!(out.files["contact/index.html"].contains("<h1>Contact</h1>"), "prerendered for GET");
    let mf = &out.files["_server/manifest.json"];
    assert!(mf.contains("\"path\":\"/contact\"") && mf.contains("\"forms\":true"), "{mf}");
    assert_eq!(out.files["_headers"].matches("\n/contact\n").count(), 1, "one header rule per path: {}", out.files["_headers"]);

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
    let hash = mf.split("\"rpc\":{\"").nth(1).unwrap().split('"').next().unwrap().to_string();
    let post = |origin: &str, ct: &str, body: &str| {
        http(port, &format!("POST /contact HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{origin}Content-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()))
    };
    let same = format!("Origin: http://127.0.0.1:{port}\r\n");
    let form = "application/x-www-form-urlencoded";
    let (get, get_body) = http(port, "GET /contact HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    let (ok, ok_body) = post(&same, form, &format!("_brain={hash}/save&name=ada"));
    let (empty, empty_body) = post(&same, form, &format!("_brain={hash}/save&name="));
    let (taken, taken_body) = post(&same, form, &format!("_brain={hash}/save&name=taken"));
    let (cross, _) = post("Origin: https://evil.example\r\n", form, &format!("_brain={hash}/save&name=ada"));
    let (json, _) = post(&same, "application/json", "{}");
    let (unknown, _) = post(&same, form, "_brain=nope/save&name=ada");
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(get, 200, "{get_body}");
    assert!(get_body.contains("<h1>Contact</h1>"), "{get_body}");
    assert_eq!(ok, 303, "{ok_body}");
    assert!(ok_body.to_ascii_lowercase().contains("location: /\r\n"), "{ok_body}");
    assert_eq!(empty, 422, "{empty_body}");
    assert!(empty_body.contains("id=\"brain-forms\"") && empty_body.contains("Required") && empty_body.contains("<h1>Contact</h1>"), "{empty_body}");
    assert_eq!(taken, 422, "{taken_body}");
    assert!(taken_body.contains("Already taken"), "{taken_body}");
    assert_eq!(cross, 403);
    assert_eq!(json, 415);
    assert_eq!(unknown, 400);
}
