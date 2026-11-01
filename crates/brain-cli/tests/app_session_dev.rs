//! `brain run` with BRAIN_BACKEND_SESSION=app: the signed session is the identity, whatever fields the app keeps in it,
//! and a leftover backend cookie never reaches the backend. A failed load keeps its cause in the log.
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SECRET: &str = "app-session-dev-secret-0123456789abcdef";

fn free_port() -> u16 { TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }

/// A session cookie the dev server accepts, holding exactly `user`.
fn signed(user: &str) -> String {
    let script = format!("const {{createHmac:h}}=require('node:crypto');const d=Buffer.from(JSON.stringify({{u:{user},exp:Date.now()+3600000}})).toString('base64url');process.stdout.write(d+'.'+h('sha256','{SECRET}').update(d).digest('base64url'))");
    let o = Command::new("node").args(["-e", &script]).stdin(Stdio::null()).output().unwrap();
    assert!(o.status.success());
    String::from_utf8(o.stdout).unwrap()
}

fn get(port: u16, path: &str, cookie: &str) -> String {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    write!(socket, "GET {path} HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    socket.read_to_string(&mut out).unwrap();
    out
}

/// Answers 401 to the session probe and an empty list to anything else, remembering each request's head.
fn backend() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut buf = [0; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let status = if head.starts_with("GET /_brain/auth/me") { "401 Unauthorized" } else { "200 OK" };
            log.lock().unwrap().push(head);
            let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]");
        }
    });
    (port, seen)
}

#[test]
fn an_app_session_user_of_any_shape_renders_and_the_log_keeps_a_load_failure_cause() {
    let dir = std::env::temp_dir().join(format!("brain-app-session-dev-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    let made = Command::new(bin).args(["new", "app"]).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let app = dir.join("app");
    let (backend_port, seen) = backend();
    fs::write(app.join(".brain_env"), format!("PUBLIC_SITE_URL=http://localhost:5180\n[development]\nBRAIN_SECRET={SECRET}\nBRAIN_BACKEND_URL=http://127.0.0.1:{backend_port}\nBRAIN_BACKEND_SESSION=app\n")).unwrap();
    let view = |load: &str| format!("<meta seo keyword=\"probe\" title=\"Probe\" description=\"A view that exists only for this test.\">\n<script>\n  export async function load(ctx) {{ {load} }}\n</script>\n<main><h1>Probe</h1></main>\n");
    fs::write(app.join("views/plain.brain"), view("return {}")).unwrap();
    fs::write(app.join("views/failing.brain"), view("throw Object.assign(Error('the upstream said no'), { status: 503 })")).unwrap();
    fs::write(app.join("views/reading.brain"), view("try { await ctx.backend.resource('items').list() } catch {} return {}")).unwrap();
    let port = free_port();
    let log = dir.join("run.log");
    let mut dev = Command::new(bin).args(["run", "--no-inspect", "--port", &port.to_string()]).current_dir(&app).stdin(Stdio::null())
        .stdout(Stdio::null()).stderr(fs::File::create(&log).unwrap()).spawn().unwrap();
    for _ in 0..200 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() { break }
        std::thread::sleep(Duration::from_millis(50));
    }
    let session = format!("__Host-session={}", signed(r#"{"id":"u1","role":"user","name":"Ana"}"#));
    let plain = get(port, "/plain", &session);
    let failing = get(port, "/failing", &session);
    let reading = get(port, "/reading", &format!("{session}; __Host-brain-access=leftover"));
    let _ = dev.kill();
    let _ = dev.wait();
    let logged = fs::read_to_string(&log).unwrap();
    let requests = seen.lock().unwrap().clone();
    let _ = fs::remove_dir_all(&dir);
    assert!(plain.starts_with("HTTP/1.1 200"), "{}\n{logged}", &plain[..plain.len().min(300)]);
    assert!(failing.starts_with("HTTP/1.1 503"), "{}", &failing[..failing.len().min(300)]);
    assert!(!failing.contains("the upstream said no"), "the cause is for the log, not the visitor");
    assert!(logged.contains("the upstream said no"), "{logged}");
    assert!(reading.starts_with("HTTP/1.1 200"), "{}", &reading[..reading.len().min(300)]);
    let resource: Vec<&String> = requests.iter().filter(|r| !r.contains("/_brain/auth/")).collect();
    assert!(!resource.is_empty(), "control: the view reached the backend: {requests:?}");
    assert!(resource.iter().all(|r| !r.to_lowercase().contains("__host-brain")), "{resource:?}");
}
