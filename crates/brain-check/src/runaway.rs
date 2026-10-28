//! INTERNALS §30, compile-time gate:
//! - B0906 a timer, interval, listener or subscription created outside `fx`, or inside an `fx` that returns no
//!   cleanup. A one-shot `setTimeout`/`requestAnimationFrame` inside an event handler is allowed. `bus().on` and
//!   `store` subscriptions unsubscribe with the component and are not reported.
//! - B0905 a signal holding a list that an `fx`, a timer or a subscription callback appends to with no bound
//!   (`items = [...items, x]` with no `slice` and no `items.length` check): it grows forever without user input.

use crate::scope::Kind;
use crate::Checker;
use brain_parse::ast::*;
use brain_parse::Span;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// Top level of the component script: runs once at mount.
    Top,
    /// Inside an `fx` callback; `true` when it returns a cleanup.
    Fx(bool),
    /// Inside a function (event handler, helper).
    Func,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Res {
    Interval,
    /// `setTimeout` / `requestAnimationFrame`: one-shot.
    Timeout,
    Listener,
    Subscribe,
    /// `x.on(f)` (bus): cleaned up with the owner, but its callback can still grow a list.
    Bus,
}

fn resource(callee: &Expr) -> Option<(Res, String)> {
    match callee.unparen() {
        Expr::Ident(i) => match i.name.as_str() {
            "setInterval" => Some((Res::Interval, i.name.clone())),
            "setTimeout" | "requestAnimationFrame" | "requestIdleCallback" => Some((Res::Timeout, i.name.clone())),
            _ => None,
        },
        Expr::Member { prop: MemberProp::Name(p), .. } => match p.name.as_str() {
            "setInterval" => Some((Res::Interval, p.name.clone())),
            "setTimeout" | "requestAnimationFrame" => Some((Res::Timeout, p.name.clone())),
            "addEventListener" => Some((Res::Listener, p.name.clone())),
            "subscribe" => Some((Res::Subscribe, p.name.clone())),
            "on" => Some((Res::Bus, p.name.clone())),
            _ => None,
        },
        _ => None,
    }
}

/// `{ signal }` / `{ signal: ctl.signal }` in the options of `addEventListener`: removed with the abort.
fn has_signal(args: &[Expr]) -> bool {
    args.iter().any(|a| match a.unparen() {
        Expr::Object { props, .. } => props.iter().any(|p| match p {
            ObjProp::Shorthand(i) => i.name == "signal",
            ObjProp::KeyValue { key: PropKey::Ident(k), .. } => k.name == "signal",
            _ => false,
        }),
        _ => false,
    })
}

/// The `fx` callback returns a function (or the unsubscribe of a subscription).
fn fx_cleanup(f: &Func) -> bool {
    let returns_cleanup = |e: &Expr| match e.unparen() {
        Expr::Func(_) | Expr::Ident(_) => true,
        Expr::Call { callee, .. } => matches!(resource(callee), Some((Res::Subscribe | Res::Bus, _))),
        _ => false,
    };
    match &f.body {
        FnBody::Expr(e) => returns_cleanup(e),
        FnBody::Block(b) => b.iter().any(|s| matches!(s, Stmt::Return { arg: Some(e), .. } if returns_cleanup(e))),
    }
}

struct Walk<'f> {
    sigs: HashSet<String>,
    funcs: HashMap<String, &'f Func>,
    /// (code, span, msg, fix)
    out: Vec<(&'static str, Span, String, String)>,
    /// Appends already reported (an inner callback is reported once, with its own name).
    seen: HashSet<u32>,
}

impl<'f> Walk<'f> {
    fn stmts(&mut self, b: &'f [Stmt], ctx: Ctx) {
        for s in b {
            self.stmt(s, ctx);
        }
    }

    fn stmt(&mut self, s: &'f Stmt, ctx: Ctx) {
        match s {
            Stmt::Var(v) => {
                for d in &v.decls {
                    if let Some(e) = &d.init {
                        self.expr(e, ctx);
                    }
                }
            }
            Stmt::Expr { expr, .. } | Stmt::Throw { arg: expr, .. } | Stmt::ExportDefault { expr, .. } => self.expr(expr, ctx),
            Stmt::Return { arg: Some(e), .. } => self.expr(e, ctx),
            Stmt::If { test, cons, alt, .. } => {
                self.expr(test, ctx);
                self.stmt(cons, ctx);
                if let Some(a) = alt {
                    self.stmt(a, ctx);
                }
            }
            Stmt::ForOf { iter, body, .. } => {
                self.expr(iter, ctx);
                self.stmt(body, ctx);
            }
            Stmt::Block { body, .. } => self.stmts(body, ctx),
            Stmt::Try { block, handler, finalizer, .. } => {
                self.stmts(block, ctx);
                if let Some(h) = handler {
                    self.stmts(h, ctx);
                }
                if let Some(f) = finalizer {
                    self.stmts(f, ctx);
                }
            }
            Stmt::Func(f) => self.func(f),
            // `load` runs on the server.
            Stmt::Export { decl, .. } if matches!(decl.as_ref(), Stmt::Func(f) if f.name.as_ref().is_some_and(|n| n.name == "load")) => {}
            Stmt::Export { decl, .. } => self.stmt(decl, ctx),
            // Server code is not a component lifecycle.
            Stmt::Server { .. } | Stmt::Import(_) | Stmt::Return { arg: None, .. } | Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Empty { .. } => {}
        }
    }

    fn func(&mut self, f: &'f Func) {
        match &f.body {
            FnBody::Expr(e) => self.expr(e, Ctx::Func),
            FnBody::Block(b) => self.stmts(b, Ctx::Func),
        }
    }

    fn exprs(&mut self, xs: &'f [Expr], ctx: Ctx) {
        for x in xs {
            self.expr(x, ctx);
        }
    }

    fn expr(&mut self, e: &'f Expr, ctx: Ctx) {
        match e {
            Expr::Call { callee, args, span, .. } => {
                if matches!(callee.unparen(), Expr::Ident(i) if i.name == "fx")
                    && let Some(Expr::Func(f)) = args.first().map(Expr::unparen)
                {
                    let ctx = Ctx::Fx(fx_cleanup(f));
                    match &f.body {
                        FnBody::Expr(b) => self.expr(b, ctx),
                        FnBody::Block(b) => self.stmts(b, ctx),
                    }
                    self.growth(f, "an `fx`");
                    return;
                }
                if let Some((kind, name)) = resource(callee) {
                    self.resource(kind, &name, args, *span, ctx);
                    for cb in args {
                        match cb.unparen() {
                            Expr::Func(f) => self.growth(f, &format!("the `{name}` callback")),
                            Expr::Ident(i) => {
                                if let Some(f) = self.funcs.get(i.name.as_str()).copied() {
                                    self.growth(f, &format!("`{}` (called by `{name}`)", i.name));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                self.expr(callee, ctx);
                self.exprs(args, ctx);
            }
            Expr::Func(f) => self.func(f),
            Expr::Template { tag, exprs, .. } => {
                if let Some(t) = tag {
                    self.expr(t, ctx);
                }
                self.exprs(exprs, ctx);
            }
            Expr::Array { elems, .. } => {
                for x in elems.iter().flatten() {
                    self.expr(x, ctx);
                }
            }
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjProp::KeyValue { value, .. } => self.expr(value, ctx),
                        ObjProp::Spread { arg, .. } => self.expr(arg, ctx),
                        ObjProp::Method { func, .. } => self.func(func),
                        ObjProp::Shorthand(_) => {}
                    }
                }
            }
            Expr::Unary { arg, .. } | Expr::Update { arg, .. } | Expr::Spread { arg, .. } | Expr::Await { arg, .. } | Expr::Paren { expr: arg, .. } => self.expr(arg, ctx),
            Expr::Binary { left, right, .. } => {
                self.expr(left, ctx);
                self.expr(right, ctx);
            }
            Expr::Assign { target, value, .. } => {
                self.expr(target, ctx);
                self.expr(value, ctx);
            }
            Expr::Cond { test, cons, alt, .. } => {
                self.expr(test, ctx);
                self.expr(cons, ctx);
                self.expr(alt, ctx);
            }
            Expr::New { callee, args, .. } => {
                self.expr(callee, ctx);
                self.exprs(args, ctx);
            }
            Expr::Member { obj, prop, .. } => {
                self.expr(obj, ctx);
                if let MemberProp::Computed(p) = prop {
                    self.expr(p, ctx);
                }
            }
            Expr::Seq { exprs, .. } => self.exprs(exprs, ctx),
            _ => {}
        }
    }

    fn resource(&mut self, kind: Res, name: &str, args: &[Expr], span: Span, ctx: Ctx) {
        if kind == Res::Bus || (kind == Res::Listener && has_signal(args)) {
            return;
        }
        let (stop, example) = match kind {
            Res::Interval => ("clearInterval", "fx(() => { const id = setInterval(tick, 1000); return () => clearInterval(id) })".to_string()),
            Res::Timeout if name == "requestAnimationFrame" => ("cancelAnimationFrame", "fx(() => { const id = requestAnimationFrame(draw); return () => cancelAnimationFrame(id) })".to_string()),
            Res::Timeout => ("clearTimeout", "fx(() => { const id = setTimeout(done, 3000); return () => clearTimeout(id) })".to_string()),
            Res::Listener => ("removeEventListener", "fx(() => { window.addEventListener('resize', fit); return () => window.removeEventListener('resize', fit) })".to_string()),
            Res::Subscribe | Res::Bus => ("unsubscribe", "fx(() => source.subscribe(update))".to_string()),
        };
        let msg = match ctx {
            Ctx::Fx(true) => return,
            // A one-shot timer started by an event handler ends by itself.
            Ctx::Func if kind == Res::Timeout => return,
            Ctx::Top => format!("`{name}` outside `fx`: it keeps running after the component is removed"),
            Ctx::Func => format!("`{name}` inside a function: nothing stops it when the component is removed"),
            Ctx::Fx(false) => format!("`{name}` inside `fx` without cleanup: it outlives the component"),
        };
        self.out.push(("B0906", span, msg, format!("create it inside `fx` and return the cleanup (`{stop}`): `{example}`")));
    }

    /// B0905 in the body of `f` (an `fx`, timer, listener or subscription callback).
    fn growth(&mut self, f: &Func, what: &str) {
        let mut guarded = HashSet::new();
        let mut appends = Vec::new();
        scan(&f.body, &self.sigs, &mut guarded, &mut appends);
        for (name, span) in appends {
            if guarded.contains(&name) || !self.seen.insert(span.start) {
                continue;
            }
            self.out.push((
                "B0905",
                span,
                format!("`{name}` grows without a bound: {what} appends to it on every run, with no user input"),
                format!("cap the list: `{name} = [...{name}, x].slice(-100)`, or stop when `{name}.length` reaches a limit"),
            ));
        }
    }
}

/// Collects unbounded appends `x = [...x, …]` / `x = x.concat(…)` to signals, and the names tested with
/// `x.length` (guards).
fn scan(body: &FnBody, sigs: &HashSet<String>, guarded: &mut HashSet<String>, appends: &mut Vec<(String, Span)>) {
    fn ex(e: &Expr, sigs: &HashSet<String>, g: &mut HashSet<String>, a: &mut Vec<(String, Span)>) {
        match e {
            Expr::Assign { op: "=", target, value, span } => {
                if let Expr::Ident(t) = target.unparen()
                    && sigs.contains(&t.name)
                {
                    let grows = match value.unparen() {
                        Expr::Array { elems, .. } => elems.iter().flatten().any(|x| matches!(x, Expr::Spread { arg, .. } if matches!(arg.unparen(), Expr::Ident(i) if i.name == t.name))),
                        Expr::Call { callee, .. } => matches!(callee.unparen(), Expr::Member { obj, prop: MemberProp::Name(p), .. } if p.name == "concat" && matches!(obj.unparen(), Expr::Ident(i) if i.name == t.name)),
                        _ => false,
                    };
                    if grows {
                        a.push((t.name.clone(), *span));
                    }
                }
                ex(value, sigs, g, a);
            }
            Expr::Member { obj, prop: MemberProp::Name(p), .. } if p.name == "length" => {
                if let Expr::Ident(i) = obj.unparen() {
                    g.insert(i.name.clone());
                }
            }
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                ex(callee, sigs, g, a);
                for x in args {
                    ex(x, sigs, g, a);
                }
            }
            Expr::Func(f) => scan(&f.body, sigs, g, a),
            Expr::Binary { left, right, .. } => {
                ex(left, sigs, g, a);
                ex(right, sigs, g, a);
            }
            Expr::Cond { test, cons, alt, .. } => {
                ex(test, sigs, g, a);
                ex(cons, sigs, g, a);
                ex(alt, sigs, g, a);
            }
            Expr::Unary { arg, .. } | Expr::Paren { expr: arg, .. } | Expr::Await { arg, .. } | Expr::Spread { arg, .. } => ex(arg, sigs, g, a),
            Expr::Seq { exprs, .. } => {
                for x in exprs {
                    ex(x, sigs, g, a);
                }
            }
            _ => {}
        }
    }
    fn st(s: &Stmt, sigs: &HashSet<String>, g: &mut HashSet<String>, a: &mut Vec<(String, Span)>) {
        match s {
            Stmt::Expr { expr, .. } | Stmt::Throw { arg: expr, .. } => ex(expr, sigs, g, a),
            Stmt::Return { arg: Some(e), .. } => ex(e, sigs, g, a),
            Stmt::Var(v) => {
                for d in &v.decls {
                    if let Some(e) = &d.init {
                        ex(e, sigs, g, a);
                    }
                }
            }
            Stmt::If { test, cons, alt, .. } => {
                ex(test, sigs, g, a);
                st(cons, sigs, g, a);
                if let Some(x) = alt {
                    st(x, sigs, g, a);
                }
            }
            Stmt::ForOf { iter, body, .. } => {
                ex(iter, sigs, g, a);
                st(body, sigs, g, a);
            }
            Stmt::Block { body, .. } => {
                for x in body {
                    st(x, sigs, g, a);
                }
            }
            Stmt::Try { block, handler, finalizer, .. } => {
                for x in block.iter().chain(handler.iter().flatten()).chain(finalizer.iter().flatten()) {
                    st(x, sigs, g, a);
                }
            }
            _ => {}
        }
    }
    match body {
        FnBody::Expr(e) => ex(e, sigs, guarded, appends),
        FnBody::Block(b) => {
            for s in b {
                st(s, sigs, guarded, appends);
            }
        }
    }
}

impl Checker<'_> {
    pub(crate) fn runaway(&mut self) {
        let c = self.c;
        if c.kind == FileKind::Module {
            return;
        }
        let top = crate::scope::top_bindings(c);
        let sigs: HashSet<String> = top.iter().filter(|(_, k)| **k == Kind::Sig).map(|(n, _)| n.clone()).collect();
        let mut funcs = HashMap::new();
        for s in &c.script {
            match s {
                Stmt::Func(f) => {
                    if let Some(n) = &f.name {
                        funcs.insert(n.name.clone(), f);
                    }
                }
                Stmt::Var(v) => {
                    for d in &v.decls {
                        if let (Pat::Ident(id), Some(Expr::Func(f))) = (&d.pat, d.init.as_ref().map(Expr::unparen)) {
                            funcs.insert(id.name.clone(), f.as_ref());
                        }
                    }
                }
                _ => {}
            }
        }
        let mut w = Walk { sigs, funcs, out: Vec::new(), seen: HashSet::new() };
        w.stmts(&c.script, Ctx::Top);
        for (code, span, msg, fix) in w.out {
            self.push(code, span, msg, fix);
        }
    }
}
