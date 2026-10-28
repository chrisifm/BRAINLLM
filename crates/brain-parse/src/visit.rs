//! Name usage over the AST, ignoring shadowing (conservative: a local with the same name counts as a use).
//! Used by the codegen to fold signals that are never written (INTERNALS §13).

use crate::ast::*;
use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct Uses {
    /// Every identifier read.
    pub reads: HashSet<String>,
    /// Root identifiers of assignment/update/delete targets and of `bind:` targets.
    pub writes: HashSet<String>,
    /// Identifiers called directly: `f(…)`.
    pub calls: HashSet<String>,
}

fn root(e: &Expr) -> Option<&str> {
    match e.unparen() {
        Expr::Ident(i) => Some(&i.name),
        Expr::Member { obj, .. } => root(obj),
        _ => None,
    }
}

impl Uses {
    pub fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Ident(i) => {
                self.reads.insert(i.name.clone());
            }
            Expr::This(_) | Expr::Num { .. } | Expr::Str { .. } | Expr::Regex { .. } | Expr::Bool { .. } | Expr::Null(_) => {}
            Expr::Template { tag, exprs, .. } => {
                if let Some(t) = tag {
                    self.expr(t);
                }
                exprs.iter().for_each(|x| self.expr(x));
            }
            Expr::Array { elems, .. } => elems.iter().flatten().for_each(|x| self.expr(x)),
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjProp::KeyValue { key, value, .. } => {
                            self.key(key);
                            self.expr(value);
                        }
                        ObjProp::Shorthand(i) => {
                            self.reads.insert(i.name.clone());
                        }
                        ObjProp::Method { key, func, .. } => {
                            self.key(key);
                            self.func(func);
                        }
                        ObjProp::Spread { arg, .. } => self.expr(arg),
                    }
                }
            }
            Expr::Func(f) => self.func(f),
            Expr::Unary { op, arg, .. } => {
                if *op == "delete"
                    && let Some(r) = root(arg)
                {
                    self.writes.insert(r.to_string());
                }
                self.expr(arg);
            }
            Expr::Update { arg, .. } => {
                if let Some(r) = root(arg) {
                    self.writes.insert(r.to_string());
                }
                self.expr(arg);
            }
            Expr::Assign { target, value, .. } => {
                if let Some(r) = root(target) {
                    self.writes.insert(r.to_string());
                }
                self.expr(target);
                self.expr(value);
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Cond { test, cons, alt, .. } => {
                self.expr(test);
                self.expr(cons);
                self.expr(alt);
            }
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                if let Expr::Ident(i) = callee.unparen() {
                    self.calls.insert(i.name.clone());
                }
                // A method call may mutate its receiver (`xs.push`): count it as a write.
                if let Expr::Member { obj, .. } = callee.unparen()
                    && let Some(r) = root(obj)
                {
                    self.writes.insert(r.to_string());
                }
                self.expr(callee);
                args.iter().for_each(|x| self.expr(x));
            }
            Expr::Member { obj, prop, .. } => {
                self.expr(obj);
                if let MemberProp::Computed(p) = prop {
                    self.expr(p);
                }
            }
            Expr::Seq { exprs, .. } => exprs.iter().for_each(|x| self.expr(x)),
            Expr::Spread { arg, .. } | Expr::Await { arg, .. } | Expr::Paren { expr: arg, .. } => self.expr(arg),
        }
    }

    fn key(&mut self, k: &PropKey) {
        if let PropKey::Computed(e) = k {
            self.expr(e);
        }
    }

    pub fn func(&mut self, f: &Func) {
        f.params.iter().for_each(|p| self.pat(p));
        match &f.body {
            FnBody::Expr(e) => self.expr(e),
            FnBody::Block(b) => b.iter().for_each(|s| self.stmt(s)),
        }
    }

    pub fn pat(&mut self, p: &Pat) {
        match p {
            Pat::Ident(_) => {}
            Pat::Default { pat, default, .. } => {
                self.pat(pat);
                self.expr(default);
            }
            Pat::Object { props, rest, .. } => {
                for pp in props {
                    self.key(&pp.key);
                    self.pat(&pp.value);
                }
                if let Some(r) = rest {
                    self.pat(r);
                }
            }
            Pat::Array { elems, rest, .. } => {
                elems.iter().flatten().for_each(|e| self.pat(e));
                if let Some(r) = rest {
                    self.pat(r);
                }
            }
            Pat::Rest { arg, .. } => self.pat(arg),
        }
    }

    pub fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Var(v) => {
                for d in &v.decls {
                    self.pat(&d.pat);
                    if let Some(i) = &d.init {
                        self.expr(i);
                    }
                }
            }
            Stmt::Expr { expr, .. } | Stmt::Throw { arg: expr, .. } => self.expr(expr),
            Stmt::If { test, cons, alt, .. } => {
                self.expr(test);
                self.stmt(cons);
                if let Some(a) = alt {
                    self.stmt(a);
                }
            }
            Stmt::ForOf { pat, iter, body, .. } => {
                self.pat(pat);
                self.expr(iter);
                self.stmt(body);
            }
            Stmt::Return { arg, .. } => {
                if let Some(a) = arg {
                    self.expr(a);
                }
            }
            Stmt::Block { body, .. } => body.iter().for_each(|s| self.stmt(s)),
            Stmt::Func(f) | Stmt::Server { func: f, .. } => self.func(f),
            Stmt::Try { block, param, handler, finalizer, .. } => {
                block.iter().for_each(|s| self.stmt(s));
                if let Some(p) = param {
                    self.pat(p);
                }
                handler.iter().flatten().for_each(|s| self.stmt(s));
                finalizer.iter().flatten().for_each(|s| self.stmt(s));
            }
            Stmt::Export { decl, .. } => self.stmt(decl),
            Stmt::ExportDefault { expr, .. } => self.expr(expr),
            Stmt::Import(_) | Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Empty { .. } => {}
        }
    }

    pub fn nodes(&mut self, nodes: &[Node]) {
        for n in nodes {
            match n {
                Node::Text(_) => {}
                Node::Slot(sl) => self.attrs(&sl.attrs),
                Node::SlotScope(b) => self.nodes(&b.body),
                Node::Interp(i) => self.expr(&i.expr),
                Node::Element(e) => {
                    self.attrs(&e.attrs);
                    self.nodes(&e.children);
                }
                Node::ComponentRef(r) => {
                    self.reads.insert(r.name.clone());
                    self.attrs(&r.attrs);
                    self.nodes(&r.children);
                }
                Node::If(b) => {
                    for br in &b.branches {
                        self.expr(&br.test);
                        self.nodes(&br.body);
                    }
                    if let Some(o) = &b.otherwise {
                        self.nodes(o);
                    }
                }
                Node::Each(b) => {
                    self.expr(&b.list);
                    if let Some(k) = &b.key {
                        self.expr(k);
                    }
                    self.nodes(&b.body);
                    if let Some(e) = &b.empty {
                        self.nodes(e);
                    }
                }
                Node::Await(b) => {
                    self.expr(&b.promise);
                    self.nodes(&b.pending);
                    for br in [&b.then, &b.catch].into_iter().flatten() {
                        self.nodes(&br.body);
                    }
                }
            }
        }
    }

    fn attrs(&mut self, attrs: &[Attr]) {
        for a in attrs {
            match &a.value {
                AttrValue::Expr(e) => {
                    if a.kind == AttrKind::Bind
                        && let Some(r) = root(e)
                    {
                        self.writes.insert(r.to_string());
                    }
                    self.expr(e);
                }
                AttrValue::Mixed(parts) => {
                    for p in parts {
                        if let AttrPart::Expr(e) = p {
                            self.expr(e);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Script and markup of a whole component.
    pub fn component(c: &Component) -> Uses {
        let mut u = Uses::default();
        c.script.iter().for_each(|s| u.stmt(s));
        u.nodes(&c.template);
        for p in &c.props {
            if let Some(d) = &p.default {
                u.expr(d);
            }
        }
        u
    }
}
