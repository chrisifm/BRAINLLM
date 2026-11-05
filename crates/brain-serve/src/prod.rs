//! Static production server for `brain start` (INTERNALS §34): serves `dist/` from Rust, no Node. The smallest
//! precompressed `.br`/`.zst`/`.gz` the client accepts, everything under brain.json `base` (#148), clean URLs (`/about` → `about/index.html`), `404.html`, the headers of
//! `dist/_headers` applied exactly as a static host would (`/*` and `/_b/*` are prefix rules), a strong `ETag` equal to
//! the build signature on HTML (304 on `If-None-Match`) and `GET /_health` → `{"ok":true,"build":"<sig>"}`.

use crate::server::native_proxy;
use std::io::{self, Read, Seek, SeekFrom, Write};
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
/// The HTTP parser already decoded the request path. Decode each pattern after identifying its wildcard;
/// an encoded literal star therefore never becomes a wildcard, and literal percent filenames remain distinct.
pub fn headers_for(rules: &Rules, path: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (pat, hs) in rules {
        let hit = match pat.strip_suffix('*') {
            Some(prefix) if prefix.contains('%') => path.starts_with(&crate::http::percent_decode(prefix)),
            Some(prefix) => path.starts_with(prefix),
            None if pat.contains('%') => crate::http::percent_decode(pat)==path,
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

fn exact_rule_path(pattern:&str,path:&str)->bool {
    if pattern.contains('%') {crate::http::percent_decode(pattern)==path} else {pattern==path}
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
            || brain_pack::private::is_private(rel)
            || rel.contains('\\')
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
        self.handle_native(s, backend, None, &Default::default())
    }
    fn handle_native(
        &self,
        mut s: TcpStream,
        backend: Option<&str>,
        sockets: Option<&crate::NativeSocketProxy>,
        routes: &native_proxy::NativeRoutes,
    ) -> io::Result<()> {
        let target = backend
            .filter(|s| !s.is_empty())
            .map(native_proxy::Target::parse)
            .transpose()
            .map_err(io::Error::other)?;
        let upload = target
            .as_ref()
            .map(|_| format!("{}/_brain/uploads", self.pre));
        let bound = target.is_some();
        let plan = |path: &str, method: &str| {
            if bound { native_proxy::plan(path, method, routes, &self.pre) } else { crate::http::BodyPlan::Default }
        };
        let mut req = match native_proxy::read_request(&s, upload.as_deref(), &plan) {
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
        let probe_target = target.clone();
        if let (Some(target), Some(native_path)) = (target, native_path) {
            if native_proxy::handles(native_path) && native_proxy::probe_alias(native_path).is_none() {
                req.path = native_path.to_string();
                return if req.upgrade.stream_body.is_some() {
                    native_proxy::serve_files(&mut s, &target, &req)
                } else {
                    native_proxy::serve(&mut s, &target, &req, None)
                };
            }
            if let Some(route) = routes.find(native_path) {
                req.path = native_path.to_string();
                return native_proxy::serve(&mut s, &target, &req, Some(route));
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
        // One URL per page, as server.mjs: `/a//b` and `/about/index.html` answer one 301, through the rules below so the
        // slash policy and the canonical host land in the same hop. A leading `//` never gets here: the parser answers 400.
        if let Some(clean) = url_variant(&path, &self.pre) {
            let q = if req.query.is_empty() { String::new() } else { format!("?{}", req.query) };
            let inner = clean.strip_prefix(&self.pre).filter(|r| r.starts_with('/')).unwrap_or(&clean);
            let loc = match brain_seo::redirects::resolve(&self.redirects.0, &self.redirects.1, inner, &q, req.header("host")) {
                Some((loc, _)) if loc.starts_with('/') => format!("{}{loc}", self.pre),
                Some((loc, _)) => loc,
                None => format!("{clean}{q}"),
            };
            return write(&mut s, 301, &[("Location".into(), loc), ("Cache-Control".into(), "public, max-age=3600".into())], b"", head);
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
        // browsers still ask for /favicon.ico: the app's SVG icon answers it
        if inner.as_deref() == Some("/favicon.ico") && self.resolve("/favicon.ico").is_none() && self.resolve("/favicon.svg").is_some() {
            let location = format!("{}/favicon.svg", self.pre);
            return write(&mut s, 301, &[("Location".into(), location), ("Cache-Control".into(), "public, max-age=86400".into())], b"", head);
        }
        // `/health` is reserved for the backend (B0735), as in server.mjs: an app file at that path wins, otherwise the
        // backend answers, or a plain healthy response without one. It also answers at the root under a base.
        let probe_path = inner.as_deref().unwrap_or(&path);
        let probe_path = native_proxy::probe_alias(probe_path).unwrap_or(probe_path);
        if native_proxy::is_probe(probe_path) && inner.as_deref().is_none_or(|p| self.resolve(p).is_none()) {
            let Some(target) = probe_target else {
                return write(&mut s, 200, &[("Content-Type".into(), "application/json".into()), ("Cache-Control".into(), "no-store".into())], b"{\"ok\":true}", head);
            };
            req.path = probe_path.to_string();
            return native_proxy::serve(&mut s, &target, &req, None);
        }
        let (status, file) = match inner.as_deref().and_then(|p| self.resolve(p).map(|f| (p, f))) {
            // a status page opened directly answers its status, as server.mjs does
            Some((p, f)) => (reserved_status(p).unwrap_or(200), f),
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
        if html && (status>=400 || !hs.iter().any(|(key,_)|key.eq_ignore_ascii_case("Content-Security-Policy")) || brain_pack::cache::SECURITY_DOC.iter().filter(|(required,_)|!required.eq_ignore_ascii_case("X-Generated-By")).any(|(required,_)|!hs.iter().any(|(key,_)|key.eq_ignore_ascii_case(required)))) {
            let document=self.document_headers(&file);
            if status>=400 {hs=document;}
            else {
                for (key,value) in document {
                    if !hs.iter().any(|(existing,_)|existing.eq_ignore_ascii_case(&key)) {hs.push((key,value));}
                }
            }
        }
        // A Markdown copy names the canonical its page declares; no canonical in the page, no header (never an invented URL).
        let markdown_of = if name.ends_with(".md") {
            inner.as_deref().map(|p| p.trim_end_matches(".md")).and_then(|p| self.resolve(if p.is_empty() || p == "/index" { "/" } else { p }))
        } else {
            None
        };
        if let Some(link) = markdown_of.and_then(|page| std::fs::read_to_string(page).ok()).and_then(|page| canonical_link(&page)) {
            hs.push(("Link".into(), link));
        }
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
        if status == 200 && !html {
            return serve_asset(&mut s, &req, hs, &file, head);
        }
        let variants = [
            ("br", PathBuf::from(format!("{name}.br"))),
            ("zstd", PathBuf::from(format!("{name}.zst"))),
            ("gzip", PathBuf::from(format!("{name}.gz"))),
        ];
        // the smallest precompressed sibling the client accepts (docs/PERFORMANCE.md)
        let selected = accepted_encodings(req.header("accept-encoding").unwrap_or(""))
            .into_iter()
            .filter_map(|enc| {
                variants
                    .iter()
                    .find(|(name, path)| *name == enc && regular_output(&self.dist, path))
            })
            .min_by_key(|(_, path)| std::fs::metadata(path).map_or(u64::MAX, |m| m.len()));
        let selected_file = if let Some((encoding, path)) = selected {
            hs.push(("Content-Encoding".into(), (*encoding).into()));
            path
        } else {
            if status == 200 && !html {
                hs.push(("Accept-Ranges".into(), "bytes".into()));
            }
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

    fn document_headers(&self,file:&Path)->Vec<(String,String)> {
        let Ok(relative)=file.strip_prefix(&self.dist) else {return Vec::new()};
        let relative=relative.to_string_lossy().replace('\\',"/");
        let actual=format!("{}/{}",self.pre,relative);
        let mut candidates=vec![actual.clone()];
        if let Some(stem)=relative.strip_suffix(".brain") {candidates.push(format!("{}/{stem}.html",self.pre));}
        for suffix in ["index.html","index.brain"] {
            if relative==suffix {candidates.push(format!("{}/",self.pre));}
            else if let Some(directory)=relative.strip_suffix(&format!("/{suffix}")) {candidates.push(format!("{}/{directory}/",self.pre));}
        }
        if let Some(stem)=relative.strip_suffix(".html").or_else(||relative.strip_suffix(".brain")) {candidates.push(format!("{}/{stem}",self.pre));}
        // Prefer an explicit policy of the selected file, with canonical aliases for builds made by older compilers.
        let identity=candidates.iter().find(|candidate| {
            let explicit=self.rules.iter().any(|(pattern,headers)|!pattern.ends_with('*') && exact_rule_path(pattern,candidate) && headers.iter().any(|(key,_)|key.eq_ignore_ascii_case("Content-Security-Policy")));
            if !explicit {return false;}
            if **candidate==actual {return true;}
            let Some(inner)=candidate.strip_prefix(&self.pre).filter(|inner|inner.starts_with('/')) else {return false;};
            let selected=self.resolve(inner);
            selected.as_deref()==Some(file) || (selected.is_none() && matches!(relative.as_str(),"404.brain"|"_offline.brain") && **candidate==format!("{}/{}.html",self.pre,relative.trim_end_matches(".brain")))
        }).unwrap_or(&actual);
        let mut headers=headers_for(&self.rules,identity);
        // A file's exact partial override (for example its own-extension MIME or opener policy) stays authoritative.
        for (_,fields) in self.rules.iter().filter(|(pattern,_)|!pattern.ends_with('*') && exact_rule_path(pattern,&actual)) {
            for (key,value) in fields {headers.retain(|(existing,_)|!existing.eq_ignore_ascii_case(key));headers.push((key.clone(),value.clone()));}
        }
        headers
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

/// The one clean path of a variant (`/a//b`, `/about/index.html`), or None when the path already is it.
fn url_variant(path: &str, pre: &str) -> Option<String> {
    let inside = pre.is_empty() || path.starts_with(&format!("{pre}/"));
    if !inside || (!path.contains("//") && !path.ends_with("/index.html")) {
        return None;
    }
    let mut clean = String::with_capacity(path.len());
    for c in path.chars() {
        if !(c == '/' && clean.ends_with('/')) {
            clean.push(c);
        }
    }
    if let Some(dir) = clean.strip_suffix("index.html") {
        clean = dir.to_string();
    }
    let root = format!("{pre}/");
    if clean != root && clean.ends_with('/') && path.ends_with("//") {
        clean.pop();
    }
    if clean != root && path.ends_with("/index.html") {
        clean = clean.trim_end_matches('/').to_string();
    }
    (clean != path).then_some(clean)
}

/// `Link: <url>; rel="canonical"` from the page's own `<link rel="canonical">`, when it is absolute and not a route pattern.
fn canonical_link(page: &str) -> Option<String> {
    let at = page.find("rel=\"canonical\"")?;
    let start = page[..at].rfind('<')?;
    let tag = &page[start..start + page[start..].find('>')?];
    let href = tag.split("href=\"").nth(1)?.split('"').next()?.replace("&amp;", "&");
    let rest = href.strip_prefix("https://").or_else(|| href.strip_prefix("http://"))?;
    let invalid = rest.is_empty() || href.chars().any(|c| c.is_control() || c.is_whitespace() || c == '<' || c == '>') || rest.contains("/:") || rest.contains("/*");
    (!invalid).then(|| format!("<{href}>; rel=\"canonical\""))
}

/// `/403`, `/404.html`, `/503`…: the status of a page named after one (a file is only served when it exists).
fn reserved_status(path: &str) -> Option<u16> {
    let path=path.strip_prefix('/')?;
    let code: u16 = path.strip_suffix(".html").or_else(||path.strip_suffix(".brain")).unwrap_or(path).parse().ok()?;
    matches!(code, 403 | 404 | 410 | 500..=599).then_some(code)
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
    let mut encodings = vec![("br", quality("br")), ("zstd", quality("zstd")), ("gzip", quality("gzip"))];
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
/// One `bytes=` span of a file of `size` bytes (RFC 9110 §14): `Some(Ok((start, end)))` inclusive, `Some(Err(()))`
/// when it cannot be satisfied (416), `None` when the header is not one well-formed span (several, another unit,
/// reversed): the whole file is sent then, which the RFC allows.
pub fn byte_range(header: &str, size: u64) -> Option<Result<(u64, u64), ()>> {
    let (a, b) = header.trim().strip_prefix("bytes=")?.split_once('-')?;
    let num = |v: &str| if !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit()) { v.parse::<u64>().ok() } else { None };
    if a.is_empty() {
        let n = num(b)?;
        return Some(if n > 0 && size > 0 { Ok((size.saturating_sub(n), size - 1)) } else { Err(()) });
    }
    let start = num(a)?;
    let end = if b.is_empty() { None } else { Some(num(b)?) };
    if end.is_some_and(|e| e < start) {
        return None;
    }
    Some(if start < size { Ok((start, end.map_or(size - 1, |e| e.min(size - 1)))) } else { Err(()) })
}

fn asset_stamp(path: &Path, metadata: &std::fs::Metadata) -> String {
    let stamp = format!("{path:?}:{}:{:?}:{:?}", metadata.len(), metadata.modified(), metadata.created());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return format!("{stamp}:{}:{}:{}:{}", metadata.dev(), metadata.ino(), metadata.ctime(), metadata.ctime_nsec());
    }
    #[cfg(not(unix))]
    stamp
}

fn asset_tag(path: &Path, file: &mut std::fs::File) -> io::Result<String> {
    static TAGS: std::sync::OnceLock<std::sync::Mutex<Vec<(String, String)>>> = std::sync::OnceLock::new();
    let tags = TAGS.get_or_init(Default::default);
    let stamp = asset_stamp(path, &file.metadata()?);
    if let Some((_, tag)) = tags.lock().map_err(|_| io::Error::other("static validator cache unavailable"))?.iter().find(|(key, _)| key == &stamp) {
        return Ok(tag.clone());
    }
    struct Reader<'a> { file: std::io::BufReader<&'a mut std::fs::File>, end: Instant }
    impl Read for Reader<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if Instant::now() > self.end { return Err(io::Error::new(io::ErrorKind::TimedOut, "static validator deadline exceeded")); }
            self.file.read(buffer)
        }
    }
    let hash = brain_pack::sha::sha256_reader(Reader { file: std::io::BufReader::with_capacity(65536, &mut *file), end: Instant::now() + Duration::from_secs(30) })?;
    if asset_stamp(path, &file.metadata()?) != stamp { return Err(io::Error::other("static file changed while computing its validator")); }
    let tag = format!("\"{}\"", brain_pack::sha::base64(&hash));
    let mut cache = tags.lock().map_err(|_| io::Error::other("static validator cache unavailable"))?;
    cache.push((stamp, tag.clone()));
    if cache.len() > 256 { cache.remove(0); }
    Ok(tag)
}

fn serve_asset(s: &mut TcpStream, req: &crate::http::Request, mut hs: Vec<(String, String)>, path: &Path, head: bool) -> io::Result<()> {
    let mut selected = path.to_path_buf();
    // Byte ranges and If-Range always describe the identity representation.
    if req.header("range").is_none() {
        let variants: Vec<_> = accepted_encodings(req.header("accept-encoding").unwrap_or("")).into_iter().filter_map(|enc| {
            let suffix = match enc { "br" => "br", "zstd" => "zst", _ => "gz" };
            let candidate = PathBuf::from(format!("{}.{suffix}", path.display()));
            let meta = std::fs::symlink_metadata(&candidate).ok()?;
            meta.is_file().then_some((enc, candidate, meta.len()))
        }).collect();
        if let Some((enc, candidate, _)) = variants.into_iter().min_by_key(|(_, _, size)| *size) {
            selected = candidate;
            hs.push(("Content-Encoding".into(), enc.into()));
        }
    }
    if let Some((_, vary)) = hs.iter_mut().find(|(key, _)| key.eq_ignore_ascii_case("vary")) {
        if !vary.split(',').any(|v| v.trim().eq_ignore_ascii_case("accept-encoding")) { vary.push_str(", Accept-Encoding"); }
    } else { hs.push(("Vary".into(), "Accept-Encoding".into())); }
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "m3u8" | "m3u" | "mpd")) {
        hs.retain(|(key, _)| !key.eq_ignore_ascii_case("cache-control"));
        hs.push(("Cache-Control".into(), "no-cache".into()));
    }
    let mut file = open_output(&selected)?;
    let tag = asset_tag(&selected, &mut file)?;
    let size = file.metadata()?.len();
    hs.retain(|(key, _)| !key.eq_ignore_ascii_case("etag"));
    hs.push(("ETag".into(), tag.clone()));
    if selected == path { hs.push(("Accept-Ranges".into(), "bytes".into())); }
    if req.header("if-none-match").is_some_and(|v| v.split(',').any(|v| v.trim() == "*" || v.trim().trim_start_matches("W/") == tag)) {
        return write(s, 304, &hs, b"", true);
    }
    match req.header("range").filter(|_| req.header("if-range").is_none_or(|v| v == tag)).and_then(|v| byte_range(v, size)) {
        Some(Err(())) => write(s, 416, &[("Content-Range".into(), format!("bytes */{size}")), ("Cache-Control".into(), "no-store".into())], b"", head),
        Some(Ok((start, end))) => {
            hs.push(("Content-Range".into(), format!("bytes {start}-{end}/{size}")));
            write_open_file_span(s, 206, &hs, file, Some((start, end - start + 1)), head)
        }
        None => write_open_file_span(s, 200, &hs, file, None, head),
    }
}

fn write_file(
    s: &mut TcpStream,
    status: u16,
    headers: &[(String, String)],
    path: &Path,
    head: bool,
) -> io::Result<()> {
    write_file_span(s, status, headers, path, None, head)
}

/// `write_file` of `span` = (start, length) only, or of the whole file.
fn write_file_span(
    s: &mut TcpStream,
    status: u16,
    headers: &[(String, String)],
    path: &Path,
    span: Option<(u64, u64)>,
    head: bool,
) -> io::Result<()> {
    write_open_file_span(s, status, headers, open_output(path)?, span, head)
}

fn open_output(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() { return Err(io::Error::other("static output must be regular")); }
    Ok(file)
}

fn write_open_file_span(
    s: &mut TcpStream,
    status: u16,
    headers: &[(String, String)],
    file: std::fs::File,
    span: Option<(u64, u64)>,
    head: bool,
) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("static output must be regular"));
    }
    let mut writer = TimedWriter {
        stream: s,
        end: Instant::now() + Duration::from_secs(10),
    };
    let (start, length) = span.unwrap_or((0, metadata.len()));
    if start.saturating_add(length) > metadata.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "static output changed during transfer"));
    }
    write_headers(&mut writer, status, headers, length)?;
    if !head {
        let mut file = file;
        file.seek(SeekFrom::Start(start))?;
        let copied = io::copy(&mut file.take(length), &mut writer)?;
        if copied != length {
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
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        304 => "Not Modified",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        _ => "",
    };
    // A 304 has no body; its length would describe the stored 200, never an empty representation.
    let mut h = format!("HTTP/1.1 {status} {reason}\r\n");
    if status != 304 { h.push_str(&format!("Content-Length: {length}\r\n")); }
    h.push_str("Connection: close\r\n");
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
    files: std::sync::Mutex<OutputSnapshot>,
    backend: Option<String>,
    sockets: Option<crate::NativeSocketProxy>,
    routes: native_proxy::NativeRoutes,
}
// Refresh cached policy/validators after a rebuild, without parsing host configuration on every request.
// Request handlers clone the snapshot and release the lock before reading bodies or doing network I/O.
struct OutputSnapshot {
    files: Arc<Static>,
    stamp: OutputStamp,
}
type OutputStamp = Vec<Option<(Option<std::time::SystemTime>, u64, u64)>>;
fn output_stamp(dist: &Path) -> OutputStamp {
    ["", "index.html", "index.brain", "404.html", "404.brain", "_headers", "_redirects"].iter().map(|path| {
        let metadata = std::fs::symlink_metadata(dist.join(path)).ok()?;
        #[cfg(unix)]
        let identity = { use std::os::unix::fs::MetadataExt; metadata.ino() };
        #[cfg(not(unix))]
        let identity = 0;
        Some((metadata.modified().ok(), metadata.len(), identity))
    }).collect()
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
        let mut routes = native_proxy::NativeRoutes::default();
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
                #[serde(default, rename = "nativeRoutes")]
                native_routes: Vec<native_proxy::NativeRoute>,
            }
            let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| {
                io::Error::other("Invalid static socket manifest JSON or field type")
            })?;
            routes = native_proxy::NativeRoutes::new(manifest.native_routes);
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
        // Stamp first: a concurrent rebuild can then only trigger another refresh, never pin stale policy to a newer stamp.
        let stamp = output_stamp(dist);
        Ok(Self {
            files: std::sync::Mutex::new(OutputSnapshot { files: Arc::new(Static::load(dist, base)), stamp }),
            backend,
            sockets,
            routes,
        })
    }
    pub fn handle(&self, stream: TcpStream) -> io::Result<()> {
        let files = {
            let mut state = self.files.lock().map_err(|_| io::Error::other("static output snapshot lock failed"))?;
            let stamp = output_stamp(&state.files.dist);
            if stamp != state.stamp {
                state.files = Arc::new(Static::load(&state.files.dist, &format!("{}/", state.files.pre)));
                state.stamp = stamp;
            }
            state.files.clone()
        };
        files.handle_native(stream, self.backend.as_deref(), self.sockets.as_ref(), &self.routes)
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
    fn running_static_gateway_recovers_from_recreated_dist_and_refreshes_validators_and_headers() {
        let dist = std::env::temp_dir().join(format!("brain-static-recreated-dist-{}", std::process::id()));
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "<meta name=\"brain-build\" content=\"first\"><h1>First</h1>").unwrap();
        std::fs::write(dist.join("_headers"), "/*\n  X-Example-Revision: first\n").unwrap();
        let gateway = StaticGateway::load(&dist, "/", None).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..3 { gateway.handle(listener.accept().unwrap().0).unwrap(); }
        });
        let get = |extra: &str| {
            let mut socket = TcpStream::connect(address).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            socket.write_all(format!("GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{extra}\r\n").as_bytes()).unwrap();
            let mut response = String::new(); socket.read_to_string(&mut response).unwrap(); response
        };
        let first = get("");
        std::fs::remove_dir_all(&dist).unwrap();
        let absent = get("");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "<meta name=\"brain-build\" content=\"second\"><h1>Second</h1>").unwrap();
        std::fs::write(dist.join("_headers"), "/*\n  X-Example-Revision: second\n").unwrap();
        let rebuilt = get("If-None-Match: \"first\"\r\n");
        server.join().unwrap();
        std::fs::remove_dir_all(dist).unwrap();
        assert!(first.starts_with("HTTP/1.1 200") && first.contains("<h1>First</h1>"), "{first}");
        assert!(absent.starts_with("HTTP/1.1 404"), "{absent}");
        assert!(rebuilt.starts_with("HTTP/1.1 200") && rebuilt.contains("<h1>Second</h1>"), "{rebuilt}");
        assert!(rebuilt.contains("X-Example-Revision: second\r\n") && rebuilt.contains("ETag: \"second\"\r\n"), "{rebuilt}");
    }

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
            "/_SERVER/private.mjs",
            "/Server.mjs",
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
    fn status_pages_answer_their_status_and_the_svg_icon_answers_favicon_ico() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        let dist = std::env::temp_dir().join(format!("brain-prod-status-{}", std::process::id()));
        std::fs::create_dir_all(&dist).unwrap();
        for f in ["403.html", "404.html", "503.html", "index.html", "2025.html"] {
            std::fs::write(dist.join(f), f).unwrap();
        }
        let get = |path: &str| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let handler = Static::load(&dist, "/");
            let task = std::thread::spawn(move || handler.handle(listener.accept().unwrap().0).unwrap());
            let mut c = TcpStream::connect(addr).unwrap();
            write!(c, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut r = String::new();
            c.read_to_string(&mut r).unwrap();
            task.join().unwrap();
            r
        };
        for (path, status) in [("/403", "403"), ("/403.html", "403"), ("/404", "404"), ("/503", "503"), ("/2025", "200"), ("/", "200")] {
            let r = get(path);
            assert!(r.starts_with(&format!("HTTP/1.1 {status}")), "{path}: {r}");
        }
        assert!(get("/favicon.ico").starts_with("HTTP/1.1 404"), "no icon, no redirect");
        std::fs::write(dist.join("favicon.svg"), "<svg/>").unwrap();
        let r = get("/favicon.ico");
        assert!(r.starts_with("HTTP/1.1 301") && r.contains("Location: /favicon.svg\r\n"), "{r}");
        std::fs::write(dist.join("favicon.ico"), "ico").unwrap();
        assert!(get("/favicon.ico").starts_with("HTTP/1.1 200"), "a real .ico wins");
        std::fs::remove_dir_all(&dist).unwrap();
    }

    #[test]
    fn encoded_document_patterns_preserve_literal_percent_and_star() {
        let rules=parse_headers("/*\n  X-Global: yes\n/rate%25\n  X-Exact: percent\n/sample-%C3%A9\n  X-Exact: unicode\n/literal%2A\n  X-Exact: star\n/assets%20dir/*\n  X-Prefix: yes\n");
        for (path,value) in [("/rate%","percent"),("/sample-\u{e9}","unicode"),("/literal*","star")] {
            assert!(headers_for(&rules,path).iter().any(|(k,v)|k=="X-Exact" && v==value));
        }
        assert!(!headers_for(&rules,"/literal-other").iter().any(|(k,_)|k=="X-Exact"));
        assert!(!headers_for(&rules,"/rate%25").iter().any(|(k,_)|k=="X-Exact"));
        assert!(headers_for(&rules,"/assets dir/a.js").iter().any(|(k,_)|k=="X-Prefix"));
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
    fn url_variants_answer_one_redirect_and_twins_name_the_page_canonical() {
        let dist = std::env::temp_dir().join(format!("brain-prod-variants-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dist);
        std::fs::create_dir_all(dist.join("about")).unwrap();
        std::fs::write(dist.join("index.html"), "<h1>Home</h1>").unwrap();
        std::fs::write(dist.join("about/index.html"), "<head><link rel=\"canonical\" href=\"https://site.example/about\"></head>").unwrap();
        std::fs::write(dist.join("about.md"), "# About").unwrap();
        std::fs::write(dist.join("plain.html"), "<h1>Plain</h1>").unwrap();
        std::fs::write(dist.join("plain.md"), "# Plain").unwrap();
        let request = |path: &str, accept: &str| {
            let handler = Static::load(&dist, "/");
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let task = std::thread::spawn(move || handler.handle_with_backend(listener.accept().unwrap().0, None).unwrap());
            let mut c = TcpStream::connect(addr).unwrap();
            // One write: a refusal after a partial read closes on unread bytes, and the peer gets a reset instead of the answer.
            c.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: {accept}\r\n\r\n").as_bytes()).unwrap();
            let mut r = String::new();
            c.read_to_string(&mut r).unwrap();
            task.join().unwrap();
            r
        };
        assert!(request("//about", "text/html").starts_with("HTTP/1.1 400"), "a leading // stays a parser refusal");
        for (path, to) in [("/about/index.html?x=1", "/about?x=1"), ("/index.html", "/"), ("/about//", "/about"), ("/a//about", "/a/about")] {
            let r = request(path, "text/html");
            assert!(r.starts_with("HTTP/1.1 301") && r.contains(&format!("Location: {to}\r\n")), "{path}: {r}");
        }
        let link = "Link: <https://site.example/about>; rel=\"canonical\"\r\n";
        assert!(request("/about.md", "*/*").contains(link));
        assert!(request("/about", "text/markdown").contains(link));
        assert!(!request("/about", "text/html").contains("rel=\"canonical\"\r\n"), "the page has it in its own head");
        assert!(!request("/plain.md", "*/*").contains("Link:"), "a page without a canonical gets no header");
        std::fs::remove_dir_all(&dist).unwrap();
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
    fn conditional_static_html_and_encoded_assets_omit_body_length_on_304() {
        use std::{io::{Read, Write}, net::{TcpListener, TcpStream}};
        let dist = std::env::temp_dir().join(format!("brain-prod-conditional-{}", std::process::id()));
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "<head><meta name=\"brain-build\" content=\"synthetic-build\"></head><body>Home</body>").unwrap();
        std::fs::write(dist.join("clip.mp4"), b"media bytes").unwrap();
        std::fs::write(dist.join("clip.mp4.br"), b"encoded").unwrap();
        let ask = |path: &str, method: &str, encoding: &str, condition: &str| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let app = super::Static::load(&dist, "/");
            let worker = std::thread::spawn(move || app.handle(listener.accept().unwrap().0).unwrap());
            client.write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: {encoding}\r\n{condition}\r\n").as_bytes()).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            worker.join().unwrap();
            response
        };
        for (path, encoding) in [("/", "identity"), ("/clip.mp4", "identity"), ("/clip.mp4", "br")] {
            let first = ask(path, "GET", encoding, "");
            let tag = first.lines().find_map(|line| line.strip_prefix("ETag: ")).unwrap().trim();
            for method in ["GET", "HEAD"] {
                let cached = ask(path, method, encoding, &format!("If-None-Match: {tag}\r\n"));
                assert!(cached.starts_with("HTTP/1.1 304") && cached.ends_with("\r\n\r\n"), "{cached}");
                assert!(!cached.contains("Content-Length:"), "{cached}");
            }
        }
        std::fs::remove_dir_all(dist).unwrap();
    }

    #[test]
    fn encodings_respect_weight_and_explicit_exclusion() {
        assert_eq!(super::accepted_encodings("gzip, br"), vec!["br", "gzip"]);
        assert_eq!(
            super::accepted_encodings("br;q=0, gzip;q=0.8"),
            vec!["gzip"]
        );
        assert_eq!(super::accepted_encodings("br;q=0, *;q=1"), vec!["zstd", "gzip"]);
        assert_eq!(super::accepted_encodings("gzip, deflate, br, zstd"), vec!["br", "zstd", "gzip"]);
        assert_eq!(super::accepted_encodings("zstd"), vec!["zstd"]);
        assert_eq!(
            super::accepted_encodings("gzip;q=1, br;q=0.2"),
            vec!["gzip", "br"]
        );
        assert!(super::accepted_encodings("xgzip, br;q=invalid").is_empty());
    }
    #[test]
    fn static_files_get_the_smallest_accepted_sibling() {
        use std::io::{Read, Write};
        let dist = std::env::temp_dir().join(format!("brain-prod-smallest-{}", std::process::id()));
        std::fs::create_dir_all(dist.join("_b")).unwrap();
        std::fs::write(dist.join("_b/app.js"), "x".repeat(400)).unwrap();
        std::fs::write(dist.join("_b/app.js.br"), "b".repeat(30)).unwrap();
        std::fs::write(dist.join("_b/app.js.zst"), "z".repeat(20)).unwrap();
        std::fs::write(dist.join("_b/app.js.gz"), "g".repeat(40)).unwrap();
        let get = |enc: &str| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let handler = super::Static::load(&dist, "/");
            let task = std::thread::spawn(move || handler.handle(listener.accept().unwrap().0).unwrap());
            let mut client = std::net::TcpStream::connect(addr).unwrap();
            write!(client, "GET /_b/app.js HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: {enc}\r\n\r\n").unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            task.join().unwrap();
            response
        };
        let all = get("gzip, br, zstd");
        assert!(all.contains("Content-Encoding: zstd") && all.ends_with(&"z".repeat(20)), "{all}");
        let no_zstd = get("gzip, br");
        assert!(no_zstd.contains("Content-Encoding: br") && no_zstd.contains("Vary: Accept-Encoding"), "{no_zstd}");
        assert!(get("gzip").ends_with(&"g".repeat(40)));
        let _ = std::fs::remove_dir_all(&dist);
    }
    #[test]
    fn atomic_replacement_keeps_static_headers_and_bytes_on_the_open_descriptor() {
        use std::{io::Read, net::{TcpListener, TcpStream}};
        let dist = std::env::temp_dir().join(format!("brain-prod-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dist).unwrap();
        let path = dist.join("clip.mp4");
        std::fs::write(&path, b"before").unwrap();
        let mut file = super::open_output(&path).unwrap();
        let tag = super::asset_tag(&path, &mut file).unwrap();
        std::fs::write(dist.join("next.mp4"), b"after!").unwrap();
        std::fs::rename(dist.join("next.mp4"), &path).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let response_tag = tag.clone();
        let worker = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            super::write_open_file_span(&mut stream, 200, &[("ETag".into(), response_tag)], file, None, false).unwrap();
        });
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        worker.join().unwrap();
        assert!(response.contains(&format!("ETag: {tag}")) && response.contains("Content-Length: 6") && response.ends_with("before"), "{response}");
        std::fs::remove_dir_all(dist).unwrap();
    }

    #[test]
    fn static_files_answer_one_byte_range_of_the_plain_file() {
        use std::io::{Read, Write};
        // Safari plays no video without Range: it asks for bytes=0-1 first and gives up on a 200
        let dist = std::env::temp_dir().join(format!("brain-prod-range-{}", std::process::id()));
        std::fs::create_dir_all(dist.join("_b")).unwrap();
        let body: String = (0..100).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        std::fs::write(dist.join("_b/clip.mp4"), &body).unwrap();
        std::fs::write(dist.join("_b/clip.mp4.br"), "b".repeat(10)).unwrap();
        let ask = |extra: &str, method: &str, enc: &str| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let handler = super::Static::load(&dist, "/");
            let task = std::thread::spawn(move || handler.handle(listener.accept().unwrap().0).unwrap());
            let mut client = std::net::TcpStream::connect(addr).unwrap();
            write!(client, "{method} /_b/clip.mp4 HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: {enc}\r\n{extra}\r\n").unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            task.join().unwrap();
            response
        };
        for (range, from, to) in [("bytes=0-1", 0, 1), ("bytes=90-", 90, 99), ("bytes=-5", 95, 99), ("bytes=50-500", 50, 99)] {
            let r = ask(&format!("Range: {range}\r\n"), "GET", "br");
            assert!(r.starts_with("HTTP/1.1 206 Partial Content"), "{range}: {r}");
            assert!(r.contains(&format!("Content-Range: bytes {from}-{to}/100")) && r.contains(&format!("Content-Length: {}", to - from + 1)), "{range}: {r}");
            assert!(!r.contains("Content-Encoding") && r.ends_with(&body[from..=to]), "the span is of the plain file: {r}");
        }
        for range in ["bytes=100-", "bytes=-0"] {
            let r = ask(&format!("Range: {range}\r\n"), "GET", "br");
            assert!(r.starts_with("HTTP/1.1 416") && r.contains("Content-Range: bytes */100") && !r.contains("immutable"), "{range}: {r}");
        }
        // several spans, another unit, a reversed span or If-Range (no validator to compare): the whole file
        for extra in ["Range: bytes=0-1,5-6\r\n", "Range: items=0-1\r\n", "Range: bytes=9-2\r\n", "Range: bytes=0-1\r\nIf-Range: \"x\"\r\n"] {
            assert!(ask(extra, "GET", "br").starts_with("HTTP/1.1 200"), "{extra}");
        }
        let plain = ask("", "GET", "identity");
        assert!(plain.contains("Accept-Ranges: bytes"), "{plain}");
        let tag = plain.lines().find_map(|line| line.strip_prefix("ETag: ")).unwrap().trim();
        assert!(tag.starts_with('"') && !tag.starts_with("W/"));
        let cached = ask(&format!("If-None-Match: W/{tag}\r\n"), "GET", "identity");
        assert!(cached.starts_with("HTTP/1.1 304") && cached.ends_with("\r\n\r\n"), "{cached}");
        assert!(!cached.contains("Content-Length:"), "{cached}");
        for (validator, status) in [(tag.to_string(), 206), (format!("W/{tag}"), 200), ("\"stale\"".into(), 200)] {
            let response = ask(&format!("Range: bytes=0-1\r\nIf-Range: {validator}\r\n"), "GET", "br");
            assert!(response.starts_with(&format!("HTTP/1.1 {status}")), "{response}");
            assert!(response.contains(&format!("ETag: {tag}")), "{response}");
        }
        let encoded = ask("", "GET", "br");
        let encoded_tag = encoded.lines().find_map(|line| line.strip_prefix("ETag: ")).unwrap().trim();
        assert_ne!(encoded_tag, tag);
        let head = ask("", "HEAD", "identity");
        assert!(head.contains(&format!("ETag: {tag}")) && head.contains("Content-Length: 100"), "{head}");
        let head = ask("Range: bytes=0-1\r\n", "HEAD", "br");
        assert!(head.starts_with("HTTP/1.1 206") && head.contains("Content-Length: 2") && head.ends_with("\r\n\r\n"), "{head}");
        let _ = std::fs::remove_dir_all(&dist);
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

    #[test]
    fn health_answers_on_the_frontend_port_and_an_app_file_wins() {
        use super::Static;
        use std::{io::{Read, Write}, net::{TcpListener, TcpStream}};
        let dist = std::env::temp_dir().join(format!("brain-prod-health-{}", std::process::id()));
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "<h1>Home</h1>").unwrap();
        let ask_method = |pre: &str, backend: Option<String>, path: &str, method: &str| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let handler = Static::load(&dist, pre);
            let task = std::thread::spawn(move || handler.handle_with_backend(listener.accept().unwrap().0, backend.as_deref()).unwrap());
            let mut client = TcpStream::connect(address).unwrap();
            client.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            client.write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            task.join().unwrap();
            response
        };
        let ask = |pre, backend, path| ask_method(pre, backend, path, "GET");
        for path in ["/health", "/health/live", "/health/ready", "/_brain/health", "/_brain/health/live", "/_brain/health/ready"] {
            let plain = ask("", None, path);
            assert!(plain.starts_with("HTTP/1.1 200") && plain.ends_with("{\"ok\":true}") && plain.contains("no-store"), "{plain}");
        }
        assert!(ask("/app", None, "/health").starts_with("HTTP/1.1 200"));
        assert!(ask("", None, "/health/other").starts_with("HTTP/1.1 404"));
        assert!(ask("/app", None, "/_brain/health").starts_with("HTTP/1.1 200"));
        assert!(ask("", None, "/_brain/health/other").starts_with("HTTP/1.1 404"));
        for (pre, path, canonical) in [("", "/health/ready", "/health/ready"), ("", "/_brain/health", "/health"),
            ("/app", "/app/_brain/health/live", "/health/live"), ("/app", "/_brain/health/ready", "/health/ready")] {
            for method in ["GET", "HEAD"] {
                let authority = TcpListener::bind("127.0.0.1:0").unwrap();
                let target = format!("http://{}", authority.local_addr().unwrap());
                let backend = std::thread::spawn(move || {
                    let (mut socket, _) = authority.accept().unwrap();
                    let req = crate::http::read_request(socket.try_clone().unwrap()).unwrap();
                    assert_eq!((req.path.as_str(), req.method.as_str()), (canonical, method));
                    crate::http::respond_with(&mut socket, 503, "application/json", b"{\"code\":\"DRAINING\"}", method == "HEAD", &[]).unwrap();
                });
                let forwarded = ask_method(pre, Some(target), path, method);
                backend.join().unwrap();
                assert!(forwarded.starts_with("HTTP/1.1 503") && forwarded.contains("no-store"), "{forwarded}");
                if method == "HEAD" { assert!(forwarded.ends_with("\r\n\r\n"), "HEAD must carry no body: {forwarded}"); }
                else { assert!(forwarded.contains("DRAINING"), "{forwarded}"); }
            }
        }
        std::fs::create_dir_all(dist.join("health")).unwrap();
        std::fs::write(dist.join("health/index.html"), "<h1>status board</h1>").unwrap();
        assert!(ask("", None, "/health").contains("status board"));
        std::fs::remove_dir_all(dist).unwrap();
    }
}
