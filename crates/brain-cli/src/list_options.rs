//! B0815: a literal option of a native `list()` call that the resource does not declare in `query.jsbrain`. The
//! native client sends it unchanged and the backend answers 422 INVALID_INPUT on every call (or the client throws 400
//! for a `limit` outside 1..100): a job sweeping one filter it never declared failed every ten minutes, and only its
//! log said so. The rule is the backend's own (`query::filter_problem`, `query::sort_problem`,
//! `relations::include_path_declared`); a dynamic key, operator or value is left to the runtime.
use brain_backend::Resource;
use brain_parse::ast::{Expr, FnBody, MemberProp, ObjProp, Pat, PropKey, Stmt};
use brain_parse::{Diag, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Folders that never hold app sources, and the depth and file count a scan stops at.
const SKIP: &[&str] = &["node_modules", "dist", "target", "public"];
const MAX_DEPTH: usize = 16;
const MAX_FILES: usize = 5000;

fn sources(app: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_symlink() || name.starts_with('.') {
            continue;
        }
        if kind.is_dir() {
            if depth < MAX_DEPTH && !(dir == app && SKIP.contains(&name.as_str())) && name != "node_modules" {
                sources(app, &path, depth + 1, out);
            }
        } else if name.ends_with(".jsbrain") || name.ends_with(".brain") {
            out.push(path);
        }
    }
}

fn key_name(key: &PropKey) -> Option<(&str, Span)> {
    match key {
        PropKey::Ident(i) => Some((i.name.as_str(), i.span)),
        PropKey::Str { value, span, .. } => Some((value.as_str(), *span)),
        _ => None,
    }
}

fn unwrap(e: &Expr) -> &Expr {
    match e {
        Expr::Await { arg, .. } | Expr::Paren { expr: arg, .. } => unwrap(arg),
        _ => e,
    }
}

/// `<…>.backend.resource('name')` (or `backend.resource('name')`) → `name`.
fn resource_of(e: &Expr) -> Option<&str> {
    let Expr::Call { callee, args, .. } = unwrap(e) else { return None };
    let Expr::Member { obj, prop: MemberProp::Name(method), .. } = callee.as_ref() else { return None };
    let backend = match obj.as_ref() {
        Expr::Member { prop: MemberProp::Name(n), .. } | Expr::Ident(n) => n.name == "backend",
        _ => false,
    };
    match (backend && method.name == "resource", args.as_slice()) {
        (true, [Expr::Str { value, .. }]) => Some(value.as_str()),
        _ => None,
    }
}

/// `const rows = ctx.backend.resource('name')` anywhere in the statements (not inside expressions).
fn bindings(stmts: &[Stmt], out: &mut BTreeMap<String, BTreeSet<String>>) {
    for s in stmts {
        match s {
            Stmt::Var(v) => {
                for d in &v.decls {
                    if let (Pat::Ident(name), Some(name_of)) = (&d.pat, d.init.as_ref().and_then(resource_of)) {
                        out.entry(name.name.clone()).or_default().insert(name_of.to_string());
                    }
                }
            }
            Stmt::If { cons, alt, .. } => {
                bindings(std::slice::from_ref(cons.as_ref()), out);
                bindings(alt.as_deref().map(std::slice::from_ref).unwrap_or_default(), out);
            }
            Stmt::ForOf { body, .. } => bindings(std::slice::from_ref(body.as_ref()), out),
            Stmt::Block { body, .. } => bindings(body, out),
            Stmt::Try { block, handler, finalizer, .. } => {
                bindings(block, out);
                bindings(handler.as_deref().unwrap_or_default(), out);
                bindings(finalizer.as_deref().unwrap_or_default(), out);
            }
            Stmt::Func(func) | Stmt::Server { func, .. } => {
                if let FnBody::Block(body) = &func.body {
                    bindings(body, out);
                }
            }
            Stmt::Export { decl, .. } => bindings(std::slice::from_ref(decl.as_ref()), out),
            _ => {}
        }
    }
}

/// Every expression of the statements, nested functions included.
fn expressions(stmts: &[Stmt], f: &mut dyn FnMut(&Expr)) {
    use brain_check::each_expr as each;
    for s in stmts {
        match s {
            Stmt::Var(v) => {
                for d in &v.decls {
                    if let Some(e) = &d.init {
                        each(e, f);
                    }
                }
            }
            Stmt::Expr { expr: e, .. } | Stmt::Throw { arg: e, .. } | Stmt::ExportDefault { expr: e, .. } | Stmt::Return { arg: Some(e), .. } => each(e, f),
            Stmt::If { test, cons, alt, .. } => {
                each(test, f);
                expressions(std::slice::from_ref(cons.as_ref()), f);
                expressions(alt.as_deref().map(std::slice::from_ref).unwrap_or_default(), f);
            }
            Stmt::ForOf { iter, body, .. } => {
                each(iter, f);
                expressions(std::slice::from_ref(body.as_ref()), f);
            }
            Stmt::Block { body, .. } => expressions(body, f),
            Stmt::Try { block, handler, finalizer, .. } => {
                expressions(block, f);
                expressions(handler.as_deref().unwrap_or_default(), f);
                expressions(finalizer.as_deref().unwrap_or_default(), f);
            }
            Stmt::Func(func) | Stmt::Server { func, .. } => match &func.body {
                FnBody::Expr(e) => each(e, f),
                FnBody::Block(body) => expressions(body, f),
            },
            Stmt::Export { decl, .. } => expressions(std::slice::from_ref(decl.as_ref()), f),
            _ => {}
        }
    }
}

/// Literal strings of a `'a,b'` or `['a', 'b']` option; `None` when any part is computed.
fn names(e: &Expr) -> Option<Vec<(String, Span)>> {
    match e {
        Expr::Str { value, span, .. } => Some(value.split(',').map(|v| (v.trim().to_string(), *span)).collect()),
        Expr::Array { elems, .. } => elems.iter().map(|x| match x {
            Some(Expr::Str { value, span, .. }) => Some((value.clone(), *span)),
            _ => None,
        }).collect(),
        _ => None,
    }
}

struct Call<'a> {
    file: &'a str,
    name: &'a str,
    resource: &'a Resource,
    resources: &'a BTreeMap<String, Resource>,
}

impl Call<'_> {
    fn query(&self) -> String {
        format!("server/resources/{}/query.jsbrain", self.name)
    }

    fn warn(&self, diags: &mut Vec<Diag>, span: Span, message: String, fix: String) {
        diags.push(Diag::new("B0815", self.file, span, message, fix));
    }

    fn filter(&self, diags: &mut Vec<Diag>, props: &[ObjProp]) {
        let q = &self.resource.query;
        for prop in props {
            let (field, span, ops) = match prop {
                ObjProp::KeyValue { key, value, .. } => {
                    let Some((field, span)) = key_name(key) else { continue };
                    let ops = match value {
                        Expr::Str { .. } | Expr::Num { .. } | Expr::Bool { .. } => Some(vec![("eq", span)]),
                        Expr::Object { props, .. } => props.iter().map(|p| match p {
                            ObjProp::KeyValue { key, .. } => key_name(key),
                            _ => None,
                        }).collect(),
                        _ => None,
                    };
                    (field, span, ops)
                }
                ObjProp::Shorthand(i) => (i.name.as_str(), i.span, None),
                _ => continue,
            };
            let problem = match &ops {
                Some(ops) => ops.iter().find_map(|&(op, at)| brain_backend::query::filter_problem(self.resource, field, op).map(|p| (op, at, p))),
                // The operator is computed: only an undeclared field is certain.
                None => {
                    let declared = q.filter.contains_key(field)
                        || field.split_once('.').is_some_and(|(group, name)| q.or.get(group).is_some_and(|g| g.contains_key(name)));
                    (!declared).then(|| ("eq", span, brain_backend::query::filter_problem(self.resource, field, "eq").unwrap_or_default()))
                }
            };
            let Some((op, span, reason)) = problem else { continue };
            let declared = q.filter.contains_key(field);
            let (what, fix) = if declared {
                (format!("filters `{field}` with `{op}`, which {} does not declare for it", self.query()),
                    format!("add '{op}' to `filter.{field}` in {}, or use a declared operator", self.query()))
            } else {
                (format!("filters by `{field}`, which {} does not declare", self.query()),
                    format!("declare it under `filter` in {} (for example `{field}: ['{op}']`), or remove it from the call", self.query()))
            };
            self.warn(diags, span, format!("`list()` on `{}` {what}: the backend answers 422 INVALID_INPUT on every call ({reason})", self.name), fix);
        }
    }

    fn options(&self, diags: &mut Vec<Diag>, props: &[ObjProp]) {
        // A spread may replace any option, so nothing in this call is certain.
        if props.iter().any(|p| matches!(p, ObjProp::Spread { .. })) {
            return;
        }
        for prop in props {
            let ObjProp::KeyValue { key, value, .. } = prop else { continue };
            let Some((option, span)) = key_name(key) else { continue };
            match (option, value) {
                ("filter", Expr::Object { props, .. }) => self.filter(diags, props),
                ("sort", value) => {
                    for (raw, span) in names(value).unwrap_or_default() {
                        let field = raw.strip_prefix('-').unwrap_or(&raw);
                        if let Some(reason) = brain_backend::query::sort_problem(self.resource, field) {
                            self.warn(diags, span, format!("`list()` on `{}` sorts by `{field}`, which {} does not declare as sortable: the backend answers 422 INVALID_INPUT on every call ({reason})", self.name, self.query()),
                                format!("add '{field}' to `sort` in {}, or sort by a declared field", self.query()));
                        }
                    }
                }
                ("include", value) => {
                    for (path, span) in names(value).unwrap_or_default() {
                        if !brain_backend::relations::include_path_declared(self.resource, self.resources, &path) {
                            self.warn(diags, span, format!("`list()` on `{}` includes `{path}`, which {} does not declare: the backend answers 422 INVALID_INPUT on every call", self.name, self.query()),
                                format!("add '{path}' to `include` in {} (each level is a ref field its resource declares), or remove it", self.query()));
                        }
                    }
                }
                ("limit" | "offset", Expr::Num { raw, .. }) => {
                    let (low, high) = if option == "limit" { (1, 100) } else { (0, 10000) };
                    if !raw.parse::<f64>().is_ok_and(|n| n.fract() == 0.0 && (low as f64..=high as f64).contains(&n)) {
                        self.warn(diags, span, format!("`list()` on `{}` passes `{option}: {raw}`: the native client accepts {low}..{high} and throws 400 before sending the request", self.name),
                            if option == "limit" { "page with `limit` up to 100 and `next_cursor`".to_string() } else { "use `cursor` (`next_cursor` of the previous page) for deeper pages".to_string() });
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn check(app: &Path) -> Vec<Diag> {
    // Invalid contracts have their own diagnostic (B0735); nothing can be said about their lists then.
    let Ok(contracts) = crate::backend::contracts(app, "resources") else { return Vec::new() };
    if contracts.is_empty() {
        return Vec::new();
    }
    let resources: BTreeMap<String, Resource> = contracts.into_iter().map(|r| (r.name.clone(), r)).collect();
    let mut files = Vec::new();
    sources(app, app, 0, &mut files);
    let mut diags = Vec::new();
    for path in files {
        let Ok(source) = std::fs::read_to_string(&path) else { continue };
        if !source.contains(".list(") {
            continue;
        }
        // A file that does not parse already has its own diagnostic.
        let Ok(component) = brain_parse::parse(&source, &path) else { continue };
        let file = path.strip_prefix(app).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let mut bound = BTreeMap::new();
        bindings(&component.script, &mut bound);
        expressions(&component.script, &mut |e| match e {
            Expr::Func(func) => {
                if let FnBody::Block(body) = &func.body {
                    bindings(body, &mut bound);
                }
            }
            Expr::Object { props, .. } => {
                for p in props {
                    if let ObjProp::Method { func: brain_parse::ast::Func { body: FnBody::Block(body), .. }, .. } = p {
                        bindings(body, &mut bound);
                    }
                }
            }
            _ => {}
        });
        expressions(&component.script, &mut |e| {
            let Expr::Call { callee, args, .. } = e else { return };
            let Expr::Member { obj, prop: MemberProp::Name(method), .. } = callee.as_ref() else { return };
            let Some(Expr::Object { props, .. }) = args.first() else { return };
            if method.name != "list" {
                return;
            }
            let name = resource_of(obj).or_else(|| match unwrap(obj) {
                // A name bound to two resources in the file is ambiguous: skipped.
                Expr::Ident(i) => bound.get(&i.name).filter(|set| set.len() == 1).and_then(|set| set.iter().next()).map(String::as_str),
                _ => None,
            });
            let Some((name, resource)) = name.and_then(|n| resources.get_key_value(n)) else { return };
            Call { file: &file, name, resource, resources: &resources }.options(&mut diags, props);
        });
    }
    diags
}
