//! INTERNALS §43: what each component reads and writes, its contract card and the project's reverse dependency
//! graph ("if I change `cart.total`, these components update"). Consumed by `brain map --json` and `brain_card`.
//!
//! Keys: a store field is `<export>.<field>` (`cart.total`), a whole store `<export>`; a context key is `<K>` and its
//! fields `<K>.<field>`; a resource is `resource:<callee>` (`resource:posts.get`); a prop is `prop:<name>`.

use crate::{meta_names, security};
use brain_parse::Json;
use brain_parse::ast::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

/// Methods that mutate the value they are called on.
const MUTATORS: &[&str] = &["push", "pop", "shift", "unshift", "splice", "sort", "reverse", "fill", "set", "update", "add", "delete", "clear"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deps {
    pub reads: BTreeSet<String>,
    pub writes: BTreeSet<String>,
}

/// Locals that stand for shared state (channel imports, `use(K)` and `resource(…)` results) and the keys read just by
/// declaring them (`use(K)`, `resource(…)`).
fn tracked(c: &Component) -> (HashMap<String, String>, BTreeSet<String>) {
    let mut t = HashMap::new();
    let mut always = BTreeSet::new();
    for s in &c.script {
        let decl = match s {
            Stmt::Export { decl, .. } => decl.as_ref(),
            other => other,
        };
        match decl {
            Stmt::Import(i) if i.source.contains("channels/") => {
                for (imported, local) in &i.named {
                    t.insert(local.name.clone(), imported.name.clone());
                }
                if let Some(d) = &i.default {
                    t.insert(d.name.clone(), d.name.clone());
                }
            }
            Stmt::Var(v) => {
                for d in &v.decls {
                    let (Pat::Ident(id), Some(Expr::Call { callee, args, .. })) = (&d.pat, d.init.as_ref().map(Expr::unparen)) else { continue };
                    let Expr::Ident(f) = callee.as_ref() else { continue };
                    match (f.name.as_str(), args.first()) {
                        ("use", Some(Expr::Ident(k))) => {
                            t.insert(id.name.clone(), k.name.clone());
                            always.insert(k.name.clone());
                        }
                        ("resource", Some(a)) => {
                            let k = format!("resource:{}", brain_parse::dump::expr(a));
                            t.insert(id.name.clone(), k.clone());
                            always.insert(k);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    (t, always)
}

/// `x` → base, `x.f…` → `base.f` (resources keep their base: `posts.more()` is not a field).
fn key_of(e: &Expr, t: &HashMap<String, String>) -> Option<String> {
    match e.unparen() {
        Expr::Ident(i) => t.get(&i.name).cloned(),
        Expr::Member { obj, prop, .. } => match (obj.unparen(), prop) {
            (Expr::Ident(i), MemberProp::Name(p)) => t.get(&i.name).map(|b| if b.starts_with("resource:") { b.clone() } else { format!("{b}.{}", p.name) }),
            (o, _) => key_of(o, t),
        },
        _ => None,
    }
}

/// What `c` reads and writes of shared state, plus the props it reads (`prop:<name>`).
pub fn deps(c: &Component) -> Deps {
    let (t, always) = tracked(c);
    let props: HashSet<&str> = c.props.iter().map(|p| p.name.as_str()).collect();
    let mut d = Deps { reads: always, ..Deps::default() };
    let mut targets: HashSet<(u32, u32)> = HashSet::new();
    let mut bare: BTreeSet<String> = BTreeSet::new();
    let mut f = |e: &Expr, _: bool, _: &[String]| {
        match e {
            Expr::Assign { target, .. } | Expr::Update { arg: target, .. } => {
                if let Some(k) = key_of(target, &t) {
                    d.writes.insert(k);
                    let s = target.span();
                    targets.insert((s.start, s.end));
                }
            }
            Expr::Call { callee, .. } => {
                if let Expr::Member { obj, prop: MemberProp::Name(m), .. } = callee.unparen() {
                    if MUTATORS.contains(&m.name.as_str()) {
                        if let Some(k) = key_of(obj, &t) {
                            d.writes.insert(k);
                            let s = obj.span();
                            targets.insert((s.start, s.end));
                        }
                    }
                }
            }
            _ => {}
        }
        let s = e.span();
        if targets.contains(&(s.start, s.end)) {
            return;
        }
        match e {
            Expr::Member { obj, prop: MemberProp::Name(_), .. } if matches!(obj.unparen(), Expr::Ident(i) if t.contains_key(&i.name)) => {
                if let Some(k) = key_of(e, &t) {
                    d.reads.insert(k);
                }
            }
            Expr::Ident(i) if t.contains_key(&i.name) => {
                bare.insert(t[&i.name].clone());
            }
            Expr::Ident(i) if props.contains(i.name.as_str()) => {
                d.reads.insert(format!("prop:{}", i.name));
            }
            Expr::Member { obj, prop: MemberProp::Name(p), .. } if matches!(obj.unparen(), Expr::Ident(i) if i.name == "props") && props.contains(p.name.as_str()) => {
                d.reads.insert(format!("prop:{}", p.name));
            }
            _ => {}
        }
    };
    security::walk(&c.script, false, &mut f);
    crate::planned::collect_calls(&c.template, &mut f);
    // A bare store read (`{cart}`, `fn(cart)`) depends on the whole store, unless only its fields are read.
    for b in bare {
        if !d.reads.iter().any(|r| r.starts_with(&format!("{b}."))) {
            d.reads.insert(b);
        }
    }
    d
}

/// Lines of markup + script (the `<style>` block does not count) and bytes of the whole file (B1010, cards).
pub fn size(c: &Component) -> (usize, usize) {
    let mut lines = c.src.lines().count();
    if let (Some(a), Some(b)) = (c.src.find("<style"), c.src.rfind("</style>")) {
        if a < b {
            lines -= c.src[a..b].lines().count();
        }
    }
    (lines, c.src.len())
}

fn slots(nodes: &[Node], out: &mut BTreeSet<String>) {
    for n in nodes {
        match n {
            Node::Slot(s) => {
                out.insert(s.name.clone().unwrap_or_else(|| "default".into()));
            }
            Node::Element(Element { children, .. }) | Node::ComponentRef(ComponentRef { children, .. }) => slots(children, out),
            Node::If(b) => {
                for br in &b.branches {
                    slots(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    slots(o, out);
                }
            }
            Node::Each(b) => {
                slots(&b.body, out);
                if let Some(e) = &b.empty {
                    slots(e, out);
                }
            }
            Node::SlotScope(b) => slots(&b.body, out),
            Node::Await(b) => {
                slots(&b.pending, out);
                for br in [&b.then, &b.catch].into_iter().flatten() {
                    slots(&br.body, out);
                }
            }
            _ => {}
        }
    }
}

fn strs<'a>(it: impl IntoIterator<Item = &'a String>) -> Json {
    Json::Arr(it.into_iter().map(|s| Json::str(s.clone())).collect())
}

/// The contract of a component, without its body: what a model needs to compose it (`brain_card`).
/// `status` is `(errors, warnings)` from `check` (the caller runs it; `None` = not checked).
pub fn card(c: &Component, status: Option<(usize, usize)>) -> Json {
    let o = crate::outline::outline(c);
    let d = deps(c);
    let props = c
        .props
        .iter()
        .map(|p| {
            let mut kv = vec![("name".to_string(), Json::str(&p.name)), ("type".to_string(), Json::str(p.ty.as_str()))];
            if p.optional {
                kv.push(("optional".into(), Json::Bool(true)));
            }
            if p.bindable {
                kv.push(("bindable".into(), Json::Bool(true)));
            }
            if let Some(def) = &p.default {
                kv.push(("default".into(), Json::str(brain_parse::dump::expr(def))));
            }
            Json::Obj(kv)
        })
        .collect();
    let mut sl = BTreeSet::new();
    slots(&c.template, &mut sl);
    let (lines, bytes) = size(c);
    let reads: Vec<&String> = d.reads.iter().filter(|r| !r.starts_with("prop:")).collect();
    let mut kv = vec![
        ("name".to_string(), Json::str(&c.name)),
        ("file".to_string(), Json::str(&c.file)),
        ("kind".to_string(), Json::str(o.kind)),
        ("props".to_string(), Json::Arr(props)),
        ("emits".to_string(), strs(&meta_names(c, "emits"))),
        ("slots".to_string(), strs(&sl)),
        ("reads".to_string(), strs(reads)),
        ("writes".to_string(), strs(&d.writes)),
        ("children".to_string(), strs(&o.children)),
        ("size".to_string(), Json::obj([("lines", Json::Num(lines as f64)), ("bytes", Json::Num(bytes as f64))])),
    ];
    if let Some((e, w)) = status {
        kv.push(("status".into(), Json::obj([("errors", Json::Num(e as f64)), ("warnings", Json::Num(w as f64))])));
    }
    Json::Obj(kv)
}

/// Reverse graph of a project: key → files that read it, and key → files that write it. Props are per component and
/// stay out of it (they are in the card).
pub fn reverse(root: &Path) -> (BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<String>>) {
    let (mut r, mut w): (BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<String>>) = Default::default();
    for f in crate::doctor::load_files(root) {
        if f.1.kind == FileKind::Module {
            continue;
        }
        let d = deps(&f.1);
        for k in d.reads.into_iter().filter(|k| !k.starts_with("prop:")) {
            r.entry(k).or_default().push(f.0.clone());
        }
        for k in d.writes {
            w.entry(k).or_default().push(f.0.clone());
        }
    }
    (r, w)
}

/// `reverse` as JSON: `{ "reads": { key: [files] }, "writes": { key: [files] } }`, optionally only the keys equal to
/// `only` or under it (`cart` matches `cart.total`).
pub fn reverse_json(root: &Path, only: Option<&str>) -> Json {
    let (r, w) = reverse(root);
    let keep = |k: &str| only.is_none_or(|o| k == o || k.starts_with(&format!("{o}.")));
    let map = |m: BTreeMap<String, Vec<String>>| Json::Obj(m.into_iter().filter(|(k, _)| keep(k)).map(|(k, v)| (k, strs(&v))).collect());
    Json::obj([("reads", map(r)), ("writes", map(w))])
}
