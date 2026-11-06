//! One explicit loopback backend authority. No arbitrary targets, redirects or credentials. The only forwarded identity
//! is `x-forwarded-for`: prior hops plus the peer this gateway observed, which the backend walks right to left through
//! `platform.trustProxy`, so a client's own header never chooses the address its rate limit is keyed on.
use crate::http;
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static BUFFERED: AtomicUsize = AtomicUsize::new(0);
static STREAMS: AtomicUsize = AtomicUsize::new(0);
const MAX_STREAMS: usize = 32;
/// Above the largest bucket `maxBytes` plus multipart framing; each bucket enforces its own limit.
const MAX_FILE_BODY: usize = 2 * 1024 * 1024 * 1024 + 1024 * 1024;
/// Backend limit of a batch and of a route `bodyLimit`.
const MAX_DECLARED_BODY: usize = 1024 * 1024;
const FILE_IDLE: Duration = Duration::from_secs(60);
const FILE_UPLOAD_TOTAL: Duration = Duration::from_secs(310);
const MAX_RESPONSE: usize = 20 * 1024 * 1024;
const GLOBAL_RESPONSE: usize = 64 * 1024 * 1024;
#[derive(Clone)]
pub(crate) struct Target {
    address: SocketAddr,
    host: String,
}
impl Target {
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }
    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        if value.is_empty()
            || value.len() > 2048
            || value.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err("BRAIN_BACKEND_URL must be a loopback HTTP origin".into());
        }
        let authority = value
            .strip_prefix("http://")
            .ok_or("BRAIN_BACKEND_URL must be a loopback HTTP origin")?
            .strip_suffix('/')
            .unwrap_or(value.strip_prefix("http://").unwrap());
        if authority.contains(['/', '?', '#', '@', '\\', '%']) {
            return Err(
                "BRAIN_BACKEND_URL must be a loopback HTTP origin without credentials or path"
                    .into(),
            );
        }
        let (ip, port) = if let Some(port) = authority.strip_prefix("[::1]") {
            (IpAddr::V6(Ipv6Addr::LOCALHOST), port)
        } else if let Some(port) = authority.strip_prefix("127.0.0.1") {
            (IpAddr::V4(Ipv4Addr::LOCALHOST), port)
        } else if let Some(port) = authority.strip_prefix("localhost") {
            (IpAddr::V4(Ipv4Addr::LOCALHOST), port)
        } else {
            return Err("BRAIN_BACKEND_URL must use a loopback host".into());
        };
        let port = if port.is_empty() {
            80
        } else {
            port.strip_prefix(':')
                .and_then(|p| p.parse::<u16>().ok())
                .filter(|p| *p > 0)
                .ok_or("BRAIN_BACKEND_URL has an invalid port")?
        };
        let address = SocketAddr::new(ip, port);
        Ok(Self {
            address,
            host: address.to_string(),
        })
    }
}
/// One `server/routes/*` contract the build listed: only these exact shapes cross the bridge.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct NativeRoute {
    pub path: String,
    #[serde(default)]
    pub headers: Vec<String>,
}
#[derive(Clone, Default)]
pub(crate) struct NativeRoutes(Vec<NativeRoute>);
impl NativeRoutes {
    pub(crate) fn new(routes: Vec<NativeRoute>) -> Self {
        Self(routes)
    }
    pub(crate) fn find(&self, path: &str) -> Option<&NativeRoute> {
        self.0.iter().find(|route| {
            let (mut shape, mut parts) = (route.path.split('/'), path.split('/'));
            loop {
                match (shape.next(), parts.next()) {
                    (None, None) => return true,
                    (Some(want), Some(part)) => {
                        let param = want.len() > 2 && want.starts_with('{') && want.ends_with('}');
                        if !(if param { !part.is_empty() } else { want == part }) {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }
        })
    }
}
/// Lists the declared routes of a live project (dev server); a build writes them to the manifest instead.
pub type RouteLoader = std::sync::Arc<dyn Fn() -> Vec<NativeRoute> + Send + Sync>;
/// Fixed native authority for an explicit gateway; sockets remain opt-in.
#[derive(Clone)]
pub struct NativeProxy {
    target: Target,
    sockets: bool,
    origins: Option<Vec<String>>,
    routes: Option<RouteLoader>,
}
impl NativeProxy {
    pub fn new(url: &str) -> Result<Self, String> {
        Ok(Self {
            target: Target::parse(url)?,
            sockets: false,
            origins: None,
            routes: None,
        })
    }
    pub fn routes(mut self, loader: RouteLoader) -> Self {
        self.routes = Some(loader);
        self
    }
    pub(crate) fn route_loader(&self) -> Option<RouteLoader> {
        self.routes.clone()
    }
    pub fn sockets(mut self, enabled: bool) -> Self {
        self.sockets = enabled;
        self
    }
    pub fn socket_origins(mut self, origins: Vec<String>) -> Result<Self, String> {
        if origins.is_empty()
            || origins.len() > 8
            || origins
                .iter()
                .any(|origin| !crate::native_socket_proxy::valid_origin(origin))
            || origins
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != origins.len()
        {
            return Err(
                "B0744: native sockets require 1..8 distinct canonical HTTP(S) frontend origins; fix: brain.json backend.sockets.allowedOrigins. Run brain explain B0744".into(),
            );
        }
        self.origins = Some(origins);
        Ok(self)
    }
    pub(crate) fn origin(&self) -> String {
        format!("http://{}", self.target.host())
    }
    pub fn start_sockets(&self) -> io::Result<Option<crate::NativeSocketProxy>> {
        if !self.sockets {
            return Ok(None);
        }
        crate::NativeSocketProxy::start(self.target.clone(), self.origins.clone()).map(Some)
    }
}

pub(crate) fn socket_cookie(header: &str) -> Result<String, &'static str> {
    cookie_for(header, &["__Host-brain", "__Host-brain-access"]).map_err(|_| "invalid-auth-cookie")
}

pub(crate) fn read_request(
    stream: &TcpStream,
    upload_path: Option<&str>,
    plan: &dyn Fn(&str, &str) -> http::BodyPlan,
) -> io::Result<http::Request> {
    http::read_request_planned(
        Deadline {
            socket: stream.try_clone()?,
            end: Instant::now() + Duration::from_secs(5),
        },
        upload_path,
        65536,
        None,
        plan,
    )
}
fn is_files(path: &str) -> bool {
    path == "/_brain/files" || path.starts_with("/_brain/files/")
}
fn is_batch(path: &str) -> bool {
    path.strip_prefix("/api/")
        .and_then(|rest| rest.strip_suffix("/batch"))
        .is_some_and(|name| !name.is_empty() && !name.contains('/'))
}
/// The body plan of a request bound for the backend; everything else keeps the gateway's default.
pub(crate) fn plan(path: &str, method: &str, routes: &NativeRoutes, pre: &str) -> http::BodyPlan {
    let Some(path) = (if pre.is_empty() { Some(path) } else { path.strip_prefix(pre).filter(|p| p.starts_with('/')) }) else {
        return http::BodyPlan::Default;
    };
    if is_files(path) && handles(path) {
        http::BodyPlan::Stream(MAX_FILE_BODY)
    } else if (is_batch(path) && method == "POST") || (!handles(path) && routes.find(path).is_some()) {
        http::BodyPlan::Limit(MAX_DECLARED_BODY)
    } else {
        http::BodyPlan::Default
    }
}
/// `$proxy_add_x_forwarded_for`: the hops received plus the observed peer. A chain over 4 KiB is dropped, not truncated.
pub(crate) fn forwarded_for(req: &http::Request, peer: Option<SocketAddr>) -> Option<String> {
    let peer = peer?.ip().to_canonical();
    match req.header("x-forwarded-for").map(str::trim) {
        Some(prior) if !prior.is_empty() && prior.len() <= 4096 && !prior.chars().any(char::is_control) => Some(format!("{prior}, {peer}")),
        _ => Some(peer.to_string()),
    }
}
/// The probes the backend owns (B0735 reserves them); a frontend answers them only where it would have answered 404.
pub(crate) fn is_probe(path: &str) -> bool {
    matches!(path, "/health" | "/health/live" | "/health/ready")
}
/// Framework aliases terminate at the gateway; the resource backend serves only the canonical probes.
pub(crate) fn probe_alias(path: &str) -> Option<&'static str> {
    match path {
        "/_brain/health" => Some("/health"),
        "/_brain/health/live" => Some("/health/live"),
        "/_brain/health/ready" => Some("/health/ready"),
        _ => None,
    }
}
pub(crate) fn handles(path: &str) -> bool {
    // the one script the backend serves; other `.js` paths are the dev server's modules
    path == "/_brain/analytics.js" || (path.starts_with("/s/")
        || path == "/api"
        || path.starts_with("/api/")
        || path == "/_brain"
        || path.starts_with("/_brain/"))
        && ![".brain", ".jsbrain", ".cssbrain", ".js", ".mjs"]
            .iter()
            .any(|suffix| path.ends_with(*suffix))
}
struct Permit;
impl Permit {
    fn acquire() -> Option<Self> {
        let mut count = ACTIVE.load(Ordering::Acquire);
        loop {
            if count >= 8 {
                return None;
            }
            match ACTIVE.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self),
                Err(now) => count = now,
            }
        }
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
struct StreamPermit;
impl StreamPermit {
    fn acquire() -> Option<Self> {
        STREAMS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| (count < MAX_STREAMS).then_some(count + 1))
            .ok()
            .map(|_| Self)
    }
}
impl Drop for StreamPermit {
    fn drop(&mut self) {
        STREAMS.fetch_sub(1, Ordering::AcqRel);
    }
}
struct BufferBudget(usize);
impl BufferBudget {
    fn grow(&mut self, bytes: usize) -> Result<(), Failure> {
        let mut used = BUFFERED.load(Ordering::Acquire);
        loop {
            if used.saturating_add(bytes) > GLOBAL_RESPONSE {
                return Err(Failure(503, "response-capacity"));
            }
            match BUFFERED.compare_exchange_weak(
                used,
                used + bytes,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.0 += bytes;
                    return Ok(());
                }
                Err(now) => used = now,
            }
        }
    }
}
impl Drop for BufferBudget {
    fn drop(&mut self) {
        BUFFERED.fetch_sub(self.0, Ordering::AcqRel);
    }
}
#[derive(Debug)]
struct Failure(u16, &'static str);
fn io_failure(error: io::Error) -> Failure {
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        Failure(504, "deadline")
    } else {
        Failure(502, "transport")
    }
}
struct Deadline {
    socket: TcpStream,
    end: Instant,
}
impl Read for Deadline {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let left = self.end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "native response deadline",
            ));
        }
        self.socket.set_read_timeout(Some(left))?;
        self.socket.read(out)
    }
}
impl Write for Deadline {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let left = self.end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "native write deadline",
            ));
        }
        self.socket.set_write_timeout(Some(left))?;
        self.socket.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}
fn line<R: Read>(reader: &mut BufReader<R>, max: usize) -> Result<String, Failure> {
    let mut bytes = Vec::new();
    reader
        .take(max as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(io_failure)?;
    if bytes.len() > max || !bytes.ends_with(b"\r\n") {
        return Err(Failure(502, "response-framing"));
    }
    bytes.truncate(bytes.len() - 2);
    String::from_utf8(bytes).map_err(|_| Failure(502, "response-header"))
}
fn cookie(header: &str) -> Result<String, Failure> {
    cookie_for(
        header,
        &[
            "__Host-brain",
            "__Host-brain-access",
            "__Host-brain-refresh",
            "__Host-brain-google",
            "__Host-brain-apple",
            "__Host-brain-linkedin",
        ],
    )
}
fn cookie_for(header: &str, allowed: &[&str]) -> Result<String, Failure> {
    if header.len() > 16384 {
        return Err(Failure(431, "cookie-budget"));
    }
    let mut values = std::collections::BTreeMap::new();
    for part in header.split(';') {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        if !allowed.contains(&name) {
            continue;
        }
        if value.len() > 8192
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || values.insert(name, value).is_some()
        {
            return Err(Failure(400, "invalid-auth-cookie"));
        }
    }
    Ok(values
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; "))
}
fn encode_path(path: &str) -> String {
    let mut out = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            out.push(byte as char)
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
/// Status and validated headers of a backend response.
fn response_head<R: Read>(reader: &mut BufReader<R>) -> Result<(u16, Vec<(String, String)>), Failure> {
    let status_line = line(reader, 8192)?;
    let mut status_parts = status_line.splitn(3, ' ');
    if !matches!(status_parts.next(), Some("HTTP/1.1" | "HTTP/1.0")) {
        return Err(Failure(502, "response-status"));
    }
    let status = status_parts
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|s| (200..=599).contains(s))
        .ok_or(Failure(502, "response-status"))?;
    let mut raw_headers: Vec<(String, String)> = Vec::new();
    let mut bytes = status_line.len();
    loop {
        let row = line(reader, 8192)?;
        bytes += row.len() + 2;
        if bytes > 32768 || raw_headers.len() >= 128 {
            return Err(Failure(502, "response-header-budget"));
        }
        if row.is_empty() {
            break;
        }
        let (name, value) = row.split_once(':').ok_or(Failure(502, "response-header"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.chars().any(char::is_control)
        {
            return Err(Failure(502, "response-header"));
        }
        let name = name.to_ascii_lowercase();
        if [
            "content-length",
            "transfer-encoding",
            "content-type",
            "content-encoding",
            "location",
        ]
        .contains(&name.as_str())
            && raw_headers.iter().any(|(old, _)| old == &name)
        {
            return Err(Failure(502, "response-duplicate-header"));
        }
        raw_headers.push((name, value.trim().into()));
    }
    Ok((status, raw_headers))
}
/// The request headers every proxied call carries.
fn upstream_headers(head: &mut String, req: &http::Request, names: &[&str]) -> Result<(), Failure> {
    let mut sent=std::collections::BTreeSet::new();
    for name in names {
        let canonical=name.to_ascii_lowercase();
        if !sent.insert(canonical.clone()) {continue;}
        // Deduplicate the allowlist, not the input: genuine duplicate values must remain detectable upstream.
        for (_,value) in req.headers.iter().filter(|(incoming,_)|incoming.eq_ignore_ascii_case(&canonical)) {
            if value.len() > 8192 || value.chars().any(char::is_control) {
                return Err(Failure(400, "request-header"));
            }
            head.push_str(&format!("{canonical}: {value}\r\n"));
        }
    }
    if req.path == "/_brain/vitals" { return Ok(()) }
    let auth = cookie(req.header("cookie").unwrap_or(""))?;
    if !auth.is_empty() {
        head.push_str(&format!("Cookie: {auth}\r\n"));
    }
    Ok(())
}
const REQUEST_HEADERS: [&str; 16] = [
    "content-type",
    "accept",
    "origin",
    "authorization",
    // A project API key: a non-ambient credential the backend verifies or refuses (docs/backend/api-keys.md).
    "x-api-key",
    "x-brain-captcha",
    "x-brain-login-grant",
    // Validators: without them optimistic locking and conditional reads are silently off (docs/backend/caching.md).
    "if-match",
    "if-none-match",
    "x-brain-webhook-id",
    "x-brain-webhook-timestamp",
    "x-brain-webhook-signature",
    // Behavior analytics filters (bots, Do Not Track, Global Privacy Control); never stored as such.
    "user-agent",
    "dnt",
    "sec-gpc",
    "cf-ipcountry",
];
const FILE_HEADERS: [&str; 4] = ["range", "if-range", "if-modified-since", "if-unmodified-since"];
const RESPONSE_HEADERS: [&str; 9] = [
    "location",
    "content-disposition",
    "retry-after",
    "www-authenticate",
    "clear-site-data",
    "allow",
    "etag",
    "last-modified",
    "cache-control",
];
const FILE_RESPONSE_HEADERS: [&str; 7] = [
    "content-range",
    "accept-ranges",
    "vary",
    "content-security-policy",
    "x-content-type-options",
    "cross-origin-resource-policy",
    "referrer-policy",
];
fn auth_cookie_reply(value: &str) -> bool {
    ["__Host-brain", "__Host-brain-access", "__Host-brain-refresh", "__Host-brain-google", "__Host-brain-apple", "__Host-brain-linkedin"].contains(&value.split('=').next().unwrap_or(""))
}
#[cfg(test)]
fn fetch(
    target: &Target,
    req: &http::Request,
) -> Result<
    (
        u16,
        String,
        Vec<(String, String)>,
        Vec<u8>,
        BufferBudget,
        Option<usize>,
        Instant,
    ),
    Failure,
> {
    fetch_with(target, req, None, None)
}
fn fetch_with(
    target: &Target,
    req: &http::Request,
    client: Option<&str>,
    route: Option<&NativeRoute>,
) -> Result<
    (
        u16,
        String,
        Vec<(String, String)>,
        Vec<u8>,
        BufferBudget,
        Option<usize>,
        Instant,
    ),
    Failure,
> {
    let _permit = Permit::acquire().ok_or(Failure(503, "request-capacity"))?;
    // Native image/GCS handlers have a thirty-second deadline; preserve their completed result through the edge.
    let resource_path = req.path.split('?').next().unwrap_or("");
    let vitals = resource_path == "/_brain/vitals";
    if vitals && req.body.len() > 4096 { return Err(Failure(413, "vitals-body-budget")) }
    let media = resource_path == "/_brain/uploads" || resource_path.starts_with("/_brain/uploads/");
    let end = Instant::now() + Duration::from_secs(if vitals { 5 } else if media || route.is_some() { 35 } else { 10 });
    let socket = TcpStream::connect_timeout(&target.address, Duration::from_secs(2))
        .map_err(|_| Failure(503, "backend-unavailable"))?;
    let mut stream = Deadline { socket, end };
    let path = encode_path(&req.path);
    let path = if req.query.is_empty() {
        path
    } else {
        format!("{path}?{}", req.query)
    };
    let request_id = if vitals { String::new() } else { format!("X-Request-Id: {}\r\n",req.id) };
    let mut headers = format!(
        "{} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept-Encoding: identity\r\n{request_id}Content-Length: {}\r\n",
        req.method,
        target.host,
        req.body.len()
    );
    if let Some(client) = client.filter(|_| !vitals) {
        headers.push_str(&format!("x-forwarded-for: {client}\r\n"));
    }
    let declared = route.map(|route| route.headers.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_or_default();
    if vitals { upstream_headers(&mut headers, req, &["origin", "content-type", "dnt", "sec-gpc"])?; }
    else { upstream_headers(&mut headers, req, &[&REQUEST_HEADERS[..], &declared[..]].concat())?; }
    headers.push_str("\r\n");
    stream
        .write_all(headers.as_bytes())
        .and_then(|_| stream.write_all(&req.body))
        .and_then(|_| stream.flush())
        .map_err(io_failure)?;
    let mut reader = BufReader::new(stream);
    let (status, raw_headers) = response_head(&mut reader)?;
    let get = |key: &str| {
        raw_headers
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    if get("content-encoding").is_some_and(|v| v != "identity") {
        return Err(Failure(502, "encoded-backend-response"));
    }
    let length = get("content-length")
        .map(|v| {
            v.parse::<usize>()
                .map_err(|_| Failure(502, "response-length"))
        })
        .transpose()?;
    if length.is_some_and(|n| n > MAX_RESPONSE) {
        return Err(Failure(502, "response-budget"));
    }
    if length.is_some() && get("transfer-encoding").is_some() {
        return Err(Failure(502, "response-framing"));
    }
    let chunked = match get("transfer-encoding") {
        None => false,
        Some(v) if v.eq_ignore_ascii_case("chunked") => true,
        _ => return Err(Failure(502, "response-encoding")),
    };
    let mut body = Vec::new();
    let mut budget = BufferBudget(0);
    let mut append = |chunk: &[u8]| -> Result<(), Failure> {
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err(Failure(502, "response-budget"));
        }
        budget.grow(chunk.len())?;
        body.try_reserve_exact(chunk.len())
            .map_err(|_| Failure(503, "allocation-budget"))?;
        body.extend_from_slice(chunk);
        Ok(())
    };
    if req.method != "HEAD" && status != 204 && status != 304 {
        if chunked {
            let mut count = 0usize;
            loop {
                count += 1;
                if count > 32768 {
                    return Err(Failure(502, "response-chunk-budget"));
                }
                let size = line(&mut reader, 128)?;
                let size = usize::from_str_radix(size.split(';').next().unwrap_or(""), 16)
                    .map_err(|_| Failure(502, "response-chunk"))?;
                if size == 0 {
                    if !line(&mut reader, 8192)?.is_empty() {
                        return Err(Failure(502, "response-trailers-unsupported"));
                    }
                    break;
                }
                if size > MAX_RESPONSE {
                    return Err(Failure(502, "response-budget"));
                }
                let mut left = size;
                let mut chunk = [0u8; 16384];
                while left > 0 {
                    let n = left.min(chunk.len());
                    reader.read_exact(&mut chunk[..n]).map_err(io_failure)?;
                    append(&chunk[..n])?;
                    left -= n;
                }
                let mut end = [0u8; 2];
                reader.read_exact(&mut end).map_err(io_failure)?;
                if end != *b"\r\n" {
                    return Err(Failure(502, "response-chunk"));
                }
            }
        } else {
            let mut left = length.unwrap_or(MAX_RESPONSE + 1);
            let mut chunk = [0u8; 16384];
            while left > 0 {
                let n = reader
                    .read(&mut chunk[..left.min(16384)])
                    .map_err(io_failure)?;
                if n == 0 {
                    if length.is_some() {
                        return Err(Failure(502, "response-truncated"));
                    }
                    break;
                }
                append(&chunk[..n])?;
                left -= n;
            }
        }
    }
    let content_type = get("content-type")
        .unwrap_or("application/octet-stream")
        .to_string();
    let mut response_headers = vec![("X-Request-Id".into(), req.id.clone())];
    for (name, value) in raw_headers {
        let allowed = if name == "set-cookie" {
            !vitals && auth_cookie_reply(&value)
        } else {
            RESPONSE_HEADERS.contains(&name.as_str())
                // the response headers a route contract allows its handler to set (docs/backend/routes.md)
                || (route.is_some() && (name == "link" || name == "content-language" || (name.starts_with("x-") && name != "x-request-id")))
        };
        if !allowed {
            continue;
        }
        response_headers.push((name, value));
    }
    Ok((
        status,
        content_type,
        response_headers,
        body,
        budget,
        length,
        end,
    ))
}
pub(crate) fn serve(
    stream: &mut TcpStream,
    target: &Target,
    req: &http::Request,
    route: Option<&NativeRoute>,
) -> io::Result<()> {
    let client = forwarded_for(req, stream.peer_addr().ok());
    match fetch_with(target, req, client.as_deref(), route) {
        Ok((status, content_type, headers, body, _budget, length, end)) => {
            let refs = headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect::<Vec<_>>();
            let mut outgoing = Deadline {
                socket: stream.try_clone()?,
                end,
            };
            http::respond_with_length(
                &mut outgoing,
                status,
                &content_type,
                &body,
                req.method == "HEAD",
                if req.method == "HEAD" {
                    length.unwrap_or(0)
                } else {
                    body.len()
                },
                &refs,
            )
        }
        Err(failure) => failure_reply(stream, req, failure),
    }
}
fn failure_reply(stream: &mut TcpStream, req: &http::Request, Failure(status, code): Failure) -> io::Result<()> {
    {
        {
            let body = format!(
                "{{\"error\":\"Native backend request could not complete\",\"code\":\"{code}\",\"rid\":{}}}",
                crate::json_str(&req.id)
            );
            let mut outgoing = Deadline {
                socket: stream.try_clone()?,
                end: Instant::now() + Duration::from_secs(1),
            };
            http::respond_with(
                &mut outgoing,
                status,
                "application/json",
                body.as_bytes(),
                req.method == "HEAD",
                &[("X-Request-Id", &req.id)],
            )
        }
    }
}
/// `/_brain/files`: both directions stream with backpressure; the bucket enforces its size limit and access checks.
pub(crate) fn serve_files(stream: &mut TcpStream, target: &Target, req: &http::Request) -> io::Result<()> {
    if !matches!(req.method.as_str(), "GET" | "HEAD" | "POST" | "DELETE") {
        return failure_reply(stream, req, Failure(405, "method"));
    }
    let Some(_permit) = StreamPermit::acquire() else {
        return failure_reply(stream, req, Failure(503, "stream-capacity"));
    };
    match transfer(stream, target, req) {
        Ok(()) => Ok(()),
        Err(Interrupted::BeforeReply(failure)) => failure_reply(stream, req, failure),
        // bytes already reached the client: a dropped connection is the only honest signal left
        Err(Interrupted::AfterReply) => Ok(()),
    }
}
enum Interrupted {
    BeforeReply(Failure),
    AfterReply,
}
impl From<Failure> for Interrupted {
    fn from(failure: Failure) -> Self {
        Self::BeforeReply(failure)
    }
}
fn transfer(stream: &mut TcpStream, target: &Target, req: &http::Request) -> Result<(), Interrupted> {
    let mut upstream = TcpStream::connect_timeout(&target.address, Duration::from_secs(2)).map_err(|_| Failure(503, "backend-unavailable"))?;
    let idle = |socket: &TcpStream| socket.set_read_timeout(Some(FILE_IDLE)).and_then(|_| socket.set_write_timeout(Some(FILE_IDLE)));
    idle(&upstream).and_then(|_| idle(stream)).map_err(io_failure)?;
    let length = req.upgrade.stream_body.unwrap_or(0);
    let path = encode_path(&req.path);
    let path = if req.query.is_empty() { path } else { format!("{path}?{}", req.query) };
    let mut head = format!("{} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept-Encoding: identity\r\nX-Request-Id: {}\r\n", req.method, target.host, req.id);
    if length > 0 || req.header("content-length").is_some() {
        head.push_str(&format!("Content-Length: {length}\r\n"));
    }
    if let Some(client) = forwarded_for(req, stream.peer_addr().ok()) {
        head.push_str(&format!("x-forwarded-for: {client}\r\n"));
    }
    upstream_headers(&mut head, req, &[&REQUEST_HEADERS[..], &FILE_HEADERS[..]].concat())?;
    head.push_str("\r\n");
    upstream.write_all(head.as_bytes()).map_err(io_failure)?;
    let unread = send_body(stream, &mut upstream, req, length)?;
    let mut reader = BufReader::with_capacity(65536, upstream);
    let (status, headers) = response_head(&mut reader)?;
    let get = |key: &str| headers.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str());
    let framed_length = get("content-length").map(|value| value.parse::<u64>().map_err(|_| Failure(502, "response-length"))).transpose()?;
    if get("transfer-encoding").is_some_and(|value| !value.eq_ignore_ascii_case("chunked") || framed_length.is_some()) {
        return Err(Failure(502, "response-framing").into());
    }
    let mut reply = format!("HTTP/1.1 {status} {}\r\nX-Request-Id: {}\r\n", http::reason(status), req.id);
    if get("cache-control").is_none() {
        reply.push_str("Cache-Control: no-store\r\n");
    }
    for (name, value) in &headers {
        let allowed = if name == "set-cookie" {
            auth_cookie_reply(value)
        } else {
            matches!(name.as_str(), "content-type" | "content-length" | "transfer-encoding") || RESPONSE_HEADERS.contains(&name.as_str()) || FILE_RESPONSE_HEADERS.contains(&name.as_str())
        };
        if allowed {
            reply.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    reply.push_str("Connection: close\r\n\r\n");
    http::trace_status(status);
    stream.write_all(reply.as_bytes()).map_err(|_| Interrupted::AfterReply)?;
    if req.method != "HEAD" && status != 204 && status != 304 {
        let mut left = framed_length;
        let mut chunk = [0u8; 65536];
        while left != Some(0) {
            let want = left.map_or(chunk.len(), |left| left.min(chunk.len() as u64) as usize);
            let n = reader.read(&mut chunk[..want]).map_err(|_| Interrupted::AfterReply)?;
            if n == 0 {
                // close-delimited or chunked bytes are complete at EOF; a short Content-Length stays visibly short
                break;
            }
            stream.write_all(&chunk[..n]).map_err(|_| Interrupted::AfterReply)?;
            left = left.map(|left| left - n as u64);
        }
        if left.is_some_and(|left| left > 0) {
            return Err(Interrupted::AfterReply);
        }
    }
    let _ = stream.flush();
    if unread > 0 {
        // the bucket refused mid-upload: finish sending first, or the client's unread bytes reset the connection
        // and destroy the reply it is waiting for
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let _ = io::copy(&mut (&*stream).take(unread.min(1024 * 1024) as u64), &mut io::sink());
    }
    Ok(())
}
/// Copies the client's body upstream; returns how much it stopped short because the backend stopped reading.
fn send_body(stream: &mut TcpStream, upstream: &mut TcpStream, req: &http::Request, length: usize) -> Result<usize, Interrupted> {
    let buffered = &req.upgrade.head[..req.upgrade.head.len().min(length)];
    if upstream.write_all(buffered).is_err() {
        return Ok(length - buffered.len());
    }
    let mut left = length - buffered.len();
    let end = Instant::now() + FILE_UPLOAD_TOTAL;
    let mut chunk = [0u8; 65536];
    while left > 0 {
        if Instant::now() >= end {
            return Err(Failure(504, "deadline").into());
        }
        let n = match stream.read(&mut chunk[..left.min(65536)]) {
            Ok(0) => return Err(Interrupted::AfterReply),
            Ok(n) => n,
            Err(error) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => return Err(Failure(408, "request-timeout").into()),
            Err(_) => return Err(Interrupted::AfterReply),
        };
        left -= n;
        if upstream.write_all(&chunk[..n]).is_err() {
            return Ok(left);
        }
    }
    let _ = upstream.flush();
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(method: &str) -> http::Request {
        http::Request {
            upgrade: Default::default(),
            method: method.into(),
            path: "/api/items".into(),
            query: "limit=2".into(),
            headers: vec![
                ("origin".into(), "http://127.0.0.1:5182".into()),
                (
                    "cookie".into(),
                    format!(
                        "__Host-brain={}; __Host-session=ignore; unrelated=ignore",
                        "a".repeat(64)
                    ),
                ),
                ("x-forwarded-for".into(), "untrusted".into()),
                ("x-forwarded-host".into(), "spoofed.example".into()),
            ],
            body: Vec::new(),
            id: http::request_id(),
            _body_budget: None,
        }
    }
    fn authority(response: Vec<u8>) -> (Target, std::thread::JoinHandle<http::Request>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target = Target::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let parsed = http::read_request(socket.try_clone().unwrap()).unwrap();
            socket.write_all(&response).unwrap();
            parsed
        });
        (target, thread)
    }
    #[test]
    fn targets_are_fixed_loopback_origins_and_source_runtime_routes_are_not_proxied() {
        for value in [
            "https://127.0.0.1:8000",
            "http://example.test:8000",
            "http://127.0.0.1:8000/api",
            "http://user:pass@127.0.0.1:8000",
            "http://127.0.0.1.evil:8000",
            "http://localhost:0",
            "http://[::1]:8000?x=1",
        ] {
            assert!(Target::parse(value).is_err());
        }
        for value in [
            "http://127.0.0.1:8000",
            "http://localhost:8000/",
            "http://[::1]:8000",
        ] {
            assert!(Target::parse(value).is_ok());
        }
        assert!(handles("/api/items"));
        assert!(handles("/_brain/auth/me"));
        assert!(handles("/_brain/analytics.js"));
        assert!(!handles("/_brain/other.js"));
        assert!(!handles("/@brain/call"));
        assert!(!handles("/api/_http.jsbrain"));
        assert!(!handles("/views/home.brain"));
    }
    #[test]
    fn declared_and_common_header_lists_do_not_fabricate_duplicates_or_hide_real_ones() {
        let (target,server)=authority(b"HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec());
        let mut req=request("POST");req.path="/events/custom".into();req.query.clear();req.body=vec![0,255,128,13,10];
        req.headers.extend([("Content-Type".into(),"application/octet-stream".into()),("x-event-signature".into(),"signed-exact-bytes".into())]);
        let route=NativeRoute{path:req.path.clone(),headers:vec!["Content-Type".into(),"content-type".into(),"X-Event-Signature".into(),"x-event-signature".into()]};
        assert_eq!(fetch_with(&target,&req,None,Some(&route)).unwrap().0,202);
        let received=server.join().unwrap();
        assert_eq!(received.body,req.body);
        assert_eq!(received.headers.iter().filter(|(name,_)|name=="content-type").count(),1);
        assert_eq!(received.headers.iter().filter(|(name,_)|name=="x-event-signature").count(),1);
        req.headers.push(("X-Event-Signature".into(),"different-signature".into()));
        req.headers.push(("content-type".into(),"text/plain".into()));
        let mut head="POST /events/custom HTTP/1.1\r\n".to_string();
        upstream_headers(&mut head,&req,&["content-type","Content-Type","x-event-signature","X-Event-Signature"]).unwrap();
        assert_eq!(head.matches("content-type:").count(),2,"genuine incoming duplicates are never collapsed to a trusted first value");
        assert_eq!(head.matches("x-event-signature:").count(),2);
        head.push_str("Content-Length: 0\r\n\r\n");
        assert!(http::read_request(head.as_bytes()).is_err(),"strict upstream parsing still rejects genuinely ambiguous headers");
    }

    #[test]
    fn signed_provider_transport_preserves_raw_body_and_only_declared_signature_headers() {
        let (target,server)=authority(b"HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec());
        let mut req = request("POST");
        req.path = "/_brain/webhooks/in/orders".into();
        req.query.clear();
        req.body = br#"{ "message": "signed exact bytes" }"#.to_vec();
        for (name, value) in [
            ("content-type", "application/json"),
            ("x-brain-webhook-id", "550e8400-e29b-41d4-a716-446655440000"),
            ("x-brain-webhook-timestamp", "1700000000"),
            ("x-brain-webhook-signature", "v1=synthetic"),
            ("x-brain-webhook-extra", "private-unapproved"),
        ] {
            req.headers.push((name.into(), value.into()));
        }
        assert!(handles(&req.path));
        assert_eq!(fetch(&target, &req).unwrap().0, 202);
        let received = server.join().unwrap();
        assert_eq!(received.body, req.body);
        for name in [
            "x-brain-webhook-id",
            "x-brain-webhook-timestamp",
            "x-brain-webhook-signature",
        ] {
            assert_eq!(received.header(name), req.header(name));
        }
        assert!(received.header("x-brain-webhook-extra").is_none());
    }
    #[test]
    fn native_vitals_transport_drops_all_identity_and_response_cookies() {
        let (target,server)=authority(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nSet-Cookie: __Host-brain=synthetic; Path=/; Secure; HttpOnly\r\nConnection: close\r\n\r\n".to_vec());
        let mut req=request("POST");req.path="/_brain/vitals".into();req.query.clear();req.body=br#"{"route":"/","consent":true,"metrics":[{"name":"TTFB","value":42}]}"#.to_vec();
        for (key,value) in [("authorization","Bearer unrelated"),("user-agent","private-agent"),("referer","https://private.example.test/path"),("dnt","1"),("sec-gpc","1")] { req.headers.push((key.into(),value.into())); }
        let result=fetch_with(&target,&req,Some("203.0.113.1"),None).unwrap();assert_eq!(result.0,204);assert!(!result.2.iter().any(|(key,_)| key=="set-cookie"));
        let received=server.join().unwrap();
        for key in ["authorization","x-api-key","cookie","user-agent","referer","x-forwarded-for","x-request-id","cf-ipcountry"] { assert!(received.header(key).is_none(),"{key}"); }
        assert_eq!(received.header("origin"),req.header("origin"));assert_eq!(received.header("dnt"),Some("1"));assert_eq!(received.header("sec-gpc"),Some("1"));assert_eq!(received.body,req.body);
    }
    #[test]
    fn analytics_beacons_forward_only_the_privacy_and_bot_filter_headers() {
        let (target,server)=authority(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
        let mut req = request("POST");
        req.path = "/_brain/analytics".into();
        req.query.clear();
        req.body = br#"{"v":1}"#.to_vec();
        for (name, value) in [("user-agent", "Mozilla/5.0 Synthetic"), ("dnt", "1"), ("sec-gpc", "1"), ("cf-ipcountry", "MX"), ("x-forwarded-for", "203.0.113.9")] {
            req.headers.push((name.into(), value.into()));
        }
        assert!(handles(&req.path));
        assert_eq!(fetch(&target, &req).unwrap().0, 204);
        let received = server.join().unwrap();
        for name in ["user-agent", "dnt", "sec-gpc", "cf-ipcountry"] {
            assert_eq!(received.header(name), req.header(name));
        }
        assert!(received.header("x-forwarded-for").is_none(), "only the observed peer adds an identity, never a client header alone");
    }
    #[test]
    fn actual_transport_forwards_only_native_credentials_and_owned_request_identity() {
        let (target,server)=authority(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nVary: origin\r\nVary: accept-encoding\r\nSet-Cookie: __Host-brain=; Path=/; Secure; HttpOnly\r\nSet-Cookie: unrelated=do-not-forward\r\nConnection: close\r\n\r\n{}".to_vec());
        let req = request("GET");
        let result = fetch(&target, &req).unwrap();
        let received = server.join().unwrap();
        assert_eq!(received.path, "/api/items");
        assert_eq!(received.query, "limit=2");
        assert_eq!(received.header("x-request-id"), Some(req.id.as_str()));
        assert_eq!(received.header("origin"), Some("http://127.0.0.1:5182"));
        assert_eq!(received.header("x-forwarded-for"), None);
        assert_eq!(received.header("x-forwarded-host"), None);
        assert!(!received.header("cookie").unwrap().contains("ignore"));
        assert_eq!(result.3, b"{}");
        assert!(result.2.iter().any(|(name, _)| name == "set-cookie"));
        assert!(
            !result
                .2
                .iter()
                .any(|(_, value)| value.contains("do-not-forward"))
        );
    }
    #[test]
    fn chunked_responses_are_decoded_but_ambiguous_or_oversized_framing_fails_closed() {
        let (target,server)=authority(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\n{\r\n1\r\n}\r\n0\r\n\r\n".to_vec());
        assert_eq!(fetch(&target, &request("GET")).unwrap().3, b"{}");
        server.join().unwrap();
        for response in [
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                MAX_RESPONSE + 1
            ),
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Length: 1\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n".into(),
        ] {
            let (target, server) = authority(response.into_bytes());
            assert!(fetch(&target, &request("GET")).is_err());
            server.join().unwrap();
        }
    }
    #[test]
    fn head_responses_preserve_advertised_length_without_allocating_or_reading_body() {
        let (target,server)=authority(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 12345\r\nConnection: close\r\n\r\n".to_vec());
        let result = fetch(&target, &request("HEAD")).unwrap();
        assert!(result.3.is_empty());
        assert_eq!(result.5, Some(12345));
        server.join().unwrap();
    }
}
