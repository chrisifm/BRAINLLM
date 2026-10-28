//! `brain run` (#269, #266, #267): the debugger on :P+1 with its data proxied from the app's dev server, the
//! project's problems as JSON, `--mock`, and the per-file diagnostics with a snippet over the HMR socket.
#![cfg(unix)]
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn get(port: u16, p: &str) -> String {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(format!("GET {p} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut b = String::new();
    let _ = s.read_to_string(&mut b);
    b
}

/// `brain run` and the two ports it prints (the debugger's when there is one).
fn run(dir: &std::path::Path, extra: &[&str]) -> (Child, u16, Option<u16>, std::sync::Arc<std::sync::Mutex<String>>) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_brain")).args(["run", "--port", "0"]).args(extra).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    let mut err = BufReader::new(c.stderr.take().unwrap());
    let port_of = |l: &str| l.split("127.0.0.1:").nth(1).and_then(|r| r.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|p| p.parse::<u16>().ok());
    let mut l = String::new();
    while !l.contains("brain run →") {
        l.clear();
        assert!(err.read_line(&mut l).unwrap() > 0, "brain run printed no address");
    }
    let app = port_of(&l).unwrap();
    let dbg = if extra.contains(&"--no-inspect") {
        None
    } else {
        l.clear();
        err.read_line(&mut l).unwrap();
        assert!(l.contains("debugger →"), "{l}");
        port_of(&l)
    };
    let log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let l2 = log.clone();
    std::thread::spawn(move || {
        for l in err.lines().map_while(Result::ok) {
            l2.lock().unwrap().push_str(&(l + "\n"));
        }
    });
    (c, app, dbg, log)
}

#[test]
fn debugger_problems_mock_and_snippets() {
    let tmp = std::env::temp_dir().join(format!("bdvt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(Command::new(env!("CARGO_BIN_EXE_brain")).args(["new", "app"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("app");
    std::fs::write(dir.join("comp/warn.brain"), "<script>\n  let el = ref()\n  mounted(() => { log(el.v) })\n</script>\n\n<p bind:ref={el}>Warn</p>\n").unwrap();
    let (mut c, app, dbg, log) = run(&dir, &[]);
    let dbg = dbg.expect("the debugger runs by default");
    assert_eq!(dbg, app + 1);
    // the problems of the project, with family and component, on the app and through the debugger's own origin
    let p = get(app, "/@brain/problems");
    assert!(p.contains("\"code\":\"B0130\"") && p.contains("\"family\":\"B01xx\"") && p.contains("\"component\":\"Warn\"") && p.contains("\"fix\":"), "{p}");
    assert!(get(dbg, "/@brain/problems").contains("\"code\":\"B0130\""), "proxied");
    assert!(get(dbg, "/@brain/metrics").contains("\"batches\":["), "proxied");
    let home = get(dbg, "/");
    assert!(home.starts_with("HTTP/1.1 200") && home.contains("BRAINLLM devtools"), "{home}");
    // a save sends every diagnostic of the file with a snippet around its line
    let mut ws = std::net::TcpStream::connect(("127.0.0.1", app)).unwrap();
    ws.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    ws.write_all(b"GET /@brain/hmr HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").unwrap(); // gitleaks:allow -- RFC WebSocket example nonce, not a credential
    std::thread::sleep(Duration::from_millis(300));
    let f = dir.join("comp/warn.brain");
    let src = std::fs::read_to_string(&f).unwrap();
    std::fs::write(&f, src + "\n").unwrap();
    let mut buf = vec![0u8; 65536];
    let mut got = String::new();
    let t = std::time::Instant::now();
    while !got.contains("\"type\":\"diags\"") && t.elapsed() < Duration::from_secs(15) {
        if let Ok(n) = ws.read(&mut buf) {
            got.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
    assert!(got.contains("\"type\":\"diags\"") && got.contains("\"file\":\"comp/warn.brain\"") && got.contains("\"snippet\":{\"line\":1"), "{got}");
    // the terminal prints the same diagnostic, readable: level, code, file:line:col, message and fix
    std::thread::sleep(Duration::from_millis(200));
    let out = log.lock().unwrap().clone();
    assert!(out.lines().any(|l| l.starts_with("warning B0130 comp/warn.brain:")) && out.contains("\n    fix: "), "{out}");
    // a broken save prints its errors the same way, not as JSON
    std::fs::write(&f, "<script>\n  let x = \n</script>\n<p>{x}</p>\n").unwrap();
    let t = std::time::Instant::now();
    while !log.lock().unwrap().contains("error(s)") && t.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(200));
    let out = log.lock().unwrap().clone();
    assert!(out.lines().any(|l| l.starts_with("error B") && l.contains("comp/warn.brain:")) && !out.contains("{\"code\""), "{out}");
    let _ = c.kill();
    let _ = c.wait();
    // --mock: the page asks the client to answer api() with fake data; --no-inspect: no debugger
    let (mut c, app, dbg, _) = run(&dir, &["--mock=seed1", "--no-inspect"]);
    assert!(dbg.is_none());
    assert!(get(app, "/about").contains("<meta name=\"brain-mock\" content=\"seed1\">"));
    let _ = c.kill();
    let _ = c.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}
