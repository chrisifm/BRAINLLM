//! brain-pack: production bundler (INTERNALS §6, §10). Builds the module graph from `views/`, emits one content-hashed
//! chunk per module (shared modules are emitted once, so each view only downloads what it imports), one CSS file per
//! view (its component styles, in `@layer` order) plus a shared tokens file, and one HTML page per static route.
//! Compilation is injected (`Compile`) so this crate does not depend on the compiler front end.

pub mod gzip;
pub mod impact;
pub mod cache;
pub mod classes;
mod server;
pub mod prerender;
pub mod routes;
pub mod sha;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What the compiler returns for one source file in production mode.
pub struct Unit {
    pub js: String,
    pub css: String,
    /// Non-empty when the module has `load` / `server:` code (needs `dist/server.mjs`, #51).
    pub server_js: String,
    /// Names of its `server:` functions (RPC endpoints `/_r/<hash>/<fn>`).
    pub server_fns: Vec<String>,
}

pub type Compile<'a> = &'a dyn Fn(&str, &Path) -> Result<Unit, String>;

pub struct Input<'a> {
    pub root: PathBuf,
    pub compile: Compile<'a>,
    /// Runtime core already minified with `RUNTIME_MAP` (or plain when `short_runtime` is false).
    pub runtime_core: String,
    /// Sibling runtime modules (brain-dev.js is never shipped; brain-wc.js, brain-form.js… only if imported).
    pub runtime_siblings: Vec<(String, String)>,
    /// `<style>@layer …;</style>` order, from brain-css.
    pub layer_order: String,
    /// Minify JS chunks (mangling + `log()` removal).
    pub minify: bool,
    /// Absolute origin (`--site`) for canonical/og:url.
    pub site: Option<String>,
    /// Runtime DOM shim source; `Some` enables build-time prerender (needs Node).
    pub shim: Option<String>,
    /// `<meta name="generator">` content (empty = none, `--no-brand`).
    pub generator: String,
    /// Writes the server module of a view (its `server_js` with imports resolved) and returns its `file://` URL and
    /// content. Needed to evaluate `paths()`, build-time `load`, `seo(props)` and `head(props)` (INTERNALS §32).
    pub server_module: Option<&'a dyn Fn(&str) -> Result<(String, String), String>>,
    /// `brain.json` `cache` (INTERNALS §35): HTTP headers, service worker, offline page, precache budget.
    pub cache: cache::CacheCfg,
    /// RUNTIME's `runtime/server/server.mjs`. With it, views with `load`, dynamic routes without `paths()` and
    /// `server:` functions ship as `dist/server.mjs` + `dist/_server/` (#51); without it they fail the build.
    pub server_mjs: Option<String>,
    /// Phase reporter `(phase, done, total)` for the terminal view (total 0 = unknown). Cheap; called from workers.
    pub progress: Option<&'a (dyn Fn(&str, usize, usize) + Sync)>,
    /// Post-pages step (brain-cli passes brain-seo's `finish`).
    pub post: Option<Post<'a>>,
}

pub struct Page {
    pub route: String,
    pub file: String,
    pub html: String,
    /// The view it comes from (`views/about.brain`).
    pub view: String,
}

/// What the post-pages step (SEO) returns: generated text and binary files plus warnings.
#[derive(Default)]
pub struct PostOut {
    pub files: Vec<(String, String)>,
    pub binary: Vec<(String, Vec<u8>)>,
    pub warnings: Vec<String>,
    /// `(CSP directive, source)` third-party origins the pages need (Google tags): added to the build's CSP.
    pub csp: Vec<(String, String)>,
}

/// Runs after the pages are assembled (HTML may be rewritten in place) and before caching, the build signature and
/// precompression, so what it adds is hashed, precached and gzipped like the rest. Gets the other dist files as
/// `(path, size)`. `Err` fails the build (e.g. a broken internal link).
pub type Post<'a> = &'a dyn Fn(&mut [Page], &[(String, u64)]) -> Result<PostOut, String>;

pub struct Output {
    /// `dist/` relative path → content.
    pub files: BTreeMap<String, String>,
    /// `dist/` relative path → bytes (the precompressed `.gz` of every text file).
    pub binary: BTreeMap<String, Vec<u8>>,
    pub pages: Vec<Page>,
    pub warnings: Vec<String>,
    /// Per page: what it downloads, before and after each pass.
    pub report: Vec<ReportRow>,
    /// Build signature (§35.7): hash of everything in dist/; the same source gives the same signature.
    pub signature: String,
}

/// Bytes one page downloads: its HTML plus every JS/CSS file it references.
pub struct ReportRow {
    pub route: String,
    /// JS as codegen emitted it (before minification and tree shaking).
    pub js_raw: usize,
    /// JS after minification + tree shaking.
    pub js_min: usize,
    /// …and gzip-compressed (own optimal-parse gzip).
    pub js_gz: usize,
    pub css: usize,
    pub css_gz: usize,
    pub html: usize,
    pub html_gz: usize,
}

/// Build-step cache in `<app>/.brain/cache/<kind>/` keyed by the input's content hash (INTERNALS §28: the build
/// reuses work whose input did not change). Failures to read or write the cache only cost a recomputation.
fn cached_bytes(root: &Path, kind: &str, input: &[u8], f: impl FnOnce() -> Result<Vec<u8>, String>) -> Result<Vec<u8>, String> {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in input.iter().chain(kind.as_bytes()).chain(env!("BRAIN_PACK_FP").as_bytes()) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let file = root.join(".brain/cache").join(kind).join(format!("{h:016x}"));
    if let Ok(b) = std::fs::read(&file) {
        return Ok(b);
    }
    let out = f()?;
    if let Some(d) = file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, &out).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
    Ok(out)
}

/// brain.json `"base"` (#148): where the app is hosted, normalized to `/` or `/x/…/`. Read here so the build needs
/// no extra input; only a top-level `"base"` string counts.
pub fn read_base(root: &Path) -> String {
    match read_json_str(root, "base") {
        Some(v) => {
            let v = v.trim().trim_matches('/');
            if v.is_empty() { "/".into() } else { format!("/{v}/") }
        }
        None => "/".into(),
    }
}

/// brain.json `"ext"` (#26): `"brain"` publishes `.brain`/`.jsbrain`/`.cssbrain`; anything else is `web`.
pub fn read_ext(root: &Path) -> Ext {
    if read_json_str(root, "ext").as_deref() == Some("brain") { Ext::Brain } else { Ext::Web }
}

/// File extensions of dist/ (#26, decided by Chris: `web` by default, the own ones opt-in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ext {
    /// `.html`/`.js`/`.css`: works on any host without configuration.
    Web,
    /// `.brain`/`.jsbrain`/`.cssbrain` (INTERNALS §10): needs the server to map the types (the emitted configs do).
    Brain,
}

impl Ext {
    pub fn js(self) -> &'static str {
        if self == Ext::Brain { "jsbrain" } else { "js" }
    }
    pub fn css(self) -> &'static str {
        if self == Ext::Brain { "cssbrain" } else { "css" }
    }
    pub fn html(self) -> &'static str {
        if self == Ext::Brain { "brain" } else { "html" }
    }
    pub fn from_name(s: &str) -> Ext {
        if s == "brain" { Ext::Brain } else { Ext::Web }
    }
}

/// brain.json `"i18n": { "default": "en", "locales": ["en", "es"] }` (INTERNALS §37). None without it (and without
/// `i18n/<default>.json`); one locale = a single-language site (no prefixes, but `t()` still needs its dictionary).
pub fn read_i18n(root: &Path) -> Option<(String, Vec<String>)> {
    let src = std::fs::read_to_string(root.join("brain.json")).ok()?;
    let i = src.find("\"i18n\"")?;
    let open = i + src[i..].find('{')?;
    let end = json_value_end(&src, open)?;
    let obj = &src[open..end];
    let default = obj.find("\"default\"").and_then(|d| obj[d + 9..].split('"').nth(1)).map(String::from)?;
    let mut locales: Vec<String> = match obj.find("\"locales\"").and_then(|l| obj[l..].find('[').map(|b| l + b)) {
        Some(b) => obj[b + 1..obj[b..].find(']').map_or(obj.len(), |e| b + e)].split(',').map(|x| x.trim().trim_matches('"').to_string()).filter(|x| !x.is_empty()).collect(),
        None => vec![],
    };
    if !locales.contains(&default) {
        locales.insert(0, default.clone());
    }
    Some((default, locales))
}

/// Index after the JSON value that starts at `at` (string, object or array; strings and escapes respected).
fn json_value_end(src: &str, at: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut i = at;
    let mut depth = 0i32;
    loop {
        match *b.get(i)? {
            b'"' => {
                i += 1;
                while *b.get(i)? != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ if depth == 0 => {
                // number, true/false/null
                let e = src[i..].find([',', '}', ']']).map_or(src.len(), |e| i + e);
                return Some(e);
            }
            _ => {}
        }
        i += 1;
    }
}

/// `i18n/<locale>.json` → (key, raw JSON value) for every top-level entry, in file order (§37: flat dotted keys;
/// a value is a string or a plural object).
pub fn read_dict(root: &Path, locale: &str) -> Vec<(String, String)> {
    let Ok(src) = std::fs::read_to_string(root.join("i18n").join(format!("{locale}.json"))) else { return vec![] };
    let mut out = Vec::new();
    let Some(mut i) = src.find('{').map(|i| i + 1) else { return out };
    let b = src.as_bytes();
    while i < b.len() {
        match b[i] {
            b'"' => {
                let Some(ke) = json_value_end(&src, i) else { break };
                let key = src[i + 1..ke - 1].to_string();
                let Some(colon) = src[ke..].find(':').map(|c| ke + c + 1) else { break };
                let vs = colon + src[colon..].len() - src[colon..].trim_start().len();
                let Some(ve) = json_value_end(&src, vs) else { break };
                out.push((key, src[vs..ve].trim().to_string()));
                i = ve;
            }
            b'}' => break,
            _ => i += 1,
        }
    }
    out
}

/// Keys a compiled module reads with `t(…)`: literal ones by name, `"*"` when one call is dynamic (`t(k)`).
fn i18n_keys_of(js: &str) -> Vec<String> {
    use brain_min::lex::{lex, Kind};
    if !js.contains("brain-i18n.js") {
        return vec![];
    }
    let Ok(t) = lex(js) else { return vec!["*".into()] };
    let mut out = Vec::new();
    for i in 0..t.len().saturating_sub(2) {
        if t[i].ident("t") && t[i + 1].punct("(") && !(i > 0 && (t[i - 1].punct(".") || t[i - 1].ident("function"))) {
            if t[i + 2].kind == Kind::Str && (t.get(i + 3).is_some_and(|x| x.punct(")") || x.punct(","))) {
                out.push(t[i + 2].text[1..t[i + 2].text.len() - 1].to_string());
            } else {
                out.push("*".into());
            }
        }
    }
    out
}

/// The source file of the component whose `data-b` hash is `hash` (codegen writes `X.h = "<hash>"` and `X.f = "<file>"`).
fn component_file(modules: &HashMap<Node, Module>, hash: &str) -> Option<String> {
    if hash.is_empty() {
        return None;
    }
    let needle = format!(".h = \"{hash}\";");
    modules.iter().find(|(_, m)| m.js.contains(&needle)).and_then(|(n, m)| {
        let i = m.js.find(".f = \"")?;
        let f = m.js[i + 6..].split('"').next().map(String::from);
        f.or_else(|| match n {
            Node::File(r) => Some(r.clone()),
            _ => None,
        })
    })
}

/// `<meta route es="/es/acerca">`: the view's translated path for a locale (§37).
fn route_meta(src: &str, locale: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = src[from..].find("<meta route").map(|i| from + i) {
        let end = tag_end(src, i).unwrap_or(src.len());
        if let Some(v) = cache::attr(&src[i..end.saturating_sub(1)], locale) {
            return Some(v);
        }
        from = end;
    }
    None
}

/// `<html …>` of a page with `lang`, `dir` (`rtl` for ar, he, fa, ur…) and `data-default-lang` (§37, RUNTIME #150).
fn html_lang(doc: &str, locale: &str, default: &str) -> String {
    let Some(i) = doc.find("<html") else { return doc.to_string() };
    let Some(end) = tag_end(doc, i) else { return doc.to_string() };
    let mut tag = doc[i..end - 1].to_string();
    for a in ["lang", "dir", "data-default-lang"] {
        if let Some(v) = cache::attr(&tag, a) {
            for q in ['"', '\''] {
                tag = tag.replace(&format!(" {a}={q}{v}{q}"), "");
            }
            tag = tag.replace(&format!(" {a}={v}"), "");
        }
    }
    let rtl = ["ar", "he", "fa", "ur", "ps", "sd", "ug", "yi", "dv"].contains(&locale.split('-').next().unwrap_or(""));
    format!("{}{tag} lang=\"{locale}\" dir=\"{}\" data-default-lang=\"{default}\">{}", &doc[..i], if rtl { "rtl" } else { "ltr" }, &doc[end..])
}

/// `views/posts/[id].brain` → `posts-id` (dictionary chunk names).
fn view_slug(view: &str) -> String {
    let s: String = view.trim_start_matches("views/").trim_end_matches(".brain").chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    let mut out = String::new();
    for c in s.chars() {
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    if out.is_empty() { "home".into() } else { out }
}

/// A top-level string value of brain.json (nested objects are skipped).
fn read_json_str(root: &Path, want: &str) -> Option<String> {
    let src = std::fs::read_to_string(root.join("brain.json")).ok()?;
    let b = src.as_bytes();
    let (mut depth, mut i) = (0usize, 0usize);
    while i < b.len() {
        match b[i] {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b'"' => {
                let start = i + 1;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                let key = &src[start..i.min(src.len())];
                if depth == 1 && key == want {
                    let rest = src[i + 1..].trim_start();
                    if let Some(v) = rest.strip_prefix(':').map(str::trim_start).and_then(|r| r.strip_prefix('"')) {
                        return Some(v.split('"').next().unwrap_or("").to_string());
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// `/app/_b/x.js` → `_b/x.js` (a dist/ path) for the URLs the build writes under the base prefix `pre`.
pub fn url_key<'a>(u: &'a str, pre: &str) -> &'a str {
    u.strip_prefix(pre).filter(|r| r.starts_with('/')).unwrap_or(u).trim_start_matches('/')
}

/// Prefixes root-relative URLs of `href`/`src`/`action`/`formaction`/`poster` attributes with `pre` (`/app`): in
/// HTML and in the template strings of compiled modules (quoted, unquoted or with escaped quotes). Protocol-relative
/// URLs (`//cdn`) and values already under the base are left alone.
pub fn prefix_links(src: &str, pre: &str) -> String {
    if pre.is_empty() {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len() + 64);
    let mut last = 0;
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        // byte comparison: `i` may sit inside a multi-byte character; every match below is ASCII, so `j` is a boundary
        let hit = ["href=", "src=", "action=", "formaction=", "poster="].iter().find(|a| b[i..].starts_with(a.as_bytes())).copied();
        let Some(a) = hit else {
            i += 1;
            continue;
        };
        let before_ok = i > 0 && (b[i - 1] as char).is_whitespace();
        let mut j = i + a.len();
        for q in ["\\\"", "\\'", "\"", "'"] {
            if b[j..].starts_with(q.as_bytes()) {
                j += q.len();
                break;
            }
        }
        let v = &src[j..];
        if before_ok && v.starts_with('/') && !v.starts_with("//") && !v.starts_with(&format!("{pre}/")) && !(v.starts_with(pre) && v[pre.len()..].starts_with(['"', '\'', ' ', '>', '\\'])) {
            out.push_str(&src[last..j]);
            out.push_str(pre);
            last = j;
        }
        i = j.max(i + 1);
    }
    out.push_str(&src[last..]);
    out
}

/// FNV-1a 64 of the content → 8 base36 chars. Content-addressed: same bytes, same name, cache forever.
pub fn content_hash(s: &str) -> String {
    bytes_hash(s.as_bytes())
}

/// `content_hash` over raw bytes (binary assets such as images).
pub fn bytes_hash(s: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let mut out = String::new();
    for _ in 0..8 {
        let d = (h % 36) as u8;
        out.push(if d < 10 { (b'0' + d) as char } else { (b'a' + d - 10) as char });
        h /= 36;
    }
    out
}

/// Import specifiers of a compiled module: `from '<s>'`, `import '<s>'` and `import('<s>')`, with byte ranges.
pub fn specifiers(js: &str) -> Result<Vec<(usize, usize, String)>, String> {
    use brain_min::lex::{lex, Kind};
    let t = lex(js)?;
    let mut out = Vec::new();
    for i in 0..t.len() {
        if t[i].kind != Kind::Str {
            continue;
        }
        let prev = i.checked_sub(1).map(|j| &t[j]);
        let is_spec = prev.is_some_and(|p| p.ident("from") || p.ident("import"))
            || (prev.is_some_and(|p| p.punct("(")) && i >= 2 && t[i - 2].ident("import"));
        if is_spec {
            out.push((t[i].start, t[i].start + t[i].text.len(), t[i].text[1..t[i].text.len() - 1].to_string()));
        }
    }
    Ok(out)
}

/// Rewrites the import specifiers of a module (`from '<s>'`, `import '<s>'`, `import('<s>')`): `f` gets each
/// specifier as written and returns the replacement, or None to keep it. The one implementation shared by the build
/// (server modules, #51) and `brain run` (#187).
pub fn rewrite_imports(src: &str, f: &mut dyn FnMut(&str) -> Result<Option<String>, String>) -> Result<String, String> {
    let mut out = String::with_capacity(src.len());
    let mut last = 0;
    for (a, b, spec) in specifiers(src)? {
        if let Some(url) = f(&spec)? {
            out.push_str(&src[last..a]);
            out.push_str(&prerender_json(&url));
            last = b;
        }
    }
    out.push_str(&src[last..]);
    Ok(out)
}

/// `rewrite_imports` for relative specifiers only (`./x`, `../x`): `f` gets `orig_dir.join(spec)`.
pub fn rewrite_specifiers(src: &str, orig_dir: &Path, f: &mut dyn FnMut(&Path) -> Result<String, String>) -> Result<String, String> {
    rewrite_imports(src, &mut |spec| if spec.starts_with("./") || spec.starts_with("../") { f(&orig_dir.join(spec)).map(Some) } else { Ok(None) })
}

/// `new URL("<s>", import.meta.url)` references with byte ranges: how compiled code points at a Web Worker module
/// (`*.worker.jsbrain` / `*.worker.js`, INTERNALS §35.5).
pub fn worker_refs(js: &str) -> Result<Vec<(usize, usize, String)>, String> {
    use brain_min::lex::{lex, Kind};
    let t = lex(js)?;
    let mut out = Vec::new();
    for i in 3..t.len().saturating_sub(2) {
        if t[i].kind == Kind::Str && t[i - 1].punct("(") && t[i - 2].ident("URL") && t[i - 3].ident("new") && t[i + 1].punct(",") && t[i + 2].ident("import") {
            out.push((t[i].start, t[i].start + t[i].text.len(), t[i].text[1..t[i].text.len() - 1].to_string()));
        }
    }
    Ok(out)
}

/// Import specifiers and worker references of a module, in source order.
fn module_refs(js: &str) -> Result<Vec<(usize, usize, String)>, String> {
    let mut v = specifiers(js)?;
    v.extend(worker_refs(js)?);
    v.sort();
    Ok(v)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Node {
    /// App file, relative to root (`comp/counter.brain`).
    File(String),
    /// Runtime module by name (`brain.js`, `brain-form.js`).
    Runtime(String),
    /// Entry of a plain `*.worker.js` (the app file, relative to root): imports it and serves its exports through
    /// brain-worker.js, so the author writes nothing (`*.worker.jsbrain` gets that trailer from the compiler, #128).
    Worker(String),
}

struct Module {
    js: String,
    css: String,
    /// (specifier as written, resolved node)
    deps: Vec<(String, Node)>,
    /// Specifiers among `deps` that start a Web Worker: their modules run off the page (no modulepreload, no CSS).
    workers: Vec<String>,
}

fn resolve(from: &str, spec: &str) -> Result<Node, String> {
    if let Some(name) = spec.strip_prefix("/@brain/") {
        return Ok(Node::Runtime(name.to_string()));
    }
    if !(spec.starts_with("./") || spec.starts_with("../")) {
        return Err(format!("`{from}` imports `{spec}`: only relative files and the runtime can be bundled; fix: copy the code into lib/ as a .jsbrain"));
    }
    let base = Path::new(from).parent().unwrap_or(Path::new(""));
    let mut parts: Vec<String> = base.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    for seg in spec.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop().ok_or_else(|| format!("`{from}` imports `{spec}`, which leaves the app folder"))?;
            }
            s => parts.push(s.to_string()),
        }
    }
    Ok(Node::File(parts.join("/")))
}

/// Runtime exports (short names after `RUNTIME_MAP`) that make a page interactive: if none of the modules a view
/// reaches imports one of these, the page ships as plain prerendered HTML with no JavaScript.
const INTERACTIVE: &[&str] = &["o", "s", "d", "f", "b", "u", "r"];

/// Runtime exports whose effects run without a user event (`mounted()`, timers, media queries, `online()`, `route()`,
/// `resource()`…): a view that reaches any of them mounts on load; an island would only wake up on the first event
/// (QA #172 S22, QA #21 F4). `None` = every export of that module. Pure helpers (`debounce`) keep the island.
const LIVE: &[(&str, Option<&[&str]>)] = &[
    ("brain-env.js", Some(&["mounted", "watch", "every", "media", "visible", "dyn", "portal", "route"])),
    ("brain-data.js", Some(&["resource", "stream", "online"])),
    ("brain-ch.js", None),
    ("brain-persist.js", None),
    ("brain-lazy.js", None),
];

/// App-wide HTTP policy (`api/_http.jsbrain`, INTERNALS §40): loaded by every interactive page before it mounts.
const HTTP_SETUP: &str = "api/_http.jsbrain";

/// A view stylesheet up to this size (minified bytes) is inlined into the page instead of being a request (§13).
const INLINE_CSS: usize = 4096;

pub fn build(
    input: &Input,
    views: &[(String, String)],
    layouts_of: &dyn Fn(&str) -> Vec<String>,
    seo_of: &dyn Fn(&str) -> Vec<(String, String)>,
) -> Result<Output, String> {
    build_inner(input, views, layouts_of, seo_of, false)
}

/// `measure`: the same pipeline for `impact::measure` (§43): no SEO, caching layer, server output or errors for
/// `load`/`server:` without a server runtime; dynamic routes count once by their pattern. Prerender still runs (static
/// pages are their HTML) and is cached per unchanged view.
fn build_inner(
    input: &Input,
    views: &[(String, String)],
    layouts_of: &dyn Fn(&str) -> Vec<String>,
    seo_of: &dyn Fn(&str) -> Vec<(String, String)>,
    measure: bool,
) -> Result<Output, String> {
    let mut warnings = Vec::new();
    // brain.json `base` (#148): every URL the build writes starts with `pre` (`/app`, or empty at the root); dist/
    // file paths never do
    let base = read_base(&input.root);
    // `ext` (#26): `brain build --ext` (cache.ext) wins over brain.json; `web` by default
    let ext = if input.cache.ext.is_empty() { read_ext(&input.root) } else { Ext::from_name(&input.cache.ext) };
    // languages (§37): brain.json `i18n`; the views compile once and t() resolves at runtime from a dictionary chunk
    // per view and locale (only the keys that view reads), loaded before the page mounts
    let i18n = read_i18n(&input.root);
    let pre = base.trim_end_matches('/').to_string();
    let key_of = |u: &str| -> String { url_key(u, &pre).to_string() };
    let mut modules: HashMap<Node, Module> = HashMap::new();
    let mut needs_server: HashSet<String> = HashSet::new();
    // views whose server module exports something the build evaluates (seo/head/paths), and those with paths()
    let mut has_server: HashSet<String> = HashSet::new();
    let mut has_paths: HashSet<String> = HashSet::new();
    // dynamic views that list their pages at request time (`export async function sitemap()`): server.mjs adds them to
    // /sitemap.xml
    let mut has_sitemap: HashSet<String> = HashSet::new();
    // views whose forms post to a `server:` function (`export const $forms`, §38): server.mjs handles their POST
    let mut has_forms: HashSet<String> = HashSet::new();
    // views whose `load` only reads collections (#273): their data is fixed at build time, so they are prerendered
    let mut content_load: HashSet<String> = HashSet::new();
    // rel → (client js, server js, server fns) of every compiled file with server code
    let mut server_src: BTreeMap<String, (String, String, Vec<String>)> = BTreeMap::new();

    let progress = input.progress;
    let report = |phase: &str, done: usize, total: usize| {
        if let Some(f) = progress {
            f(phase, done, total)
        }
    };
    // Guards, middleware and error pages by folder (§22): `views/a/_guard.jsbrain` protects every view under views/a/,
    // outermost folder first; the nearest `_error.brain` renders a failure of a server route.
    let folder_chain = |view: &str, name: &str| -> Vec<String> {
        let parts: Vec<&str> = view.split('/').collect();
        let mut out = Vec::new();
        for i in 1..parts.len() {
            let f = format!("{}/{name}", parts[..i].join("/"));
            if input.root.join(&f).is_file() {
                out.push(f);
            }
        }
        out
    };
    let protected = |view: &str| !folder_chain(view, "_guard.jsbrain").is_empty() || !folder_chain(view, "_middleware.jsbrain").is_empty();
    let error_of = |view: &str| folder_chain(view, "_error.brain").pop();
    let has_server_runtime = input.server_mjs.is_some() && input.shim.is_some();
    // 1. Load the graph (DFS from every view and its layouts, plus the error pages server routes may render).
    let mut stack: Vec<Node> = Vec::new();
    // the client router is a graph module (it imports the core: its import must point at the hashed core, QA #172)
    if input.runtime_siblings.iter().any(|(n, _)| n == "brain-router.js") {
        stack.push(Node::Runtime("brain-router.js".into()));
    }
    if i18n.is_some() && input.runtime_siblings.iter().any(|(n, _)| n == "brain-i18n.js") {
        stack.push(Node::Runtime("brain-i18n.js".into()));
    }
    let http_setup = input.root.join(HTTP_SETUP).is_file();
    if http_setup {
        stack.push(Node::File(HTTP_SETUP.into()));
    }
    // app.brain's `<meta http …>` policy: its compiled module calls `http({…})` at the top (COMPILER), so the page
    // imports it for that side effect, before api/_http.jsbrain (whose headers add to the policy)
    let app_http = std::fs::read_to_string(input.root.join("app.brain")).is_ok_and(|s| s.contains("<meta http"));
    if app_http {
        stack.push(Node::File("app.brain".into()));
    }
    for (_, v) in views {
        stack.push(Node::File(v.clone()));
        for l in layouts_of(v) {
            stack.push(Node::File(l));
        }
        if let (true, Some(e)) = (has_server_runtime, error_of(v)) {
            for l in layouts_of(&e) {
                stack.push(Node::File(l));
            }
            stack.push(Node::File(e));
        }
        // the nearest `_loading.brain` (#235): the router mounts it while a slow page is on its way
        if let Some(l) = folder_chain(v, "_loading.brain").pop() {
            stack.push(Node::File(l));
        }
        // folder guards as client chunks (#283): the router imports one when a navigation enters its folder
        for g in folder_chain(v, "_guard.jsbrain") {
            stack.push(Node::File(g));
        }
    }
    while let Some(n) = stack.pop() {
        if modules.contains_key(&n) {
            continue;
        }
        report("compile", modules.len() + 1, 0);
        let (js, css) = match &n {
            Node::File(rel) => {
                let src = std::fs::read_to_string(input.root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
                if rel.ends_with(".js") {
                    (src, String::new())
                } else {
                    let u = (input.compile)(&src, Path::new(rel))?;
                    // Only `load` needs the server to render the page: since #104 server_js also carries `seo`/`head`
                    // (evaluated at build time) and `paths`, which a static build handles itself.
                    if u.server_js.contains("function load(") || u.server_js.contains("const load =") {
                        needs_server.insert(rel.clone());
                    }
                    if !u.server_js.trim().is_empty() {
                        has_server.insert(rel.clone());
                    }
                    if u.server_js.contains("function paths(") {
                        has_paths.insert(rel.clone());
                    }
                    if u.server_js.contains("function sitemap(") {
                        has_sitemap.insert(rel.clone());
                    }
                    if content_only_load(&u.server_js) {
                        content_load.insert(rel.clone());
                    }
                    if u.server_js.contains("const $forms") {
                        has_forms.insert(rel.clone());
                    }
                    if !u.server_js.trim().is_empty() {
                        server_src.insert(rel.clone(), (u.js.clone(), u.server_js.clone(), u.server_fns.clone()));
                    }
                    (u.js, u.css)
                }
            }
            Node::Runtime(name) if name == "brain.js" => (input.runtime_core.clone(), String::new()),
            // the ui/ kit (`<Button>` → `/@brain/ui/button.brain`): compiled like an app component
            Node::Runtime(name) if name.ends_with(".brain") => {
                let src = input.runtime_siblings.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
                let src = src.ok_or_else(|| format!("kit component `{name}` is not embedded; fix: pass runtime/{name} in runtime_siblings"))?;
                let u = (input.compile)(&src, Path::new(name))?;
                (u.js, u.css)
            }
            Node::Runtime(name) => {
                let src = input.runtime_siblings.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
                (src.ok_or_else(|| format!("runtime module `{name}` is not embedded"))?, String::new())
            }
            Node::Worker(rel) => {
                let file = rel.rsplit('/').next().unwrap_or(rel);
                (format!("import * as $m from \"./{file}\";\nimport {{ serve as $serve }} from \"/@brain/brain-worker.js\";\n$serve({{ ...$m }});\n"), String::new())
            }
        };
        let from = match &n {
            Node::File(r) | Node::Worker(r) => r.clone(),
            Node::Runtime(r) => format!("runtime/{r}"),
        };
        let mut deps = Vec::new();
        let worker_specs: Vec<String> = worker_refs(&js)?.into_iter().map(|(_, _, s)| s).collect();
        for (_, _, spec) in module_refs(&js)? {
            let d = match &n {
                // runtime siblings import ./brain.js; kit components import their neighbours (`ui/x` → `ui/y`)
                Node::Runtime(r) if spec.starts_with("./") => {
                    let dir = r.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
                    Node::Runtime(format!("{dir}{}", spec.trim_start_matches("./")))
                }
                _ => match resolve(&from, &spec)? {
                    // a plain JS worker gets the generated entry that serves its exports
                    Node::File(r) if worker_specs.contains(&spec) && r.ends_with(".js") => Node::Worker(r),
                    d => d,
                },
            };
            stack.push(d.clone());
            deps.push((spec, d));
        }
        modules.insert(n, Module { js, css, deps, workers: worker_specs });
    }

    // 1b. Production: class names renamed everywhere the compiler wrote them (#260, INTERNALS §11.6)
    let shell_src = std::fs::read_to_string(input.root.join("app.brain")).unwrap_or_default();
    let class_map: BTreeMap<String, String> = if input.minify {
        let (_, shell_style_src) = take_block(&shell_src, "style");
        let mut sheets: Vec<&str> = modules.values().map(|m| m.css.as_str()).collect();
        sheets.push(&shell_style_src);
        let codes: Vec<&str> = modules.values().map(|m| m.js.as_str()).collect();
        let mut public_css: Vec<String> = Vec::new();
        let mut stack = vec![input.root.join("public")];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "css") {
                    public_css.push(std::fs::read_to_string(&p).unwrap_or_default());
                }
            }
        }
        // the shell's own markup classes count as compiler-owned (they are renamed with the map below)
        let public_refs: Vec<&str> = public_css.iter().map(String::as_str).collect();
        classes::plan(&sheets, &codes, &public_refs)
    } else {
        BTreeMap::new()
    };
    if !class_map.is_empty() {
        for m in modules.values_mut() {
            m.js = classes::rename_js(&m.js, &class_map);
            m.css = classes::rename_css(&m.css, &class_map);
        }
    }
    // #287/#306: codegen in adoption mode (Options.adopt) imports fc/nx from brain-hydrate.js in every component
    // module. Without it no page is stamped, so brain-island's adopter is dead code: drop its static import there, and
    // brain-hydrate.js leaves the graph (no chunk, no modulepreload on every hydrating page).
    let adopt = modules.iter().any(|(n, m)| matches!(n, Node::File(_)) && m.js.contains("brain-hydrate.js"));
    if !adopt {
        let island = Node::Runtime("brain-island.js".into());
        if let Some(m) = modules.get_mut(&island) {
            if let Some(line) = m.js.lines().find(|l| l.trim_start().starts_with("import { adopter } from './brain-hydrate.js'")).map(String::from) {
                m.js = m.js.replacen(&line, "const adopter = () => { throw Error('adopt') }", 1);
                m.deps.retain(|(spec, _)| !spec.ends_with("brain-hydrate.js"));
            }
        }
        let hydrate = Node::Runtime("brain-hydrate.js".into());
        if !modules.values().any(|m| m.deps.iter().any(|(_, d)| *d == hydrate)) {
            modules.remove(&hydrate);
        }
    }
    // 2. Minify every module first (it needs the original runtime specifiers to apply RUNTIME_MAP); then tree-shake
    //    the runtime core down to what the graph imports; then emit chunks in post-order (dependencies first) so
    //    each import can point to its dependency's final hashed URL.
    let core_node = Node::Runtime("brain.js".into());
    let mut min_src: HashMap<Node, String> = HashMap::new();
    for (i, (n, m)) in modules.iter().enumerate() {
        report("minify", i + 1, modules.len());
        let src = if input.minify && *n != core_node {
            let b = cached_bytes(&input.root, "min", m.js.as_bytes(), || brain_min::minify_module(&m.js, true).map(String::into_bytes))?;
            String::from_utf8(b).map_err(|e| e.to_string())?
        } else {
            m.js.clone()
        };
        min_src.insert(n.clone(), src);
    }
    if input.minify && modules.contains_key(&core_node) {
        // pages always mount (`m`) or hydrate (`h`); internal exports used by sibling modules stay
        let mut keep: HashSet<String> = ["m", "h"].iter().map(|s| s.to_string()).collect();
        for (n, src) in &min_src {
            if *n == core_node {
                continue;
            }
            let from_core = |sp: &str| brain_min::is_runtime_spec(sp);
            keep.extend(brain_min::imported_names(src, &from_core)?);
        }
        let shaken = brain_min::tree_shake(&min_src[&core_node], &keep)?;
        min_src.insert(core_node.clone(), shaken);
        // sibling runtime modules (brain-ch.js, brain-form.js…): keep only the exports some module imports
        let siblings: Vec<String> = modules.keys().filter_map(|n| match n {
            // kit components are imported by default export: never shaken
            // kit components (default export) and the router (imported by the page script) keep all their exports
            Node::Runtime(r) if r != "brain.js" && !r.ends_with(".brain") => Some(r.clone()),
            _ => None,
        }).collect();
        for sib in siblings {
            let node = Node::Runtime(sib.clone());
            // the page script imports `router` from the router module (it is not a graph edge)
            // and the dictionary chunks (written later) import `add` from the i18n module
            let mut keep: HashSet<String> = match sib.as_str() {
                "brain-router.js" => ["router".to_string()].into(),
                "brain-i18n.js" => ["add".to_string()].into(),
                _ => HashSet::new(),
            };
            let is_it = |sp: &str| sp.ends_with(&format!("/{sib}")) || sp == format!("./{sib}");
            // a module loaded with import() is used through property access (`m.hydrate`): keep all its exports
            if min_src.iter().any(|(n, src)| *n != node && dynamic_import_of(src, &is_it)) {
                continue;
            }
            for (n, src) in &min_src {
                if *n == node {
                    continue;
                }
                keep.extend(brain_min::imported_names(src, &is_it)?);
            }
            let shaken = brain_min::tree_shake(&min_src[&node], &keep)?;
            min_src.insert(node, shaken);
        }
    }
    // public/ assets named in JS (`"/og.png"`) point at their immutable copy, before the chunk is hashed, so the
    // chunk's name changes with the asset (QA #124). Same URLs as cache.rs (`hashed_url`).
    if input.cache.http {
        let public: BTreeMap<String, String> = cache::public_urls(&input.root).into_iter().map(|(k, v)| (k, format!("{pre}{v}"))).collect();
        if !public.is_empty() {
            for (n, src) in min_src.iter_mut() {
                if matches!(n, Node::File(_)) {
                    *src = cache::rewrite(src, &public);
                }
            }
        }
    }
    // links inside the app's templates (`<a href=/about>`) go under the base too
    if !pre.is_empty() {
        for (n, src) in min_src.iter_mut() {
            if matches!(n, Node::File(_)) {
                *src = prefix_links(src, &pre);
            }
        }
    }
    let mut url_of: HashMap<Node, String> = HashMap::new();
    let mut files = BTreeMap::new();
    fn emit(
        n: &Node,
        modules: &HashMap<Node, Module>,
        min_src: &HashMap<Node, String>,
        url_of: &mut HashMap<Node, String>,
        visiting: &mut HashSet<Node>,
        files: &mut BTreeMap<String, String>,
        pre: &str,
        jsx: &str,
    ) -> Result<String, String> {
        if let Some(u) = url_of.get(n) {
            return Ok(u.clone());
        }
        if !visiting.insert(n.clone()) {
            return Err(format!("import cycle through {n:?}; fix: move the shared code into a lib/*.jsbrain both import"));
        }
        let m = &modules[n];
        let src = &min_src[n];
        let mut js = String::with_capacity(src.len());
        let mut last = 0;
        for (a, b, spec) in module_refs(src)? {
            let Some((_, d)) = m.deps.iter().find(|(s, _)| *s == spec) else { continue };
            let u = emit(d, modules, min_src, url_of, visiting, files, pre, jsx)?;
            js.push_str(&src[last..a]);
            js.push_str(&format!("\"{u}\""));
            last = b;
        }
        js.push_str(&src[last..]);
        let key = format!("_b/{}.{jsx}", content_hash(&js));
        let url = format!("{pre}/{key}");
        files.insert(key, js);
        visiting.remove(n);
        url_of.insert(n.clone(), url.clone());
        Ok(url)
    }
    let mut visiting = HashSet::new();
    let mut nodes: Vec<Node> = modules.keys().cloned().collect();
    nodes.sort();
    for n in &nodes {
        emit(n, &modules, &min_src, &mut url_of, &mut visiting, &mut files, &pre, ext.js())?;
    }

    // 3. Shared tokens CSS + one CSS file per view (styles of every module it reaches statically).
    let tokens = ["tokens.cssbrain", "tokens.brain.css"].iter().find_map(|t| std::fs::read_to_string(input.root.join(t)).ok()).unwrap_or_default();
    let tokens = if class_map.is_empty() { tokens } else { classes::rename_css(&tokens, &class_map) };
    let tokens_url = if tokens.trim().is_empty() {
        None
    } else {
        let css = minify_css(&format!("@layer tokens{{{tokens}}}"));
        let key = format!("_b/{}.{}", content_hash(&css), ext.css());
        files.insert(key.clone(), css);
        Some(format!("{pre}/{key}"))
    };

    report("pack", 0, views.len());
    // 4. Pages: plan each page, prerender the ones without server data, then assemble the HTML.
    let shell = std::fs::read_to_string(input.root.join("app.brain")).unwrap_or_else(|_| "<html lang=\"en\"><head><meta charset=\"utf-8\"></head><body><slot /></body></html>".into());
    let (shell, _) = take_block(&shell, "script");
    let shell = strip_meta_seo(&shell);
    let (shell, shell_style) = take_block(&shell, "style");
    let (shell, shell_style) = if class_map.is_empty() { (shell, shell_style) } else { (classes::rename_class_attrs(&shell, &class_map), classes::rename_css(&shell_style, &class_map)) };
    struct Plan {
        /// SSR page (served by dist/server.mjs): `route` is the pattern (`/users/:id`).
        ssr: bool,
        route: String,
        view: String,
        view_url: String,
        head: String,
        script: String,
        interactive: bool,
        prerender: bool,
        /// Route params as JSON (`{}` for static routes): `<meta name="brain-params">` for route() (RUNTIME #169).
        params: String,
        /// Reads a live runtime primitive: mounts on load instead of waiting as an island.
        live: bool,
        /// The view's stylesheet (`<style>` or `<link>`), placed first inside the region.
        css: String,
        /// i18n: the page's locale and its dictionary chunk (imported before the page mounts).
        locale: Option<String>,
        dict: Option<String>,
    }
    let mut plans: Vec<Plan> = Vec::new();
    let mut dicts: HashMap<String, String> = HashMap::new(); // route → its dictionary chunk URL
    // RUNTIME's runaway probe (runtime/shim/probe.js, passed as the sibling `shim/probe.js`); brain.json
    // `"probe": { "seconds": N }` (default 30, 0 turns it off)
    // #287: codegen in adoption mode (Options.adopt) imports fc/nx from brain-hydrate.js in every component module;
    // then the manifest says `adopt`, server.mjs and the prerender stamp the HTML, and islands resume on it
    let hydrate_src: Option<&str> = if adopt { input.runtime_siblings.iter().find(|(n, _)| n == "brain-hydrate.js").map(|(_, s)| s.as_str()) } else { None };
    let probe_src = input.runtime_siblings.iter().find(|(n, _)| n == "shim/probe.js").map(|(_, s)| s.as_str()).filter(|_| !measure);
    let probe_seconds: u32 = std::fs::read_to_string(input.root.join("brain.json"))
        .ok()
        .and_then(|s| {
            let i = s.find("\"probe\"")?;
            let j = i + s[i..].find("\"seconds\"")?;
            s[j + 9..].trim_start().strip_prefix(':')?.trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
        })
        .unwrap_or(30);
    // `api/_http.jsbrain` runs first on every interactive page (its headers apply to every api() call)
    let mut http_import = String::new();
    if app_http {
        http_import.push_str(&format!("import\"{}\";", url_of[&Node::File("app.brain".into())]));
    }
    if http_setup {
        http_import.push_str(&format!("import\"{}\";", url_of[&Node::File(HTTP_SETUP.into())]));
    }
    let mut jobs: Vec<prerender::Job> = Vec::new();
    let core = url_of[&Node::Runtime("brain.js".into())].clone();
    // Server modules, written once per view that has one (only when prerender can evaluate them).
    let mut server_of: HashMap<String, (String, String)> = HashMap::new();
    if let (Some(f), true) = (input.server_module, input.shim.is_some()) {
        for (_, view) in views {
            if has_server.contains(view) {
                server_of.insert(view.clone(), f(view)?);
            }
        }
    }
    // Dynamic routes with paths() expand to one page per param set (INTERNALS §32); the rest wait for SSR (#51).
    let mut pages_of: Vec<(String, String, String, bool, bool)> = Vec::new(); // (route, view, params JSON, build-time load, ssr)
    let can_ssr = input.server_mjs.is_some() && input.shim.is_some();
    let want_paths: Vec<(String, String)> = views
        .iter()
        .filter(|(r, v)| routes::is_dynamic(r) && has_paths.contains(v) && server_of.contains_key(v))
        .map(|(_, v)| (v.clone(), server_of[v].0.clone()))
        .collect();
    let listed = match (&input.shim, want_paths.is_empty()) {
        (Some(shim), false) => prerender::paths(&files, shim, &want_paths)?,
        _ => BTreeMap::new(),
    };
    for (route, view) in views {
        // a protected view never becomes a static file: only the server can run its guards (§22)
        let guarded = route != "*" && protected(view);
        if guarded && !can_ssr && !measure {
            return Err(format!(
                "{view}: is protected by {}, but this build has no server runtime (runtime/server/server.mjs) to run it; fix: rebuild brain with the runtime",
                folder_chain(view, "_guard.jsbrain").into_iter().chain(folder_chain(view, "_middleware.jsbrain")).collect::<Vec<_>>().join(", ")
            ));
        }
        if guarded && can_ssr {
            pages_of.push((route.clone(), view.clone(), "{}".into(), false, true));
            continue;
        }
        // the not-found view (`*`) without load is a static page, dist/404.html (static hosts and server.mjs use it)
        // a `load` that only reads `brain:content` runs at build time, as with paths() (#273): the collection index is a
        // static page (sitemap, llms.txt, .md) and needs no server; it is rebuilt with the content
        if !routes::is_dynamic(route) && route != "*" && needs_server.contains(view) && content_load.contains(view) && input.shim.is_some() && server_of.contains_key(view) {
            pages_of.push((route.clone(), view.clone(), "{}".into(), true, false));
            continue;
        }
        if !routes::is_dynamic(route) && (route != "*" || !needs_server.contains(view)) {
            let ssr = can_ssr && needs_server.contains(view);
            pages_of.push((route.clone(), view.clone(), "{}".into(), false, ssr));
            // HTML first (§38): a page whose forms post to the server is prerendered for GET and also gets a server
            // template, so server.mjs can re-render it with the errors (422) or redirect (303) without JavaScript
            if !ssr && can_ssr && has_forms.contains(view) && route != "*" {
                pages_of.push((route.clone(), view.clone(), "{}".into(), false, true));
            }
            continue;
        }
        match listed.get(view) {
            Some(Ok(list)) => {
                for params in list {
                    match concrete_route(route, params) {
                        Some(r) => pages_of.push((r, view.clone(), params_json(params), needs_server.contains(view), false)),
                        None => warnings.push(format!("{view}: paths() returned {params:?}, which does not fill every param of `{route}`")),
                    }
                }
            }
            Some(Err(e)) => return Err(format!("{view}: paths() threw: {}", e.lines().next().unwrap_or(""))),
            None if can_ssr => pages_of.push((route.clone(), view.clone(), "{}".into(), false, true)),
            None if measure => pages_of.push((route.clone(), view.clone(), "{}".into(), false, false)),
            None => warnings.push(format!("{view}: dynamic route `{route}` has no paths() and the server runtime is not embedded; it is left out of dist/")),
        }
    }
    // §37 routing: the default locale has no prefix, the others do (`/es/about`), or the view's translated slug
    // (`<meta route es="/es/acerca">`); views are not duplicated, only their pages
    let mut locale_of: HashMap<String, String> = HashMap::new();
    if let Some((def, locs)) = &i18n {
        let base_pages = pages_of.clone();
        for p in &base_pages {
            locale_of.insert(p.0.clone(), def.clone());
        }
        for loc in locs.iter().filter(|l| *l != def) {
            for (route, view, params, bl, ssr) in &base_pages {
                if route == "*" || route.starts_with('!') {
                    continue;
                }
                let src = std::fs::read_to_string(input.root.join(view.as_str())).unwrap_or_default();
                let slug = route_meta(&src, loc).filter(|s| s.starts_with('/') && !routes::is_dynamic(route));
                let r = slug.unwrap_or_else(|| if route == "/" { format!("/{loc}") } else { format!("/{loc}{route}") });
                locale_of.insert(r.clone(), loc.clone());
                pages_of.push((r, view.clone(), params.clone(), *bl, *ssr));
            }
        }
    }
    // the same page in every locale (same view and params), for <LangSwitch> and the server manifest
    let alternates: HashMap<String, String> = pages_of
        .iter()
        .filter(|me| locale_of.contains_key(&me.0))
        .map(|me| {
            let list: Vec<String> = pages_of
                .iter()
                .filter(|p| p.1 == me.1 && p.2 == me.2 && locale_of.contains_key(&p.0))
                .map(|p| format!("{{\"locale\":{},\"href\":{}}}", prerender_json(&locale_of[&p.0]), prerender_json(&format!("{pre}{}", p.0))))
                .collect();
            (me.0.clone(), format!("[{}]", list.join(",")))
        })
        .collect();
    let alternates_of = |route: &str| -> String { alternates.get(route).cloned().unwrap_or_default() };
    // server templates for the error pages (`!error:<view>`) and views/403.brain (`!forbidden`): never routes
    if can_ssr && pages_of.iter().any(|p| p.4) {
        let mut errs: Vec<String> = pages_of.iter().filter(|p| p.4).filter_map(|p| error_of(&p.1)).collect();
        errs.sort();
        errs.dedup();
        for e in errs {
            pages_of.push((format!("!error:{e}"), e.clone(), "{}".into(), false, true));
        }
        if views.iter().any(|(_, v)| v == "views/403.brain") {
            pages_of.push(("!forbidden".into(), "views/403.brain".into(), "{}".into(), false, true));
        }
    }
    for (route, view, params, build_load, ssr) in &pages_of {
        let has_load = needs_server.contains(view) && !*build_load;
        if has_load && !*ssr && !measure {
            // Without dist/server.mjs nothing runs load(), so the page would mount with undefined props and render
            // broken (QA #70). Failing is better than shipping that page.
            return Err(format!(
                "{view}: exports `load`, but this build has no server runtime (runtime/server/server.mjs) to run it; fix: rebuild brain with the runtime, or add paths() / move the data into the view for a static page"
            ));
        }
        let layouts = layouts_of(view);
        let mut reach: Vec<Node> = Vec::new();
        let mut seen = HashSet::new();
        let mut st: Vec<Node> = std::iter::once(Node::File(view.clone())).chain(layouts.iter().cloned().map(Node::File)).collect();
        while let Some(n) = st.pop() {
            if !seen.insert(n.clone()) {
                continue;
            }
            let m = &modules[&n];
            // only the imports the emitted chunk still has: the minifier drops unused ones, and preloading a module
            // nobody imports (often an empty chunk) is a wasted request on every page (QA #292)
            let emitted = files.get(&key_of(&url_of[&n])).map(String::as_str).unwrap_or("");
            for (spec, d) in &m.deps {
                if !m.workers.contains(spec) && url_of.get(d).is_some_and(|u| emitted.contains(&format!("\"{u}\""))) {
                    st.push(d.clone());
                }
            }
            if !emitted.trim().is_empty() || matches!(n, Node::File(ref f) if *f == *view || layouts.contains(f)) {
                reach.push(n);
            }
        }
        reach.sort();
        // Keep lazy component CSS in the route stylesheet, but preload only static JS imports.
        // A dynamic import is an explicit loading boundary, including its transitive dependencies.
        let mut preload = HashSet::new();
        let mut pending: Vec<Node> = std::iter::once(Node::File(view.clone())).chain(layouts.iter().cloned().map(Node::File)).collect();
        while let Some(n) = pending.pop() {
            if !preload.insert(n.clone()) { continue; }
            let emitted = files.get(&key_of(&url_of[&n])).map(String::as_str).unwrap_or("");
            let tokens = brain_min::lex::lex(emitted)?;
            let static_specs: HashSet<&str> = tokens.windows(2).filter_map(|w| {
                (w[1].kind == brain_min::lex::Kind::Str && (w[0].ident("from") || w[0].ident("import")))
                    .then(|| &w[1].text[1..w[1].text.len() - 1])
            }).collect();
            for (spec, child) in &modules[&n].deps {
                if !modules[&n].workers.contains(spec) && url_of.get(child).is_some_and(|url| static_specs.contains(url.as_str())) {
                    pending.push(child.clone());
                }
            }
        }
        let live_urls: Vec<(&String, Option<&[&str]>)> = LIVE.iter().filter_map(|(n, names)| url_of.get(&Node::Runtime(n.to_string())).map(|u| (u, *names))).collect();
        let live = reach.iter().any(|n| {
            let src = files.get(&key_of(&url_of[n])).map(String::as_str).unwrap_or("");
            matches!(n, Node::File(_))
                && live_urls.iter().any(|(u, names)| {
                    let used = brain_min::imported_names(src, &|sp: &str| sp == u.as_str()).unwrap_or_default();
                    match names {
                        Some(list) => used.iter().any(|x| list.contains(&x.as_str())),
                        None => !used.is_empty() || specifiers(src).is_ok_and(|sp| sp.iter().any(|(_, _, s)| s == *u)),
                    }
                })
        });
        let interactive = has_load
            || *ssr
            || live
            || reach.iter().any(|n| {
                let src = files.get(&key_of(&url_of[n])).map(String::as_str).unwrap_or("");
                matches!(n, Node::File(_)) && brain_min::imported_names(src, &|sp: &str| sp == core).is_ok_and(|names| names.iter().any(|x| INTERACTIVE.contains(&x.as_str())))
            });
        // SEO tags are added after the render: `seo(props)` needs the props (`head` here is the rest)
        let mut head = input.layer_order.clone();
        if let Some(t) = &tokens_url {
            head.push_str(&format!("<link rel=\"stylesheet\" href=\"{t}\">"));
        }
        let css: String = reach.iter().map(|n| modules[n].css.as_str()).collect::<Vec<_>>().join("\n");
        // a ui/ child inlined into two parents brings its scoped CSS twice (COMPILER §13): keep one copy
        let css = dedupe_scopes(&css);
        let css = format!("{}{css}", if shell_style.trim().is_empty() { String::new() } else { format!("@layer view{{{shell_style}}}") });
        // the view's CSS travels inside the region (#35): the router swaps it with the view, so a client navigation
        // brings the next view's styles and drops the old ones; small sheets go inline (critical CSS, no request)
        let mut view_css = String::new();
        if !css.trim().is_empty() {
            let css = minify_css(&css);
            if css.len() <= INLINE_CSS {
                view_css = format!("<style>{css}</style>");
            } else {
                let key = format!("_b/{}.{}", content_hash(&css), ext.css());
                view_css = format!("<link rel=\"stylesheet\" href=\"{pre}/{key}\">");
                files.insert(key, css);
            }
        }
        let v = url_of[&Node::File(view.clone())].clone();
        let layout_urls: Vec<String> = layouts.iter().map(|l| url_of[&Node::File(l.clone())].clone()).collect();
        let mut script = String::new();
        if interactive {
            for n in reach.iter().filter(|n| preload.contains(*n)) {
                head.push_str(&format!("<link rel=\"modulepreload\" href=\"{}\">", url_of[n]));
            }
            let mut imports = format!("{http_import}import{{m}}from\"{core}\";import V from\"{v}\";");
            let mut tree = format!("V({PROPS})");
            for (i, l) in layout_urls.iter().enumerate().rev() {
                imports.push_str(&format!("import L{i} from\"{l}\";"));
                tree = format!("L{i}({{children:()=>{tree}}})");
            }
            let mount = match tree.strip_prefix("L0(").and_then(|t| t.strip_suffix(')')) {
                Some(props) => format!("e.$d=m(L0,e,{props})"),
                None => format!("e.$d=m(V,e,{PROPS})"),
            };
            // Client mount (views with `load` or without prerender). The script lives inside the region so the router
            // re-runs it after a swap; the dispose goes to `e.$d` so the router can tear the view down (RUNTIME contract).
            script = format!(
                "<script type=\"module\">{imports}const e=document.getElementById('brain-view');e.$d&&e.$d();for(const c of[...e.childNodes])c.matches&&c.matches('style,link')||c.remove();{mount}</script>"
            );
        }
        // the page's dictionary (§37): the keys its modules read with t() (all of them when one call is dynamic), in
        // its locale, falling back to the default locale's text for keys it lacks
        let locale = locale_of.get(route).cloned();
        let mut dict_url: Option<String> = None;
        if let (Some(loc), Some((def, _)), Some(rt)) = (&locale, &i18n, url_of.get(&Node::Runtime("brain-i18n.js".into()))) {
            // client modules and the view's server code (`<meta seo title={t('…')}>` is evaluated there, QA #153)
            let mut keys: Vec<String> = reach.iter().filter(|n| matches!(n, Node::File(_))).flat_map(|n| i18n_keys_of(&modules[n].js)).collect();
            if let Some((_, sjs, _)) = server_src.get(view.as_str()) {
                keys.extend(i18n_keys_of(sjs));
            }
            keys.sort();
            keys.dedup();
            // a page that validates (form(), api() schemas) reads its messages from the `form.*` keys (§14, §37)
            let validates = reach.iter().any(|n| matches!(n, Node::Runtime(r) if r == "brain-schema.js"));
            let mut entries: Vec<String> = Vec::new();
            if !keys.is_empty() || validates {
                let mine = read_dict(&input.root, loc);
                let fallback = if loc == def { Vec::new() } else { read_dict(&input.root, def) };
                let all = keys.iter().any(|k| k == "*");
                let mut seen: HashSet<String> = HashSet::new();
                for (k, v) in mine.iter().chain(fallback.iter()) {
                    if (all || keys.contains(k) || (validates && k.starts_with("form."))) && seen.insert(k.clone()) {
                        entries.push(format!("{}:{v}", prerender_json(k)));
                    }
                }
            }
            if !keys.is_empty() || !entries.is_empty() {
                let js = format!("import{{add}}from\"{rt}\";add({},{{{}}});\n", prerender_json(loc), entries.join(","));
                let key = format!("_b/i18n-{}-{loc}.{}.{}", view_slug(view), content_hash(&js), ext.js());
                dict_url = Some(format!("{pre}/{key}"));
                dicts.insert(route.clone(), format!("{pre}/{key}"));
                files.insert(key, js);
            }
        }
        let prerender = input.shim.is_some() && !has_load && !*ssr;
        if prerender {
            jobs.push(prerender::Job {
                route: route.clone(),
                core: core.clone(),
                view: v,
                layouts: layout_urls,
                server: server_of.get(view).map(|(u, _)| u.clone()),
                params: params.clone(),
                load: *build_load,
                locale: locale.clone(),
                i18n: dict_url.clone(),
                alternates: alternates_of(route),
                // §30.2: interactive pages are stepped with fake timers to catch runaway growth (#284)
                probe: if interactive && probe_src.is_some() { probe_seconds } else { 0 },
                hydrate: hydrate_src.is_some(),
            });
        }
        plans.push(Plan { ssr: *ssr, route: route.clone(), view: view.clone(), view_url: url_of[&Node::File(view.clone())].clone(), head, script, interactive, prerender, params: params.clone(), live, css: view_css, locale, dict: dict_url });
    }
    // More than one page: the client router (brain-router.js) turns link clicks into fetch + region swap.
    let router_tag = if plans.len() > 1 {
        match url_of.get(&Node::Runtime("brain-router.js".into())) {
            Some(u) => {
                // #235: `loading` = per route (most specific first) the chunk of its nearest _loading.brain; the router
                // mounts it with `core` when the next page takes longer than 150 ms (RUNTIME contract)
                let mut with_loading: Vec<(&String, String)> =
                    views.iter().filter(|(r, _)| r != "*").filter_map(|(r, v)| folder_chain(v, "_loading.brain").pop().map(|l| (r, url_of[&Node::File(l)].clone()))).collect();
                with_loading.sort_by(|a, b| routes::order(a.0, b.0));
                let loading = if with_loading.is_empty() {
                    String::new()
                } else {
                    let items: Vec<String> = with_loading.iter().map(|(r, u)| format!("{{rx:{},view:{}}}", prerender_json(&routes::regex_source(r)), prerender_json(u))).collect();
                    format!(",core:{},loading:[{}]", prerender_json(&core), items.join(","))
                };
                // #283: `guards` = folder route → chunk of its `_guard.jsbrain` (RUNTIME runs them outermost first)
                let mut guard_dirs: BTreeMap<String, String> = BTreeMap::new();
                for (_, v) in views.iter() {
                    for g in folder_chain(v, "_guard.jsbrain") {
                        let dir = g.trim_start_matches("views").trim_end_matches("/_guard.jsbrain").to_string();
                        let dir = if dir.is_empty() { "/".to_string() } else { dir };
                        if let Some(u) = url_of.get(&Node::File(g.clone())) {
                            guard_dirs.insert(dir, u.clone());
                        }
                    }
                }
                let guards = if guard_dirs.is_empty() {
                    String::new()
                } else {
                    let items: Vec<String> = guard_dirs.iter().map(|(d, u)| format!("{}:{}", prerender_json(d), prerender_json(u))).collect();
                    format!(",guards:{{{}}}", items.join(","))
                };
                format!("<script type=\"module\">import{{router}}from\"{u}\";router({{region:\"#brain-view\"{loading}{guards}}})</script>")
            }
            None => {
                warnings.push("brain-router.js is not embedded: pages navigate with full reloads".into());
                String::new()
            }
        }
    } else {
        String::new()
    };
    // Speculation Rules (§13, §19): hovering a link to another static page prerenders it, so the click is instant.
    // Only build-time pages: they have no server side effects (SSR and guarded routes are never listed).
    let static_routes: Vec<String> = plans.iter().filter(|p| !p.ssr && !p.route.starts_with('!') && p.route != "*" && !routes::is_dynamic(&p.route)).map(|p| format!("{pre}{}", p.route)).collect();
    let speculation = if !router_tag.is_empty() && static_routes.len() > 1 {
        let list = static_routes.iter().map(|r| prerender_json(r)).collect::<Vec<_>>().join(",");
        format!("<script type=\"speculationrules\">{{\"prerender\":[{{\"source\":\"document\",\"where\":{{\"href_matches\":[{list}]}},\"eagerness\":\"moderate\"}}]}}</script>")
    } else {
        String::new()
    };
    // Prerender is deterministic given the chunks it runs, and chunk URLs are content hashes: a job whose URLs did
    // not change reuses the cached HTML (.brain/cache/pre/), so only edited views start Node.
    // The server module's content (seo/head code) is part of the key; a build-time `load` reads live data, so its
    // pages are never cached.
    let job_key = |j: &prerender::Job| {
        let srv = j.server.as_ref().and_then(|u| server_of.values().find(|(x, _)| x == u)).map_or(String::new(), |(_, c)| content_hash(c));
        format!("{}|{}|{}|{}|{}|{srv}|{:?}|{:?}|{}|{}|{}", j.route, j.core, j.view, j.layouts.join(","), j.params, j.locale, j.i18n, j.alternates, j.probe, j.hydrate)
    };
    let mut rendered: BTreeMap<String, Result<prerender::Rendered, String>> = BTreeMap::new();
    let mut todo: Vec<prerender::Job> = Vec::new();
    for j in jobs {
        if j.load {
            todo.push(j);
            continue;
        }
        let key = job_key(&j);
        match cached_bytes(&input.root, "pre", key.as_bytes(), || Err("miss".into())).ok().and_then(|b| decode_rendered(&b)) {
            Some(r) => {
                rendered.insert(j.route.clone(), Ok(r));
            }
            None => todo.push(j),
        }
    }
    if !todo.is_empty() {
        report("prerender", 0, todo.len());
    }
    if let (Some(shim), false) = (&input.shim, todo.is_empty()) {
        match prerender::run_probed(&files, shim, &prerender::Aux { probe: probe_src, hydrate: hydrate_src }, &todo) {
            Ok(fresh) => {
                for j in todo.iter().filter(|j| !j.load) {
                    if let Some(Ok(r)) = fresh.get(&j.route) {
                        let bytes = encode_rendered(r);
                        let _ = cached_bytes(&input.root, "pre", job_key(j).as_bytes(), || Ok(bytes));
                    }
                }
                rendered.extend(fresh);
            }
            Err(e) => warnings.push(format!("prerender skipped: {e}")),
        }
    }
    // B0907 (§30.2): any runaway finding fails the build, naming the component file (data-b hash → `C.f`)
    let mut runaway: Vec<String> = Vec::new();
    for (route, r) in rendered.iter() {
        let Ok(r) = r else { continue };
        for f in &r.probe {
            let p: Vec<&str> = f.splitn(5, '|').collect();
            let (comp, what, growth, msg, fix) = (p[0], p.get(1).unwrap_or(&""), p.get(2).unwrap_or(&""), p.get(3).unwrap_or(&""), p.get(4).unwrap_or(&""));
            let file = component_file(&modules, comp).unwrap_or_else(|| if comp.is_empty() { "a component".into() } else { format!("component {comp}") });
            runaway.push(format!("B0907 {file} on {route}: {msg} ({what} {growth}); fix: {fix}"));
        }
    }
    if !runaway.is_empty() && !measure {
        runaway.sort();
        runaway.dedup();
        return Err(runaway.join("\n"));
    }
    let app_seo = seo_of("app.brain");
    // With client navigation, a static page can become the host of an interactive one: every page then registers
    // hydrate() so islands arriving through the router resume (the core is one cached chunk).
    let hydrate_everywhere = !router_tag.is_empty() && plans.iter().any(|p| p.interactive);
    // Islands resume through the core's hydrate(), which loads brain-island.js and hands it the core functions
    // (RUNTIME #132; the island never imports the core: that would be a hash cycle). The island is preloaded.
    let hydrate_tag = format!("<script type=\"module\">{http_import}import{{h}}from\"{core}\";h()</script>");
    let island_preload = url_of.get(&Node::Runtime("brain-island.js".into())).map(|u| format!("<link rel=\"modulepreload\" href=\"{u}\">")).unwrap_or_default();
    let mut pages = Vec::new();
    for p in plans {
        let r = rendered.get(&p.route).and_then(|r| r.as_ref().ok());
        // SEO: app.brain defaults (titleTemplate, og:image…) < the view's static `<meta seo>` < `seo(props)`
        let mut seo: Vec<(String, String)> = app_seo.iter().filter(|(k, _)| k != "title" && k != "canonical" && k != "description").cloned().collect();
        let over = |seo: &mut Vec<(String, String)>, kv: &[(String, String)]| {
            for (k, v) in kv {
                if k == "seo" || k == "keyword" || v.starts_with('{') && k != "jsonld" {
                    continue; // unevaluated expressions from the static fallback, and audit-only fields
                }
                seo.retain(|(a, _)| a != k);
                seo.push((k.clone(), v.clone()));
            }
        };
        let static_seo = seo_of(&p.view);
        over(&mut seo, &static_seo);
        if let Some(r) = r {
            // the compiler already applied app.brain's titleTemplate inside seo(props): never twice
            if r.seo.iter().any(|(k, _)| k == "title") {
                seo.retain(|(k, _)| k != "titleTemplate");
            }
            over(&mut seo, &r.seo);
        }
        // canonical/og:url = site + base + route (#148; brain-seo builds its own absolute URLs the same way)
        let site = input.site.as_deref().map(|s| format!("{}{pre}", s.trim_end_matches('/')));
        let (title, seo_html) = prerender::seo_head(&seo, site.as_deref(), &p.route, &input.generator);
        let props = r.map_or("{}".to_string(), |r| r.props.replace("</", "<\\/"));
        let mut head = format!("{seo_html}{}{}", p.head, r.map_or("", |r| r.head.as_str()));
        if !p.ssr && p.params.trim() != "{}" && !p.params.trim().is_empty() {
            head.push_str(&format!("<meta name=\"brain-params\" content='{}'>", p.params.replace('&', "&amp;").replace('\'', "&#39;")));
        }
        let body_html = match rendered.get(&p.route) {
            Some(Ok(h)) => {
                // the page still ships (with the fallback), but the author must know (QA #176)
                for f in &h.failed {
                    warnings.push(format!("{}: component failed during prerender of {}: {f}; the page shows its fallback", p.view, p.route));
                }
                h.html.clone()
            }
            Some(Err(_)) if p.ssr => String::new(),
            Some(Err(e)) => {
                warnings.push(format!("{}: prerender threw, the page renders on the client: {}", p.view, e.lines().next().unwrap_or("")));
                String::new()
            }
            None => String::new(),
        };
        if !p.interactive && body_html.is_empty() && p.prerender {
            warnings.push(format!("{}: static view rendered empty", p.view));
        }
        // Islands (INTERNALS §6.1): prerendered interactive view → its root element is followed by
        // `<script type="b/state">` and the page only calls hydrate(); the view chunk loads on the first event.
        // This also survives client-side navigation (router swaps the region, hydrate listens on document).
        let mut script = p.script.replace(PROPS, &props);
        let mut body_html = body_html;
        if p.ssr {
            // server.mjs fills the markers (RUNTIME #98 contract); the mount reads the props before clearing the region
            body_html = "<!--brain:ssr--><script type=\"application/json\" id=\"brain-props\"><!--brain:props--></script>".into();
            script = p
                .script
                .replace("const e=document.getElementById('brain-view');", "const e=document.getElementById('brain-view'),P=JSON.parse(document.getElementById('brain-props').textContent);")
                .replace(PROPS, "P");
        } else if p.interactive && !body_html.is_empty() && (p.live || r.is_some_and(|r| r.fetched)) {
            // the render asked for data: keep the prerendered loading state and mount on load so the client fetches
            // (an island would wait for the first event)
        } else if p.interactive && !body_html.is_empty() {
            let hash = files.get(&key_of(&p.view_url)).and_then(|js| view_hash(js));
            if let Some(island) = hash.and_then(|h| island(&body_html, &h, &p.view_url)) {
                body_html = island;
                script = hydrate_tag.clone();
            } else {
                warnings.push(format!("{}: no island root found; the page mounts on load instead", p.view));
            }
        }
        if script.is_empty() && hydrate_everywhere {
            script = hydrate_tag.clone();
        }
        let head = if script == hydrate_tag { format!("{head}{island_preload}") } else { head };
        // the dictionary loads before the page mounts or resumes (t() is synchronous)
        // (a static page renders its text at build: its dictionary only served the prerender)
        if let (Some(d), true) = (&p.dict, p.interactive && script.starts_with("<script type=\"module\">")) {
            script = script.replacen("<script type=\"module\">", &format!("<script type=\"module\">import\"{d}\";"), 1);
        }
        // page scripts go INSIDE the region (the router recreates them after a swap); the router itself stays outside
        let slot = format!("<div id=\"brain-view\">{}{body_html}{script}</div>{router_tag}{speculation}", p.css);
        let mut doc = shell.replace("<slot />", &slot).replace("<slot/>", &slot);
        if let (Some(loc), Some((def, _))) = (&p.locale, &i18n) {
            doc = html_lang(&doc, loc, def);
        }
        if let Some(t) = &title {
            let t = t.replace('&', "&amp;").replace('<', "&lt;");
            doc = match (doc.find("<title>"), doc.find("</title>")) {
                (Some(a), Some(b)) if a < b => format!("{}<title>{t}</title>{}", &doc[..a], &doc[b + 8..]),
                _ => doc.replacen("</head>", &format!("<title>{t}</title></head>"), 1),
            };
        }
        let html = match doc.find("</head>") {
            Some(i) => format!("<!doctype html>{}{}{}", &doc[..i], head, &doc[i..]),
            None => format!("<!doctype html>{}{doc}", head),
        };
        let html = prerender::dedupe_head(&html);
        let file = if p.ssr {
            format!("_server/pages/{}.html", content_hash(&p.route))
        } else if p.route == "/" {
            format!("index.{}", ext.html())
        } else if p.route == "*" {
            format!("404.{}", ext.html())
        } else {
            format!("{}/index.{}", p.route.trim_matches('/'), ext.html())
        };
        files.insert(file.clone(), html.clone());
        pages.push(Page { route: p.route, file, html, view: p.view });
    }
    // 4a. Server output (#51): server modules, RPC map, manifest, server.mjs and the DOM shim for SSR
    let mut ssr_pages: Vec<(&Page, &String)> = pages.iter().filter(|p| p.file.starts_with("_server/")).filter_map(|p| pages_of.iter().find(|x| x.0 == p.route && x.4).map(|x| (p, &x.1))).collect();
    // pseudo pages (`!error:…`, `!forbidden`) are templates the routes point at, not routes
    let special: Vec<(&Page, &String)> = ssr_pages.iter().filter(|(p, _)| p.route.starts_with('!')).cloned().collect();
    ssr_pages.retain(|(p, _)| !p.route.starts_with('!'));
    // the one route table (#187): manifest routes in match order, each with the regex server.mjs uses
    ssr_pages.sort_by(|a, b| routes::order(&a.0.route, &b.0.route));
    let rpc_files: Vec<&String> = server_src.iter().filter(|(_, (_, _, f))| !f.is_empty()).map(|(r, _)| r).collect();
    if !ssr_pages.is_empty() || !rpc_files.is_empty() {
        match &input.server_mjs {
            None => {
                if !rpc_files.is_empty() && !measure {
                    return Err(format!("{}: has `server:` functions, but this build has no server runtime (runtime/server/server.mjs); fix: rebuild brain with the runtime", rpc_files[0]));
                }
            }
            Some(server_mjs) => {
                let runtime_url = |name: &str| url_of.get(&Node::Runtime(name.to_string())).cloned();
                let mut written: BTreeMap<String, String> = BTreeMap::new(); // rel → _server/<h>.mjs
                let mut routes_json = Vec::new();
                // a template page as server.mjs renders it: `{view, layouts, page}`
                let template = |view: &str, page: &str| {
                    let layouts: Vec<String> = layouts_of(view).iter().map(|l| prerender_json(&url_of[&Node::File(l.clone())])).collect();
                    format!("{{\"view\":{},\"layouts\":[{}],\"page\":{}}}", prerender_json(&url_of[&Node::File(view.to_string())]), layouts.join(","), prerender_json(page))
                };
                let template_of = |key: &str| special.iter().find(|(p, _)| p.route == key).map(|(p, v)| template(v, &p.file));
                for (p, view) in &ssr_pages {
                    let srv = if server_src.contains_key(*view) { Some(server::emit(&input.root, view, input.compile, &runtime_url, &mut written, &mut files, 0)?) } else { None };
                    let layouts: Vec<String> = layouts_of(view).iter().map(|l| prerender_json(&url_of[&Node::File(l.clone())])).collect();
                    // §22: guards and middleware as server modules, outermost folder first; the nearest _error page
                    let mut chain = String::new();
                    for (key, name) in [("guards", "_guard.jsbrain"), ("middleware", "_middleware.jsbrain")] {
                        let mut mods = Vec::new();
                        for f in folder_chain(view, name) {
                            mods.push(prerender_json(&server::emit(&input.root, &f, input.compile, &runtime_url, &mut written, &mut files, 0)?));
                        }
                        if !mods.is_empty() {
                            chain.push_str(&format!(",\"{key}\":[{}]", mods.join(",")));
                        }
                    }
                    if let Some(t) = error_of(view).and_then(|e| template_of(&format!("!error:{e}"))) {
                        chain.push_str(&format!(",\"error\":{t}"));
                    }
                    let forms_only = !needs_server.contains(*view) && !protected(view) && has_forms.contains(*view);
                    // #235 / #232: nearest _loading chunk, `<meta cache="N">`, streaming when the head is static, and the
                    // page's preloads (filled after the SRI pass, below) for 103 Early Hints
                    let src = std::fs::read_to_string(input.root.join(view.as_str())).unwrap_or_default();
                    if let Some(l) = folder_chain(view, "_loading.brain").pop() {
                        chain.push_str(&format!(",\"loading\":{}", prerender_json(&url_of[&Node::File(l)])));
                    }
                    if let Some(n) = meta_attr(&src, "cache").and_then(|v| v.trim().parse::<u32>().ok()) {
                        chain.push_str(&format!(",\"cache\":{n}"));
                    }
                    // a load() that can answer 404 (`notFound()`, F11) is not streamed: the status must not be out yet
                    let not_found = server_src.get(view.as_str()).is_some_and(|(_, sjs, _)| sjs.contains("notFound: true"));
                    if needs_server.contains(*view) && static_seo(&src) && !not_found {
                        chain.push_str(",\"stream\":true");
                    }
                    if has_sitemap.contains(*view) && routes::is_dynamic(&p.route) {
                        chain.push_str(",\"sitemap\":true");
                    }
                    chain.push_str(&format!(",\"links\":\"{LINKS}{}\"", p.file));
                    if let Some(loc) = locale_of.get(&p.route) {
                        chain.push_str(&format!(",\"locale\":{},\"alternates\":{}", prerender_json(loc), alternates_of(&p.route)));
                        // the page's dictionary: server.mjs imports it before render() and seo() (QA #153)
                        if let Some(d) = dicts.get(&p.route) {
                            chain.push_str(&format!(",\"i18n\":{}", prerender_json(d)));
                        }
                    }
                    routes_json.push(format!(
                        "{{\"path\":{},\"rx\":{},\"view\":{},\"layouts\":[{}],\"server\":{},\"page\":{}{}{chain}}}",
                        prerender_json(&p.route),
                        prerender_json(&routes::regex_source(&p.route)),
                        prerender_json(&url_of[&Node::File((*view).clone())]),
                        layouts.join(","),
                        srv.map_or("null".to_string(), |s| prerender_json(&s)),
                        prerender_json(&p.file),
                        // forms only (no load, no guard): GET is the prerendered static page, only POST goes to the server
                        if forms_only { ",\"forms\":true" } else { "" }
                    ));
                }
                let mut rpc_json = Vec::new();
                for rel in &rpc_files {
                    let (js, _, fns) = &server_src[*rel];
                    let Some(hash) = fns.iter().find_map(|f| rpc_hash(js, f)) else {
                        warnings.push(format!("{rel}: server functions without a client stub; they are not callable"));
                        continue;
                    };
                    let m = server::emit(&input.root, rel, input.compile, &runtime_url, &mut written, &mut files, 0)?;
                    rpc_json.push(format!("{}:{}", prerender_json(&hash), prerender_json(&m)));
                }
                let nf = format!("404.{}", ext.html());
                let not_found = if files.contains_key(&nf) { prerender_json(&nf) } else { "null".into() };
                files.insert(
                    "_server/manifest.json".into(),
                    format!(
                        "{{\"v\":1,{}\"core\":{},\"routes\":[{}],\"rpc\":{{{}}},\"notFound\":{not_found}{}}}\n",
                        if adopt { "\"adopt\":true," } else { "" },
                        prerender_json(&core),
                        routes_json.join(","),
                        rpc_json.join(","),
                        template_of("!forbidden").map_or(String::new(), |t| format!(",\"forbidden\":{t}"))
                    ),
                );
                files.insert("server.mjs".into(), server_mjs.clone());
                if let Some(shim) = &input.shim {
                    files.insert("_server/dom.js".into(), shim.clone());
                }
                if adopt {
                    if let Some((_, src)) = input.runtime_siblings.iter().find(|(n, _)| n == "brain-hydrate.js") {
                        files.insert("_server/brain-hydrate.js".into(), src.clone());
                    }
                }
                // server.mjs validates form posts with `_server/brain-form.js` (RUNTIME contract): copied unminified
                // with the runtime modules it imports (brain.js, brain-schema.js…), which keep their export names.
                // `brain.js` is the raw core when the caller lists it among the siblings (the bundle uses runtime_core).
                let mut todo = vec!["brain-form.js".to_string()];
                while let Some(name) = todo.pop() {
                    let key = format!("_server/{name}");
                    if files.contains_key(&key) {
                        continue;
                    }
                    match input.runtime_siblings.iter().find(|(n, _)| *n == name) {
                        Some((_, src)) => {
                            for (_, _, spec) in specifiers(src)? {
                                if let Some(dep) = spec.strip_prefix("./").filter(|d| !d.contains('/')) {
                                    todo.push(dep.to_string());
                                }
                            }
                            files.insert(key, src.clone());
                        }
                        None => warnings.push(format!("runtime {name} is not embedded: form posts without JavaScript fail on the server")),
                    }
                }
            }
        }
    }
    // 4a'. SEO (post step): head rewrite, robots/sitemap/llms/manifest/feeds, social images
    let mut post_bin: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut post_csp: Vec<(String, String)> = Vec::new();
    if let Some(post) = input.post {
        report("SEO", 0, pages.len());
        let assets: Vec<(String, u64)> = files.iter().filter(|(p, _)| !pages.iter().any(|x| &x.file == *p)).map(|(p, c)| (p.clone(), c.len() as u64)).collect();
        let out = post(&mut pages, &assets)?;
        for p in &pages {
            files.insert(p.file.clone(), p.html.clone());
        }
        files.extend(out.files);
        post_bin.extend(out.binary);
        warnings.extend(out.warnings);
        post_csp = out.csp;
    }
    // #148: root-relative links of every page (the author's and SEO's) go under the base, after SEO checked them
    if !pre.is_empty() {
        for p in pages.iter_mut() {
            p.html = prefix_links(&p.html, &pre);
            files.insert(p.file.clone(), p.html.clone());
        }
    }
    // an empty chunk nobody references (its only importers lost the import to tree-shaking) is not written (QA #292)
    let empty: Vec<String> = files.iter().filter(|(k, v)| k.starts_with("_b/") && k.ends_with(&format!(".{}", ext.js())) && v.trim().is_empty()).map(|(k, _)| k.clone()).collect();
    for k in empty {
        let name = k.trim_start_matches("_b/");
        if !files.iter().any(|(o, v)| *o != k && v.contains(name)) {
            files.remove(&k);
        }
    }
    report("cache/SW", 0, 1);
    // 4b. Caching (§35): public/ hashing, headers, service worker and its registration
    let refs: Vec<cache::PageRef> = pages.iter().map(|p| cache::PageRef { route: &p.route, file: &p.file }).collect();
    let mut public_bin: BTreeMap<String, Vec<u8>> = post_bin;
    // measuring skips it: the signature changes on every edit and would show as noise in every page's bytes
    let applied = if measure { cache::Applied { warnings: vec![], build_hash: String::new() } } else { cache::apply(&input.root, &cache::CacheCfg { base: base.clone(), ext: (if ext == Ext::Brain { "brain" } else { "web" }).into(), ..input.cache.clone() }, &refs, &post_csp, &mut files, &mut public_bin) };
    warnings.extend(applied.warnings);
    let signature = applied.build_hash.clone();
    // server.mjs answers /_health and X-Brain-Build with it (written after the signature: see cache::apply)
    if let Some(mut m) = files.remove("_server/manifest.json") {
        // `csp`: the site policy without inline hashes; server.mjs adds its per-response nonce to script-src (RUNTIME)
        let policy = prerender_json(&cache::csp(&[], &post_csp));
        // `base` (#148): server.mjs strips it before routing; routes stay as the app writes them
        m = m.replacen("{\"v\":1,", &format!("{{\"v\":1,\"build\":\"{signature}\",\"base\":{},\"csp\":{policy},", prerender_json(&base)), 1);
        // redirects.json for server.mjs (#219): the table brain-seo builds (`redirects::server_json`), in the manifest
        if let Some(r) = files.remove("_server/redirects.json") {
            m = m.replacen("{\"v\":1,", &format!("{{\"v\":1,\"redirects\":{},", r.trim()), 1);
        }
        // each route's preloads, from its final template (with the integrity the SRI pass added)
        while let Some(i) = m.find(&format!("\"{LINKS}")) {
            let end = m[i + 1..].find('"').map_or(m.len(), |e| i + 1 + e + 1);
            let page = m[i + 1 + LINKS.len()..end - 1].to_string();
            let list = files.get(&page).map(|h| cache::early_link_list(h, &pre)).unwrap_or_default();
            let json = format!("[{}]", list.iter().map(|l| prerender_json(l)).collect::<Vec<_>>().join(","));
            m.replace_range(i..end, &json);
        }
        files.insert("_server/manifest.json".into(), m);
    }
    // a static build has no server to read it
    files.remove("_server/redirects.json");
    for p in pages.iter_mut() {
        if let Some(h) = files.get(&p.file) {
            p.html = h.clone();
        }
    }
    // 5. Report and precompression (.gz next to every text file; brotli is left to the deploy step, §26).
    // gzip with optimal parsing is the slowest pass: cached by content and spread over threads
    // host config files are read by the host, never downloaded: no .gz for them
    // text files, plus the binary copies whose format compresses (SVG, JSON, XML, icons…: QA #21 F7)
    let compressible = |p: &str| p.rsplit_once('.').is_some_and(|(_, e)| matches!(e, "svg" | "json" | "txt" | "xml" | "webmanifest" | "ico" | "map" | "wasm" | "csv" | "md"));
    let entries: Vec<(&String, &[u8])> = files
        .iter()
        .filter(|(p, _)| *p != "_headers" && *p != "server.mjs" && !p.starts_with("_brain/") && !p.starts_with("_server/"))
        .map(|(p, c)| (p, c.as_bytes()))
        .chain(public_bin.iter().filter(|(p, b)| compressible(p) && b.len() > 256 && !p.ends_with(".gz")).map(|(p, b)| (p, b.as_slice())))
        .collect();
    let results: std::sync::Mutex<BTreeMap<String, Vec<u8>>> = Default::default();
    let next = std::sync::atomic::AtomicUsize::new(0);
    report("compress", 0, entries.len());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(entries.len().max(1));
    let root = input.root.clone(); // `input` holds the (non-Sync) compile closure: share only the root
    std::thread::scope(|sc| {
        for _ in 0..workers {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((p, c)) = entries.get(i) else { break };
                let z = cached_bytes(&root, "gz", c, || Ok(gzip::gzip(c, 15))).unwrap_or_else(|_| gzip::gzip(c, 15));
                let mut r = results.lock().unwrap();
                // a .gz that is not smaller is never served (`gzip_static` would pick it anyway)
                if z.len() < c.len() {
                    r.insert(format!("{p}.gz"), z);
                }
                if let Some(f) = progress {
                    f("compress", r.len(), entries.len());
                }
            });
        }
    });
    let mut binary = results.into_inner().unwrap();
    binary.extend(public_bin);
    let raw_of: HashMap<String, usize> = url_of.iter().map(|(n, u)| (key_of(u), modules[n].js.len())).collect();
    let mut report = Vec::new();
    for p in pages.iter().filter(|p| !p.route.starts_with('!')) {
        let (mut js_raw, mut js_min, mut js_gz, mut css, mut css_gz) = (0, 0, 0, 0, 0);
        let mut seen = HashSet::new();
        // Conservative route graph: count each emitted JS/CSS asset once, including lazy chunks.
        // Images/fonts are separate payloads, not view code. Incompressible files transfer raw bytes.
        let mut queue: Vec<String> = p.html.match_indices("/_b/").map(|(i, _)| p.html[i + 1..].chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')).collect()).collect();
        while let Some(f) = queue.pop() {
            if !seen.insert(f.clone()) { continue; }
            let Some(c) = files.get(&f) else { continue };
            let z = transfer_size(c.as_bytes(), binary.get(&format!("{f}.gz")).map(Vec::as_slice));
            if f.ends_with(".css") || f.ends_with(".cssbrain") {
                css += c.len();
                css_gz += z;
            } else if f.ends_with(".js") || f.ends_with(".jsbrain") {
                js_min += c.len();
                js_gz += z;
                js_raw += raw_of.get(&f).copied().unwrap_or(c.len());
                for (_, _, spec) in specifiers(c)? {
                    if spec.starts_with(&format!("{pre}/_b/")) { queue.push(key_of(&spec)); }
                }
            }
        }
        report.push(ReportRow {
            route: p.route.clone(),
            js_raw,
            js_min,
            js_gz,
            css,
            css_gz,
            html: p.html.len(),
            html_gz: transfer_size(p.html.as_bytes(), binary.get(&format!("{}.gz", p.file)).map(Vec::as_slice)),
        });
    }
    Ok(Output { files, binary, pages, warnings, report, signature })
}

/// Match production serving: precompressed bytes exist only when smaller than raw output.
fn transfer_size(raw: &[u8], gzip: Option<&[u8]>) -> usize {
    gzip.map_or(raw.len(), |compressed| compressed.len().min(raw.len()))
}

#[test]
fn transfer_size_counts_uncompressed_payload_instead_of_zero() {
    assert_eq!(transfer_size(b"abcd", None), 4);
    assert_eq!(transfer_size(b"abcd", Some(b"ab")), 2);
    assert_eq!(transfer_size(b"abcd", Some(b"larger header")), 4);
}

/// `Comp.h = "abcd"` of a compiled (possibly minified) view chunk.
fn view_hash(js: &str) -> Option<String> {
    let i = js.rfind(".h=\"").or_else(|| js.rfind(".h = \""))?;
    let rest = &js[i..];
    let a = rest.find('"')? + 1;
    let b = rest[a..].find('"')? + a;
    Some(rest[a..b].to_string())
}

/// Inserts `<script type="b/state">{"c":url,"p":{}}</script>` right after the element with `data-b="hash"`.
fn island(html: &str, hash: &str, url: &str) -> Option<String> {
    let attr = format!("data-b=\"{hash}\"");
    let at = html.find(&attr)?;
    let open = html[..at].rfind('<')?;
    let end = element_end(html, open)?;
    Some(format!("{}<script type=\"b/state\">{{\"c\":\"{url}\",\"p\":{{}}}}</script>{}", &html[..end], &html[end..]))
}

/// Byte index just past the element that opens at `open` (well-formed HTML from the prerender serializer).
fn element_end(html: &str, open: usize) -> Option<usize> {
    const VOID: &[&str] = &["area", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];
    let b = html.as_bytes();
    let mut i = open;
    let mut depth = 0i32;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if html[i..].starts_with("<!--") {
            i = html[i..].find("-->").map(|e| i + e + 3)?;
            continue;
        }
        let close = html[i..].find('>').map(|e| i + e)?;
        let tag = &html[i + 1..close];
        if let Some(name) = tag.strip_prefix('/') {
            let _ = name;
            depth -= 1;
            if depth == 0 {
                return Some(close + 1);
            }
        } else {
            let name: String = tag.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>().to_ascii_lowercase();
            if !VOID.contains(&name.as_str()) && !tag.ends_with('/') {
                depth += 1;
            } else if depth == 0 {
                return Some(close + 1);
            }
        }
        i = close + 1;
    }
    None
}

/// Removes the first `<tag>…</tag>` block that belongs to BrainScript and returns (rest, body). A `<script src=…>`
/// (third-party script with SRI in app.brain's head, B0123) is markup, not the app's script: it is skipped and kept.
/// Does `src` load a module matching `is_it` with a dynamic `import("…")`?
fn dynamic_import_of(src: &str, is_it: &dyn Fn(&str) -> bool) -> bool {
    let Ok(t) = brain_min::lex::lex(src) else { return true };
    t.windows(3).any(|w| w[0].ident("import") && w[1].punct("(") && w[2].kind == brain_min::lex::Kind::Str && is_it(&w[2].text[1..w[2].text.len() - 1]))
}

/// The `<hash>` of the client stub `"<hash>/<fn>"` for server function `f`.
fn rpc_hash(js: &str, f: &str) -> Option<String> {
    let pat = format!("/{f}\"");
    js.match_indices(&pat).find_map(|(i, _)| {
        let start = js[..i].rfind('"')? + 1;
        let h = &js[start..i];
        (!h.is_empty() && h.chars().all(|c| c.is_ascii_alphanumeric())).then(|| h.to_string())
    })
}

/// app.brain's `<meta seo …>` holds site defaults (read through `seo_of`); it is not markup of the page.
pub fn strip_meta_seo(src: &str) -> String {
    let mut out = src.to_string();
    // `<meta http …>` is the app's HTTP policy (compiled into app.brain's module), never markup either
    while let Some(i) = out.find("<meta seo").or_else(|| out.find("<meta http ").or_else(|| out.find("<meta http>"))) {
        // only removes the tag from the shell markup: its values come from brain-check's view_info (`seo_of`)
        let Some(end) = tag_end(&out, i) else { break };
        let end = if out[end..].starts_with('\n') { end + 1 } else { end };
        out.replace_range(i..end, "");
    }
    out
}

/// Placeholder for a route's preload list in the server manifest until the SRI pass has run.
const LINKS: &str = "\u{1}links:";

/// The server code imports only `brain:content` and its `load` reads nothing of the request (#273).
fn content_only_load(server_js: &str) -> bool {
    if !(server_js.contains("function load(") || server_js.contains("const load =")) {
        return false;
    }
    let Ok(specs) = specifiers(server_js) else { return false };
    if specs.is_empty() || specs.iter().any(|(_, _, s)| s != "brain:content") {
        return false;
    }
    !["ctx.request", "ctx.req", "ctx.cookies", "ctx.session", "ctx.user", "ctx.query", "ctx.url", "ctx.headers", "ctx.ip", "ctx.env", "ctx.params"]
        .iter()
        .any(|k| server_js.contains(k))
}

/// `<meta NAME="value">` of a view source (`<meta cache="60">`, #235).
fn meta_attr(src: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = src[from..].find("<meta ").map(|i| from + i) {
        let end = tag_end(src, i).unwrap_or(src.len());
        if let Some(v) = cache::attr(&src[i..end.saturating_sub(1)], name) {
            return Some(v);
        }
        from = end;
    }
    None
}

/// The view's `<meta seo …>` has no expressions, so the head can go out before `load()` finishes (#232).
fn static_seo(src: &str) -> bool {
    match src.find("<meta seo") {
        Some(i) => !src[i..tag_end(src, i).unwrap_or(src.len())].contains('{'),
        None => true,
    }
}

/// Placeholder for the mount props in page scripts, filled once the prerender returns them.
const PROPS: &str = "__BRAIN_PROPS__";

/// `/blog/:slug` + {slug: "a b"} → `/blog/a%20b`; None when a param is missing.
fn concrete_route(route: &str, params: &[(String, String)]) -> Option<String> {
    let enc = |v: &str| {
        v.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    let segs: Option<Vec<String>> = route
        .split('/')
        .map(|s| match (s.strip_prefix(':'), s.strip_prefix('*').filter(|k| !k.is_empty())) {
            (Some(k), _) => params.iter().find(|(a, _)| a == k).map(|(_, v)| enc(v)),
            // a catch-all value keeps its `/` separators (each part encoded on its own)
            (_, Some(k)) => params.iter().find(|(a, _)| a == k).filter(|(_, v)| !v.is_empty()).map(|(_, v)| v.split('/').map(enc).collect::<Vec<_>>().join("/")),
            _ => Some(s.to_string()),
        })
        .collect();
    segs.map(|s| s.join("/"))
}

fn params_json(params: &[(String, String)]) -> String {
    let esc = |s: &str| prerender_json(s);
    format!("{{{}}}", params.iter().map(|(k, v)| format!("{}:{}", esc(k), esc(v))).collect::<Vec<_>>().join(","))
}

fn prerender_json(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Cache format for a rendered page: fields separated by NUL (none of them can contain one).
fn encode_rendered(r: &prerender::Rendered) -> Vec<u8> {
    let seo: Vec<String> = r.seo.iter().map(|(k, v)| format!("{k}\u{1f}{v}")).collect();
    [r.html.as_str(), &seo.join("\u{1e}"), &r.head, &r.props, if r.fetched { "1" } else { "" }, &r.failed.join("\u{1e}"), &r.probe.join("\u{1e}")].join("\0").into_bytes()
}

fn decode_rendered(b: &[u8]) -> Option<prerender::Rendered> {
    let s = String::from_utf8_lossy(b);
    let f: Vec<&str> = s.split('\0').collect();
    if f.len() != 7 {
        return None;
    }
    let seo = f[1].split('\u{1e}').filter(|x| !x.is_empty()).filter_map(|kv| kv.split_once('\u{1f}').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
    Some(prerender::Rendered { html: f[0].into(), seo, head: f[2].into(), props: f[3].into(), fetched: f[4] == "1", failed: f[5].split('\u{1e}').filter(|x| !x.is_empty()).map(String::from).collect(), probe: f[6].split('\u{1e}').filter(|x| !x.is_empty()).map(String::from).collect() })
}

/// Drops every `@scope (…) { … }` block identical to one seen earlier in the same stylesheet (at any depth, e.g.
/// inside `@layer view{…}`): the same component's styles reached twice. Strings and comments are skipped.
pub fn dedupe_scopes(css: &str) -> String {
    let b = css.as_bytes();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = String::with_capacity(css.len());
    let (mut i, mut last) = (0usize, 0usize);
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            }
            b'@' if b[i..].starts_with(b"@scope") => {
                // the block: up to the `}` that closes its `{`
                let Some(open) = css[i..].find('{').map(|o| i + o) else { break };
                let (mut depth, mut j) = (0usize, open);
                while j < b.len() {
                    match b[j] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        q @ (b'"' | b'\'') => {
                            j += 1;
                            while j < b.len() && b[j] != q {
                                j += if b[j] == b'\\' { 2 } else { 1 };
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let end = (j + 1).min(b.len());
                if !seen.insert(&css[i..end]) {
                    out.push_str(&css[last..i]);
                    last = end;
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    out.push_str(&css[last..]);
    out
}

/// Index just after the `>` that closes the start tag at `start`, skipping `>` inside quoted values and inside
/// `{…}` expressions (with their own strings and template literals): `title={a > b}`, `jsonld={{ "k": ">" }}`.
pub fn tag_end(src: &str, start: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut i = start + 1;
    let mut depth = 0usize; // `{` nesting
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'' | b'`') if depth > 0 || q != b'`' => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' && depth > 0 {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'>' if depth == 0 => return Some(i + 1),
            _ => {}
        }
        i += 1;
    }
    None
}

fn take_block(src: &str, tag: &str) -> (String, String) {
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut from = 0;
    while let Some(a) = src[from..].find(&open).map(|a| from + a) {
        let after = src[a + open.len()..].chars().next();
        let Some(gt) = tag_end(src, a).map(|i| i - 1) else { break };
        let Some(b) = src[gt..].find(&close).map(|b| gt + b) else { break };
        let is_tag = matches!(after, Some(c) if c == '>' || c == '/' || c.is_whitespace());
        let external = src[a..gt].contains(" src=");
        if is_tag && !external {
            return (format!("{}{}", &src[..a], &src[b + close.len()..]), src[gt + 1..b].to_string());
        }
        from = b + close.len();
    }
    (src.to_string(), String::new())
}

/// Conservative CSS minifier: drops comments and collapses whitespace outside strings; keeps one space where a
/// token boundary needs it (`a b`, `1px solid`, `and (`), removes it around `{}:;,>` and before `!important`.
pub fn minify_css(css: &str) -> String {
    shorten_css(&minify_css_ws(css))
}

/// Shortest forms after whitespace minification: `#aabbcc` → `#abc`, `0.5` → `.5`, and `0px`/`0em`… → `0` outside
/// parentheses (inside `calc()` a unitless 0 is invalid). Strings are left untouched.
fn shorten_css(css: &str) -> String {
    let b: Vec<char> = css.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    let mut depth = 0u32;
    let is_num_start = |prev: Option<char>| prev.is_none_or(|p| matches!(p, ':' | ' ' | ',' | '(' | '-' | '+' | '/' | '*'));
    while i < b.len() {
        let c = b[i];
        if c == '"' || c == '\'' {
            let start = i;
            i += 1;
            while i < b.len() && b[i] != c {
                if b[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            out.extend(&b[start..i]);
            continue;
        }
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth = depth.saturating_sub(1);
        }
        // #aabbcc → #abc (only in values: the previous char is `:`, space, `,` or `(`)
        if c == '#' && i + 7 <= b.len() && matches!(out.chars().last(), Some(':' | ' ' | ',' | '(')) {
            let h: Vec<char> = b[i + 1..i + 7].to_vec();
            let ends = b.get(i + 7).is_none_or(|n| !n.is_ascii_hexdigit());
            if ends && h.iter().all(|x| x.is_ascii_hexdigit()) && h[0] == h[1] && h[2] == h[3] && h[4] == h[5] {
                out.push('#');
                out.push(h[0].to_ascii_lowercase());
                out.push(h[2].to_ascii_lowercase());
                out.push(h[4].to_ascii_lowercase());
                i += 7;
                continue;
            }
        }
        if c == '0' && is_num_start(out.chars().last()) {
            // 0.5 → .5
            if b.get(i + 1) == Some(&'.') && b.get(i + 2).is_some_and(|d| d.is_ascii_digit()) {
                i += 1;
                continue;
            }
            // 0px → 0 (not inside parentheses, not 0.x, not part of a longer number)
            if depth == 0 {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let unit: String = b[i + 1..j].iter().collect();
                let ends = b.get(j).is_none_or(|n| matches!(n, ';' | '}' | ' ' | ',' | '!' | ')'));
                if ends && matches!(unit.as_str(), "px" | "em" | "rem" | "vh" | "vw" | "pt" | "cm" | "mm" | "in" | "ex" | "ch") {
                    out.push('0');
                    i = j;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn minify_css_ws(css: &str) -> String {
    let b: Vec<char> = css.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    let mut pending_space = false;
    // inside (...) — calc(), min(), max(), clamp(), media features — `+`, `-`, `>` and `~` are operators that
    // need their surrounding spaces (`calc(a+b)` is invalid CSS), so only `,` `:` `(` `)` drop them there
    let mut depth = 0u32;
    while i < b.len() {
        let c = b[i];
        if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            pending_space = true;
            continue;
        }
        if c == '"' || c == '\'' {
            if pending_space && needs_space(out.chars().last(), c) {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
            i += 1;
            while i < b.len() && b[i] != c {
                if b[i] == '\\' && i + 1 < b.len() {
                    out.push(b[i]);
                    i += 1;
                }
                out.push(b[i]);
                i += 1;
            }
            if i < b.len() {
                out.push(c);
            }
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            pending_space = true;
            i += 1;
            continue;
        }
        if pending_space && (if depth > 0 { needs_space_in_parens(out.chars().last(), c) } else { needs_space(out.chars().last(), c) }) {
            out.push(' ');
        }
        pending_space = false;
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth = depth.saturating_sub(1);
        }
        if c == ';' && b[i + 1..].iter().find(|x| !x.is_whitespace()) == Some(&'}') {
            i += 1; // last `;` before `}`
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn needs_space_in_parens(prev: Option<char>, next: char) -> bool {
    match prev {
        None => false,
        Some(p) => !matches!(p, '(' | ',' | ':') && !matches!(next, ')' | ',' | ':'),
    }
}

fn needs_space(prev: Option<char>, next: char) -> bool {
    match prev {
        None => false,
        Some(p) => !matches!(p, '{' | '}' | ':' | ';' | ',' | '>' | '(' | '~' | '+') && !matches!(next, '{' | '}' | ':' | ';' | ',' | '>' | ')' | '!' | '~' | '+'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_minifier_keeps_meaning() {
        assert_eq!(minify_css("a  b ,c > d { color : red ; margin: 0 auto !important; }\n/* x */ .e{content:\"a  b\"}"), "a b,c>d{color:red;margin:0 auto!important}.e{content:\"a  b\"}");
        assert_eq!(minify_css("@media (min-width: 600px) and (hover: hover) { a { b: c } }"), "@media (min-width:600px) and (hover:hover){a{b:c}}");
        assert_eq!(minify_css(":where(:scope):is(button) { x: 1 }"), ":where(:scope):is(button){x:1}");
    }

    #[test]
    fn css_shortest_forms() {
        assert_eq!(minify_css("a { color: #FFFFFF; b: #aabbcd; c: 0.5s; d: 0px 10px; e: calc(0px + 1em); f: 10.5px; g: url(\"#ffffff\") }"), "a{color:#fff;b:#aabbcd;c:.5s;d:0 10px;e:calc(0px + 1em);f:10.5px;g:url(\"#ffffff\")}");
        assert_eq!(minify_css("#aabbcc { x: 1 }"), "#aabbcc{x:1}", "an id selector is not a color");
    }

    #[test]
    fn css_math_keeps_operator_spaces() {
        assert_eq!(minify_css("a { r: calc(var(--radius) + 6px); }"), "a{r:calc(var(--radius) + 6px)}");
        assert_eq!(minify_css("a { r: calc( var(--a) - 4px ) }"), "a{r:calc(var(--a) - 4px)}");
        assert_eq!(minify_css("a { m: calc(-1 * var(--space-2)) }"), "a{m:calc(-1 * var(--space-2))}");
        assert_eq!(
            minify_css("a { w: clamp(10px , min(50% , calc(100vw - 2 * var(--s))) , max(20px, 1rem + 2vw)) }"),
            "a{w:clamp(10px,min(50%,calc(100vw - 2 * var(--s))),max(20px,1rem + 2vw))}"
        );
        assert_eq!(minify_css("@media (width > 600px) { a { b: c } }"), "@media (width > 600px){a{b:c}}");
    }

    #[test]
    fn islands_follow_the_view_root() {
        let html = "<section data-b=\"lay1\"><nav>x</nav><main data-b=\"v1\"><input><p>a<!----></p></main></section>";
        let out = island(html, "v1", "/_b/v.js").unwrap();
        assert!(out.contains("</main><script type=\"b/state\">{\"c\":\"/_b/v.js\",\"p\":{}}</script></section>"), "{out}");
        assert_eq!(view_hash("function a(){}a.h=\"bhng\";export default a;").as_deref(), Some("bhng"));
    }

    #[test]
    fn meta_seo_with_expressions_is_removed_whole() {
        let shell = "<html><head><meta seo title={a > b ? 'x>' : `y>${c}`} jsonld={{ \"@type\": \"WebSite\", \"k\": \"a>b\" }}>\n<meta charset=\"utf-8\"></head><body><slot /></body></html>";
        assert_eq!(strip_meta_seo(shell), "<html><head><meta charset=\"utf-8\"></head><body><slot /></body></html>");
        assert_eq!(tag_end("<a title=\"x>y\">", 0), Some(15));
        let (rest, css) = take_block("<style data-x={1 > 0}>b{}</style><p>", "style");
        assert_eq!((rest.as_str(), css.as_str()), ("<p>", "b{}"));
    }

    #[test]
    fn base_from_brain_json_and_link_prefixing() {
        let d = std::env::temp_dir().join(format!("brain-base-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for (json, want) in [
            ("{\"name\":\"x\",\"base\":\"/app\"}", "/app/"),
            ("{\"base\": \"docs/v2/\"}", "/docs/v2/"),
            ("{\"http\":{\"base\":\"/api\"},\"name\":\"x\"}", "/"),
            ("{\"base\":\"/\"}", "/"),
        ] {
            std::fs::write(d.join("brain.json"), json).unwrap();
            assert_eq!(read_base(&d), want, "{json}");
        }
        let _ = std::fs::remove_dir_all(&d);
        let html = "<a href=\"/about\">a</a><a href=\"//cdn.x/y\">b</a><a href=\"https://x.y/\">c</a><form action=\"/save\"></form><img src=\"/app/_b/x.png\"><a href=\"/app\">d</a><a data-href=\"/no\">e</a>";
        assert_eq!(
            prefix_links(html, "/app"),
            "<a href=\"/app/about\">a</a><a href=\"//cdn.x/y\">b</a><a href=\"https://x.y/\">c</a><form action=\"/app/save\"></form><img src=\"/app/_b/x.png\"><a href=\"/app\">d</a><a data-href=\"/no\">e</a>"
        );
        // compiled templates: unquoted and escaped quotes inside a JS string
        assert_eq!(prefix_links("tpl(\"<a href=/about>x</a><a href=\\\"/b\\\">\")", "/app"), "tpl(\"<a href=/app/about>x</a><a href=\\\"/app/b\\\">\")");
        assert_eq!(prefix_links(html, ""), html);
        assert_eq!(prefix_links("<p>a · b</p><a href=\"/x\">", "/app"), "<p>a · b</p><a href=\"/app/x\">", "multi-byte text before a link");
    }

    #[test]
    fn identical_scoped_blocks_ship_once() {
        let child = "@scope ([data-b=\"k1\"]) to ([data-b]){:where(:scope){color:red}.a{content:\"}\"}}";
        let css = format!("@layer view{{{child}.p1{{x:1}}}}\n@layer view{{.p2{{x:2}}{child}}}\n@layer view{{@scope ([data-b=\"k2\"]) to ([data-b]){{b{{y:1}}}}}}");
        let out = dedupe_scopes(&css);
        assert_eq!(out.matches("data-b=\"k1\"").count(), 1, "{out}");
        assert!(out.contains(".p1{x:1}") && out.contains(".p2{x:2}") && out.contains("data-b=\"k2\""), "{out}");
        assert_eq!(dedupe_scopes("a{b:c}"), "a{b:c}");
    }

    #[test]
    fn content_only_loads() {
        assert!(content_only_load("import { page } from \"brain:content\";\nexport async function load() { return { posts: page(1) } }"));
        assert!(!content_only_load("import { page } from \"brain:content\";\nexport async function load(ctx) { return { posts: page(ctx.query.p) } }"), "reads the request");
        assert!(!content_only_load("import db from \"../lib/db.js\";\nexport async function load() { return db.all() }"), "another import");
        assert!(!content_only_load("export async function load() { return { n: Date.now() } }"), "no collection: stays on the server");
    }

    #[test]
    fn catch_all_pages_from_paths() {
        let p = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        assert_eq!(concrete_route("/docs/*rest", &p("rest", "guide/a b")).as_deref(), Some("/docs/guide/a%20b"));
        assert_eq!(concrete_route("/docs/*rest", &p("rest", "")), None);
        assert_eq!(concrete_route("/u/:id", &p("id", "a/b")).as_deref(), Some("/u/a%2Fb"));
    }

    #[test]
    fn hashes_are_stable_base36() {
        let h = content_hash("abc");
        assert_eq!(h.len(), 8);
        assert_eq!(h, content_hash("abc"));
        assert_ne!(h, content_hash("abd"));
    }

    #[test]
    fn resolves_relative_specifiers() {
        assert_eq!(resolve("views/home.brain", "../comp/a.brain").unwrap(), Node::File("comp/a.brain".into()));
        assert_eq!(resolve("views/home.brain", "/@brain/brain.js").unwrap(), Node::Runtime("brain.js".into()));
        assert!(resolve("views/home.brain", "lodash").unwrap_err().contains("fix"));
        assert!(resolve("a.brain", "../../x.brain").is_err());
    }
}
