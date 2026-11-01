//! WebMCP agent tools (INTERNALS §32):
//! `<meta tools="add_task: Add a task (text:str!, due:str); clear_done: Remove completed tasks">` + a function of
//! the same name, and declarative `<form tool="search_products" tool-description="…">`.
//! - B1701 a tool without a same-named function in the script.
//! - B1702 a tool name that is not snake_case, or repeated in the file.
//! - B1703 a parameter without a type.
//! - B1704 the function destructures other parameters than the tool declares (#106): an undeclared one never
//!   arrives (agents send only what the schema lists) and a declared one the function ignores misleads the agent.
//! - B1705 a tool that deletes, pays or administers, in a file with no confirmation step.
//! - B1706 a warning: a public form with a real action (search, filter, apply, subscribe) and no `tool=`.
//! - B1707 a warning: a tool description too short to choose by, or missing on a form.
//!
//! WebMCP is a proposal, not on the W3C Standards Track, so authors write `tool=` and the compiler owns emitted names.

use crate::Checker;
use brain_parse::ast::*;
use std::collections::HashSet;

pub const PARAM_TYPES: &[&str] = &["str", "num", "bool", "list", "obj", "email", "url", "date", "any"];

#[derive(Clone, Debug, PartialEq)]
pub struct ToolParam {
    pub name: String,
    /// `None` when written without `:type` (B1703).
    pub ty: Option<String>,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub params: Vec<ToolParam>,
}

/// Parses the `tools` mini-language: `name: description (p:type!, q:type)`, tools separated by `;`.
pub fn parse_tools(v: &str) -> Vec<Tool> {
    let mut out = Vec::new();
    for entry in v.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let (name, rest) = entry.split_once(':').unwrap_or((entry, ""));
        let rest = rest.trim();
        let (description, params) = match (rest.rfind('('), rest.ends_with(')')) {
            (Some(i), true) => (rest[..i].trim(), &rest[i + 1..rest.len() - 1]),
            _ => (rest, ""),
        };
        let params = params
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| {
                let required = p.ends_with('!');
                let p = p.trim_end_matches('!');
                let (n, t) = match p.split_once(':') {
                    Some((n, t)) => (n.trim(), Some(t.trim().to_string()).filter(|t| !t.is_empty())),
                    None => (p.trim(), None),
                };
                ToolParam { name: n.to_string(), ty: t, required }
            })
            .collect();
        out.push(Tool { name: name.trim().to_string(), description: description.to_string(), params });
    }
    out
}

pub fn is_snake(n: &str) -> bool {
    !n.is_empty() && n.starts_with(|c: char| c.is_ascii_lowercase()) && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') && !n.contains("__") && !n.ends_with('_')
}

fn plain<'a>(e: &'a Element, name: &str) -> Option<&'a Attr> {
    e.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == name)
}
fn text_of(a: &Attr) -> Option<&str> {
    match &a.value {
        AttrValue::Text(t) => Some(t),
        _ => None,
    }
}
/// Every `<form>` of a template, through control flow and component children.
pub(crate) fn forms(nodes: &[Node]) -> Vec<&Element> {
    fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Element>) {
        for n in nodes {
            match n {
                Node::Element(e) => {
                    if e.tag == "form" {
                        out.push(e);
                    }
                    walk(&e.children, out);
                }
                Node::ComponentRef(r) => walk(&r.children, out),
                Node::If(b) => {
                    for br in &b.branches {
                        walk(&br.body, out);
                    }
                    if let Some(o) = &b.otherwise {
                        walk(o, out);
                    }
                }
                Node::Each(b) => walk(&b.body, out),
                Node::SlotScope(b) => walk(&b.body, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(nodes, &mut out);
    out
}
/// The tool names a component registers: `<meta tools>` entries and `<form tool>`.
pub(crate) fn names(c: &Component) -> Vec<String> {
    let declared = c.meta.iter().filter(|m| m.name == "tools").flat_map(|m| parse_tools(m.value.as_deref().unwrap_or(""))).map(|t| t.name);
    declared.chain(forms(&c.template).into_iter().filter_map(|e| plain(e, "tool").and_then(text_of)).map(str::to_string)).collect()
}
/// The real action a form performs, read from `role="search"`, a search field or its submit label. A login, a contact
/// form or a form of unknown purpose answers `None`: exposing those is the author's call.
fn public_action(e: &Element) -> Option<&'static str> {
    const WORDS: &[&str] = &["search", "filter", "apply", "subscribe"];
    if plain(e, "role").and_then(text_of) == Some("search") {
        return Some("search");
    }
    fn walk(nodes: &[Node], inside_button: bool) -> Option<&'static str> {
        nodes.iter().find_map(|n| match n {
            Node::Element(e) if e.tag == "input" && plain(e, "type").and_then(text_of) == Some("search") => Some("search"),
            Node::Element(e) => walk(&e.children, inside_button || e.tag == "button"),
            Node::ComponentRef(r) if r.attrs.iter().any(|a| a.name == "type" && text_of(a) == Some("search")) => Some("search"),
            Node::ComponentRef(r) => walk(&r.children, inside_button || r.name == "Button"),
            Node::Text(t) if inside_button => {
                let first = t.value.split_whitespace().next()?.to_ascii_lowercase();
                WORDS.iter().find(|w| first == **w).copied()
            }
            _ => None,
        })
    }
    walk(&e.children, false)
}
/// A form behind a login has no outside agent to serve, so B1706 stays out of it.
fn private_area(file: &str) -> bool {
    // `ui/` is a reusable kit: it cannot name one tool, since the page that mounts it decides what the form acts on.
    ["ui", "admin", "account", "dashboard", "settings"].iter().any(|word| file.split(['/', '\\']).any(|part| part == *word))
}
/// First word of the name says the tool deletes, pays or administers, or the file sits under an admin folder.
fn consequential(name: &str, file: &str) -> bool {
    const VERBS: &[&str] = &["delete", "remove", "destroy", "purge", "wipe", "pay", "charge", "refund", "checkout", "purchase", "admin"];
    VERBS.contains(&name.split('_').next().unwrap_or("")) || file.split(['/', '\\']).any(|part| part == "admin")
}
/// Why a description cannot help an agent choose, if it cannot: too short, or only the tool's own name again.
fn weak_description(name: &str, description: &str) -> Option<&'static str> {
    let words: Vec<String> = description.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase()).filter(|w| !w.is_empty()).collect();
    let own: Vec<&str> = name.split('_').collect();
    if words.iter().all(|w| own.contains(&w.as_str()) || w == "tool") {
        return Some("only repeats its name");
    }
    (words.len() < 3).then_some("is too short")
}

impl Checker<'_> {
    pub(crate) fn tools(&mut self) {
        let c = self.c;
        let mut seen = HashSet::new();
        let fns: HashSet<String> = crate::scope::top_bindings(c).into_iter().filter(|(_, k)| matches!(k, crate::scope::Kind::Func | crate::scope::Kind::Const | crate::scope::Kind::Local)).map(|(n, _)| n).collect();
        for m in c.meta.iter().filter(|m| m.name == "tools") {
            for t in parse_tools(m.value.as_deref().unwrap_or("")) {
                if !is_snake(&t.name) {
                    self.push("B1702", m.span, format!("tool name `{}` is not snake_case", t.name), format!("rename it to `{}` (and its function)", snake(&t.name)));
                } else if !seen.insert(t.name.clone()) {
                    self.push("B1702", m.span, format!("tool `{}` is declared twice", t.name), "give each tool a unique name on the page");
                }
                if !fns.contains(&t.name) {
                    self.push("B1701", m.span, format!("tool `{}` has no function `{0}` in the <script>", t.name), format!("write `const {} = async ({{ {} }}) => {{ … }}` — it runs when an agent calls the tool", t.name, t.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")));
                }
                for p in t.params.iter().filter(|p| p.ty.as_deref().is_none_or(|ty| !PARAM_TYPES.contains(&ty))) {
                    self.push("B1703", m.span, format!("parameter `{}` of tool `{}` has no valid type", p.name, t.name), format!("write `{}:str` (types: {}; `!` = required)", p.name, PARAM_TYPES.join(" ")));
                }
                if t.description.is_empty() {
                    self.push("B1703", m.span, format!("tool `{}` has no description", t.name), format!("write `{}: what it does (params)` — agents choose tools by their description", t.name));
                } else if let Some(why) = weak_description(&t.name, &t.description) {
                    self.warn("B1707", m.span, format!("the description of tool `{}` {why}", t.name), "say what it does, what it needs and what it returns or changes, in one sentence");
                }
                self.needs_confirmation(&t.name, m.span);
                if let Some((read, span)) = fns.contains(&t.name).then(|| read_params(c, &t.name)).flatten() {
                    for (p, at) in read.iter().filter(|(p, _)| !t.params.iter().any(|d| d.name == *p)) {
                        self.push("B1704", *at, format!("function `{}` reads `{p}`, which tool `{0}` does not declare (agents never send it)", t.name), format!("declare it in <meta tools> (`{p}:str`; types: {}) or remove it from the function's parameters", PARAM_TYPES.join(" ")));
                    }
                    for d in t.params.iter().filter(|d| !read.iter().any(|(p, _)| *p == d.name)) {
                        self.push("B1704", span, format!("tool `{}` declares `{}`, which function `{0}` never reads", t.name, d.name), format!("destructure it (`({{ {} }})`) or remove `{}` from <meta tools>", t.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "), d.name));
                    }
                }
            }
        }
        for e in forms(&c.template) {
            let Some(a) = plain(e, "tool") else {
                if let Some(action) = public_action(e).filter(|_| !private_area(&c.file)) {
                    self.warn("B1706", e.span, format!("this public form performs `{action}` and agents cannot call it"), format!("add `tool=\"{action}_...\"` and `tool-description=\"...\"`, naming what it acts on (`brain guide webmcp`)"));
                }
                continue;
            };
            let name = text_of(a).unwrap_or("").to_string();
            let span = a.span;
            if !is_snake(&name) {
                self.push("B1702", span, format!("tool name `{name}` is not snake_case"), format!("write `tool=\"{}\"`", snake(&name)));
            } else if !seen.insert(name.clone()) {
                self.push("B1702", span, format!("tool `{name}` is declared twice"), "give each tool a unique name on the page");
            }
            match plain(e, "tool-description").and_then(text_of) {
                None => self.warn("B1707", span, format!("tool `{name}` has no `tool-description`"), "add one sentence: what it does, what it needs and what it returns or changes"),
                Some(text) => {
                    if let Some(why) = weak_description(&name, text) {
                        self.warn("B1707", span, format!("the description of tool `{name}` {why}"), "say what it does, what it needs and what it returns or changes, in one sentence");
                    }
                }
            }
            self.needs_confirmation(&name, span);
        }
    }
    fn warn(&mut self, code: &'static str, span: brain_parse::Span, msg: impl Into<String>, fix: impl Into<String>) {
        self.push(code, span, msg, fix);
        if let Some(last) = self.diags.last_mut() {
            last.level = brain_parse::Severity::Warning;
        }
    }
    /// A tool runs with the page's permissions, so a consequential one needs a person in the loop. The file is searched
    /// for any sign of one (`confirm(`, a `<dialog>`, a Confirm component, a description that says it asks).
    fn needs_confirmation(&mut self, name: &str, span: brain_parse::Span) {
        let src = self.c.src.to_ascii_lowercase();
        if consequential(name, &self.c.file) && !src.contains("confirm") && !src.contains("<dialog") {
            self.push("B1705", span, format!("tool `{name}` can delete, pay or administer, and this file has no confirmation step"), "ask before acting (`confirm(...)` or a `<dialog>`) and say so in the description, or do not expose this action to agents");
        }
    }
}

/// The keys a tool function destructures from its argument object, with the span of each, plus the span of the
/// parameter list (the function's when it takes none). `None` when the checker cannot tell and trusts the author: a
/// whole-object parameter (`(args) =>`), a rest element, a computed key, or no plain function of that name.
fn read_params(c: &Component, name: &str) -> Option<(Vec<(String, brain_parse::Span)>, brain_parse::Span)> {
    let f = c.script.iter().find_map(|s| match s {
        Stmt::Func(f) if f.name.as_ref().is_some_and(|n| n.name == name) => Some(f),
        Stmt::Var(v) => v.decls.iter().find_map(|d| match (&d.pat, d.init.as_ref().map(Expr::unparen)) {
            (Pat::Ident(i), Some(Expr::Func(f))) if i.name == name => Some(&**f),
            _ => None,
        }),
        _ => None,
    })?;
    let Some(first) = f.params.first() else { return Some((Vec::new(), f.span)) };
    let span = first.span();
    let Pat::Object { props, rest: None, .. } = (match first {
        Pat::Default { pat, .. } => pat,
        p => p,
    }) else {
        return None;
    };
    let keys = props
        .iter()
        .map(|p| match &p.key {
            PropKey::Ident(i) => Some((i.name.clone(), i.span)),
            PropKey::Str { value, span, .. } => Some((value.clone(), *span)),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some((keys, span))
}

fn snake(n: &str) -> String {
    let mut s = String::new();
    for (i, ch) in n.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                s.push('_');
            }
            s.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_alphanumeric() {
            s.push(ch);
        } else {
            s.push('_');
        }
    }
    s.trim_matches('_').to_string()
}
