//! B0128: a framework feature whose runtime module is not shipped yet. The compiler would emit an import the build
//! cannot resolve; the checker says so at the source, with the alternative to use today.

use crate::scope::{runtime_has, EXT_PRIMITIVES};
use crate::security::walk;
use crate::Checker;
use brain_parse::ast::*;

/// What to write instead while a primitive is planned.
fn instead(name: &str) -> &'static str {
    match name {
        "t" | "locale" => "write the text directly for now (single-language site); `t()` arrives with the runtime's brain-i18n.js",
        _ => "use a supported primitive; this one is not shipped by the runtime yet",
    }
}

impl Checker<'_> {
    pub(crate) fn planned(&mut self) {
        let c = self.c;
        let local: std::collections::HashSet<String> = crate::scope::top_bindings(c).into_keys().collect();
        let mut hits: Vec<(String, &'static str, brain_parse::Span)> = Vec::new();
        let check = |name: &str, span: brain_parse::Span, hits: &mut Vec<(String, &'static str, brain_parse::Span)>| {
            if local.contains(name) {
                return;
            }
            if let Some((_, module)) = EXT_PRIMITIVES.iter().find(|(n, _)| *n == name)
                && !runtime_has(module, name)
            {
                hits.push((name.to_string(), module, span));
            }
        };
        // `t('key')` with a plain text in the default locale is resolved at build: it needs no runtime.
        let i18n = self.ctx.i18n.clone();
        let is_static_t = |e: &Expr| -> bool {
            let Expr::Call { args, .. } = e else { return false };
            let Some(Expr::Str { value, .. }) = args.first().map(Expr::unparen) else { return false };
            i18n.as_ref().and_then(|i| i.default_file()).and_then(|f| f.entries.get(value)).is_some_and(|(t, _, _)| t.as_plain().is_some())
        };
        let mut calls = Vec::new();
        let mut take = |e: &Expr, _: bool, _: &[String]| {
            if let Expr::Call { callee, span, .. } = e
                && let Expr::Ident(i) = callee.unparen()
                && !(i.name == "t" && is_static_t(e))
            {
                calls.push((i.name.clone(), *span));
            }
        };
        walk(&c.script, false, &mut take);
        collect_calls(&c.template, &mut take);
        for (n, s) in calls {
            check(&n, s, &mut hits);
        }
        // Emitted by the codegen, not written by the author.
        // A `<meta tools>` that already has a B17xx error is fixed first; B0128 comes once it is well formed.
        for m in c.meta.iter().filter(|m| m.name == "tools") {
            if !self.diags.iter().any(|d| d.code.starts_with("B17") && d.line == m.span.line) {
                check("registerTools", m.span, &mut hits);
            }
        }
        let mut seen = std::collections::HashSet::new();
        for (name, module, span) in hits {
            if seen.insert(name.clone()) {
                let what = if name == "registerTools" { "`<meta tools>`".to_string() } else { format!("`{name}()`") };
                self.push("B0128", span, format!("{what} is planned and not available yet (runtime module {module})"), instead(&name));
            }
        }
    }
}

pub(crate) fn collect_calls(nodes: &[Node], f: &mut dyn FnMut(&Expr, bool, &[String])) {
    fn go(nodes: &[Node], f: &mut dyn FnMut(&Expr, bool, &[String])) {
        for n in nodes {
            match n {
                Node::Interp(i) => crate::security::expr(&i.expr, false, &[], f),
                Node::Element(Element { attrs, children, .. }) | Node::ComponentRef(ComponentRef { attrs, children, .. }) => {
                    for a in attrs {
                        match &a.value {
                            AttrValue::Expr(x) => crate::security::expr(x, false, &[], f),
                            AttrValue::Mixed(parts) => {
                                for p in parts {
                                    if let AttrPart::Expr(x) = p {
                                        crate::security::expr(x, false, &[], f);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    go(children, f);
                }
                Node::If(b) => {
                    for br in &b.branches {
                        crate::security::expr(&br.test, false, &[], f);
                        go(&br.body, f);
                    }
                    if let Some(o) = &b.otherwise {
                        go(o, f);
                    }
                }
                Node::Each(b) => {
                    crate::security::expr(&b.list, false, &[], f);
                    go(&b.body, f);
                    if let Some(e) = &b.empty {
                        go(e, f);
                    }
                }
                Node::SlotScope(b) => go(&b.body, f),
                Node::Slot(s) => {
                    for a in &s.attrs {
                        match &a.value {
                            AttrValue::Expr(x) => crate::security::expr(x, false, &[], f),
                            AttrValue::Mixed(parts) => {
                                for p in parts {
                                    if let AttrPart::Expr(x) = p {
                                        crate::security::expr(x, false, &[], f);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Node::Await(a) => {
                    crate::security::expr(&a.promise, false, &[], f);
                    go(&a.pending, f);
                    for br in [&a.then, &a.catch].into_iter().flatten() {
                        go(&br.body, f);
                    }
                }
                _ => {}
            }
        }
    }
    go(nodes, f);
}
