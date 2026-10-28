//! `brain map [--json]`: the project graph (routes, components with their props, modules, tests, tokens) so that
//! an LLM reads it instead of opening files (SPEC «CLI»).

use brain_mcp::Json;
use brain_parse::ast::{FileKind, Level, Stmt};
use std::path::{Path, PathBuf};

fn walk(top: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    // A sub-app (folder with its own tokens.cssbrain) is another project: it is not part of this map.
    if dir != top && crate::scaffold::is_app_dir(dir) {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || name == "dist" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            walk(top, &p, out);
        } else if name.ends_with(".brain") || name.ends_with(".jsbrain") {
            out.push(p);
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
        if t.get(j).is_some_and(|x| x.ident("const") || x.ident("let") || x.ident("function") || x.ident("class")) {
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
    let mut files = Vec::new();
    walk(root, root, &mut files);
    let (mut routes, mut comps, mut modules, mut tests) = (vec![], vec![], vec![], vec![]);
    for f in &files {
        let r = rel(root, f);
        let Ok(src) = std::fs::read_to_string(f) else { continue };
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
            comps.push(Json::obj([("file", Json::str(&r)), ("error", Json::str("does not parse; run `brain check`"))]));
            continue;
        };
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
        let imports: Vec<Json> =
            c.script.iter().filter_map(|s| if let Stmt::Import(i) = s { Some(Json::str(&i.source)) } else { None }).collect();
        let has_load = src.contains("function load");
        let server_fns: Vec<Json> = c
            .script
            .iter()
            .filter_map(|s| if let Stmt::Server { func, .. } = s { func.name.as_ref().map(|n| Json::str(&n.name)) } else { None })
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
            && let Some(path) = route_of(&r) {
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
        .find_map(|n| std::fs::read_to_string(root.join(n)).ok().map(|s| (n, s)))
        .and_then(|(n, s)| brain_css::parse_str(&s, n).ok())
        .map(|sheet| brain_css::custom_props(&sheet).into_iter().map(|(k, _)| Json::Str(k)).collect())
        .unwrap_or_default();
    Json::obj([
        ("routes", Json::Arr(routes)),
        ("components", Json::Arr(comps)),
        ("modules", Json::Arr(modules)),
        ("tests", Json::Arr(tests)),
        ("tokens", Json::Arr(tokens)),
        // .brain_env key names per environment (never values)
        ("env", crate::env::map(root).unwrap_or(Json::Obj(vec![]))),
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
    let reverse = brain_check::contract::reverse_json(root, q.reads.as_deref());
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
        let mut keep: Vec<String> = routes.iter().filter_map(|r| r.get("file").and_then(Json::as_str).map(str::to_string)).collect();
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
        comps.retain(|c| c.get("name").and_then(Json::as_str) == Some(n.as_str()) || file_of(c) == *n || file_of(c).trim_end_matches(".brain").ends_with(n.as_str()));
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
    let limit = q.limit.unwrap_or(100).max(1);
    let page: Vec<Json> = comps.into_iter().skip(q.cursor).take(limit).collect();
    let next = q.cursor + page.len();
    let filtered = q.view.is_some() || q.comp.is_some() || q.reads.is_some();
    let mut kv = vec![("routes".to_string(), Json::Arr(routes)), ("components".into(), Json::Arr(page)), ("total".into(), Json::Num(total as f64))];
    if next < total {
        kv.push(("nextCursor".into(), Json::Num(next as f64)));
    }
    kv.push(("reverse".into(), reverse));
    if !filtered {
        for k in ["modules", "tests", "tokens", "env"] {
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
        o.push_str(&format!("  {:<24} {}{}\n", s(&r, "path"), s(&r, "file"), if r.get("load") == Some(&Json::Bool(true)) { "  (load)" } else { "" }));
    }
    o.push_str("components\n");
    for c in arr("components") {
        let props = match c.get("props") {
            Some(Json::Arr(p)) => p.iter().map(|p| format!("{}:{}", s(p, "name"), s(p, "type"))).collect::<Vec<_>>().join(" "),
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
        assert_eq!(exports_of("export const a = 1\nexport async function b() {}\nexport { c, d as e }\nexport default x"), ["a", "b", "c", "e", "default"]);
        assert_eq!(tests_of("test('one', async () => {}); test(\"two\", f)"), [Json::str("one"), Json::str("two")]);
    }
}
