//! Cheap reading for an LLM (INTERNALS §27): a file's outline without bodies, the project graph, a focused
//! subgraph and the minimal file set for a task within a token budget. JSON is written by hand (no serde).

use crate::{meta_names, scope};
use brain_parse::push_json_str;
use brain_parse::ast::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// What a file declares, without bodies.
#[derive(Clone, Debug, Default)]
pub struct Outline {
    pub file: String,
    pub name: String,
    /// `component` | `view` | `ui` | `app` | `module`.
    pub kind: &'static str,
    /// `name:type` with `?` (optional) or `~` (bindable).
    pub props: Vec<String>,
    pub emits: Vec<String>,
    pub expose: Vec<String>,
    /// `name: sig|drv|ref|form|const|let|fn|server|load`.
    pub state: Vec<(String, &'static str)>,
    /// Component tags used in the markup.
    pub children: Vec<String>,
    /// `(source, names)`.
    pub imports: Vec<(String, Vec<String>)>,
    /// Exported names of a `.jsbrain`.
    pub exports: Vec<String>,
    /// Approximate tokens of the whole file (bytes / 4).
    pub tokens: usize,
}

pub fn outline(c: &Component) -> Outline {
    let kind = match (c.kind, c.level) {
        (FileKind::App, _) => "app",
        (FileKind::Module, _) => "module",
        (_, Level::View) => "view",
        (_, Level::Micro) => "ui",
        _ => "component",
    };
    let mut o = Outline { file: c.file.clone(), name: c.name.clone(), kind, tokens: c.src.len().div_ceil(4), ..Outline::default() };
    for p in &c.props {
        o.props.push(format!("{}:{}{}{}", p.name, p.ty.as_str(), if p.bindable { "~" } else { "" }, if p.optional { "?" } else { "" }));
    }
    o.emits = meta_names(c, "emits");
    o.expose = meta_names(c, "expose");
    for s in &c.script {
        let (decl, exported) = match s {
            Stmt::Export { decl, .. } => (decl.as_ref(), true),
            other => (other, false),
        };
        match decl {
            Stmt::Import(i) => {
                let names = i.default.iter().chain(i.namespace.iter()).chain(i.named.iter().map(|(_, l)| l)).map(|x| x.name.clone()).collect();
                o.imports.push((i.source.clone(), names));
            }
            Stmt::Var(v) => {
                for d in &v.decls {
                    let mut ids = Vec::new();
                    d.pat.bound_names(&mut ids);
                    let k = match scope::init_kind(d.init.as_ref()) {
                        Some(scope::Kind::Sig) => "sig",
                        Some(scope::Kind::Drv) => "drv",
                        Some(scope::Kind::Ref) => "ref",
                        Some(scope::Kind::Form) => "form",
                        _ if matches!(d.init.as_ref().map(Expr::unparen), Some(Expr::Func(_))) => "fn",
                        _ if v.kind == VarKind::Const => "const",
                        _ => "let",
                    };
                    for id in ids {
                        if exported {
                            o.exports.push(id.name.clone());
                        }
                        o.state.push((id.name.clone(), k));
                    }
                }
            }
            Stmt::Func(f) => {
                if let Some(n) = &f.name {
                    if exported {
                        o.exports.push(n.name.clone());
                    }
                    o.state.push((n.name.clone(), if n.name == "load" { "load" } else { "fn" }));
                }
            }
            Stmt::Server { func, .. } => {
                if let Some(n) = &func.name {
                    o.state.push((n.name.clone(), "server"));
                }
            }
            Stmt::ExportDefault { .. } => o.exports.push("default".into()),
            _ => {}
        }
    }
    let mut kids = BTreeSet::new();
    tags(&c.template, &mut kids);
    o.children = kids.into_iter().collect();
    o
}

fn tags(nodes: &[Node], out: &mut BTreeSet<String>) {
    for n in nodes {
        match n {
            Node::Element(e) => tags(&e.children, out),
            Node::ComponentRef(r) => {
                out.insert(r.name.clone());
                tags(&r.children, out);
            }
            Node::If(b) => {
                for br in &b.branches {
                    tags(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    tags(o, out);
                }
            }
            Node::Each(b) => {
                tags(&b.body, out);
                if let Some(e) = &b.empty {
                    tags(e, out);
                }
            }
            Node::SlotScope(b) => tags(&b.body, out),
            Node::Await(b) => {
                tags(&b.pending, out);
                for br in [&b.then, &b.catch].into_iter().flatten() {
                    tags(&br.body, out);
                }
            }
            _ => {}
        }
    }
}

fn arr(out: &mut String, key: &str, v: &[String]) {
    out.push_str(&format!(",\"{key}\":["));
    for (i, s) in v.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_json_str(out, s);
    }
    out.push(']');
}

impl Outline {
    pub fn to_json(&self) -> String {
        let mut s = String::from("{\"file\":");
        push_json_str(&mut s, &self.file);
        s.push_str(",\"name\":");
        push_json_str(&mut s, &self.name);
        s.push_str(&format!(",\"kind\":\"{}\",\"tokens\":{}", self.kind, self.tokens));
        arr(&mut s, "props", &self.props);
        arr(&mut s, "emits", &self.emits);
        arr(&mut s, "expose", &self.expose);
        s.push_str(",\"state\":{");
        for (i, (n, k)) in self.state.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            push_json_str(&mut s, n);
            s.push_str(&format!(":\"{k}\""));
        }
        s.push('}');
        arr(&mut s, "children", &self.children);
        s.push_str(",\"imports\":[");
        for (i, (src, names)) in self.imports.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str("{\"from\":");
            push_json_str(&mut s, src);
            arr(&mut s, "names", names);
            s.push('}');
        }
        s.push(']');
        arr(&mut s, "exports", &self.exports);
        s.push('}');
        s
    }
}

/// Every `.brain`/`.jsbrain` of an app and its import edges (paths relative to the root, `/`-separated).
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: BTreeMap<String, Outline>,
    /// file → files it imports (resolved, existing only).
    pub edges: BTreeMap<String, BTreeSet<String>>,
    /// Files that failed to parse (still listed so the agent sees them).
    pub broken: Vec<String>,
    texts: HashMap<String, String>,
}

/// Parses the whole app under `root` (skips `dist`, `node_modules`, dot folders and tests).
pub fn graph(root: &Path) -> Graph {
    let mut g = Graph::default();
    let mut stack = vec![root.to_path_buf()];
    let mut files: Vec<PathBuf> = Vec::new();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with('.') || matches!(n.as_str(), "dist" | "node_modules" | "target") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if (n.ends_with(".brain") || n.ends_with(".jsbrain")) && !n.ends_with(".test.brain") {
                files.push(p);
            }
        }
    }
    files.sort();
    for abs in files {
        let rel = brain_parse::display_path(abs.strip_prefix(root).unwrap_or(&abs));
        let Ok(src) = std::fs::read_to_string(&abs) else { continue };
        match brain_parse::parse(&src, Path::new(&rel)) {
            Ok(c) => {
                let o = outline(&c);
                let dir = Path::new(&rel).parent().unwrap_or(Path::new("")).to_path_buf();
                let mut deps = BTreeSet::new();
                for (srcp, _) in &o.imports {
                    if srcp.starts_with("./") || srcp.starts_with("../") {
                        let target = normalize(&dir.join(srcp));
                        if root.join(&target).exists() {
                            deps.insert(target);
                        }
                    }
                }
                g.edges.insert(rel.clone(), deps);
                g.nodes.insert(rel.clone(), o);
            }
            Err(_) => g.broken.push(rel.clone()),
        }
        g.texts.insert(rel, src.to_ascii_lowercase());
    }
    g
}

fn normalize(p: &Path) -> String {
    let mut out: Vec<String> = Vec::new();
    for c in p.components() {
        match c.as_os_str().to_str().unwrap_or("") {
            "." | "" => {}
            ".." => {
                out.pop();
            }
            s => out.push(s.to_string()),
        }
    }
    out.join("/")
}

impl Graph {
    /// Files that import `file`.
    pub fn importers(&self, file: &str) -> Vec<String> {
        self.edges.iter().filter(|(_, d)| d.contains(file)).map(|(f, _)| f.clone()).collect()
    }

    /// `file` (or the view of a route like `/users/42`), what it imports and what imports it.
    pub fn focus(&self, target: &str) -> Vec<String> {
        let Some(file) = self.resolve(target) else { return Vec::new() };
        let mut set = BTreeSet::new();
        set.insert(file.clone());
        set.extend(self.edges.get(&file).into_iter().flatten().cloned());
        set.extend(self.importers(&file));
        set.into_iter().collect()
    }

    /// A file path, a component name or a route (`/users/42` → `views/users/[id].brain`).
    pub fn resolve(&self, target: &str) -> Option<String> {
        if self.nodes.contains_key(target) {
            return Some(target.to_string());
        }
        if let Some(f) = self.nodes.values().find(|o| o.name == target) {
            return Some(f.file.clone());
        }
        let route = target.trim_start_matches('/').trim_end_matches('/');
        let segs: Vec<&str> = if route.is_empty() { vec!["index"] } else { route.split('/').collect() };
        self.nodes.keys().find(|f| {
            let Some(r) = f.strip_prefix("views/").and_then(|r| r.strip_suffix(".brain")) else { return false };
            let parts: Vec<&str> = r.split('/').collect();
            parts.len() == segs.len() && parts.iter().zip(&segs).all(|(p, s)| p == s || (p.starts_with('[') && p.ends_with(']')))
        }).cloned()
    }

    /// The minimal set of files for a task: files whose names, declarations or text match the task words,
    /// then their neighbours with decaying weight; cheapest-first within `budget` tokens.
    pub fn context(&self, task: &str, budget: usize) -> Vec<(String, f64, usize)> {
        let words: Vec<String> = task
            .to_ascii_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2 && !STOP.contains(w))
            .map(|w| w.trim_end_matches('s').to_string())
            .collect();
        let mut score: BTreeMap<String, f64> = BTreeMap::new();
        for (f, o) in &self.nodes {
            let head = format!("{} {} {} {} {}", f.to_ascii_lowercase(), o.name.to_ascii_lowercase(), o.props.join(" "), o.emits.join(" "), o.state.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(" ")).to_ascii_lowercase();
            let text = self.texts.get(f).map(String::as_str).unwrap_or("");
            let mut s = 0.0;
            for w in &words {
                if head.contains(w.as_str()) {
                    s += 3.0;
                }
                s += (text.matches(w.as_str()).count() as f64).min(5.0) * 0.3;
            }
            if s > 0.0 {
                score.insert(f.clone(), s);
            }
        }
        // Spread to imports and importers (half weight, one hop).
        let seeds: Vec<(String, f64)> = score.iter().map(|(f, s)| (f.clone(), *s)).collect();
        for (f, s) in seeds {
            for n in self.edges.get(&f).into_iter().flatten().cloned().chain(self.importers(&f)) {
                *score.entry(n).or_insert(0.0) += s * 0.5;
            }
        }
        let mut ranked: Vec<(String, f64, usize)> = score.into_iter().map(|(f, s)| {
            let t = self.nodes.get(&f).map(|o| o.tokens).unwrap_or(0);
            (f, s, t)
        }).collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
        let mut used = 0;
        ranked.into_iter().filter(|(_, _, t)| {
            if used + t <= budget {
                used += t;
                true
            } else {
                false
            }
        }).collect()
    }

    /// `{"files":[outline…],"edges":{"a":["b"]},"broken":[…]}` for `brain map --json` / `--focus`.
    pub fn to_json(&self, only: Option<&[String]>) -> String {
        let keep = |f: &String| only.is_none_or(|o| o.contains(f));
        let files: Vec<String> = self.nodes.iter().filter(|(f, _)| keep(f)).map(|(_, o)| o.to_json()).collect();
        let mut s = format!("{{\"files\":[{}],\"edges\":{{", files.join(","));
        let mut first = true;
        for (f, d) in self.edges.iter().filter(|(f, _)| keep(f)) {
            if !first {
                s.push(',');
            }
            first = false;
            push_json_str(&mut s, f);
            s.push(':');
            let v: Vec<String> = d.iter().cloned().collect();
            let mut a = String::new();
            arr(&mut a, "x", &v);
            s.push_str(&a[5..]);
        }
        s.push('}');
        arr(&mut s, "broken", &self.broken);
        s.push('}');
        s
    }
}

const STOP: &[&str] = &["the", "and", "for", "with", "add", "make", "change", "update", "fix", "new", "into", "from", "that", "this", "when", "page", "view", "component"];
