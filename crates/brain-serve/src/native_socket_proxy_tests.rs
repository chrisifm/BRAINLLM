use super::*;
use crate::NativeProxy;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpListener},
    sync::atomic::AtomicBool,
    time::Instant,
};
const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";
fn raw(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: frontend.example.test\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: brain.v1\r\nSec-WebSocket-Key: {KEY}\r\nOrigin: https://frontend.example.test\r\nCookie: __Host-brain={}; __Host-brain-refresh=invalid+private-refresh; unrelated=private-other\r\nX-Forwarded-For: private-spoof\r\nX-Request-Id: untrusted-request-identity\r\n\r\n","a".repeat(64)).into_bytes()
}
fn response() -> Vec<u8> {
    format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {ACCEPT}\r\nSec-WebSocket-Protocol: brain.v1\r\n\r\n").into_bytes()
}
fn client(proxy: &NativeSocketProxy, bytes: &[u8], base: &str) -> BufReader<TcpStream> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(7)))
        .unwrap();
    client
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let (mut incoming, _) = listener.accept().unwrap();
    incoming
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client.write_all(bytes).unwrap();
    let mut request = http::read_request(incoming.try_clone().unwrap()).unwrap();
    if !base.is_empty() {
        request.path = request.path.strip_prefix(base).unwrap().to_owned();
    }
    proxy.takeover(&mut incoming, &mut request).unwrap();
    drop(incoming);
    BufReader::new(client)
}
fn read_head(client: &mut BufReader<TcpStream>) -> String {
    let mut head = String::new();
    loop {
        let mut line = String::new();
        client.read_line(&mut line).unwrap();
        assert!(
            !line.is_empty(),
            "the transport must respond or finish its handshake"
        );
        head.push_str(&line);
        assert!(head.len() <= 32768);
        if line == "\r\n" {
            return head;
        }
    }
}
fn proxy(address: std::net::SocketAddr) -> NativeSocketProxy {
    NativeProxy::new(&format!("http://{address}"))
        .unwrap()
        .sockets(true)
        .start_sockets()
        .unwrap()
        .unwrap()
}
fn inactive(proxy: &NativeSocketProxy) {
    let end = Instant::now() + Duration::from_secs(2);
    while proxy.stats().active != 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(proxy.stats().active, 0);
}

#[test]
fn disabled_by_default_and_wire_metadata_preserve_http_compatibility() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = NativeProxy::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    assert!(config.start_sockets().unwrap().is_none());
    assert_eq!(
        listener.accept().err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    let with_empty_query = raw("/_brain/socket?");
    let request = http::read_request(with_empty_query.as_slice()).unwrap();
    assert!(request.upgrade.query_present);
    assert!(request.query.is_empty());
    assert_eq!(request.upgrade.version, "HTTP/1.1");
    let request = http::read_request(
        b"POST /api/items HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}private-following-data".as_slice(),
    )
    .unwrap();
    assert_eq!(request.body, b"{}");
    assert!(request.upgrade.head.is_empty());
}
#[test]
fn malformed_handshakes_are_rejected_before_any_upstream_contact() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let proxy = proxy(listener.local_addr().unwrap());
    let original = String::from_utf8(raw("/_brain/socket")).unwrap();
    let variants = vec![
        original.replace("GET /", "POST /"),
        original.replace("HTTP/1.1", "HTTP/1.0"),
        String::from_utf8(raw("/_brain/socket?")).unwrap(),
        String::from_utf8(raw("/_brain/socket?token=private-url-value")).unwrap(),
        original.replace("Version: 13", "Version: 12"),
        original.replace("Protocol: brain.v1", "Protocol: other"),
        original.replace(KEY, "invalid-key"),
        original.replace("Origin: https://frontend.example.test\r\n", ""),
        original.replace("https://frontend.example.test", "null"),
        original.replace("\r\n\r\n", "\r\nContent-Length: 1\r\n\r\nx"),
    ];
    for value in variants {
        let mut client = client(&proxy, value.as_bytes(), "");
        let head = read_head(&mut client);
        assert!(head.starts_with("HTTP/1.1 400") || head.starts_with("HTTP/1.1 403"));
        let mut body = String::new();
        client.read_to_string(&mut body).unwrap();
        assert!(!body.contains("private-url-value"));
        assert!(!body.contains("private-refresh"));
    }
    assert_eq!(proxy.stats().accepted, 0);
    assert_eq!(
        listener.accept().err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    for event in proxy.drain_diagnostics() {
        let json = event.to_json();
        assert!(json.contains("\"phase\":\"request\""));
        assert!(!json.contains("private-refresh"));
        assert!(!json.contains(&"a".repeat(64)));
    }
    proxy.stop().unwrap();
}
#[test]
fn both_read_aheads_and_opaque_binary_bytes_survive_without_forwarded_identity() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = proxy(listener.local_addr().unwrap());
    let outgoing = b"\x82\x05\0\xffxyz";
    let incoming = b"\x81\x83maskxyz";
    let backend = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let request = http::read_request(socket.try_clone().unwrap()).unwrap();
        assert_eq!(request.path, "/_brain/socket");
        assert!(request.query.is_empty());
        assert_eq!(
            request.header("origin"),
            Some("https://frontend.example.test")
        );
        assert!(!request.header("cookie").unwrap().contains("private-"));
        assert!(request.header("x-forwarded-for").is_none());
        assert_ne!(
            request.header("x-request-id"),
            Some("untrusted-request-identity")
        );
        let mut reply = response();
        reply.extend_from_slice(outgoing);
        socket.write_all(&reply).unwrap();
        let mut bytes = [0u8; 10];
        socket.read_exact(&mut bytes[..incoming.len()]).unwrap();
        assert_eq!(&bytes[..incoming.len()], incoming);
        socket.write_all(incoming).unwrap();
    });
    let mut bytes = raw("/app/_brain/socket");
    bytes.extend_from_slice(incoming);
    let mut client = client(&proxy, &bytes, "/app");
    assert!(read_head(&mut client).starts_with("HTTP/1.1 101"));
    let mut received = vec![0; outgoing.len() + incoming.len()];
    client.read_exact(&mut received).unwrap();
    assert_eq!(&received[..outgoing.len()], outgoing);
    assert_eq!(&received[outgoing.len()..], incoming);
    drop(client);
    backend.join().unwrap();
    inactive(&proxy);
    proxy.stop().unwrap();
}
#[test]
fn half_close_delivers_the_backend_tail_before_finishing_the_other_direction() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = proxy(listener.local_addr().unwrap());
    let backend = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        http::read_request(socket.try_clone().unwrap()).unwrap();
        socket.write_all(&response()).unwrap();
        let mut body = Vec::new();
        socket.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"client-before-fin");
        socket.write_all(b"backend-after-fin").unwrap();
    });
    let mut client = client(&proxy, &raw("/_brain/socket"), "");
    read_head(&mut client);
    client.get_mut().write_all(b"client-before-fin").unwrap();
    client.get_mut().shutdown(Shutdown::Write).unwrap();
    let mut tail = Vec::new();
    client.read_to_end(&mut tail).unwrap();
    assert_eq!(tail, b"backend-after-fin");
    backend.join().unwrap();
    inactive(&proxy);
    proxy.stop().unwrap();
}
#[test]
fn malformed_upstream_and_auth_rejection_never_become_successful_upgrades() {
    let valid = String::from_utf8(response()).unwrap();
    let replies = vec![
        valid.replace(ACCEPT, "wrong"),
        valid.replace("Sec-WebSocket-Protocol: brain.v1\r\n", ""),
        valid.replace(" 101 ", " 0101 "),
        valid.replace(
            "\r\n\r\n",
            "\r\nSec-WebSocket-Extensions: permessage-deflate\r\n\r\n",
        ),
        valid.replace(
            "\r\n\r\n",
            &format!("\r\nSec-WebSocket-Accept: {ACCEPT}\r\n\r\n"),
        ),
    ];
    for reply in replies {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = proxy(listener.local_addr().unwrap());
        let backend = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            http::read_request(s.try_clone().unwrap()).unwrap();
            let _ = s.write_all(reply.as_bytes());
        });
        let mut client = client(&proxy, &raw("/_brain/socket"), "");
        assert!(read_head(&mut client).starts_with("HTTP/1.1 502"));
        let mut body = String::new();
        client.read_to_string(&mut body).unwrap();
        assert!(!body.contains("private-refresh"));
        backend.join().unwrap();
        inactive(&proxy);
        assert_eq!(proxy.drain_diagnostics()[0].phase, "handshake");
        proxy.stop().unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = proxy(listener.local_addr().unwrap());
    let backend = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        http::read_request(s.try_clone().unwrap()).unwrap();
        s.write_all(
            b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 25\r\n\r\nprivate-upstream-response",
        )
        .unwrap();
    });
    let mut client = client(&proxy, &raw("/_brain/socket"), "");
    assert!(read_head(&mut client).starts_with("HTTP/1.1 401"));
    let mut body = String::new();
    client.read_to_string(&mut body).unwrap();
    assert!(!body.contains("private-upstream-response"));
    backend.join().unwrap();
    inactive(&proxy);
    proxy.stop().unwrap();
}
#[test]
fn slow_handshake_has_one_total_deadline_and_cancellation_reaps_the_worker() {
    for cancel in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = proxy(listener.local_addr().unwrap());
        let (tx, rx) = sync::sync_channel(1);
        let backend = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
            http::read_request(s.try_clone().unwrap()).unwrap();
            tx.send(()).unwrap();
            let mut tail = Vec::new();
            let _ = s.read_to_end(&mut tail);
        });
        let mut client = client(&proxy, &raw("/_brain/socket"), "");
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let start = Instant::now();
        if cancel {
            proxy.stop().unwrap();
            assert!(start.elapsed() < Duration::from_secs(2));
            let mut tail = Vec::new();
            let _ = client.read_to_end(&mut tail);
            assert!(tail.is_empty());
        } else {
            assert!(read_head(&mut client).starts_with("HTTP/1.1 504"));
            assert!(start.elapsed() < Duration::from_secs(7));
            assert_eq!(proxy.drain_diagnostics()[0].phase, "handshake");
            proxy.stop().unwrap();
        }
        backend.join().unwrap();
        assert_eq!(proxy.stats().active, 0);
    }
}

struct HeldBackend {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    http: Arc<AtomicUsize>,
    auth: Arc<AtomicUsize>,
}
impl HeldBackend {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let http_count = Arc::new(AtomicUsize::new(0));
        let auth_count = Arc::new(AtomicUsize::new(0));
        let http_worker = http_count.clone();
        let auth_worker = auth_count.clone();
        let thread = std::thread::spawn(move || {
            let mut workers = Vec::new();
            while !done.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut s, _)) => {
                        // BSD sockets inherit O_NONBLOCK from the listener; blocking reads below would see WouldBlock.
                        s.set_nonblocking(false).unwrap();
                        if workers.len() >= 64 {
                            drop(s);
                            continue;
                        }
                        let stopped = done.clone();
                        let http_count = http_worker.clone();
                        let auth_count = auth_worker.clone();
                        workers.push(std::thread::spawn(move || {
                            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                            let Ok(request) = native_proxy::read_request(&s,None) else { return; };
                            if request.path == "/api/authority" {
                                http_count.fetch_add(1, Ordering::SeqCst);
                                let body = "{\"authority\":\"explicit\"}";
                                let _=write!(s,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                                return;
                            }
                            if request.path == "/_brain/auth/me" {
                                auth_count.fetch_add(1, Ordering::SeqCst);
                                let _=s.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                                return;
                            }
                            if request.path != "/_brain/socket" { return; }
                            if s.write_all(&response()).is_err() {
                                return;
                            }
                            s.set_read_timeout(Some(Duration::from_millis(25))).unwrap();
                            let mut data = [0u8; 4096];
                            while !stopped.load(Ordering::Acquire) {
                                match s.read(&mut data) {
                                    Ok(0) => break,
                                    Ok(_) => {}
                                    Err(e)
                                        if matches!(
                                            e.kind(),
                                            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                                        ) => {}
                                    Err(_) => break,
                                }
                            }
                        }));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => break,
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
            http: http_count,
            auth: auth_count,
        }
    }
}
impl Drop for HeldBackend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
#[test]
fn thirty_two_connections_have_fixed_admission_and_stop_releases_every_owned_tunnel() {
    let backend = HeldBackend::new();
    let proxy = proxy(backend.address);
    let mut clients = Vec::new();
    for _ in 0..CONNECTIONS {
        let mut client = client(&proxy, &raw("/_brain/socket"), "");
        assert!(read_head(&mut client).starts_with("HTTP/1.1 101"));
        clients.push(client);
    }
    let mut excess = client(&proxy, &raw("/_brain/socket"), "");
    assert!(read_head(&mut excess).starts_with("HTTP/1.1 503"));
    let stats = proxy.stats();
    assert_eq!(stats.active, 32);
    assert_eq!(stats.high_water, 32);
    assert_eq!(stats.accepted, 32);
    assert_eq!(stats.rejected, 1);
    let start = Instant::now();
    proxy.stop().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(proxy.stats().active, 0);
    for client in &mut clients {
        let mut byte = [0u8; 1];
        assert!(matches!(client.read(&mut byte), Ok(0) | Err(_)));
    }
    proxy.stop().unwrap();
}

#[cfg(unix)]
#[test]
fn blocked_upstream_writes_expire_without_growing_application_buffers() {
    use std::os::fd::AsRawFd;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = proxy(listener.local_addr().unwrap());
    let (ready_tx, ready) = sync::sync_channel(1);
    let (stop_tx, stop) = sync::sync_channel(1);
    let backend = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let bytes = 4096i32;
        // SAFETY: this owned live TCP descriptor and the fixed integer option remain valid for the call.
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    s.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_RCVBUF,
                    (&bytes as *const i32).cast(),
                    std::mem::size_of_val(&bytes) as libc::socklen_t,
                )
            },
            0
        );
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        http::read_request(s.try_clone().unwrap()).unwrap();
        s.write_all(&response()).unwrap();
        ready_tx.send(()).unwrap();
        let _ = stop.recv_timeout(Duration::from_secs(8));
    });
    let mut client = client(&proxy, &raw("/_brain/socket"), "");
    read_head(&mut client);
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    client
        .get_ref()
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let start = Instant::now();
    assert!(
        client
            .get_mut()
            .write_all(&vec![b'x'; 16 * 1024 * 1024])
            .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(6));
    inactive(&proxy);
    assert!(
        proxy
            .drain_diagnostics()
            .iter()
            .any(|item| item.code == "BNATIVE_SOCKET_WRITE_TIMEOUT" && item.phase == "write")
    );
    let _ = stop_tx.send(());
    backend.join().unwrap();
    proxy.stop().unwrap();
}

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "brain-dev-native-proxy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("views")).unwrap();
        std::fs::write(root.join("views/index.brain"), "<p>Native fixture</p>").unwrap();
        Self(root)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dev_options(root: &Temp, env: Option<crate::EnvLoader>) -> crate::Options {
    let mut runtime = vec![(
        "server/server.mjs".into(),
        include_str!("../../../runtime/server/server.mjs").into(),
    )];
    // A newer production helper may import the additive native tunnel module. Never hide a missing import failure.
    let optional = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/server/native-sockets.mjs");
    if let Ok(source) = std::fs::read_to_string(optional) {
        runtime.push(("server/native-sockets.mjs".into(), source));
    }
    crate::Options {
        root: root.0.clone(),
        host: "127.0.0.1".into(),
        port: 0,
        head_prepend: String::new(),
        runtime,
        env,
        base: "/app/".into(),
        diagnose: None,
        problems: None,
        upstream: None,
        mock: None,
    }
}
fn compile_loader(code: &str) -> crate::CompileFn {
    let code = code.to_owned();
    Arc::new(move |_, _| {
        Ok(crate::Compiled {
            js: "export default function(){}".into(),
            css: String::new(),
            deps: vec![],
            server_js: code.clone(),
        })
    })
}
fn dev_client(addr: std::net::SocketAddr, bytes: &[u8]) -> BufReader<TcpStream> {
    let mut client = TcpStream::connect(addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(6)))
        .unwrap();
    client
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client.write_all(bytes).unwrap();
    BufReader::new(client)
}
#[test]
fn managed_dev_stop_reaps_thirty_two_tunnels_without_intercepting_http_or_hmr() {
    let backend = HeldBackend::new();
    let wrong = TcpListener::bind("127.0.0.1:0").unwrap();
    wrong.set_nonblocking(true).unwrap();
    let wrong_url = format!("http://{}", wrong.local_addr().unwrap());
    let root = Temp::new();
    let env: crate::EnvLoader = Arc::new(move || {
        vec![
            ("BRAIN_BACKEND_URL".into(), wrong_url.clone()),
            ("PUBLIC_KEEP".into(), "kept".into()),
        ]
    });
    let mut dev = crate::start_dev_with_native_proxy(
        dev_options(&root, Some(env)),
        compile_loader(&format!("export async function load(ctx) {{ if(ctx.env.BRAIN_BACKEND_URL !== {}) throw new Error('Authority mismatch'); return {{ marker: ctx.env.PUBLIC_KEEP }} }}",crate::json_str(&format!("http://{}",backend.address)))),
        false,
        None,
        Some(
            NativeProxy::new(&format!("http://{}", backend.address))
                .unwrap()
                .sockets(true),
        ),
    )
    .unwrap();
    let mut clients = Vec::new();
    for _ in 0..CONNECTIONS {
        let mut client = dev_client(dev.addr, &raw("/app/_brain/socket"));
        assert!(read_head(&mut client).starts_with("HTTP/1.1 101"));
        clients.push(client);
    }
    let mut excess = dev_client(dev.addr, &raw("/app/_brain/socket"));
    assert!(read_head(&mut excess).starts_with("HTTP/1.1 503"));
    let mut outside = dev_client(dev.addr, &raw("/_brain/socket"));
    assert!(read_head(&mut outside).starts_with("HTTP/1.1 404"));
    let mut static_file = dev_client(
        dev.addr,
        b"GET /app/@brain/client.js HTTP/1.1\r\nHost: local\r\n\r\n",
    );
    assert!(read_head(&mut static_file).starts_with("HTTP/1.1 200"));
    let mut http = dev_client(
        dev.addr,
        b"GET /app/api/authority HTTP/1.1\r\nHost: local\r\n\r\n",
    );
    assert!(read_head(&mut http).starts_with("HTTP/1.1 200"));
    let mut hmr = dev_client(dev.addr, &raw("/app/@brain/hmr"));
    assert!(read_head(&mut hmr).starts_with("HTTP/1.1 101"));
    assert!(dev.load("/").unwrap().contains("kept"));
    assert_eq!(backend.http.load(Ordering::SeqCst), 1);
    assert_eq!(
        backend.auth.load(Ordering::SeqCst),
        0,
        "anonymous loads must avoid auth requests"
    );
    assert_eq!(
        wrong.accept().err().unwrap().kind(),
        io::ErrorKind::WouldBlock,
        "explicit authority must win for HTTP, SSR and sockets"
    );
    let log = dev.logs(0, None);
    assert!(log.contains("BNATIVE_SOCKET_CAPACITY"));
    assert!(!log.contains("private-refresh"));
    let start = Instant::now();
    dev.stop();
    assert!(start.elapsed() < Duration::from_secs(2));
    for client in &mut clients {
        let mut byte = [0u8; 1];
        assert!(matches!(client.read(&mut byte), Ok(0) | Err(_)));
    }
    let mut hmr_tail = Vec::new();
    hmr.read_to_end(&mut hmr_tail).unwrap();
    assert!(!hmr_tail.is_empty());
    drop(dev);
}
#[test]
fn explicit_native_authority_preserves_process_environment_without_returning_its_values() {
    assert!(
        std::env::var_os("PATH").is_some(),
        "the toolchain fixture has PATH"
    );
    let backend = HeldBackend::new();
    let root = Temp::new();
    let mut dev=crate::start_dev_with_native_proxy(dev_options(&root,None),compile_loader("export async function load(ctx) { if (!ctx.env.PATH) throw new Error('Process environment was discarded'); return { ok:true } }"),false,None,Some(NativeProxy::new(&format!("http://{}",backend.address)).unwrap())).unwrap();
    assert!(dev.load("/").unwrap().contains("true"));
    assert_eq!(
        backend.auth.load(Ordering::SeqCst),
        0,
        "anonymous loads must avoid auth requests"
    );
    let mut disabled = dev_client(dev.addr, &raw("/app/_brain/socket"));
    assert!(read_head(&mut disabled).starts_with("HTTP/1.1 404"));
    dev.stop();
    drop(dev);
}

#[test]
fn explicit_socket_origin_selection_fails_locally_without_widening_backend_policy() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = NativeProxy::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    assert!(config.clone().socket_origins(vec!["*".into()]).is_err());
    assert!(
        config
            .clone()
            .socket_origins(vec!["https://allowed.example.test".into(); 2])
            .is_err()
    );
    let proxy = config
        .sockets(true)
        .socket_origins(vec!["https://allowed.example.test".into()])
        .unwrap()
        .start_sockets()
        .unwrap()
        .unwrap();
    let mut client = client(&proxy, &raw("/_brain/socket"), "");
    assert!(read_head(&mut client).starts_with("HTTP/1.1 403"));
    let diagnostics = proxy.drain_diagnostics();
    assert_eq!(diagnostics[0].phase, "origin");
    assert!(!diagnostics[0].to_json().contains("https://allowed"));
    assert_eq!(
        listener.accept().err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    proxy.stop().unwrap();
}

#[test]
fn static_gateway_preserves_http_base_and_reaps_socket_tunnels_on_stop_or_last_drop() {
    let backend = HeldBackend::new();
    for explicit_stop in [true, false] {
        let root = Temp::new();
        std::fs::create_dir_all(root.0.join("_server")).unwrap();
        std::fs::write(root.0.join("index.html"), "static gateway page").unwrap();
        std::fs::write(root.0.join("_server/manifest.json"), r#"{"v":1,"nativeSockets":true,"publicOrigin":"https://frontend.example.test","routes":[],"rpc":{}}"#).unwrap();
        let gateway = std::sync::Arc::new(
            crate::prod::StaticGateway::load(
                &root.0,
                "/app",
                Some(&format!("http://{}", backend.address)),
            )
            .unwrap(),
        );
        let request = |bytes: &[u8]| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let mut client = dev_client(listener.local_addr().unwrap(), bytes);
            let copy = gateway.clone();
            let task =
                std::thread::spawn(move || copy.handle(listener.accept().unwrap().0).unwrap());
            let head = read_head(&mut client);
            task.join().unwrap();
            (head, client)
        };
        assert!(
            request(b"GET /app/ HTTP/1.1\r\nHost: local\r\n\r\n")
                .0
                .starts_with("HTTP/1.1 200")
        );
        assert!(
            request(b"GET /app/api/authority HTTP/1.1\r\nHost: local\r\n\r\n")
                .0
                .starts_with("HTTP/1.1 200")
        );
        assert!(
            request(&raw("/_brain/socket"))
                .0
                .starts_with("HTTP/1.1 404")
        );
        let (head, mut client) = request(&raw("/app/_brain/socket"));
        assert!(head.starts_with("HTTP/1.1 101"));
        let start = Instant::now();
        if explicit_stop {
            gateway.stop().unwrap();
        }
        drop(gateway);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(matches!(client.read(&mut [0u8; 1]), Ok(0) | Err(_)));
    }
    assert_eq!(backend.http.load(Ordering::SeqCst), 2);
}

#[test]
fn static_gateway_manifest_is_opt_in_bounded_and_fail_closed() {
    let root = Temp::new();
    std::fs::create_dir_all(root.0.join("_server")).unwrap();
    let manifest = root.0.join("_server/manifest.json");
    assert!(crate::prod::StaticGateway::load(&root.0, "/", None).is_ok());
    for input in [
        "not json",
        r#"{"v":1,"nativeSockets":false,"nativeSockets":true,"publicOrigin":"https://frontend.example.test"}"#,
        r#"{"nativeSockets":"true"}"#,
        r#"{"v":2,"nativeSockets":true,"publicOrigin":"https://frontend.example.test"}"#,
        r#"{"v":1,"nativeSockets":true,"publicOrigin":"https://frontend.example.test/path"}"#,
        r#"{"v":1,"nativeSockets":true}"#,
    ] {
        std::fs::write(&manifest, input).unwrap();
        assert!(
            crate::prod::StaticGateway::load(&root.0, "/", Some("http://127.0.0.1:9")).is_err()
        );
    }
    std::fs::write(&manifest, " ".repeat(65537)).unwrap();
    assert!(crate::prod::StaticGateway::load(&root.0, "/", None).is_err());
    std::fs::write(
        &manifest,
        r#"{"v":1,"nativeSockets":true,"publicOrigin":"https://frontend.example.test"}"#,
    )
    .unwrap();
    assert!(crate::prod::StaticGateway::load(&root.0, "/", None).is_err());
    std::fs::write(&manifest, r#"{"v":1,"nativeSockets":false}"#).unwrap();
    let app = crate::prod::StaticGateway::load(&root.0, "/", Some("http://127.0.0.1:9")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = dev_client(listener.local_addr().unwrap(), &raw("/_brain/socket"));
    let task = std::thread::spawn(move || app.handle(listener.accept().unwrap().0).unwrap());
    assert!(read_head(&mut client).starts_with("HTTP/1.1 404"));
    task.join().unwrap();
}
