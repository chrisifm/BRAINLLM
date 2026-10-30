//! INTERNALS §36: the `.brain_env` file — one file, every environment.
//!
//! `KEY=value` lines before any `[section]` are shared; a `[name]` section overrides them for that environment.
//! dotenv syntax: optional single/double quotes, `#` comments (outside quotes), `${OTHER}` expanded within the same
//! environment. Keys are `UPPER_SNAKE_CASE`. Precedence: process environment > section > shared.
//!
//! Diagnostics never contain a value, only key names:
//! - B0720 a key whose final value is empty in the build environment ([`empty_keys`], for `brain build`/`start`).
//! - B0721 (warning) a key used in code but missing from `.brain_env` (edit adds `KEY=` to the shared block).
//! - B0722 (info) a key declared but used nowhere in the app ([`unused_keys`]).
//! - B0723 a `PUBLIC_*` value shaped like a secret (it would ship to every visitor).
//! - B0724 a malformed line or section header.

use brain_parse::{Diag, Span};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub const FILE: &str = ".brain_env";

/// Keys the toolchain reads (brain build, brain-seo, brain start): never reported as unused (B0722).
pub const TOOLING_KEYS: &[&str] = &["PUBLIC_SITE_URL", "PUBLIC_SITE_NAME", "INDEXNOW_KEY", "BRAIN_SECRET", "PORT", "HOST", "NODE_ENV", "DATABASE_URL", "BRAIN_BACKEND_URL", "BRAIN_JWT_SECRET", "BRAIN_JWT_ISSUER", "BRAIN_JWT_AUDIENCE", "GOOGLE_CLIENT_ID", "BRAIN_GCS_BUCKET", "BRAIN_GCS_PREFIX", "BRAIN_GCS_ADAPTER", "BRAIN_MAIL_RESEND_KEY", "BRAIN_MAIL_FROM", "BRAIN_VIEWER_PROFILE"];

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub value: String,
    /// 1-based line in the file.
    pub line: u32,
    /// Byte offset of the start of the line.
    pub offset: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnvFile {
    pub shared: Vec<Entry>,
    /// `[name]` sections in file order.
    pub sections: Vec<(String, Vec<Entry>)>,
    /// `(line, offset, message)` of malformed lines (B0724). The message never quotes the value.
    pub errors: Vec<(u32, usize, String)>,
    /// Byte offset just after the last line of the shared block (where B0721 inserts `KEY=`).
    pub shared_end: usize,
}

fn is_key(k: &str) -> bool {
    !k.is_empty() && k.starts_with(|c: char| c.is_ascii_uppercase()) && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn unquote(raw: &str) -> Result<String, &'static str> {
    let v = raw.trim();
    for q in ['"', '\''] {
        if let Some(rest) = v.strip_prefix(q) {
            return match rest.find(q) {
                Some(end) => {
                    let after = rest[end + 1..].trim();
                    if !after.is_empty() && !after.starts_with('#') {
                        return Err("text after the closing quote");
                    }
                    let inner = &rest[..end];
                    Ok(if q == '"' { inner.replace("\\n", "\n").replace("\\\"", "\"") } else { inner.to_string() })
                }
                None => Err("unclosed quote"),
            };
        }
    }
    // Unquoted: a ` #` (or a `#` right after `=`) starts a comment.
    if v.starts_with('#') {
        return Ok(String::new());
    }
    let v = match v.find(" #").or_else(|| v.find("\t#")) {
        Some(i) => &v[..i],
        None => v,
    };
    Ok(v.trim().to_string())
}

/// Parses the text of a `.brain_env` file.
pub fn parse(src: &str) -> EnvFile {
    let mut f = EnvFile::default();
    let mut current: Option<usize> = None;
    let mut offset = 0;
    let mut seen_shared: HashSet<String> = HashSet::new();
    let mut seen_section: HashSet<String> = HashSet::new();
    for (i, raw) in src.split('\n').enumerate() {
        let line = (i + 1) as u32;
        let start = offset;
        offset += raw.len() + 1;
        let t = raw.trim().trim_end_matches('\r');
        if t.is_empty() || t.starts_with('#') {
            if current.is_none() {
                // Comments and blank lines after the last shared key still belong to the shared block's tail only
                // when a key follows; `shared_end` is updated on keys.
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix('[') {
            let name = rest.split_once(']').map(|(n, after)| (n.trim(), after.trim()));
            match name {
                Some((n, after)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && (after.is_empty() || after.starts_with('#')) => {
                    if f.sections.iter().any(|(s, _)| s == n) {
                        f.errors.push((line, start, format!("section `[{n}]` appears twice")));
                    }
                    f.sections.push((n.to_string(), Vec::new()));
                    current = Some(f.sections.len() - 1);
                    seen_section.clear();
                }
                _ => f.errors.push((line, start, "a section header is `[name]` (letters, digits, `-`, `_`)".into())),
            }
            continue;
        }
        let t = t.strip_prefix("export ").unwrap_or(t);
        let Some((k, v)) = t.split_once('=') else {
            f.errors.push((line, start, "expected `KEY=value`".into()));
            continue;
        };
        let key = k.trim();
        if !is_key(key) {
            f.errors.push((line, start, format!("`{key}` is not an UPPER_SNAKE_CASE key")));
            continue;
        }
        let value = match unquote(v) {
            Ok(v) => v,
            Err(why) => {
                f.errors.push((line, start, format!("`{key}`: {why}")));
                continue;
            }
        };
        let seen = if current.is_some() { &mut seen_section } else { &mut seen_shared };
        if !seen.insert(key.to_string()) {
            f.errors.push((line, start, format!("`{key}` is declared twice in the same block")));
        }
        let e = Entry { key: key.to_string(), value, line, offset: start };
        match current {
            Some(s) => f.sections[s].1.push(e),
            None => {
                f.shared.push(e);
                f.shared_end = offset.min(src.len());
            }
        }
    }
    f
}

impl EnvFile {
    /// Every key declared anywhere (shared or any section).
    pub fn keys(&self) -> HashSet<String> {
        self.shared.iter().chain(self.sections.iter().flat_map(|(_, e)| e.iter())).map(|e| e.key.clone()).collect()
    }

    /// Section names in file order.
    pub fn environments(&self) -> Vec<String> {
        self.sections.iter().map(|(n, _)| n.clone()).collect()
    }

    /// Final values for `env`: process > section > shared, with `${OTHER}` expanded inside that environment.
    pub fn resolve(&self, env: &str, process: &dyn Fn(&str) -> Option<String>) -> BTreeMap<String, String> {
        let mut raw: BTreeMap<String, String> = BTreeMap::new();
        for e in &self.shared {
            raw.insert(e.key.clone(), e.value.clone());
        }
        if let Some((_, entries)) = self.sections.iter().find(|(n, _)| n == env) {
            for e in entries {
                raw.insert(e.key.clone(), e.value.clone());
            }
        }
        for (k, v) in raw.iter_mut() {
            if let Some(p) = process(k) {
                *v = p;
            }
        }
        // Expansion, at most a few rounds so `A=${B}`, `B=${C}` resolve and a cycle stops.
        let mut out = raw.clone();
        for _ in 0..8 {
            let mut changed = false;
            let snapshot = out.clone();
            for v in out.values_mut() {
                let mut s = String::new();
                let mut rest = v.as_str();
                while let Some(i) = rest.find("${") {
                    s.push_str(&rest[..i]);
                    match rest[i + 2..].find('}') {
                        Some(end) => {
                            let name = &rest[i + 2..i + 2 + end];
                            s.push_str(snapshot.get(name).map(String::as_str).or(process(name).as_deref()).unwrap_or(""));
                            rest = &rest[i + 3 + end..];
                        }
                        None => {
                            s.push_str(&rest[i..]);
                            rest = "";
                        }
                    }
                }
                s.push_str(rest);
                if s != *v {
                    *v = s;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        out
    }

    /// The `PUBLIC_*` values of `env`, for the codegen (constants in client code, §13).
    pub fn public(&self, env: &str, process: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
        self.resolve(env, process).into_iter().filter(|(k, _)| k.starts_with("PUBLIC_")).collect()
    }
}

fn span_at(src: &str, line: u32, offset: usize) -> Span {
    let len = src[offset.min(src.len())..].find('\n').unwrap_or(src.len() - offset.min(src.len()));
    Span { start: offset as u32, end: (offset + len) as u32, line, col: 1 }
}

/// B0724 and B0723 for the file itself (`file` is how the diagnostics name it, usually `.brain_env`).
pub fn check_file(src: &str, file: &str) -> Vec<Diag> {
    let f = parse(src);
    let mut out = Vec::new();
    for (line, offset, msg) in &f.errors {
        out.push(Diag::new("B0724", file, span_at(src, *line, *offset), format!("malformed line: {msg}"), "write `KEY=value` (UPPER_SNAKE_CASE key, optional quotes) or a `[section]` header; comments start with `#`"));
    }
    for e in f.shared.iter().chain(f.sections.iter().flat_map(|(_, e)| e.iter())) {
        if e.key.starts_with("PUBLIC_") && !crate::scan_secrets(&e.value).is_empty() {
            out.push(Diag::new(
                "B0723",
                file,
                span_at(src, e.line, e.offset),
                format!("`{}` looks like a secret and every `PUBLIC_*` value ships to the browser", e.key),
                format!("rename it without `PUBLIC_` and read it on the server (`load()`, `server:`, `ctx.env.{}`)", e.key.trim_start_matches("PUBLIC_")),
            ));
        }
    }
    out
}

/// B0720: keys with an empty final value in `env` (call from `brain build` / `brain start`).
pub fn empty_keys(src: &str, file: &str, env: &str, process: &dyn Fn(&str) -> Option<String>) -> Vec<Diag> {
    let f = parse(src);
    let resolved = f.resolve(env, process);
    let mut out = Vec::new();
    for (k, v) in &resolved {
        if v.trim().is_empty() {
            let decl = f.sections.iter().find(|(n, _)| n == env).and_then(|(_, es)| es.iter().find(|e| &e.key == k)).or_else(|| f.shared.iter().find(|e| &e.key == k));
            let span = decl.map(|e| span_at(src, e.line, e.offset)).unwrap_or(Span { start: 0, end: 0, line: 1, col: 1 });
            out.push(Diag::new("B0720", file, span, format!("variable `{k}` has no value in `{env}`"), format!("set `{k}=…` under `[{env}]` in .brain_env or in the host's environment")));
        }
    }
    out
}

/// Env keys read by a source file: `import.meta.env.X`, `process.env.X`, `ctx.env.X` (also `['X']`).
pub fn keys_used(src: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for prefix in ["import.meta.env", "process.env", "ctx.env"] {
        let mut from = 0;
        while let Some(i) = src[from..].find(prefix) {
            let at = from + i;
            let rest = &src[at + prefix.len()..];
            let key = if let Some(r) = rest.strip_prefix('.') {
                r.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or("")
            } else if let Some(r) = rest.strip_prefix("['").or_else(|| rest.strip_prefix("[\"")) {
                r.split(['\'', '"']).next().unwrap_or("")
            } else {
                ""
            };
            if is_key(key) {
                out.push((key.to_string(), at));
            }
            from = at + prefix.len();
        }
    }
    out
}

/// B0722: keys declared in `<root>/.brain_env` that no `.brain`/`.jsbrain` of the app reads.
pub fn unused_keys(root: &Path) -> Vec<Diag> {
    let Ok(src) = std::fs::read_to_string(root.join(FILE)) else { return Vec::new() };
    let f = parse(&src);
    let mut used = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "dist" | "target") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if (name.ends_with(".brain") || name.ends_with(".jsbrain") || name.ends_with(".js") || name.ends_with(".mjs"))
                && let Ok(text) = std::fs::read_to_string(&p)
            {
                used.extend(keys_used(&text).into_iter().map(|(k, _)| k));
            }
        }
    }
    let mut first: HashMap<String, &Entry> = HashMap::new();
    for e in f.shared.iter().chain(f.sections.iter().flat_map(|(_, e)| e.iter())) {
        first.entry(e.key.clone()).or_insert(e);
    }
    let mut keys: Vec<&String> = first.keys().collect();
    keys.sort();
    keys.into_iter()
        // Keys read by the toolchain itself (site URL/name for SEO files, IndexNow), not by app code.
        .filter(|k| !used.contains(*k) && !TOOLING_KEYS.contains(&k.as_str()))
        .map(|k| {
            let e = first[k];
            Diag::new("B0722", FILE, span_at(&src, e.line, e.offset), format!("`{k}` is declared but nothing reads it"), format!("delete `{k}` from every block of .brain_env, or read it with `ctx.env.{k}`")).info()
        })
        .collect()
}

/// Declared and framework-managed environment keys, plus source used only for diagnostic locations.
#[derive(Clone, Debug, Default)]
pub struct EnvInfo {
    pub keys: HashSet<String>,
    /// Where `KEY=` is inserted (end of the shared block, or the file start when there is none).
    pub insert_at: usize,
    pub src_len: usize,
    /// Line index of the env file, to locate the B0721 edit.
    pub src: String,
}

/// Loads `<root>/.brain_env` for the checker.
pub fn load(root: &Path) -> Option<EnvInfo> {
    let src = std::fs::read_to_string(root.join(FILE)).ok()?;
    let f = parse(&src);
    let mut keys = f.keys();
    keys.extend(TOOLING_KEYS.iter().map(|key|(*key).to_owned()));
    Some(EnvInfo { keys, insert_at: f.shared_end, src_len: src.len(), src })
}
