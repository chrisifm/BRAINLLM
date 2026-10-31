//! Caching on by default (INTERNALS §35), applied after the pages are assembled and before precompression:
//! - `public/` assets that HTML or CSS reference are copied into `/_b/<hash>.<ext>` and their references rewritten
//!   (immutable); the rest of `public/` is copied as is.
//! - `_headers` (static hosts) and `_brain/nginx.conf`: `/_b/*` immutable, HTML and `sw.js` `no-cache`, unhashed
//!   `public/` files `max-age=3600, stale-while-revalidate=86400`, `Vary: Accept-Encoding`.
//! - `sw.js`: `self.BRAIN_BUILD = {…}` (build hash, precache list, offline page) + RUNTIME's `brain-sw.js`; with
//!   `sw: false` a worker that deletes its caches and unregisters itself, so the opt-out reaches past visitors.
//! - The registration snippet before `</body>` of every page: after `load`, in idle time, HTTPS or localhost only.
//! - Security (§31, #177): `integrity="sha384-…"` on every `/_b/` stylesheet, module preload and script; one CSP for
//!   the whole site (inline scripts by `'sha256-…'` hash: the router re-runs the region scripts of other pages, so
//!   each page must allow them all) plus the security headers, in `_headers` and the nginx snippet. server.mjs sends
//!   its own nonce-based CSP for what it serves.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// `brain.json` → `"cache": { "http", "sw", "offline", "precacheKB", "data" }` (`data` belongs to the runtime).
#[derive(Debug, Clone, PartialEq)]
pub struct CacheCfg {
    pub http: bool,
    pub sw: bool,
    pub offline: bool,
    pub precache_kb: usize,
    /// `data: false`: the runtime caches no data (`<meta name="brain-data-cache" content="off">`).
    pub data: bool,
    /// RUNTIME's `runtime/sw/brain-sw.js`; None = not available yet (no worker is emitted, only a warning).
    pub sw_template: Option<String>,
    /// brain.json `base` (#148), `/` or `/app/`: the build fills it from brain.json (callers leave the default).
    pub base: String,
    /// `brain build --ext web|brain` (#26); empty = brain.json `"ext"`, else `web`.
    pub ext: String,
    /// `"update": "prompt"`: a new worker waits until the page calls `applyUpdate()` (§35.8); default takes over at once.
    pub update_prompt: bool,
    /// `"routes": [...]`: declarative service worker strategies for same-origin data (§35.8).
    pub routes: Vec<SwRoute>,
}

/// One `cache.routes` entry, validated by [`sw_route`].
#[derive(Debug, Clone, PartialEq)]
pub struct SwRoute {
    /// Path relative to the app (`/api/products/`); `prefix` when the author wrote a trailing `*`.
    pub path: String,
    pub prefix: bool,
    pub strategy: &'static str,
    pub max_entries: u32,
    /// Seconds; 0 = no age limit.
    pub max_age: u64,
    /// Network-first deadline in milliseconds.
    pub timeout_ms: u64,
}

pub const SW_STRATEGIES: &[&str] = &["cache-first", "network-first", "stale-while-revalidate", "network-only"];

/// `30s`, `10m`, `1h`, `7d` or a number of seconds.
pub fn duration_secs(v: &str) -> Option<u64> {
    let v = v.trim();
    let (n, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len()));
    let n: u64 = n.parse().ok()?;
    Some(n * match unit { "" | "s" => 1, "m" => 60, "h" => 3600, "d" => 86400, _ => return None })
}

/// Validates one route: `match` starts with `/`, may end in `*`, never covers framework paths; bounded limits.
pub fn sw_route(pattern: &str, strategy: &str, max_entries: Option<f64>, max_age: Option<&str>, timeout: Option<&str>) -> Result<SwRoute, String> {
    let fix = "fix: e.g. { \"match\": \"/api/products/*\", \"strategy\": \"stale-while-revalidate\", \"maxEntries\": 50, \"maxAge\": \"1h\" }";
    let prefix = pattern.ends_with('*');
    let path = pattern.trim_end_matches('*');
    if !path.starts_with('/') || path.len() > 256 || path.contains('*') || path.chars().any(|c| c.is_whitespace() || c == '?' || c == '#') {
        return Err(format!("brain.json: cache.routes match `{pattern}` must be a path starting with / with an optional trailing *; {fix}"));
    }
    if ["/_b/", "/_r/", "/sw.js"].iter().any(|f| path.starts_with(f) || (prefix && f.starts_with(path))) {
        return Err(format!("brain.json: cache.routes match `{pattern}` overlaps framework files (/_b/, /_r/, /sw.js); match only your data paths such as /api/…"));
    }
    let Some(strategy) = SW_STRATEGIES.iter().copied().find(|s| *s == strategy) else {
        return Err(format!("brain.json: cache.routes strategy `{strategy}` must be one of {}", SW_STRATEGIES.join(", ")));
    };
    let max_entries = match max_entries {
        None => 64,
        Some(n) if n >= 1.0 && n <= 1000.0 && n.fract() == 0.0 => n as u32,
        Some(_) => return Err("brain.json: cache.routes maxEntries must be a whole number from 1 to 1000 (default 64)".into()),
    };
    let max_age = match max_age {
        None => 0,
        Some(v) => duration_secs(v).filter(|s| *s <= 31_536_000).ok_or("brain.json: cache.routes maxAge must be a duration such as \"10m\", \"1h\" or \"7d\" (at most one year)")?,
    };
    let timeout_ms = match timeout {
        None => 3000,
        Some(v) => duration_secs(v).filter(|s| (1..=60).contains(s)).ok_or("brain.json: cache.routes timeout must be a duration from \"1s\" to \"60s\" (default 3s)")? * 1000,
    };
    Ok(SwRoute { path: path.into(), prefix, strategy, max_entries, max_age, timeout_ms })
}

impl Default for CacheCfg {
    fn default() -> Self {
        CacheCfg { http: true, sw: true, offline: true, precache_kb: 2048, data: true, sw_template: None, base: "/".into(), ext: String::new(), update_prompt: false, routes: Vec::new() }
    }
}

pub const IMMUTABLE: &str = "public, max-age=31536000, immutable";
pub const REVALIDATE: &str = "no-cache";
/// Unhashed `public/` files: revalidated every time (§35.7: nothing may stay stale after a deploy).
pub const PUBLIC_SWR: &str = "no-cache";

/// Registration snippet (not part of the core runtime: §35 keeps the 4 KB budget), for an app at the root.
pub const REGISTER: &str = "<script>if('serviceWorker'in navigator&&(location.protocol=='https:'||/^(localhost|127\\.0\\.0\\.1|\\[::1\\])$/.test(location.hostname)))addEventListener('load',()=>(self.requestIdleCallback||setTimeout)(()=>navigator.serviceWorker.register('/sw.js',{updateViaCache:'none'}).then(r=>r.update())))</script>";

/// The registration for an app hosted under `base` (`/app/`): the worker file and its scope live under it.
pub fn register(base: &str) -> String {
    if base == "/" {
        return REGISTER.into();
    }
    REGISTER.replace("register('/sw.js',{", &format!("register('{base}sw.js',{{scope:'{base}',"))
}

/// Service worker for `sw: false` and for `brain run`: deletes every cache, unregisters and reloads open pages so
/// they stop being controlled.
pub const KILL_SW: &str = "self.addEventListener('install',()=>self.skipWaiting());self.addEventListener('activate',e=>e.waitUntil(caches.keys().then(k=>Promise.all(k.map(n=>caches.delete(n)))).then(()=>self.registration.unregister()).then(()=>self.clients.matchAll({type:'window'})).then(c=>c.forEach(w=>w.navigate(w.url)))))\n";

/// Minimal built-in offline page (used when the app has no `views/_offline.brain`).
/// No inline event handler: the CSP allows inline scripts by hash only.
const OFFLINE_HTML: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Offline</title><style>body{font:16px/1.5 system-ui,sans-serif;display:grid;place-items:center;min-height:100vh;margin:0;padding:16px}</style></head><body><main><h1>You are offline</h1><p>This page has not been visited yet. Reconnect and try again.</p><button id=\"retry\">Retry</button></main><script>retry.onclick=()=>location.reload()</script></body></html>";

/// Security headers on every response (INTERNALS §31); with `SECURITY_DOC` the same set runtime/server/server.mjs sends.
pub const SECURITY: &[(&str, &str)] = &[
    ("Strict-Transport-Security", "max-age=63072000; includeSubDomains"),
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "strict-origin-when-cross-origin"),
];
/// Headers that only mean something on a document: sent with the HTML only (plus the CSP), so chunks, images and
/// 304s do not carry ~0.8 KB they do not use (QA #124).
pub const SECURITY_DOC: &[(&str, &str)] = &[
    ("X-Frame-Options", "DENY"),
    ("Permissions-Policy", "camera=(), microphone=(), geolocation=(), payment=(), usb=()"),
    ("Cross-Origin-Opener-Policy", "same-origin"),
];

pub struct Applied {
    pub warnings: Vec<String>,
    /// Content hash of the whole build (the worker's cache name and the pages' strong ETag).
    pub build_hash: String,
}

/// A page as the cache pass sees it.
pub struct PageRef<'a> {
    pub route: &'a str,
    pub file: &'a str,
}

/// Explicit document-head opt-in; comments, raw text, attribute values and inert templates cannot enable an SDK.
pub fn google_signin(html:&str)->bool {
    let lower=html.to_ascii_lowercase();let bytes=lower.as_bytes();let mut at=0;let mut head=false;let mut templates=0usize;
    while let Some(offset)=lower[at..].find('<') {
        let start=at+offset;
        if lower[start..].starts_with("<!--") {let Some(end)=lower[start+4..].find("-->") else{return false};at=start+4+end+3;continue}
        let mut end=start+1;let mut quote=0;
        while end<bytes.len() {let byte=bytes[end];if quote!=0 {if byte==quote {quote=0}} else if matches!(byte,b'\''|b'"') {quote=byte} else if byte==b'>' {break}end+=1;}
        if end==bytes.len() {return false}at=end+1;
        let tag=&lower[start+1..end];let closing=tag.starts_with('/');let tag=tag.trim_start_matches('/');let length=tag.find(|ch:char|ch.is_ascii_whitespace() || ch=='/').unwrap_or(tag.len());let name=&tag[..length];
        if name=="head" && templates==0 {if closing{return false}head=true;continue}
        if name=="body" && templates==0 {return false}
        if name=="template" {if closing {templates=templates.saturating_sub(1)} else {templates+=1;}continue}
        if !closing && matches!(name,"script"|"style"|"title"|"noscript") {
            let close=format!("</{name}");let mut cursor=at;
            loop {let Some(offset)=lower[cursor..].find(&close) else{return false};let candidate=cursor+offset;let after=candidate+close.len();if bytes.get(after).is_some_and(|byte|byte.is_ascii_whitespace() || matches!(byte,b'>'|b'/')) {at=candidate;break}cursor=after;}
            continue
        }
        if !head || templates!=0 || closing || name!="meta" {continue}
        let mut rest=&tag[length..];let mut values=BTreeMap::new();
        while !rest.trim_start().is_empty() {
            rest=rest.trim_start();let count=rest.find(|ch:char|ch.is_ascii_whitespace() || matches!(ch,'='|'/')).unwrap_or(rest.len());if count==0 {break}let key=&rest[..count];rest=rest[count..].trim_start();
            if !rest.starts_with('=') {continue}rest=rest[1..].trim_start();let Some(first)=rest.chars().next() else{break};let value;
            if matches!(first,'\''|'"') {let Some(end)=rest[1..].find(first) else{break};value=&rest[1..1+end];rest=&rest[end+2..];}else {let end=rest.find(char::is_whitespace).unwrap_or(rest.len());value=&rest[..end];rest=&rest[end..];}
            values.entry(key).or_insert(value);
        }
        if values.get("name")==Some(&"brain-google-signin") && values.get("content")==Some(&"1") {return true}
    }false
}
pub fn google_csp()->Vec<(String,String)> {[("script-src","https://accounts.google.com/gsi/client"),("style-src","https://accounts.google.com/gsi/style"),("frame-src","https://accounts.google.com/gsi/"),("connect-src","https://accounts.google.com/gsi/")].into_iter().map(|(key,value)|(key.into(),value.into())).collect()}
fn google_paths(pages:&[PageRef],files:&BTreeMap<String,String>)->BTreeMap<String,String> {
    let mut paths=BTreeMap::new();
    for page in pages.iter().filter(|page|!crate::routes::is_dynamic(page.route) && page.route!="*" && !page.file.starts_with("_server/")) {
        if !files.get(page.file).is_some_and(|html|google_signin(html)) {continue}
        let route=page.route.trim_end_matches('/');let route=if route.is_empty() {"/"}else{route};
        for path in [route.to_string(),format!("{}/",route.trim_end_matches('/')),format!("/{}",page.file)] {if path.starts_with('/') && path.len()<=4096 && !path.chars().any(|ch|ch.is_whitespace() || matches!(ch,'"'|'\''|'\\'|'$'|';'|'{'|'}')) {paths.insert(path,format!("/{}",page.file));}}
    }paths
}
fn google_static_headers(headers:String,paths:&BTreeMap<String,String>,build:&str,http:bool,csp:&str)->String {
    let mut current=String::new();let mut present=BTreeSet::new();let mut out=String::new();
    for line in headers.lines() {
        if line.starts_with('/') {current=line.to_string();present.insert(current.clone());}
        if paths.contains_key(&current) && line=="  Cross-Origin-Opener-Policy: same-origin" {out.push_str("  Cross-Origin-Opener-Policy: same-origin-allow-popups\n");}else{out.push_str(line);out.push('\n');}
    }
    for path in paths.keys().filter(|path|!present.contains(*path)) {out.push_str(&format!("{path}\n"));if http {out.push_str(&format!("  Cache-Control: {REVALIDATE}\n  X-Brain-Build: {build}\n  Vary: Accept-Encoding\n"));}for (key,value) in SECURITY_DOC {let value=if *key=="Cross-Origin-Opener-Policy" {"same-origin-allow-popups"}else{value};out.push_str(&format!("  {key}: {value}\n"));}out.push_str(&format!("  Content-Security-Policy: {csp}\n"));}
    out
}
fn google_nginx(paths:&BTreeMap<String,String>,pre:&str,build:&str,http:bool,csp:&str)->String {
    let mut out=String::new();for (path,file) in paths {out.push_str(&format!("location = {pre}{path} {{\n"));for (key,value) in SECURITY.iter().chain(SECURITY_DOC) {let value=if *key=="Cross-Origin-Opener-Policy" {"same-origin-allow-popups"}else{value};out.push_str(&format!("    add_header {key} \"{value}\" always;\n"));}out.push_str(&format!("    add_header Content-Security-Policy \"{csp}\" always;\n"));if http {out.push_str(&format!("    etag on;\n    add_header Cache-Control \"{REVALIDATE}\";\n    add_header X-Brain-Build {build};\n"));}out.push_str(&format!("    try_files $uri $uri/index.html {pre}{file} =404;\n}}\n"));}out
}

/// Applies §35 to the built files. `files` holds text files, `binary` raw ones (copied `public/` assets).
/// `extra_csp`: `(directive, source)` pairs third-party tags need (brain-seo's Google tags).
pub fn apply(root: &Path, cfg: &CacheCfg, pages: &[PageRef], extra_csp: &[(String, String)], files: &mut BTreeMap<String, String>, binary: &mut BTreeMap<String, Vec<u8>>) -> Applied {
    let mut warnings = Vec::new();
    let pre = cfg.base.trim_end_matches('/'); // `/app` or empty: prefix of every URL (never of dist/ paths)
    let ext = crate::Ext::from_name(&cfg.ext);
    let offline_file = format!("_offline.{}", ext.html());
    // 1. public/: referenced → hashed into _b/; everything else copied as is
    let public = root.join("public");
    let mut assets: BTreeMap<String, Vec<u8>> = BTreeMap::new(); // "/logo.svg" → bytes
    walk(&public, &public, &mut assets);
    // every public file also gets an immutable copy; references in HTML, CSS, the web manifest and (before their
    // hash, in lib.rs) JS chunks point at it, while the original path stays published for URLs that live outside
    // the build (og:image in social caches, the manifest icons, favicon requests, /.well-known)
    let hashed: BTreeMap<String, String> = if cfg.http { assets.iter().map(|(p, b)| (p.clone(), hashed_url(p, b))).collect() } else { BTreeMap::new() };
    // what references point at: the hashed copy under the base
    // (links already moved under the base are `{pre}/logo.svg`: both spellings point at the copy)
    let hashed_ref: BTreeMap<String, String> =
        hashed.iter().flat_map(|(p, u)| [(p.clone(), format!("{pre}{u}")), (format!("{pre}{p}"), format!("{pre}{u}"))]).collect();
    if cfg.http && !assets.is_empty() {
        // CSS first: a rewritten stylesheet changes its own hash, so its URL changes in the pages too
        let css: Vec<String> = files.keys().filter(|f| f.starts_with("_b/") && f.ends_with(&format!(".{}", ext.css()))).cloned().collect();
        let mut renamed: Vec<(String, String)> = Vec::new();
        for f in css {
            let src = files[&f].clone();
            let out = rewrite(&src, &hashed_ref);
            if out != src {
                let new = format!("_b/{}.{}", crate::content_hash(&out), ext.css());
                files.remove(&f);
                files.insert(new.clone(), out);
                renamed.push((format!("{pre}/{f}"), format!("{pre}/{new}")));
            }
        }
        let docs: Vec<String> = pages.iter().map(|p| p.file.to_string()).chain(files.keys().filter(|f| f.ends_with(".webmanifest")).cloned()).collect();
        for f in docs {
            if let Some(html) = files.get(&f).cloned() {
                let mut out = rewrite(&html, &hashed_ref);
                for (a, b) in &renamed {
                    out = out.replace(a.as_str(), b);
                }
                files.insert(f, out);
            }
        }
    }
    for (p, bytes) in &assets {
        if let Some(url) = hashed.get(p) {
            binary.insert(url.trim_start_matches('/').to_string(), bytes.clone());
        }
        let rel = p.trim_start_matches('/').to_string();
        // files the build writes after this point (the service worker, host configs, the offline page) are reserved:
        // a public/ copy would silently replace them (QA #101, conformance 186)
        let reserved = rel == "sw.js" || rel == "_headers" || rel == "_redirects" || rel.starts_with("_brain/") || rel.starts_with("_server/") || rel == offline_file;
        if reserved {
            warnings.push(format!("public{p} is ignored: the build generates {rel} itself (never write your own service worker or host config there); fix: delete public{p}"));
        } else if files.contains_key(&rel) || binary.contains_key(&rel) {
            warnings.push(format!("public{p} is shadowed by a built file with the same path; rename it"));
        } else {
            binary.insert(rel, bytes.clone());
        }
    }

    // 1b. Subresource Integrity on every page (after the CSS renames above, so the hashes are of the final bytes)
    let mut sri: BTreeMap<String, String> = BTreeMap::new(); // url → integrity, each asset hashed once
    for p in pages {
        if let Some(html) = files.get(p.file) {
            let out = add_integrity_at(html, files, binary, &mut sri, pre);
            files.insert(p.file.to_string(), out);
        }
    }

    // 2. Build signature (§35.7): a hash of every file that ships, paths and bytes, sorted. ORDER MATTERS for a
    //    stable signature: it is computed here, BEFORE writing what carries it (the brain-build meta, the sw.js
    //    header, _headers, nginx.conf, the manifest's `build`) and before the registration snippet; those parts are
    //    derived from the signature plus inputs it already covers (the worker template and the cache config).
    let mut all = String::new();
    for (k, v) in files.iter() {
        all.push_str(k);
        all.push_str(&crate::content_hash(v));
    }
    for (k, v) in binary.iter() {
        all.push_str(k);
        all.push_str(&crate::bytes_hash(v));
    }
    all.push_str(cfg.sw_template.as_deref().unwrap_or(""));
    all.push_str(&format!("{cfg:?}").replace(cfg.sw_template.as_deref().unwrap_or("\u{0}"), ""));
    let build_hash = crate::content_hash(&all);

    // 3. service worker + offline page + registration
    if cfg.sw {
        match &cfg.sw_template {
            Some(tpl) => {
                // views/_offline.brain is built as the page /_offline/; without it, a minimal built-in page
                let built_offline = files.contains_key(&format!("_offline/index.{}", ext.html()));
                let offline_url = if built_offline { "/_offline/".to_string() } else { format!("/{offline_file}") };
                if cfg.offline && !built_offline && !files.contains_key(&offline_file) {
                    files.insert(offline_file.clone(), OFFLINE_HTML.into());
                }
                let list = precache_ext(pages, files, binary, cfg.precache_kb * 1024, pre, ext);
                // routes/update only when declared: the default worker stays byte-identical
                let mut extra = String::new();
                if !cfg.routes.is_empty() {
                    let r: Vec<String> = cfg.routes.iter().map(|r| format!("{{match:{},prefix:{},strategy:{},maxEntries:{},maxAge:{},timeout:{}}}", js(&format!("{pre}{}", r.path)), r.prefix, js(r.strategy), r.max_entries, r.max_age, r.timeout_ms)).collect();
                    extra.push_str(&format!(",routes:[{}]", r.join(",")));
                }
                if cfg.update_prompt {
                    extra.push_str(",update:\"prompt\"");
                }
                let manifest = format!(
                    "self.BRAIN_BUILD={{hash:{},base:{},precache:[{}],offline:{}{extra}}};\n",
                    js(&build_hash),
                    js(&cfg.base),
                    list.iter().map(|u| js(u)).collect::<Vec<_>>().join(","),
                    if cfg.offline { js(&format!("{pre}{offline_url}")) } else { "null".into() }
                );
                files.insert("sw.js".into(), format!("{manifest}{tpl}"));
                let reg = register(&cfg.base);
                for p in pages {
                    if let Some(html) = files.get_mut(p.file) {
                        // Successful static routes deliberately execute no JavaScript on first entry.
                        if html.contains("<meta name=\"brain-zero-js\" content=\"1\">") { continue; }
                        *html = match html.rfind("</body>") {
                            Some(i) => format!("{}{reg}{}", &html[..i], &html[i..]),
                            None => format!("{html}{reg}"),
                        };
                    }
                }
            }
            None => warnings.push("runtime/sw/brain-sw.js is not embedded yet (RUNTIME #122): no service worker this build".into()),
        }
    } else {
        // opt-out that reaches past visitors: their registered worker fetches this one and removes itself
        files.insert("sw.js".into(), KILL_SW.into());
    }

    // 4. the signature in every page (the router and the missing-chunk reload compare it), and the data switch
    let meta = format!(
        "<meta name=\"brain-build\" content=\"{build_hash}\">{}{}",
        if cfg.data { "" } else { "<meta name=\"brain-data-cache\" content=\"off\">" },
        // the router, navigate(), rpc() and the worker read the base from here (RUNTIME, #148)
        if cfg.base == "/" { String::new() } else { format!("<meta name=\"brain-base\" content=\"{}\">", cfg.base) }
    );
    let mut html_files: Vec<String> = pages.iter().map(|p| p.file.to_string()).collect();
    html_files.push(offline_file.clone());
    for f in html_files {
        if let Some(html) = files.get_mut(&f) {
            if !html.contains("name=\"brain-build\"") {
                *html = match html.find("</head>") {
                    Some(i) => format!("{}{meta}{}", &html[..i], &html[i..]),
                    None => format!("{meta}{html}"),
                };
            }
        }
    }
    // 5. headers for static hosts and nginx: caching (when `http`) and always the CSP + security headers. The CSP
    //    is computed from the final pages (after the registration snippet), so every inline script is covered.
    let mut inline: Vec<String> = Vec::new();
    for f in pages.iter().map(|p| p.file).filter(|f| !f.starts_with("_server/")).chain([offline_file.as_str()]) {
        if let Some(html) = files.get(f) {
            for body in inline_scripts(html) {
                let h = crate::sha::csp_hash(&body);
                if !inline.contains(&h) {
                    inline.push(h);
                }
            }
        }
    }
    inline.sort();
    if inline.len() > 64 {
        warnings.push(format!("the CSP lists {} inline script hashes (one per distinct page script); keep page scripts generic so the header stays small", inline.len()));
    }
    let policy = csp(&inline, extra_csp);
    let unhashed: Vec<String> = if cfg.http { assets.keys().cloned().collect() } else { Vec::new() }; // original paths (revalidated)
    // 103 Early Hints (§13): hosts that turn `Link` headers into early hints (Cloudflare, Netlify) start fetching the
    // page's module chunks and stylesheets while the HTML is still on its way
    let mut links: BTreeMap<String, String> = BTreeMap::new();
    for p in pages {
        if let Some(html) = files.get(p.file) {
            let l = early_links(html, pre);
            if !l.is_empty() {
                links.insert(p.route.to_string(), l);
            }
        }
    }
    let google=google_paths(pages,files);
    let mut h = headers_with_links(pages, &unhashed, files.contains_key(&offline_file), &build_hash, cfg.http, &policy, &links);
    if !google.is_empty() {h=google_static_headers(h,&google,&build_hash,cfg.http,&policy);}
    if ext == crate::Ext::Brain {
        h = h.replace("\n/404.html\n", "\n/404.brain\n").replace("\n/_offline.html\n", "\n/_offline.brain\n");
        h = brain_types(&h, pages, files, &offline_file);
    }
    files.insert("_headers".into(), if pre.is_empty() { h } else { base_paths(&h, pre) });
    // redirects.json (SEO, #219): fragments the CLI passes through the post step, placed in the host configs here
    let redir_nginx = files.remove("_brain/redirects.nginx").map(|r| base_redirects(&r, pre)).unwrap_or_default();
    let redir_caddy = files.remove("_brain/redirects.caddy").map(|r| base_redirects(&r, pre)).unwrap_or_default();
    let mut n = nginx_at(&build_hash, cfg.http, &policy, pre);
    if !google.is_empty() {n.push_str(&google_nginx(&google,pre,&build_hash,cfg.http,&policy));}
    let n = if redir_nginx.is_empty() { n } else { insert_after_first_line(&n, &format!("# redirects.json\n{redir_nginx}")) };
    files.insert("_brain/nginx.conf".into(), if ext == crate::Ext::Brain { nginx_brain(&n) } else { n });
    let mut c = caddy(&build_hash, cfg.http, &policy, pre, ext);
    if !google.is_empty() {
        c=c.replace("    Cross-Origin-Opener-Policy \"same-origin\"\n", "");
        let paths=google.keys().map(|path|format!("{pre}{path}")).collect::<Vec<_>>().join(" ");
        c.push_str(&format!("@google_signin path {paths}\nheader @google_signin Cross-Origin-Opener-Policy same-origin-allow-popups\n@ordinary_opener not path {paths}\nheader @ordinary_opener Cross-Origin-Opener-Policy same-origin\n"));
    }
    files.insert("_brain/Caddyfile".into(), if redir_caddy.is_empty() { c } else { insert_after_first_line(&c, &format!("# redirects.json\n{redir_caddy}")) });
    Applied { warnings, build_hash }
}

/// Precache list: core and router (every `_b/*.js` the shell pages load first), each route's entry chunks and CSS,
/// then the offline page, while the total raw size stays under `budget` bytes. Pages themselves are not precached
/// (navigations are network-first); the worker caches them on first visit.
pub fn precache(pages: &[PageRef], files: &BTreeMap<String, String>, binary: &BTreeMap<String, Vec<u8>>, budget: usize) -> Vec<String> {
    precache_at(pages, files, binary, budget, "")
}

/// `precache` for an app under the base prefix `pre` (`/app`): the URLs carry it, the dist/ lookups do not.
pub fn precache_at(pages: &[PageRef], files: &BTreeMap<String, String>, binary: &BTreeMap<String, Vec<u8>>, budget: usize, pre: &str) -> Vec<String> {
    precache_ext(pages, files, binary, budget, pre, crate::Ext::Web)
}

/// `precache_at` for the dist/ extensions `ext` (#26).
pub fn precache_ext(pages: &[PageRef], files: &BTreeMap<String, String>, binary: &BTreeMap<String, Vec<u8>>, budget: usize, pre: &str, ext: crate::Ext) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut used = 0usize;
    let size = |u: &str| {
        let k = crate::url_key(u, pre);
        let k = if k.ends_with('/') { format!("{k}index.{}", ext.html()) } else { k.to_string() };
        let k = k.as_str();
        files.get(k).map(String::len).or_else(|| binary.get(k).map(Vec::len)).unwrap_or(0)
    };
    let mut add = |u: String, out: &mut Vec<String>| {
        let s = size(&u);
        if s > 0 && !out.contains(&u) && used + s <= budget {
            used += s;
            out.push(u);
        }
    };
    if files.contains_key(&format!("_offline/index.{}", ext.html())) {
        add(format!("{pre}/_offline/"), &mut out);
    } else if files.contains_key(&format!("_offline.{}", ext.html())) {
        add(format!("{pre}/_offline.{}", ext.html()), &mut out);
    }
    for p in pages {
        let Some(html) = files.get(p.file) else { continue };
        for u in attr_urls(html) {
            // dictionaries are per locale: only the visited language's ever downloads (§37), so none is precached
            if u.starts_with(&format!("{pre}/_b/")) && !u.contains("/_b/i18n-") && (u.ends_with(&format!(".{}", ext.js())) || u.ends_with(&format!(".{}", ext.css()))) {
                add(u, &mut out);
            }
        }
    }
    out
}

/// Every `href="…"` / `src="…"` / `import … from"…"` URL in a page, in document order.
fn attr_urls(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for pat in ["href=\"", "src=\"", "from\"", "from \""] {
        let mut rest = html;
        while let Some(i) = rest.find(pat) {
            let s = &rest[i + pat.len()..];
            let Some(e) = s.find('"') else { break };
            out.push((i, s[..e].to_string()));
            rest = &s[e..];
        }
    }
    // keep document order across the patterns (offsets are relative, so order by first appearance instead)
    let mut seen = Vec::new();
    for (_, u) in out {
        if !seen.contains(&u) {
            seen.push(u);
        }
    }
    seen.sort_by_key(|u| html.find(u.as_str()).unwrap_or(usize::MAX));
    seen
}

/// `_headers` (Netlify, Cloudflare Pages and compatible hosts). Every path gets exactly one rule so hosts that merge
/// matching rules never combine two Cache-Control values.
/// `/logo.svg` + bytes → `/_b/<hash>.svg` (the same name lib.rs uses when it rewrites JS chunks).
pub fn hashed_url(path: &str, bytes: &[u8]) -> String {
    let ext = path.rsplit_once('.').filter(|(a, _)| !a.ends_with('/')).map_or(String::new(), |(_, e)| format!(".{e}"));
    format!("/_b/{}{ext}", crate::bytes_hash(bytes))
}

/// Every `public/` file → its immutable URL.
pub fn public_urls(root: &Path) -> BTreeMap<String, String> {
    let public = root.join("public");
    let mut assets = BTreeMap::new();
    walk(&public, &public, &mut assets);
    assets.iter().map(|(p, b)| (p.clone(), hashed_url(p, b))).collect()
}

/// `Link` value with the page's preloads: `<…/_b/x.js>; rel=modulepreload, <…/_b/y.css>; rel=preload; as=style`.
pub fn early_links(html: &str, pre: &str) -> String {
    early_link_list(html, pre).join(", ")
}

/// The `Link` values of a page, one per preload (server.mjs sends them as 103 Early Hints, RUNTIME #232).
pub fn early_link_list(html: &str, pre: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<link") {
        let Some(end) = rest[i..].find('>') else { break };
        let tag = &rest[i..i + end];
        let href = attr(tag, "href").filter(|h| h.starts_with(&format!("{pre}/_b/")));
        // the same integrity as the tag: a preload without it does not match the SRI request, so the browser
        // would discard it and download the file twice (QA #21 F5)
        let sri = attr(tag, "integrity").map_or(String::new(), |i| format!("; integrity=\"{i}\""));
        match (attr(tag, "rel").as_deref(), href) {
            (Some("modulepreload"), Some(h)) => out.push(format!("<{h}>; rel=modulepreload{sri}")),
            (Some("stylesheet"), Some(h)) => out.push(format!("<{h}>; rel=preload; as=style{sri}")),
            _ => {}
        }
        rest = &rest[i + end..];
    }
    out.dedup();
    out
}

pub fn headers(pages: &[PageRef], unhashed_public: &[String], offline: bool, build: &str, http: bool, csp: &str) -> String {
    headers_with_links(pages, unhashed_public, offline, build, http, csp, &BTreeMap::new())
}

/// `headers` plus a `Link` (early hints) per page route.
pub fn headers_with_links(pages: &[PageRef], unhashed_public: &[String], offline: bool, build: &str, http: bool, csp: &str, links: &BTreeMap<String, String>) -> String {
    let mut o = String::new();
    // transport headers for everything: one `/*` rule, the only one that sets them, so no host merges two values
    o.push_str("/*\n");
    for (k, v) in SECURITY {
        o.push_str(&format!("  {k}: {v}\n"));
    }
    let mut rule = |path: &str, cc: &str, html: bool, link: Option<&String>| {
        if !http && !html {
            return;
        }
        o.push_str(&format!("{path}\n"));
        if http {
            o.push_str(&format!("  Cache-Control: {cc}\n  Vary: Accept-Encoding\n"));
        }
        if html {
            if http {
                o.push_str(&format!("  X-Brain-Build: {build}\n"));
            }
            for (k, v) in SECURITY_DOC {
                o.push_str(&format!("  {k}: {v}\n"));
            }
            o.push_str(&format!("  Content-Security-Policy: {csp}\n"));
            if let Some(l) = link {
                o.push_str(&format!("  Link: {l}\n"));
            }
        }
    };
    rule("/_b/*", IMMUTABLE, false, None);
    rule("/sw.js", REVALIDATE, false, None);
    if offline {
        rule("/_offline.html", REVALIDATE, true, None);
    }
    for p in pages.iter().filter(|p| !crate::routes::is_dynamic(p.route) && p.route != "*" && !p.file.starts_with("_server/")) {
        let link = links.get(p.route);
        rule(p.route, REVALIDATE, true, link);
        if p.route != "/" {
            rule(&format!("{}/", p.route.trim_end_matches('/')), REVALIDATE, true, link);
        }
    }
    if pages.iter().any(|p| p.route == "*") {
        rule("/404.html", REVALIDATE, true, links.get("*"));
    }
    for p in unhashed_public {
        rule(p, PUBLIC_SWR, false, None);
    }
    o
}

/// nginx snippet to `include` inside the app's `server {}`: precompressed files, immutable chunks, revalidated HTML
/// (nginx answers `If-None-Match` with its own ETag; unchanged files keep their mtime because `brain build` only
/// writes what changed).
/// `_headers` for the own extensions (#26): hosts pick the type by extension, so every `.brain`/`.jsbrain`/`.cssbrain`
/// the build wrote gets an explicit `Content-Type` (one extra rule per file; header names never repeat a rule's).
pub fn brain_types(headers: &str, pages: &[PageRef], files: &BTreeMap<String, String>, offline_file: &str) -> String {
    let mut o = headers.to_string();
    let mut add = |path: &str, ct: &str| o.push_str(&format!("{path}\n  Content-Type: {ct}\n"));
    for f in files.keys().filter(|f| f.starts_with("_b/")) {
        if f.ends_with(".jsbrain") {
            add(&format!("/{f}"), "text/javascript; charset=utf-8");
        } else if f.ends_with(".cssbrain") {
            add(&format!("/{f}"), "text/css; charset=utf-8");
        }
    }
    let _ = (pages, offline_file);
    for f in files.keys().filter(|f| f.ends_with(".brain") && !f.starts_with("_server/")) {
        add(&format!("/{f}"), "text/html; charset=utf-8");
        // the clean URL of the same page (`/about/` → about/index.brain)
        if let Some(dir) = f.strip_suffix("index.brain") {
            add(&format!("/{dir}"), "text/html; charset=utf-8");
            if !dir.is_empty() {
                add(&format!("/{}", dir.trim_end_matches('/')), "text/html; charset=utf-8");
            }
        }
    }
    o
}

/// The nginx snippet for the own extensions: MIME types, `index.brain` and `.brain` pages.
pub fn nginx_brain(n: &str) -> String {
    let n = n
        .replace("try_files $uri $uri/index.html $uri.html =404;", "try_files $uri $uri/index.brain $uri.brain =404;")
        .replace("try_files $uri $uri/index.html =404;", "try_files $uri $uri/index.brain $uri.brain =404;")
        .replace("\\.html$)", "\\.brain$)");
    let first = n.find('\n').map_or(0, |i| i + 1);
    format!("{}types {{\n    text/html brain;\n    text/javascript jsbrain;\n    text/css cssbrain;\n}}\nindex index.brain;\n{}", &n[..first], &n[first..])
}

/// A Caddy block to paste inside the site (`root * <dist>`): precompressed files, immutable chunks, revalidated pages,
/// the security headers and CSP, and with the own extensions their MIME types and `index.brain`.
pub fn caddy(build_hash: &str, http: bool, csp: &str, pre: &str, ext: crate::Ext) -> String {
    let mut o = format!("# BRAINLLM build {build_hash}: paste inside your site block, with `root * <path to dist>`.\n");
    let mut sec: Vec<(String, String)> = SECURITY.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    sec.extend(SECURITY_DOC.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    sec.push(("Content-Security-Policy".into(), csp.to_string()));
    o.push_str("header {\n");
    for (k, v) in &sec {
        o.push_str(&format!("    {k} \"{}\"\n", v.replace('"', "\\\"")));
    }
    o.push_str(&format!("    X-Brain-Build {build_hash}\n}}\n"));
    o.push_str(&format!("@private path {pre}/_brain/* {pre}/_server/*\nrespond @private 404\n"));
    if http {
        o.push_str(&format!("@chunks path {pre}/_b/*\nheader @chunks Cache-Control \"{IMMUTABLE}\"\nheader {pre}/sw.js Cache-Control \"{REVALIDATE}\"\n"));
        o.push_str(&format!("@pages not path {pre}/_b/*\nheader @pages Cache-Control \"{REVALIDATE}\"\n"));
    }
    if ext == crate::Ext::Brain {
        o.push_str("@jsbrain path *.jsbrain\nheader @jsbrain Content-Type \"text/javascript; charset=utf-8\"\n");
        o.push_str("@cssbrain path *.cssbrain\nheader @cssbrain Content-Type \"text/css; charset=utf-8\"\n");
        o.push_str("@brainpage path *.brain */\nheader @brainpage Content-Type \"text/html; charset=utf-8\"\n");
        o.push_str("try_files {path} {path}/index.brain {path}.brain\nfile_server {\n    precompressed br gzip\n    index index.brain\n}\n");
    } else {
        o.push_str("try_files {path} {path}/index.html {path}.html\nfile_server {\n    precompressed br gzip\n}\n");
    }
    if !pre.is_empty() {
        o.push_str(&format!("# the app lives under {pre}/: serve it with `handle_path {pre}/* {{ … }}` around this block\n"));
    }
    o
}

/// Redirect rules written for an app at the root, moved under the base prefix `pre` (sources and same-site targets).
pub fn base_redirects(rules: &str, pre: &str) -> String {
    if pre.is_empty() {
        return rules.to_string();
    }
    rules
        .lines()
        .map(|l| {
            l.replace("location = /", &format!("location = {pre}/"))
                .replace("location ~ ^/", &format!("location ~ ^{pre}/"))
                .split(' ')
                .enumerate()
                .map(|(i, w)| if i > 0 && w.starts_with('/') && !w.starts_with("//") && !w.starts_with(pre) { format!("{pre}{w}") } else if w.starts_with("^/") && !w.starts_with(&format!("^{pre}/")) { format!("^{pre}{}", &w[1..]) } else { w.to_string() })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn insert_after_first_line(text: &str, block: &str) -> String {
    match text.find('\n') {
        Some(i) => format!("{}{block}{}", &text[..=i], &text[i + 1..]),
        None => format!("{text}\n{block}"),
    }
}

/// `_headers` paths moved under the base prefix `pre` (`/app`): `/about` → `/app/about`, `/*` → `/app/*`.
pub fn base_paths(headers: &str, pre: &str) -> String {
    headers.lines().map(|l| if l.starts_with('/') { format!("{pre}{l}") } else { l.to_string() }).collect::<Vec<_>>().join("\n") + "\n"
}

/// `nginx` for an app under the base prefix `pre` (`/app`): every location moves under it; the server's `root` must
/// be the folder where `<root>/app/` is dist/ (for example a symlink `app` → dist).
pub fn nginx_at(build_hash: &str, http: bool, csp: &str, pre: &str) -> String {
    let n = nginx(build_hash, http, csp);
    if pre.is_empty() {
        return n;
    }
    n.replace("(root = dist/).", &format!("(root = the folder where {pre}/ is dist/, e.g. a symlink)."))
        .replace("location /_b/", &format!("location {pre}/_b/"))
        .replace("location = /sw.js", &format!("location = {pre}/sw.js"))
        .replace("location ~ ^/_brain/", &format!("location ~ ^{pre}/_brain/"))
        .replace("location / {", &format!("location {pre}/ {{"))
        .replace("location ~ (^/$|/$|\\.html$)", &format!("location ~ (^{pre}/$|/$|\\.html$)"))
}

pub fn nginx(build_hash: &str, http: bool, csp: &str) -> String {
    // nginx drops server-level add_header in any location that has its own: every location repeats the security set
    let mut sec = String::new();
    for (k, v) in SECURITY {
        sec.push_str(&format!("    add_header {k} \"{v}\" always;\n"));
    }
    // documents also get the document-only set and the CSP
    let mut doc = sec.clone();
    for (k, v) in SECURITY_DOC {
        doc.push_str(&format!("    add_header {k} \"{v}\" always;\n"));
    }
    doc.push_str(&format!("    add_header Content-Security-Policy \"{csp}\" always;\n"));
    if !http {
        return format!("# BRAINLLM build {build_hash}: include this file inside the app's server {{}} block (root = dist/).\nlocation ~ ^/_brain/ {{\n    return 404;\n}}\nlocation / {{\n{doc}    try_files $uri $uri/index.html $uri.html =404;\n}}\n");
    }
    format!(
        "# BRAINLLM build {build_hash}: include this file inside the app's server {{}} block (root = dist/).\n\
gzip_static on;\n\
# brotli_static on;  # enable when nginx has ngx_brotli; brain build emits .br files\n\
location /_b/ {{\n{sec}    add_header Cache-Control \"{IMMUTABLE}\";\n    add_header Vary Accept-Encoding;\n    try_files $uri =404;\n}}\n\
location = /sw.js {{\n{sec}    add_header Cache-Control \"{REVALIDATE}\";\n}}\n\
location ~ ^/_brain/ {{\n    return 404;\n}}\n\
location / {{\n{doc}    etag on;\n    add_header Cache-Control \"{PUBLIC_SWR}\";\n    try_files $uri $uri/index.html =404;\n}}\n\
location ~ (^/$|/$|\\.html$) {{\n{doc}    etag on;\n    add_header Cache-Control \"{REVALIDATE}\";\n    add_header Vary Accept-Encoding;\n    add_header X-Brain-Build {build_hash};\n    try_files $uri $uri/index.html $uri.html =404;\n}}\n"
    )
}

/// Preserve existing app/legacy sources; add only the explicitly selected public socket origin.
pub fn native_socket_csp(options: &crate::NativeSocketOptions) -> Vec<(String, String)> {
    vec![("connect-src".into(), options.connect_source().into())]
}

/// The site's Content-Security-Policy (§31): scripts from the site itself and from the listed inline hashes only (no
/// `unsafe-inline`, no `unsafe-eval`), no plugins, no framing, no `<base>` hijack, forms to the same origin.
/// Styles allow inline (`style=` attributes and the layer-order `<style>`); `connect-src` allows HTTPS APIs.
pub fn csp(inline_hashes: &[String], extra: &[(String, String)]) -> String {
    let mut d: Vec<(&str, Vec<String>)> = vec![
        ("default-src", vec!["'self'".into()]),
        ("script-src", vec!["'self'".into()]),
        ("style-src", vec!["'self'".into(), "'unsafe-inline'".into()]),
        ("img-src", vec!["'self'".into(), "data:".into(), "blob:".into(), "https:".into()]),
        ("font-src", vec!["'self'".into(), "data:".into()]),
        ("connect-src", vec!["'self'".into(), "https:".into(), "wss:".into()]),
        ("object-src", vec!["'none'".into()]),
        ("base-uri", vec!["'self'".into()]),
        ("form-action", vec!["'self'".into()]),
        ("frame-ancestors", vec!["'none'".into()]),
        ("frame-src", vec!["'self'".into()]),
    ];
    d[1].1.extend(inline_hashes.iter().cloned());
    for (dir, src) in extra {
        // a source with a quote, semicolon or comma would break the header: never pass it through
        if src.contains(['"', ';', ',', '\n']) || dir.contains([' ', ';']) {
            continue;
        }
        match d.iter_mut().find(|(k, _)| k == dir) {
            Some((_, v)) => {
                if !v.contains(src) {
                    v.push(src.clone());
                }
            }
            None => {}
        }
    }
    d.iter().map(|(k, v)| format!("{k} {}", v.join(" "))).collect::<Vec<_>>().join("; ")
}

/// True for executable script tags, including external modules; JSON/state metadata does not execute.
pub fn has_executable_scripts(html: &str) -> bool {
    // Case-insensitive HTML tags/attributes and uncommon legacy JS MIME types must not create a false proof.
    let lower = html.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("<script") {
        let tag_rest = &rest[at..];
        let Some(end) = tag_rest.find('>') else { return true };
        let tag = &tag_rest[..end];
        let kind = attr(tag, "type").unwrap_or_default();
        // Unknown types are conservative: only known inert metadata is accepted as zero-JS.
        if !matches!(kind.as_str(), "application/json" | "application/ld+json" | "b/state") { return true; }
        rest = &tag_rest[end+1..];
        if let Some(close) = rest.find("</script") { rest = &rest[close+8..]; }
    }
    false
}

/// Bodies of the inline scripts the browser executes (classic, module, import map, speculation rules); data blocks
/// (`application/json`, `application/ld+json`, `b/state`) are not subject to CSP.
pub fn inline_scripts(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<script") {
        let tag_rest = &rest[i..];
        let Some(gt) = tag_rest.find('>') else { break };
        let tag = &tag_rest[..gt];
        let body_rest = &tag_rest[gt + 1..];
        let end = body_rest.find("</script").unwrap_or(body_rest.len());
        let ty = attr(tag, "type").unwrap_or_default().to_ascii_lowercase();
        let runs = matches!(ty.as_str(), "" | "module" | "text/javascript" | "application/javascript" | "importmap" | "speculationrules");
        if runs && attr(tag, "src").is_none() && !body_rest[..end].is_empty() {
            out.push(body_rest[..end].to_string());
        }
        rest = &body_rest[end..];
    }
    out
}

/// Value of `name="…"` (or `name='…'`) inside a start tag.
pub(crate) fn attr(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = tag[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before = tag[..at].chars().last();
        if !before.is_some_and(|c| c.is_whitespace()) {
            continue;
        }
        let after = tag[from..].trim_start();
        let Some(v) = after.strip_prefix('=') else { continue };
        let v = v.trim_start();
        let q = v.chars().next()?;
        if q == '"' || q == '\'' {
            return v[1..].find(q).map(|e| v[1..1 + e].to_string());
        }
        return Some(v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string());
    }
    None
}

/// `integrity="sha384-…"` on `<link rel="stylesheet|modulepreload" href="/_b/…">` and `<script src="/_b/…">` whose
/// bytes the build has. Tags that already carry `integrity` are left alone.
pub fn add_integrity(html: &str, files: &BTreeMap<String, String>, binary: &BTreeMap<String, Vec<u8>>, memo: &mut BTreeMap<String, String>) -> String {
    add_integrity_at(html, files, binary, memo, "")
}

/// `add_integrity` for an app under the base prefix `pre` (`/app`).
pub fn add_integrity_at(html: &str, files: &BTreeMap<String, String>, binary: &BTreeMap<String, Vec<u8>>, memo: &mut BTreeMap<String, String>, pre: &str) -> String {
    let mut out = String::with_capacity(html.len() + 256);
    let mut rest = html;
    loop {
        let next = [rest.find("<link"), rest.find("<script")].into_iter().flatten().min();
        let Some(i) = next else { break };
        let Some(gt) = rest[i..].find('>') else { break };
        let tag = &rest[i..i + gt];
        out.push_str(&rest[..i]);
        let url = if tag.starts_with("<link") {
            let rel = attr(tag, "rel").unwrap_or_default();
            if rel == "stylesheet" || rel == "modulepreload" { attr(tag, "href") } else { None }
        } else {
            attr(tag, "src")
        };
        let sum = url.filter(|u| u.starts_with(&format!("{pre}/_b/"))).and_then(|u| {
            if let Some(v) = memo.get(&u) {
                return Some(v.clone());
            }
            let k = crate::url_key(&u, pre);
            let v = files.get(k).map(|s| crate::sha::integrity(s.as_bytes())).or_else(|| binary.get(k).map(|b| crate::sha::integrity(b)))?;
            memo.insert(u, v.clone());
            Some(v)
        });
        match sum {
            Some(sum) if attr(tag, "integrity").is_none() => {
                let (head, close) = match tag.strip_suffix('/') {
                    Some(h) => (h.trim_end(), " /"),
                    None => (tag, ""),
                };
                out.push_str(&format!("{head} integrity=\"{sum}\"{close}>"));
            }
            _ => {
                out.push_str(tag);
                out.push('>');
            }
        }
        rest = &rest[i + gt + 1..];
    }
    out.push_str(rest);
    out
}

fn js(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Replaces every whole-URL reference to a hashed public asset.
pub fn rewrite(src: &str, hashed: &BTreeMap<String, String>) -> String {
    let mut out = src.to_string();
    for (p, url) in hashed {
        // `=`: unquoted attribute values in minified template markup (`<img src=/logo.svg>`, QA #124)
        for b in ["\"", "'", "`", "(", " ", ",", "="] {
            let pat = format!("{b}{p}");
            let mut res = String::with_capacity(out.len());
            let mut last = 0;
            for (i, _) in out.match_indices(&pat) {
                let end = i + pat.len();
                if out[end..].chars().next().is_none_or(|c| matches!(c, '"' | '\'' | '`' | ')' | ' ' | ',' | '?' | '#' | '>' | '\\')) {
                    res.push_str(&out[last..i]);
                    res.push_str(b);
                    res.push_str(url);
                    last = end;
                }
            }
            res.push_str(&out[last..]);
            out = res;
        }
    }
    out
}

fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        // dotfiles stay private (.DS_Store, .env…); `.well-known/` is public by definition (RFC 8615: security.txt,
        // assetlinks.json, apple-app-site-association)
        if name.starts_with('.') && !(name == ".well-known" && p.is_dir() && dir == base) {
            continue;
        }
        if p.is_dir() {
            walk(base, &p, out);
        } else if let Ok(b) = std::fs::read(&p) {
            let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.insert(format!("/{rel}"), b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("brain-cache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("public/img")).unwrap();
        d
    }

    fn built() -> BTreeMap<String, String> {
        let mut f = BTreeMap::new();
        f.insert("index.html".into(), "<html><head><link rel=\"stylesheet\" href=\"/_b/aaaa.css\"><link rel=\"icon\" href=\"/favicon.ico\"></head><body><img src=\"/img/logo.svg\" alt=\"\"><script type=\"module\">import{m}from\"/_b/core.js\";</script></body></html>".into());
        f.insert("_b/aaaa.css".into(), ".h{background:url(/img/bg.png)}".into());
        f.insert("_b/core.js".into(), "export const m=1".into());
        f
    }

    #[test]
    fn public_assets_are_hashed_and_rewritten() {
        let root = tmp("public");
        std::fs::write(root.join("public/img/logo.svg"), "<svg/>").unwrap();
        std::fs::write(root.join("public/img/bg.png"), [137u8, 80, 78, 71]).unwrap();
        std::fs::write(root.join("public/favicon.ico"), [0u8, 0, 1, 0]).unwrap();
        std::fs::write(root.join("public/robots-extra.txt"), "x").unwrap();
        let (mut files, mut bin) = (built(), BTreeMap::new());
        let pages = [PageRef { route: "/", file: "index.html" }];
        let a = apply(&root, &CacheCfg::default(), &pages, &[], &mut files, &mut bin);
        let html = &files["index.html"];
        assert!(!html.contains("\"/img/logo.svg\"") && !html.contains("\"/favicon.ico\""), "{html}");
        let logo = html.split("<img src=\"").nth(1).unwrap().split('"').next().unwrap();
        assert!(logo.starts_with("/_b/") && logo.ends_with(".svg") && bin.contains_key(logo.trim_start_matches('/')), "{logo}");
        // the stylesheet was rewritten, so it was renamed and the page points at the new name
        assert!(!files.contains_key("_b/aaaa.css") && !html.contains("/_b/aaaa.css"), "{html}");
        let css = files.iter().find(|(k, _)| k.ends_with(".css")).unwrap();
        assert!(css.1.contains("url(/_b/") && html.contains(&format!("/{}", css.0)), "{css:?}");
        // every public file stays at its original path (revalidated) as well as its hashed copy
        assert!(bin.contains_key("robots-extra.txt") && bin.contains_key("img/logo.svg") && bin.contains_key("favicon.ico"));
        assert!(files["_headers"].contains("/robots-extra.txt\n  Cache-Control: no-cache"));
        assert!(a.warnings.iter().any(|w| w.contains("RUNTIME #122")), "no template given: {:?}", a.warnings);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn headers_one_rule_per_path() {
        let pages = [PageRef { route: "/", file: "index.html" }, PageRef { route: "/about", file: "about/index.html" }];
        let h = headers(&pages, &["/a.txt".into()], true, "sig12345", true, "default-src 'self'");
        assert!(h.starts_with("/*\n  Strict-Transport-Security: ") && h.contains("  Content-Security-Policy: default-src 'self'\n"), "{h}");
        assert!(h.contains("/_b/*\n  Cache-Control: public, max-age=31536000, immutable\n  Vary: Accept-Encoding\n"), "{h}");
        // document-only headers and the CSP go with the HTML, not with every chunk (QA #124)
        let chunks = h.split("/_b/*\n").nth(1).unwrap().split("\n/").next().unwrap();
        assert!(!chunks.contains("Content-Security-Policy") && !chunks.contains("Cross-Origin-Opener-Policy"), "{h}");
        assert!(h.split("/about/\n").nth(1).unwrap().split("\n/").next().unwrap().contains("Content-Security-Policy"), "{h}");
        assert_eq!(rewrite("tpl(\"<img src=/a.png alt=x><img src=\\\"/a.png\\\">\")", &[("/a.png".to_string(), "/_b/h.png".to_string())].into()), "tpl(\"<img src=/_b/h.png alt=x><img src=\\\"/_b/h.png\\\">\")");
        assert!(h.contains("/sw.js\n  Cache-Control: no-cache") && h.contains("/about/\n  Cache-Control: no-cache") && h.contains("/\n  Cache-Control: no-cache"));
        let paths: Vec<&str> = h.lines().filter(|l| !l.starts_with(' ')).collect();
        let mut dedup = paths.clone();
        dedup.dedup();
        assert_eq!(paths, dedup, "{h}");
        assert!(nginx("abc", true, "default-src 'self'").contains("location /_b/") && nginx("abc", true, "default-src 'self'").contains("immutable"));
    }

    #[test]
    fn worker_routes_and_update_prompt() {
        assert_eq!(duration_secs("90"), Some(90));
        assert_eq!(duration_secs("10m"), Some(600));
        assert_eq!(duration_secs("2x"), None);
        let r = sw_route("/api/products/*", "stale-while-revalidate", Some(50.0), Some("1h"), None).unwrap();
        assert_eq!((r.path.as_str(), r.prefix, r.max_entries, r.max_age, r.timeout_ms), ("/api/products/", true, 50, 3600, 3000));
        for bad in ["api", "/_b/*", "/*", "/a*b", "/api?x"] {
            assert!(sw_route(bad, "cache-first", None, None, None).is_err(), "{bad}");
        }
        assert!(sw_route("/api", "fastest", None, None, None).unwrap_err().contains("network-first"));
        assert!(sw_route("/api", "cache-first", Some(0.0), None, None).is_err());
        assert!(sw_route("/api", "network-first", None, None, Some("2m")).is_err());
        let root = tmp("swroutes");
        let pages = [PageRef { route: "/", file: "index.html" }];
        let cfg = CacheCfg { sw_template: Some("/*T*/".into()), base: "/app/".into(), update_prompt: true, routes: vec![r], ..Default::default() };
        let (mut files, mut bin) = (built(), BTreeMap::new());
        apply(&root, &cfg, &pages, &[], &mut files, &mut bin);
        let sw = &files["sw.js"];
        assert!(sw.contains(",routes:[{match:\"/app/api/products/\",prefix:true,strategy:\"stale-while-revalidate\",maxEntries:50,maxAge:3600,timeout:3000}],update:\"prompt\"};"), "{sw}");
        let plain = CacheCfg { sw_template: Some("/*T*/".into()), ..Default::default() };
        let (mut f2, mut b2) = (built(), BTreeMap::new());
        apply(&root, &plain, &pages, &[], &mut f2, &mut b2);
        assert!(!f2["sw.js"].contains("routes") && !f2["sw.js"].contains("update:"));
    }

    #[test]
    fn worker_manifest_and_opt_out() {
        let root = tmp("sw");
        let pages = [PageRef { route: "/", file: "index.html" }];
        let cfg = CacheCfg { sw_template: Some("/*TEMPLATE*/".into()), ..Default::default() };
        let (mut files, mut bin) = (built(), BTreeMap::new());
        let a = apply(&root, &cfg, &pages, &[], &mut files, &mut bin);
        let sw = &files["sw.js"];
        assert!(sw.starts_with(&format!("self.BRAIN_BUILD={{hash:\"{}\",base:\"/\",precache:[", a.build_hash)) && sw.ends_with("/*TEMPLATE*/"), "{sw}");
        assert!(sw.contains("\"/_offline.html\"") && sw.contains("\"/_b/core.js\""), "{sw}");
        assert!(files["index.html"].contains(REGISTER) && files.contains_key("_offline.html"));
        // budget: nothing fits in 0 KB
        let none = precache(&pages, &files, &bin, 0);
        assert!(none.is_empty());
        // the signature is in every page, the worker and (with http) the headers
        assert!(files["index.html"].contains(&format!("<meta name=\"brain-build\" content=\"{}\">", a.build_hash)));
        assert!(files["_headers"].contains(&format!("X-Brain-Build: {}", a.build_hash)) && files["_brain/nginx.conf"].contains(&a.build_hash));
        assert!(REGISTER.contains("updateViaCache:'none'") && REGISTER.contains("r.update()"));
        // any change in any shipped file gives a new signature
        let (mut fx, mut bx) = (built(), BTreeMap::new());
        fx.insert("_b/core.js".into(), "export const m=2".into());
        assert_ne!(apply(&root, &cfg, &pages, &[], &mut fx, &mut bx).build_hash, a.build_hash);
        // same input → same build hash (deterministic)
        let (mut f2, mut b2) = (built(), BTreeMap::new());
        assert_eq!(apply(&root, &cfg, &pages, &[], &mut f2, &mut b2).build_hash, a.build_hash);
        // opt-out: a self-removing worker, no registration
        let (mut f3, mut b3) = (built(), BTreeMap::new());
        apply(&root, &CacheCfg { sw: false, ..cfg.clone() }, &pages, &[], &mut f3, &mut b3);
        assert_eq!(f3["sw.js"], KILL_SW);
        assert!(!f3["index.html"].contains("serviceWorker"));
        // http off: no cache rules, but the security headers and the CSP stay
        let (mut f4, mut b4) = (built(), BTreeMap::new());
        apply(&root, &CacheCfg { http: false, ..cfg }, &pages, &[], &mut f4, &mut b4);
        assert!(!f4["_headers"].contains("Cache-Control") && f4["_headers"].contains("Content-Security-Policy"), "{}", f4["_headers"]);
        assert!(!f4["_brain/nginx.conf"].contains("immutable") && f4["_brain/nginx.conf"].contains("X-Content-Type-Options"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn base_path_moves_every_url_under_it() {
        let root = tmp("base");
        std::fs::create_dir_all(root.join("public")).unwrap();
        std::fs::write(root.join("public/logo.svg"), "<svg/>").unwrap();
        let pages = [PageRef { route: "/", file: "index.html" }, PageRef { route: "/about", file: "about/index.html" }];
        let cfg = CacheCfg { sw_template: Some("/*T*/".into()), base: "/app/".into(), ..Default::default() };
        let mut files = BTreeMap::new();
        files.insert("_b/core.js".into(), "export const m=1".into());
        let page = "<html><head><script type=\"module\">import{m}from\"/app/_b/core.js\"</script></head><body><img src=\"/app/logo.svg\" alt=\"\"></body></html>";
        files.insert("index.html".into(), page.into());
        files.insert("about/index.html".into(), page.into());
        let mut bin = BTreeMap::new();
        apply(&root, &cfg, &pages, &[], &mut files, &mut bin);
        let html = &files["index.html"];
        assert!(html.contains("<meta name=\"brain-base\" content=\"/app/\">"), "{html}");
        assert!(!html.contains("\"/app/logo.svg\"") && html.contains("src=\"/app/_b/"), "public/ link under the base points at the hashed copy: {html}");
        assert!(html.contains("register('/app/sw.js',{scope:'/app/',updateViaCache:'none'})"), "{html}");
        let sw = &files["sw.js"];
        assert!(sw.contains("base:\"/app/\"") && sw.contains("\"/app/_b/core.js\"") && sw.contains("offline:\"/app/_offline.html\""), "{sw}");
        let h = &files["_headers"];
        assert!(h.starts_with("/app/*\n") && h.contains("\n/app/_b/*\n") && h.contains("\n/app/about/\n") && h.contains("\n/app/logo.svg\n") && !h.contains("\n/about"), "{h}");
        let n = &files["_brain/nginx.conf"];
        assert!(n.contains("location /app/_b/") && n.contains("location = /app/sw.js") && !n.contains("location /_b/"), "{n}");
        // dist/ paths never carry the base
        assert!(bin.contains_key("logo.svg") && bin.keys().any(|k| k.starts_with("_b/")) && !files.keys().any(|k| k.starts_with("app/")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn public_cannot_replace_generated_files() {
        let root = tmp("reserved");
        std::fs::create_dir_all(root.join("public")).unwrap();
        std::fs::write(root.join("public/sw.js"), "self.evil = 1").unwrap();
        let pages = [PageRef { route: "/", file: "index.html" }];
        let cfg = CacheCfg { sw_template: Some("/*T*/".into()), ..Default::default() };
        let (mut files, mut bin) = (built(), BTreeMap::new());
        let a = apply(&root, &cfg, &pages, &[], &mut files, &mut bin);
        assert!(files["sw.js"].ends_with("/*T*/") && !bin.contains_key("sw.js"), "the generated worker wins");
        assert!(a.warnings.iter().any(|w| w.contains("public/sw.js is ignored")), "{:?}", a.warnings);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn redirects_reach_nginx_and_caddy_under_the_base() {
        let root = tmp("redir");
        let pages = [PageRef { route: "/", file: "index.html" }];
        let cfg = CacheCfg { base: "/app/".into(), ..Default::default() };
        let (mut files, mut bin) = (built(), BTreeMap::new());
        files.insert("_brain/redirects.nginx".into(), "location = /old { return 301 /new; }\nlocation ~ ^/blog/(.*)$ { return 302 https://x.dev/$1; }\n".into());
        files.insert("_brain/redirects.caddy".into(), "redir /old /new 301\n".into());
        apply(&root, &cfg, &pages, &[], &mut files, &mut bin);
        let n = &files["_brain/nginx.conf"];
        assert!(n.contains("location = /app/old { return 301 /app/new; }") && n.contains("location ~ ^/app/blog/(.*)$ { return 302 https://x.dev/$1; }"), "{n}");
        assert!(files["_brain/Caddyfile"].contains("redir /app/old /app/new 301"), "{}", files["_brain/Caddyfile"]);
        assert!(!files.contains_key("_brain/redirects.nginx") && !files.contains_key("_brain/redirects.caddy"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn integrity_and_csp_cover_every_script() {
        let root = tmp("csp");
        let pages = [PageRef { route: "/", file: "index.html" }];
        let cfg = CacheCfg { sw_template: Some("/*T*/".into()), ..Default::default() };
        let (mut files, mut bin) = (built(), BTreeMap::new());
        files.insert(
            "index.html".into(),
            "<html><head><link rel=\"stylesheet\" href=\"/_b/aaaa.css\"><link rel=\"modulepreload\" href=\"/_b/core.js\"><link rel=\"preload\" href=\"/_b/core.js\"></head><body><script type=\"application/ld+json\">{}</script><script type=\"b/state\">[]</script><script type=\"module\">import{m}from\"/_b/core.js\"</script><script src=\"/_b/core.js\" type=\"module\"></script></body></html>".into(),
        );
        let extra = [("script-src".to_string(), "https://www.googletagmanager.com".to_string()), ("script-src".to_string(), "x;y".to_string())];
        apply(&root, &cfg, &pages, &extra, &mut files, &mut bin);
        let html = &files["index.html"];
        let core = crate::sha::integrity(files["_b/core.js"].as_bytes());
        assert!(html.contains(&format!("<link rel=\"modulepreload\" href=\"/_b/core.js\" integrity=\"{core}\">")), "{html}");
        assert!(html.contains(&format!("<script src=\"/_b/core.js\" type=\"module\" integrity=\"{core}\">")), "{html}");
        assert!(html.contains("<link rel=\"preload\" href=\"/_b/core.js\">"), "plain preloads get no integrity: {html}");
        assert_eq!(html.matches("integrity=").count(), 3, "{html}");
        // every executable inline script (the mount and the registration) is hashed; data blocks are not
        let h = &files["_headers"];
        let policy = h.lines().find_map(|l| l.trim().strip_prefix("Content-Security-Policy: ")).unwrap();
        let scripts = inline_scripts(html);
        assert_eq!(scripts.len(), 2, "{scripts:?}");
        for s in &scripts {
            assert!(policy.contains(&crate::sha::csp_hash(s)), "{policy}");
        }
        assert!(!policy.contains("unsafe-eval") && policy.contains("frame-ancestors 'none'") && policy.contains("object-src 'none'"));
        assert!(policy.contains("https://www.googletagmanager.com") && !policy.contains("x;y"), "{policy}");
        // the offline page has no inline handler and its script is in the policy too
        assert!(!files["_offline.html"].contains("onclick=\""));
        for s in inline_scripts(&files["_offline.html"]) {
            assert!(policy.contains(&crate::sha::csp_hash(&s)));
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
