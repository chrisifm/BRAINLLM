//! #276: what the server module of a component may use besides its imports and server functions.
//! - Pure top-level constants (`const PAGE = 20`, `const LABELS = { paid: 'Paid' }`): copied into the server module.
//! - Helpers only server code reaches (`function fetchKpis() { return fetch(process.env.API_URL) }` called from
//!   `load`): they live only in the server module, so private env reads in them are fine (no B0702) and they never
//!   reach the client bundle.
//! Anything reactive (`sig`, `drv`, props, refs, `form`, `use`…) stays client-only and keeps B0704 on the server.

use brain_parse::ast::*;
use brain_parse::visit::Uses;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Shared {
    /// Pure constants and pure functions reached from server code that the client also uses (copied into the server
    /// module, kept in the client).
    pub consts: HashSet<String>,
    /// Functions reached only from server code (moved to the server module, dropped from the client).
    pub helpers: HashSet<String>,
}

impl Shared {
    pub fn contains(&self, n: &str) -> bool {
        self.consts.contains(n) || self.helpers.contains(n)
    }
}

/// `Some(func)` for `function f() {}` and `const f = (…) => …` at the top level.
fn as_func(s: &Stmt) -> Option<(&str, &Func)> {
    match s {
        Stmt::Func(f) => f.name.as_ref().map(|n| (n.name.as_str(), f)),
        Stmt::Var(v) if v.kind == VarKind::Const => match (v.decls.as_slice(), ) {
            ([d],) => match (&d.pat, d.init.as_ref().map(Expr::unparen)) {
                (Pat::Ident(id), Some(Expr::Func(f))) => Some((id.name.as_str(), f.as_ref())),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// A literal value built only from literals and other pure constants.
fn pure(e: &Expr, consts: &HashSet<String>) -> bool {
    match e.unparen() {
        Expr::Num { .. } | Expr::Str { .. } | Expr::Bool { .. } | Expr::Null(_) | Expr::Regex { .. } => true,
        Expr::Template { tag: None, exprs, .. } => exprs.iter().all(|x| pure(x, consts)),
        Expr::Array { elems, .. } => elems.iter().flatten().all(|x| pure(x, consts)),
        Expr::Object { props, .. } => props.iter().all(|p| match p {
            ObjProp::KeyValue { key: PropKey::Computed(_), .. } => false,
            ObjProp::KeyValue { value, .. } => pure(value, consts),
            ObjProp::Shorthand(i) => consts.contains(&i.name),
            ObjProp::Spread { arg, .. } => pure(arg, consts),
            ObjProp::Method { .. } => false,
        }),
        Expr::Unary { arg, .. } => pure(arg, consts),
        Expr::Binary { left, right, .. } => pure(left, consts) && pure(right, consts),
        Expr::Cond { test, cons, alt, .. } => pure(test, consts) && pure(cons, consts) && pure(alt, consts),
        Expr::Ident(i) => consts.contains(&i.name),
        Expr::Member { obj, prop: MemberProp::Name(_), .. } => pure(obj, consts),
        _ => false,
    }
}

fn is_server_fn(s: &Stmt) -> Option<&Func> {
    match s {
        Stmt::Server { func, .. } => Some(func),
        Stmt::Export { decl, .. } => match decl.as_ref() {
            Stmt::Func(f) if f.name.as_ref().is_some_and(|n| matches!(n.name.as_str(), "load" | "paths" | "sitemap")) => Some(f),
            _ => None,
        },
        _ => None,
    }
}

pub fn shared(c: &Component) -> Shared {
    if c.kind != FileKind::Component {
        return Shared::default();
    }
    // Top-level pure constants, to a fixpoint (a constant may be built from another).
    let mut consts: HashSet<String> = HashSet::new();
    loop {
        let before = consts.len();
        for s in &c.script {
            if let Stmt::Var(v) = s
                && v.kind == VarKind::Const
                && let [d] = v.decls.as_slice()
                && let (Pat::Ident(id), Some(init)) = (&d.pat, &d.init)
                && !matches!(init.unparen(), Expr::Func(_))
                && pure(init, &consts)
            {
                consts.insert(id.name.clone());
            }
        }
        if consts.len() == before {
            break;
        }
    }
    let funcs: HashMap<&str, &Func> = c.script.iter().filter_map(as_func).collect();
    let reads_of = |f: &Func| {
        let mut u = Uses::default();
        u.func(f);
        u.reads.into_iter().chain(u.calls).collect::<HashSet<String>>()
    };
    let closure = |roots: HashSet<String>| {
        let mut seen: HashSet<String> = HashSet::new();
        let mut todo: Vec<String> = roots.into_iter().collect();
        while let Some(n) = todo.pop() {
            if !seen.insert(n.clone()) {
                continue;
            }
            if let Some(f) = funcs.get(n.as_str()) {
                todo.extend(reads_of(f));
            }
        }
        seen
    };
    // Roots on each side.
    let mut server_roots = HashSet::new();
    let mut client_roots = HashSet::new();
    for s in &c.script {
        if let Some(f) = is_server_fn(s) {
            server_roots.extend(reads_of(f));
            continue;
        }
        if as_func(s).is_some() || matches!(s, Stmt::Import(_)) {
            continue;
        }
        let mut u = Uses::default();
        u.stmt(s);
        client_roots.extend(u.reads.into_iter().chain(u.calls));
    }
    let mut u = Uses::default();
    u.nodes(&c.template);
    for p in &c.props {
        if let Some(d) = &p.default {
            u.expr(d);
        }
    }
    client_roots.extend(u.reads.into_iter().chain(u.calls));
    let server = closure(server_roots);
    let client = closure(client_roots);
    // Names a server-only helper may not touch: the component's reactive state and props.
    let top = crate::scope::top_bindings(c);
    let reactive = |n: &str| matches!(top.get(n), Some(k) if !matches!(k, crate::scope::Kind::Const | crate::scope::Kind::Func | crate::scope::Kind::Import)) || c.props.iter().any(|p| p.name == n);
    // Functions server code reaches that read no reactive state: the server gets a copy; the ones the client never
    // reaches leave the client bundle.
    let pure_fns: HashSet<String> = funcs
        .iter()
        .filter(|(n, f)| server.contains(**n) && !reads_of(f).iter().any(|r| reactive(r) && !consts.contains(r) && !funcs.contains_key(r.as_str())))
        .map(|(n, _)| n.to_string())
        .collect();
    let helpers: HashSet<String> = pure_fns.iter().filter(|n| !client.contains(*n)).cloned().collect();
    let consts = consts.into_iter().filter(|n| server.contains(n)).chain(pure_fns.into_iter().filter(|n| client.contains(n))).collect();
    Shared { consts, helpers }
}
