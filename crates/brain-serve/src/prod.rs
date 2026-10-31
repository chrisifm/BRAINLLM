//! Static production server for `brain start` (INTERNALS §34): serves `dist/` from Rust, no Node. Precompressed
//! `.gz` when the client accepts it, everything under brain.json `base` (#148), clean URLs (`/about` → `about/index.html`), `404.html`, the headers of
//! `dist/_headers` applied exactly as a static host would (`/*` and `/_b/*` are prefix rules), a strong `ETag` equal to
//! the build signature on HTML (304 on `If-None-Match`) and `GET /_health` → `{"ok":true,"build":"<sig>"}`.

use crate::server::native_proxy;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// `(pattern, [(name, value)])` from `_headers` (Netlify format).
pub type Rules = Vec<(String, Vec<(String, String)>)>;

pub fn parse_headers(src: &str) -> Rules {
    let mut out: Rules = Vec::new();
    for line in src.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            out.push((line.trim().to_string(), Vec::new()));
        } else if let (Some(last), Some((k, v))) = (out.last_mut(), line.trim().split_once(':')) {
            last.1.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

/// Headers for `path`: every matching rule in file order (exact path, or `prefix*`); a later value of the same
/// header replaces an earlier one, so a page rule overrides `/*`.
pub fn headers_for(rules: &Rules, path: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (pat, hs) in rules {
        let hit = match pat.strip_suffix('*') {
            Some(prefix) => path.starts_with(prefix),
            None => pat == path,
        };
        if hit {
            for (k, v) in hs {
                out.retain(|(a, _)| !a.eq_ignore_ascii_case(k));
                out.push((k.clone(), v.clone()));
            }
        }
    }
    out
}

// Every served candidate, including gzip and fallback pages, must be a regular file
// reached without following symlinks below the configured output directory.
fn regular_output(dist: &Path, file: &Path) -> bool {
    if !std::fs::symlink_metadata(dist).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return false;
    }
    let Ok(relative) = file.strip_prefix(dist) else {
        return false;
    };
    let mut cursor = dist.to_path_buf();
    for part in relative.components() {
        if !matches!(part, Component::Normal(_)) {
            return false;
        }
        cursor.push(part);
        let Ok(meta) = std::fs::symlink_metadata(&cursor) else {
            return false;
        };
        if meta.file_type().is_symlink() {
            return false;
        }
    }
    file.is_file()
}

/// The build signature (`<meta name="brain-build">` of the home page, else of the 404 page; `.html` or `.brain`).
pub fn signature(dist: &Path) -> Option<String> {
    for f in ["index.html", "404.html", "index.brain", "404.brain"] {
        if !regular_output(dist, &dist.join(f)) {
            continue;
        }
        if let Ok(html) = std::fs::read_to_string(dist.join(f)) {
            if let Some(i) = html.find("name=\"brain-build\" content=\"") {
                let rest = &html[i + 28..];
                return rest.split('"').next().map(str::to_string);
            }
        }
    }
    None
}

pub struct Static {
    pub dist: PathBuf,
    pub rules: Rules,
    pub build: String,
    /// brain.json `base` without its trailing slash (`/app`), empty at the root (#148).
    pub pre: String,
    /// `_redirects` (redirects.json + `seo.trailingSlash`/`seo.canonicalHost`, §33): one hop per request.
    pub redirects: (Vec<brain_seo::redirects::Rule>, brain_seo::redirects::Policy),
}

impl Static {
    pub fn load(dist: &Path, base: &str) -> Static {
        Static {
            pre: base.trim_end_matches('/').to_string(),
            dist: dist.to_path_buf(),
            rules: if regular_output(dist, &dist.join("_headers")) {
                std::fs::read_to_string(dist.join("_headers"))
                    .map(|s| parse_headers(&s))
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
            build: signature(dist).unwrap_or_default(),
            redirects: if regular_output(dist, &dist.join("_redirects")) {
                std::fs::read_to_string(dist.join("_redirects"))
                    .map(|s| brain_seo::redirects::parse_netlify(&s))
                    .unwrap_or_default()
            } else {
                Default::default()
            },
        }
    }

    /// `/about` → `about/index.html`, `about.html` or `about`; None outside dist/ or for host config files.
    fn resolve(&self, url_path: &str) -> Option<PathBuf> {
        let rel = url_path.trim_start_matches('/');
        if Path::new(rel)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
            || rel.starts_with("_server")
            || rel.starts_with("_brain")
            || rel == "_headers"
            || rel.starts_with("server.mjs")
            || rel == "_redirects"
            || rel.contains('\\')
            || rel
                .split('/')
                .any(|part| part.starts_with('.') && part != ".well-known")
        {
            return None;
        }
        let base = self.dist.join(rel);
        // pages are `index.html` (ext web) or `index.brain` (ext brain, #26)
        let cands = if rel.is_empty() || rel.ends_with('/') {
            vec![base.join("index.html"), base.join("index.brain")]
        } else {
            vec![
                base.clone(),
                base.join("index.html"),
                base.join("index.brain"),
                self.dist.join(format!("{rel}.html")),
                self.dist.join(format!("{rel}.brain")),
            ]
        };
        cands.into_iter().find(|p| regular_output(&self.dist, p))
    }

    /// Serves one connection (one request; `Connection: close`).
    pub fn handle(&self, s: TcpStream) -> io::Result<()> {
        let backend = std::env::var("BRAIN_BACKEND_URL").ok();
        self.handle_with_backend(s, backend.as_deref())
    }
    /// Explicit embedding variant; existing load/handle APIs and Static fields remain unchanged.
    pub fn handle_with_backend(&self, s: TcpStream, backend: Option<&str>) -> io::Result<()> {
        self.handle_native(s, backend, None)
    }
    fn handle_native(
        &self,
        mut s: TcpStream,
        backend: Option<&str>,
        sockets: Option<&crate::NativeSocketProxy>,
    ) -> io::Result<()> {
        let target = backend
            .filter(|s| !s.is_empty())
            .map(native_proxy::Target::parse)
            .transpose()
            .map_err(io::Error::other)?;
        let upload = target
            .as_ref()
            .map(|_| format!("{}/_brain/uploads", self.pre));
        let mut req = match native_proxy::read_request(&s, upload.as_deref()) {
            Ok(req) => req,
            Err(error) => {
                let status = crate::http::request_error_status(&error);
                return crate::http::respond(
                    &mut s,
                    status,
                    "application/json",
                    b"{\"error\":\"Invalid or over-budget request\"}",
                    false,
                );
            }
        };
        let path = req.path.clone();
        let native_path = if self.pre.is_empty() {
            Some(path.as_str())
        } else {
            path.strip_prefix(&self.pre)
                .filter(|path| path.starts_with('/'))
        };
        if native_path == Some("/_brain/socket") {
            req.path = "/_brain/socket".into();
            return match sockets {
                Some(proxy) => proxy.takeover(&mut s, &mut req),
                None => crate::http::respond(
                    &mut s,
                    404,
                    "application/json",
                    b"{\"error\":\"Native sockets are disabled\"}",
                    false,
                ),
            };
        }
        let analytics = target.is_some() && std::env::var("BRAIN_ANALYTICS").as_deref() != Ok("off");
        if let (Some(target), Some(native_path)) = (target, native_path) {
            if native_proxy::handles(native_path) {
                req.path = native_path.to_string();
                return native_proxy::serve(&mut s, &target, &req);
            }
        }
        if req.method != "GET" && req.method != "HEAD" {
            return write(
                &mut s,
                405,
                &[("Allow".into(), "GET, HEAD".into())],
                b"",
                false,
            );
        }
        let head = req.method == "HEAD";
        if path == "/_health" {
            let body = format!("{{\"ok\":true,\"build\":{}}}", crate::json_str(&self.build));
            return write(
                &mut s,
                200,
                &[
                    ("Content-Type".into(), "application/json".into()),
                    ("Cache-Control".into(), "no-store".into()),
                ],
                body.as_bytes(),
                head,
            );
        }
        // base (#148): `/app` → `/app/`; files resolve without it; outside it → 404 (`/_health` stays at the root)
        let inner = if self.pre.is_empty() {
            Some(path.clone())
        } else if path == self.pre {
            return write(
                &mut s,
                301,
                &[("Location".into(), format!("{}/", self.pre))],
                b"",
                head,
            );
        } else {
            path.strip_prefix(&self.pre)
                .filter(|r| r.starts_with('/'))
                .map(str::to_string)
        };
        // §33: the redirect rule, the trailing-slash policy and the canonical host as ONE hop, query kept
        if let Some((loc, code)) = inner.as_deref().and_then(|p| {
            let q = if req.query.is_empty() { String::new() } else { format!("?{}", req.query) };
            brain_seo::redirects::resolve(&self.redirects.0, &self.redirects.1, p, &q, req.header("host"))
        }) {
            let loc = if loc.starts_with('/') { format!("{}{loc}", self.pre) } else { loc };
            let cache = if code == 301 || code == 308 { "public, max-age=3600" } else { "no-store" };
            return write(&mut s, code, &[("Location".into(), loc), ("Cache-Control".into(), cache.into())], b"", head);
        }
        let (status, file) = match inner.as_deref().and_then(|p| self.resolve(p)) {
            Some(f) => (200, f),
            None => match ["404.html", "404.brain"]
                .iter()
                .map(|f| self.dist.join(f))
                .find(|f| regular_output(&self.dist, f))
            {
                Some(f) => (404, f),
                None => {
                    return write(
                        &mut s,
                        404,
                        &[("Content-Type".into(), "text/plain".into())],
                        b"not found",
                        head,
                    );
                }
            },
        };
        let mut name = file.to_string_lossy().to_string();
        // in a build, `.brain` is a page and `.jsbrain`/`.cssbrain` its script and styles (in dev they are sources)
        let mut html = name.ends_with(".html") || name.ends_with(".brain");
        // §33: a page with a Markdown twin (`/pricing` -> pricing.md) answers it when `Accept` prefers text/markdown;
        // both representations say `Vary: Accept` (no twin with seo.markdown=false or on noindex pages)
        let twin = if status == 200 && html {
            inner
                .as_deref()
                .map(|p| self.dist.join(crate::http::md_path(p)))
                .filter(|f| regular_output(&self.dist, f))
        } else {
            None
        };
        let file = match &twin {
            Some(md)
                if req
                    .header("accept")
                    .is_some_and(crate::http::prefers_markdown) =>
            {
                name = md.to_string_lossy().to_string();
                html = false;
                md.clone()
            }
            _ => file,
        };
        let mut hs = headers_for(&self.rules, &path);
        if twin.is_some() {
            hs.retain(|(k, _)| !k.eq_ignore_ascii_case("vary"));
            hs.push(("Vary".into(), "Accept, Accept-Encoding".into()));
        }
        hs.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
        hs.push(("Content-Type".into(), built_mime(&name).into()));
        if html {
            hs.push(("X-Brain-Static".into(), "1".into()));
        }
        if html && !self.build.is_empty() {
            // the page's validator is the build signature: unchanged build → 304 with no body
            let tag = format!("\"{}\"", self.build);
            hs.retain(|(k, _)| !k.eq_ignore_ascii_case("x-brain-build"));
            hs.push(("X-Brain-Build".into(), self.build.clone()));
            hs.push(("ETag".into(), tag.clone()));
            if status == 200
                && req
                    .header("if-none-match")
                    .is_some_and(|v| v.split(',').any(|t| t.trim() == tag))
            {
                return write(&mut s, 304, &hs, b"", true);
            }
        }
        if html && analytics {
            // behavior analytics: the page gains the backend-served collector at response time (never in the build)
            if let Ok(page) = std::fs::read_to_string(&file) {
                return write(&mut s, status, &hs, with_analytics(&page, &self.pre).as_bytes(), head);
            }
        }
        let variants = [
            ("br", PathBuf::from(format!("{name}.br"))),
            ("gzip", PathBuf::from(format!("{name}.gz"))),
        ];
        let selected = accepted_encodings(req.header("accept-encoding").unwrap_or(""))
            .into_iter()
            .find_map(|enc| {
                variants
                    .iter()
                    .find(|(name, path)| *name == enc && regular_output(&self.dist, path))
            });
        let selected_file = if let Some((encoding, path)) = selected {
            hs.push(("Content-Encoding".into(), (*encoding).into()));
            path
        } else {
            &file
        };
        if variants.iter().any(|(_, p)| regular_output(&self.dist, p)) {
            let index = hs.iter().position(|(k, _)| k.eq_ignore_ascii_case("vary"));
            if let Some(i) = index {
                if !hs[i]
                    .1
                    .split(',')
                    .any(|s| s.trim().eq_ignore_ascii_case("accept-encoding"))
                {
                    hs[i].1.push_str(", Accept-Encoding");
                }
            } else {
                hs.push(("Vary".into(), "Accept-Encoding".into()));
            }
        }
        write_file(&mut s, status, &hs, selected_file, head)
    }
}

/// Adds the async collector module (`<pre>/_brain/analytics.js`, served by the native backend) before `</head>`, once.
pub(crate) fn with_analytics(html: &str, pre: &str) -> String {
    let tag = format!("<script type=\"module\" async src=\"{pre}/_brain/analytics.js\"></script>");
    if html.contains(&tag) {
        html.to_string()
    } else if html.contains("</head>") {
        html.replacen("</head>", &format!("{tag}</head>"), 1)
    } else {
        format!("{html}{tag}")
    }
}

/// Accept only complete coding tokens and honor q=0, relative weights and wildcard fallback.
pub fn accepted_encodings(header: &str) -> Vec<&'static str> {
    let quality = |coding: &str| {
        let mut explicit = None;
        let mut wildcard = None;
        for part in header.split(',') {
            let mut fields = part.trim().split(';');
            let name = fields.next().unwrap_or("").trim();
            let mut q = 1.0_f32;
            for field in fields {
                if let Some((key, value)) = field.trim().split_once('=') {
                    if key.trim().eq_ignore_ascii_case("q") {
                        q = value
                            .trim()
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                            .unwrap_or(0.0);
                    }
                }
            }
            if name.eq_ignore_ascii_case(coding) {
                explicit = Some(q)
            } else if name == "*" {
                wildcard = Some(q)
            }
        }
        explicit.or(wildcard).unwrap_or(0.0)
    };
    let mut encodings = vec![("br", quality("br")), ("gzip", quality("gzip"))];
    encodings.retain(|(_, q)| *q > 0.0);
    encodings.sort_by(|a, b| b.1.total_cmp(&a.1));
    encodings.into_iter().map(|(name, _)| name).collect()
}

/// Content-Type of a built file: `http::mime`, except the `--ext brain` names, which are published pages and assets.
pub fn built_mime(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "brain" => "text/html; charset=utf-8",
        "jsbrain" => "text/javascript; charset=utf-8",
        "cssbrain" => "text/css; charset=utf-8",
        _ => crate::http::mime(name),
    }
}

struct TimedWriter<'a> {
    stream: &'a mut TcpStream,
    end: Instant,
}
impl Write for TimedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let left = self.end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "static response deadline",
            ));
        }
        self.stream.set_write_timeout(Some(left))?;
        self.stream.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}
fn write_file(
    s: &mut TcpStream,
    status: u16,
    headers: &[(String, String)],
    path: &Path,
    head: bool,
) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("static output must be regular"));
    }
    let mut writer = TimedWriter {
        stream: s,
        end: Instant::now() + Duration::from_secs(10),
    };
    write_headers(&mut writer, status, headers, metadata.len())?;
    if !head {
        let copied = io::copy(&mut file.take(metadata.len()), &mut writer)?;
        if copied != metadata.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "static output changed during transfer",
            ));
        }
    }
    writer.flush()
}
fn write(
    s: &mut TcpStream,
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    head: bool,
) -> io::Result<()> {
    let mut writer = TimedWriter {
        stream: s,
        end: Instant::now() + Duration::from_secs(10),
    };
    write_headers(&mut writer, status, headers, body.len() as u64)?;
    if !head {
        writer.write_all(body)?;
    }
    writer.flush()
}
fn write_headers<W: Write>(
    s: &mut W,
    status: u16,
    headers: &[(String, String)],
    length: u64,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        304 => "Not Modified",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "",
    };
    let mut h = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        length
    );
    for (k, v) in headers {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str("\r\n");
    s.write_all(h.as_bytes())?;
    Ok(())
}

/// Managed static gateway. Its authority is fixed for HTTP and sockets at construction.
/// Clone an `Arc<StaticGateway>` across request workers; dropping the last owner reaps tunnels.
pub struct StaticGateway {
    files: Static,
    backend: Option<String>,
    sockets: Option<crate::NativeSocketProxy>,
}
impl StaticGateway {
    pub fn load(dist: &Path, base: &str, backend: Option<&str>) -> io::Result<Self> {
        let backend = backend
            .filter(|v| !v.is_empty())
            .map(|value| {
                native_proxy::Target::parse(value)
                    .map(|_| value.to_owned())
                    .map_err(io::Error::other)
            })
            .transpose()?;
        let manifest = dist.join("_server/manifest.json");
        let enabled = if !manifest.try_exists()? {
            None
        } else {
            if !regular_output(dist, &manifest) {
                return Err(io::Error::other(
                    "Static socket manifest must be a regular owned file",
                ));
            }
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = options.open(&manifest)?;
            if !file.metadata()?.is_file() {
                return Err(io::Error::other(
                    "Static socket manifest must remain a regular file",
                ));
            }
            let mut bytes = Vec::new();
            file.take(65537).read_to_end(&mut bytes)?;
            if bytes.len() > 65536 {
                return Err(io::Error::other("Static socket manifest exceeds 64 KiB"));
            }
            #[derive(serde::Deserialize)]
            struct Manifest {
                v: Option<u64>,
                #[serde(default, rename = "nativeSockets")]
                native_sockets: bool,
                #[serde(rename = "publicOrigin")]
                public_origin: Option<String>,
            }
            let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| {
                io::Error::other("Invalid static socket manifest JSON or field type")
            })?;
            if manifest.native_sockets {
                if manifest.v != Some(1) {
                    return Err(io::Error::other(
                        "Unsupported static socket manifest version",
                    ));
                }
                Some(
                    manifest
                        .public_origin
                        .ok_or_else(|| io::Error::other("Static sockets require publicOrigin"))?,
                )
            } else {
                None
            }
        };
        let sockets = match enabled {
            None => None,
            Some(origin) => crate::NativeProxy::new(
                backend
                    .as_deref()
                    .ok_or_else(|| io::Error::other("Static sockets require BRAIN_BACKEND_URL"))?,
            )
            .map_err(io::Error::other)?
            .sockets(true)
            .socket_origins(vec![origin])
            .map_err(io::Error::other)?
            .start_sockets()?,
        };
        Ok(Self {
            files: Static::load(dist, base),
            backend,
            sockets,
        })
    }
    pub fn handle(&self, stream: TcpStream) -> io::Result<()> {
        self.files
            .handle_native(stream, self.backend.as_deref(), self.sockets.as_ref())
    }
    pub fn stop(&self) -> io::Result<()> {
        self.sockets.as_ref().map_or(Ok(()), |proxy| proxy.stop())
    }
}

/// Serves `dist/` on `listener` until `stop` is set; one thread per connection.
pub fn serve_static(
    listener: TcpListener,
    dist: &Path,
    base: &str,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    let backend = std::env::var("BRAIN_BACKEND_URL").ok();
    let gateway = StaticGateway::load(dist, base, backend.as_deref())?;
    serve_static_with_gateway(listener, gateway, stop)
}

/// Serves an already validated gateway without reading or changing process environment.
pub fn serve_static_with_gateway(
    listener: TcpListener,
    gateway: StaticGateway,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    let st = Arc::new(gateway);
    let active = Arc::new(AtomicUsize::new(0));
    listener.set_nonblocking(true)?;
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((s, _)) => {
                let _ = s.set_nonblocking(false);
                if active
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                        if count < 64 { Some(count + 1) } else { None }
                    })
                    .is_err()
                {
                    let mut s = s;
                    let _ = s.set_write_timeout(Some(Duration::from_millis(100)));
                    let _ = crate::http::respond(
                        &mut s,
                        503,
                        "application/json",
                        b"{\"error\":\"Static server capacity exhausted\"}",
                        false,
                    );
                    continue;
                }
                struct Slot(Arc<AtomicUsize>);
                impl Drop for Slot {
                    fn drop(&mut self) {
                        self.0.fetch_sub(1, Ordering::AcqRel);
                    }
                }
                let slot = Slot(active.clone());
                let st = st.clone();
                let _ = std::thread::Builder::new()
                    .name("brain-static-request".into())
                    .spawn(move || {
                        let _slot = slot;
                        let _ = st.handle(s);
                    });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            Err(e) => return Err(e),
        }
    }
    st.stop()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_files_and_compressed_server_sources_are_never_public() {
        let dist = std::env::temp_dir().join(format!("brain-prod-private-{}", std::process::id()));
        std::fs::create_dir_all(dist.join("_server")).unwrap();
        std::fs::create_dir_all(dist.join(".well-known")).unwrap();
        for path in [
            "server.mjs",
            "server.mjs.gz",
            ".brain_env",
            "_headers",
            "_redirects",
            "_server/private.mjs",
            ".well-known/security.txt",
        ] {
            std::fs::write(dist.join(path), "synthetic private source").unwrap();
        }
        let app = Static::load(&dist, "/");
        for path in [
            "/server.mjs",
            "/server.mjs.gz",
            "/.brain_env",
            "/_headers",
            "/_redirects",
            "/_server/private.mjs",
        ] {
            assert!(app.resolve(path).is_none(), "{path}");
        }
        assert!(app.resolve("/.well-known/security.txt").is_some());
        std::fs::remove_dir_all(dist).unwrap();
    }

    #[test]
    fn redirects_file_and_slash_policy_are_one_hop() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        let dist = std::env::temp_dir().join(format!("brain-prod-redir-{}", std::process::id()));
        std::fs::create_dir_all(dist.join("about")).unwrap();
        std::fs::write(dist.join("about/index.html"), "about-page").unwrap();
        let get = |files: &str, path: &str, host: &str| {
            std::fs::write(dist.join("_redirects"), files).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let handler = Static::load(&dist, "/");
            let task = std::thread::spawn(move || handler.handle(listener.accept().unwrap().0).unwrap());
            let mut c = TcpStream::connect(addr).unwrap();
            write!(c, "GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
            let mut r = String::new();
            c.read_to_string(&mut r).unwrap();
            task.join().unwrap();
            r
        };
        let r = get("/old /about 301\n", "/old/?a=1", "localhost");
        assert!(r.starts_with("HTTP/1.1 301") && r.contains("Location: /about?a=1\r\n"), "{r}");
        let r = get("", "/about/", "localhost");
        assert!(r.contains("Location: /about\r\n"), "default trailingSlash never: {r}");
        assert!(get("", "/about", "localhost").ends_with("about-page"));
        let r = get("# brain trailingSlash=always host=https://shop.example\n", "/about", "www.shop.example");
        assert!(r.contains("Location: https://shop.example/about/\r\n"), "{r}");
        assert!(get("# brain trailingSlash=always host=https://shop.example\n", "/about/", "127.0.0.1:8080").ends_with("about-page"));
        std::fs::remove_dir_all(&dist).unwrap();
    }

    #[test]
    fn headers_rules_match_like_a_static_host() {
        let r = parse_headers(
            "/*\n  Content-Security-Policy: default-src 'self'\n  X-Frame-Options: DENY\n/_b/*\n  Cache-Control: public, max-age=31536000, immutable\n/\n  Cache-Control: no-cache\n",
        );
        let page = headers_for(&r, "/");
        assert!(
            page.iter()
                .any(|(k, v)| k == "Content-Security-Policy" && v == "default-src 'self'")
        );
        assert!(
            page.iter()
                .any(|(k, v)| k == "Cache-Control" && v == "no-cache")
        );
        let chunk = headers_for(&r, "/_b/x.js");
        assert!(
            chunk
                .iter()
                .any(|(k, v)| k == "Cache-Control" && v.contains("immutable"))
                && chunk.iter().any(|(k, _)| k == "X-Frame-Options")
        );
        assert!(
            !headers_for(&r, "/about")
                .iter()
                .any(|(k, _)| k == "Cache-Control"),
            "exact rules do not leak"
        );
    }

    #[test]
    fn markdown_twin_on_accept() {
        use std::io::{Read, Write};
        let dist = std::env::temp_dir().join(format!("brain-prod-md-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dist);
        std::fs::create_dir_all(&dist).unwrap();
        for (f, body) in [
            ("index.html", "<h1>Home</h1>"),
            ("index.md", "# Home"),
            ("pricing.html", "<h1>Pricing</h1>"),
            ("pricing.md", "# Pricing"),
            ("about.html", "<h1>About</h1>"),
        ] {
            std::fs::write(dist.join(f), body).unwrap();
        }
        let request = |path: &str, accept: &str| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let handler = Static::load(&dist, "/");
            let task =
                std::thread::spawn(move || handler.handle(listener.accept().unwrap().0).unwrap());
            let mut client = TcpStream::connect(addr).unwrap();
            write!(
                client,
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept: {accept}\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            task.join().unwrap();
            response
        };
        let md = request("/pricing", "text/markdown, text/html;q=0.5");
        assert!(
            md.contains("Content-Type: text/markdown; charset=utf-8")
                && md.contains("Vary: Accept")
                && md.ends_with("# Pricing"),
            "{md}"
        );
        assert!(request("/", "text/markdown").ends_with("# Home"));
        let page = request("/pricing", "text/html,*/*;q=0.8");
        assert!(
            page.ends_with("<h1>Pricing</h1>") && page.contains("Vary: Accept"),
            "{page}"
        );
        // no twin (seo.markdown=false, noindex): HTML, no Vary on Accept
        let about = request("/about", "text/markdown");
        assert!(
            about.ends_with("<h1>About</h1>") && !about.contains("Vary: Accept"),
            "{about}"
        );
        std::fs::remove_dir_all(&dist).unwrap();
    }
}

#[cfg(all(test, unix))]
mod confinement_tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::symlink;
    #[test]
    fn static_handler_rejects_symlink_files_directories_fallback_and_gzip() {
        let root =
            std::env::temp_dir().join(format!("brain-prod-confinement-{}", std::process::id()));
        std::fs::create_dir_all(root.join("dist/about")).unwrap();
        std::fs::create_dir_all(root.join("private")).unwrap();
        let dist = root.join("dist");
        std::fs::write(root.join("private/secret.txt"), "PRIVATE_SENTINEL").unwrap();
        std::fs::write(dist.join("about/index.html"), "public-page").unwrap();
        std::fs::write(dist.join("asset.js"), "public-script").unwrap();
        std::fs::write(dist.join("about/index.html.gz"), "compressed-public").unwrap();
        for name in ["leak.txt", "404.html", "asset.js.gz", "_headers"] {
            symlink(root.join("private/secret.txt"), dist.join(name)).unwrap();
        }
        symlink(root.join("private"), dist.join("linked")).unwrap();
        let request = |path: &str, gzip: bool| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let handler = Static::load(&dist, "/app");
            let task =
                std::thread::spawn(move || handler.handle(listener.accept().unwrap().0).unwrap());
            let mut client = TcpStream::connect(addr).unwrap();
            client
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            write!(
                client,
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: {}\r\n\r\n",
                if gzip { "gzip" } else { "identity" }
            )
            .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            task.join().unwrap();
            response
        };
        for path in ["/app/leak.txt", "/app/linked/secret.txt", "/app/missing"] {
            let response = request(path, true);
            assert!(response.starts_with("HTTP/1.1 404"), "{response}");
            assert!(!response.contains("PRIVATE_SENTINEL"));
        }
        let plain = request("/app/about", false);
        assert!(plain.ends_with("public-page"));
        let compressed = request("/app/about", true);
        assert!(
            compressed.contains("Content-Encoding: gzip")
                && compressed.ends_with("compressed-public")
        );
        std::fs::write(dist.join("about.md"), "# About").unwrap();
        let page = request("/app/about", false);
        assert!(
            page.contains("Vary: Accept, Accept-Encoding") && page.ends_with("public-page"),
            "browsers keep HTML: {page}"
        );
        let safe = request("/app/asset.js", true);
        assert!(safe.ends_with("public-script") && !safe.contains("Content-Encoding: gzip"));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod encoding_tests {
    #[test]
    fn encodings_respect_weight_and_explicit_exclusion() {
        assert_eq!(super::accepted_encodings("gzip, br"), vec!["br", "gzip"]);
        assert_eq!(
            super::accepted_encodings("br;q=0, gzip;q=0.8"),
            vec!["gzip"]
        );
        assert_eq!(super::accepted_encodings("br;q=0, *;q=1"), vec!["gzip"]);
        assert_eq!(
            super::accepted_encodings("gzip;q=1, br;q=0.2"),
            vec!["gzip", "br"]
        );
        assert!(super::accepted_encodings("xgzip, br;q=invalid").is_empty());
    }
    #[test]
    fn optional_native_proxy_preserves_static_cache_head_and_base_namespace() {
        use super::Static;
        use std::{
            io::{Read, Write},
            net::{TcpListener, TcpStream},
            time::Duration,
        };
        let dist = std::env::temp_dir().join(format!("brain-prod-native-{}", std::process::id()));
        std::fs::create_dir_all(dist.join("_b")).unwrap();
        std::fs::write(dist.join("index.html"), "<h1>Home</h1>").unwrap();
        std::fs::write(dist.join("_b/asset.js"), "export const cached=1").unwrap();
        std::fs::write(
            dist.join("_headers"),
            "/app/_b/*\n  Cache-Control: public, max-age=31536000, immutable\n",
        )
        .unwrap();
        let authority = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = format!("http://{}", authority.local_addr().unwrap());
        let backend = std::thread::spawn(move || {
            let (mut socket, _) = authority.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let req = crate::http::read_request(socket.try_clone().unwrap()).unwrap();
            assert_eq!(req.path, "/s/short");
            crate::http::respond_with(
                &mut socket,
                302,
                "text/plain",
                b"",
                false,
                &[("Location", "https://example.test/destination")],
            )
            .unwrap();
        });
        let request = |method: &str, path: &str| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let handler = Static::load(&dist, "/app");
            let target = target.clone();
            let task = std::thread::spawn(move || {
                handler
                    .handle_with_backend(listener.accept().unwrap().0, Some(&target))
                    .unwrap()
            });
            let mut client = TcpStream::connect(address).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(
                client,
                "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            task.join().unwrap();
            response
        };
        let proxied = request("GET", "/app/s/short");
        assert!(proxied.starts_with("HTTP/1.1 302"));
        assert!(proxied.contains("https://example.test/destination"));
        backend.join().unwrap();
        let cached = request("GET", "/app/_b/asset.js");
        assert!(cached.starts_with("HTTP/1.1 200"));
        assert!(cached.contains("max-age=31536000, immutable"));
        assert!(cached.ends_with("export const cached=1"));
        let head = request("HEAD", "/app/");
        assert!(head.starts_with("HTTP/1.1 200"));
        // with a native backend the page gains the collector tag at response time; the built file is unchanged
        let served = super::with_analytics("<h1>Home</h1>", "/app");
        assert!(head.contains(&format!("Content-Length: {}", served.len())));
        let page = request("GET", "/app/");
        assert!(page.ends_with("<h1>Home</h1><script type=\"module\" async src=\"/app/_brain/analytics.js\"></script>"));
        assert_eq!(std::fs::read_to_string(dist.join("index.html")).unwrap(), "<h1>Home</h1>");
        assert_eq!(super::with_analytics(&served, "/app"), served);
        assert_eq!(
            super::with_analytics("<html><head><title>T</title></head><body></body></html>", ""),
            "<html><head><title>T</title><script type=\"module\" async src=\"/_brain/analytics.js\"></script></head><body></body></html>"
        );
        assert!(head.ends_with("\r\n\r\n"));
        assert!(request("GET", "/s/short").starts_with("HTTP/1.1 404"));
        std::fs::remove_dir_all(dist).unwrap();
    }
}
