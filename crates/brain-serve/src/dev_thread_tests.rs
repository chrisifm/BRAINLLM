use super::*;
use crate::server::{CompileFn, Compiled, MAX_CONNECTIONS, Options, start_with_threads};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "brain-thread-failure-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn options(&self, port: u16) -> Options {
        Options {
            root: self.0.clone(),
            host: "127.0.0.1".into(),
            port,
            head_prepend: String::new(),
            runtime: vec![("brain.js".into(), "export const healthy=true".into())],
            env: None,
            base: "/".into(),
            diagnose: None,
            problems: None,
            upstream: None,
            mock: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn compiler() -> CompileFn {
    Arc::new(|_, _| {
        Ok(Compiled {
            js: String::new(),
            css: String::new(),
            deps: vec![],
            server_js: String::new(),
        })
    })
}
fn request(addr: std::net::SocketAddr) -> (String, String) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(b"GET /@brain/brain.js?probe=private-canary HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer private-canary\r\nConnection: close\r\n\r\n").unwrap();
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    let mut length = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        assert!(!line.is_empty());
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let length = length.unwrap();
    assert!(length < 4096);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    (status, String::from_utf8(body).unwrap())
}
fn diagnostics(messages: &Mutex<Vec<String>>) -> Vec<serde_json::Value> {
    messages
        .lock()
        .unwrap()
        .iter()
        .filter_map(|message| serde_json::from_str::<serde_json::Value>(message).ok())
        .filter_map(|event| {
            event["msg"]
                .as_str()
                .and_then(|message| serde_json::from_str::<serde_json::Value>(message).ok())
        })
        .filter(|event| event["code"] == "BDEV_THREAD_DISPATCH")
        .collect()
}

#[test]
fn failed_dispatch_answers_503_releases_every_permit_and_next_request_recovers() {
    let fixture = Fixture::new();
    let threads = Threads::default();
    threads.state.failures[Role::Request as usize].store(1, Ordering::Release);
    let messages = Arc::new(Mutex::new(Vec::new()));
    let observed = messages.clone();
    let event: crate::EventFn =
        Arc::new(move |message| observed.lock().unwrap().push(message.to_string()));
    let mut dev = start_with_threads(
        fixture.options(0),
        compiler(),
        false,
        Some(event),
        None,
        threads.clone(),
    )
    .unwrap();
    let (status, body) = request(dev.addr);
    assert!(status.contains("503"));
    assert_eq!(body, "Dev request worker unavailable; retry shortly");
    assert!(!body.contains("private-canary"));
    assert!(
        request(dev.addr).0.contains("200"),
        "the immediately following request must recover"
    );
    // More failed dispatches than the entire admission budget expose even one retained slot.
    threads.state.failures[Role::Request as usize].store(MAX_CONNECTIONS, Ordering::Release);
    for _ in 0..MAX_CONNECTIONS {
        assert!(request(dev.addr).0.contains("503"));
    }
    assert!(request(dev.addr).0.contains("200"));
    dev.stop();
    let events = diagnostics(&messages);
    assert!(!events.is_empty());
    assert_eq!(
        events
            .iter()
            .map(|event| event["failures"].as_u64().unwrap())
            .sum::<u64>(),
        (MAX_CONNECTIONS + 1) as u64
    );
    assert_eq!(
        events.last().unwrap()["totalFailures"],
        (MAX_CONNECTIONS + 1) as u64
    );
    assert!(
        !messages
            .lock()
            .unwrap()
            .join("\n")
            .contains("private-canary")
    );
}

#[test]
fn dispatch_failure_logs_coalesce_without_losing_counts() {
    let fixture = Fixture::new();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let observed = messages.clone();
    let event: crate::EventFn =
        Arc::new(move |message| observed.lock().unwrap().push(message.to_string()));
    let mut dev = start_with_threads(
        fixture.options(0),
        compiler(),
        false,
        Some(event),
        None,
        Threads::default(),
    )
    .unwrap();
    let mut log = FailureLog::default();
    let at = Instant::now();
    for _ in 0..1000 {
        log.failed_at(&dev.ctx, at);
    }
    assert_eq!(diagnostics(&messages).len(), 1);
    log.flush_at(&dev.ctx, at + Duration::from_secs(1));
    let events = diagnostics(&messages);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["failures"], 1);
    assert_eq!(events[1]["failures"], 999);
    assert_eq!(events[1]["totalFailures"], 1000);
    log.finish(&dev.ctx);
    assert_eq!(diagnostics(&messages).len(), 2);
    dev.stop();
}

#[test]
fn startup_spawn_errors_release_listener_watcher_context_and_owned_socket_worker() {
    for role in [Role::Watch, Role::Accept] {
        let fixture = Fixture::new();
        let picked = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = picked.local_addr().unwrap();
        drop(picked);
        let threads = Threads::default();
        threads.state.failures[role as usize].store(1, Ordering::Release);
        let native = crate::NativeProxy::new("http://127.0.0.1:9")
            .unwrap()
            .sockets(true);
        let result = start_with_threads(
            fixture.options(address.port()),
            compiler(),
            true,
            None,
            Some(native),
            threads.clone(),
        );
        let error = match result {
            Err(error) => error,
            Ok(mut dev) => {
                dev.stop();
                panic!("injected startup must fail")
            }
        };
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error.to_string().contains("BDEV_THREAD_START"));
        assert!(error.to_string().contains("fix:"));
        assert!(
            TcpListener::bind(address).is_ok(),
            "failed startup must release its listener"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let gone = threads
                .state
                .context
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|context| context.upgrade().is_none());
            let running = threads
                .state
                .running
                .iter()
                .any(|count| count.load(Ordering::Acquire) > 0);
            if gone && !running {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "startup retained its owned context or watcher worker"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
