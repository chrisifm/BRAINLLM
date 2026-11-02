//! RPC (`POST /_r/<hash>/<fn>`) and form posts (`_brain=<hash>/<fn>`) reach only the functions a view declares with
//! `server:`, in production (`dist/server.mjs`) and in dev (`brain run`). A view's `load()` (and its guards' data),
//! `seo` and `head` answer as an unknown function does.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const VIEW: &str = "<meta seo keyword=\"private page\" title=\"Private page\" description=\"A page whose load data must stay behind the page.\">\n<script>\n  export async function load(ctx) { return { secret: 'LOAD-ONLY-DATA' } }\n  server: async function save(ctx, v) { return 'saved' }\n  const f = form({ email: 'str!' }, { submit: save })\n  let done = sig('')\n  const send = async () => { done = await save('x') }\n</script>\n<main><h1>Private page</h1><p>{props.secret}</p><p>{done}</p><button type=\"button\" on:click={send}>Send</button>\n<form on:submit={f.submit}><label>Email <input bind:value={f.v.email}></label><button>Go</button></form></main>\n";

fn brain(args: &[&str], cwd: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(cwd).stdin(Stdio::null()).output().unwrap()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn http(port: u16, raw: &str) -> String {
    let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    c.write_all(raw.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = c.read_to_string(&mut out);
    out
}

fn rpc(port: u16, id: &str) -> String {
    http(port, &format!("POST /_r/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nX-Brain-RPC: 1\r\nContent-Type: application/json\r\nContent-Length: 5\r\nConnection: close\r\n\r\n[\"x\"]"))
}

fn form(port: u16, id: &str) -> String {
    let body = format!("_brain={}&email=a", id.replace('/', "%2F").replace('$', "%24"));
    http(port, &format!("POST /private HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()))
}

fn wait(port: u16, child: &mut Child) {
    for _ in 0..300 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        assert!(child.try_wait().unwrap().is_none(), "the server exited early");
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the server never listened on {port}");
}

/// Every non-`server:` export is refused the way an unknown name is; `save` still answers.
fn assert_only_server_functions(port: u16, hash: &str, mode: &str) {
    let unknown = rpc(port, &format!("{hash}/nope"));
    let unknown_status = unknown.lines().next().unwrap_or("").to_string();
    assert!(unknown_status.contains(" 404"), "{mode}: control, unknown name: {unknown}");
    for f in ["load", "seo", "head", "$forms", "$server"] {
        let r = rpc(port, &format!("{hash}/{f}"));
        assert_eq!(r.lines().next().unwrap_or(""), unknown_status, "{mode}: /_r/{hash}/{f} must answer as an unknown function: {r}");
        assert!(r.contains("Unknown function") && !r.contains("LOAD-ONLY-DATA"), "{mode}: {f}: {r}");
        let p = form(port, &format!("{hash}/{f}"));
        assert!(p.starts_with("HTTP/1.1 400") && !p.contains("LOAD-ONLY-DATA"), "{mode}: form {f}: {p}");
    }
    let ok = rpc(port, &format!("{hash}/save"));
    assert!(ok.starts_with("HTTP/1.1 200") && ok.contains("saved"), "{mode}: control, the server: function answers: {ok}");
    let posted = form(port, &format!("{hash}/save"));
    assert!(posted.starts_with("HTTP/1.1 303"), "{mode}: control, the form posts: {posted}");
}

#[test]
fn only_server_functions_are_callable_in_production_and_in_dev() {
    if Command::new("node").arg("--version").stdin(Stdio::null()).output().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("brain-rpc-server-only-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(brain(&["new", "demo"], &dir).status.success());
    let app = dir.join("demo");
    std::fs::write(app.join("views/private.brain"), VIEW).unwrap();
    let out = brain(&["build"], &app);
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let mf = std::fs::read_to_string(app.join("dist/_server/manifest.json")).unwrap();
    // the view's entry in `"rpc":{"<hash>":"_server/<module>.mjs",…}`: the module that holds LOAD-ONLY-DATA
    let rpc = mf.split("\"rpc\":{").nth(1).expect("an rpc map").split('}').next().unwrap();
    let hash = rpc
        .split(',')
        .find_map(|pair| {
            let (h, m) = pair.split_once(':')?;
            let module = std::fs::read_to_string(app.join("dist").join(m.trim_matches('"'))).ok()?;
            module.contains("LOAD-ONLY-DATA").then(|| h.trim_matches('"').to_string())
        })
        .expect("the view's rpc entry");

    // dev serves the view's own module; its client stub names the dev id `"<hash>/save"` (longer than the build's)
    let port = free_port();
    let mut dev = Command::new(env!("CARGO_BIN_EXE_brain")).args(["run", "--no-inspect", "--port", &port.to_string()]).current_dir(&app).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    wait(port, &mut dev);
    let result = std::panic::catch_unwind(|| {
        let module = http(port, &format!("GET /views/private.brain HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"));
        let end = module.find("/save\"").expect("the dev client stub");
        let dev_hash = &module[module[..end].rfind('"').unwrap() + 1..end];
        assert_only_server_functions(port, dev_hash, "dev")
    });
    let _ = dev.kill();
    let _ = dev.wait();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }

    let port = free_port();
    let mut prod = Command::new("node").arg("dist/server.mjs").envs([("PORT", port.to_string()), ("HOST", "127.0.0.1".to_string())]).current_dir(&app).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    wait(port, &mut prod);
    let result = std::panic::catch_unwind(|| assert_only_server_functions(port, &hash, "production"));
    let _ = prod.kill();
    let _ = prod.wait();
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
