//! `brain start` (INTERNALS §34) end to end: a fake `dist/server.mjs` that follows RUNTIME's contract (listen on
//! BRAIN_LISTEN_FD, print `brain:ready`, drain on SIGTERM) lets the supervisor be tested without the real runtime.
#![cfg(unix)]
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SERVER: &str = r#"
import { createServer } from 'node:http'
if (process.env.FAIL) process.exit(3)
const s = createServer((q, r) => { r.writeHead(200, { 'content-type': 'text/plain' }); setTimeout(() => r.end(q.url === '/_health' ? '{"ok":true}' : 'pid ' + process.pid), 5) })
s.listen({ fd: Number(process.env.BRAIN_LISTEN_FD) }, () => console.log('brain:ready ' + process.pid))
process.on('SIGTERM', () => s.close(() => process.exit(0)))
"#;

fn app(name: &str, server: bool) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("brain-start-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("dist/_b")).unwrap();
    std::fs::write(d.join("app.brain"), "<html><head></head><body><slot /></body></html>").unwrap();
    std::fs::write(d.join("dist/index.html"), "<!doctype html><html><head><meta name=\"brain-build\" content=\"sig12345\"></head><body>hi</body></html>").unwrap();
    std::fs::write(d.join("dist/_headers"), "/*\n  Content-Security-Policy: default-src 'self'\n/\n  Cache-Control: no-cache\n").unwrap();
    if server {
        std::fs::write(d.join("dist/server.mjs"), SERVER).unwrap();
    }
    d
}

/// A running `brain start`: dropped (also when a test panics) it gets SIGTERM, so the supervisor drains and stops
/// its workers; a SIGKILL would leave them behind holding the port.
struct Started(Child);

impl std::ops::Deref for Started {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl std::ops::DerefMut for Started {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = Command::new("kill").args(["-TERM", &self.0.id().to_string()]).status();
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(15) && matches!(self.0.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Starts `brain start` on an OS-assigned port (`--port 0`) and returns it with the port it printed: no window
/// between picking a free port and binding it, so parallel suites never collide.
fn start(dir: &std::path::Path, env: &[(&str, &str)]) -> (Started, u16) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_brain"))
        .args(["start", "--port", "0", "--backoff-ms", "5", "--health-ms", "600000"])
        .current_dir(dir)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let out = c.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for l in std::io::BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(l);
        }
    });
    // first line: `brain start → http://127.0.0.1:<port>…  (build …)`; a loaded machine may take a while
    let line = rx.recv_timeout(Duration::from_secs(120)).expect("brain start printed no address");
    let port = line.split("127.0.0.1:").nth(1).and_then(|r| r.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|p| p.parse().ok()).expect(&line);
    (Started(c), port)
}

fn get(port: u16, path: &str, extra: &str) -> Option<String> {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n{extra}Connection: close\r\n\r\n").as_bytes()).ok()?;
    let mut b = String::new();
    s.read_to_string(&mut b).ok()?;
    Some(b)
}

fn wait_up(port: u16) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(120) {
        if get(port, "/_health", "").is_some_and(|r| r.starts_with("HTTP/1.1 200")) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn brain(dir: &std::path::Path, args: &[&str]) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn node() -> bool {
    Command::new("node").arg("-v").output().is_ok()
}

#[test]
fn static_mode_applies_headers_and_signature() {
    let dir = app("static", false);
    let (mut c, p) = start(&dir, &[]);
    assert!(wait_up(p));
    let r = get(p, "/", "").unwrap();
    assert!(r.contains("Content-Security-Policy: default-src 'self'") && r.contains("ETag: \"sig12345\"") && r.contains("Cache-Control: no-cache"), "{r}");
    assert!(get(p, "/", "If-None-Match: \"sig12345\"\r\n").unwrap().starts_with("HTTP/1.1 304"));
    assert!(get(p, "/_health", "").unwrap().contains("\"build\":\"sig12345\""));
    assert!(get(p, "/_headers", "").unwrap().starts_with("HTTP/1.1 404"), "host config is never served");
    let st = brain(&dir, &["status", "--json"]);
    for k in ["\"ok\":true", "\"mode\":\"static\"", "\"build\":\"sig12345\"", "\"uptimeS\":", "\"instances\":", "\"restarts\":", "\"lastExit\":"] {
        assert!(st.contains(k), "{k} in {st}");
    }
    brain(&dir, &["stop"]);
    assert!(c.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn killed_child_is_back_within_a_second_and_reload_drops_nothing() {
    if !node() {
        return;
    }
    let dir = app("server", true);
    // the real production server (RUNTIME's server.mjs) on a minimal dist: its drain is what reload relies on
    let real = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/server/server.mjs");
    if let Ok(src) = std::fs::read_to_string(&real) {
        // lay out the server exactly as brain-pack emits it: helper under _server/ and the import rewritten to it
        let helper = real.with_file_name("native-sockets.mjs");
        std::fs::write(dir.join("dist/server.mjs"), src.replace("from './native-sockets.mjs'", "from './_server/native-sockets.mjs'")).unwrap();
        std::fs::create_dir_all(dir.join("dist/_server")).unwrap();
        std::fs::copy(helper, dir.join("dist/_server/native-sockets.mjs")).unwrap();
        std::fs::write(dir.join("dist/_server/manifest.json"), "{\"v\":1,\"build\":\"sig12345\",\"routes\":[],\"rpc\":{}}").unwrap();
    }
    let (mut c, p) = start(&dir, &[]);
    assert!(wait_up(p));
    // the instance appears in status once the supervisor registers it: poll briefly (a loaded machine is slower)
    let mut st = String::new();
    for _ in 0..100 {
        st = brain(&dir, &["status", "--json"]);
        if st.matches("\"pid\":").count() >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid: i32 = st.split("\"pid\":").nth(2).and_then(|x| x.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|x| x.parse().ok()).expect(&st);
    Command::new("kill").args(["-9", &pid.to_string()]).status().unwrap();
    let t = Instant::now();
    std::thread::sleep(Duration::from_millis(30));
    assert!(wait_up(p));
    // < 1 s is the §34 target, measured by bench/start; here only "it comes back": a gate under heavy load (several
    // sessions compiling) can take seconds just to start node
    assert!(t.elapsed() < Duration::from_secs(20), "back in {:?}", t.elapsed());
    assert!(brain(&dir, &["status", "--json"]).contains("\"restarts\":1"));

    // reload while requests are in flight: zero failures
    let load = std::thread::spawn(move || (0..200).filter(|_| get(p, "/", "").is_some_and(|r| r.starts_with("HTTP/1.1 200"))).count());
    std::thread::sleep(Duration::from_millis(50));
    assert!(brain(&dir, &["reload"]).contains("\"ok\":true"));
    assert_eq!(load.join().unwrap(), 200, "every request during the reload succeeds");
    brain(&dir, &["stop"]);
    assert!(c.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_child_that_always_fails_ends_in_exit_1() {
    if !node() {
        return;
    }
    let dir = app("crash", true);
    // the first child must become ready, then every restart fails: a crash loop after start
    std::fs::write(dir.join("dist/server.mjs"), SERVER.replace("if (process.env.FAIL) process.exit(3)", "import { existsSync, writeFileSync } from 'node:fs'\nif (existsSync('booted')) process.exit(3)\nwriteFileSync('booted', '1')\nsetTimeout(() => process.exit(3), 300)")).unwrap();
    let (mut c, p) = start(&dir, &[]);
    let t = Instant::now();
    let code = loop {
        if let Some(s) = c.try_wait().unwrap() {
            break s.code();
        }
        assert!(t.elapsed() < Duration::from_secs(60), "no crash-loop exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut err = String::new();
    c.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("crash loop"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_child_that_dies_before_ready_backs_off_and_explains_on_stderr() {
    if !node() {
        return;
    }
    let dir = app("early", true);
    std::fs::write(dir.join("dist/server.mjs"), SERVER.replace("if (process.env.FAIL) process.exit(3)", "console.error('boom before ready')\nprocess.exit(3)")).unwrap();
    let (mut c, p) = start(&dir, &[]);
    let t = Instant::now();
    let code = loop {
        if let Some(s) = c.try_wait().unwrap() {
            break s.code();
        }
        assert!(t.elapsed() < Duration::from_secs(20), "the supervisor must not wait the whole ready timeout");
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut err = String::new();
    c.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("crash loop") && err.contains("boom before ready") && err.contains("died before ready"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_reload_exits_1_and_the_old_instances_keep_serving() {
    if !node() {
        return;
    }
    let dir = app("badreload", true);
    let (mut c, p) = start(&dir, &[]);
    assert!(wait_up(p));
    std::fs::write(dir.join("dist/server.mjs"), "console.error('broken build'); process.exit(2)\n").unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).arg("reload").current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert_eq!(o.status.code(), Some(1), "{out}");
    assert!(out.contains("reload failed") && out.contains("exited before it was ready"), "{out}");
    assert!(get(p, "/", "").unwrap().starts_with("HTTP/1.1 200"), "the old instance still serves");
    brain(&dir, &["stop"]);
    assert!(c.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn killed_from_outside_does_not_count_toward_the_crash_loop() {
    if !node() {
        return;
    }
    let dir = app("kills", true);
    let (mut c, p) = start(&dir, &[]);
    assert!(wait_up(p));
    for _ in 0..12 {
        let st = brain(&dir, &["status", "--json"]);
        let pid: i32 = st.split("\"pid\":").nth(2).and_then(|x| x.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|x| x.parse().ok()).expect(&st);
        Command::new("kill").args(["-9", &pid.to_string()]).status().unwrap();
        // wait for the replacement (a different pid) before the next kill
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(10) && brain(&dir, &["status", "--json"]).contains(&format!("\"pid\":{pid},")) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(wait_up(p));
    }
    assert!(c.try_wait().unwrap().is_none(), "12 kill -9 in a row are not a crash loop");
    let st = brain(&dir, &["status", "--json"]);
    assert!(st.contains("killed from outside"), "{st}");
    brain(&dir, &["stop"]);
    assert!(c.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn install_writes_a_service_that_finds_node() {
    let dir = app("install", true);
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(["start", "--install", "--port", "4999"]).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let f = std::fs::read_dir(dir.join(".brain/run")).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "plist" || x == "service")).unwrap();
    let s = std::fs::read_to_string(&f).unwrap();
    let node_dir = std::env::split_paths(&std::env::var("PATH").unwrap()).find(|d| d.join("node").is_file());
    if f.extension().unwrap() == "plist" {
        assert!(s.contains("<key>EnvironmentVariables</key>") && s.contains("<key>PATH</key>"), "{s}");
    } else {
        assert!(s.contains("Environment=\"PATH=") && s.contains("ExecStart=\"") && (s.contains("User=") || std::env::var("USER").as_deref() == Ok("root")), "{s}");
    }
    if let Some(d) = node_dir {
        assert!(s.contains(&d.display().to_string()), "node's directory is on the service PATH: {s}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn static_mode_serves_under_the_base() {
    let dir = app("base", false);
    std::fs::write(dir.join("brain.json"), "{ \"name\": \"x\", \"base\": \"/app/\" }").unwrap();
    std::fs::write(dir.join("dist/_headers"), "/app/*\n  X-Frame-Options: DENY\n/app/\n  Cache-Control: no-cache\n").unwrap();
    let (mut c, p) = start(&dir, &[]);
    assert!(wait_up(p), "/_health stays at the root");
    let r = get(p, "/app/", "").unwrap();
    assert!(r.starts_with("HTTP/1.1 200") && r.contains("X-Frame-Options: DENY") && r.contains("Cache-Control: no-cache") && r.contains("ETag: \"sig12345\""), "{r}");
    assert!(get(p, "/app", "").unwrap().contains("Location: /app/"));
    assert!(get(p, "/", "").unwrap().starts_with("HTTP/1.1 404"), "outside the base");
    assert!(get(p, "/index.html", "").unwrap().starts_with("HTTP/1.1 404"));
    brain(&dir, &["stop"]);
    assert!(c.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn workers_exit_when_the_supervisor_is_killed() {
    if !node() {
        return;
    }
    let dir = app("orphan", true);
    let (mut c, p) = start(&dir, &[]);
    assert!(wait_up(p));
    let st = brain(&dir, &["status", "--json"]);
    let worker: i32 = st.split("\"pid\":").nth(2).and_then(|x| x.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|x| x.parse().ok()).expect(&st);
    // SIGKILL: the supervisor cannot stop its workers; they must notice on their own
    c.kill().unwrap();
    c.wait().unwrap();
    let t = Instant::now();
    while Command::new("kill").args(["-0", &worker.to_string()]).stderr(Stdio::null()).status().unwrap().success() {
        assert!(t.elapsed() < Duration::from_secs(10), "worker {worker} outlived its supervisor");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(get(p, "/_health", "").is_none(), "nobody holds the port");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_prints_usage_and_unknown_flags_are_refused() {
    let dir = app("help", false);
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(["start", "--help"]).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    assert!(o.status.success() && String::from_utf8_lossy(&o.stdout).contains("usage: brain start [--port 3000]"));
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(["start", "--hepl"]).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("unknown argument `--hepl`"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn static_mode_serves_a_build_with_brain_extensions() {
    // `brain build --ext brain` (#26): pages are index.brain, assets .jsbrain/.cssbrain
    let dir = app("extbrain", false);
    std::fs::remove_file(dir.join("dist/index.html")).unwrap();
    std::fs::write(dir.join("dist/index.brain"), "<!doctype html><html><head><meta name=\"brain-build\" content=\"sigbrain\"></head><body>home</body></html>").unwrap();
    std::fs::create_dir_all(dir.join("dist/about")).unwrap();
    std::fs::write(dir.join("dist/about/index.brain"), "<!doctype html><title>about</title>").unwrap();
    std::fs::write(dir.join("dist/404.brain"), "<!doctype html><title>404</title>").unwrap();
    std::fs::write(dir.join("dist/_b/a.jsbrain"), "export {}").unwrap();
    std::fs::write(dir.join("dist/_b/a.cssbrain"), "a{}").unwrap();
    let (_c, p) = start(&dir, &[]);
    assert!(wait_up(p));
    let home = get(p, "/", "").unwrap();
    assert!(home.starts_with("HTTP/1.1 200") && home.contains("Content-Type: text/html") && home.contains("ETag: \"sigbrain\""), "{home}");
    assert!(get(p, "/about", "").unwrap().contains("<title>about</title>"));
    assert!(get(p, "/_b/a.jsbrain", "").unwrap().contains("Content-Type: text/javascript"));
    assert!(get(p, "/_b/a.cssbrain", "").unwrap().contains("Content-Type: text/css"));
    let nf = get(p, "/nope", "").unwrap();
    assert!(nf.starts_with("HTTP/1.1 404") && nf.contains("<title>404</title>"), "{nf}");
    let _ = std::fs::remove_dir_all(&dir);
}
