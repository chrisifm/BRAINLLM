//! Per-file language rules (INTERNALS §37); the app-wide ones are in [`crate::i18n::check_locales`].

use crate::i18n::{insert_edit, key_scope, key_words, logical, missing_key, translatable, valid_locale, LocaleFile};
use crate::security::{expr as visit, walk};
use crate::Checker;
use brain_parse::ast::*;
use brain_parse::{Diag, Span};
use std::collections::{BTreeMap, BTreeSet};

/// `t('key', { a, b: x })` calls: (key, variable names passed, call span). Dynamic keys are skipped.
fn t_calls(c: &Component) -> Vec<(String, Option<BTreeSet<String>>, Span)> {
    let mut out = Vec::new();
    let mut take = |e: &Expr, _: bool, _: &[String]| {
        if let Expr::Call { callee, args, span, .. } = e
            && matches!(callee.unparen(), Expr::Ident(i) if i.name == "t")
            && let Some(Expr::Str { value, .. }) = args.first().map(Expr::unparen)
        {
            let vars = args.get(1).map(|a| match a.unparen() {
                Expr::Object { props, .. } => props
                    .iter()
                    .filter_map(|p| match p {
                        ObjProp::Shorthand(i) => Some(i.name.clone()),
                        ObjProp::KeyValue { key: PropKey::Ident(k), .. } => Some(k.name.clone()),
                        ObjProp::KeyValue { key: PropKey::Str { value, .. }, .. } => Some(value.clone()),
                        _ => None,
                    })
                    .collect(),
                // A variable holding the object: unknown fields, trust it.
                _ => BTreeSet::from(["*".to_string()]),
            });
            out.push((value.clone(), vars, *span));
        }
    };
    walk(&c.script, false, &mut take);
    fn nodes(ns: &[Node], f: &mut dyn FnMut(&Expr, bool, &[String])) {
        for n in ns {
            match n {
                Node::Interp(i) => visit(&i.expr, false, &[], f),
                Node::Element(Element { attrs, children, .. }) | Node::ComponentRef(ComponentRef { attrs, children, .. }) => {
                    for a in attrs {
                        match &a.value {
                            AttrValue::Expr(x) => visit(x, false, &[], f),
                            AttrValue::Mixed(parts) => {
                                for p in parts {
                                    if let AttrPart::Expr(x) = p {
                                        visit(x, false, &[], f);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    nodes(children, f);
                }
                Node::If(b) => {
                    for br in &b.branches {
                        visit(&br.test, false, &[], f);
                        nodes(&br.body, f);
                    }
                    if let Some(o) = &b.otherwise {
                        nodes(o, f);
                    }
                }
                Node::Each(b) => {
                    visit(&b.list, false, &[], f);
                    nodes(&b.body, f);
                    if let Some(e) = &b.empty {
                        nodes(e, f);
                    }
                }
                Node::SlotScope(b) => nodes(&b.body, f),
                Node::Slot(s) => {
                    for a in &s.attrs {
                        match &a.value {
                            AttrValue::Expr(x) => visit(x, false, &[], f),
                            AttrValue::Mixed(parts) => {
                                for p in parts {
                                    if let AttrPart::Expr(x) = p {
                                        visit(x, false, &[], f);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Node::Await(a) => {
                    nodes(&a.pending, f);
                    for br in [&a.then, &a.catch].into_iter().flatten() {
                        nodes(&br.body, f);
                    }
                }
                _ => {}
            }
        }
    }
    nodes(&c.template, &mut take);
    nodes(&c.head, &mut take);
    for m in &c.meta {
        if let Some(x) = &m.expr {
            visit(x, false, &[], &mut take);
        }
    }
    out
}

/// One fixed text that B1403 moves to a key: the source range it replaces, the message with `{name}` placeholders and
/// how the `t()` call is written back.
struct Found {
    span: Span,
    msg: String,
    shape: Shape,
    vars: Vec<Var>,
    /// Key words instead of the message's first words (`meta_title`).
    words: Option<&'static str>,
}

enum Shape {
    /// `{t('k')}` in markup.
    Markup,
    /// `name={t('k')}` replacing a whole attribute.
    Attr(String),
    /// `t('k')` inside an expression.
    Expr,
}

/// A placeholder: its name, the expression it takes, and fixed texts inside that expression (`{open ? 'Yes' : 'No'}`).
struct Var {
    name: String,
    span: Span,
    inner: Vec<Found>,
}

/// Attributes whose text a person reads.
const TEXT_ATTRS: &[&str] = &["placeholder", "title", "alt", "aria-label", "aria-description", "aria-placeholder", "aria-roledescription", "aria-valuetext", "label", "description", "help", "hint", "caption", "heading", "message", "empty"];

/// Fixed texts of a component, in source order: `<meta seo>` title and description, text nodes merged with the
/// interpolations around them into one sentence, readable attributes, and string literals in the value positions of
/// markup expressions (`a ? 'x' : 'y'`, `a || 'x'`, `'x ' + a`, `` `x ${a}` ``).
fn fixed_texts(c: &Component) -> Vec<Found> {
    let mut out = Vec::new();
    for m in c.meta.iter().skip_while(|m| m.name != "seo").skip(1) {
        if matches!(m.name.as_str(), "props" | "emits" | "expose" | "export" | "tools" | "server" | "fallback" | "cache" | "http" | "client" | "budget" | "route" | "seo") {
            break;
        }
        let words = match m.name.as_str() {
            "title" => "meta_title",
            "description" => "meta_description",
            _ => continue,
        };
        match (&m.value, &m.expr) {
            (Some(v), None) if translatable(v) => out.push(Found { span: m.span, msg: collapse(v), shape: Shape::Attr(m.name.clone()), vars: Vec::new(), words: Some(words) }),
            (_, Some(x)) => lits(x, &c.src, &mut out),
            _ => {}
        }
    }
    nodes(&c.template, &c.src, &mut out);
    out
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn nodes(ns: &[Node], src: &str, out: &mut Vec<Found>) {
    let mut i = 0;
    while i < ns.len() {
        match &ns[i] {
            Node::Text(_) | Node::Interp(Interp { html: false, .. }) => {
                let j = i + ns[i..].iter().take_while(|n| matches!(n, Node::Text(_) | Node::Interp(Interp { html: false, .. }))).count();
                run(&ns[i..j], src, out);
                i = j;
                continue;
            }
            Node::Element(e) => {
                attrs(&e.tag, &e.attrs, src, out);
                if !matches!(e.tag.as_str(), "code" | "pre" | "kbd" | "samp" | "svg" | "math" | "script" | "style") {
                    nodes(&e.children, src, out);
                }
            }
            Node::ComponentRef(r) => {
                attrs(&r.name, &r.attrs, src, out);
                nodes(&r.children, src, out);
            }
            Node::If(b) => {
                for br in &b.branches {
                    nodes(&br.body, src, out);
                }
                if let Some(o) = &b.otherwise {
                    nodes(o, src, out);
                }
            }
            Node::Each(b) => {
                nodes(&b.body, src, out);
                if let Some(e) = &b.empty {
                    nodes(e, src, out);
                }
            }
            Node::SlotScope(b) => nodes(&b.body, src, out),
            Node::Await(a) => {
                nodes(&a.pending, src, out);
                for br in [&a.then, &a.catch].into_iter().flatten() {
                    nodes(&br.body, src, out);
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// Sibling text and interpolations: one message when any text is translatable (`Signed in as {email}.`), else only
/// the literals inside the interpolations.
fn run(ns: &[Node], src: &str, out: &mut Vec<Found>) {
    if !ns.iter().any(|n| matches!(n, Node::Text(t) if translatable(&t.value))) {
        for n in ns {
            if let Node::Interp(i) = n {
                lits(&i.expr, src, out);
            }
        }
        return;
    }
    // The range from the first non-blank character to the last one.
    let first = ns.iter().position(|n| !matches!(n, Node::Text(t) if t.value.trim().is_empty())).unwrap_or(0);
    let last = ns.iter().rposition(|n| !matches!(n, Node::Text(t) if t.value.trim().is_empty())).unwrap_or(0);
    let ns = &ns[first..=last];
    let start = match &ns[0] {
        Node::Text(t) => t.span.start + (t.value.len() - t.value.trim_start().len()) as u32,
        n => node_span(n).start,
    };
    let end = match &ns[ns.len() - 1] {
        Node::Text(t) => t.span.start + t.value.trim_end().len() as u32,
        n => node_span(n).end,
    };
    let (line, col) = match &ns[0] {
        Node::Text(t) => (t.span.line, t.span.col),
        n => (node_span(n).line, node_span(n).col),
    };
    let mut msg = String::new();
    let mut vars: Vec<Var> = Vec::new();
    for n in ns {
        match n {
            Node::Text(t) => msg.push_str(&t.value),
            Node::Interp(i) => {
                let name = var(&mut vars, &i.expr, src);
                msg.push_str(&format!("{{{name}}}"));
            }
            _ => {}
        }
    }
    out.push(Found { span: Span { start, end, line, col }, msg: collapse(&msg), shape: Shape::Markup, vars, words: None });
}

fn node_span(n: &Node) -> Span {
    match n {
        Node::Text(t) => t.span,
        Node::Interp(i) => i.span,
        _ => Span::default(),
    }
}

/// Adds a placeholder for `e` (or reuses one with the same source) and returns its name.
fn var(vars: &mut Vec<Var>, e: &Expr, src: &str) -> String {
    let span = e.span();
    let text = &src[span.start as usize..span.end as usize];
    if let Some(v) = vars.iter().find(|v| &src[v.span.start as usize..v.span.end as usize] == text) {
        return v.name.clone();
    }
    let base = var_name(e);
    let mut name = base.clone();
    let mut n = 2;
    while vars.iter().any(|v| v.name == name) {
        name = format!("{base}{n}");
        n += 1;
    }
    let mut inner = Vec::new();
    lits(e, src, &mut inner);
    vars.push(Var { name: name.clone(), span, inner });
    name
}

fn var_name(e: &Expr) -> String {
    let n = match e.unparen() {
        Expr::Ident(i) => i.name.clone(),
        Expr::Member { prop: MemberProp::Name(i), .. } => i.name.clone(),
        Expr::Call { callee, .. } => return var_name(callee),
        _ => String::new(),
    };
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || n.starts_with(|c: char| c.is_ascii_digit()) { "value".into() } else { n }
}

fn attrs(tag: &str, attrs: &[Attr], src: &str, out: &mut Vec<Found>) {
    let submit = tag == "input" && attrs.iter().any(|a| a.name == "type" && matches!(&a.value, AttrValue::Text(t) if matches!(t.as_str(), "submit" | "button" | "reset")));
    for a in attrs.iter().filter(|a| a.kind == AttrKind::Plain) {
        if !(TEXT_ATTRS.contains(&a.name.as_str()) || (submit && a.name == "value")) {
            continue;
        }
        match &a.value {
            AttrValue::Text(t) if translatable(t) => out.push(Found { span: a.span, msg: collapse(t), shape: Shape::Attr(a.name.clone()), vars: Vec::new(), words: None }),
            AttrValue::Mixed(parts) if parts.iter().any(|p| matches!(p, AttrPart::Text(t) if translatable(t))) => {
                let mut msg = String::new();
                let mut vars = Vec::new();
                for p in parts {
                    match p {
                        AttrPart::Text(t) => msg.push_str(t),
                        AttrPart::Expr(x) => {
                            let name = var(&mut vars, x, src);
                            msg.push_str(&format!("{{{name}}}"));
                        }
                    }
                }
                out.push(Found { span: a.span, msg: collapse(&msg), shape: Shape::Attr(a.name.clone()), vars, words: None });
            }
            AttrValue::Expr(x) => lits(x, src, out),
            AttrValue::Mixed(parts) => {
                for p in parts {
                    if let AttrPart::Expr(x) = p {
                        lits(x, src, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// String literals a person reads, in the value positions of an expression: both branches of `?:`, the fallback of
/// `||`/`??`, and a `+` concatenation or template literal with text (one message with placeholders).
fn lits(e: &Expr, src: &str, out: &mut Vec<Found>) {
    match e.unparen() {
        Expr::Str { value, span, .. } if translatable(value) => out.push(Found { span: *span, msg: value.clone(), shape: Shape::Expr, vars: Vec::new(), words: None }),
        Expr::Template { tag: None, quasis, exprs, span } if quasis.iter().any(|q| translatable(q)) && !quasis.iter().any(|q| q.contains('\\')) => {
            let mut msg = String::new();
            let mut vars = Vec::new();
            for (i, q) in quasis.iter().enumerate() {
                msg.push_str(q);
                if let Some(x) = exprs.get(i) {
                    let name = var(&mut vars, x, src);
                    msg.push_str(&format!("{{{name}}}"));
                }
            }
            out.push(Found { span: *span, msg, shape: Shape::Expr, vars, words: None });
        }
        Expr::Cond { cons, alt, .. } => {
            lits(cons, src, out);
            lits(alt, src, out);
        }
        Expr::Binary { op: "||" | "??", right, .. } => lits(right, src, out),
        Expr::Binary { op: "+", span, .. } => {
            let mut parts = Vec::new();
            fn flat<'a>(e: &'a Expr, parts: &mut Vec<&'a Expr>) {
                match e.unparen() {
                    Expr::Binary { op: "+", left, right, .. } => {
                        flat(left, parts);
                        flat(right, parts);
                    }
                    x => parts.push(x),
                }
            }
            flat(e.unparen(), &mut parts);
            if !parts.iter().any(|p| matches!(p, Expr::Str { value, .. } if translatable(value))) {
                return;
            }
            let mut msg = String::new();
            let mut vars = Vec::new();
            for p in parts {
                match p {
                    Expr::Str { value, .. } => msg.push_str(value),
                    x => {
                        let name = var(&mut vars, x, src);
                        msg.push_str(&format!("{{{name}}}"));
                    }
                }
            }
            out.push(Found { span: *span, msg, shape: Shape::Expr, vars, words: None });
        }
        _ => {}
    }
}

/// Keys of one file: `<scope>.<words>`, reused for the same text, suffixed `_2`, `_3`… for another text, and reused
/// when the default locale file already has the key with the same text.
struct Keys<'a> {
    scope: String,
    default: Option<&'a LocaleFile>,
    mine: BTreeMap<String, String>,
}

impl Keys<'_> {
    /// (key, whether it is new to the locale files).
    fn alloc(&mut self, words: &str, text: &str) -> (String, bool) {
        let base = format!("{}.{words}", self.scope);
        for n in 1.. {
            let k = if n == 1 { base.clone() } else { format!("{base}_{n}") };
            if let Some(t) = self.mine.get(&k) {
                if t == text {
                    return (k, false);
                }
                continue;
            }
            if let Some((t, _, _)) = self.default.and_then(|f| f.entries.get(&k)) {
                if t.as_plain() == Some(text) {
                    self.mine.insert(k.clone(), text.to_string());
                    return (k, false);
                }
                continue;
            }
            self.mine.insert(k.clone(), text.to_string());
            return (k, true);
        }
        unreachable!()
    }

    /// The `t()` call for `f` (keys allocated depth first) and the new (key, text) pairs it needs.
    fn render(&mut self, f: &Found, src: &str, new: &mut Vec<(String, String)>) -> String {
        let words = f.words.map(str::to_string).unwrap_or_else(|| key_words(&f.msg, 4));
        let (key, fresh) = self.alloc(&words, &f.msg);
        if fresh {
            new.push((key.clone(), f.msg.clone()));
        }
        let mut args = Vec::new();
        for v in &f.vars {
            let mut text = src[v.span.start as usize..v.span.end as usize].to_string();
            let mut inner: Vec<(u32, u32, String)> = v.inner.iter().map(|i| (i.span.start, i.span.end, self.render(i, src, new))).collect();
            inner.sort_by_key(|x| std::cmp::Reverse(x.0));
            for (a, b, r) in inner {
                text.replace_range((a - v.span.start) as usize..(b - v.span.start) as usize, &r);
            }
            args.push(if text == v.name { text } else { format!("{}: {text}", v.name) });
        }
        let call = if args.is_empty() { format!("t('{key}')") } else { format!("t('{key}', {{ {} }})", args.join(", ")) };
        match &f.shape {
            Shape::Markup => format!("{{{call}}}"),
            Shape::Attr(name) => format!("{name}={{{call}}}"),
            Shape::Expr => call,
        }
    }
}
/// `<meta route es="/es/about-us" pt="/pt/about-us">` → `[("es", "/es/about-us"), …]`.
pub fn route_slugs(c: &Component) -> Vec<(String, String, Span)> {
    let mut out = Vec::new();
    let mut on = false;
    for m in &c.meta {
        if m.name == "route" {
            on = true;
            continue;
        }
        if on {
            if matches!(m.name.as_str(), "props" | "seo" | "emits" | "expose" | "export" | "tools" | "server" | "fallback" | "cache" | "http" | "client" | "budget" | "route") {
                break;
            }
            out.push((m.name.clone(), m.value.clone().unwrap_or_default(), m.span));
        }
    }
    out
}

impl Checker<'_> {
    pub(crate) fn i18n_rules(&mut self) {
        let c = self.c;
        // `<meta route>`: valid codes and absolute paths.
        for (loc, path, span) in route_slugs(c) {
            if !valid_locale(&loc) || self.ctx.i18n.as_ref().is_some_and(|i| !i.locales.contains(&loc)) {
                self.push("B1406", span, format!("`{loc}` is not one of the site's locales"), "use a locale listed in brain.json `i18n.locales`, e.g. `es=\"/es/about-us\"`");
            } else if !path.starts_with('/') {
                self.push("B1407", span, format!("the `{loc}` route must be an absolute path"), format!("write `{loc}=\"/{loc}/…\"`"));
            }
        }
        let Some(i18n) = self.ctx.i18n.clone() else { return };
        let mut extra: Vec<Diag> = Vec::new();
        // B1401 / B1404 for each `t()` call with a literal key.
        let mut seen = BTreeSet::new();
        let idx = brain_parse::LineIndex::new(&c.src);
        for (key, vars, span) in t_calls(c) {
            let (line, col) = idx.line_col(&c.src, span.start as usize);
            if seen.insert(key.clone()) {
                extra.extend(missing_key(&i18n, &key, &format!("{}:{line}:{col}", c.file)));
            }
            if let Some(text) = i18n.default_file().and_then(|f| f.entries.get(&key)).map(|(t, _, _)| t.clone()) {
                let need = text.vars();
                let given = vars.clone().unwrap_or_default();
                if !given.contains("*") {
                    let missing: Vec<&String> = need.iter().filter(|v| !given.contains(*v)).collect();
                    if !missing.is_empty() {
                        let list: Vec<String> = missing.iter().map(|v| v.to_string()).collect();
                        self.push("B1404", span, format!("`t('{key}')` needs {}", list.iter().map(|v| format!("`{{{v}}}`")).collect::<Vec<_>>().join(", ")), format!("pass them: `t('{key}', {{ {} }})`", need.iter().cloned().collect::<Vec<_>>().join(", ")));
                    }
                }
            }
        }
        // B1403: fixed text in a multilingual site.
        // Not in app.brain: the shell is static HTML (B0132); its text belongs in views/_layout.brain.
        if i18n.multi() && c.kind == FileKind::Component && !c.file.ends_with(".test.brain") {
            let found = fixed_texts(c);
            // While the default locale file is still `{}`, its insertions cannot compose in one `brain fix` round:
            // the template keeps its text until the file has entries, so no key ever falls back to its slug.
            let default_empty = i18n.default_file().is_some_and(|f| f.src[..f.close].trim_end().ends_with('{'));
            let mut keys = Keys { scope: key_scope(&c.file), default: i18n.default_file(), mine: BTreeMap::new() };
            for f in &found {
                let mut new = Vec::new();
                let call = keys.render(f, &c.src, &mut new);
                let shown = call.split('\'').nth(1).unwrap_or_default().to_string();
                let mut d = Diag::new("B1403", &c.file, f.span, format!("fixed text \"{}\" in a site with {} languages", crate::i18n::short(&f.msg), i18n.locales.len()), format!("move it to `{shown}` in every i18n/<locale>.json and write `{call}`"));
                if !default_empty {
                    d = d.edit(f.span.start, f.span.end, call);
                }
                self.diags.push(d);
                for (key, text) in new {
                    for lf in &i18n.files {
                        if lf.entries.contains_key(&key) || lf.error.is_some() {
                            continue;
                        }
                        let (a, b, txt) = insert_edit(lf, &key, &text);
                        let line = lf.src[..lf.close].matches('\n').count() as u32 + 1;
                        let mut fd = Diag::new("B1403", &lf.file, Span { start: lf.close as u32, end: lf.close as u32, line, col: 1 }, format!("new key `{key}` for the fixed text in {}", c.file), format!("translate `{key}` into `{}`", lf.locale)).edit(a, b, txt);
                        fd.locate(&lf.src);
                        extra.push(fd);
                    }
                }
            }
        }
        // B0306: physical CSS properties with a right-to-left locale.
        if i18n.rtl()
            && let Some(sheet) = &c.style
            && let Ok(sheet) = brain_css::expand_mixins(sheet, &c.file)
        {
            self.physical(&sheet.items);
        }
        for mut d in extra {
            if let Some(f) = i18n.files.iter().find(|f| f.file == d.file) {
                d.locate(&f.src);
            }
            self.diags.push(d);
        }
    }

    fn physical(&mut self, items: &[brain_css::CssItem]) {
        use brain_css::CssItem;
        for it in items {
            match it {
                CssItem::Rule { items: inner, .. } => {
                    for d in inner {
                        let CssItem::Decl { name, value, span } = d else { continue };
                        let text = self.text(*span).to_string();
                        if let Some(l) = logical(name)
                            && let Some(i) = text.find(name.as_str())
                        {
                            let at = span.start + i as u32;
                            self.push_edits("B0306", *span, format!("`{name}` does not flip in right-to-left languages"), format!("use `{l}`"), vec![(at, at + name.len() as u32, l.to_string())]);
                        } else if matches!(name.as_str(), "text-align" | "float" | "clear") && matches!(value.trim(), "left" | "right") {
                            let new = match (name.as_str(), value.trim()) {
                                ("text-align", "left") => "start",
                                ("text-align", "right") => "end",
                                (_, "left") => "inline-start",
                                _ => "inline-end",
                            };
                            let edits = text.rfind(value.trim()).map(|i| vec![(span.start + i as u32, span.start + (i + value.trim().len()) as u32, new.to_string())]).unwrap_or_default();
                            self.push_edits("B0306", *span, format!("`{name}: {}` does not flip in right-to-left languages", value.trim()), format!("use `{name}: {new}`"), edits);
                        }
                    }
                    self.physical(inner);
                }
                CssItem::At { block: Some(b), .. } => self.physical(b),
                _ => {}
            }
        }
    }
}
