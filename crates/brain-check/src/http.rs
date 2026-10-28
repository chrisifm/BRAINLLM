//! INTERNALS §40-§41.4, HTTP checks:
//! - B1107 (warning) `fetch()` in client code: `api()` brings typed errors, retries, dedupe, cancel and caching.
//! - B1104 an unknown attribute or value in `<meta http>`, or `<meta http>` outside `app.brain`.
//! - B1106 (warning) independent `await`s in a row in `load()`: they wait for each other; the edit runs them with
//!   `Promise.all`.
//!
//! [`policy_js`] turns a valid `<meta http>` into the `http({ … })` call of the app entry (RUNTIME brain-data.js).

use crate::Checker;
use brain_parse::ast::*;
use brain_parse::Span;

const HTTP_ATTRS: &[&str] = &["base", "timeout", "retry", "login"];
const CLIENT_ATTRS: &[&str] = &["width", "height", "aspect"];
const BUDGET_KEYS: &[&str] = &["rows", "nodes", "runs", "ms"];

/// `<meta budget="rows:200000 ms:400">` → `[("rows", 200000), ("ms", 400)]` (valid pairs only; §30.3, RUNTIME #90).
pub fn budget(c: &Component) -> Vec<(String, u64)> {
    let Some(v) = c.meta.iter().find(|m| m.name == "budget").and_then(|m| m.value.as_deref()) else { return Vec::new() };
    v.split_whitespace()
        .filter_map(|p| p.split_once(':'))
        .filter(|(k, _)| BUDGET_KEYS.contains(k))
        .filter_map(|(k, n)| n.replace('_', "").parse::<u64>().ok().filter(|n| *n > 0).map(|n| (k.to_string(), n)))
        .collect()
}

fn css_length_ok(v: &str) -> bool {
    let v = v.trim();
    let num = v.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
    let unit = &v[num.len()..];
    !num.is_empty() && num.parse::<f64>().is_ok_and(|n| n >= 0.0) && matches!(unit, "" | "px" | "rem" | "em" | "%" | "vh" | "vw" | "dvh" | "svh" | "ch")
}

/// `<meta client height="320px" aspect="16/9">` → the CSS that sizes its server placeholder (§38), or `None`.
pub fn client_placeholder_css(c: &Component, hash: &str) -> Option<String> {
    let attrs = meta_tag(c, "client")?;
    let mut decls = vec!["display:block".to_string()];
    for m in attrs.iter().filter(|m| m.name != "client") {
        let Some(v) = m.value.as_deref() else { continue };
        match m.name.as_str() {
            "width" if css_length_ok(v) => decls.push(format!("inline-size:{}", px(v))),
            "height" if css_length_ok(v) => decls.push(format!("block-size:{}", px(v))),
            "aspect" if aspect_ok(v) => decls.push(format!("aspect-ratio:{v}")),
            _ => {}
        }
    }
    Some(format!("[data-b=\"{hash}\"][data-client]{{{}}}\n", decls.join(";")))
}

fn px(v: &str) -> String {
    if v.chars().all(|c| c.is_ascii_digit() || c == '.') { format!("{v}px") } else { v.to_string() }
}

fn aspect_ok(v: &str) -> bool {
    match v.split_once('/') {
        Some((a, b)) => a.trim().parse::<f64>().is_ok_and(|x| x > 0.0) && b.trim().parse::<f64>().is_ok_and(|x| x > 0.0),
        None => v.trim().parse::<f64>().is_ok_and(|x| x > 0.0),
    }
}

/// Attributes of the `<meta>` tag whose first attribute is `first` (`<meta http base=…>` → `http`, `base`, …).
pub fn meta_tag<'a>(c: &'a Component, first: &str) -> Option<Vec<&'a MetaAttr>> {
    let head = c.meta.iter().find(|m| m.name == first)?;
    let at = head.span.start as usize;
    let start = c.src[..at].rfind("<meta").unwrap_or(0);
    let end = at + c.src[at..].find('>').unwrap_or(c.src.len() - at);
    Some(c.meta.iter().filter(|m| (m.span.start as usize) >= start && (m.span.start as usize) < end).collect())
}

fn duration_ok(v: &str) -> bool {
    let n = v.strip_suffix("ms").or_else(|| v.strip_suffix('s')).unwrap_or(v);
    !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
}

/// Why a `<meta http>` value is wrong, if it is.
fn bad_value(name: &str, v: &str) -> Option<&'static str> {
    match name {
        "base" if !(v.starts_with('/') || v.starts_with("https://") || v.starts_with("http://")) => Some("an absolute path (`/api`) or an origin (`https://api.example.com`)"),
        "timeout" if !duration_ok(v) => Some("a duration such as `10s` or `500ms`"),
        "retry" if !v.parse::<u8>().is_ok_and(|n| n <= 5) => Some("a whole number from 0 to 5"),
        "login" if !v.starts_with('/') || v.starts_with("//") => Some("a path of this app such as `/login` (same origin only)"),
        _ => None,
    }
}

/// `http({ base: "/api", timeout: "10s", retry: 2, login: "/login" })` for a valid `<meta http>` in `app.brain`.
pub fn policy_js(c: &Component) -> Option<String> {
    if c.kind != FileKind::App {
        return None;
    }
    let attrs = meta_tag(c, "http")?;
    let mut parts = Vec::new();
    for m in attrs.iter().filter(|m| m.name != "http") {
        let (Some(v), None) = (&m.value, &m.expr) else { continue };
        if !HTTP_ATTRS.contains(&m.name.as_str()) || bad_value(&m.name, v).is_some() {
            continue;
        }
        let val = if m.name == "retry" { v.clone() } else { brain_parse::Json::str(v.clone()).stringify() };
        parts.push(format!("{}: {val}", m.name));
    }
    Some(format!("http({{ {} }});\n", parts.join(", ")))
}

impl Checker<'_> {
    pub(crate) fn http_rules(&mut self) {
        let c = self.c;
        // B1104
        if let Some(attrs) = meta_tag(c, "http") {
            if c.kind != FileKind::App {
                let span = attrs[0].span;
                self.push("B1104", span, "`<meta http>` is the app-wide HTTP policy and only goes in `app.brain`", "move it to `app.brain`; per-endpoint options go in `api(path, { … })`");
            } else {
                for m in attrs.iter().filter(|m| m.name != "http") {
                    if !HTTP_ATTRS.contains(&m.name.as_str()) {
                        let fix = match crate::scope::closest(&m.name, HTTP_ATTRS.iter().copied()).or_else(|| HTTP_ATTRS.iter().copied().find(|a| a.chars().zip(m.name.chars()).take_while(|(x, y)| x == y).count() >= 4)) {
                            Some(s) => format!("did you mean `{s}`? `<meta http>` takes base, timeout, retry and login"),
                            None => "remove it: `<meta http>` takes base, timeout, retry and login".to_string(),
                        };
                        let start = if c.src.as_bytes().get(m.span.start as usize - 1) == Some(&b' ') { m.span.start - 1 } else { m.span.start };
                        self.push_edits("B1104", m.span, format!("`<meta http>` has no attribute `{}`", m.name), fix, vec![(start, m.span.end, String::new())]);
                        continue;
                    }
                    if m.expr.is_some() {
                        self.push("B1104", m.span, format!("`<meta http {}>` must be a literal", m.name), "write the value as text; state-dependent headers go in `api/_http.jsbrain`");
                        continue;
                    }
                    let v = m.value.as_deref().unwrap_or("");
                    if let Some(want) = bad_value(&m.name, v) {
                        self.push("B1104", m.span, format!("`{}=\"{v}\"` is not valid in `<meta http>`", m.name), format!("use {want}"));
                    }
                }
            }
        }

        // B0909: `<meta budget>` keys and values.
        if let Some(m) = c.meta.iter().find(|m| m.name == "budget") {
            let v = m.value.as_deref().unwrap_or("");
            let bad: Vec<&str> = v.split_whitespace().filter(|p| !p.split_once(':').is_some_and(|(k, n)| BUDGET_KEYS.contains(&k) && n.replace('_', "").parse::<u64>().is_ok_and(|n| n > 0))).collect();
            if v.trim().is_empty() || !bad.is_empty() {
                let what = if bad.is_empty() { "is empty".to_string() } else { format!("has `{}`", bad.join("`, `")) };
                self.push("B0909", m.span, format!("`<meta budget>` {what}"), "write `key:number` pairs with keys rows, nodes, runs and ms, e.g. `<meta budget=\"rows:200000\">`");
            }
        }

        // B0730 (info): a client-only component; B0731: its placeholder size is invalid.
        if let Some(attrs) = meta_tag(c, "client") {
            let mut sized = false;
            for m in attrs.iter().filter(|m| m.name != "client") {
                let v = m.value.as_deref().unwrap_or("");
                let ok = match m.name.as_str() {
                    "width" | "height" => css_length_ok(v),
                    "aspect" => aspect_ok(v),
                    _ => {
                        let near = crate::scope::closest(&m.name, CLIENT_ATTRS.iter().copied());
                        let fix = near.map_or("remove it: `<meta client>` takes width, height and aspect".to_string(), |n| format!("did you mean `{n}`?"));
                        self.push("B0731", m.span, format!("`<meta client>` has no attribute `{}`", m.name), fix);
                        continue;
                    }
                };
                if !ok || m.expr.is_some() {
                    self.push("B0731", m.span, format!("`{}=\"{v}\"` is not a valid placeholder size", m.name), if m.name == "aspect" { "write a ratio such as `16/9`" } else { "write a length such as `320px`, `20rem` or `100%`" });
                } else if m.name != "width" {
                    sized = true;
                }
            }
            let fix = if sized { "keep it client-only only if it needs browser APIs (canvas, maps); everything else renders on the server" } else { "add `height=\"…\"` or `aspect=\"16/9\"` so the server placeholder reserves its space (no layout shift)" };
            self.push("B0730", attrs[0].span, format!("`{}` renders only in the browser: the server sends a placeholder", c.name), fix);
        }

        // B1107: client code only; `load`, `server:` and `<meta server>` modules may call third parties with fetch.
        let api_dir = c.file.starts_with("api/") || c.file.contains("/api/");
        if !self.server_ctx && !api_dir {
            let mut hits: Vec<Span> = Vec::new();
            let mut f = |e: &Expr, server: bool, _: &[String]| {
                if server {
                    return;
                }
                if let Expr::Call { callee, span, .. } = e {
                    let direct = match callee.unparen() {
                        Expr::Ident(i) => i.name == "fetch",
                        Expr::Member { obj, prop: MemberProp::Name(p), .. } => p.name == "fetch" && matches!(obj.unparen(), Expr::Ident(w) if matches!(w.name.as_str(), "window" | "globalThis" | "self")),
                        _ => false,
                    };
                    if direct {
                        hits.push(*span);
                    }
                }
            };
            crate::security::walk(&c.script, false, &mut f);
            crate::planned::collect_calls(&c.template, &mut f);
            if self.scopes.lookup("fetch").is_none() {
                for span in hits {
                    self.push("B1107", span, "`fetch()` in app code skips typed errors, safe retries, dedupe, cancel on unmount and caching", "declare the endpoint in `api/` (`export default api('/path', { get: { out: 'Type' } })`) and call it, or move the call into `load()`");
                }
            }
        }

        // B1106
        for s in &c.script {
            let Stmt::Export { decl, .. } = s else { continue };
            let Stmt::Func(f) = decl.as_ref() else { continue };
            if !f.name.as_ref().is_some_and(|n| n.name == "load") {
                continue;
            }
            let FnBody::Block(body) = &f.body else { continue };
            self.parallel_awaits(body);
        }
    }

    /// Runs of `const x = await f()` where no await reads a name bound earlier in the run (B1106).
    fn parallel_awaits(&mut self, body: &[Stmt]) {
        fn one(s: &Stmt) -> Option<(VarKind, String, &Expr, Span)> {
            let Stmt::Var(v) = s else { return None };
            let [d] = v.decls.as_slice() else { return None };
            let (Pat::Ident(id), Some(Expr::Await { arg, .. })) = (&d.pat, d.init.as_ref().map(Expr::unparen)) else { return None };
            Some((v.kind, id.name.clone(), arg.as_ref(), v.span))
        }
        let mut i = 0;
        while i < body.len() {
            let mut run = Vec::new();
            let mut names: Vec<String> = Vec::new();
            let mut j = i;
            while j < body.len() {
                let Some((k, name, arg, span)) = one(&body[j]) else { break };
                if run.first().is_some_and(|(k0, ..): &(VarKind, String, &Expr, Span)| *k0 != k) {
                    break;
                }
                let mut reads = Vec::new();
                crate::security::expr(arg, false, &[], &mut |x, _, _| {
                    if let Expr::Ident(i) = x {
                        reads.push(i.name.clone());
                    }
                });
                if reads.iter().any(|r| names.contains(r)) {
                    break;
                }
                names.push(name.clone());
                run.push((k, name, arg, span));
                j += 1;
            }
            if run.len() >= 2 {
                let kw = if run[0].0 == VarKind::Const { "const" } else { "let" };
                let vars: Vec<&str> = run.iter().map(|r| r.1.as_str()).collect();
                let calls: Vec<&str> = run.iter().map(|r| self.text(r.2.span())).collect();
                let edit = format!("{kw} [{}] = await Promise.all([{}])", vars.join(", "), calls.join(", "));
                let (start, end) = (run[0].3.start, run[run.len() - 1].3.end);
                let span = run[0].3;
                self.push_edits("B1106", span, format!("`load` awaits {} independent requests one after another", run.len()), format!("run them together: `{edit}`"), vec![(start, end, edit)]);
                i = j;
            } else {
                i += 1;
            }
        }
    }
}
