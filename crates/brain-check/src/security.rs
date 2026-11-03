//! INTERNALS §31, compiler side:
//! - B0213 static `javascript:`/`vbscript:`/`data:` URL in `href`/`src`/`action`/`formaction`/`srcset`/`xlink:href`
//!   (`data:image/*` is fine in `src`/`srcset`). Bound expressions are checked by the runtime.
//! - B0210 a bound iframe `srcdoc` without a `sandbox` that keeps it in an opaque origin: the iframe would render the value
//!   as a same-origin document. (`sanitize` does not help here: it marks a value for `{@html}`, which cleans it.)
//! - B0215 (info) `target="_blank"` without `rel="noopener noreferrer"`: added automatically.
//! - B0122 import by URL (`http(s)://`, `//host`).
//! - B0123 third-party `<script src>` in `app.brain` without `integrity`.
//! - B0737 (warning) a credential-shaped value in `ctx.session.set`: signed, not encrypted (use `ctx.session.setSecret`).
//! - B0783 (warning) a credential in a log or browser storage, or a password returned from server code to the browser.
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

/// Attributes whose value is a URL; the same names as runtime/brain.js `URL_ATTRS` (a test keeps them identical).
/// `srcset`/`imagesrcset` hold one URL per comma-separated candidate, `ping` space-separated URLs.
const URL_ATTRS: &[&str] = &["href", "src", "action", "formaction", "poster", "xlink:href", "cite", "data", "background", "longdesc", "manifest", "codebase", "icon", "srcset", "imagesrcset", "ping"];

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

/// The whole value when it is known at compile time (`'/a' + '/b'`, a template without expressions).
fn literal_value(e: &Expr) -> Option<String> {
    match e.unparen() {
        Expr::Str { value, .. } => Some(value.clone()),
        Expr::Template { tag: None, quasis, exprs, .. } if exprs.is_empty() => Some(quasis.concat()),
        Expr::Binary { op: "+", left, right, .. } => Some(literal_value(left)? + &literal_value(right)?),
        _ => None,
    }
}

/// A same-site path: `/` and then a character that cannot turn it into another origin. Browsers read `//host` and
/// `/\host` as protocol-relative and drop tabs and newlines first (`/\t/host`), so the second character decides.
/// `complete` says the value ends here: the bare `/` is safe on its own but not as the start of a longer value.
fn safe_path_start(p: &str, complete: bool) -> bool {
    let mut c = p.chars();
    c.next() == Some('/') && c.next().map_or(complete, |x| x != '/' && x != '\\' && !x.is_whitespace() && !x.is_control())
}

/// An allowed host only when the host part cannot grow: `'https://good.example' + rest` can become `good.example.evil`.
fn fixed_host(p: &str, complete: bool) -> Option<&str> {
    let h = host_of(p)?;
    let start = p.find("//").map_or(0, |i| i + 2);
    (complete || p.len() > start + h.len()).then_some(h)
}

/// `https://host/…` → `host`.
fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).or_else(|| url.strip_prefix("//"))?;
    Some(rest.split(['/', '?', '#', ':']).next().unwrap_or(""))
}

/// The URLs of an attribute value: each srcset candidate, each space-separated `ping` URL, else the value itself.
fn url_list<'a>(v: &'a str, attr: &str) -> Vec<&'a str> {
    match attr {
        "srcset" | "imagesrcset" => v.split(',').filter_map(|c| c.split_whitespace().next()).collect(),
        "ping" => v.split_whitespace().collect(),
        _ => vec![v],
    }
}

/// Markup the author wrote: text, a string literal or a template without values.
fn literal_markup(v: &AttrValue) -> bool {
    match v {
        AttrValue::Text(_) | AttrValue::None => true,
        AttrValue::Mixed(parts) => parts.iter().all(|p| matches!(p, AttrPart::Text(_))),
        AttrValue::Expr(x) => match x.unparen() {
            Expr::Str { .. } => true,
            Expr::Template { tag: None, exprs, .. } => exprs.is_empty(),
            _ => false,
        },
    }
}

/// A literal `sandbox` without `allow-same-origin`: the framed document gets an opaque origin, so it cannot reach the page.
fn opaque_sandbox(attrs: &[Attr]) -> bool {
    attrs.iter().any(|a| {
        a.kind == AttrKind::Plain
            && a.name.eq_ignore_ascii_case("sandbox")
            && match &a.value {
                AttrValue::None => true,
                AttrValue::Text(t) => !t.split_ascii_whitespace().any(|f| f.eq_ignore_ascii_case("allow-same-origin")),
                _ => false,
            }
    })
}

fn dangerous_url(v: &str, attr: &str) -> Option<&'static str> {
    let l: String = v.trim().chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    if l.starts_with("javascript:") {
        Some("javascript:")
    } else if l.starts_with("vbscript:") {
        Some("vbscript:")
    } else if l.starts_with("data:") && !(l.starts_with("data:image/") && matches!(attr, "src" | "srcset" | "imagesrcset" | "poster")) {
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

/// A key that names a credential: `token`, `accessToken`, `refresh_token`, `password`, `apiKey`… Word-based so that
/// `tokens_used` style counters and `passwordless` flags stay quiet.
fn secret_key(name: &str) -> bool {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    for c in name.chars() {
        if c == '_' || c == '-' {
            words.push(std::mem::take(&mut word));
        } else if c.is_ascii_uppercase() {
            words.push(std::mem::take(&mut word));
            word.push(c.to_ascii_lowercase());
        } else {
            word.push(c);
        }
    }
    words.push(word);
    words.retain(|w| !w.is_empty());
    let Some(last) = words.last().map(String::as_str) else { return false };
    let before = words.len().checked_sub(2).map(|i| words[i].as_str());
    matches!(last, "token" | "jwt" | "password" | "passwd" | "pwd" | "secret" | "credential" | "credentials" | "apikey")
        || (last == "key" && matches!(before, Some("api" | "private" | "secret")))
}

/// A stored password in any form: `password`, `passwordHash`, `password_digest`. A token may legitimately reach its
/// owner's browser, so a return value only warns on these; `password_changed_at` and `passwordless` are not secrets.
fn password_key(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["password", "passwd", "pwd"].iter().any(|word| lower.ends_with(word) || ["hash", "_hash", "digest", "_digest"].iter().any(|tail| lower.ends_with(&format!("{word}{tail}"))))
}

/// Credential-shaped keys in a literal object (nested too), identifiers and member names: `{ token: t }`,
/// `{ user: { password } }`, `accessToken`, `d.password`, `` `got ${token}` ``, `'got ' + token`.
fn secret_shaped(e: &Expr, out: &mut Vec<String>) {
    named_like(e, &secret_key, out);
}

fn named_like(e: &Expr, is_secret: &dyn Fn(&str) -> bool, out: &mut Vec<String>) {
    match e.unparen() {
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    ObjProp::KeyValue { key, value, .. } => {
                        let name = match key {
                            PropKey::Ident(i) => Some(i.name.as_str()),
                            PropKey::Str { value, .. } => Some(value.as_str()),
                            _ => None,
                        };
                        if let Some(name) = name.filter(|n| is_secret(n)) {
                            out.push(name.to_string());
                        }
                        named_like(value, is_secret, out);
                    }
                    ObjProp::Shorthand(i) if is_secret(&i.name) => out.push(i.name.clone()),
                    _ => {}
                }
            }
        }
        Expr::Ident(i) if is_secret(&i.name) => out.push(i.name.clone()),
        Expr::Member { prop: MemberProp::Name(n), .. } if is_secret(&n.name) => out.push(n.name.clone()),
        Expr::Template { exprs, .. } => exprs.iter().for_each(|x| named_like(x, is_secret, out)),
        Expr::Binary { left, right, .. } => {
            named_like(left, is_secret, out);
            named_like(right, is_secret, out);
        }
        _ => {}
    }
}

/// `return` statements of a function body, without entering nested functions.
fn returns<'a>(stmts: &'a [Stmt], out: &mut Vec<&'a Expr>) {
    for s in stmts {
        match s {
            Stmt::Return { arg: Some(e), .. } => out.push(e),
            Stmt::If { cons, alt, .. } => {
                returns(std::slice::from_ref(cons), out);
                alt.iter().for_each(|a| returns(std::slice::from_ref(a), out));
            }
            Stmt::Block { body, .. } => returns(body, out),
            Stmt::ForOf { body, .. } => returns(std::slice::from_ref(body), out),
            Stmt::Try { block, handler, finalizer, .. } => {
                returns(block, out);
                handler.iter().for_each(|h| returns(h, out));
                finalizer.iter().for_each(|f| returns(f, out));
            }
            _ => {}
        }
    }
}

/// Functions whose return value is sent to the browser: `server:` functions, `load` and route handlers.
fn answering_functions(c: &Component) -> Vec<&Func> {
    let handler = c.file.starts_with("server/routes/") && c.file.ends_with("/handler.jsbrain");
    let mut out = Vec::new();
    for s in &c.script {
        match s {
            Stmt::Server { func, .. } => out.push(func),
            Stmt::Export { decl, .. } => {
                if let Stmt::Func(f) = decl.as_ref()
                    && f.name.as_ref().is_some_and(|n| n.name == "load")
                {
                    out.push(f)
                }
            }
            Stmt::ExportDefault { expr, .. } if handler => {
                if let Expr::Func(f) = expr.unparen() {
                    out.push(f)
                }
            }
            _ => {}
        }
    }
    out
}

impl Checker<'_> {
    pub(crate) fn security(&mut self) {
        let c = self.c;
        let module_server = c.kind == FileKind::Module && c.meta.iter().any(|m| m.name == "server");
        // Imports by URL.
        for s in &c.script {
            if let Stmt::Import(i) = s
                && (i.source.starts_with("http://") || i.source.starts_with("https://") || i.source.starts_with("//"))
                && !self.ctx.approved_remote_imports.contains(&i.source)
            {
                self.push("B0122", i.span, "remote import has no verified immutable local artifact", "explicitly fetch the exact CDN file/version/SHA-256 with brain deps fetch, or import a reviewed local module");
            }
        }
        for func in answering_functions(c) {
            let mut answers = Vec::new();
            match &func.body {
                FnBody::Block(body) => returns(body, &mut answers),
                FnBody::Expr(e) => answers.push(e),
            }
            for answer in answers {
                let mut found = Vec::new();
                named_like(answer, &password_key, &mut found);
                if let Some(key) = found.first() {
                    self.push("B0783", answer.span(), format!("`{key}` is returned to the browser"), "return only what the page shows and leave the stored password on the server");
                }
            }
        }
        let allowed: Vec<String> = self.ctx.redirect_hosts.clone();
        let mut same_origin = same_origin_helpers(&c.script).0;
        same_origin.extend(self.ctx.redirect_helpers.iter().cloned());
        let mut out: Vec<(&'static str, Span, String, String)> = Vec::new();
        walk(&c.script, module_server, &mut |e, server, rpc_args| match e {
            Expr::Call { callee, args, span, .. } => {
                let name = path(callee).unwrap_or_default();
                if name == "redirect"
                    && let Some(a) = args.first()
                {
                    let whole = literal_value(a);
                    let prefix = whole.clone().or_else(|| url_prefix(a));
                    let ok = same_origin_target(a, &same_origin, false)
                        || prefix.as_deref().and_then(|p| fixed_host(p, whole.is_some())).is_some_and(|h| !h.is_empty() && allowed.iter().any(|x| x == h));
                    if !ok {
                        let what = match prefix.as_deref().and_then(host_of) {
                            Some(h) => format!("`redirect` to another origin (`{h}`)"),
                            None => "`redirect` to a URL built at runtime: it can be pointed at any site (open redirect)".to_string(),
                        };
                        out.push(("B0710", *span, what, "redirect to a same-origin path with a fixed first segment (`redirect('/home')`, `redirect('/posts/' + slug)`; a bare `'/' + value` can become `/\\host`), or list the host in `brain.json` → `\"redirects\": [\"host\"]`".into()));
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
                if (name == "ctx.session.set" || name.ends_with(".session.set")) && let Some(a) = args.first() {
                    let mut found = Vec::new();
                    secret_shaped(a, &mut found);
                    if !found.is_empty() {
                        found.dedup();
                        let keys = found.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ");
                        out.push(("B0737", *span, format!("{keys} goes into `ctx.session.set`: that cookie is signed, not encrypted, so anyone holding it can read the value"), "keep only the user's identity in `ctx.session.set`, and move the value to the encrypted `ctx.session.setSecret(value)` (read it with `ctx.session.secret()`)".into()));
                    }
                }
                if name.strip_prefix("console.").is_some() {
                    let mut found = Vec::new();
                    args.iter().for_each(|a| secret_shaped(a, &mut found));
                    if let Some(key) = found.first() {
                        out.push(("B0783", *span, format!("`{key}` goes into a log, which keeps it where others can read it"), "log the identity or an outcome instead (`user.id`, `'signed in'`), never the credential".into()));
                    }
                }
                if matches!(name.as_str(), "localStorage.setItem" | "sessionStorage.setItem") {
                    let mut found = Vec::new();
                    if let Some(Expr::Str { value, .. }) = args.first().map(Expr::unparen)
                        && secret_key(value)
                    {
                        found.push(value.clone());
                    }
                    args.get(1).iter().for_each(|a| secret_shaped(a, &mut found));
                    if let Some(key) = found.first() {
                        out.push(("B0783", *span, format!("`{key}` goes into browser storage, which any script on the page can read"), "keep credentials on the server: sign in with `ctx.backend.login(…)` or `ctx.session.setSecret(value)`, which stay out of reach of scripts".into()));
                    }
                }
                if name == "RegExp"
                    && let Some(a) = args.first()
                    && !regex_source_safe(a)
                {
                    out.push(("B1311", *span, "`RegExp` built from a runtime value: a crafted pattern can freeze the page (ReDoS)".into(), "escape it first: `new RegExp(text.replace(/[.*+?^${}()|[\\]\\\\]/g, '\\\\$&'))`, or use `includes()`".into()));
                }
            }
            Expr::New { callee, args, span } if path(callee).as_deref() == Some("RegExp") => {
                if let Some(a) = args.first()
                    && !regex_source_safe(a)
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
                    self.url_attrs(&e.tag, &e.attrs);
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

    fn url_attrs(&mut self, tag: &str, attrs: &[Attr]) {
        for a in attrs {
            let name = a.name.to_ascii_lowercase();
            if a.kind == AttrKind::Plain && name == "srcdoc" && tag.eq_ignore_ascii_case("iframe") && !literal_markup(&a.value) && !opaque_sandbox(attrs) {
                self.push("B0210", a.span, "bound `srcdoc` without `sandbox`: the iframe renders the value as a same-origin document, so its scripts run with the page's rights", "add `sandbox` without `allow-same-origin` to the iframe (the document then runs in an opaque origin), or write the document literally");
                continue;
            }
            if a.kind != AttrKind::Plain || !URL_ATTRS.contains(&name.as_str()) || (name == "data" && !tag.eq_ignore_ascii_case("object")) {
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
                && let Some(scheme) = url_list(&v, &name).into_iter().find_map(|u| dangerous_url(u, &name))
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

/// B1311: a `RegExp` source that cannot carry a crafted pattern: literals, an `escape…(…)` helper, the escape the
/// fix itself proposes (`x.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')`, also `replaceAll`), and `+` or template
/// concatenations made only of those parts.
fn regex_source_safe(e: &Expr) -> bool {
    match e.unparen() {
        Expr::Str { .. } | Expr::Regex { .. } | Expr::Num { .. } => true,
        Expr::Template { tag: None, exprs, .. } => exprs.iter().all(regex_source_safe),
        Expr::Binary { op: "+", left, right, .. } => regex_source_safe(left) && regex_source_safe(right),
        Expr::Call { callee, args, .. } => {
            if path(callee).is_some_and(|p| p.to_ascii_lowercase().contains("escape")) {
                return true;
            }
            let Expr::Member { prop: MemberProp::Name(m), .. } = callee.unparen() else { return false };
            if !matches!(m.name.as_str(), "replace" | "replaceAll") {
                return false;
            }
            let escapes_all = matches!(args.first().map(Expr::unparen), Some(Expr::Regex { raw, .. })
                if raw.rsplit_once('/').is_some_and(|(_, flags)| flags.contains('g')) && ['.', '*', '+', '?', '^', '$', '{', '}', '(', ')', '|', '[', ']', '\\'].iter().all(|c| raw.contains(*c)));
            let keeps_match = matches!(args.get(1).map(Expr::unparen), Some(Expr::Str { value, .. }) if value == "\\$&");
            escapes_all && keeps_match
        }
        _ => false,
    }
}

/// A `redirect` target that stays on this origin: a literal path prefix (`'/x' + …`, `` `/x/${…}` ``), a ternary of
/// two such targets, or a call to a helper (in `known`, by its call path) whose every return is one.
/// `strict`: the value may be followed by more text (a helper's return, the left side of `+`), so the bare `/` is not
/// enough: `'/' + next` with `next = '/evil.example'` leaves the site.
fn same_origin_target(e: &Expr, known: &HashSet<String>, strict: bool) -> bool {
    if let Some(v) = literal_value(e) {
        return safe_path_start(&v, !strict);
    }
    match e.unparen() {
        Expr::Cond { cons, alt, .. } => same_origin_target(cons, known, strict) && same_origin_target(alt, known, strict),
        Expr::Call { callee, .. } => path(callee).is_some_and(|p| known.contains(&p)),
        Expr::Binary { op: "+", left, .. } if url_prefix(left).is_none() => same_origin_target(left, known, true),
        Expr::Template { tag: None, quasis, exprs, .. } if quasis.first().is_some_and(|q| q.is_empty()) => exprs.first().is_some_and(|x| same_origin_target(x, known, true)),
        _ => url_prefix(e).is_some_and(|p| safe_path_start(&p, false)),
    }
}

fn func_returns_same_origin(f: &Func, known: &HashSet<String>) -> bool {
    let mut answers = Vec::new();
    match &f.body {
        FnBody::Block(body) => returns(body, &mut answers),
        FnBody::Expr(e) => answers.push(e),
    }
    !answers.is_empty() && answers.iter().all(|a| same_origin_target(a, known, true))
}

/// Helpers of a file whose every return is a same-origin path (B0710 accepts `redirect(helper(x))`).
/// Returns the top-level names and the members of `export default { … }` that qualify.
pub(crate) fn same_origin_helpers(stmts: &[Stmt]) -> (HashSet<String>, HashSet<String>) {
    let mut funcs: Vec<(String, &Func)> = Vec::new();
    fn top<'a>(s: &'a Stmt, funcs: &mut Vec<(String, &'a Func)>) {
        match s {
            Stmt::Func(f) => {
                if let Some(n) = &f.name {
                    funcs.push((n.name.clone(), f));
                }
            }
            Stmt::Var(v) => {
                for d in &v.decls {
                    if let (Pat::Ident(id), Some(Expr::Func(f))) = (&d.pat, d.init.as_ref().map(Expr::unparen)) {
                        funcs.push((id.name.clone(), f));
                    }
                }
            }
            Stmt::Export { decl, .. } => top(decl, funcs),
            _ => {}
        }
    }
    for s in stmts {
        top(s, &mut funcs);
    }
    let mut members: Vec<(String, &Func)> = Vec::new();
    let mut shorthand: Vec<(String, String)> = Vec::new();
    for s in stmts {
        let Stmt::ExportDefault { expr: Expr::Object { props, .. }, .. } = s else { continue };
        for p in props {
            match p {
                ObjProp::KeyValue { key: PropKey::Ident(k), value, .. } => match value.unparen() {
                    Expr::Func(f) => members.push((k.name.clone(), f)),
                    Expr::Ident(i) => shorthand.push((k.name.clone(), i.name.clone())),
                    _ => {}
                },
                ObjProp::Method { key: PropKey::Ident(k), func, .. } => members.push((k.name.clone(), func)),
                ObjProp::Shorthand(i) => shorthand.push((i.name.clone(), i.name.clone())),
                _ => {}
            }
        }
    }
    // Fixed point: a helper may build its path with another helper of the same file.
    let mut known: HashSet<String> = HashSet::new();
    loop {
        let before = known.len();
        for (name, f) in &funcs {
            if !known.contains(name) && func_returns_same_origin(f, &known) {
                known.insert(name.clone());
            }
        }
        if known.len() == before {
            break;
        }
    }
    let mut exported: HashSet<String> = shorthand.into_iter().filter(|(_, local)| known.contains(local)).map(|(k, _)| k).collect();
    exported.extend(members.into_iter().filter(|(_, f)| func_returns_same_origin(f, &known)).map(|(k, _)| k));
    (known, exported)
}

/// Where a value is known to go to an origin the app chose: inside `if (new URL(x, location.origin).origin ===
/// location.origin) …`, or after `if (new URL(x, location.origin).origin !== location.origin) return` in the same block.
/// The same two forms against a literal external origin (`new URL(x).host === 'checkout.example.com'`, `.hostname`, or
/// `.origin === 'https://checkout.example.com'`) carry that host; B0814 accepts them when `brain.json` → `"redirects"`
/// lists it. Each entry: (value path, `None` for this origin or `Some(host)`, where the guarantee holds).
fn origin_guarded(stmts: &[Stmt], out: &mut Vec<(String, Option<String>, Span)>) {
    let own = |e: &Expr| path(e).is_some_and(|p| matches!(p.as_str(), "location.origin" | "window.location.origin" | "document.location.origin"));
    // `new URL(x, base).<prop>` → (x, prop, base is this origin).
    let checked = |e: &Expr| match e.unparen() {
        Expr::Member { obj, prop: MemberProp::Name(p), .. } if matches!(p.name.as_str(), "origin" | "host" | "hostname") => match obj.unparen() {
            Expr::New { callee, args, .. } if path(callee).as_deref() == Some("URL") => args.first().and_then(path).map(|x| (x, p.name.clone(), args.get(1).is_some_and(own))),
            _ => None,
        },
        _ => None,
    };
    let against = |(x, prop, own_base): (String, String, bool), other: &Expr| -> Option<(String, Option<String>)> {
        if own(other) {
            return (prop == "origin" && own_base).then_some((x, None));
        }
        let lit = literal_value(other)?.to_ascii_lowercase();
        let host = if prop == "origin" { lit.strip_prefix("https://").or_else(|| lit.strip_prefix("http://"))? } else { lit.as_str() };
        let host = host.split(':').next().unwrap_or("");
        (!host.is_empty() && !host.contains(['/', '?', '#', '@', '\\'])).then(|| (x, Some(host.to_string())))
    };
    let guard = |t: &Expr| match t.unparen() {
        Expr::Binary { op, left, right, .. } if matches!(*op, "===" | "!==" | "==" | "!=") => {
            checked(left).and_then(|c| against(c, right)).or_else(|| checked(right).and_then(|c| against(c, left))).map(|(p, h)| (p, h, op.starts_with('=')))
        }
        _ => None,
    };
    let leaves = |s: &Stmt| match s {
        Stmt::Return { .. } | Stmt::Throw { .. } => true,
        Stmt::Block { body, .. } => matches!(body.last(), Some(Stmt::Return { .. } | Stmt::Throw { .. })),
        _ => false,
    };
    let last = stmts.last().map(|x| x.span().end).unwrap_or(0);
    for s in stmts {
        if let Stmt::If { test, cons, span, .. } = s
            && let Some((p, host, same)) = guard(test)
        {
            if same {
                out.push((p, host, cons.span()));
            } else if leaves(cons) {
                out.push((p, host, Span { start: span.end, end: last, ..*span }));
            }
        }
        match s {
            Stmt::If { cons, alt, .. } => {
                origin_guarded(std::slice::from_ref(cons.as_ref()), out);
                if let Some(a) = alt {
                    origin_guarded(std::slice::from_ref(a.as_ref()), out);
                }
            }
            Stmt::Block { body, .. } => origin_guarded(body, out),
            Stmt::Func(f) => func_guards(f, out),
            Stmt::Export { decl, .. } => origin_guarded(std::slice::from_ref(decl.as_ref()), out),
            Stmt::Var(v) => {
                for d in &v.decls {
                    if let Some(Expr::Func(f)) = d.init.as_ref().map(Expr::unparen) {
                        func_guards(f, out);
                    }
                }
            }
            Stmt::Try { block, handler, finalizer, .. } => {
                origin_guarded(block, out);
                origin_guarded(handler.as_deref().unwrap_or(&[]), out);
                origin_guarded(finalizer.as_deref().unwrap_or(&[]), out);
            }
            Stmt::ForOf { body, .. } => origin_guarded(std::slice::from_ref(body.as_ref()), out),
            _ => {}
        }
    }
}

fn func_guards(f: &Func, out: &mut Vec<(String, Option<String>, Span)>) {
    if let FnBody::Block(b) = &f.body {
        origin_guarded(b, out);
    }
}

/// A fetch of a fixed same-origin path: `fetch('/api/checkout', …)`, awaited or not.
fn own_fetch(e: &Expr) -> bool {
    match e.unparen() {
        Expr::Await { arg, .. } => own_fetch(arg),
        Expr::Call { callee, args, .. } => {
            path(callee).as_deref().is_some_and(|c| matches!(c, "fetch" | "window.fetch"))
                && args.first().is_some_and(|a| url_prefix(a).is_some_and(|p| safe_path_start(&p, literal_value(a).is_some())))
        }
        _ => false,
    }
}

/// What the app's own server answered: `await f(…)` where `f` is a `server:` function of this file, `await r.json()`
/// where `responses(r)` (an own fetch), or `await (await fetch('/api/…')).json()`.
fn server_answer(e: &Expr, server_fns: &HashSet<String>, responses: &dyn Fn(&str) -> bool) -> bool {
    let Expr::Await { arg, .. } = e.unparen() else { return false };
    let Expr::Call { callee, .. } = arg.unparen() else { return false };
    match callee.unparen() {
        Expr::Ident(i) => server_fns.contains(&i.name),
        Expr::Member { obj, prop: MemberProp::Name(m), .. } if m.name == "json" => own_fetch(obj) || matches!(obj.unparen(), Expr::Ident(i) if responses(&i.name)),
        _ => false,
    }
}

/// `const` bindings that hold what the app's own server answered (`const r = await pay(…)`, `const { url } = await
/// pay(…)`, `const res = await fetch('/api/pay'); const { url } = await res.json()`), each with the rest of its block,
/// where the binding is visible and cannot change.
fn server_answers(stmts: &[Stmt], server_fns: &HashSet<String>, out: &mut Vec<(String, Span)>) {
    let last = stmts.last().map(|x| x.span().end).unwrap_or(0);
    let mut fetched: Vec<(String, Span)> = Vec::new();
    let nested = |f: &Func, out: &mut Vec<(String, Span)>| {
        if let FnBody::Block(b) = &f.body {
            server_answers(b, server_fns, out);
        }
    };
    for s in stmts {
        match s {
            Stmt::Var(v) => {
                for d in &v.decls {
                    let Some(init) = d.init.as_ref() else { continue };
                    if let Expr::Func(f) = init.unparen() {
                        nested(f, out);
                        continue;
                    }
                    if v.kind != VarKind::Const {
                        continue;
                    }
                    let visible = Span { start: d.span.end, end: last, ..d.span };
                    if let Pat::Ident(i) = &d.pat
                        && matches!(init.unparen(), Expr::Await { arg, .. } if own_fetch(arg))
                    {
                        fetched.push((i.name.clone(), visible));
                        continue;
                    }
                    let at = d.span.start;
                    let responses = |n: &str| fetched.iter().any(|(r, sp)| r == n && sp.start <= at && at <= sp.end);
                    if !server_answer(init, server_fns, &responses) {
                        continue;
                    }
                    match &d.pat {
                        Pat::Ident(i) => out.push((i.name.clone(), visible)),
                        Pat::Object { props, .. } => out.extend(props.iter().filter_map(|p| match &p.value {
                            Pat::Ident(i) => Some((i.name.clone(), visible)),
                            _ => None,
                        })),
                        _ => {}
                    }
                }
            }
            Stmt::Func(f) | Stmt::Server { func: f, .. } => nested(f, out),
            Stmt::Block { body, .. } => server_answers(body, server_fns, out),
            Stmt::If { cons, alt, .. } => {
                server_answers(std::slice::from_ref(cons.as_ref()), server_fns, out);
                if let Some(a) = alt {
                    server_answers(std::slice::from_ref(a.as_ref()), server_fns, out);
                }
            }
            Stmt::Export { decl, .. } => server_answers(std::slice::from_ref(decl.as_ref()), server_fns, out),
            Stmt::Try { block, handler, finalizer, .. } => {
                server_answers(block, server_fns, out);
                server_answers(handler.as_deref().unwrap_or(&[]), server_fns, out);
                server_answers(finalizer.as_deref().unwrap_or(&[]), server_fns, out);
            }
            _ => {}
        }
    }
}

impl Checker<'_> {
    /// B0814 (warning): browser navigation to a URL built at runtime (`location.href = x`, `location.assign(x)`,
    /// `location.replace(x)`, `window.open(x)`). Accepted: a literal, a value whose host cannot change, a same-origin
    /// path (the B0710 rules, helpers included), a value checked against `location.origin` in the same file, a value
    /// checked against a host that `brain.json` → `"redirects"` lists (an external checkout), and a value the app's own
    /// server answered (`await` of a `server:` function of the file, or `.json()` of a fetch of a fixed same-origin path).
    pub(crate) fn client_redirect(&mut self, e: &Expr) {
        if self.server_ctx {
            return;
        }
        let shadowed = self.scopes.lookup("location").is_some();
        let is_loc = |x: &Expr| match path(x).as_deref() {
            Some("location") => !shadowed,
            Some("window.location" | "document.location") => true,
            _ => false,
        };
        let (sink, target, span) = match e {
            Expr::Call { callee, args, span, .. } => {
                let Some(a) = args.first() else { return };
                match callee.unparen() {
                    Expr::Member { obj, prop: MemberProp::Name(m), .. } if matches!(m.name.as_str(), "assign" | "replace") && is_loc(obj) => (format!("location.{}()", m.name), a, *span),
                    c if path(c).as_deref() == Some("window.open") => ("window.open()".to_string(), a, *span),
                    _ => return,
                }
            }
            Expr::Assign { op: "=", target, value, span } => match target.unparen() {
                Expr::Member { obj, prop: MemberProp::Name(p), .. } if p.name == "href" && is_loc(obj) => ("location.href".to_string(), value.as_ref(), *span),
                t if is_loc(t) => ("location".to_string(), value.as_ref(), *span),
                _ => return,
            },
            _ => return,
        };
        if literal_value(target).is_some() {
            return;
        }
        let prefix = url_prefix(target);
        let relative = prefix.as_deref().is_some_and(|p| p.starts_with('?') || p.starts_with('#') || p.starts_with("./") || p.starts_with("../"));
        let fixed = prefix.as_deref().and_then(|p| fixed_host(p, false)).is_some_and(|h| !h.is_empty());
        if relative || fixed {
            return;
        }
        let mut known = same_origin_helpers(&self.c.script).0;
        known.extend(self.ctx.redirect_helpers.iter().cloned());
        let mut guarded = Vec::new();
        origin_guarded(&self.c.script, &mut guarded);
        let covers = |at: &Span| at.start <= span.start && span.end <= at.end;
        let listed = |h: &str| self.ctx.redirect_hosts.iter().any(|x| x.eq_ignore_ascii_case(h));
        let target_path = path(target);
        let guard_of = |p: &String| guarded.iter().filter(|(g, _, at)| g == p && covers(at)).map(|(_, h, _)| h.clone()).collect::<Vec<_>>();
        let guards = target_path.as_ref().map(guard_of).unwrap_or_default();
        if guards.iter().any(|h| h.as_deref().is_none_or(listed)) {
            return;
        }
        // A value the app's own server answered: a `server:` function of this file, or a fetch of a fixed same-origin path.
        let mut answers = Vec::new();
        server_answers(&self.c.script, &self.server_fns, &mut answers);
        let mut base = target.unparen();
        let mut answered = false;
        while let Expr::Member { obj, prop: MemberProp::Name(_), .. } = base {
            base = obj.unparen();
            answered |= server_answer(base, &self.server_fns, &|_| false);
        }
        answered |= target_path.is_some() && matches!(base, Expr::Ident(i) if answers.iter().any(|(n, at)| *n == i.name && covers(at)));
        if answered || same_origin_target(target, &known, false) {
            return;
        }
        let v = self.text(target.span()).to_string();
        let msg = match guards.iter().flatten().next() {
            Some(h) => format!("`{sink}` gets a URL checked against `{h}`, which `brain.json` → `\"redirects\"` does not list"),
            None => format!("`{sink}` gets a URL built at runtime: a crafted link can send the user to any site (open redirect)"),
        };
        self.push(
            "B0814",
            span,
            msg,
            format!("navigate to a same-origin path (`'/items/' + id`, or a helper whose every return starts with `/`), use the URL your own server answered (`const {{ url }} = await checkout()`), check it first: `if (new URL({v}, location.origin).origin === location.origin)`, or, for an external destination such as a payment checkout, list its host in `brain.json` → `\"redirects\": [\"checkout.example.com\"]` and check `if (new URL({v}).host === 'checkout.example.com')`"),
        );
    }
}

#[cfg(test)]
mod tests {
    /// The runtime sanitizes bound values (client and SSR) and the checker literal ones: the same attribute names.
    #[test]
    fn url_attributes_match_the_runtime_list() {
        let src = include_str!("../../../runtime/brain.js");
        let line = src.lines().find(|l| l.starts_with("const URL_ATTRS = {")).expect("runtime/brain.js declares URL_ATTRS");
        let names: Vec<String> = line.split(['{', ',']).skip(1).filter_map(|e| e.split(": ").next()).map(|n| n.trim().trim_matches('\'').to_string()).filter(|n| !n.is_empty() && !n.contains('}')).collect();
        let runtime: Vec<String> = { let mut v = names; v.sort(); v };
        let mut checker: Vec<String> = super::URL_ATTRS.iter().map(|s| s.to_string()).collect();
        checker.sort();
        assert_eq!(runtime, checker);
    }
}
