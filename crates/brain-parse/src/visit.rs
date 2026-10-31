//! Name usage over the AST. `Uses::component` ignores shadowing (conservative: a local with the same name counts as a
//! use), which the codegen needs to fold signals that are never written (INTERNALS §13). `Uses::free` honors the scopes
//! of functions, blocks, loops and catch clauses, which is what deciding a module-level import needs.

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
    /// `free` only: every name some function, block, loop or catch clause binds.
    pub bound: HashSet<String>,
    scoped: bool,
    scopes: Vec<HashSet<String>>,
}

fn pat_names(p: &Pat, out: &mut HashSet<String>) {
    match p {
        Pat::Ident(i) => {
            out.insert(i.name.clone());
        }
        Pat::Default { pat, .. } => pat_names(pat, out),
        Pat::Rest { arg, .. } => pat_names(arg, out),
        Pat::Object { props, rest, .. } => {
            props.iter().for_each(|p| pat_names(&p.value, out));
            rest.iter().for_each(|r| pat_names(r, out));
        }
        Pat::Array { elems, rest, .. } => {
            elems.iter().flatten().for_each(|e| pat_names(e, out));
            rest.iter().for_each(|r| pat_names(r, out));
        }
    }
}
/// `let`, `const` and function declarations directly in a block.
fn block_names(body: &[Stmt], out: &mut HashSet<String>) {
    for s in body {
        match s {
            Stmt::Var(v) => v.decls.iter().for_each(|d| pat_names(&d.pat, out)),
            Stmt::Func(f) | Stmt::Server { func: f, .. } => out.extend(f.name.iter().map(|n| n.name.clone())),
            Stmt::Export { decl, .. } => block_names(std::slice::from_ref(decl), out),
            _ => {}
        }
    }
}
/// `var` hoists to its function from any nested statement.
fn hoisted_names(body: &[Stmt], out: &mut HashSet<String>) {
    for s in body {
        match s {
            Stmt::Var(v) if v.kind == VarKind::Var => v.decls.iter().for_each(|d| pat_names(&d.pat, out)),
            Stmt::Block { body, .. } => hoisted_names(body, out),
            Stmt::If { cons, alt, .. } => {
                hoisted_names(std::slice::from_ref(cons), out);
                alt.iter().for_each(|a| hoisted_names(std::slice::from_ref(a), out));
            }
            Stmt::ForOf { kind, pat, body, .. } => {
                if *kind == VarKind::Var {
                    pat_names(pat, out);
                }
                hoisted_names(std::slice::from_ref(body), out);
            }
            Stmt::Try { block, handler, finalizer, .. } => {
                hoisted_names(block, out);
                handler.iter().for_each(|h| hoisted_names(h, out));
                finalizer.iter().for_each(|f| hoisted_names(f, out));
            }
            _ => {}
        }
    }
}

fn root(e: &Expr) -> Option<&str> {
    match e.unparen() {
        Expr::Ident(i) => Some(&i.name),
        Expr::Member { obj, .. } => root(obj),
        _ => None,
    }
}

impl Uses {
    fn shadowed(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }
    fn read(&mut self, name: &str) {
        if !self.shadowed(name) {
            self.reads.insert(name.to_string());
        }
    }
    fn write(&mut self, name: &str) {
        if !self.shadowed(name) {
            self.writes.insert(name.to_string());
        }
    }
    fn call(&mut self, name: &str) {
        if !self.shadowed(name) {
            self.calls.insert(name.to_string());
        }
    }
    /// Run `walk` inside a scope that binds `names`; without scope tracking it just walks.
    fn within(&mut self, names: HashSet<String>, walk: impl FnOnce(&mut Self)) {
        if !self.scoped {
            return walk(self);
        }
        self.bound.extend(names.iter().cloned());
        self.scopes.push(names);
        walk(self);
        self.scopes.pop();
    }
    pub fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Ident(i) => {
                self.read(&i.name);
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
                            self.read(&i.name);
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
                    self.write(r);
                }
                self.expr(arg);
            }
            Expr::Update { arg, .. } => {
                if let Some(r) = root(arg) {
                    self.write(r);
                }
                self.expr(arg);
            }
            Expr::Assign { target, value, .. } => {
                if let Some(r) = root(target) {
                    self.write(r);
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
                    self.call(&i.name);
                }
                // A method call may mutate its receiver (`xs.push`): count it as a write.
                if let Expr::Member { obj, .. } = callee.unparen()
                    && let Some(r) = root(obj)
                {
                    self.write(r);
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
        let mut names = HashSet::new();
        f.params.iter().for_each(|p| pat_names(p, &mut names));
        names.extend(f.name.iter().map(|n| n.name.clone()));
        if let FnBody::Block(b) = &f.body {
            block_names(b, &mut names);
            hoisted_names(b, &mut names);
        }
        self.within(names, |u| {
            f.params.iter().for_each(|p| u.pat(p));
            match &f.body {
                FnBody::Expr(e) => u.expr(e),
                FnBody::Block(b) => b.iter().for_each(|s| u.stmt(s)),
            }
        });
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
                self.expr(iter);
                let mut names = HashSet::new();
                pat_names(pat, &mut names);
                self.within(names, |u| {
                    u.pat(pat);
                    u.stmt(body);
                });
            }
            Stmt::Return { arg, .. } => {
                if let Some(a) = arg {
                    self.expr(a);
                }
            }
            Stmt::Block { body, .. } => self.block(body),
            Stmt::Func(f) | Stmt::Server { func: f, .. } => self.func(f),
            Stmt::Try { block, param, handler, finalizer, .. } => {
                self.block(block);
                let mut names = HashSet::new();
                param.iter().for_each(|p| pat_names(p, &mut names));
                handler.iter().for_each(|h| block_names(h, &mut names));
                self.within(names, |u| {
                    param.iter().for_each(|p| u.pat(p));
                    handler.iter().flatten().for_each(|s| u.stmt(s));
                });
                finalizer.iter().for_each(|f| self.block(f));
            }
            Stmt::Export { decl, .. } => self.stmt(decl),
            Stmt::ExportDefault { expr, .. } => self.expr(expr),
            Stmt::Import(_) | Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Empty { .. } => {}
        }
    }

    fn block(&mut self, body: &[Stmt]) {
        let mut names = HashSet::new();
        block_names(body, &mut names);
        self.within(names, |u| body.iter().for_each(|s| u.stmt(s)));
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
                    self.read(&r.name);
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
                        self.write(r);
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
        Self::walk(c, false)
    }
    /// Like `component`, but a name only counts where no enclosing function, block, loop or catch clause binds it.
    /// Template bindings (`each`, `await`) are not scopes here: their names still count, which at worst adds an import.
    pub fn free(c: &Component) -> Uses {
        Self::walk(c, true)
    }
    fn walk(c: &Component, scoped: bool) -> Uses {
        let mut u = Uses { scoped, ..Uses::default() };
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
