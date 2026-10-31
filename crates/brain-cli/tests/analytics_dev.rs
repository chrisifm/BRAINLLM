//! Behavior analytics under `brain run`: the page links the collector only when a backend serves it, and the link works.
#![cfg(unix)]
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn app(name: &str, backend: Option<&str>) -> PathBuf {
    let tmp = std::env::temp_dir().join(format!("brain-analytics-dev-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let new = Command::new(env!("CARGO_BIN_EXE_brain")).args(["new", "app"]).current_dir(&tmp).stdin(Stdio::null()).output().unwrap();
    assert!(new.status.success());
    let dir = tmp.join("app");
    if let Some(url) = backend {
        let env = std::fs::read_to_string(dir.join(".brain_env")).unwrap();
        std::fs::write(dir.join(".brain_env"), env.replacen("[development]", &format!("[development]\nBRAIN_BACKEND_URL={url}"), 1)).unwrap();
    }
    dir
}

fn get(port: u16, path: &str) -> String {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(socket, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    socket.read_to_string(&mut out).unwrap();
    out
}

/// `brain run` on a free port, its output discarded; `body` gets the port and the dev server is stopped after it.
fn with_dev(dir: &std::path::Path, extra: &[&str], body: impl FnOnce(u16)) {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut dev = Command::new(env!("CARGO_BIN_EXE_brain")).args(["run", "--no-inspect", "--port", &port.to_string()]).args(extra).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    for _ in 0..200 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() { break }
        std::thread::sleep(Duration::from_millis(50));
    }
    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(port)));
    let _ = dev.kill();
    let _ = dev.wait();
    if let Err(panic) = done { std::panic::resume_unwind(panic) }
}

/// A backend that answers the collector request and reports the path it was asked for.
fn collector_backend() -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let served = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut bytes = [0u8; 4096];
        let n = socket.read(&mut bytes).unwrap();
        let head = String::from_utf8_lossy(&bytes[..n]).into_owned();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: 9\r\nConnection: close\r\n\r\nexport {}").unwrap();
        head.split(' ').nth(1).unwrap_or("").to_string()
    });
    (url, served)
}

#[test]
fn a_page_without_a_backend_does_not_link_the_collector() {
    let dir = app("none", None);
    with_dev(&dir, &[], |port| {
        let page = get(port, "/");
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(!page.contains("_brain/analytics.js"), "{page}");
    });
}

#[test]
fn the_collector_a_native_page_links_is_served_by_the_backend() {
    let (url, served) = collector_backend();
    let dir = app("native", Some(&url));
    with_dev(&dir, &[], |port| {
        assert!(get(port, "/").contains("src=\"/_brain/analytics.js\""));
        let script = get(port, "/_brain/analytics.js");
        assert!(script.starts_with("HTTP/1.1 200"), "{script}");
        assert!(script.ends_with("export {}"), "{script}");
    });
    assert_eq!(served.join().unwrap(), "/_brain/analytics.js");
}

#[test]
fn the_collector_link_keeps_the_base_path() {
    let (url, served) = collector_backend();
    let dir = app("base", Some(&url));
    with_dev(&dir, &["--base", "/app/"], |port| {
        let page = get(port, "/app/");
        assert!(page.contains("src=\"/app/_brain/analytics.js\""), "{page}");
        assert!(get(port, "/app/_brain/analytics.js").starts_with("HTTP/1.1 200"));
    });
    assert_eq!(served.join().unwrap(), "/_brain/analytics.js");
}
