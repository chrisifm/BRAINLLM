//! Per-file language rules (INTERNALS §37); the app-wide ones are in [`crate::i18n::check_locales`].

use crate::i18n::{insert_edit, key_for, logical, missing_key, translatable, valid_locale};
use crate::security::{expr as visit, walk};
use crate::Checker;
use brain_parse::ast::*;
use brain_parse::{Diag, Span};
use std::collections::BTreeSet;

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

/// Text nodes with translatable text, outside `<code>`/`<pre>`/`<script>`/`<style>`.
fn texts<'a>(ns: &'a [Node], out: &mut Vec<&'a Text>) {
    for n in ns {
        match n {
            Node::Text(t) if translatable(&t.value) => out.push(t),
            Node::Element(e) if !matches!(e.tag.as_str(), "code" | "pre" | "kbd" | "samp" | "svg" | "math") => texts(&e.children, out),
            Node::ComponentRef(r) => texts(&r.children, out),
            Node::If(b) => {
                for br in &b.branches {
                    texts(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    texts(o, out);
                }
            }
            Node::Each(b) => {
                texts(&b.body, out);
                if let Some(e) = &b.empty {
                    texts(e, out);
                }
            }
            Node::SlotScope(b) => texts(&b.body, out),
            Node::Await(a) => {
                texts(&a.pending, out);
                for br in [&a.then, &a.catch].into_iter().flatten() {
                    texts(&br.body, out);
                }
            }
            _ => {}
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
            let mut ts = Vec::new();
            texts(&c.template, &mut ts);
            let mut new_keys: BTreeSet<String> = BTreeSet::new();
            for t in ts {
                let lead = t.value.len() - t.value.trim_start().len();
                let text = t.value.trim();
                let key = key_for(&c.file, text);
                let first = new_keys.insert(key.clone());
                let start = t.span.start + lead as u32;
                let mut d = Diag::new("B1403", &c.file, Span { start, end: start + text.len() as u32, line: t.span.line, col: t.span.col }, format!("fixed text \"{}\" in a site with {} languages", crate::i18n::short(text), i18n.locales.len()), format!("move it to `{key}` in every i18n/<locale>.json and write `{{t('{key}')}}`"));
                // While the default locale file is still `{}`, its insertions cannot compose in one `brain fix` round:
                // the template keeps its text until the file has entries, so no key ever falls back to its slug.
                let default_empty = i18n.default_file().is_some_and(|f| f.src[..f.close].trim_end().ends_with('{'));
                if !default_empty {
                    d = d.edit(start, start + text.len() as u32, format!("{{t('{key}')}}"));
                }
                self.diags.push(d);
                for f in &i18n.files {
                    if !first || f.entries.contains_key(&key) || f.error.is_some() {
                        continue;
                    }
                    let (a, b, txt) = insert_edit(f, &key, text);
                    let line = f.src[..f.close].matches('\n').count() as u32 + 1;
                    let mut fd = Diag::new("B1403", &f.file, Span { start: f.close as u32, end: f.close as u32, line, col: 1 }, format!("new key `{key}` for the fixed text in {}", c.file), format!("translate `{key}` into `{}`", f.locale)).edit(a, b, txt);
                    fd.locate(&f.src);
                    extra.push(fd);
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
