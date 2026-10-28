//! INTERNALS §43: `brain split <comp> "<selector>" <NewName>` — extract one element subtree into a new component.
//!
//! - Props: every name of the original (prop, script local, `{#each}`/`{#await}` binding around the subtree) the
//!   subtree reads; the type comes from the original prop or the `sig(…)` initializer, `fn` for functions, else `any`.
//!   `bind:value={x}` on an original signal makes `x` a model prop (`x:type~`) bound by the parent.
//! - Emits: a handler that calls an original function (`on:click={remove}`, `on:click={remove(it.id)}`) becomes
//!   `emit('remove', …)` in the new component and `on:remove={remove}` in the parent; any other handler over original
//!   state (`count++`, `emit('close')`) becomes an event named after the DOM event, run by the parent.
//! - Imports used by the subtree are copied (paths rewritten); CSS rules that reach the subtree are copied into the new
//!   component (the parent's `@scope` stops at child components) and removed from the parent when nothing else there
//!   uses them.
//! - Both results must parse or the split is refused. The CLI writes the files and runs `brain check` on them.

use brain_parse::ast::*;
use brain_css::CssItem;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component as PC, Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    /// New source of the original file.
    pub original: String,
    /// Source of the new component.
    pub extracted: String,
    /// `name:type` (with `~` when bindable), in the order they were found.
    pub props: Vec<String>,
    pub emits: Vec<String>,
}

/// Elements matched by a simple selector: `tag`, `.class`, `#id`, `tag.class`, `[attr]`… (one compound, no combinators).
fn matches(e: &Element, sel: &str) -> bool {
    let mut rest = sel;
    let tag_end = rest.find(['.', '#', '[']).unwrap_or(rest.len());
    let (tag, tail) = rest.split_at(tag_end);
    if !tag.is_empty() && tag != "*" && !tag.eq_ignore_ascii_case(&e.tag) {
        return false;
    }
    rest = tail;
    let text_attr = |name: &str| {
        e.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == name).map(|a| match &a.value {
            AttrValue::Text(t) => t.clone(),
            _ => String::new(),
        })
    };
    while !rest.is_empty() {
        let c = rest.as_bytes()[0];
        if c == b'[' {
            let end = rest.find(']').unwrap_or(rest.len() - 1);
            let inner = &rest[1..end];
            let (name, val) = match inner.split_once('=') {
                Some((n, v)) => (n.trim(), Some(v.trim().trim_matches(['"', '\'']))),
                None => (inner.trim(), None),
            };
            match (e.attrs.iter().find(|a| a.name == name), val) {
                (None, _) => return false,
                (Some(a), Some(v)) => {
                    if !matches!(&a.value, AttrValue::Text(t) if t == v) {
                        return false;
                    }
                }
                _ => {}
            }
            rest = &rest[end + 1..];
            continue;
        }
        let end = rest[1..].find(['.', '#', '[']).map(|i| i + 1).unwrap_or(rest.len());
        let word = &rest[1..end];
        let ok = match c {
            b'.' => text_attr("class").is_some_and(|t| t.split_whitespace().any(|w| w == word)),
            b'#' => text_attr("id").as_deref() == Some(word),
            _ => false,
        };
        if !ok {
            return false;
        }
        rest = &rest[end..];
    }
    true
}

/// Matching elements with the names bound around them by `{#each}`/`{#await}`.
fn find<'a>(nodes: &'a [Node], sel: &str, bound: &mut Vec<String>, out: &mut Vec<(&'a Element, Vec<String>)>) {
    for n in nodes {
        match n {
            Node::Element(e) => {
                if matches(e, sel) {
                    out.push((e, bound.clone()));
                }
                find(&e.children, sel, bound, out);
            }
            Node::ComponentRef(r) => find(&r.children, sel, bound, out),
            Node::If(b) => {
                for br in &b.branches {
                    find(&br.body, sel, bound, out);
                }
                if let Some(o) = &b.otherwise {
                    find(o, sel, bound, out);
                }
            }
            Node::Each(b) => {
                let n0 = bound.len();
                bound.extend(each_names(b));
                find(&b.body, sel, bound, out);
                bound.truncate(n0);
                if let Some(e) = &b.empty {
                    find(e, sel, bound, out);
                }
            }
            Node::SlotScope(b) => {
                let n0 = bound.len();
                bound.extend(b.names.iter().map(|i| i.name.clone()));
                find(&b.body, sel, bound, out);
                bound.truncate(n0);
            }
            Node::Await(a) => {
                find(&a.pending, sel, bound, out);
                for br in [&a.then, &a.catch].into_iter().flatten() {
                    let n0 = bound.len();
                    bound.extend(pat_names(br.binding.as_ref()));
                    find(&br.body, sel, bound, out);
                    bound.truncate(n0);
                }
            }
            _ => {}
        }
    }
}

fn pat_names(p: Option<&Pat>) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(p) = p {
        p.bound_names(&mut ids);
    }
    ids.into_iter().map(|i| i.name.clone()).collect()
}

fn each_names(b: &EachBlock) -> Vec<String> {
    let mut v = pat_names(Some(&b.item));
    v.extend(b.index.iter().map(|i| i.name.clone()));
    v
}

/// Free identifiers an expression reads (member names and object keys excluded), in order.
fn idents(e: &Expr, out: &mut Vec<String>) {
    crate::security::expr(e, false, &[], &mut |x, _, _| {
        if let Expr::Ident(i) = x {
            out.push(i.name.clone());
        }
        if let Expr::Object { props, .. } = x {
            for p in props {
                if let ObjProp::Shorthand(i) = p {
                    out.push(i.name.clone());
                }
            }
        }
    });
}

fn attr_exprs(a: &Attr) -> Vec<&Expr> {
    match &a.value {
        AttrValue::Expr(x) => vec![x],
        AttrValue::Mixed(parts) => parts.iter().filter_map(|p| if let AttrPart::Expr(x) = p { Some(x) } else { None }).collect(),
        _ => Vec::new(),
    }
}

/// What the subtree needs from outside.
#[derive(Default)]
struct Needs {
    /// Names read (in order of first use).
    reads: Vec<String>,
    /// Names bound to `bind:value|checked` (model props).
    binds: Vec<String>,
    /// (attr span, child attr text, parent attr text, event name)
    handlers: Vec<(Span, String, String, String)>,
    /// Component tags used.
    tags: BTreeSet<String>,
    classes: BTreeSet<String>,
    elems: BTreeSet<String>,
    /// `props.<f>` fields read.
    prop_fields: Vec<String>,
}

struct Ctx<'a> {
    src: &'a str,
    /// Names the original defines that the subtree can see: prop, script local, outer binding.
    outer: &'a HashSet<String>,
    fns: &'a HashSet<String>,
    events: HashSet<String>,
}

fn walk(nodes: &[Node], cx: &mut Ctx, inner: &mut Vec<String>, n: &mut Needs) -> Result<(), String> {
    for node in nodes {
        match node {
            Node::Element(Element { tag, attrs, children, .. }) | Node::ComponentRef(ComponentRef { name: tag, attrs, children, .. }) => {
                if matches!(node, Node::ComponentRef(_)) {
                    n.tags.insert(tag.clone());
                } else {
                    n.elems.insert(tag.to_ascii_lowercase());
                }
                for a in attrs {
                    if a.kind == AttrKind::Plain && a.name == "class" {
                        if let AttrValue::Text(t) = &a.value {
                            n.classes.extend(t.split_whitespace().map(str::to_string));
                        }
                    }
                    if a.kind == AttrKind::Class {
                        n.classes.insert(a.name.clone());
                    }
                    handle_attr(a, cx, inner, n)?;
                }
                walk(children, cx, inner, n)?;
            }
            Node::Interp(i) => read(&i.expr, inner, n),
            Node::If(b) => {
                for br in &b.branches {
                    read(&br.test, inner, n);
                    walk(&br.body, cx, inner, n)?;
                }
                if let Some(o) = &b.otherwise {
                    walk(o, cx, inner, n)?;
                }
            }
            Node::Each(b) => {
                read(&b.list, inner, n);
                let n0 = inner.len();
                inner.extend(each_names(b));
                if let Some(k) = &b.key {
                    read(k, inner, n);
                }
                walk(&b.body, cx, inner, n)?;
                inner.truncate(n0);
                if let Some(e) = &b.empty {
                    walk(e, cx, inner, n)?;
                }
            }
            Node::Await(a) => {
                read(&a.promise, inner, n);
                walk(&a.pending, cx, inner, n)?;
                for br in [&a.then, &a.catch].into_iter().flatten() {
                    let n0 = inner.len();
                    inner.extend(pat_names(br.binding.as_ref()));
                    walk(&br.body, cx, inner, n)?;
                    inner.truncate(n0);
                }
            }
            Node::Slot(_) => return Err("the subtree contains a <slot>; extract a smaller subtree".into()),
            Node::SlotScope(b) => {
                let n0 = inner.len();
                inner.extend(b.names.iter().map(|x| x.name.clone()));
                walk(&b.body, cx, inner, n)?;
                inner.truncate(n0);
            }
            Node::Text(_) => {}
        }
    }
    Ok(())
}

fn read(e: &Expr, inner: &[String], n: &mut Needs) {
    let mut ids = Vec::new();
    idents(e, &mut ids);
    for id in ids {
        if !inner.contains(&id) && !n.reads.contains(&id) {
            n.reads.push(id);
        }
    }
    crate::security::expr(e, false, &[], &mut |x, _, _| {
        if let Expr::Member { obj, prop: MemberProp::Name(p), .. } = x {
            if matches!(obj.unparen(), Expr::Ident(i) if i.name == "props") && !n.prop_fields.contains(&p.name) {
                n.prop_fields.push(p.name.clone());
            }
        }
    });
}

fn handle_attr(a: &Attr, cx: &mut Ctx, inner: &[String], n: &mut Needs) -> Result<(), String> {
    match a.kind {
        AttrKind::Bind if a.name == "ref" => {
            let mut ids = Vec::new();
            for x in attr_exprs(a) {
                idents(x, &mut ids);
            }
            if let Some(r) = ids.iter().find(|i| cx.outer.contains(*i) && !inner.contains(i)) {
                return Err(format!("the subtree binds the ref `{r}` of the original; move the code that uses it first"));
            }
            Ok(())
        }
        AttrKind::Bind => {
            for x in attr_exprs(a) {
                let root = match x.unparen() {
                    Expr::Ident(i) => Some(i.name.clone()),
                    Expr::Member { .. } => {
                        let mut ids = Vec::new();
                        idents(x, &mut ids);
                        ids.into_iter().next()
                    }
                    _ => None,
                };
                if let Some(r) = root.filter(|r| cx.outer.contains(r) && !inner.contains(r)) {
                    if !n.binds.contains(&r) {
                        n.binds.push(r);
                    }
                }
            }
            Ok(())
        }
        AttrKind::On => {
            let Some(x) = attr_exprs(a).into_iter().next() else { return Ok(()) };
            let mut ids = Vec::new();
            idents(x, &mut ids);
            let uses_outer = ids.iter().any(|i| cx.outer.contains(i) && !inner.contains(i)) || ids.iter().any(|i| i == "emit");
            if !uses_outer {
                read(x, inner, n);
                return Ok(());
            }
            let text = |e: &Expr| cx.src[e.span().start as usize..e.span().end as usize].to_string();
            let (ev, child, parent) = match x.unparen() {
                Expr::Ident(f) if cx.fns.contains(&f.name) => (f.name.clone(), format!("on:{}={{(e) => emit('{}', e)}}", a.name, f.name), format!("on:{0}={{{0}}}", f.name)),
                Expr::Call { callee, args, .. } if matches!(callee.unparen(), Expr::Ident(f) if cx.fns.contains(&f.name)) => {
                    let Expr::Ident(f) = callee.unparen() else { unreachable!() };
                    for g in args {
                        read(g, inner, n);
                    }
                    let (data, parent) = match args.len() {
                        0 => (String::new(), format!("on:{0}={{{0}}}", f.name)),
                        1 => (format!(", {}", text(&args[0])), format!("on:{0}={{{0}}}", f.name)),
                        _ => (format!(", [{}]", args.iter().map(&text).collect::<Vec<_>>().join(", ")), format!("on:{0}={{(v) => {0}(...v)}}", f.name)),
                    };
                    (f.name.clone(), format!("on:{}={{emit('{}'{data})}}", a.name, f.name), parent)
                }
                // Writes to the original's state only (`filter = 'all'`, `count++`): the written names become model
                // props and the handler stays as written.
                other if !ids.iter().any(|i| i == "emit" || (cx.fns.contains(i) && !inner.contains(i))) && {
                    let mut ok = false;
                    crate::security::expr(other, false, &[], &mut |x, _, _| {
                        if let Expr::Assign { .. } | Expr::Update { .. } = x {
                            ok = true;
                        }
                    });
                    ok
                } =>
                {
                    crate::security::expr(other, false, &[], &mut |x, _, _| {
                        if let Expr::Assign { target, .. } | Expr::Update { arg: target, .. } = x {
                            let mut r = Vec::new();
                            idents(target, &mut r);
                            if let Some(r) = r.into_iter().next().filter(|r| cx.outer.contains(r) && !inner.contains(r)) {
                                if !n.binds.contains(&r) {
                                    n.binds.push(r);
                                }
                            }
                        }
                    });
                    read(x, inner, n);
                    return Ok(());
                }
                other => {
                    if let Some(b) = ids.iter().find(|i| inner.contains(i)) {
                        return Err(format!("the `on:{}` handler mixes `{b}` (bound inside the subtree) with state of the original; call a function instead: `on:{}={{fn({b})}}`", a.name, a.name));
                    }
                    let mut ev = a.name.clone();
                    let mut k = 2;
                    while cx.events.contains(&ev) {
                        ev = format!("{}{k}", a.name);
                        k += 1;
                    }
                    let body = text(other);
                    let parent = if matches!(other, Expr::Func(_)) { format!("on:{ev}={{{body}}}") } else { format!("on:{ev}={{() => {body}}}") };
                    (ev.clone(), format!("on:{}={{(e) => emit('{ev}', e)}}", a.name), parent)
                }
            };
            cx.events.insert(ev.clone());
            n.handlers.push((a.span, child, parent, ev));
            Ok(())
        }
        _ => {
            for x in attr_exprs(a) {
                read(x, inner, n);
            }
            Ok(())
        }
    }
}

fn type_of(init: Option<&Expr>) -> &'static str {
    let lit = |e: &Expr| match e.unparen() {
        Expr::Num { .. } => "num",
        Expr::Str { .. } | Expr::Template { .. } => "str",
        Expr::Bool { .. } => "bool",
        Expr::Array { .. } => "list",
        Expr::Object { .. } => "obj",
        Expr::Func(_) => "fn",
        _ => "any",
    };
    match init.map(Expr::unparen) {
        Some(Expr::Call { callee, args, .. }) if matches!(callee.unparen(), Expr::Ident(i) if i.name == "sig") => args.first().map_or("any", lit),
        Some(e) => lit(e),
        None => "any",
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in p.components() {
        match part {
            PC::CurDir => {}
            PC::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Import path from the file `from` to the project path `to` (`./x`, `../comp/x.brain`).
fn rel_import(from: &str, to: &Path) -> String {
    let from_dir: Vec<String> = Path::new(from).parent().unwrap_or(Path::new("")).components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    let to_parts: Vec<String> = to.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    let common = from_dir.iter().zip(&to_parts).take_while(|(a, b)| a == b).count();
    let ups = from_dir.len() - common;
    let rest = to_parts[common..].join("/");
    if ups == 0 { format!("./{rest}") } else { format!("{}{rest}", "../".repeat(ups)) }
}

/// Top-level CSS items as (source text range, mentioned classes, mentioned tags).
fn css_items(c: &Component) -> Vec<(usize, usize, BTreeSet<String>, BTreeSet<String>, bool)> {
    fn mentions(items: &[CssItem], cls: &mut BTreeSet<String>, tags: &mut BTreeSet<String>) {
        for it in items {
            match it {
                CssItem::Rule { selector, items, .. } => {
                    let b = selector.as_bytes();
                    let mut i = 0;
                    while i < b.len() {
                        let start_word = |i: usize| {
                            let mut j = i;
                            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b'_') {
                                j += 1;
                            }
                            j
                        };
                        if b[i] == b'.' {
                            let j = start_word(i + 1);
                            cls.insert(selector[i + 1..j].to_string());
                            i = j;
                        } else if b[i].is_ascii_alphabetic() && (i == 0 || matches!(b[i - 1], b' ' | b'>' | b'+' | b'~' | b',' | b'(')) {
                            let j = start_word(i);
                            tags.insert(selector[i..j].to_ascii_lowercase());
                            i = j;
                        } else {
                            i += 1;
                        }
                    }
                    mentions(items, cls, tags);
                }
                CssItem::At { block: Some(b), .. } => mentions(b, cls, tags),
                _ => {}
            }
        }
    }
    let Some(sheet) = &c.style else { return Vec::new() };
    sheet
        .items
        .iter()
        .map(|it| {
            let (mut cls, mut tags) = (BTreeSet::new(), BTreeSet::new());
            mentions(std::slice::from_ref(it), &mut cls, &mut tags);
            let s = it.span();
            let mixin = matches!(it, CssItem::At { name, .. } if name == "mixin");
            (s.start as usize, s.end as usize, cls, tags, mixin)
        })
        .collect()
}

/// Extracts the one element matching `selector` from `c` into a new component `name` that will live at `new_rel`
/// (project-relative, e.g. `comp/todo-row.brain`).
pub fn split(c: &Component, selector: &str, name: &str, new_rel: &str) -> Result<Split, String> {
    if !name.chars().next().is_some_and(|ch| ch.is_ascii_uppercase()) || !name.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(format!("`{name}` is not a component name; use PascalCase, e.g. `TodoRow`"));
    }
    let selector = selector.trim();
    if selector.contains([' ', '>', '+', '~', ',']) {
        return Err(format!("`{selector}` has combinators; use one compound selector such as `li.todo` or `#summary`"));
    }
    let mut found = Vec::new();
    find(&c.template, selector, &mut Vec::new(), &mut found);
    let (el, outer_bound) = match found.len() {
        0 => return Err(format!("no element of `{}` matches `{selector}`", c.file)),
        1 => found.remove(0),
        k => return Err(format!("`{selector}` matches {k} elements of `{}`; add a class or id that only the subtree has", c.file)),
    };

    // What the original defines.
    let mut kinds: HashMap<String, &'static str> = HashMap::new();
    let mut fns: HashSet<String> = HashSet::new();
    let mut imports: BTreeMap<String, (String, String)> = BTreeMap::new(); // local → (import text, resolved path)
    let dir = Path::new(&c.file).parent().unwrap_or(Path::new("")).to_path_buf();
    for s in &c.script {
        let decl = match s {
            Stmt::Export { decl, .. } => decl.as_ref(),
            other => other,
        };
        match decl {
            Stmt::Import(i) => {
                let target = if i.source.starts_with('.') { normalize(&dir.join(&i.source)) } else { PathBuf::from(&i.source) };
                let path = if i.source.starts_with('.') { rel_import(new_rel, &target) } else { i.source.clone() };
                if let Some(d) = &i.default {
                    imports.insert(d.name.clone(), (format!("import {} from '{path}'", d.name), path.clone()));
                }
                for (imp, local) in &i.named {
                    let spec = if imp.name == local.name { imp.name.clone() } else { format!("{} as {}", imp.name, local.name) };
                    imports.insert(local.name.clone(), (format!("import {{ {spec} }} from '{path}'"), path.clone()));
                }
                if let Some(ns) = &i.namespace {
                    imports.insert(ns.name.clone(), (format!("import * as {} from '{path}'", ns.name), path.clone()));
                }
            }
            Stmt::Var(v) => {
                for d in &v.decls {
                    let mut ids = Vec::new();
                    d.pat.bound_names(&mut ids);
                    let t = type_of(d.init.as_ref());
                    for id in ids {
                        if t == "fn" {
                            fns.insert(id.name.clone());
                        }
                        kinds.insert(id.name.clone(), t);
                    }
                }
            }
            Stmt::Func(f) => {
                if let Some(n) = &f.name {
                    fns.insert(n.name.clone());
                    kinds.insert(n.name.clone(), "fn");
                }
            }
            Stmt::Server { func, .. } => {
                if let Some(n) = &func.name {
                    fns.insert(n.name.clone());
                    kinds.insert(n.name.clone(), "fn");
                }
            }
            _ => {}
        }
    }
    for p in &c.props {
        kinds.insert(p.name.clone(), p.ty.as_str());
        if p.ty == PropType::Fn {
            fns.insert(p.name.clone());
        }
    }
    for b in &outer_bound {
        kinds.insert(b.clone(), "any");
    }
    let outer: HashSet<String> = kinds.keys().cloned().collect();

    let mut cx = Ctx { src: &c.src, outer: &outer, fns: &fns, events: HashSet::new() };
    let mut n = Needs::default();
    walk(std::slice::from_ref(&Node::Element(el.clone())), &mut cx, &mut Vec::new(), &mut n)?;

    // Props and the attributes the parent passes.
    let mut props: Vec<String> = Vec::new();
    let mut pass: Vec<String> = Vec::new();
    let mut prop_names: HashSet<String> = HashSet::new();
    let mut child_imports: Vec<String> = Vec::new();
    for t in &n.tags {
        if let Some((line, _)) = imports.get(t) {
            child_imports.push(line.clone());
        }
    }
    for r in &n.reads {
        if let Some((line, _)) = imports.get(r) {
            if !child_imports.contains(line) {
                child_imports.push(line.clone());
            }
            continue;
        }
        if r == "props" {
            for f in &n.prop_fields {
                if prop_names.insert(f.clone()) {
                    let t = c.props.iter().find(|p| &p.name == f).map_or("any", |p| p.ty.as_str());
                    props.push(format!("{f}:{t}"));
                    pass.push(format!("{f}={{props.{f}}}"));
                }
            }
            continue;
        }
        let Some(t) = kinds.get(r) else { continue };
        if n.binds.contains(r) || !prop_names.insert(r.clone()) {
            continue;
        }
        props.push(format!("{r}:{t}"));
        pass.push(format!("{r}={{{r}}}"));
    }
    for b in &n.binds {
        if prop_names.insert(b.clone()) {
            let t = kinds.get(b).copied().unwrap_or("any");
            let t = if t == "any" { "str" } else { t };
            props.push(format!("{b}:{t}~"));
            pass.push(format!("bind:{b}={{{b}}}"));
        }
    }
    let emits: Vec<String> = {
        let mut v: Vec<String> = Vec::new();
        for (_, _, _, ev) in &n.handlers {
            if !v.contains(ev) {
                v.push(ev.clone());
            }
        }
        v
    };
    for (_, _, parent, _) in &n.handlers {
        if !pass.contains(parent) {
            pass.push(parent.clone());
        }
    }

    // New component: markup with handlers rewritten.
    let (s0, s1) = (el.span.start as usize, el.span.end as usize);
    let mut markup = c.src[s0..s1].to_string();
    let mut hs: Vec<&(Span, String, String, String)> = n.handlers.iter().collect();
    hs.sort_by_key(|h| std::cmp::Reverse(h.0.start));
    for (sp, child, _, _) in hs {
        markup.replace_range(sp.start as usize - s0..sp.end as usize - s0, child);
    }
    let markup = dedent(&markup);

    // CSS that reaches the subtree.
    let items = css_items(c);
    let mut rest_classes = BTreeSet::new();
    let mut rest_tags = BTreeSet::new();
    {
        // Classes and tags of the original without the subtree.
        let without = format!("{}{}", &c.src[..s0], &c.src[s1..]);
        if let Ok(rc) = brain_parse::parse(&without, Path::new(&c.file)) {
            let mut nn = Needs::default();
            let mut cx2 = Ctx { src: &without, outer: &outer, fns: &fns, events: HashSet::new() };
            let _ = walk(&rc.template, &mut cx2, &mut Vec::new(), &mut nn);
            rest_classes = nn.classes;
            rest_tags = nn.elems;
        }
    }
    let mut css_child = Vec::new();
    let mut css_remove: Vec<(usize, usize)> = Vec::new();
    let mut needs_mixins = false;
    for (a, b, cls, tags, mixin) in &items {
        if *mixin {
            continue;
        }
        let reaches = cls.iter().any(|k| n.classes.contains(k)) || (cls.is_empty() && tags.iter().any(|t| n.elems.contains(t)));
        if !reaches {
            continue;
        }
        let text = c.src[*a..*b].to_string();
        needs_mixins |= text.contains("@include");
        css_child.push(text);
        let still = cls.iter().any(|k| rest_classes.contains(k)) || (cls.is_empty() && tags.iter().any(|t| rest_tags.contains(t)));
        if !still {
            css_remove.push((*a, *b));
        }
    }
    if needs_mixins {
        for (a, b, _, _, mixin) in &items {
            if *mixin {
                css_child.insert(0, c.src[*a..*b].to_string());
            }
        }
    }

    let mut extracted = String::new();
    let mut meta = Vec::new();
    if !props.is_empty() {
        meta.push(format!("props=\"{}\"", props.join(" ")));
    }
    if !emits.is_empty() {
        meta.push(format!("emits=\"{}\"", emits.join(" ")));
    }
    if !meta.is_empty() {
        extracted.push_str(&format!("<meta {}>\n\n", meta.join(" ")));
    }
    if !child_imports.is_empty() {
        extracted.push_str("<script>\n");
        for l in &child_imports {
            extracted.push_str(&format!("  {l}\n"));
        }
        extracted.push_str("</script>\n\n");
    }
    extracted.push_str(&markup);
    extracted.push('\n');
    if !css_child.is_empty() {
        extracted.push_str("\n<style>\n");
        for r in &css_child {
            extracted.push_str(&format!("  {}\n", r.trim()));
        }
        extracted.push_str("</style>\n");
    }

    // Original: the subtree becomes `<Name …/>`, its import is added, moved CSS is removed.
    let usage = if pass.is_empty() { format!("<{name} />") } else { format!("<{name} {} />", pass.join(" ")) };
    let import = format!("import {name} from '{}'", rel_import(&c.file, Path::new(new_rel)));
    let mut edits: Vec<(usize, usize, String)> = vec![(s0, s1, usage)];
    for (a, b) in css_remove {
        let mut b = b;
        while c.src.as_bytes().get(b).is_some_and(|x| *x == b' ' || *x == b'\t') {
            b += 1;
        }
        if c.src.as_bytes().get(b) == Some(&b'\n') {
            b += 1;
        }
        let mut a = a;
        while a > 0 && matches!(c.src.as_bytes()[a - 1], b' ' | b'\t') {
            a -= 1;
        }
        edits.push((a, b, String::new()));
    }
    match c.src.find("<script>") {
        Some(at) => edits.push((at + "<script>".len(), at + "<script>".len(), format!("\n  {import}"))),
        None => {
            // After the leading `<meta …>` tags, or at the top.
            let mut at = 0;
            while c.src[at..].trim_start().starts_with("<meta") {
                let st = at + (c.src[at..].len() - c.src[at..].trim_start().len());
                match c.src[st..].find('>') {
                    Some(e) => at = st + e + 1,
                    None => break,
                }
            }
            let lead = if at == 0 { "" } else { "\n\n" };
            edits.push((at, at, format!("{lead}<script>\n  {import}\n</script>{}", if at == 0 { "\n\n" } else { "" })));
        }
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut original = c.src.clone();
    for (a, b, t) in edits {
        original.replace_range(a..b, &t);
    }
    // An emptied `<style>` goes away.
    if let (Some(a), Some(b)) = (original.find("<style>"), original.find("</style>")) {
        if original[a + 7..b].trim().is_empty() {
            let end = b + "</style>".len();
            let start = original[..a].trim_end().len();
            original.replace_range(start..end, "");
            if !original.ends_with('\n') {
                original.push('\n');
            }
        }
    }

    brain_parse::parse(&extracted, Path::new(new_rel)).map_err(|d| format!("the extracted component does not parse ({}); split refused", d.first().map_or(String::new(), |x| x.msg.clone())))?;
    brain_parse::parse(&original, Path::new(&c.file)).map_err(|d| format!("`{}` would not parse after the split ({}); split refused", c.file, d.first().map_or(String::new(), |x| x.msg.clone())))?;
    Ok(Split { original, extracted, props, emits })
}

/// B1010 limits (INTERNALS §43): lines of markup + script, or bytes of the whole file.
pub const MAX_LINES: usize = 150;
pub const MAX_BYTES: usize = 20 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum Suggestion {
    /// The biggest block a unique selector can extract.
    Split { selector: String, name: String, line: u32 },
    /// Sibling blocks with the same structure.
    Repeated { line: u32, count: usize },
}

fn elements<'a>(nodes: &'a [Node], out: &mut Vec<&'a Element>) {
    for n in nodes {
        match n {
            Node::Element(e) => {
                out.push(e);
                elements(&e.children, out);
            }
            Node::ComponentRef(r) => elements(&r.children, out),
            Node::If(b) => {
                for br in &b.branches {
                    elements(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    elements(o, out);
                }
            }
            Node::Each(b) => {
                elements(&b.body, out);
                if let Some(e) = &b.empty {
                    elements(e, out);
                }
            }
            Node::SlotScope(b) => elements(&b.body, out),
            Node::Await(a) => {
                elements(&a.pending, out);
                for br in [&a.then, &a.catch].into_iter().flatten() {
                    elements(&br.body, out);
                }
            }
            _ => {}
        }
    }
}

fn pascal(s: &str) -> String {
    s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(|w| {
        let mut ch = w.chars();
        ch.next().map(|f| f.to_ascii_uppercase().to_string() + ch.as_str()).unwrap_or_default()
    }).collect()
}

/// What B1010 proposes: repeated sibling blocks first (views stay thin), else the biggest non-root block (at least
/// 15 % of the markup) that one selector picks out.
pub fn suggest(c: &Component) -> Option<Suggestion> {
    let mut all = Vec::new();
    elements(&c.template, &mut all);
    let lines = |e: &Element| c.src[e.span.start as usize..e.span.end as usize].lines().count();
    // Repeated: ≥ 2 siblings of ≥ 3 lines with the same shape.
    for parent in all.iter().map(|e| &e.children).chain(std::iter::once(&c.template)) {
        let mut seen: HashMap<String, (u32, usize)> = HashMap::new();
        for ch in parent {
            if let Node::Element(e) = ch {
                if lines(e) < 3 {
                    continue;
                }
                let mut sh = String::new();
                crate::doctor::shape(std::slice::from_ref(ch), &mut sh);
                let ent = seen.entry(sh).or_insert((e.span.line, 0));
                ent.1 += 1;
            }
        }
        if let Some((line, count)) = seen.into_values().filter(|(_, k)| *k >= 2).min_by_key(|(l, _)| *l) {
            return Some(Suggestion::Repeated { line, count });
        }
    }
    let total: usize = c.template.iter().map(|n| c.src[n.span().start as usize..n.span().end as usize].len()).sum();
    let mut best: Option<(usize, Suggestion)> = None;
    for e in &all {
        let len = (e.span.end - e.span.start) as usize;
        let root = c.template.iter().any(|n| matches!(n, Node::Element(r) if std::ptr::eq(r, *e)));
        if root || total == 0 || len * 100 < total * 15 {
            continue;
        }
        let attr = |n: &str| e.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == n).and_then(|a| if let AttrValue::Text(t) = &a.value { Some(t.clone()) } else { None });
        let mut cands = Vec::new();
        if let Some(id) = attr("id") {
            cands.push((format!("#{id}"), id));
        }
        if let Some(cl) = attr("class").and_then(|t| t.split_whitespace().next().map(str::to_string)) {
            cands.push((format!("{}.{cl}", e.tag), cl));
        }
        cands.push((e.tag.clone(), e.tag.clone()));
        for (sel, base) in cands {
            let mut found = Vec::new();
            find(&c.template, &sel, &mut Vec::new(), &mut found);
            if found.len() == 1 {
                let name = pascal(&base);
                let name = if name.len() < 4 || name.eq_ignore_ascii_case(&e.tag) { format!("{}{}", c.name, pascal(&e.tag)) } else { name };
                if best.as_ref().is_none_or(|(l, _)| len > *l) {
                    best = Some((len, Suggestion::Split { selector: sel, name, line: e.span.line }));
                }
                break;
            }
        }
    }
    best.map(|b| b.1)
}

/// Removes the common indentation of every line but the first (which starts at the element).
fn dedent(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let min = lines.iter().skip(1).filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let last_indent = lines.last().map_or(0, |l| l.len() - l.trim_start().len());
    let cut = min.min(last_indent);
    lines.iter().enumerate().map(|(i, l)| if i == 0 || l.len() < cut { l.trim_start().to_string() } else { l[cut..].to_string() }).collect::<Vec<_>>().join("\n")
}
