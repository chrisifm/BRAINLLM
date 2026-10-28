//! INTERNALS §31, compiler side:
//! - B0213 static `javascript:`/`vbscript:`/`data:` URL in `href`/`src`/`action`/`formaction`/`srcset`/`xlink:href`
//!   (`data:image/*` is fine in `src`/`srcset`). Bound expressions are checked by the runtime.
//! - B0215 (info) `target="_blank"` without `rel="noopener noreferrer"`: added automatically.
//! - B0122 import by URL (`http(s)://`, `//host`).
//! - B0123 third-party `<script src>` in `app.brain` without `integrity`.
//! - B0710 `redirect(url)` to another origin (not a same-origin path and not a host listed in `brain.json`
//!   `"redirects"`).
//! - B0711 (warning) server `fetch` whose URL is built from `ctx.params`/`ctx.query`/RPC arguments.
//! - B0713 `document.cookie` write of a session cookie (`session…`, `__Host-…`, `__Secure-…`).
//! - B1310 (warning) a signal created with more than 10 000 items and rendered by `{#each}` (use `Table` or paginate).
//! - B1311 (warning) `new RegExp(<value>)` from a non-literal without escaping (ReDoS).

use crate::Checker;
use brain_parse::ast::*;
use brain_parse::Span;
use std::collections::HashSet;

const URL_ATTRS: &[&str] = &["href", "src", "action", "formaction", "srcset", "xlink:href", "poster", "cite"];

/// Every expression of a statement list, with whether it runs on the server, depth first.
pub(crate) fn walk(stmts: &[Stmt], server: bool, f: &mut dyn FnMut(&Expr, bool, &[String])) {
    for s in stmts {
        stmt(s, server, &[], f);
    }
}

fn stmt(s: &Stmt, server: bool, rpc_args: &[String], f: &mut dyn FnMut(&Expr, bool, &[String])) {
    match s {
        Stmt::Var(v) => {
            for d in &v.decls {
                if let Some(e) = &d.init {
                    expr(e, server, rpc_args, f);
                }
            }
        }
        Stmt::Expr { expr: e, .. } | Stmt::Throw { arg: e, .. } | Stmt::ExportDefault { expr: e, .. } => expr(e, server, rpc_args, f),
        Stmt::Return { arg: Some(e), .. } => expr(e, server, rpc_args, f),
        Stmt::If { test, cons, alt, .. } => {
            expr(test, server, rpc_args, f);
            stmt(cons, server, rpc_args, f);
            if let Some(a) = alt {
                stmt(a, server, rpc_args, f);
            }
        }
        Stmt::ForOf { iter, body, .. } => {
            expr(iter, server, rpc_args, f);
            stmt(body, server, rpc_args, f);
        }
        Stmt::Block { body, .. } => {
            for x in body {
                stmt(x, server, rpc_args, f);
            }
        }
        Stmt::Try { block, handler, finalizer, .. } => {
            for x in block.iter().chain(handler.iter().flatten()).chain(finalizer.iter().flatten()) {
                stmt(x, server, rpc_args, f);
            }
        }
        Stmt::Func(func) => body(func, server, rpc_args, f),
        Stmt::Export { decl, .. } => match decl.as_ref() {
            Stmt::Func(func) if func.name.as_ref().is_some_and(|n| n.name == "load") => body(func, true, &[], f),
            d => stmt(d, server, rpc_args, f),
        },
        Stmt::Server { func, .. } => {
            // Every parameter after `ctx` comes from the client.
            let args: Vec<String> = func.params.iter().skip(1).filter_map(|p| if let Pat::Ident(i) = p { Some(i.name.clone()) } else { None }).collect();
            body(func, true, &args, f)
        }
        _ => {}
    }
}

fn body(func: &Func, server: bool, rpc_args: &[String], f: &mut dyn FnMut(&Expr, bool, &[String])) {
    match &func.body {
        FnBody::Expr(e) => expr(e, server, rpc_args, f),
        FnBody::Block(b) => {
            for s in b {
                stmt(s, server, rpc_args, f);
            }
        }
    }
}

pub(crate) fn expr(e: &Expr, server: bool, rpc_args: &[String], f: &mut dyn FnMut(&Expr, bool, &[String])) {
    f(e, server, rpc_args);
    let kids: Vec<&Expr> = match e {
        Expr::Template { tag, exprs, .. } => tag.iter().map(|t| t.as_ref()).chain(exprs.iter()).collect(),
        Expr::Array { elems, .. } => elems.iter().flatten().collect(),
        Expr::Object { props, .. } => {
            let mut v = Vec::new();
            for p in props {
                match p {
                    ObjProp::KeyValue { value, .. } => v.push(value),
                    ObjProp::Spread { arg, .. } => v.push(arg),
                    ObjProp::Method { func, .. } => body(func, server, rpc_args, f),
                    ObjProp::Shorthand(_) => {}
                }
            }
            v
        }
        Expr::Func(func) => {
            body(func, server, rpc_args, f);
            Vec::new()
        }
        Expr::Unary { arg, .. } | Expr::Update { arg, .. } | Expr::Spread { arg, .. } | Expr::Await { arg, .. } | Expr::Paren { expr: arg, .. } => vec![arg.as_ref()],
        Expr::Binary { left, right, .. } | Expr::Assign { target: left, value: right, .. } => vec![left.as_ref(), right.as_ref()],
        Expr::Cond { test, cons, alt, .. } => vec![test.as_ref(), cons.as_ref(), alt.as_ref()],
        Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => std::iter::once(callee.as_ref()).chain(args.iter()).collect(),
        Expr::Member { obj, prop, .. } => match prop {
            MemberProp::Computed(p) => vec![obj.as_ref(), p.as_ref()],
            _ => vec![obj.as_ref()],
        },
        Expr::Seq { exprs, .. } => exprs.iter().collect(),
        _ => Vec::new(),
    };
    for k in kids {
        expr(k, server, rpc_args, f);
    }
}

/// Identifiers and `a.b.c` paths read by `e`.
fn reads(e: &Expr, out: &mut Vec<String>) {
    expr(e, false, &[], &mut |x, _, _| match x {
        Expr::Ident(i) => out.push(i.name.clone()),
        Expr::Member { .. } => {
            if let Some(p) = path(x) {
                out.push(p);
            }
        }
        _ => {}
    });
}

pub(crate) fn path(e: &Expr) -> Option<String> {
    match e.unparen() {
        Expr::Ident(i) => Some(i.name.clone()),
        Expr::Member { obj, prop: MemberProp::Name(p), .. } => Some(format!("{}.{}", path(obj)?, p.name)),
        _ => None,
    }
}

/// Literal start of a URL expression: the string, or the first chunk of a template / `+` chain.
fn url_prefix(e: &Expr) -> Option<String> {
    match e.unparen() {
        Expr::Str { value, .. } => Some(value.clone()),
        Expr::Template { tag: None, quasis, .. } => quasis.first().cloned(),
        Expr::Binary { op: "+", left, .. } => url_prefix(left),
        _ => None,
    }
}

/// `https://host/…` → `host`.
fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).or_else(|| url.strip_prefix("//"))?;
    Some(rest.split(['/', '?', '#', ':']).next().unwrap_or(""))
}

fn dangerous_url(v: &str, attr: &str) -> Option<&'static str> {
    let l: String = v.trim().chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    if l.starts_with("javascript:") {
        Some("javascript:")
    } else if l.starts_with("vbscript:") {
        Some("vbscript:")
    } else if l.starts_with("data:") && !(l.starts_with("data:image/") && matches!(attr, "src" | "srcset" | "poster")) {
        Some("data:")
    } else {
        None
    }
}

/// Literal `length` of `Array.from({ length: N })`, `new Array(N)`, `Array(N)`.
fn literal_len(e: &Expr) -> Option<f64> {
    let num = |x: &Expr| match x.unparen() {
        Expr::Num { raw, .. } => raw.replace('_', "").parse::<f64>().ok(),
        _ => None,
    };
    match e.unparen() {
        Expr::Call { callee, args, .. } => {
            let name = path(callee)?;
            match name.as_str() {
                "Array.from" => match args.first().map(Expr::unparen) {
                    Some(Expr::Object { props, .. }) => props.iter().find_map(|p| match p {
                        ObjProp::KeyValue { key: PropKey::Ident(k), value, .. } if k.name == "length" => num(value),
                        _ => None,
                    }),
                    _ => None,
                },
                "Array" => args.first().and_then(num),
                _ => match callee.unparen() {
                    // `Array.from(…).map(…)`, `new Array(n).fill(0)`
                    Expr::Member { obj, .. } => literal_len(obj),
                    _ => None,
                },
            }
        }
        Expr::New { callee, args, .. } if matches!(callee.unparen(), Expr::Ident(i) if i.name == "Array") => args.first().and_then(num),
        _ => None,
    }
}

fn each_sources(nodes: &[Node], out: &mut HashSet<String>) {
    for n in nodes {
        match n {
            Node::Each(b) => {
                let mut r = Vec::new();
                reads(&b.list, &mut r);
                out.extend(r);
                each_sources(&b.body, out);
            }
            Node::SlotScope(b) => each_sources(&b.body, out),
            Node::Element(e) => each_sources(&e.children, out),
            Node::ComponentRef(c) => each_sources(&c.children, out),
            Node::If(b) => {
                for br in &b.branches {
                    each_sources(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    each_sources(o, out);
                }
            }
            Node::Await(a) => {
                each_sources(&a.pending, out);
            }
            _ => {}
        }
    }
}

impl Checker<'_> {
    pub(crate) fn security(&mut self) {
        let c = self.c;
        let module_server = c.kind == FileKind::Module && c.meta.iter().any(|m| m.name == "server");
        // Imports by URL.
        for s in &c.script {
            if let Stmt::Import(i) = s
                && (i.source.starts_with("http://") || i.source.starts_with("https://") || i.source.starts_with("//"))
            {
                self.push("B0122", i.span, format!("import by URL `{}`: remote code is not reviewed, pinned or bundled", i.source), "copy the module into `lib/` (or install it) and import it with a relative path");
            }
        }
        let allowed: Vec<String> = self.ctx.redirect_hosts.clone();
        let mut out: Vec<(&'static str, Span, String, String)> = Vec::new();
        walk(&c.script, module_server, &mut |e, server, rpc_args| match e {
            Expr::Call { callee, args, span, .. } => {
                let name = path(callee).unwrap_or_default();
                if name == "redirect"
                    && let Some(a) = args.first()
                {
                    let prefix = url_prefix(a);
                    let ok = match prefix.as_deref() {
                        Some(p) if p.starts_with('/') && !p.starts_with("//") => true,
                        Some(p) => host_of(p).is_some_and(|h| !h.is_empty() && allowed.iter().any(|x| x == h)),
                        None => false,
                    };
                    if !ok {
                        let what = match prefix.as_deref().and_then(host_of) {
                            Some(h) => format!("`redirect` to another origin (`{h}`)"),
                            None => "`redirect` to a URL built at runtime: it can be pointed at any site (open redirect)".to_string(),
                        };
                        out.push(("B0710", *span, what, "redirect to a same-origin path (`redirect('/home')`), or list the host in `brain.json` → `\"redirects\": [\"host\"]`".into()));
                    }
                }
                if server && (name == "fetch" || name.ends_with(".fetch")) && let Some(a) = args.first() {
                    let fixed_host = url_prefix(a).is_some_and(|p| p.starts_with('/') && !p.starts_with("//") || host_of(&p).is_some_and(|h| !h.is_empty()) && p.len() > p.find(host_of(&p).unwrap_or("")).unwrap_or(0) + host_of(&p).unwrap_or("").len());
                    let mut r = Vec::new();
                    reads(a, &mut r);
                    let tainted = r.iter().any(|x| x.starts_with("ctx.params") || x.starts_with("ctx.query") || rpc_args.iter().any(|a| x == a || x.starts_with(&format!("{a}."))));
                    if tainted && !fixed_host {
                        out.push(("B0711", *span, "server `fetch` to a URL built from request input: it can reach internal hosts (SSRF)".into(), "fix the origin in the code (`fetch(`https://api.example.com/users/${id}`)`) or check the host against an allowlist first".into()));
                    }
                }
                if name == "RegExp"
                    && let Some(a) = args.first()
                    && !matches!(a.unparen(), Expr::Str { .. } | Expr::Regex { .. })
                    && !matches!(a.unparen(), Expr::Template { exprs, .. } if exprs.is_empty())
                    && !matches!(a.unparen(), Expr::Call { callee, .. } if path(callee).is_some_and(|p| p.to_ascii_lowercase().contains("escape")))
                {
                    out.push(("B1311", *span, "`RegExp` built from a runtime value: a crafted pattern can freeze the page (ReDoS)".into(), "escape it first: `new RegExp(text.replace(/[.*+?^${}()|[\\]\\\\]/g, '\\\\$&'))`, or use `includes()`".into()));
                }
            }
            Expr::New { callee, args, span } if path(callee).as_deref() == Some("RegExp") => {
                if let Some(a) = args.first()
                    && !matches!(a.unparen(), Expr::Str { .. } | Expr::Regex { .. })
                    && !matches!(a.unparen(), Expr::Template { exprs, .. } if exprs.is_empty())
                    && !matches!(a.unparen(), Expr::Call { callee, .. } if path(callee).is_some_and(|p| p.to_ascii_lowercase().contains("escape")))
                {
                    out.push(("B1311", *span, "`RegExp` built from a runtime value: a crafted pattern can freeze the page (ReDoS)".into(), "escape it first: `new RegExp(text.replace(/[.*+?^${}()|[\\]\\\\]/g, '\\\\$&'))`, or use `includes()`".into()));
                }
            }
            Expr::Assign { target, value, span, .. } if path(target).as_deref() == Some("document.cookie") => {
                let name = url_prefix(value).map(|p| p.trim_start().to_ascii_lowercase()).unwrap_or_default();
                if name.starts_with("session") || name.starts_with("__host-") || name.starts_with("__secure-") || name.starts_with("sid=") {
                    out.push(("B0713", *span, "session cookie written from the browser: script-readable, so any XSS can steal it".into(), "set it on the server (`ctx.cookies.set(…)` / the signed session), where it is `HttpOnly; Secure; SameSite=Lax`".into()));
                }
            }
            _ => {}
        });
        // B1310: a big literal list in a signal that `{#each}` renders.
        let mut iterated = HashSet::new();
        each_sources(&c.template, &mut iterated);
        for s in &c.script {
            let Stmt::Var(v) = s else { continue };
            for d in &v.decls {
                let (Pat::Ident(id), Some(Expr::Call { callee, args, span, .. })) = (&d.pat, d.init.as_ref().map(Expr::unparen)) else { continue };
                if !matches!(callee.unparen(), Expr::Ident(k) if k.name == "sig") || !iterated.contains(&id.name) {
                    continue;
                }
                if let Some(n) = args.first().and_then(literal_len)
                    && n > 10_000.0
                {
                    out.push(("B1310", *span, format!("`{}` holds {n} items and `{{#each}}` renders them all", id.name), "render it with the `Table` kit component (virtualized) or paginate: `{#each page as row (row.id)}`".into()));
                }
            }
        }
        for (code, span, msg, fix) in out {
            self.push(code, span, msg, fix);
        }
        self.security_markup(&c.template);
    }

    fn security_markup(&mut self, nodes: &[Node]) {
        for n in nodes {
            match n {
                Node::Element(e) => {
                    self.url_attrs(&e.attrs);
                    if e.tag == "script" && e.attrs.iter().any(|a| a.name == "src") && !e.attrs.iter().any(|a| a.name == "integrity") {
                        let src = e.attrs.iter().find(|a| a.name == "src").map(|a| match &a.value {
                            AttrValue::Text(t) => t.clone(),
                            _ => String::new(),
                        });
                        let third = src.as_deref().is_some_and(|s| s.starts_with("http://") || s.starts_with("https://") || s.starts_with("//"));
                        if third {
                            self.push("B0123", e.span, "third-party `<script src>` without `integrity`: a change on that server runs in your page", "add `integrity=\"sha384-…\"` and `crossorigin=\"anonymous\"` (the provider publishes the hash), or self-host the file");
                        }
                    }
                    self.blank_target(e);
                    self.security_markup(&e.children);
                }
                Node::ComponentRef(r) => self.security_markup(&r.children),
                Node::If(b) => {
                    for br in &b.branches {
                        self.security_markup(&br.body);
                    }
                    if let Some(o) = &b.otherwise {
                        self.security_markup(o);
                    }
                }
                Node::Each(b) => {
                    self.security_markup(&b.body);
                    if let Some(e) = &b.empty {
                        self.security_markup(e);
                    }
                }
                Node::SlotScope(b) => self.security_markup(&b.body),
                Node::Await(a) => self.security_markup(&a.pending),
                _ => {}
            }
        }
    }

    fn url_attrs(&mut self, attrs: &[Attr]) {
        for a in attrs {
            if a.kind != AttrKind::Plain || !URL_ATTRS.contains(&a.name.as_str()) {
                continue;
            }
            let lit = match &a.value {
                AttrValue::Text(t) => Some(t.clone()),
                AttrValue::Expr(x) => match x.unparen() {
                    Expr::Str { value, .. } => Some(value.clone()),
                    Expr::Template { tag: None, quasis, .. } => quasis.first().cloned(),
                    _ => None,
                },
                AttrValue::Mixed(parts) => match parts.first() {
                    Some(AttrPart::Text(t)) => Some(t.clone()),
                    _ => None,
                },
                AttrValue::None => None,
            };
            if let Some(v) = lit
                && let Some(scheme) = dangerous_url(&v, &a.name)
            {
                self.push("B0213", a.span, format!("`{scheme}` URL in `{}`: it runs or injects code when followed", a.name), "use a button with `on:click` for actions, or a real `https://`/same-origin URL");
            }
        }
    }

    /// B0215 (info): `target="_blank"` gets `rel="noopener noreferrer"`.
    fn blank_target(&mut self, e: &Element) {
        let blank = e.attrs.iter().any(|a| a.name == "target" && matches!(&a.value, AttrValue::Text(t) if t == "_blank"));
        if !blank {
            return;
        }
        let rel = e.attrs.iter().find(|a| a.name == "rel");
        let has = |t: &str| t.split_whitespace().any(|x| x == "noopener" || x == "noreferrer");
        match rel.map(|a| &a.value) {
            Some(AttrValue::Text(t)) if has(t) => {}
            Some(AttrValue::Text(t)) => {
                let a = rel.unwrap_or(&e.attrs[0]);
                let text = self.text(a.span).to_string();
                let new = text.replacen(t.as_str(), &format!("{t} noopener noreferrer"), 1);
                self.push_info("B0215", a.span, "`target=\"_blank\"` without `noopener` (normalized)", "add `noopener noreferrer` to `rel`", vec![(a.span.start, a.span.end, new)]);
            }
            Some(_) => {}
            None => {
                let Some(t) = e.attrs.iter().find(|a| a.name == "target") else { return };
                self.push_info("B0215", t.span, "`target=\"_blank\"` without `rel=\"noopener noreferrer\"` (normalized)", "add `rel=\"noopener noreferrer\"`", vec![(t.span.end, t.span.end, " rel=\"noopener noreferrer\"".into())]);
            }
        }
    }
}
