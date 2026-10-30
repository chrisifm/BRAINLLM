//! Opaque, bounded native Upgrade transport. Rust backend remains the authentication/frame authority.
use crate::{
    http,
    server::native_proxy::{self, Target},
    ws,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    net::TcpStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc as sync,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
    task::JoinSet,
};

const CONNECTIONS: usize = 32;
const HEADERS: usize = 32768;
const HEAD: usize = 8192;
const BUFFER: usize = 16384;
const HANDSHAKE: Duration = Duration::from_secs(5);
const WRITE: Duration = Duration::from_secs(2);
const LIFETIME: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug)]
pub struct SocketProxyDiagnostic {
    pub rid: String,
    pub phase: &'static str,
    pub code: &'static str,
    pub fix: &'static str,
    pub status: u16,
    pub file: &'static str,
    pub line: u32,
}
impl SocketProxyDiagnostic {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schemaVersion\":1,\"event\":\"native-socket-proxy-error\",\"rid\":{},\"phase\":{},\"code\":{},\"fix\":{},\"status\":{},\"location\":{{\"file\":{},\"line\":{}}},\"valuesCaptured\":false}}",
            crate::json_str(&self.rid),
            crate::json_str(self.phase),
            crate::json_str(self.code),
            crate::json_str(self.fix),
            self.status,
            crate::json_str(self.file),
            self.line
        )
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SocketProxyStats {
    pub active: usize,
    pub high_water: usize,
    pub accepted: u64,
    pub rejected: u64,
    pub errors: u64,
    pub omitted_diagnostics: u64,
}
struct State {
    slots: Arc<Semaphore>,
    active: AtomicUsize,
    high: AtomicUsize,
    accepted: AtomicU64,
    rejected: AtomicU64,
    errors: AtomicU64,
    omitted: AtomicU64,
    diagnostics: Mutex<VecDeque<SocketProxyDiagnostic>>,
}
impl State {
    fn report(&self, rid: &str, failure: Failure) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        let mut ring = self
            .diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ring.len() == 64 {
            ring.pop_front();
            self.omitted.fetch_add(1, Ordering::Relaxed);
        }
        ring.push_back(SocketProxyDiagnostic {
            rid: rid.into(),
            phase: failure.phase,
            code: failure.code,
            fix: failure.fix,
            status: failure.status,
            file: failure.file,
            line: failure.line,
        });
    }
}
struct Slot {
    state: Arc<State>,
    _permit: OwnedSemaphorePermit,
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.state.active.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone, Copy)]
struct Failure {
    status: u16,
    phase: &'static str,
    code: &'static str,
    fix: &'static str,
    file: &'static str,
    line: u32,
}
impl Failure {
    #[track_caller]
    fn new(status: u16, phase: &'static str, code: &'static str, fix: &'static str) -> Self {
        let at = std::panic::Location::caller();
        Self {
            status,
            phase,
            code,
            fix,
            file: at.file(),
            line: at.line(),
        }
    }
    fn body(self, rid: &str) -> String {
        format!(
            "{{\"schemaVersion\":1,\"rid\":{},\"phase\":{},\"error\":{{\"code\":{},\"fix\":{}}}}}",
            crate::json_str(rid),
            crate::json_str(self.phase),
            crate::json_str(self.code),
            crate::json_str(self.fix)
        )
    }
}
struct Request {
    origin: String,
    cookie: String,
    bearer: Option<String>,
    key: String,
    rid: String,
    head: Vec<u8>,
}
fn one<'a>(req: &'a http::Request, name: &str) -> Result<Option<&'a str>, Failure> {
    let mut values = req
        .headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name));
    let value = values.next().map(|(_, value)| value.as_str());
    if values.next().is_some() {
        return Err(Failure::new(
            400,
            "request",
            "BNATIVE_SOCKET_HEADER",
            "Send each native socket handshake header once",
        ));
    }
    if value.is_some_and(|value| value.len() > 16384 || value.chars().any(char::is_control)) {
        return Err(Failure::new(
            431,
            "request",
            "BNATIVE_SOCKET_HEADER",
            "Keep native handshake headers within their byte budget",
        ));
    }
    Ok(value)
}
pub(crate) fn valid_origin(origin: &str) -> bool {
    if origin.len() > 2048 {
        return false;
    }
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path() == "/"
        && url.origin().ascii_serialization() == origin
        && !origin.contains('*')
}
fn validate(req: &http::Request) -> Result<Request, Failure> {
    let bad = || {
        Failure::new(
            400,
            "request",
            "BNATIVE_SOCKET_HANDSHAKE",
            "Use GET HTTP/1.1, WebSocket version 13 and the single brain.v1 subprotocol without a body",
        )
    };
    if req.method != "GET"
        || req.path != "/_brain/socket"
        || req.upgrade.version != "HTTP/1.1"
        || !req.body.is_empty()
    {
        return Err(bad());
    }
    if req.upgrade.query_present || !req.query.is_empty() {
        return Err(Failure::new(
            403,
            "request",
            "BNATIVE_SOCKET_QUERY",
            "Omit all socket query parameters; credentials belong only in native headers",
        ));
    }
    if one(req, "upgrade")?.is_none_or(|v| !v.eq_ignore_ascii_case("websocket"))
        || one(req, "connection")?.is_none_or(|v| {
            !v.split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("upgrade"))
        })
        || one(req, "sec-websocket-version")? != Some("13")
        || one(req, "sec-websocket-protocol")? != Some("brain.v1")
        || one(req, "transfer-encoding")?.is_some()
        || one(req, "content-length")?.is_some_and(|v| v != "0")
    {
        return Err(bad());
    }
    let key = one(req, "sec-websocket-key")?.ok_or_else(bad)?;
    let mut decoded = [0u8; 18];
    if key.len() != 24 || STANDARD.decode_slice(key, &mut decoded).ok() != Some(16) {
        return Err(bad());
    }
    let origin = one(req, "origin")?.ok_or_else(|| {
        Failure::new(
            403,
            "request",
            "BNATIVE_SOCKET_ORIGIN",
            "Send one canonical frontend Origin allowed by the native backend",
        )
    })?;
    if !valid_origin(origin) {
        return Err(bad());
    }
    let raw_cookie = one(req, "cookie")?.unwrap_or("");
    let bearer = one(req, "authorization")?;
    if raw_cookie.len() + bearer.map_or(0, str::len) > 16384 {
        return Err(Failure::new(
            431,
            "request",
            "BNATIVE_SOCKET_CREDENTIAL_BUDGET",
            "Keep native credentials within 16 KiB combined",
        ));
    }
    let cookie = native_proxy::socket_cookie(raw_cookie).map_err(|_| bad())?;
    if bearer.is_some_and(|v| {
        !v.starts_with("Bearer ")
            || v.len() > 8192
            || v.as_bytes().iter().any(|b| *b <= 32 && *b != b' ')
    }) {
        return Err(bad());
    }
    if req.upgrade.head.len() > HEAD {
        return Err(bad());
    }
    Ok(Request {
        origin: origin.into(),
        cookie,
        bearer: bearer.map(str::to_owned),
        key: key.into(),
        rid: req.id.clone(),
        head: Vec::new(),
    })
}
struct Job {
    admitted: Instant,
    stream: TcpStream,
    request: Request,
    _slot: Slot,
}
struct Worker {
    done: sync::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}
struct Inner {
    target: Target,
    origins: Option<Vec<String>>,
    state: Arc<State>,
    send: mpsc::Sender<Job>,
    stop: watch::Sender<bool>,
    worker: Mutex<Worker>,
}
/// Owned opaque transport. Construct with NativeProxy::sockets(true); stop it with its Dev owner.
#[derive(Clone)]
pub struct NativeSocketProxy(Arc<Inner>);
impl NativeSocketProxy {
    pub(crate) fn start(target: Target, origins: Option<Vec<String>>) -> io::Result<Self> {
        let state = Arc::new(State {
            slots: Arc::new(Semaphore::new(CONNECTIONS)),
            active: AtomicUsize::new(0),
            high: AtomicUsize::new(0),
            accepted: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            omitted: AtomicU64::new(0),
            diagnostics: Mutex::new(VecDeque::new()),
        });
        let (send, mut receive) = mpsc::channel::<Job>(CONNECTIONS);
        let (stop, mut stopped) = watch::channel(false);
        let (ready_tx, ready) = sync::sync_channel(1);
        let (done_tx, done) = sync::sync_channel(1);
        let target_thread = target.clone();
        let state_thread = state.clone();
        let thread=std::thread::Builder::new().name("brain-native-sockets".into()).spawn(move || {
            let runtime=match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime)=>runtime,
                Err(_)=>{let _=ready_tx.send(false);let _=done_tx.send(());return;}
            };
            let _=ready_tx.send(true);
            runtime.block_on(async move {
                let mut tasks=JoinSet::new();
                loop {
                    tokio::select! {
                        biased;
                        _=stopped.changed()=>break,
                        // Reap completed tasks before admitting more; a continuous request stream
                        // must not retain an unbounded JoinSet of already-completed handles.
                        result=tasks.join_next(),if !tasks.is_empty()=>{
                            if result.is_some_and(|result|result.is_err_and(|error|error.is_panic())) {
                                state_thread.report("unavailable",Failure::new(503,"worker","BNATIVE_SOCKET_TASK_FAILED","Inspect the owned transport worker and restart the dev server"));
                            }
                        },
                        job=receive.recv(),if tasks.len()<CONNECTIONS=>match job {Some(job)=>{tasks.spawn(run(target_thread.clone(),job,stopped.clone()));},None=>break},
                    }
                }
                receive.close();
                while receive.try_recv().is_ok() {}
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
            });
            drop(runtime);
            let _=done_tx.send(());
        })?;
        if ready.recv_timeout(Duration::from_secs(3)) != Ok(true) {
            stop.send_replace(true);
            let _ = thread.join();
            return Err(io::Error::other("Native socket runtime could not start"));
        }
        Ok(Self(Arc::new(Inner {
            target,
            origins,
            state,
            send,
            stop,
            worker: Mutex::new(Worker {
                done,
                thread: Some(thread),
            }),
        })))
    }
    pub(crate) fn takeover(
        &self,
        stream: &mut TcpStream,
        req: &mut http::Request,
    ) -> io::Result<()> {
        let result = (|| {
            if *self.0.stop.borrow() {
                return Err(Failure::new(
                    503,
                    "admission",
                    "BNATIVE_SOCKET_STOPPED",
                    "Restart the owned dev server before reconnecting",
                ));
            }
            let mut request = validate(req)?;
            if self
                .0
                .origins
                .as_ref()
                .is_some_and(|origins| !origins.contains(&request.origin))
            {
                return Err(Failure::new(
                    403,
                    "origin",
                    "BNATIVE_SOCKET_ORIGIN",
                    "Open the configured site/port or review backend.sockets.allowedOrigins; never widen CORS automatically",
                ));
            }
            if stream
                .local_addr()
                .ok()
                .is_some_and(|local| local.port() == self.0.target.address().port())
            {
                return Err(Failure::new(
                    503,
                    "admission",
                    "BNATIVE_SOCKET_SELF_TARGET",
                    "Configure a separate loopback native backend port",
                ));
            }
            let permit = self
                .0
                .state
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| {
                    Failure::new(
                        503,
                        "admission",
                        "BNATIVE_SOCKET_CAPACITY",
                        "Close unused native sockets or retry with bounded backoff",
                    )
                })?;
            let count = self.0.state.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.0.state.high.fetch_max(count, Ordering::Relaxed);
            let slot = Slot {
                state: self.0.state.clone(),
                _permit: permit,
            };
            let socket = stream.try_clone().map_err(|_| {
                Failure::new(
                    503,
                    "admission",
                    "BNATIVE_SOCKET_TRANSPORT",
                    "Retry the native connection after checking the local transport",
                )
            })?;
            request.head = std::mem::take(&mut req.upgrade.head);
            self.0
                .send
                .try_send(Job {
                    admitted: Instant::now(),
                    stream: socket,
                    request,
                    _slot: slot,
                })
                .map_err(|_| {
                    Failure::new(
                        503,
                        "admission",
                        "BNATIVE_SOCKET_STOPPED",
                        "Restart the owned dev server before reconnecting",
                    )
                })?;
            self.0.state.accepted.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })();
        if let Err(failure) = result {
            self.0.state.rejected.fetch_add(1, Ordering::Relaxed);
            self.0.state.report(&req.id, failure);
            stream.set_write_timeout(Some(Duration::from_secs(1)))?;
            let body = failure.body(&req.id);
            return http::respond_with(
                stream,
                failure.status,
                "application/json",
                body.as_bytes(),
                false,
                &[("X-Request-Id", &req.id)],
            );
        }
        Ok(())
    }
    pub fn stats(&self) -> SocketProxyStats {
        let state = &self.0.state;
        SocketProxyStats {
            active: state.active.load(Ordering::Acquire),
            high_water: state.high.load(Ordering::Relaxed),
            accepted: state.accepted.load(Ordering::Relaxed),
            rejected: state.rejected.load(Ordering::Relaxed),
            errors: state.errors.load(Ordering::Relaxed),
            omitted_diagnostics: state.omitted.load(Ordering::Relaxed),
        }
    }
    pub fn drain_diagnostics(&self) -> Vec<SocketProxyDiagnostic> {
        self.0
            .state
            .diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect()
    }
    pub fn stop(&self) -> io::Result<()> {
        self.0.stop()
    }
}
impl Inner {
    fn stop(&self) -> io::Result<()> {
        self.stop.send_replace(true);
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if worker.thread.is_none() {
            return Ok(());
        }
        worker
            .done
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Native socket worker did not stop within three seconds",
                )
            })?;
        worker
            .thread
            .take()
            .unwrap()
            .join()
            .map_err(|_| io::Error::other("Native socket worker failed while stopping"))
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        // No user callbacks or blocking I/O run on this worker. Always reap its already-cancelled I/O tasks.
        let worker = self
            .worker
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(thread) = worker.thread.take() {
            let _ = thread.join();
        }
    }
}
async fn write_failure(stream: &mut tokio::net::TcpStream, rid: &str, failure: Failure) {
    let body = failure.body(rid);
    let response = format!(
        "HTTP/1.1 {} Native socket unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nX-Request-Id: {}\r\n\r\n{body}",
        failure.status,
        body.len(),
        rid
    );
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        stream.write_all(response.as_bytes()),
    )
    .await;
}
fn response_head(bytes: &[u8], key: &str) -> Result<u16, Failure> {
    let bad = || {
        Failure::new(
            502,
            "handshake",
            "BNATIVE_SOCKET_UPSTREAM_HANDSHAKE",
            "Use a native backend that accepts brain.v1 with a valid WebSocket handshake",
        )
    };
    let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
    let mut lines = text.split("\r\n");
    let status = lines.next().ok_or_else(bad)?;
    let parts = status.splitn(3, ' ').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "HTTP/1.1"
        || parts[1].len() != 3
        || !parts[1].bytes().all(|b| b.is_ascii_digit())
    {
        return Err(bad());
    }
    let status = parts[1].parse::<u16>().map_err(|_| bad())?;
    let mut headers = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.len() > 8192 || headers.len() >= 128 {
            return Err(bad());
        }
        let (name, value) = line.split_once(':').ok_or_else(bad)?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.chars().any(char::is_control)
        {
            return Err(bad());
        }
        if headers
            .insert(name.to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return Err(bad());
        }
    }
    if status != 101 {
        if !(400..=599).contains(&status) {
            return Err(bad());
        }
        return Ok(status);
    }
    if headers
        .get("upgrade")
        .is_none_or(|value| !value.eq_ignore_ascii_case("websocket"))
        || headers.get("connection").is_none_or(|value| {
            !value
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("upgrade"))
        })
        || headers.get("sec-websocket-accept").copied() != Some(ws::accept_key(key).as_str())
        || headers.get("sec-websocket-protocol").copied() != Some("brain.v1")
        || headers.contains_key("transfer-encoding")
        || headers.contains_key("sec-websocket-extensions")
        || headers
            .get("content-length")
            .is_some_and(|value| *value != "0")
    {
        return Err(bad());
    }
    Ok(status)
}
async fn handshake(
    target: &Target,
    client: &mut tokio::net::TcpStream,
    request: &Request,
    committed: &mut bool,
) -> Result<tokio::net::TcpStream, Failure> {
    let bad_io = || {
        Failure::new(
            502,
            "handshake",
            "BNATIVE_SOCKET_UPSTREAM_IO",
            "Check the native backend and reconnect with bounded backoff",
        )
    };
    let mut upstream = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(target.address()),
    )
    .await
    .map_err(|_| bad_io())?
    .map_err(|_| bad_io())?;
    upstream.set_nodelay(true).map_err(|_| bad_io())?;
    let mut head = format!(
        "GET /_brain/socket HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: brain.v1\r\nSec-WebSocket-Key: {}\r\nOrigin: {}\r\nX-Request-Id: {}\r\n",
        target.host(),
        request.key,
        request.origin,
        request.rid
    );
    if !request.cookie.is_empty() {
        head.push_str(&format!("Cookie: {}\r\n", request.cookie));
    }
    if let Some(bearer) = &request.bearer {
        head.push_str(&format!("Authorization: {bearer}\r\n"));
    }
    head.push_str("\r\n");
    tokio::time::timeout(WRITE, upstream.write_all(head.as_bytes()))
        .await
        .map_err(|_| bad_io())?
        .map_err(|_| bad_io())?;
    let mut response = Vec::new();
    response
        .try_reserve_exact(HEADERS + HEAD)
        .map_err(|_| bad_io())?;
    let header_end;
    loop {
        let mut chunk = [0u8; HEAD];
        let n = upstream.read(&mut chunk).await.map_err(|_| bad_io())?;
        if n == 0 {
            return Err(bad_io());
        }
        let scan = response.len().saturating_sub(3);
        response.extend_from_slice(&chunk[..n]);
        if let Some(index) = response[scan..]
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
        {
            header_end = scan + index + 4;
            if header_end > HEADERS {
                return Err(bad_io());
            }
            break;
        }
        if response.len() > HEADERS {
            return Err(bad_io());
        }
    }
    let status = response_head(&response[..header_end], &request.key)?;
    if status != 101 {
        return Err(Failure::new(
            status,
            "handshake",
            "BNATIVE_SOCKET_UPSTREAM_REJECTED",
            "Check the native session, configured origins and socket opt-in; resynchronize after reconnecting",
        ));
    }
    let outgoing = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: brain.v1\r\nX-Request-Id: {}\r\n\r\n",
        ws::accept_key(&request.key),
        request.rid
    );
    // After any 101 byte reaches the browser an HTTP error must never be appended to its opaque stream.
    *committed = true;
    tokio::time::timeout(WRITE, client.write_all(outgoing.as_bytes()))
        .await
        .map_err(|_| bad_io())?
        .map_err(|_| bad_io())?;
    tokio::time::timeout(WRITE, client.write_all(&response[header_end..]))
        .await
        .map_err(|_| bad_io())?
        .map_err(|_| bad_io())?;
    tokio::time::timeout(WRITE, upstream.write_all(&request.head))
        .await
        .map_err(|_| bad_io())?
        .map_err(|_| bad_io())?;
    Ok(upstream)
}
async fn pump<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut read: R,
    mut write: W,
) -> io::Result<()> {
    let mut bytes = [0u8; BUFFER];
    loop {
        let n = read.read(&mut bytes).await?;
        if n == 0 {
            return tokio::time::timeout(WRITE, write.shutdown())
                .await
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Native socket shutdown write stalled",
                    )
                })?;
        }
        tokio::time::timeout(WRITE, write.write_all(&bytes[..n]))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "Native socket write stalled")
            })??;
    }
}
async fn run(target: Target, job: Job, mut stop: watch::Receiver<bool>) {
    let Job {
        admitted,
        stream,
        request,
        _slot,
    } = job;
    let state = _slot.state.clone();
    if *stop.borrow() {
        return;
    }
    if stream.set_nonblocking(true).is_err() {
        state.report(
            &request.rid,
            Failure::new(
                502,
                "admission",
                "BNATIVE_SOCKET_TRANSPORT",
                "Retry after checking local socket transport",
            ),
        );
        return;
    }
    let Ok(mut client) = tokio::net::TcpStream::from_std(stream) else {
        state.report(
            &request.rid,
            Failure::new(
                502,
                "admission",
                "BNATIVE_SOCKET_TRANSPORT",
                "Retry after checking local socket transport",
            ),
        );
        return;
    };
    if client.set_nodelay(true).is_err() {
        state.report(
            &request.rid,
            Failure::new(
                502,
                "admission",
                "BNATIVE_SOCKET_TRANSPORT",
                "Retry after checking local socket transport",
            ),
        );
        return;
    }
    if admitted.elapsed() >= HANDSHAKE {
        let failure = Failure::new(
            504,
            "admission",
            "BNATIVE_SOCKET_QUEUE_TIMEOUT",
            "Retry with bounded backoff after current native handshakes finish",
        );
        state.report(&request.rid, failure);
        write_failure(&mut client, &request.rid, failure).await;
        return;
    }
    let mut committed = false;
    let exchange = async {
        let mut upstream = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(admitted + HANDSHAKE),
            handshake(&target, &mut client, &request, &mut committed),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            result => {
                let failure = match result {
                    Ok(Err(failure)) => failure,
                    _ => Failure::new(
                        504,
                        "handshake",
                        "BNATIVE_SOCKET_HANDSHAKE_TIMEOUT",
                        "Check native backend availability and retry with bounded backoff",
                    ),
                };
                state.report(&request.rid, failure);
                if !committed {
                    write_failure(&mut client, &request.rid, failure).await;
                }
                return;
            }
        };
        let (client_read, client_write) = tokio::io::split(&mut client);
        let (upstream_read, upstream_write) = tokio::io::split(&mut upstream);
        if let Err(error) = tokio::try_join!(
            pump(client_read, upstream_write),
            pump(upstream_read, client_write)
        ) {
            let failure = if error.kind() == io::ErrorKind::TimedOut {
                Failure::new(
                    504,
                    "write",
                    "BNATIVE_SOCKET_WRITE_TIMEOUT",
                    "Check the stalled peer, reconnect with bounded backoff and resynchronize HTTP state",
                )
            } else {
                Failure::new(
                    502,
                    "tunnel",
                    "BNATIVE_SOCKET_TRANSPORT",
                    "Reconnect with bounded backoff and resynchronize authorized HTTP state",
                )
            };
            state.report(&request.rid, failure);
        }
    };
    tokio::select! {biased;_=stop.changed()=>{},_=tokio::time::sleep_until(tokio::time::Instant::from_std(admitted + LIFETIME))=>{state.report(&request.rid,Failure::new(504,"lifetime","BNATIVE_SOCKET_LIFETIME","Reconnect and resynchronize after the finite transport lifetime"));},_=exchange=>{}}
}

#[cfg(test)]
#[path = "native_socket_proxy_tests.rs"]
mod tests;
