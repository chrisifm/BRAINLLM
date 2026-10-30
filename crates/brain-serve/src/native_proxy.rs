//! One explicit loopback backend authority. No arbitrary targets, redirects, credentials or forwarded identities.
use crate::http;
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static BUFFERED: AtomicUsize = AtomicUsize::new(0);
const MAX_RESPONSE: usize = 20 * 1024 * 1024;
const GLOBAL_RESPONSE: usize = 64 * 1024 * 1024;
#[derive(Clone)]
pub(crate) struct Target {
    address: SocketAddr,
    host: String,
}
impl Target {
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
pub(crate) fn read_request(
    stream: &TcpStream,
    upload_path: Option<&str>,
) -> io::Result<http::Request> {
    http::read_request_with_uploads(
        Deadline {
            socket: stream.try_clone()?,
            end: Instant::now() + Duration::from_secs(5),
        },
        upload_path,
    )
}
pub(crate) fn handles(path: &str) -> bool {
    (path.starts_with("/s/")
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
    if header.len() > 16384 {
        return Err(Failure(431, "cookie-budget"));
    }
    let mut values = std::collections::BTreeMap::new();
    for part in header.split(';') {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        if ![
            "__Host-brain",
            "__Host-brain-access",
            "__Host-brain-refresh",
            "__Host-brain-google",
        ]
        .contains(&name)
        {
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
    let _permit = Permit::acquire().ok_or(Failure(503, "request-capacity"))?;
    // Native image/GCS handlers have a thirty-second deadline; preserve their completed result through the edge.
    let resource_path = req.path.split('?').next().unwrap_or("");
    let media = resource_path == "/_brain/uploads" || resource_path.starts_with("/_brain/uploads/");
    let end = Instant::now() + Duration::from_secs(if media { 35 } else { 10 });
    let socket = TcpStream::connect_timeout(&target.address, Duration::from_secs(2))
        .map_err(|_| Failure(503, "backend-unavailable"))?;
    let mut stream = Deadline { socket, end };
    let path = encode_path(&req.path);
    let path = if req.query.is_empty() {
        path
    } else {
        format!("{path}?{}", req.query)
    };
    let mut headers = format!(
        "{} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept-Encoding: identity\r\nX-Request-Id: {}\r\nContent-Length: {}\r\n",
        req.method,
        target.host,
        req.id,
        req.body.len()
    );
    for name in [
        "content-type",
        "accept",
        "origin",
        "authorization",
        "x-brain-webhook-id",
        "x-brain-webhook-timestamp",
        "x-brain-webhook-signature",
    ] {
        if let Some(value) = req.header(name) {
            if value.len() > 8192 || value.chars().any(char::is_control) {
                return Err(Failure(400, "request-header"));
            }
            headers.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    let auth = cookie(req.header("cookie").unwrap_or(""))?;
    if !auth.is_empty() {
        headers.push_str(&format!("Cookie: {auth}\r\n"));
    }
    headers.push_str("\r\n");
    stream
        .write_all(headers.as_bytes())
        .and_then(|_| stream.write_all(&req.body))
        .and_then(|_| stream.flush())
        .map_err(io_failure)?;
    let mut reader = BufReader::new(stream);
    let status_line = line(&mut reader, 8192)?;
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
        let row = line(&mut reader, 8192)?;
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
        if name == "set-cookie" {
            let cookie_name = value.split('=').next().unwrap_or("");
            if ![
                "__Host-brain",
                "__Host-brain-access",
                "__Host-brain-refresh",
                "__Host-brain-google",
            ]
            .contains(&cookie_name)
            {
                continue;
            }
        } else if ![
            "location",
            "content-disposition",
            "retry-after",
            "www-authenticate",
            "clear-site-data",
            "allow",
        ]
        .contains(&name.as_str())
        {
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
) -> io::Result<()> {
    match fetch(target, req) {
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
        Err(Failure(status, code)) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    fn request(method: &str) -> http::Request {
        http::Request {
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
        assert!(!handles("/@brain/call"));
        assert!(!handles("/api/_http.jsbrain"));
        assert!(!handles("/views/home.brain"));
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
