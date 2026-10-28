//! INTERNALS §18, overengineering across files (`brain doctor`; level info, never blocks):
//! - B1002 a `store` channel used by a single component (→ a local `sig`).
//! - B1003 a context with a single direct consumer (→ a prop).
//! - B1006 a static component (no props, no script) used once and shorter than 10 lines (→ inline it).
//! - B1007 two components that differ only in classes and text (→ one component with a prop).
//! - B1008 a prop forwarded untouched through more than 3 components (→ context).
//!
//! [`doctor`] scans the app under `root`; `file` in each diagnostic is relative to `root`.

use brain_parse::ast::*;
use brain_parse::{Diag, Span};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component as P, Path, PathBuf};

struct File {
    rel: String,
    c: Component,
    /// Local default-import name → resolved rel path.
    imports: HashMap<String, String>,
}

fn normalize(p: &Path) -> String {
    let mut out = PathBuf::new();
    for part in p.components() {
        match part {
            P::CurDir => {}
            P::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().replace('\\', "/")
}

fn load(root: &Path) -> Vec<File> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "dist" | "target" | "invalid") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if !(name.ends_with(".brain") || name.ends_with(".jsbrain")) || name.ends_with(".test.brain") {
                continue;
            }
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            let Ok(src) = std::fs::read_to_string(&p) else { continue };
            let Ok(c) = brain_parse::parse(&src, Path::new(&rel)) else { continue };
            let dir = Path::new(&rel).parent().unwrap_or(Path::new("")).to_path_buf();
            let mut imports = HashMap::new();
            for s in &c.script {
                if let Stmt::Import(i) = s {
                    let target = normalize(&dir.join(&i.source));
                    for id in i.default.iter().chain(i.named.iter().map(|(_, l)| l)) {
                        imports.insert(id.name.clone(), target.clone());
                    }
                }
            }
            out.push(File { rel, c, imports });
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// Every `.brain`/`.jsbrain` of the project that parses, as (rel path, component), sorted by path.
pub(crate) fn load_files(root: &Path) -> Vec<(String, Component)> {
    load(root).into_iter().map(|f| (f.rel, f.c)).collect()
}

fn refs<'a>(nodes: &'a [Node], out: &mut Vec<&'a ComponentRef>) {
    for n in nodes {
        match n {
            Node::ComponentRef(r) => {
                out.push(r);
                refs(&r.children, out);
            }
            Node::Element(e) => refs(&e.children, out),
            Node::If(b) => {
                for br in &b.branches {
                    refs(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    refs(o, out);
                }
            }
            Node::Each(b) => refs(&b.body, out),
            Node::SlotScope(b) => refs(&b.body, out),
            Node::Await(a) => {
                refs(&a.pending, out);
                for br in [&a.then, &a.catch].into_iter().flatten() {
                    refs(&br.body, out);
                }
            }
            _ => {}
        }
    }
}

fn info(code: &'static str, file: &str, span: Span, msg: String, fix: String) -> Diag {
    Diag::new(code, file, span, msg, fix).info()
}

/// Calls `name(…)` anywhere in the script, with their span.
fn calls<'a>(stmts: &'a [Stmt], name: &str, out: &mut Vec<(&'a Expr, Span)>) {
    let mut all = Vec::new();
    collect(stmts, &mut all);
    for e in all {
        if let Expr::Call { callee, span, .. } = e
            && matches!(callee.unparen(), Expr::Ident(i) if i.name == name)
        {
            out.push((e, *span));
        }
    }
}

fn collect<'a>(stmts: &'a [Stmt], out: &mut Vec<&'a Expr>) {
    fn ex<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
        out.push(e);
        match e {
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                ex(callee, out);
                args.iter().for_each(|a| ex(a, out));
            }
            Expr::Func(f) => body(&f.body, out),
            Expr::Binary { left, right, .. } | Expr::Assign { target: left, value: right, .. } => {
                ex(left, out);
                ex(right, out);
            }
            Expr::Cond { test, cons, alt, .. } => {
                ex(test, out);
                ex(cons, out);
                ex(alt, out);
            }
            Expr::Unary { arg, .. } | Expr::Update { arg, .. } | Expr::Spread { arg, .. } | Expr::Await { arg, .. } | Expr::Paren { expr: arg, .. } => ex(arg, out),
            Expr::Member { obj, .. } => ex(obj, out),
            Expr::Array { elems, .. } => elems.iter().flatten().for_each(|x| ex(x, out)),
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjProp::KeyValue { value, .. } => ex(value, out),
                        ObjProp::Spread { arg, .. } => ex(arg, out),
                        ObjProp::Method { func, .. } => body(&func.body, out),
                        ObjProp::Shorthand(_) => {}
                    }
                }
            }
            Expr::Template { exprs, .. } | Expr::Seq { exprs, .. } => exprs.iter().for_each(|x| ex(x, out)),
            _ => {}
        }
    }
    fn body<'a>(b: &'a FnBody, out: &mut Vec<&'a Expr>) {
        match b {
            FnBody::Expr(e) => ex(e, out),
            FnBody::Block(s) => st(s, out),
        }
    }
    fn st<'a>(stmts: &'a [Stmt], out: &mut Vec<&'a Expr>) {
        for s in stmts {
            match s {
                Stmt::Expr { expr, .. } | Stmt::Throw { arg: expr, .. } | Stmt::ExportDefault { expr, .. } => ex(expr, out),
                Stmt::Return { arg: Some(e), .. } => ex(e, out),
                Stmt::Var(v) => v.decls.iter().filter_map(|d| d.init.as_ref()).for_each(|e| ex(e, out)),
                Stmt::If { test, cons, alt, .. } => {
                    ex(test, out);
                    st(std::slice::from_ref(cons.as_ref()), out);
                    if let Some(a) = alt {
                        st(std::slice::from_ref(a.as_ref()), out);
                    }
                }
                Stmt::ForOf { iter, body: b, .. } => {
                    ex(iter, out);
                    st(std::slice::from_ref(b.as_ref()), out);
                }
                Stmt::Block { body: b, .. } => st(b, out),
                Stmt::Func(f) => body(&f.body, out),
                Stmt::Export { decl, .. } => st(std::slice::from_ref(decl.as_ref()), out),
                Stmt::Server { func, .. } => body(&func.body, out),
                Stmt::Try { block, handler, finalizer, .. } => {
                    st(block, out);
                    if let Some(h) = handler {
                        st(h, out);
                    }
                    if let Some(f) = finalizer {
                        st(f, out);
                    }
                }
                _ => {}
            }
        }
    }
    st(stmts, out);
}

/// Structure of a template with class names, static text and literal attribute values removed.
pub(crate) fn shape(nodes: &[Node], out: &mut String) {
    for n in nodes {
        match n {
            Node::Element(e) => {
                out.push('<');
                out.push_str(&e.tag);
                let mut names: Vec<String> = e.attrs.iter().map(|a| format!("{:?}:{}", a.kind, a.name)).collect();
                names.sort();
                out.push_str(&names.join(","));
                out.push('>');
                shape(&e.children, out);
                out.push_str("</>");
            }
            Node::Interp(i) => out.push_str(&format!("{{{}}}", brain_parse::dump::expr(&i.expr))),
            Node::ComponentRef(r) => {
                out.push_str(&format!("<{}>", r.name));
                shape(&r.children, out);
            }
            Node::Text(t) if t.value.trim().is_empty() => {}
            Node::Text(_) => out.push('T'),
            Node::Slot(s) => out.push_str(&format!("<slot {:?}>", s.name)),
            _ => out.push('?'),
        }
    }
}

fn class_tokens(nodes: &[Node], out: &mut Vec<String>) {
    for n in nodes {
        if let Node::Element(e) = n {
            for a in &e.attrs {
                if a.kind == AttrKind::Plain
                    && a.name == "class"
                    && let AttrValue::Text(t) = &a.value
                {
                    out.extend(t.split_whitespace().map(str::to_string));
                }
            }
            class_tokens(&e.children, out);
        }
    }
}

fn common_suffix(a: &str, b: &str) -> String {
    let split = |s: &str| -> Vec<String> {
        let mut words = Vec::new();
        for ch in s.chars() {
            if ch.is_uppercase() || words.is_empty() {
                words.push(String::new());
            }
            if let Some(w) = words.last_mut() {
                w.push(ch);
            }
        }
        words
    };
    let (wa, wb) = (split(a), split(b));
    let mut common = Vec::new();
    for (x, y) in wa.iter().rev().zip(wb.iter().rev()) {
        if x != y {
            break;
        }
        common.push(x.clone());
    }
    common.reverse();
    common.concat()
}

/// Overengineering findings for the app under `root` (B1002, B1003, B1006, B1007, B1008).
pub fn doctor(root: &Path) -> Vec<Diag> {
    let files = load(root);
    let by_rel: HashMap<&str, &File> = files.iter().map(|f| (f.rel.as_str(), f)).collect();
    let comps: Vec<&File> = files.iter().filter(|f| f.c.kind != FileKind::Module).collect();
    let mut out = Vec::new();

    // Who imports each file (components only).
    let mut importers: HashMap<&str, Vec<&File>> = HashMap::new();
    for f in &comps {
        let mut seen = HashSet::new();
        for t in f.imports.values() {
            if seen.insert(t.as_str()) {
                importers.entry(t.as_str()).or_default().push(f);
            }
        }
    }

    // B1002
    for f in files.iter().filter(|f| f.c.kind == FileKind::Module && f.rel.contains("channels/")) {
        let mut st = Vec::new();
        calls(&f.c.script, "store", &mut st);
        let Some((call, span)) = st.first() else { continue };
        let users = importers.get(f.rel.as_str()).map(Vec::as_slice).unwrap_or(&[]);
        if users.len() != 1 {
            continue;
        }
        let user = &users[0].rel;
        let first = match call {
            Expr::Call { args, .. } => match args.first().map(Expr::unparen) {
                Some(Expr::Object { props, .. }) => props.iter().find_map(|p| match p {
                    ObjProp::KeyValue { key: PropKey::Ident(k), value, .. } => Some(format!("let {} = sig({})", k.name, brain_parse::dump::expr(value))),
                    _ => None,
                }),
                _ => None,
            },
            _ => None,
        };
        let file = Path::new(user).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        out.push(info(
            "B1002",
            &f.rel,
            *span,
            format!("`store` used by a single component (`{user}`)"),
            format!("use `{}` inside `{file}` and delete the channel", first.unwrap_or_else(|| "let x = sig(…)".into())),
        ));
    }

    // B1003: provide(K, v) whose K is `use`d by one component, rendered directly by the provider.
    let mut consumers: HashMap<String, Vec<&File>> = HashMap::new();
    for f in &comps {
        let mut u = Vec::new();
        calls(&f.c.script, "use", &mut u);
        for (call, _) in u {
            if let Expr::Call { args, .. } = call
                && let Some(Expr::Ident(k)) = args.first().map(Expr::unparen)
            {
                consumers.entry(f.imports.get(&k.name).cloned().unwrap_or_default() + "#" + &k.name).or_default().push(f);
            }
        }
    }
    for f in &comps {
        let mut p = Vec::new();
        calls(&f.c.script, "provide", &mut p);
        for (call, span) in p {
            let Expr::Call { args, .. } = call else { continue };
            let (Some(Expr::Ident(k)), Some(v)) = (args.first().map(Expr::unparen), args.get(1)) else { continue };
            let key = f.imports.get(&k.name).cloned().unwrap_or_default() + "#" + &k.name;
            let Some(cs) = consumers.get(&key) else { continue };
            if cs.len() != 1 {
                continue;
            }
            let consumer = cs[0];
            let mut rs = Vec::new();
            refs(&f.c.template, &mut rs);
            if rs.iter().any(|r| f.imports.get(&r.name).is_some_and(|t| *t == consumer.rel)) {
                let prop = brain_parse::dump::expr(v);
                out.push(info(
                    "B1003",
                    &f.rel,
                    span,
                    format!("context `{}` has a single direct consumer (`{}`)", k.name, consumer.c.name),
                    format!("pass it as a prop: `<{} {prop}={{{prop}}} />`", consumer.c.name),
                ));
            }
        }
    }

    // Uses of each component across the app.
    let mut uses: HashMap<String, Vec<(&File, &ComponentRef)>> = HashMap::new();
    for f in &comps {
        let mut rs = Vec::new();
        refs(&f.c.template, &mut rs);
        for r in rs {
            if let Some(t) = f.imports.get(&r.name) {
                uses.entry(t.clone()).or_default().push((f, r));
            }
        }
    }

    // B1006
    for f in &comps {
        if f.c.level == Level::View || !f.c.props.is_empty() || !f.c.script.is_empty() || f.c.src.lines().count() >= 10 {
            continue;
        }
        if let Some(u) = uses.get(&f.rel)
            && u.len() == 1
        {
            let (user, r) = u[0];
            out.push(info("B1006", &user.rel, r.span, format!("`{}` is used once and has fewer than 10 lines", f.c.name), format!("inline its markup and delete `{}`", f.rel)));
        }
    }

    // B1007: same props and the same template shape, differing only in classes/text.
    let mut groups: BTreeMap<String, Vec<&File>> = BTreeMap::new();
    for f in comps.iter().filter(|f| f.c.level != Level::View && !f.c.template.is_empty()) {
        let mut s = String::new();
        shape(&f.c.template, &mut s);
        let props: Vec<String> = f.c.props.iter().map(|p| format!("{}:{:?}", p.name, p.ty)).collect();
        groups.entry(format!("{}|{s}", props.join(","))).or_default().push(f);
    }
    for g in groups.values().filter(|g| g.len() > 1) {
        let (a, b) = (g[0], g[1]);
        let (mut ca, mut cb) = (Vec::new(), Vec::new());
        class_tokens(&a.c.template, &mut ca);
        class_tokens(&b.c.template, &mut cb);
        let only_b: Vec<&String> = cb.iter().filter(|x| !ca.contains(x)).collect();
        let common = common_suffix(&a.c.name, &b.c.name);
        let name = if common.is_empty() { a.c.name.clone() } else { common };
        let prop = only_b.first().map(|t| format!("`tone:str='{t}'` prop")).unwrap_or_else(|| "prop for what differs".into());
        let span = a.c.template.iter().find(|n| !matches!(n, Node::Text(t) if t.value.trim().is_empty())).map(Node::span).unwrap_or_default();
        out.push(info("B1007", &a.rel, span, format!("`{}` is nearly identical to `{}`", a.c.name, b.c.name), format!("one `{name}` with a {prop}")));
    }

    // B1008: a prop forwarded untouched (`<Child p={p} />`, nothing else reads it).
    let forward = |f: &File, prop: &str| -> Option<String> {
        let mut u = brain_parse::visit::Uses::default();
        f.c.script.iter().for_each(|s| u.stmt(s));
        if u.reads.contains(prop) {
            return None;
        }
        let mut rs = Vec::new();
        refs(&f.c.template, &mut rs);
        let mut target = None;
        let mut other_reads = 0;
        let mut all = brain_parse::visit::Uses::default();
        all.nodes(&f.c.template);
        for r in &rs {
            for a in &r.attrs {
                if let AttrValue::Expr(Expr::Ident(i)) = &a.value
                    && i.name == prop
                {
                    target = f.imports.get(&r.name).map(|t| format!("{t}#{}", a.name));
                    other_reads += 1;
                }
            }
        }
        // Read exactly once in markup, and that read is the forwarding attribute.
        (all.reads.contains(prop) && other_reads == 1 && count_reads(&f.c.template, prop) == 1).then_some(target).flatten()
    };
    let mut reported = HashSet::new();
    for f in &comps {
        let mut rs = Vec::new();
        refs(&f.c.template, &mut rs);
        for r in rs {
            let Some(target) = f.imports.get(&r.name) else { continue };
            for a in &r.attrs {
                if a.kind != AttrKind::Plain {
                    continue;
                }
                // The caller must use the value itself (not forward it), so the chain starts here.
                if let AttrValue::Expr(Expr::Ident(i)) = &a.value
                    && f.c.props.iter().any(|p| p.name == i.name)
                {
                    continue;
                }
                let mut chain = vec![];
                let mut cur = format!("{target}#{}", a.name);
                let mut guard = 0;
                while let Some((file, prop)) = cur.split_once('#').map(|(x, y)| (x.to_string(), y.to_string())) {
                    let Some(cf) = by_rel.get(file.as_str()) else { break };
                    chain.push(cf.c.name.clone());
                    guard += 1;
                    match forward(cf, &prop) {
                        Some(next) if guard < 32 => cur = next,
                        _ => break,
                    }
                }
                if chain.len() > 3 && reported.insert((f.rel.clone(), r.span.start)) {
                    out.push(info(
                        "B1008",
                        &f.rel,
                        r.span,
                        format!("`{}` passes through {} levels of props without being used ({})", a.name, chain.len(), chain.join(" → ")),
                        format!("`provide({}, {})` in the view and `use({0})` in `{}`", pascal(&a.name), a.name, chain.last().cloned().unwrap_or_default()),
                    ));
                }
            }
        }
    }
    for d in &mut out {
        if let Some(f) = by_rel.get(d.file.as_str()) {
            d.locate(&f.c.src);
        }
    }
    out.extend(layout_rules(root, &files));
    out.sort_by(|a, b| (a.file.as_str(), a.line, a.col).cmp(&(b.file.as_str(), b.line, b.col)));
    out
}

/// The folders `brain new` creates (SPEC); every source file lives in one of them or at the root.
pub const APP_FOLDERS: &[&str] = &["views", "comp", "ui", "channels", "services", "api", "lib", "server", "content", "public", "i18n"];

/// B0131: a top-level folder with source files outside the app layout. B0732: a `.env` file (only `.brain_env` is
/// read; a `.env` is silently ignored). Never prints a value of either file.
fn layout_rules(root: &Path, files: &[File]) -> Vec<Diag> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for f in files {
        let Some((top, _)) = f.rel.split_once('/') else { continue };
        if APP_FOLDERS.contains(&top) || !seen.insert(top.to_string()) {
            continue;
        }
        let into = if f.c.kind == FileKind::Module { "lib/ (logic), services/ or channels/" } else { "comp/ (components) or views/ (pages)" };
        let mut d = Diag::new("B0131", &f.rel, Span::default(), format!("`{top}/` is not a folder of the app layout"), format!("move its files into {into}; the layout is {}", APP_FOLDERS.join(", ")));
        d.line = 1;
        d.col = 1;
        out.push(d);
    }
    if root.join(".env").is_file() {
        let mut d = Diag::new("B0732", ".env", Span::default(), "a `.env` file is ignored: BRAINLLM reads `.brain_env`", "move its keys into `.brain_env` (shared block, then one block per environment) and delete `.env`");
        d.line = 1;
        d.col = 1;
        out.push(d);
    }
    out
}

fn pascal(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn count_reads(nodes: &[Node], name: &str) -> usize {
    let mut u = String::new();
    fn dump(nodes: &[Node], out: &mut String) {
        for n in nodes {
            match n {
                Node::Interp(i) => {
                    out.push_str(&brain_parse::dump::expr(&i.expr));
                    out.push(' ');
                }
                Node::Element(Element { attrs, children, .. }) | Node::ComponentRef(ComponentRef { attrs, children, .. }) => {
                    for a in attrs {
                        match &a.value {
                            AttrValue::Expr(e) => {
                                out.push_str(&brain_parse::dump::expr(e));
                                out.push(' ');
                            }
                            AttrValue::Mixed(parts) => {
                                for p in parts {
                                    if let AttrPart::Expr(e) = p {
                                        out.push_str(&brain_parse::dump::expr(e));
                                        out.push(' ');
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    dump(children, out);
                }
                Node::If(b) => {
                    for br in &b.branches {
                        out.push_str(&brain_parse::dump::expr(&br.test));
                        out.push(' ');
                        dump(&br.body, out);
                    }
                    if let Some(o) = &b.otherwise {
                        dump(o, out);
                    }
                }
                Node::Each(b) => {
                    out.push_str(&brain_parse::dump::expr(&b.list));
                    out.push(' ');
                    dump(&b.body, out);
                }
                Node::SlotScope(b) => dump(&b.body, out),
                Node::Slot(s) => {
                    for a in &s.attrs {
                        match &a.value {
                            AttrValue::Expr(e) => {
                                out.push_str(&brain_parse::dump::expr(e));
                                out.push(' ');
                            }
                            AttrValue::Mixed(parts) => {
                                for p in parts {
                                    if let AttrPart::Expr(e) = p {
                                        out.push_str(&brain_parse::dump::expr(e));
                                        out.push(' ');
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    dump(nodes, &mut u);
    u.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '.')).filter(|w| *w == name || w.starts_with(&format!("{name}."))).count()
}

/// Context keys guaranteed to be provided above `rel` on every render path (INTERNALS §12, B0807): app.brain and the
/// `_layout`s of a view's folders wrap it; a component inherits the intersection over every parent that renders it.
/// `None` when `rel` is not reachable from any view (nothing can be said).
pub fn ancestor_provided(root: &Path, rel: &str) -> Option<HashSet<String>> {
    let files = load(root);
    let provides = |f: &File| -> HashSet<String> {
        let mut out = HashSet::new();
        let mut cs = Vec::new();
        calls(&f.c.script, "provide", &mut cs);
        for (call, _) in cs {
            if let Expr::Call { args, .. } = call
                && let Some(Expr::Ident(k)) = args.first().map(Expr::unparen)
            {
                out.insert(k.name.clone());
            }
        }
        out
    };
    let by_rel: HashMap<&str, &File> = files.iter().map(|f| (f.rel.as_str(), f)).collect();
    let app: HashSet<String> = by_rel.get("app.brain").map(|f| provides(f)).unwrap_or_default();
    // Above each view: app.brain + every `_layout.brain` from views/ down to the view's folder.
    let above_view = |view: &str| -> HashSet<String> {
        let mut set = app.clone();
        let mut dir = Path::new(view).parent();
        while let Some(d) = dir {
            if let Some(l) = by_rel.get(d.join("_layout.brain").to_string_lossy().replace('\\', "/").as_str()) {
                set.extend(provides(l));
            }
            if d.as_os_str().is_empty() || d == Path::new("views") {
                break;
            }
            dir = d.parent();
        }
        set
    };
    // Fixpoint over the render graph: what is guaranteed *inside* each file (above it ∪ its own provides).
    let mut inside: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    for f in files.iter().filter(|f| f.c.kind != FileKind::Module) {
        let is_view = f.rel.starts_with("views/") && !Path::new(&f.rel).file_name().is_some_and(|n| n.to_string_lossy().starts_with('_'));
        let is_shell = f.rel == "app.brain" || f.rel.ends_with("_layout.brain");
        inside.insert(f.rel.clone(), if is_view || is_shell { Some(above_view(&f.rel).union(&provides(f)).cloned().collect()) } else { None });
    }
    let mut edges: Vec<(String, String)> = Vec::new();
    for f in files.iter().filter(|f| f.c.kind != FileKind::Module) {
        let mut rs = Vec::new();
        refs(&f.c.template, &mut rs);
        for r in rs {
            if let Some(t) = f.imports.get(&r.name) {
                edges.push((f.rel.clone(), t.clone()));
            }
        }
    }
    let mut above: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    for _ in 0..64 {
        let mut changed = false;
        for (parent, child) in &edges {
            let Some(Some(p)) = inside.get(parent).cloned() else { continue };
            let entry = above.entry(child.clone()).or_insert(None);
            let next = match entry.take() {
                None => p.clone(),
                Some(prev) => prev.intersection(&p).cloned().collect(),
            };
            *entry = Some(next.clone());
            let own = by_rel.get(child.as_str()).map(|f| provides(f)).unwrap_or_default();
            let new_inside: HashSet<String> = next.union(&own).cloned().collect();
            let slot = inside.entry(child.clone()).or_insert(None);
            if slot.as_ref() != Some(&new_inside) && !(child.starts_with("views/") || child == "app.brain") {
                *slot = Some(new_inside);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let is_root = rel == "app.brain" || rel.starts_with("views/");
    if is_root {
        return inside.get(rel).cloned().flatten();
    }
    above.get(rel).cloned().flatten()
}
