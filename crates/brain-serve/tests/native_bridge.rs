//! The Rust static gateway's bridge: client identity, streamed files, declared routes, conditional requests.
use brain_serve::prod::StaticGateway;
use std::{
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

const MIB: usize = 1024 * 1024;

#[derive(Clone, Debug, Default)]
struct Seen {
    head: String,
    body: Vec<u8>,
}
impl Seen {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim().to_string())
        })
    }
}

fn read_upstream(socket: &mut TcpStream) -> Seen {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 65536];
    let head_end = loop {
        if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        let n = socket.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            return Seen::default();
        }
        bytes.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&bytes[..head_end]).into_owned();
    let mut seen = Seen { head, body: bytes[head_end + 4..].to_vec() };
    let length = seen.header("content-length").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    while seen.body.len() < length {
        let n = socket.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        seen.body.extend_from_slice(&chunk[..n]);
    }
    seen
}

/// A backend that answers each connection through `respond`; records what it received.
struct Backend {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}
fn backend(respond: impl Fn(&Seen, &mut TcpStream) + Send + Sync + 'static) -> Backend {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (record, respond) = (seen.clone(), Arc::new(respond));
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { return };
            let (record, respond) = (record.clone(), respond.clone());
            std::thread::spawn(move || {
                let request = read_upstream(&mut socket);
                record.lock().unwrap().push(request.clone());
                respond(&request, &mut socket);
            });
        }
    });
    Backend { url, seen }
}

fn app(manifest: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "brain-native-bridge-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("_server")).unwrap();
    std::fs::write(dir.join("_server/manifest.json"), manifest).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><title>Shop</title>").unwrap();
    dir
}
const ROUTES: &str = r#"{"v":1,"routes":[],"rpc":{},"nativeRoutes":[{"path":"/payments/events","headers":["x-pay-signature"]},{"path":"/orders/{id}/quote","headers":[]}]}"#;

/// Serves connections through the gateway on a free port.
fn gateway(manifest: &str, backend: &Backend) -> SocketAddr {
    let dist = app(manifest);
    let gateway = Arc::new(StaticGateway::load(&dist, "/", Some(&backend.url)).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(socket) = socket else { return };
            let gateway = gateway.clone();
            std::thread::spawn(move || {
                let _ = gateway.handle(socket);
            });
        }
    });
    address
}

fn call(address: SocketAddr, head: &str, body: &[u8]) -> (u16, String, Vec<u8>) {
    let mut socket = TcpStream::connect(address).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    socket.write_all(head.as_bytes()).unwrap();
    let _ = socket.write_all(body);
    let mut all = Vec::new();
    let _ = socket.read_to_end(&mut all);
    let at = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(0);
    let text = String::from_utf8_lossy(&all[..at]).into_owned();
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, text.to_ascii_lowercase(), all.get(at + 4..).unwrap_or_default().to_vec())
}
fn ok_json(socket: &mut TcpStream, status: &str, extra: &str) {
    let _ = socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n{extra}Connection: close\r\n\r\n{{}}").as_bytes());
}

#[test]
fn the_peer_is_appended_after_client_hops_so_a_browser_header_cannot_set_the_identity() {
    let b = backend(|_, s| ok_json(s, "200 OK", ""));
    let at = gateway(ROUTES, &b);
    call(at, "GET /api/items HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", b"");
    call(at, "GET /api/items HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 1.1.1.1\r\nX-Forwarded-Host: evil.test\r\nConnection: close\r\n\r\n", b"");
    let long = "9".repeat(5000);
    call(at, &format!("GET /api/items HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: {long}\r\nConnection: close\r\n\r\n"), b"");
    let seen = b.seen.lock().unwrap();
    assert_eq!(seen[0].header("x-forwarded-for").as_deref(), Some("127.0.0.1"));
    assert_eq!(seen[1].header("x-forwarded-for").as_deref(), Some("1.1.1.1, 127.0.0.1"));
    assert_eq!(seen[1].header("x-forwarded-host"), None);
    assert_eq!(seen[2].header("x-forwarded-for").as_deref(), Some("127.0.0.1"));
}

#[test]
fn a_declared_webhook_keeps_raw_bytes_and_declared_headers_and_unknown_paths_stay_local() {
    let b = backend(|_, s| ok_json(s, "202 Accepted", "X-Shop-Trace: t1\r\nLink: </next>; rel=\"next\"\r\nContent-Language: en\r\nCache-Control: public, max-age=5\r\nSet-Cookie: unrelated=never\r\nX-Request-Id: upstream\r\n"));
    let at = gateway(ROUTES, &b);
    let raw = [0x7bu8, 0x20, 0x20, 0xff, 0x00, 0x0a, 0x7d, 0x20];
    let (status, head, _) = call(at, &format!("POST /payments/events?attempt=1 HTTP/1.1\r\nHost: x\r\nContent-Type: application/octet-stream\r\nX-Pay-Signature: sha256=abc\r\nX-Private: never\r\nCookie: __Host-brain=s; other=never\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", raw.len()), &raw);
    assert_eq!(status, 202);
    assert!(head.contains("x-shop-trace: t1") && head.contains("link: </next>") && head.contains("content-language: en"), "{head}");
    assert!(head.contains("cache-control: public, max-age=5") && !head.contains("no-store"), "{head}");
    assert!(!head.contains("unrelated") && !head.contains("upstream"), "{head}");
    {
        let seen = b.seen.lock().unwrap();
        assert_eq!(seen[0].body, raw);
        assert!(seen[0].head.starts_with("POST /payments/events?attempt=1 "), "{}", seen[0].head);
        assert_eq!(seen[0].header("x-pay-signature").as_deref(), Some("sha256=abc"));
        assert_eq!(seen[0].header("x-private"), None);
        assert_eq!(seen[0].header("cookie").as_deref(), Some("__Host-brain=s"));
    }
    assert_eq!(call(at, "POST /orders/42/quote HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", b"").0, 202);
    let count = b.seen.lock().unwrap().len();
    assert_eq!(call(at, "POST /orders/42/43/quote HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", b"").0, 405);
    assert_eq!(call(at, "POST /not/declared HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", b"").0, 405);
    assert_eq!(call(at, "GET /not/declared HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", b"").0, 404);
    assert_eq!(b.seen.lock().unwrap().len(), count, "undeclared paths never reach the backend");
}

#[test]
fn declared_routes_and_batches_accept_one_mib_and_other_bodies_keep_their_budget() {
    let b = backend(|_, s| ok_json(s, "200 OK", ""));
    let at = gateway(ROUTES, &b);
    let body = vec![1u8; MIB];
    let post = |path: &str, len: usize| format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
    assert_eq!(call(at, &post("/payments/events", MIB), &body).0, 200);
    assert_eq!(b.seen.lock().unwrap()[0].body.len(), MIB);
    assert_eq!(call(at, &post("/api/items/batch", MIB), &body).0, 200);
    assert_eq!(call(at, &post("/payments/events", MIB + 1), b"").0, 413);
    assert_eq!(call(at, &post("/api/items", 65537), b"").0, 413);
    assert_eq!(b.seen.lock().unwrap().len(), 2);
}

#[test]
fn a_large_download_streams_with_range_validators_and_safety_headers() {
    let b = backend(|request, s| {
        if request.head.contains("/ranged") {
            let _ = s.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Type: application/pdf\r\nContent-Range: bytes 0-3/10\r\nContent-Length: 4\r\nAccept-Ranges: bytes\r\nETag: \"f1\"\r\nConnection: close\r\n\r\nabcd");
        } else if request.head.contains("/fresh") {
            let _ = s.write_all(b"HTTP/1.1 304 Not Modified\r\nETag: \"f1\"\r\nConnection: close\r\n\r\n");
        } else {
            let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\nETag: \"f1\"\r\nAccept-Ranges: bytes\r\nContent-Security-Policy: default-src 'none'\r\nX-Content-Type-Options: nosniff\r\nCache-Control: public, max-age=31536000, immutable\r\nX-Private-Upstream: never\r\nConnection: close\r\n\r\n", 25 * MIB).as_bytes());
            if !request.head.starts_with("HEAD") {
                let _ = s.write_all(&vec![2u8; 25 * MIB]);
            }
        }
    });
    let at = gateway(ROUTES, &b);
    let (status, head, body) = call(at, "GET /_brain/files/attachments/large HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", b"");
    assert_eq!(status, 200);
    assert_eq!(body.len(), 25 * MIB);
    for wanted in ["etag: \"f1\"", "accept-ranges: bytes", "content-security-policy: default-src 'none'", "x-content-type-options: nosniff", "cache-control: public, max-age=31536000, immutable"] {
        assert!(head.contains(wanted), "{wanted}: {head}");
    }
    assert!(!head.contains("x-private-upstream"), "{head}");
    let (status, head, body) = call(at, "HEAD /_brain/files/attachments/large HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", b"");
    assert_eq!(status, 200);
    assert!(head.contains(&format!("content-length: {}", 25 * MIB)) && body.is_empty(), "{head}");
    let (status, head, body) = call(at, "GET /_brain/files/attachments/ranged HTTP/1.1\r\nHost: x\r\nRange: bytes=0-3\r\nIf-Range: \"f1\"\r\nConnection: close\r\n\r\n", b"");
    assert_eq!((status, body.as_slice()), (206, &b"abcd"[..]));
    assert!(head.contains("content-range: bytes 0-3/10"), "{head}");
    {
        let seen = b.seen.lock().unwrap();
        let ranged = seen.iter().find(|s| s.head.contains("/ranged")).unwrap();
        assert_eq!(ranged.header("range").as_deref(), Some("bytes=0-3"));
        assert_eq!(ranged.header("if-range").as_deref(), Some("\"f1\""));
    }
    let (status, head, _) = call(at, "GET /_brain/files/attachments/fresh HTTP/1.1\r\nHost: x\r\nIf-None-Match: \"f1\"\r\nConnection: close\r\n\r\n", b"");
    assert_eq!(status, 304);
    assert!(head.contains("etag: \"f1\""), "{head}");
    call(at, "GET /api/items HTTP/1.1\r\nHost: x\r\nRange: bytes=0-1\r\nConnection: close\r\n\r\n", b"");
    assert_eq!(b.seen.lock().unwrap().last().unwrap().header("range"), None, "Range is a file header only");
}

#[test]
fn first_bytes_arrive_before_the_backend_finishes_and_a_disconnect_closes_the_upstream() {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let b = backend(move |_, s| {
        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", 200 * MIB).as_bytes());
        let chunk = vec![1u8; MIB];
        let mut failed = false;
        for _ in 0..200 {
            if s.write_all(&chunk).is_err() {
                failed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = tx.lock().unwrap().send(failed);
    });
    let at = gateway(ROUTES, &b);
    let mut socket = TcpStream::connect(at).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    socket.write_all(b"GET /_brain/files/attachments/huge HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    let mut first = [0u8; 4096];
    assert!(socket.read(&mut first).unwrap() > 0, "bytes flow while the backend is still sending");
    socket.shutdown(Shutdown::Both).unwrap();
    drop(socket);
    assert!(rx.recv_timeout(Duration::from_secs(30)).unwrap(), "the upstream was destroyed before finishing");
}

#[test]
fn uploads_stream_past_the_api_budget_and_a_bucket_refusal_reaches_the_client() {
    let b = backend(|request, s| {
        if request.head.contains("/refused") {
            let _ = s.write_all(b"HTTP/1.1 413 Content Too Large\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
        } else {
            ok_json(s, "201 Created", "Location: /_brain/files/attachments/1\r\n");
        }
    });
    let at = gateway(ROUTES, &b);
    let body = vec![3u8; 8 * MIB];
    let (status, head, _) = call(at, &format!("POST /_brain/files/attachments HTTP/1.1\r\nHost: x\r\nContent-Type: multipart/form-data; boundary=x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()), &body);
    assert_eq!(status, 201);
    assert!(head.contains("location: /_brain/files/attachments/1"), "{head}");
    assert_eq!(b.seen.lock().unwrap()[0].body.len(), 8 * MIB);
    assert_eq!(call(at, "POST /_brain/files/attachments/refused HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd", b"").0, 413);
}

#[test]
fn api_validators_and_public_lifetimes_survive_so_if_match_can_refuse_a_lost_update() {
    let b = backend(|request, s| {
        if request.header("if-match").is_some() {
            ok_json(s, "412 Precondition Failed", "");
        } else if request.header("if-none-match").is_some() {
            let _ = s.write_all(b"HTTP/1.1 304 Not Modified\r\nETag: \"r1\"\r\nCache-Control: public, max-age=60\r\nConnection: close\r\n\r\n");
        } else {
            ok_json(s, "200 OK", "ETag: \"r1\"\r\nCache-Control: public, max-age=60\r\n");
        }
    });
    let at = gateway(ROUTES, &b);
    let (status, head, _) = call(at, "GET /api/items/1 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", b"");
    assert_eq!(status, 200);
    assert!(head.contains("etag: \"r1\"") && head.contains("cache-control: public, max-age=60") && !head.contains("no-store"), "{head}");
    let (status, head, _) = call(at, "GET /api/items/1 HTTP/1.1\r\nHost: x\r\nIf-None-Match: \"r1\"\r\nConnection: close\r\n\r\n", b"");
    assert_eq!(status, 304);
    assert!(head.contains("cache-control: public, max-age=60"), "{head}");
    assert_eq!(call(at, "PATCH /api/items/1 HTTP/1.1\r\nHost: x\r\nIf-Match: \"old\"\r\nContent-Length: 2\r\nConnection: close\r\n\r\n", b"{}").0, 412);
}

#[test]
fn a_project_api_key_crosses_the_bridge_and_no_origin_is_invented() {
    let b = backend(|_, s| ok_json(s, "201 Created", ""));
    let at = gateway(ROUTES, &b);
    assert_eq!(call(at, "POST /api/items HTTP/1.1\r\nHost: x\r\nX-API-Key: bk_live_0123456789ab_secret\r\nContent-Length: 2\r\nConnection: close\r\n\r\n", b"{}").0, 201);
    call(at, "GET /api/items HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer bk_live_0123456789ab_secret\r\nConnection: close\r\n\r\n", b"");
    let seen = b.seen.lock().unwrap();
    assert_eq!(seen[0].header("x-api-key").as_deref(), Some("bk_live_0123456789ab_secret"));
    assert_eq!(seen[0].header("origin"), None);
    assert_eq!(seen[1].header("authorization").as_deref(), Some("Bearer bk_live_0123456789ab_secret"));
}
