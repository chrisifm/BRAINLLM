//! Minimal HTTP/1.1 for the dev server: one request per connection (`Connection: close`).

use std::io::{self, BufRead, BufReader, Read, Write};

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    /// Request body (only read when `Content-Length` says there is one; capped at 64 KB like the whole request).
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Reads the request line and headers (the body is ignored: the dev server only handles GET/HEAD).
pub fn read_request<R: Read>(r: R) -> io::Result<Request> {
    let mut br = BufReader::new(r.take(64 * 1024));
    let mut line = String::new();
    br.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "malformed request"));
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (method, path, query) = (method.to_string(), percent_decode(path), query.to_string());
    let mut headers = Vec::new();
    loop {
        line.clear();
        if br.read_line(&mut line)? == 0 {
            break;
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let len = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(0);
    let mut body = vec![0; len.min(64 * 1024)];
    if !body.is_empty() {
        br.read_exact(&mut body)?;
    }
    Ok(Request { method, path, query, headers, body })
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        // Bytes, not `&s[..]`: a `%` followed by a multibyte character must not panic.
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
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

pub fn respond<W: Write>(w: &mut W, status: u16, ctype: &str, body: &[u8], head_only: bool) -> io::Result<()> {
    respond_with(w, status, ctype, body, head_only, &[])
}

/// `respond` plus extra headers (`Location` of a 303, `Retry-After` of a 429).
pub fn respond_with<W: Write>(w: &mut W, status: u16, ctype: &str, body: &[u8], head_only: bool, extra: &[(&str, &str)]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "",
    };
    let extra: String = extra.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    write!(
        w,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{extra}Connection: close\r\n\r\n",
        body.len()
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
        let Some(q) = q.filter(|q| *q >= 0.0) else { continue };
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
    if r.is_empty() { "index.md".into() } else { format!("{r}.md") }
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
        assert_eq!(mime("dist/blog/rss.xml"), "application/rss+xml; charset=utf-8");
        assert_eq!(mime("dist/blog/atom.xml"), "application/atom+xml; charset=utf-8");
        assert_eq!(mime("dist/manifest.webmanifest"), "application/manifest+json");
        assert_eq!(mime("dist/docs/seo.md"), "text/markdown; charset=utf-8");
        for a in ["text/markdown", "text/markdown, */*", "text/markdown, text/html", "text/html;q=0.5, text/markdown", "text/x-markdown"] {
            assert!(prefers_markdown(a), "{a}");
        }
        for a in ["", "*/*", "text/html", "text/html, text/markdown", "text/markdown;q=0", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8", "text/markdown;q=0.5, */*"] {
            assert!(!prefers_markdown(a), "{a}");
        }
        assert_eq!((md_path("/"), md_path("/pricing"), md_path("/docs/setup/")), ("index.md".into(), "pricing.md".into(), "docs/setup.md".into()));
        for (f, m) in [("a.mp4", "video/mp4"), ("a.webm", "video/webm"), ("a.avif", "image/avif"), ("a.gif", "image/gif"), ("a.woff", "font/woff"), ("a.pdf", "application/pdf"), ("a.mp3", "audio/mpeg")] {
            assert_eq!(mime(f), m);
        }
    }

    #[test]
    fn parses_request_line_and_headers() {
        let raw = b"GET /views/home.brain?css&t=1 HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n";
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
}
