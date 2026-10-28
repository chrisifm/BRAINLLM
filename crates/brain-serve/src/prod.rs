//! Static production server for `brain start` (INTERNALS §34): serves `dist/` from Rust, no Node. Precompressed
//! `.gz` when the client accepts it, everything under brain.json `base` (#148), clean URLs (`/about` → `about/index.html`), `404.html`, the headers of
//! `dist/_headers` applied exactly as a static host would (`/*` and `/_b/*` are prefix rules), a strong `ETag` equal to
//! the build signature on HTML (304 on `If-None-Match`) and `GET /_health` → `{"ok":true,"build":"<sig>"}`.

use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
    if !std::fs::symlink_metadata(dist).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) { return false; }
    let Ok(relative) = file.strip_prefix(dist) else { return false };
    let mut cursor = dist.to_path_buf();
    for part in relative.components() {
        if !matches!(part, Component::Normal(_)) { return false; }
        cursor.push(part);
        let Ok(meta) = std::fs::symlink_metadata(&cursor) else { return false };
        if meta.file_type().is_symlink() { return false; }
    }
    file.is_file()
}

/// The build signature (`<meta name="brain-build">` of the home page, else of the 404 page; `.html` or `.brain`).
pub fn signature(dist: &Path) -> Option<String> {
    for f in ["index.html", "404.html", "index.brain", "404.brain"] {
        if !regular_output(dist, &dist.join(f)) { continue; }
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
}

impl Static {
    pub fn load(dist: &Path, base: &str) -> Static {
        Static {
            pre: base.trim_end_matches('/').to_string(),
            dist: dist.to_path_buf(),
            rules: if regular_output(dist, &dist.join("_headers")) { std::fs::read_to_string(dist.join("_headers")).map(|s| parse_headers(&s)).unwrap_or_default() } else { Vec::new() },
            build: signature(dist).unwrap_or_default(),
        }
    }

    /// `/about` → `about/index.html`, `about.html` or `about`; None outside dist/ or for host config files.
    fn resolve(&self, url_path: &str) -> Option<PathBuf> {
        let rel = url_path.trim_start_matches('/');
        if Path::new(rel).components().any(|c| !matches!(c, Component::Normal(_))) || rel.starts_with("_server") || rel.starts_with("_brain") || rel == "_headers" || rel == "server.mjs" {
            return None;
        }
        let base = self.dist.join(rel);
        // pages are `index.html` (ext web) or `index.brain` (ext brain, #26)
        let cands = if rel.is_empty() || rel.ends_with('/') {
            vec![base.join("index.html"), base.join("index.brain")]
        } else {
            vec![base.clone(), base.join("index.html"), base.join("index.brain"), self.dist.join(format!("{rel}.html")), self.dist.join(format!("{rel}.brain"))]
        };
        cands.into_iter().find(|p| regular_output(&self.dist, p))
    }

    /// Serves one connection (one request; `Connection: close`).
    pub fn handle(&self, mut s: TcpStream) -> io::Result<()> {
        let req = crate::http::read_request(&s)?;
        let path = req.path.clone();
        if req.method != "GET" && req.method != "HEAD" {
            return write(&mut s, 405, &[("Allow".into(), "GET, HEAD".into())], b"", false);
        }
        let head = req.method == "HEAD";
        if path == "/_health" {
            let body = format!("{{\"ok\":true,\"build\":{}}}", crate::json_str(&self.build));
            return write(&mut s, 200, &[("Content-Type".into(), "application/json".into()), ("Cache-Control".into(), "no-store".into())], body.as_bytes(), head);
        }
        // base (#148): `/app` → `/app/`; files resolve without it; outside it → 404 (`/_health` stays at the root)
        let inner = if self.pre.is_empty() {
            Some(path.clone())
        } else if path == self.pre {
            return write(&mut s, 301, &[("Location".into(), format!("{}/", self.pre))], b"", head);
        } else {
            path.strip_prefix(&self.pre).filter(|r| r.starts_with('/')).map(str::to_string)
        };
        let (status, file) = match inner.as_deref().and_then(|p| self.resolve(p)) {
            Some(f) => (200, f),
            None => match ["404.html", "404.brain"].iter().map(|f| self.dist.join(f)).find(|f| regular_output(&self.dist, f)) {
                Some(f) => (404, f),
                None => return write(&mut s, 404, &[("Content-Type".into(), "text/plain".into())], b"not found", head),
            },
        };
        let name = file.to_string_lossy().to_string();
        // in a build, `.brain` is a page and `.jsbrain`/`.cssbrain` its script and styles (in dev they are sources)
        let html = name.ends_with(".html") || name.ends_with(".brain");
        let mut hs = headers_for(&self.rules, &path);
        hs.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
        hs.push(("Content-Type".into(), built_mime(&name).into()));
        if html && !self.build.is_empty() {
            // the page's validator is the build signature: unchanged build → 304 with no body
            let tag = format!("\"{}\"", self.build);
            hs.retain(|(k, _)| !k.eq_ignore_ascii_case("x-brain-build"));
            hs.push(("X-Brain-Build".into(), self.build.clone()));
            hs.push(("ETag".into(), tag.clone()));
            if status == 200 && req.header("if-none-match").is_some_and(|v| v.split(',').any(|t| t.trim() == tag)) {
                return write(&mut s, 304, &hs, b"", true);
            }
        }
        let gz = PathBuf::from(format!("{name}.gz"));
        let accepts = req.header("accept-encoding").is_some_and(|a| a.contains("gzip"));
        let has_gzip = regular_output(&self.dist, &gz);
        let body = if accepts && has_gzip {
            hs.push(("Content-Encoding".into(), "gzip".into()));
            std::fs::read(&gz)?
        } else {
            std::fs::read(&file)?
        };
        if !hs.iter().any(|(k, _)| k.eq_ignore_ascii_case("vary")) && has_gzip {
            hs.push(("Vary".into(), "Accept-Encoding".into()));
        }
        write(&mut s, status, &hs, &body, head)
    }
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

fn write(s: &mut TcpStream, status: u16, headers: &[(String, String)], body: &[u8], head: bool) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        301 => "Moved Permanently",
        304 => "Not Modified",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "",
    };
    let mut h = format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (k, v) in headers {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str("\r\n");
    s.write_all(h.as_bytes())?;
    if !head {
        s.write_all(body)?;
    }
    s.flush()
}

/// Serves `dist/` on `listener` until `stop` is set; one thread per connection.
pub fn serve_static(listener: TcpListener, dist: &Path, base: &str, stop: Arc<AtomicBool>) -> io::Result<()> {
    let st = Arc::new(Static::load(dist, base));
    listener.set_nonblocking(true)?;
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((s, _)) => {
                let _ = s.set_nonblocking(false);
                let st = st.clone();
                std::thread::spawn(move || {
                    let _ = st.handle(s);
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_rules_match_like_a_static_host() {
        let r = parse_headers("/*\n  Content-Security-Policy: default-src 'self'\n  X-Frame-Options: DENY\n/_b/*\n  Cache-Control: public, max-age=31536000, immutable\n/\n  Cache-Control: no-cache\n");
        let page = headers_for(&r, "/");
        assert!(page.iter().any(|(k, v)| k == "Content-Security-Policy" && v == "default-src 'self'"));
        assert!(page.iter().any(|(k, v)| k == "Cache-Control" && v == "no-cache"));
        let chunk = headers_for(&r, "/_b/x.js");
        assert!(chunk.iter().any(|(k, v)| k == "Cache-Control" && v.contains("immutable")) && chunk.iter().any(|(k, _)| k == "X-Frame-Options"));
        assert!(!headers_for(&r, "/about").iter().any(|(k, _)| k == "Cache-Control"), "exact rules do not leak");
    }
}

#[cfg(all(test, unix))]
mod confinement_tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::symlink;
    #[test]
    fn static_handler_rejects_symlink_files_directories_fallback_and_gzip() {
        let root = std::env::temp_dir().join(format!("brain-prod-confinement-{}", std::process::id()));
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
            let task = std::thread::spawn(move || handler.handle(listener.accept().unwrap().0).unwrap());
            let mut client = TcpStream::connect(addr).unwrap();
            client.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
            write!(client, "GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: {}\r\n\r\n", if gzip {"gzip"} else {"identity"}).unwrap();
            let mut response = String::new();client.read_to_string(&mut response).unwrap();task.join().unwrap();response
        };
        for path in ["/app/leak.txt", "/app/linked/secret.txt", "/app/missing"] {
            let response = request(path, true);
            assert!(response.starts_with("HTTP/1.1 404"), "{response}");
            assert!(!response.contains("PRIVATE_SENTINEL"));
        }
        let plain = request("/app/about", false);assert!(plain.ends_with("public-page"));
        let compressed = request("/app/about", true);
        assert!(compressed.contains("Content-Encoding: gzip") && compressed.ends_with("compressed-public"));
        let safe = request("/app/asset.js", true);
        assert!(safe.ends_with("public-script") && !safe.contains("Content-Encoding: gzip"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
