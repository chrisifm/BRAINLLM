//! B0746: literal absolute addresses a page calls, compared with the origins the app declares. The compiler only reads
//! literals, so a computed address is never guessed at, and server code is out of scope because CORS and the browser
//! `connect-src` do not apply to it.
use brain_parse::ast::*;
use brain_parse::{Diag, Severity, Span};
use std::path::Path;

/// `scheme://host[:port]` with `ws` mapped to `http`, without userinfo. `None` for anything that is not absolute.
pub fn absolute_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "http" | "ws" => "http",
        "https" | "wss" => "https",
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let host = host.strip_suffix(if scheme == "http" { ":80" } else { ":443" }).unwrap_or(&host);
    Some(format!("{scheme}://{host}"))
}

fn literal(e: &Expr) -> Option<&str> {
    match e.unparen() {
        Expr::Str { value, .. } => Some(value),
        Expr::Template { tag: None, quasis, .. } => quasis.first().map(String::as_str),
        _ => None,
    }
}

/// Server-only statements of a client file: `server function`, and the exported hooks that run on the server.
fn server_only(s: &Stmt) -> bool {
    match s {
        Stmt::Server { .. } => true,
        Stmt::Export { decl, .. } => match decl.as_ref() {
            Stmt::Func(f) => f.name.as_ref().is_some_and(|n| matches!(n.name.as_str(), "load" | "seo" | "head" | "paths")),
            _ => false,
        },
        _ => false,
    }
}

fn client_file(rel: &str, c: &Component) -> bool {
    let client_path = rel.ends_with(".brain") || rel.starts_with("api/") || rel.starts_with("channels/");
    client_path && !rel.starts_with("server/") && !c.meta.iter().any(|m| m.name == "server")
}

fn callee_name(e: &Expr) -> Option<&str> {
    let Expr::Ident(i) = e.unparen() else { return None };
    Some(i.name.as_str())
}

fn called_address<'a>(e: &'a Expr) -> Option<(&'a str, Span)> {
    let (name, args, span, ctor) = match e {
        Expr::Call { callee, args, span, .. } => (callee_name(callee)?, args, *span, false),
        Expr::New { callee, args, span } => (callee_name(callee)?, args, *span, true),
        _ => return None,
    };
    let wanted = if ctor { matches!(name, "WebSocket" | "EventSource") } else { matches!(name, "fetch" | "api") };
    if !wanted {
        return None;
    }
    Some((literal(args.first()?)?, span))
}

/// True when a `csp.connectSrc` list covers this address (exact origin, or a `*.` source covering a subdomain).
fn listed(address: &str, connect: &[String]) -> bool {
    let (scheme, rest) = address.split_once("://").unwrap_or(("", address));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").rsplit('@').next().unwrap_or("").to_ascii_lowercase();
    let scheme = scheme.to_ascii_lowercase();
    connect.iter().any(|source| {
        let Some((want_scheme, want_host)) = source.split_once("://") else { return false };
        // connect-src: an https source also covers wss on the same host, and the reverse does not hold
        let scheme_ok = want_scheme == scheme || (want_scheme, scheme.as_str()) == ("https", "wss") || (want_scheme, scheme.as_str()) == ("http", "ws");
        scheme_ok && match want_host.strip_prefix("*.") {
            Some(parent) => host.strip_suffix(parent).is_some_and(|left| left.ends_with('.') && left.len() > 1),
            None => want_host == host,
        }
    })
}

/// One diagnostic per literal call to an origin outside `declared`. A plain `http:`/`ws:` address is a Warning because
/// the production `connect-src` blocks it; `https:`/`wss:` is an Info because only that server can say whether it allows
/// us, and a Warning when `connect` (brain.json `csp.connectSrc`) is declared and does not cover it.
pub fn check(root: &Path, declared: &[String], connect: Option<&[String]>) -> Vec<Diag> {
    let mut out = Vec::new();
    for (rel, c) in crate::doctor::load_files(root) {
        if !client_file(&rel, &c) {
            continue;
        }
        let mut exprs = Vec::new();
        for s in c.script.iter().filter(|s| !server_only(s)) {
            crate::doctor::collect(std::slice::from_ref(s), &mut exprs);
        }
        for e in exprs {
            let Some((address, span)) = called_address(e) else { continue };
            let Some(origin) = absolute_origin(address) else { continue };
            if declared.contains(&origin) || connect.is_some_and(|list| listed(address, list)) {
                continue;
            }
            let (msg, fix, level) = if let (Some(_), false) = (connect, origin.starts_with("http://")) {
                (
                    format!("this page calls {origin}, which is outside `csp.connectSrc`, so the production `connect-src` blocks it and the browser reports only a network error"),
                    format!("add {origin} to `csp.connectSrc` in brain.json, or call it from a server function. Run brain explain B0746"),
                    Severity::Warning,
                )
            } else if origin.starts_with("http://") {
                (
                    format!("this page calls {origin}, which is plain http and not a declared origin; the production `connect-src` blocks it and the browser reports only a network error"),
                    format!("use a relative path through your own server, or the https address of {origin}. Run brain explain B0746"),
                    Severity::Warning,
                )
            } else {
                (
                    format!("this page calls {origin}, which is not a declared origin; it only works if that server sends CORS headers for this site, and this check cannot prove it does"),
                    format!("call it from a server function, or confirm that {origin} allows this site. Run brain explain B0746"),
                    Severity::Info,
                )
            };
            let mut diag = Diag::new("B0746", &rel, span, msg, fix);
            diag.level = level;
            out.push(diag);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(files: &[(&str, &str)]) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("brain-origins-{}-{n}", std::process::id()));
        for (rel, src) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, src).unwrap();
        }
        root
    }

    #[test]
    fn normalizes_what_a_browser_would_send() {
        assert_eq!(absolute_origin("wss://Shop.Example.test:443/live").as_deref(), Some("https://shop.example.test"));
        assert_eq!(absolute_origin("https://user:pass@shop.example.test/a").as_deref(), Some("https://shop.example.test"));
        assert_eq!(absolute_origin("/api/tasks"), None);
        assert_eq!(absolute_origin("mailto:a@shop.example.test"), None);
    }

    #[test]
    fn undeclared_literal_calls_are_reported_by_their_scheme() {
        let root = app(&[("views/shop.brain", "<script>\nasync function a() { return fetch('http://api.shop.example.test/items') }\nasync function b() { return fetch('https://pay.example.test/x') }\n</script>\n<p>Shop</p>\n")]);
        let found = check(&root, &[], None);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(found.len(), 2, "{found:?}");
        let plain = found.iter().find(|d| d.msg.contains("http://api.shop.example.test")).unwrap();
        assert_eq!(plain.level, Severity::Warning);
        assert_eq!((plain.file.as_str(), plain.line), ("views/shop.brain", 2));
        assert!(plain.fix.contains("brain explain B0746"));
        assert_eq!(found.iter().find(|d| d.msg.contains("https://pay.example.test")).unwrap().level, Severity::Info);
    }

    #[test]
    fn declared_relative_computed_and_server_calls_stay_silent() {
        let root = app(&[
            ("views/a.brain", "<script>\nasync function a(u) { await fetch('/api/tasks'); await fetch(u); await fetch(`${u}/x`); return fetch('https://api.shop.example.test/x') }\n</script>\n<p>A</p>\n"),
            ("views/b.brain", "<script>\nexport async function load() { return fetch('http://internal.example.test/x') }\nserver function s() { return fetch('http://internal.example.test/y') }\n</script>\n<p>B</p>\n"),
            ("server/mail.jsbrain", "<meta server>\nexport async function send() { return fetch('https://mail.example.test/x') }\n"),
            ("lib/util.jsbrain", "export function get() { return fetch('https://other.example.test/x') }\n"),
        ]);
        let found = check(&root, &["https://api.shop.example.test".to_string()], None);
        std::fs::remove_dir_all(&root).ok();
        assert!(found.is_empty(), "{found:?}");
    }
}

/// The literal address of a template attribute: its whole text, or the text before the first interpolation.
fn attr_address(value: &AttrValue) -> Option<&str> {
    match value {
        AttrValue::Text(text) => Some(text),
        AttrValue::Expr(e) => literal(e),
        AttrValue::Mixed(parts) => match parts.first()? {
            AttrPart::Text(text) => Some(text),
            AttrPart::Expr(_) => None,
        },
        AttrValue::None => None,
    }
}

fn embeds_in(nodes: &[Node], out: &mut Vec<(&'static str, String, Span)>) {
    for node in nodes {
        match node {
            Node::Element(e) => {
                let kind = if e.tag.eq_ignore_ascii_case("iframe") { Some("frame") } else if e.tag.eq_ignore_ascii_case("script") { Some("script") } else { None };
                if let (Some(kind), Some(attr)) = (kind, e.attrs.iter().find(|a| a.name == "src")) {
                    if let Some(address) = attr_address(&attr.value) {
                        out.push((kind, address.to_string(), attr.span));
                    }
                }
                embeds_in(&e.children, out);
            }
            Node::ComponentRef(r) => embeds_in(&r.children, out),
            Node::If(b) => {
                for branch in &b.branches {
                    embeds_in(&branch.body, out);
                }
                if let Some(otherwise) = &b.otherwise {
                    embeds_in(otherwise, out);
                }
            }
            Node::Each(b) => embeds_in(&b.body, out),
            Node::SlotScope(b) => embeds_in(&b.body, out),
            Node::Await(b) => {
                embeds_in(&b.pending, out);
                for branch in [&b.then, &b.catch].into_iter().flatten() {
                    embeds_in(&branch.body, out);
                }
            }
            Node::Text(_) | Node::Interp(_) | Node::Slot(_) => {}
        }
    }
}

/// B0811: a literal `<iframe src>` or `<script src>` the production policy blocks, which only the browser console reports.
pub fn check_embeds(root: &Path, frames: &[String], scripts: &[String]) -> Vec<Diag> {
    let mut out = Vec::new();
    for (rel, c) in crate::doctor::load_files(root) {
        let mut found = Vec::new();
        embeds_in(&c.template, &mut found);
        for (kind, address, span) in found {
            let Some(origin) = absolute_origin(&address) else { continue };
            let (list, key, directive) = if kind == "frame" { (frames, "frameSrc", "frame-src") } else { (scripts, "scriptSrc", "script-src") };
            if listed(&address, list) {
                continue;
            }
            let mut diag = Diag::new(
                "B0811",
                &rel,
                span,
                format!("this {kind} loads {origin}, which is outside `csp.{key}`, so the production `{directive}` blocks it"),
                format!("add {origin} to `csp.{key}` in brain.json. Run brain explain B0811"),
            );
            diag.level = Severity::Warning;
            out.push(diag);
        }
    }
    out
}
