//! Conservative structural performance advice. No timing, purity proof, rewrite or application execution.
use crate::Ctx;
use brain_parse::{Diag, Severity, Span, ast::*};
use std::collections::{BTreeMap, BTreeSet};

fn exprs<'a>(
    e: &'a Expr,
    functions: bool,
    stmt: &mut dyn FnMut(&'a Stmt),
    expr: &mut dyn FnMut(&'a Expr),
) {
    expr(e);
    match e {
        Expr::Func(f) if functions => function(f, functions, stmt, expr),
        Expr::Template {
            tag, exprs: values, ..
        } => {
            if let Some(t) = tag {
                exprs(t, functions, stmt, expr);
            }
            for value in values {
                exprs(value, functions, stmt, expr);
            }
        }
        Expr::Array { elems, .. } => {
            for value in elems.iter().flatten() {
                exprs(value, functions, stmt, expr);
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    ObjProp::KeyValue { key, value, .. } => {
                        key_exprs(key, functions, stmt, expr);
                        exprs(value, functions, stmt, expr)
                    }
                    ObjProp::Spread { arg, .. } => exprs(arg, functions, stmt, expr),
                    ObjProp::Method { key, func, .. } => {
                        key_exprs(key, functions, stmt, expr);
                        if functions {
                            function(func, functions, stmt, expr);
                        }
                    }
                    // Shorthand owns an Ident, not a borrowed Expr. Identifier dependency reads below
                    // use the shared Uses visitor, which includes shorthand and computed/default reads.
                    ObjProp::Shorthand(_) => (),
                }
            }
        }
        Expr::Unary { arg, .. }
        | Expr::Update { arg, .. }
        | Expr::Spread { arg, .. }
        | Expr::Await { arg, .. }
        | Expr::Paren { expr: arg, .. } => exprs(arg, functions, stmt, expr),
        Expr::Binary { left, right, .. } => {
            exprs(left, functions, stmt, expr);
            exprs(right, functions, stmt, expr);
        }
        Expr::Assign { target, value, .. } => {
            exprs(target, functions, stmt, expr);
            exprs(value, functions, stmt, expr);
        }
        Expr::Cond {
            test, cons, alt, ..
        } => {
            exprs(test, functions, stmt, expr);
            exprs(cons, functions, stmt, expr);
            exprs(alt, functions, stmt, expr);
        }
        Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
            exprs(callee, functions, stmt, expr);
            for a in args {
                exprs(a, functions, stmt, expr);
            }
        }
        Expr::Member { obj, prop, .. } => {
            exprs(obj, functions, stmt, expr);
            if let MemberProp::Computed(p) = prop {
                exprs(p, functions, stmt, expr);
            }
        }
        Expr::Seq { exprs: values, .. } => {
            for value in values {
                exprs(value, functions, stmt, expr);
            }
        }
        _ => (),
    }
}
fn function<'a>(
    f: &'a Func,
    functions: bool,
    stmt: &mut dyn FnMut(&'a Stmt),
    expr: &mut dyn FnMut(&'a Expr),
) {
    for parameter in &f.params {
        pattern_exprs(parameter, functions, stmt, expr);
    }
    match &f.body {
        FnBody::Expr(e) => exprs(e, functions, stmt, expr),
        FnBody::Block(body) => {
            for s in body {
                statements(s, functions, stmt, expr)
            }
        }
    }
}
fn key_exprs<'a>(
    key: &'a PropKey,
    functions: bool,
    stmt: &mut dyn FnMut(&'a Stmt),
    expr: &mut dyn FnMut(&'a Expr),
) {
    if let PropKey::Computed(value) = key {
        exprs(value, functions, stmt, expr);
    }
}
fn pattern_exprs<'a>(
    pattern: &'a Pat,
    functions: bool,
    stmt: &mut dyn FnMut(&'a Stmt),
    expr: &mut dyn FnMut(&'a Expr),
) {
    match pattern {
        Pat::Default { pat, default, .. } => {
            pattern_exprs(pat, functions, stmt, expr);
            exprs(default, functions, stmt, expr);
        }
        Pat::Object { props, rest, .. } => {
            for property in props {
                key_exprs(&property.key, functions, stmt, expr);
                pattern_exprs(&property.value, functions, stmt, expr);
            }
            if let Some(rest) = rest {
                pattern_exprs(rest, functions, stmt, expr);
            }
        }
        Pat::Array { elems, rest, .. } => {
            for element in elems.iter().flatten() {
                pattern_exprs(element, functions, stmt, expr);
            }
            if let Some(rest) = rest {
                pattern_exprs(rest, functions, stmt, expr);
            }
        }
        Pat::Rest { arg, .. } => pattern_exprs(arg, functions, stmt, expr),
        Pat::Ident(_) => (),
    }
}
fn statements<'a>(
    s: &'a Stmt,
    functions: bool,
    stmt: &mut dyn FnMut(&'a Stmt),
    expr: &mut dyn FnMut(&'a Expr),
) {
    stmt(s);
    match s {
        Stmt::Var(v) => {
            for d in &v.decls {
                pattern_exprs(&d.pat, functions, stmt, expr);
                if let Some(e) = &d.init {
                    exprs(e, functions, stmt, expr);
                }
            }
        }
        Stmt::Expr { expr: e, .. }
        | Stmt::Throw { arg: e, .. }
        | Stmt::ExportDefault { expr: e, .. } => exprs(e, functions, stmt, expr),
        Stmt::Return { arg: Some(e), .. } => exprs(e, functions, stmt, expr),
        Stmt::If {
            test, cons, alt, ..
        } => {
            exprs(test, functions, stmt, expr);
            statements(cons, functions, stmt, expr);
            if let Some(a) = alt {
                statements(a, functions, stmt, expr);
            }
        }
        Stmt::ForOf {
            pat, iter, body, ..
        } => {
            pattern_exprs(pat, functions, stmt, expr);
            exprs(iter, functions, stmt, expr);
            statements(body, functions, stmt, expr);
        }
        Stmt::Block { body, .. } => {
            for s in body {
                statements(s, functions, stmt, expr);
            }
        }
        Stmt::Try {
            block,
            param,
            handler,
            finalizer,
            ..
        } => {
            if let Some(parameter) = param {
                pattern_exprs(parameter, functions, stmt, expr);
            }
            for s in block
                .iter()
                .chain(handler.iter().flatten())
                .chain(finalizer.iter().flatten())
            {
                statements(s, functions, stmt, expr);
            }
        }
        Stmt::Func(f) | Stmt::Server { func: f, .. } if functions => {
            function(f, functions, stmt, expr)
        }
        Stmt::Export { decl, .. } => statements(decl, functions, stmt, expr),
        _ => (),
    }
}
fn names(p: &Pat) -> BTreeSet<String> {
    let mut found = vec![];
    p.bound_names(&mut found);
    found.into_iter().map(|n| n.name.clone()).collect()
}
fn reads(e: &Expr) -> BTreeSet<String> {
    let mut uses = brain_parse::visit::Uses::default();
    uses.expr(e);
    uses.reads.into_iter().collect()
}
fn member(e: &Expr) -> Option<(&Expr, &str)> {
    if let Expr::Member {
        obj,
        prop: MemberProp::Name(p),
        optional: false,
        ..
    } = e.unparen()
    {
        Some((obj, &p.name))
    } else {
        None
    }
}
fn ident(e: &Expr) -> Option<&str> {
    if let Expr::Ident(i) = e.unparen() {
        Some(&i.name)
    } else {
        None
    }
}
fn root(e: &Expr) -> Option<&str> {
    match e.unparen() {
        Expr::Ident(i) => Some(&i.name),
        Expr::Member { obj, .. } => root(obj),
        _ => None,
    }
}
fn literal_array(e: &Expr) -> bool {
    matches!(e.unparen(), Expr::Array { .. })
}
fn dom_origin(e: &Expr) -> bool {
    let Expr::Call {
        callee,
        args,
        optional: false,
        ..
    } = e.unparen()
    else {
        return false;
    };
    member(callee).is_some_and(|(obj, name)| {
        ident(obj) == Some("document")
            && matches!(name, "querySelector" | "getElementById")
            && args.len() == 1
            && matches!(args[0].unparen(), Expr::Str { .. })
    })
}
fn template_refs(nodes: &[Node], out: &mut BTreeSet<String>) {
    for node in nodes {
        match node {
            Node::Element(e) => {
                for a in &e.attrs {
                    if a.kind == AttrKind::Bind && a.name == "ref" {
                        if let AttrValue::Expr(value) = &a.value {
                            if let Some(name) = ident(value) {
                                out.insert(name.into());
                            }
                        }
                    }
                }
                template_refs(&e.children, out);
            }
            Node::ComponentRef(e) => template_refs(&e.children, out),
            Node::If(b) => {
                for branch in &b.branches {
                    template_refs(&branch.body, out);
                }
                if let Some(body) = &b.otherwise {
                    template_refs(body, out);
                }
            }
            Node::Each(b) => {
                template_refs(&b.body, out);
                if let Some(body) = &b.empty {
                    template_refs(body, out);
                }
            }
            Node::Await(b) => {
                template_refs(&b.pending, out);
                for branch in [&b.then, &b.catch].into_iter().flatten() {
                    template_refs(&branch.body, out);
                }
            }
            Node::SlotScope(s) => template_refs(&s.body, out),
            _ => (),
        }
    }
}
/// References retained by a value, including arbitrary nesting of containers and branch/sequence results.
/// Calls expose their arguments; their receiver is not itself a returned alias. Known layout/length reads are
/// scalar values, so recording their result must not invalidate the DOM/array positive controls.
fn value_roots(value: &Expr, out: &mut BTreeSet<String>) {
    match value.unparen() {
        Expr::Ident(name) => {
            out.insert(name.name.clone());
        }
        Expr::Array { elems, .. } => {
            for value in elems.iter().flatten() {
                value_roots(value, out);
            }
        }
        Expr::Object { props, .. } => {
            for property in props {
                match property {
                    ObjProp::Shorthand(name) => {
                        out.insert(name.name.clone());
                    }
                    ObjProp::KeyValue { key, value, .. } => {
                        if let PropKey::Computed(key) = key {
                            value_roots(key, out);
                        }
                        value_roots(value, out);
                    }
                    ObjProp::Spread { arg, .. } => value_roots(arg, out),
                    ObjProp::Method { func, .. } => {
                        if let FnBody::Expr(value) = &func.body {
                            value_roots(value, out);
                        }
                    }
                }
            }
        }
        Expr::Cond { cons, alt, .. } => {
            value_roots(cons, out);
            value_roots(alt, out);
        }
        Expr::Binary {
            op: "&&" | "||" | "??",
            left,
            right,
            ..
        } => {
            value_roots(left, out);
            value_roots(right, out);
        }
        Expr::Seq { exprs, .. } => {
            if let Some(value) = exprs.last() {
                value_roots(value, out);
            }
        }
        Expr::Assign { value, .. }
        | Expr::Await { arg: value, .. }
        | Expr::Spread { arg: value, .. } => value_roots(value, out),
        Expr::Member { obj, prop, .. } => {
            if !matches!(prop,MemberProp::Name(name) if matches!(name.name.as_str(),"length"|"offsetWidth"|"offsetHeight"|"clientWidth"|"clientHeight"|"scrollWidth"|"scrollHeight"))
            {
                value_roots(obj, out);
                if let MemberProp::Computed(key) = prop {
                    value_roots(key, out);
                }
            }
        }
        Expr::Call { args, .. } | Expr::New { args, .. } => {
            for value in args {
                value_roots(value, out);
            }
        }
        Expr::Func(func) => {
            if let FnBody::Expr(value) = &func.body {
                value_roots(value, out);
            }
        }
        Expr::Template {
            tag: Some(_),
            exprs,
            ..
        } => {
            for value in exprs {
                value_roots(value, out);
            }
        }
        _ => (),
    }
}
#[derive(Default)]
struct Facts {
    bindings: BTreeMap<String, usize>,
    arrays: BTreeSet<String>,
    dom: BTreeSet<String>,
    reassigned: BTreeSet<String>,
    mutated: BTreeSet<String>,
    escaped: BTreeSet<String>,
}
impl Facts {
    fn escape(&mut self, value: &Expr) {
        let mut roots = BTreeSet::new();
        value_roots(value, &mut roots);
        self.mutated.extend(roots.iter().cloned());
        self.escaped.extend(roots);
    }
    fn pattern_escapes(&mut self, p: &Pat) {
        match p {
            Pat::Default { pat, default, .. } => {
                self.escape(default);
                self.pattern_escapes(pat);
            }
            Pat::Object { props, rest, .. } => {
                for property in props {
                    if let PropKey::Computed(key) = &property.key {
                        self.escape(key);
                    }
                    self.pattern_escapes(&property.value);
                }
                if let Some(rest) = rest {
                    self.pattern_escapes(rest);
                }
            }
            Pat::Array { elems, rest, .. } => {
                for element in elems.iter().flatten() {
                    self.pattern_escapes(element);
                }
                if let Some(rest) = rest {
                    self.pattern_escapes(rest);
                }
            }
            Pat::Rest { arg, .. } => self.pattern_escapes(arg),
            Pat::Ident(_) => (),
        }
    }
    fn write(&mut self, target: &Expr) {
        if let Some(name) = root(target) {
            self.mutated.insert(name.into());
            let known_dom_write = member(target).is_some_and(|(obj, property)| {
                matches!(property, "className" | "textContent" | "innerHTML")
                    || member(obj).is_some_and(|(_, property)| property == "style")
            });
            if ident(target).is_none() && !known_dom_write {
                self.escaped.insert(name.into());
            }
            if ident(target).is_some()
                || matches!(name, "document" | "ref" | "window" | "globalThis" | "self")
            {
                self.reassigned.insert(name.into());
            }
        }
    }
    fn bind(&mut self, p: &Pat) {
        self.pattern_escapes(p);
        for n in names(p) {
            *self.bindings.entry(n).or_default() += 1;
        }
    }
    fn func(&mut self, f: &Func) {
        for p in &f.params {
            self.bind(p);
        }
        if let Some(n) = &f.name {
            *self.bindings.entry(n.name.clone()).or_default() += 1;
        }
        if let FnBody::Expr(value) = &f.body {
            self.escape(value);
        }
    }
    fn stable(&self, name: &str) -> bool {
        self.bindings.get(name) == Some(&1) && !self.reassigned.contains(name)
    }
}
fn facts(c: &Component) -> Facts {
    let mut out = Facts::default();
    let mut native_refs = BTreeSet::new();
    template_refs(&c.template, &mut native_refs);
    let mut ref_candidates = BTreeSet::new();
    // Separate passes keep callback borrows simple and make every shadowed binding invalidate the candidate.
    for s in &c.script {
        statements(
            s,
            true,
            &mut |s| match s {
                Stmt::Var(v) => {
                    for d in &v.decls {
                        out.bind(&d.pat);
                        if let Some(value) = &d.init {
                            out.escape(value);
                        }
                        if let (
                            Pat::Ident(name),
                            Some(Expr::Call {
                                callee,
                                args,
                                optional: false,
                                ..
                            }),
                        ) = (&d.pat, d.init.as_ref().map(Expr::unparen))
                        {
                            if ident(callee) == Some("ref")
                                && args.is_empty()
                                && native_refs.contains(&name.name)
                            {
                                ref_candidates.insert(name.name.clone());
                            }
                        }
                        if v.kind == VarKind::Const {
                            if let (Pat::Ident(i), Some(e)) = (&d.pat, &d.init) {
                                if literal_array(e) {
                                    out.arrays.insert(i.name.clone());
                                }
                                if dom_origin(e) {
                                    out.dom.insert(i.name.clone());
                                }
                            }
                        }
                    }
                }
                Stmt::ForOf { pat, iter, .. } => {
                    out.bind(pat);
                    if matches!(
                        iter.unparen(),
                        Expr::Array { .. }
                            | Expr::Object { .. }
                            | Expr::Cond { .. }
                            | Expr::Seq { .. }
                    ) {
                        out.escape(iter);
                    }
                }
                Stmt::Func(f) | Stmt::Server { func: f, .. } => out.func(f),
                Stmt::Try { param: Some(p), .. } => out.bind(p),
                Stmt::Return {
                    arg: Some(value), ..
                }
                | Stmt::Throw { arg: value, .. }
                | Stmt::ExportDefault { expr: value, .. } => out.escape(value),
                _ => (),
            },
            &mut |_| {},
        );
    }
    for s in &c.script {
        statements(s, true, &mut |_| {}, &mut |e| match e {
            Expr::Func(f) => out.func(f),
            Expr::Object { props, .. } => {
                for property in props {
                    if let ObjProp::Method { func, .. } = property {
                        out.func(func);
                    }
                }
            }
            Expr::Assign { target, value, .. } => {
                out.write(target);
                out.escape(value);
            }
            Expr::Update { arg, .. }
            | Expr::Unary {
                op: "delete", arg, ..
            } => out.write(arg),
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                if let Some((obj, name)) = member(callee) {
                    if !matches!(
                        name,
                        "filter"
                            | "map"
                            | "reduce"
                            | "find"
                            | "some"
                            | "every"
                            | "includes"
                            | "getBoundingClientRect"
                            | "get"
                    ) {
                        if let Some(n) = root(obj) {
                            out.mutated.insert(n.into());
                        }
                    }
                }
                for arg in args {
                    out.escape(arg);
                }
            }
            Expr::Template {
                tag: Some(_),
                exprs,
                ..
            } => {
                for value in exprs {
                    out.escape(value);
                }
            }
            _ => (),
        });
    }
    let imported_document = c.script.iter().any(|s| match s {
        Stmt::Import(i) => i
            .default
            .iter()
            .chain(i.namespace.iter())
            .chain(i.named.iter().map(|(_, local)| local))
            .any(|name| name.name == "document"),
        _ => false,
    });
    if imported_document
        || out.bindings.contains_key("document")
        || out.reassigned.contains("document")
        || out.escaped.contains("document")
        || ["window", "globalThis", "self"]
            .iter()
            .any(|name| out.escaped.contains(*name) || out.reassigned.contains(*name))
    {
        out.dom.clear();
    }
    let imported_ref = c.script.iter().any(|s| match s {
        Stmt::Import(i) => i
            .default
            .iter()
            .chain(i.namespace.iter())
            .chain(i.named.iter().map(|(_, local)| local))
            .any(|name| name.name == "ref"),
        _ => false,
    });
    if !imported_ref
        && !out.bindings.contains_key("ref")
        && !out.reassigned.contains("ref")
        && !out.escaped.contains("ref")
    {
        out.dom.extend(ref_candidates);
    }
    let top_arrays = c
        .script
        .iter()
        .filter_map(|s| if let Stmt::Var(v) = s { Some(v) } else { None })
        .filter(|v| v.kind == VarKind::Const)
        .flat_map(|v| v.decls.iter())
        .filter_map(|d| {
            if let (Pat::Ident(name), Some(value)) = (&d.pat, &d.init) {
                literal_array(value).then_some(name.name.clone())
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>();
    out.arrays.retain(|name| top_arrays.contains(name));
    out
}
fn warn(out: &mut Vec<Diag>, c: &Component, code: &'static str, span: Span, msg: &str, fix: &str) {
    if out.len() >= 64
        || out
            .iter()
            .any(|d| d.code == code && d.line == span.line && d.col == span.col)
    {
        return;
    }
    let mut d = Diag::new(code, &c.file, span, msg, fix);
    d.level = Severity::Warning;
    out.push(d);
}
fn direct_exprs<'a>(s: &'a Stmt) -> Vec<&'a Expr> {
    let mut out = vec![];
    statements(s, false, &mut |_| {}, &mut |e| out.push(e));
    out
}
fn body_statements(s: &Stmt) -> &[Stmt] {
    match s {
        Stmt::Block { body, .. } => body,
        _ => std::slice::from_ref(s),
    }
}
fn writes(s: &Stmt) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for e in direct_exprs(s) {
        if let Expr::Assign { target, .. } | Expr::Update { arg: target, .. } = e {
            if let Some(n) = root(target) {
                out.insert(n.into());
            }
        }
    }
    out
}
fn declared_apis(c: &Component, ctx: &Ctx, facts: &Facts) -> BTreeSet<String> {
    c.script
        .iter()
        .filter_map(|s| {
            if let Stmt::Import(i) = s {
                i.default
                    .as_ref()
                    .filter(|n| {
                        !facts.bindings.contains_key(&n.name)
                            && !facts.reassigned.contains(&n.name)
                            && !facts.mutated.contains(&n.name)
                            && ctx
                                .api_contracts
                                .get(&i.source)
                                .is_some_and(|contract| contract.methods.contains_key("get"))
                    })
                    .map(|n| n.name.clone())
            } else {
                None
            }
        })
        .collect()
}
fn dom_receiver<'a>(e: &'a Expr, facts: &Facts) -> Option<&'a str> {
    ident(e).filter(|n| facts.dom.contains(*n) && facts.stable(n) && !facts.escaped.contains(*n))
}
fn layout_read(e: &Expr, facts: &Facts) -> Option<String> {
    if let Some((obj, name)) = member(e) {
        if matches!(
            name,
            "offsetWidth"
                | "offsetHeight"
                | "clientWidth"
                | "clientHeight"
                | "scrollWidth"
                | "scrollHeight"
        ) {
            return dom_receiver(obj, facts).map(String::from);
        }
    }
    if let Expr::Call {
        callee,
        args,
        optional: false,
        ..
    } = e
    {
        if args.is_empty() {
            if let Some((obj, "getBoundingClientRect")) = member(callee) {
                return dom_receiver(obj, facts).map(String::from);
            }
        }
    }
    None
}
fn layout_write(e: &Expr, facts: &Facts) -> Option<String> {
    if let Expr::Assign { target, .. } = e {
        if let Some((obj, name)) = member(target) {
            if matches!(name, "className" | "textContent" | "innerHTML") {
                return dom_receiver(obj, facts).map(String::from);
            }
            if let Some((node, "style")) = member(obj) {
                return dom_receiver(node, facts).map(String::from);
            }
        }
    }
    None
}
fn layout_block(c: &Component, body: &Stmt, facts: &Facts, out: &mut Vec<Diag>) {
    let mut dirty = BTreeSet::new();
    for s in body_statements(body) {
        let expressions = direct_exprs(s);
        if matches!(
            s,
            Stmt::If { .. } | Stmt::ForOf { .. } | Stmt::Try { .. } | Stmt::Block { .. }
        ) || matches!(s,Stmt::Var(declaration) if declaration.decls.iter().any(|d| !matches!(d.pat,Pat::Ident(_))))
            || expressions.iter().any(|e| {
                matches!(
                    e,
                    Expr::Await { .. }
                        | Expr::New { .. }
                        | Expr::Cond { .. }
                        | Expr::Binary {
                            op: "&&" | "||" | "??",
                            ..
                        }
                ) || (matches!(e, Expr::Call { .. }) && layout_read(e, facts).is_none())
            })
        {
            // A write inside an argument or sequence must not survive a later call/await barrier.
            // Without evaluation-order proof this entire statement contributes no layout facts.
            dirty.clear();
            continue;
        }
        for e in &expressions {
            if let Some(name) = layout_read(e, facts) {
                if dirty.contains(&name) {
                    warn(
                        out,
                        c,
                        "B1313",
                        e.span(),
                        "A layout read follows a DOM write in this loop; this may force synchronous layout.",
                        "If behavior permits, collect layout reads before applying DOM writes. Measure a browser trace before claiming a speedup; no reordering is applied.",
                    );
                    return;
                }
            }
        }
        for e in expressions {
            if let Some(name) = layout_write(e, facts) {
                dirty.insert(name);
            }
        }
    }
}

/// Structural cases only. Unknown receiver types, aliases, shadowing and mutations remain unproven.
pub fn check(c: &Component, ctx: &Ctx) -> Vec<Diag> {
    let facts = facts(c);
    let apis = declared_apis(c, ctx, &facts);
    let mut out = vec![];
    let mut loops = vec![];
    for s in &c.script {
        statements(
            s,
            true,
            &mut |s| {
                if matches!(s, Stmt::ForOf { .. }) {
                    loops.push(s)
                }
            },
            &mut |_| {},
        );
    }
    for s in loops {
        let Stmt::ForOf {
            pat, iter, body, ..
        } = s
        else {
            continue;
        };
        let bound = names(pat);
        let changed = writes(body);
        let expressions = direct_exprs(body);
        let mut per_item = false;
        for e in &expressions {
            if let Expr::Call {
                callee,
                args,
                optional: false,
                span,
            } = e
            {
                if member(callee).is_some_and(|(obj, name)| {
                    name == "get" && ident(obj).is_some_and(|n| apis.contains(n))
                }) && args.iter().any(|a| !reads(a).is_disjoint(&bound))
                {
                    per_item = true;
                    warn(
                        &mut out,
                        c,
                        "B1312",
                        *span,
                        "An item-dependent declared API GET appears inside this loop.",
                        "Review a batched endpoint or a bounded read plan if authorization and output remain equivalent. This pattern does not prove database query count or cache misses.",
                    );
                    break;
                }
            }
        }
        if !per_item {
            for e in &expressions {
                if let Expr::Await { arg, span } = e {
                    if !reads(arg).is_disjoint(&changed) {
                        continue;
                    }
                    if matches!(arg.unparen(), Expr::Call { .. }) {
                        warn(
                            &mut out,
                            c,
                            "B1301",
                            *span,
                            "This for-of awaits a call before advancing to its next iteration.",
                            "If iterations are independent and order is irrelevant, consider batching or bounded concurrency. Keep serial execution when calls share state, have ordering requirements or depend on prior results.",
                        );
                        break;
                    }
                }
            }
        }
        if let Some(collection) = ident(iter).filter(|name| {
            facts.stable(name) && !facts.mutated.contains(*name) && !bound.contains(*name)
        }) {
            let mut nested = vec![];
            let mut has_exit = false;
            statements(
                body,
                false,
                &mut |s| match s {
                    Stmt::ForOf {
                        pat, iter, span, ..
                    } if ident(iter) == Some(collection) && !names(pat).contains(collection) => {
                        nested.push(*span)
                    }
                    Stmt::Break { .. } | Stmt::Return { .. } | Stmt::Throw { .. } => {
                        has_exit = true
                    }
                    _ => (),
                },
                &mut |_| {},
            );
            if !has_exit {
                if let Some(span) = nested.first() {
                    warn(
                        &mut out,
                        c,
                        "B1303",
                        *span,
                        "A nested for-of traverses the same stable collection as its enclosing loop.",
                        "Review an index or Map when repeated lookups can preserve ordering, duplicate handling and side effects. This syntax alone does not prove runtime complexity or permit an automatic rewrite.",
                    );
                }
            }
        }
        layout_block(c, body, &facts, &mut out);
    }
    markup(c, &c.template, &facts, &BTreeSet::new(), &mut out);
    out
}
fn attr_exprs<'a>(attrs: &'a [Attr]) -> Vec<&'a Expr> {
    let mut out = vec![];
    for a in attrs {
        if a.kind == AttrKind::On {
            continue;
        }
        match &a.value {
            AttrValue::Expr(e) => out.push(e),
            AttrValue::Mixed(parts) => {
                for p in parts {
                    if let AttrPart::Expr(e) = p {
                        out.push(e);
                    }
                }
            }
            _ => (),
        }
    }
    out
}
fn markup(
    c: &Component,
    nodes: &[Node],
    facts: &Facts,
    bound: &BTreeSet<String>,
    out: &mut Vec<Diag>,
) {
    for node in nodes {
        let mut values = vec![];
        match node {
            Node::Interp(i) => values.push(&i.expr),
            Node::Element(e) => {
                values.extend(attr_exprs(&e.attrs));
                markup(c, &e.children, facts, bound, out);
            }
            Node::ComponentRef(e) => {
                values.extend(attr_exprs(&e.attrs));
                markup(c, &e.children, facts, bound, out);
            }
            Node::If(b) => {
                for branch in &b.branches {
                    values.push(&branch.test);
                    markup(c, &branch.body, facts, bound, out);
                }
                if let Some(body) = &b.otherwise {
                    markup(c, body, facts, bound, out);
                }
            }
            Node::Each(b) => {
                let mut next = bound.clone();
                next.extend(names(&b.item));
                if let Some(index) = &b.index {
                    next.insert(index.name.clone());
                }
                markup(c, &b.body, facts, &next, out);
                if let Some(empty) = &b.empty {
                    markup(c, empty, facts, bound, out);
                }
            }
            Node::Await(b) => {
                markup(c, &b.pending, facts, bound, out);
                for branch in [&b.then, &b.catch].into_iter().flatten() {
                    let mut next = bound.clone();
                    if let Some(p) = &branch.binding {
                        next.extend(names(p));
                    }
                    markup(c, &branch.body, facts, &next, out);
                }
            }
            Node::SlotScope(s) => {
                let mut next = bound.clone();
                next.extend(s.names.iter().map(|i| i.name.clone()));
                markup(c, &s.body, facts, &next, out);
            }
            _ => (),
        }
        if bound.is_empty() {
            continue;
        }
        for value in values {
            exprs(value, false, &mut |_| {}, &mut |e| {
                if let Expr::Call {
                    callee,
                    args,
                    optional: false,
                    span,
                } = e
                {
                    let Some((obj, method)) = member(callee) else {
                        return;
                    };
                    let Some(name) = ident(obj) else { return };
                    if !matches!(
                        method,
                        "filter" | "map" | "reduce" | "find" | "some" | "every" | "includes"
                    ) || !facts.arrays.contains(name)
                        || !facts.stable(name)
                        || facts.mutated.contains(name)
                        || !reads(e).is_disjoint(bound)
                    {
                        return;
                    }
                    let mut risky = false;
                    for arg in args {
                        exprs(
                            arg,
                            true,
                            &mut |s| {
                                if matches!(
                                    s,
                                    Stmt::ForOf { .. } | Stmt::Throw { .. } | Stmt::Try { .. }
                                ) {
                                    risky = true;
                                }
                            },
                            &mut |_| {},
                        );
                        exprs(arg, true, &mut |_| {}, &mut |x| {
                            if matches!(
                                x,
                                Expr::Assign { .. }
                                    | Expr::Update { .. }
                                    | Expr::Await { .. }
                                    | Expr::New { .. }
                                    | Expr::Call { .. }
                            ) {
                                risky = true;
                            }
                        });
                    }
                    if !risky {
                        warn(
                            out,
                            c,
                            "B1305",
                            *span,
                            "A collection traversal appears inside each-item rendering without a reference to the item's bound names.",
                            "If reads are stable and the computation is effect-free, derive the shared value outside the each block. Unknown purity, getters and changing external state still require review; no hoist is applied.",
                        );
                    }
                }
            });
        }
    }
}
