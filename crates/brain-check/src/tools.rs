//! WebMCP agent tools (INTERNALS §32):
//! `<meta tools="add_task: Add a task (text:str!, due:str); clear_done: Remove completed tasks">` + a function of
//! the same name, and declarative `<form tool="search_products" tool-description="…">`.
//! - B1701 a tool without a same-named function in the script.
//! - B1702 a tool name that is not snake_case, or repeated in the file.
//! - B1703 a parameter without a type.
//! - B1704 the function destructures other parameters than the tool declares (#106): an undeclared one never
//!   arrives (agents send only what the schema lists) and a declared one the function ignores misleads the agent.

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

fn form_tools(nodes: &[Node], out: &mut Vec<(String, brain_parse::Span)>) {
    for n in nodes {
        match n {
            Node::Element(e) => {
                if e.tag == "form"
                    && let Some(a) = e.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == "tool")
                {
                    let name = match &a.value {
                        AttrValue::Text(t) => t.clone(),
                        _ => String::new(),
                    };
                    out.push((name, a.span));
                }
                form_tools(&e.children, out);
            }
            Node::ComponentRef(r) => form_tools(&r.children, out),
            Node::If(b) => {
                for br in &b.branches {
                    form_tools(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    form_tools(o, out);
                }
            }
            Node::Each(b) => form_tools(&b.body, out),
            Node::SlotScope(b) => form_tools(&b.body, out),
            _ => {}
        }
    }
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
                }
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
        let mut forms = Vec::new();
        form_tools(&c.template, &mut forms);
        for (name, span) in forms {
            if !is_snake(&name) {
                self.push("B1702", span, format!("tool name `{name}` is not snake_case"), format!("write `tool=\"{}\"`", snake(&name)));
            } else if !seen.insert(name.clone()) {
                self.push("B1702", span, format!("tool `{name}` is declared twice"), "give each tool a unique name on the page");
            }
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
