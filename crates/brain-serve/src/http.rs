//! Minimal HTTP/1.1 for the dev server: one request per connection (`Connection: close`).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static BODY_BYTES: AtomicUsize = AtomicUsize::new(0);
const GLOBAL_BODY_BYTES: usize = 64 * 1024 * 1024;
pub(crate) struct BodyBudget(usize);
impl Drop for BodyBudget {
    fn drop(&mut self) {
        BODY_BYTES.fetch_sub(self.0, Ordering::AcqRel);
    }
}
fn reserve_body(bytes: usize) -> io::Result<Option<BodyBudget>> {
    if bytes == 0 {
        return Ok(None);
    }
    let mut used = BODY_BYTES.load(Ordering::Acquire);
    loop {
        if used.saturating_add(bytes) > GLOBAL_BODY_BYTES {
            return Err(io::Error::other("HTTP503 request body capacity exhausted"));
        }
        match BODY_BYTES.compare_exchange_weak(
            used,
            used + bytes,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Ok(Some(BodyBudget(bytes))),
            Err(now) => used = now,
        }
    }
}
/// Correlation only, never an authentication token. Locally generated, not a forwarded client identity.
pub(crate) fn request_id() -> String {
    let seed = format!(
        "{}:{}:{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let mut bytes = brain_pack::sha::sha256(seed.as_bytes());
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex = bytes[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Wire facts needed only when an HTTP connection is transferred to an opaque Upgrade tunnel.
#[derive(Default)]
pub(crate) struct UpgradeMetadata {
    pub version: String,
    pub query_present: bool,
    pub head: Vec<u8>,
}

pub struct Request {
    pub(crate) upgrade: UpgradeMetadata,
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    /// Request body (only read when `Content-Length` says there is one; capped at 64 KB like the whole request).
    pub body: Vec<u8>,
    pub id: String,
    pub(crate) _body_budget: Option<BodyBudget>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Strict nonstreaming HTTP/1.1; upload capacity is enabled only for an explicitly configured native proxy.
#[cfg(test)]
pub fn read_request<R: Read>(reader: R) -> io::Result<Request> {
    read_request_with_uploads(reader, None)
}
pub fn read_request_with_uploads<R: Read>(
    reader: R,
    upload_path: Option<&str>,
) -> io::Result<Request> {
    read_request_with(reader, upload_path, 65536, None)
}
/// `max_body`: the body limit outside the upload route (the dev server passes brain.json `server.maxBodyKB`).
/// `on_continue` answers `Expect: 100-continue` once the length is known to fit (it writes the interim response);
/// without it every expectation is refused with 417.
pub fn read_request_with<R: Read>(
    reader: R,
    upload_path: Option<&str>,
    max_body: usize,
    on_continue: Option<&mut dyn FnMut() -> io::Result<()>>,
) -> io::Result<Request> {
    const HEADERS: usize = 32768;
    let mut reader = BufReader::with_capacity(8192, reader);
    let mut used = 0usize;
    fn line<R: Read>(reader: &mut BufReader<R>, used: &mut usize) -> io::Result<String> {
        let left = 32768usize.saturating_sub(*used);
        if left == 0 {
            return Err(io::Error::other("HTTP431 request headers exceed 32 KiB"));
        }
        let mut bytes = Vec::new();
        let n = reader.take(left as u64 + 1).read_until(b'\n', &mut bytes)?;
        *used = used.saturating_add(n);
        if *used > 32768 || n > 8192 {
            return Err(io::Error::other(
                "HTTP431 request header line exceeds its budget",
            ));
        }
        if !bytes.ends_with(b"\r\n") {
            return Err(io::Error::other(
                "HTTP400 incomplete or invalid request headers",
            ));
        }
        bytes.truncate(bytes.len() - 2);
        String::from_utf8(bytes).map_err(|_| io::Error::other("HTTP400 headers must be UTF-8"))
    }
    let request = line(&mut reader, &mut used)?;
    let parts = request.split(' ').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0].is_empty()
        || parts[0].len() > 16
        || !parts[0].bytes().all(|b| b.is_ascii_uppercase())
        || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1")
        || (!parts[1].starts_with('/') || parts[1].starts_with("//"))
        || parts[1].len() > 8192
        || parts[1].bytes().any(|b| b <= 32 || b == 127)
    {
        return Err(io::Error::other("HTTP400 malformed request line"));
    }
    let (raw_path, query) = parts[1].split_once('?').unwrap_or((parts[1], ""));
    let path = percent_decode(raw_path);
    if path
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '?' | '#'))
    {
        return Err(io::Error::other("HTTP400 unsafe decoded request path"));
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let text = line(&mut reader, &mut used)?;
        if text.is_empty() {
            break;
        }
        if text.starts_with([' ', '\t']) {
            return Err(io::Error::other("HTTP400 folded headers are unsupported"));
        }
        let (name, value) = text
            .split_once(':')
            .ok_or_else(|| io::Error::other("HTTP400 malformed header"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.chars().any(|c| c.is_control() && c != '\t')
        {
            return Err(io::Error::other("HTTP400 unsafe header"));
        }
        if headers
            .iter()
            .any(|(old, _)| old.eq_ignore_ascii_case(name))
        {
            return Err(io::Error::other("HTTP400 duplicate request header"));
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        if headers.len() > 128 || used > HEADERS {
            return Err(io::Error::other("HTTP431 request header budget exceeded"));
        }
    }
    let header = |key: &str| {
        headers
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    if header("transfer-encoding").is_some() {
        return Err(io::Error::other(
            "HTTP400 transfer encoding is unsupported; send an exact Content-Length",
        ));
    }
    let length = match header("content-length") {
        None => 0,
        Some(value) if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) => {
            return Err(io::Error::other("HTTP400 invalid Content-Length"));
        }
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| io::Error::other("HTTP413 request body exceeds its budget"))?,
    };
    let upload = upload_path == Some(path.as_str()) && parts[0] == "POST";
    let limit = if upload { 5 * 1024 * 1024 + 65536 } else { max_body };
    if length > limit {
        return Err(io::Error::other(if upload {
            "HTTP413 upload body exceeds 5 MB".to_string()
        } else {
            format!("HTTP413 Body larger than {} KB; raise brain.json server.maxBodyKB", max_body / 1024)
        }));
    }
    // checked after the length: a body past the limit gets its 413 before the client sends it
    match (header("expect"), on_continue) {
        (None, _) => {}
        (Some(value), Some(send)) if value.eq_ignore_ascii_case("100-continue") => send()?,
        (Some(_), _) => {
            return Err(io::Error::other(
                "HTTP417 only Expect: 100-continue is supported",
            ));
        }
    }
    let body_budget = reserve_body(length)?;
    let mut body = Vec::new();
    body.try_reserve_exact(length)
        .map_err(|_| io::Error::other("HTTP503 request body allocation unavailable"))?;
    body.resize(length, 0);
    reader.read_exact(&mut body)?;
    let upgrade = UpgradeMetadata {
        version: parts[2].into(),
        query_present: parts[1].contains('?'),
        head: if header("upgrade").is_some() {
            reader.buffer().to_vec()
        } else {
            Vec::new()
        },
    };
    Ok(Request {
        upgrade,
        method: parts[0].into(),
        path,
        query: query.into(),
        headers,
        body,
        id: request_id(),
        _body_budget: body_budget,
    })
}
pub fn request_error_status(error: &io::Error) -> u16 {
    let text = error.to_string();
    for code in [413u16, 417, 431, 503] {
        if text.starts_with(&format!("HTTP{code}")) {
            return code;
        }
    }
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        408
    } else {
        400
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        // Bytes, not `&s[..]`: a `%` followed by a multibyte character must not panic.
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn respond<W: Write>(
    w: &mut W,
    status: u16,
    ctype: &str,
    body: &[u8],
    head_only: bool,
) -> io::Result<()> {
    respond_with(w, status, ctype, body, head_only, &[])
}

/// `respond` plus extra headers (`Location` of a 303, `Retry-After` of a 429).
pub fn respond_with<W: Write>(
    w: &mut W,
    status: u16,
    ctype: &str,
    body: &[u8],
    head_only: bool,
    extra: &[(&str, &str)],
) -> io::Result<()> {
    respond_with_length(w, status, ctype, body, head_only, body.len(), extra)
}
pub(crate) fn respond_with_length<W: Write>(
    w: &mut W,
    status: u16,
    ctype: &str,
    body: &[u8],
    head_only: bool,
    advertised_length: usize,
    extra: &[(&str, &str)],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Content Too Large",
        417 => "Expectation Failed",
        431 => "Request Header Fields Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    };
    let extra: String = extra.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    write!(
        w,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{extra}Connection: close\r\n\r\n",
        advertised_length
    )?;
    if !head_only {
        w.write_all(body)?;
    }
    w.flush()
}

/// True when `Accept` ranks `text/markdown` above `text/html` (equal weights: whichever is listed first; an explicit
/// type beats a wildcard). Wildcards alone never pick Markdown, so browsers keep HTML. Same rule as server.mjs.
pub fn prefers_markdown(accept: &str) -> bool {
    let (mut md, mut html, mut any) = (-1.0f64, -1.0f64, -1.0f64);
    let (mut md_at, mut html_at) = (usize::MAX, usize::MAX);
    for (i, part) in accept.split(',').enumerate() {
        let mut it = part.split(';');
        let ty = it.next().unwrap_or("").trim().to_ascii_lowercase();
        let q = it
            .map(str::trim)
            .find_map(|p| p.strip_prefix("q=").or_else(|| p.strip_prefix("Q=")))
            .map_or(Some(1.0), |v| v.trim().parse::<f64>().ok());
        let Some(q) = q.filter(|q| *q >= 0.0) else {
            continue;
        };
        match ty.as_str() {
            "text/markdown" | "text/x-markdown" => {
                md = md.max(q);
                md_at = md_at.min(i);
            }
            "text/html" | "application/xhtml+xml" => {
                html = html.max(q);
                html_at = html_at.min(i);
            }
            "text/*" | "*/*" => any = any.max(q),
            _ => {}
        }
    }
    if html < 0.0 {
        html = any;
    }
    md > 0.0 && (md > html || (md == html && md_at < html_at))
}

/// `/` -> `index.md`, `/docs/setup/` -> `docs/setup.md` (brain-seo `files::md_path`).
pub fn md_path(url_path: &str) -> String {
    let r = url_path.trim_matches('/');
    if r.is_empty() {
        "index.md".into()
    } else {
        format!("{r}.md")
    }
}

pub fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" | "brain" | "jsbrain" => "text/javascript; charset=utf-8",
        "css" | "cssbrain" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        // per-page Markdown copies (llms.txt convention)
        "md" => "text/markdown; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        "xml" if path.ends_with("rss.xml") => "application/rss+xml; charset=utf-8",
        "xml" if path.ends_with("atom.xml") => "application/atom+xml; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "avif" => "image/avif",
        "gif" => "image/gif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_covers_seo_files_and_public_media() {
        assert_eq!(mime("dist/sitemap.xml"), "application/xml; charset=utf-8");
        assert_eq!(
            mime("dist/blog/rss.xml"),
            "application/rss+xml; charset=utf-8"
        );
        assert_eq!(
            mime("dist/blog/atom.xml"),
            "application/atom+xml; charset=utf-8"
        );
        assert_eq!(
            mime("dist/manifest.webmanifest"),
            "application/manifest+json"
        );
        assert_eq!(mime("dist/docs/seo.md"), "text/markdown; charset=utf-8");
        for a in [
            "text/markdown",
            "text/markdown, */*",
            "text/markdown, text/html",
            "text/html;q=0.5, text/markdown",
            "text/x-markdown",
        ] {
            assert!(prefers_markdown(a), "{a}");
        }
        for a in [
            "",
            "*/*",
            "text/html",
            "text/html, text/markdown",
            "text/markdown;q=0",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "text/markdown;q=0.5, */*",
        ] {
            assert!(!prefers_markdown(a), "{a}");
        }
        assert_eq!(
            (md_path("/"), md_path("/pricing"), md_path("/docs/setup/")),
            (
                "index.md".into(),
                "pricing.md".into(),
                "docs/setup.md".into()
            )
        );
        for (f, m) in [
            ("a.mp4", "video/mp4"),
            ("a.webm", "video/webm"),
            ("a.avif", "image/avif"),
            ("a.gif", "image/gif"),
            ("a.woff", "font/woff"),
            ("a.pdf", "application/pdf"),
            ("a.mp3", "audio/mpeg"),
        ] {
            assert_eq!(mime(f), m);
        }
    }

    #[test]
    fn parses_request_line_and_headers() {
        let raw =
            b"GET /views/home.brain?css&t=1 HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n";
        let r = read_request(&raw[..]).unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/views/home.brain");
        assert_eq!(r.query, "css&t=1");
        assert_eq!(r.header("upgrade"), Some("websocket"));
    }

    #[test]
    fn decodes_percent() {
        assert_eq!(percent_decode("/a%20b"), "/a b");
        assert_eq!(percent_decode("/100%"), "/100%");
        assert_eq!(percent_decode("/%€"), "/%€");
    }
    #[test]
    fn oversized_bodies_are_rejected_before_reading_or_allocating_them() {
        let request =
            b"POST /api/items HTTP/1.1\r\nHost: localhost\r\nContent-Length: 65537\r\n\r\n";
        let error = read_request(&request[..]).err().unwrap();
        assert_eq!(request_error_status(&error), 413);
        let request =
            b"POST /_brain/uploads HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5308417\r\n\r\n";
        let error = read_request_with_uploads(&request[..], Some("/_brain/uploads"))
            .err()
            .unwrap();
        assert_eq!(request_error_status(&error), 413);
    }
    fn post(len: usize, extra: &str) -> Vec<u8> {
        let mut request = format!("POST /_r/h1/save HTTP/1.1\r\nHost: localhost\r\n{extra}Content-Length: {len}\r\n\r\n").into_bytes();
        request.extend(std::iter::repeat_n(b'x', len));
        request
    }
    #[test]
    fn body_limit_comes_from_the_caller_and_the_413_names_the_setting() {
        let request = post(200 * 1024, "");
        assert_eq!(read_request_with(&request[..], None, 1024 * 1024, None).unwrap().body.len(), 200 * 1024);
        let error = read_request_with(&request[..], None, 100 * 1024, None).err().unwrap();
        assert_eq!(request_error_status(&error), 413);
        assert!(error.to_string().contains("100 KB") && error.to_string().contains("brain.json server.maxBodyKB"), "{error}");
    }
    #[test]
    fn expect_100_continue_is_answered_within_the_limit_and_refused_past_it() {
        let mut sent = 0;
        let mut send = || -> io::Result<()> { sent += 1; Ok(()) };
        let request = post(2048, "Expect: 100-continue\r\n");
        assert_eq!(read_request_with(&request[..], None, 4096, Some(&mut send)).unwrap().body.len(), 2048);
        assert_eq!(sent, 1, "100 Continue is sent once, before the body is read");
        let mut send = || -> io::Result<()> { panic!("no 100 Continue past the limit") };
        let error = read_request_with(&request[..], None, 1024, Some(&mut send)).err().unwrap();
        assert_eq!(request_error_status(&error), 413);
        let mut send = || -> io::Result<()> { Ok(()) };
        let other = post(10, "Expect: something-else\r\n");
        assert_eq!(request_error_status(&read_request_with(&other[..], None, 4096, Some(&mut send)).err().unwrap()), 417);
        // a reader with no way to answer (the native proxy) keeps refusing expectations
        assert_eq!(request_error_status(&read_request_with(&request[..], None, 4096, None).err().unwrap()), 417);
    }
    #[test]
    fn enabled_exact_upload_route_accepts_larger_bodies_and_generates_local_request_identity() {
        let body = vec![b'x'; 100000];
        let mut request=format!("POST /app/_brain/uploads HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nX-Request-Id: client-supplied\r\n\r\n",body.len()).into_bytes();
        request.extend_from_slice(&body);
        assert_eq!(
            request_error_status(
                &read_request_with_uploads(&request[..], Some("/_brain/uploads"))
                    .err()
                    .unwrap()
            ),
            413
        );
        let parsed = read_request_with_uploads(&request[..], Some("/app/_brain/uploads")).unwrap();
        assert_eq!(parsed.body, body);
        assert_ne!(parsed.id, "client-supplied");
        assert_eq!(parsed.id.len(), 36);
    }
    #[test]
    fn duplicate_length_transfer_encoding_and_control_headers_are_rejected() {
        for headers in [
            "Content-Length: 0\r\ncontent-length: 0\r\n",
            "Content-Length: 0\r\nTransfer-Encoding: chunked\r\n",
            "Host: second\r\n",
            "X-Test: bad\u{1}value\r\n",
            "Content-Length: +2\r\n",
        ] {
            let request = format!("POST /api/items HTTP/1.1\r\nHost: localhost\r\n{headers}\r\n");
            let error = read_request(request.as_bytes()).err().unwrap();
            assert_eq!(request_error_status(&error), 400);
        }
        assert_eq!(
            request_error_status(
                &read_request(b"GET /api/%0d%0aevil HTTP/1.1\r\nHost: local\r\n\r\n".as_slice())
                    .err()
                    .unwrap()
            ),
            400
        );
    }
}
