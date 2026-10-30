//! Startup tests use synthetic loopback authorities, not application data.
use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "brain-native-start-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn options(&self, port: u16) -> brain_serve::Options {
        brain_serve::Options {
            root: self.0.clone(),
            host: "127.0.0.1".into(),
            port,
            head_prepend: String::new(),
            runtime: vec![],
            env: Some(env::loader(&self.0, "development")),
            base: "/".into(),
            diagnose: None,
            problems: None,
            upstream: None,
            mock: None,
        }
    }
    fn enable(&self, port: u16, backend: Option<&str>) {
        let origin = format!("http://127.0.0.1:{port}");
        std::fs::write(self.0.join("brain.json"), format!(r#"{{"backend":{{"origins":["{origin}"],"sockets":{{"allowedOrigins":["{origin}"]}}}}}}"#)).unwrap();
        std::fs::write(
            self.0.join(".brain_env"),
            format!(
                "[development]\nPUBLIC_SITE_URL={origin}/\n{}",
                backend
                    .map(|url| format!("BRAIN_BACKEND_URL={url}\n"))
                    .unwrap_or_default()
            ),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn compiler() -> brain_serve::CompileFn {
    Arc::new(|_, _| {
        Ok(brain_serve::Compiled {
            js: String::new(),
            css: String::new(),
            deps: vec![],
            server_js: String::new(),
        })
    })
}

#[test]
fn native_startup_disabled_keeps_ephemeral_server_and_stops_owned_listener() {
    let root = Fixture::new("disabled");
    let mut dev =
        start_project_dev(root.options(0), compiler(), false, None, "development").unwrap();
    let addr = dev.addr;
    assert_ne!(addr.port(), 0);
    dev.stop();
    assert!(
        TcpListener::bind(addr).is_ok(),
        "owned listener must be released"
    );
}

#[test]
fn native_startup_rejects_missing_authority_and_stops_disallowed_auto_port() {
    let root = Fixture::new("mismatch");
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = occupied.local_addr().unwrap().port();
    root.enable(port, None);
    let error = start_project_dev(root.options(port), compiler(), false, None, "development")
        .err()
        .expect("missing backend must fail");
    assert!(error.contains("fixed BRAIN_BACKEND_URL"));
    root.enable(port, Some("http://127.0.0.1:9"));
    let error = start_project_dev(root.options(port), compiler(), false, None, "development")
        .err()
        .expect("auto port must fail closed");
    assert!(error.contains("was stopped"));
    let selected: u16 = error
        .split("selected port ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(selected, port);
    assert!(
        TcpListener::bind(("127.0.0.1", selected)).is_ok(),
        "rejected auto-selected listener must be released"
    );
    // The next available port was created by this operation and must be reaped.
    // Existing occupied listeners are not touched.
    assert!(TcpListener::bind(("127.0.0.1", port)).is_err());
}

#[test]
fn native_startup_pins_selected_authority_across_environment_edits() {
    let root = Fixture::new("fixed");
    let backend = TcpListener::bind("127.0.0.1:0").unwrap();
    backend.set_nonblocking(true).unwrap();
    let target = format!("http://{}", backend.local_addr().unwrap());
    std::fs::write(
        root.0.join(".brain_env"),
        format!("[development]\nBRAIN_BACKEND_URL={target}\n"),
    )
    .unwrap();
    let mut dev =
        start_project_dev(root.options(0), compiler(), false, None, "development").unwrap();
    std::fs::write(
        root.0.join(".brain_env"),
        "[development]\nBRAIN_BACKEND_URL=http://127.0.0.1:9\n",
    )
    .unwrap();
    let request = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match backend.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut bytes = [0; 4096];
                    let size = stream.read(&mut bytes).unwrap();
                    assert!(
                        std::str::from_utf8(&bytes[..size])
                            .unwrap()
                            .starts_with("GET /api/items ")
                    );
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
                    return;
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!(
                    "owned backend did not receive the request: {:?}",
                    error.kind()
                ),
            }
        }
    });
    let mut stream = TcpStream::connect(dev.addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(6)))
        .unwrap();
    stream
        .write_all(b"GET /api/items HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut result = String::new();
    stream.read_to_string(&mut result).unwrap();
    assert!(
        result.starts_with("HTTP/1.1 200"),
        "fixed authority must serve the request"
    );
    request.join().unwrap();
    dev.stop();
}

#[cfg(unix)]
#[test]
fn native_startup_test_address_uses_only_declared_numeric_http_loopback() {
    let root = Fixture::new("test-address");
    assert_eq!(
        project_test_address(&root.0).unwrap(),
        ("127.0.0.1".into(), 0)
    );
    root.enable(5187, Some("http://127.0.0.1:9"));
    assert_eq!(
        project_test_address(&root.0).unwrap(),
        ("127.0.0.1".into(), 5187)
    );
    for site in [
        "https://127.0.0.1:5187",
        "http://localhost:5187",
        "http://192.0.2.1:5187",
        "http://127.0.0.1:5188",
    ] {
        std::fs::write(
            root.0.join(".brain_env"),
            format!("[development]\nPUBLIC_SITE_URL={site}\n"),
        )
        .unwrap();
        assert!(
            project_test_address(&root.0).is_err(),
            "unlisted or non-numeric test authority must fail"
        );
    }
}

#[cfg(unix)]
#[test]
fn native_startup_static_manifest_rejection_precedes_pid_and_control_files() {
    let root = Fixture::new("static-invalid");
    std::fs::create_dir_all(root.0.join("dist/_server")).unwrap();
    std::fs::write(
        root.0.join("dist/index.html"),
        "<!doctype html><p>Fixture</p>",
    )
    .unwrap();
    std::fs::write(
        root.0.join("dist/_server/manifest.json"),
        r#"{"v":1,"nativeSockets":"true"}"#,
    )
    .unwrap();
    let options = start::Opts {
        host: "127.0.0.1".into(),
        port: 0,
        instances: 1,
        max_rss_kb: 300 * 1024,
        env: "production".into(),
        health_ms: 10000,
        rss_ms: 5000,
        backoff_ms: 100,
    };
    let error = start::start(&root.0, &options).unwrap_err();
    assert!(error.contains("static gateway"));
    assert!(!root.0.join(".brain/run/brain.pid").exists());
    assert!(!root.0.join(".brain/run/brain.sock").exists());
}
