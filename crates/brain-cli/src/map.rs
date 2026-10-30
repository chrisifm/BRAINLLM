//! `brain map [--json]`: the project graph (routes, components with their props, modules, tests, tokens) so that
//! an LLM reads it instead of opening files (SPEC «CLI»).

use brain_mcp::Json;
use brain_parse::ast::{FileKind, Level, Stmt};
use std::path::{Path, PathBuf};

const MAX_FILES: usize = 4096;
const MAX_ENTRIES: usize = 8192;
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;
#[derive(Default)]
struct Scan {
    files: Vec<PathBuf>,
    entries: usize,
    bytes: usize,
    skipped: Vec<Json>,
    skipped_count: usize,
    truncated: bool,
    backend_unknown: bool,
    context_unsafe: bool,
}
impl Scan {
    fn skip(&mut self, path: &str, reason: &str, truncated: bool) {
        self.skipped_count += 1;
        self.truncated |= truncated;
        self.backend_unknown |= path.is_empty() || path == "server" || path.starts_with("server/");
        if self.skipped.len() < 128 {
            self.skipped.push(Json::obj([
                ("path", Json::str(path)),
                ("status", Json::str("unknown")),
                ("reason", Json::str(reason)),
            ]));
        }
    }
    fn read(&mut self, root: &Path, path: &str) -> Option<String> {
        let remaining = MAX_TOTAL_BYTES.saturating_sub(self.bytes);
        if remaining == 0 {
            self.skip(path, "total read budget exceeded", true);
            return None;
        }
        match read_source_limit(root, path, remaining.min(MAX_FILE_BYTES)) {
            Ok(src) if self.bytes + src.len() <= MAX_TOTAL_BYTES => {
                self.bytes += src.len();
                Some(src)
            }
            Ok(_) => {
                self.skip(path, "total read budget exceeded", true);
                None
            }
            Err(reason) => {
                let truncated = reason.contains("budget");
                self.skip(path, &reason, truncated);
                None
            }
        }
    }
    fn coverage(&self) -> Json {
        Json::obj([
            ("complete", Json::Bool(!self.truncated && self.skipped_count == 0)),
            (
                "checkerContextSafe",
                Json::Bool(!self.context_unsafe && !self.truncated && self.skipped_count == 0),
            ),
            ("truncated", Json::Bool(self.truncated)),
            ("skippedCount", Json::Num(self.skipped_count as f64)),
            ("skippedMore", Json::Bool(self.skipped_count > self.skipped.len())),
            ("skipped", Json::Arr(self.skipped.clone())),
            ("maxDepth", Json::Num(32.0)),
            ("maxFiles", Json::Num(MAX_FILES as f64)),
            ("maxEntries", Json::Num(MAX_ENTRIES as f64)),
            ("maxFileBytes", Json::Num(MAX_FILE_BYTES as f64)),
            ("maxReadBytes", Json::Num(MAX_TOTAL_BYTES as f64)),
            ("readBytes", Json::Num(self.bytes as f64)),
        ])
    }
}
/// Shared bounded public-source reader for map/card; private paths are refused.
/// The private scan helper alone can read fixed environment metadata for key names.
pub(crate) fn read_source(root: &Path, path: &str) -> Result<String, String> {
    crate::changes::source_path(root, path)?;
    read_source_limit(root, path, MAX_FILE_BYTES)
}
pub(crate) fn read_source_limit(root: &Path, path: &str, limit: usize) -> Result<String, String> {
    use std::io::Read;
    let target = if path == ".brain_env" {
        root.canonicalize().map_err(|_| "app root is unavailable")?.join(path)
    } else {
        crate::changes::source_path(root, path)?
    };
    let meta = std::fs::symlink_metadata(&target).map_err(|_| "file is unreadable or missing")?;
    if meta.file_type().is_symlink() {
        return Err("symlink skipped".into());
    }
    if !meta.is_file() {
        return Err("nonregular file skipped".into());
    }
    if meta.len() > MAX_FILE_BYTES as u64 {
        return Err("file exceeds 2 MiB".into());
    }
    if meta.len() > limit as u64 {
        return Err("total read budget exceeded".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut bytes = Vec::new();
    let file = options.open(target).map_err(|_| "file is unreadable")?;
    if !file.metadata().is_ok_and(|m| m.is_file() && !m.file_type().is_symlink()) {
        return Err("nonregular opened file skipped".into());
    }
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "file is unreadable")?;
    if bytes.len() > limit {
        return Err("read budget exceeded while file changed".into());
    }
    let source = String::from_utf8(bytes).map_err(|_| "source is not UTF-8")?;
    if path != ".brain_env" && !brain_check::scan_secrets(&source).is_empty() {
        return Err("detected secret literal; source metadata is unknown".into());
    }
    Ok(source)
}
fn walk(top: &Path, dir: &Path, depth: usize, scan: &mut Scan) {
    if depth > 32 {
        scan.skip(&rel(top, dir), "directory depth exceeds 32", true);
        return;
    }
    if std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
        scan.skip(&rel(top, dir), "symlink skipped", false);
        return;
    }
    if dir != top && crate::scaffold::is_app_dir(dir) {
        scan.context_unsafe = true;
        let path = rel(top, dir);
        scan.backend_unknown |= path == "server" || path.starts_with("server/");
        return;
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => {
            scan.skip(&rel(top, dir), "directory is unreadable", false);
            return;
        }
    };
    let remaining = MAX_ENTRIES.saturating_sub(scan.entries);
    let mut entries: Vec<_> = rd.take(remaining + 1).collect();
    if entries.len() > remaining {
        scan.skip(&rel(top, dir), "directory entry budget exceeded", true);
        entries.truncate(remaining);
    }
    entries.sort_by_key(|e| e.as_ref().ok().map(|e| e.file_name()));
    for entry in entries {
        if scan.entries >= MAX_ENTRIES {
            scan.skip(&rel(top, dir), "directory entry budget exceeded", true);
            return;
        }
        scan.entries += 1;
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                scan.skip(&rel(top, dir), "directory entry is unreadable", false);
                continue;
            }
        };
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = match std::fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(_) => {
                scan.skip(&rel(top, &p), "metadata is unreadable", false);
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            scan.skip(&rel(top, &p), "symlink skipped", false);
            continue;
        }
        if name == "target" {
            scan.context_unsafe = true;
            continue;
        }
        if name.starts_with('.') || ["dist", "node_modules"].contains(&name.as_str()) {
            continue;
        }
        if meta.is_dir() {
            walk(top, &p, depth + 1, scan);
        } else if meta.is_file() && (name.ends_with(".brain") || name.ends_with(".jsbrain")) {
            if scan.files.len() == MAX_FILES {
                scan.skip(&rel(top, &p), "source file budget exceeded", true);
                return;
            }
            scan.files.push(p);
        } else if !meta.is_file() {
            scan.skip(&rel(top, &p), "nonregular entry skipped", false);
        }
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// The build's single route table decides what a view's URL is (`[...rest]` → `/x/*rest`).
pub use brain_pack::routes::route_of;

/// Exported names of a script (our own lexer; robust even if the file has check errors).
fn exports_of(src: &str) -> Vec<String> {
    let Ok(t) = brain_min::lex::lex(src) else { return vec![] };
    let mut out = Vec::new();
    for i in 0..t.len() {
        if !t[i].ident("export") {
            continue;
        }
        let mut j = i + 1;
        if t.get(j).is_some_and(|x| x.ident("default")) {
            out.push("default".to_string());
            continue;
        }
        if t.get(j).is_some_and(|x| x.ident("async")) {
            j += 1;
        }
        if t.get(j)
            .is_some_and(|x| x.ident("const") || x.ident("let") || x.ident("function") || x.ident("class"))
        {
            if let Some(n) = t.get(j + 1).filter(|x| x.kind == brain_min::lex::Kind::Ident) {
                out.push(n.text.clone());
            }
        } else if t.get(j).is_some_and(|x| x.punct("{")) {
            let mut k = j + 1;
            while k < t.len() && !t[k].punct("}") {
                if t[k].kind == brain_min::lex::Kind::Ident && !t[k].ident("as") && !t.get(k + 1).is_some_and(|x| x.ident("as")) {
                    out.push(t[k].text.clone());
                }
                k += 1;
            }
        }
    }
    out
}

fn tests_of(src: &str) -> Vec<Json> {
    let Ok(t) = brain_min::lex::lex(src) else { return vec![] };
    t.windows(3)
        .filter(|w| w[0].ident("test") && w[1].punct("(") && w[2].kind == brain_min::lex::Kind::Str)
        .map(|w| Json::str(&w[2].text[1..w[2].text.len() - 1]))
        .collect()
}

fn meta_list(c: &brain_parse::ast::Component, name: &str) -> Vec<Json> {
    c.meta
        .iter()
        .filter(|m| m.name == name)
        .filter_map(|m| m.value.clone())
        .flat_map(|v| v.split_whitespace().map(Json::str).collect::<Vec<_>>())
        .collect()
}

pub fn build(root: &Path) -> Json {
    let canonical = root.canonicalize().ok();
    let root = canonical.as_deref().unwrap_or(root);
    let mut scan = Scan::default();
    walk(root, root, 0, &mut scan);
    let files = scan.files.clone();
    let (mut reads, mut writes): (
        std::collections::BTreeMap<String, Vec<Json>>,
        std::collections::BTreeMap<String, Vec<Json>>,
    ) = Default::default();
    let (mut routes, mut comps, mut modules, mut tests) = (vec![], vec![], vec![], vec![]);
    for f in &files {
        let r = rel(root, f);
        let Some(src) = scan.read(root, &r) else { continue };
        if r.ends_with(".test.brain") {
            tests.push(Json::obj([("file", Json::str(&r)), ("tests", Json::Arr(tests_of(&src)))]));
            continue;
        }
        let parsed = brain_parse::parse(&src, Path::new(&r));
        if r.ends_with(".jsbrain") {
            let server = src.trim_start().starts_with("<meta server") || src.trim_start().starts_with("<meta protect");
            modules.push(Json::obj([
                ("file", Json::str(&r)),
                ("exports", Json::Arr(exports_of(&src).into_iter().map(Json::Str).collect())),
                ("server", Json::Bool(server)),
            ]));
            continue;
        }
        let Ok(c) = parsed else {
            comps.push(Json::obj([
                ("file", Json::str(&r)),
                ("error", Json::str("does not parse; run `brain check`")),
            ]));
            continue;
        };
        let deps = brain_check::contract::deps(&c);
        for key in deps.reads.into_iter().filter(|k| !k.starts_with("prop:")) {
            reads.entry(key).or_default().push(Json::str(&r));
        }
        for key in deps.writes {
            writes.entry(key).or_default().push(Json::str(&r));
        }
        if c.kind == FileKind::App {
            continue;
        }
        let props: Vec<Json> = c
            .props
            .iter()
            .map(|p| {
                let mut kv = vec![
                    ("name".to_string(), Json::str(&p.name)),
                    ("type".into(), Json::str(format!("{:?}", p.ty).to_lowercase())),
                    ("optional".into(), Json::Bool(p.optional || p.default.is_some())),
                ];
                if p.default.is_some() {
                    // default text exactly as written (`start:num=0` → "0")
                    let span = &src[p.span.start as usize..(p.span.end as usize).min(src.len())];
                    if let Some((_, d)) = span.split_once('=') {
                        kv.push(("default".into(), Json::str(d.trim())));
                    }
                }
                Json::Obj(kv)
            })
            .collect();
        let imports: Vec<Json> = c
            .script
            .iter()
            .filter_map(|s| {
                if let Stmt::Import(i) = s {
                    Some(Json::str(&i.source))
                } else {
                    None
                }
            })
            .collect();
        let has_load = src.contains("function load");
        let server_fns: Vec<Json> = c
            .script
            .iter()
            .filter_map(|s| {
                if let Stmt::Server { func, .. } = s {
                    func.name.as_ref().map(|n| Json::str(&n.name))
                } else {
                    None
                }
            })
            .collect();
        let level = match c.level {
            Level::Micro => "ui",
            Level::Comp => "comp",
            Level::View => "view",
        };
        let mut kv = vec![
            ("file".to_string(), Json::str(&r)),
            ("name".into(), Json::str(&c.name)),
            ("level".into(), Json::str(level)),
            ("props".into(), Json::Arr(props)),
            ("emits".into(), Json::Arr(meta_list(&c, "emits"))),
            ("expose".into(), Json::Arr(meta_list(&c, "expose"))),
            ("imports".into(), Json::Arr(imports)),
        ];
        if !server_fns.is_empty() {
            kv.push(("server".into(), Json::Arr(server_fns)));
        }
        if c.level == Level::View
            && let Some(path) = route_of(&r)
        {
            let params: Vec<Json> = path.split('/').filter_map(|s| s.strip_prefix(':')).map(Json::str).collect();
            routes.push(Json::obj([
                ("path", Json::str(&path)),
                ("file", Json::str(&r)),
                ("params", Json::Arr(params)),
                ("load", Json::Bool(has_load)),
            ]));
        }
        comps.push(Json::Obj(kv));
    }
    let tokens: Vec<Json> = ["tokens.cssbrain", "tokens.brain.css"]
        .iter()
        .find_map(|n| match std::fs::symlink_metadata(root.join(n)) {
            Ok(_) => scan.read(root, n).map(|src| (*n, src)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                scan.skip(n, "tokens metadata is unreadable", false);
                None
            }
        })
        .and_then(|(n, s)| brain_css::parse_str(&s, n).ok())
        .map(|sheet| brain_css::custom_props(&sheet).into_iter().map(|(k, _)| Json::Str(k)).collect())
        .unwrap_or_default();
    let mut compatibility = crate::compatibility(None);
    if std::fs::symlink_metadata(root.join("brain.json")).is_ok() {
        let parsed = scan.read(root, "brain.json").and_then(|s| Json::parse(&s).ok());
        if parsed.is_none() {
            if let Json::Obj(kv) = &mut compatibility {
                kv.push(("status".into(), Json::str("unknown")));
                if let Some((_, target)) = kv.iter_mut().find(|(k, _)| k == "target") {
                    *target = Json::str("unknown");
                }
            }
        }
        let target = parsed.and_then(|j| j.get("targets").and_then(Json::as_str).map(str::to_owned));
        if let Some(target) = target {
            let valid = target.len() <= 32
                && (matches!(target.as_str(), "baseline-widely" | "baseline-newly")
                    || target
                        .strip_prefix("baseline-")
                        .is_some_and(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit())));
            if let Json::Obj(kv) = &mut compatibility {
                if let Some((_, v)) = kv.iter_mut().find(|(k, _)| k == "target") {
                    *v = Json::str(if valid { &target } else { "invalid" });
                }
            }
        }
    }
    let env = if std::fs::symlink_metadata(root.join(".brain_env")).is_ok() {
        scan.read(root, ".brain_env")
            .map(|src| {
                let file = brain_check::env::parse(&src);
                let none = |_: &str| None;
                Json::Obj(
                    std::iter::once("shared".to_string()).chain(file.environments())
                        .collect::<std::collections::BTreeSet<_>>().into_iter()
                        .map(|e| {
                            let mut keys: Vec<_> = file.resolve(&e, &none).into_keys().collect();
                            keys.sort();
                            (e, Json::Arr(keys.into_iter().map(Json::Str).collect()))
                        })
                        .collect(),
                )
            })
            .unwrap_or_else(|| Json::obj([("status", Json::str("unknown"))]))
    } else {
        Json::Obj(vec![])
    };
    let backend = if scan.truncated || scan.backend_unknown {
        Json::obj([
            ("status", Json::str("unknown")),
            (
                "error",
                Json::str("Backend discovery skipped because the source scan is incomplete"),
            ),
        ])
    } else {
        crate::backend::card(root, None)
            .and_then(|j| Json::parse(&j.to_string()))
            .unwrap_or_else(|_| Json::obj([("error", Json::str("Invalid backend contracts; run brain backend schema --json"))]))
    };
    Json::obj([
        ("routes", Json::Arr(routes)),
        ("components", Json::Arr(comps)),
        ("modules", Json::Arr(modules)),
        ("tests", Json::Arr(tests)),
        ("tokens", Json::Arr(tokens)),
        ("compatibility", compatibility),
        ("backend", backend),
        ("coverage", scan.coverage()),
        (
            "reverse",
            Json::obj([
                ("reads", Json::Obj(reads.into_iter().map(|(k, v)| (k, Json::Arr(v))).collect())),
                ("writes", Json::Obj(writes.into_iter().map(|(k, v)| (k, Json::Arr(v))).collect())),
            ]),
        ),
        // .brain_env key names per environment (never values)
        ("env", env),
    ])
}

/// Filters of `brain map` (INTERNALS §43): a 500-component app answers in a few hundred tokens.
#[derive(Default)]
pub struct Query {
    /// a route path (`/users/:id`) or view file: that route plus the components it uses, transitively
    pub view: Option<String>,
    /// a component name or file
    pub comp: Option<String>,
    /// a store field, context key or resource (`cart`, `cart.total`, `resource:posts.get`): who reads or writes it
    pub reads: Option<String>,
    pub limit: Option<usize>,
    pub cursor: usize,
}

/// `build` plus the reverse dependency graph (`reverse`: key → readers/writers), filtered and paginated: components
/// come `limit` at a time (default 100) with `total` and `nextCursor` while more remain.
pub fn query(root: &Path, q: &Query) -> Json {
    let m = build(root);
    let mut reverse = m
        .get("reverse")
        .cloned()
        .unwrap_or_else(|| Json::obj([("reads", Json::Obj(vec![])), ("writes", Json::Obj(vec![]))]));
    if let (Some(only), Json::Obj(sides)) = (&q.reads, &mut reverse) {
        for (_, side) in sides {
            if let Json::Obj(keys) = side {
                keys.retain(|(key, _)| key == only || key.starts_with(&format!("{only}.")));
            }
        }
    }
    let arr = |k: &str| match m.get(k) {
        Some(Json::Arr(a)) => a.clone(),
        _ => vec![],
    };
    let file_of = |c: &Json| c.get("file").and_then(Json::as_str).unwrap_or("").to_string();
    let (mut routes, mut comps) = (arr("routes"), arr("components"));
    if let Some(v) = &q.view {
        let v = v.trim_start_matches("./");
        routes.retain(|r| r.get("path").and_then(Json::as_str) == Some(v) || r.get("file").and_then(Json::as_str) == Some(v));
        // the view's file and every component it imports, transitively
        let mut keep: Vec<String> = routes
            .iter()
            .filter_map(|r| r.get("file").and_then(Json::as_str).map(str::to_string))
            .collect();
        let mut i = 0;
        while i < keep.len() {
            let f = keep[i].clone();
            if let Some(c) = comps.iter().find(|c| file_of(c) == f) {
                let dir = Path::new(&f).parent().unwrap_or(Path::new("")).to_path_buf();
                if let Some(Json::Arr(imps)) = c.get("imports") {
                    for s in imps.iter().filter_map(Json::as_str).filter(|s| s.ends_with(".brain")) {
                        let t = normalize(&dir.join(s));
                        if !keep.contains(&t) {
                            keep.push(t);
                        }
                    }
                }
            }
            i += 1;
        }
        comps.retain(|c| keep.contains(&file_of(c)));
    }
    if let Some(n) = &q.comp {
        comps.retain(|c| {
            c.get("name").and_then(Json::as_str) == Some(n.as_str())
                || file_of(c) == *n
                || file_of(c).trim_end_matches(".brain").ends_with(n.as_str())
        });
    }
    if q.reads.is_some() {
        let mut touched: Vec<String> = Vec::new();
        for side in ["reads", "writes"] {
            if let Some(Json::Obj(kv)) = reverse.get(side) {
                for (_, files) in kv {
                    if let Json::Arr(fs) = files {
                        touched.extend(fs.iter().filter_map(Json::as_str).map(str::to_string));
                    }
                }
            }
        }
        comps.retain(|c| touched.contains(&file_of(c)));
    }
    let total = comps.len();
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let page: Vec<Json> = comps.into_iter().skip(q.cursor).take(limit).collect();
    let next = q.cursor.saturating_add(page.len());
    let filtered = q.view.is_some() || q.comp.is_some() || q.reads.is_some();
    let mut kv = vec![
        ("routes".to_string(), Json::Arr(routes)),
        ("components".into(), Json::Arr(page)),
        ("total".into(), Json::Num(total as f64)),
    ];
    if next < total {
        kv.push(("nextCursor".into(), Json::Num(next as f64)));
    }
    kv.push(("reverse".into(), reverse));
    if let Some(coverage) = m.get("coverage") {
        kv.push(("coverage".into(), coverage.clone()));
    }
    if !filtered {
        for k in ["modules", "tests", "tokens", "env", "backend", "compatibility"] {
            if let Some(v) = m.get(k) {
                kv.push((k.into(), v.clone()));
            }
        }
    }
    Json::Obj(kv)
}

/// `comp/../ui/x.brain` → `ui/x.brain`.
fn normalize(p: &Path) -> String {
    let mut out: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::Normal(x) => out.push(x.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    out.join("/")
}

/// Human-readable version: one line per item.
pub fn text(j: &Json) -> String {
    let mut o = String::new();
    let arr = |k: &str| match j.get(k) {
        Some(Json::Arr(a)) => a.clone(),
        _ => vec![],
    };
    let s = |v: &Json, k: &str| v.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    o.push_str("routes\n");
    for r in arr("routes") {
        o.push_str(&format!(
            "  {:<24} {}{}\n",
            s(&r, "path"),
            s(&r, "file"),
            if r.get("load") == Some(&Json::Bool(true)) { "  (load)" } else { "" }
        ));
    }
    o.push_str("components\n");
    for c in arr("components") {
        let props = match c.get("props") {
            Some(Json::Arr(p)) => p
                .iter()
                .map(|p| format!("{}:{}", s(p, "name"), s(p, "type")))
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        };
        o.push_str(&format!("  {:<14} {:<32} {props}\n", s(&c, "name"), s(&c, "file")));
    }
    o.push_str("modules\n");
    for m in arr("modules") {
        o.push_str(&format!("  {}\n", s(&m, "file")));
    }
    o.push_str("tests\n");
    for t in arr("tests") {
        o.push_str(&format!("  {}\n", s(&t, "file")));
    }
    if let Some(coverage) = j.get("coverage") {
        if coverage.get("complete") != Some(&Json::Bool(true)) {
            o.push_str("coverage: partial; inspect --json coverage.skipped and coverage.truncated\n");
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_from_files() {
        assert_eq!(route_of("views/home.brain").as_deref(), Some("/"));
        assert_eq!(route_of("views/users/[id].brain").as_deref(), Some("/users/:id"));
        assert_eq!(route_of("views/users/index.brain").as_deref(), Some("/users"));
        assert_eq!(route_of("views/404.brain").as_deref(), Some("*"));
        assert_eq!(route_of("views/users/_layout.brain"), None);
    }

    #[test]
    fn exports_and_tests() {
        assert_eq!(
            exports_of("export const a = 1\nexport async function b() {}\nexport { c, d as e }\nexport default x"),
            ["a", "b", "c", "e", "default"]
        );
        assert_eq!(
            tests_of("test('one', async () => {}); test(\"two\", f)"),
            [Json::str("one"), Json::str("two")]
        );
    }
    struct App(PathBuf);
    impl App {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "brain-map-bounds-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&dir).unwrap();
            Self(dir)
        }
        fn write(&self, path: &str, content: &[u8]) {
            let p = self.0.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn map_query_ignores_symlink_loops_and_external_component_metadata() {
        let app = App::new();
        let external = App::new();
        app.write("comp/local.brain", b"<p>Local</p>");
        external.write("outside.brain", b"<meta props=\"externalSecret:str\"><p>Outside</p>");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&app.0, app.0.join("loop")).unwrap();
            std::os::unix::fs::symlink(&external.0, app.0.join("external")).unwrap();
            let j = query(&app.0, &Query::default());
            let out = j.stringify();
            assert!(!out.contains("externalSecret"));
            assert!(out.contains("comp/local.brain"));
            assert_eq!(j.get("coverage").unwrap().get("complete"), Some(&Json::Bool(false)));
            assert!(out.contains("symlink skipped"));
        }
    }
    #[test]
    fn oversized_and_invalid_sources_are_unknown_and_never_parsed() {
        let app = App::new();
        app.write("comp/large.brain", &vec![b'x'; MAX_FILE_BYTES + 1]);
        app.write("comp/invalid.brain", &[0xff, 0xfe]);
        let j = build(&app.0);
        let out = j.stringify();
        assert!(out.contains("file exceeds 2 MiB") && out.contains("source is not UTF-8"));
        assert_eq!(j.get("components"), Some(&Json::Arr(vec![])));
        assert_eq!(j.get("coverage").unwrap().get("complete"), Some(&Json::Bool(false)));
    }
    #[test]
    fn discovery_limits_depth_files_entries_and_total_source_bytes() {
        let app = App::new();
        let mut path = app.0.clone();
        for _ in 0..34 {
            path.push("nested");
            std::fs::create_dir(&path).unwrap();
        }
        std::fs::write(path.join("deep.brain"), "<p>Deep</p>").unwrap();
        let mut scan = Scan::default();
        walk(&app.0, &app.0, 0, &mut scan);
        assert!(scan.truncated);
        assert!(scan.files.is_empty());
        let app = App::new();
        for n in 0..=MAX_FILES {
            app.write(&format!("comp/{n:05}.brain"), b"<p>Component</p>");
        }
        let mut scan = Scan::default();
        walk(&app.0, &app.0, 0, &mut scan);
        assert_eq!(scan.files.len(), MAX_FILES);
        assert!(scan.truncated);
        let app = App::new();
        for n in 0..=MAX_ENTRIES {
            app.write(&format!("entry-{n}.txt"), b"");
        }
        let mut scan = Scan::default();
        walk(&app.0, &app.0, 0, &mut scan);
        assert!(scan.truncated);
        assert!(scan.entries <= MAX_ENTRIES);
        assert!(scan.files.is_empty());
        let app = App::new();
        app.write("large.brain", &vec![b' '; MAX_FILE_BYTES]);
        let mut scan = Scan::default();
        for _ in 0..MAX_TOTAL_BYTES / MAX_FILE_BYTES {
            assert!(scan.read(&app.0, "large.brain").is_some());
        }
        assert!(scan.read(&app.0, "large.brain").is_none());
        assert!(scan.truncated);
        assert_eq!(scan.bytes, MAX_TOTAL_BYTES);
    }
    #[test]
    fn metadata_keeps_backend_compatibility_fields_and_environment_keys_without_values() {
        let app = App::new();
        app.write("brain.json", b"{\"targets\":\"baseline-2024\"}");
        app.write(".brain_env", b"PUBLIC_TITLE=private-value\nSECRET=never-print-this\n");
        let j = build(&app.0);
        let out = j.stringify();
        assert!(j.get("backend").is_some());
        assert_eq!(
            j.get("compatibility").unwrap().get("target").and_then(Json::as_str),
            Some("baseline-2024")
        );
        assert!(out.contains("PUBLIC_TITLE") && out.contains("SECRET"));
        assert!(!out.contains("private-value") && !out.contains("never-print-this"));
        let query = query(
            &app.0,
            &Query {
                limit: Some(usize::MAX),
                cursor: usize::MAX,
                ..Default::default()
            },
        );
        assert!(query.get("coverage").is_some());
    }
}
