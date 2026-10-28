//! B0408 on the resolved cascade (QA #21 F8): per element of the markup, the `color` and the visible `background`
//! that win by specificity (color inherits, the background is the nearest painted ancestor's). `.card p` over
//! `.badge` can pair a muted text with an accent fill although each rule alone is fine. Conservative: selectors with
//! pseudo-classes, attributes, ids or `:scope`/`&` are ignored, so the check never guesses.

use crate::{contrast_ratio, resolve_color, Checker};
use brain_css::CssItem;
use brain_parse::ast::*;
use brain_parse::Span;

/// One compound: optional tag + classes.
#[derive(Debug)]
struct Compound {
    tag: Option<String>,
    classes: Vec<String>,
}

#[derive(Debug)]
struct Rule {
    /// Compounds from the outermost ancestor to the subject (descendant and child combinators both match as
    /// descendant: conservative for `>` only in the direction of reporting more pairs, never fewer).
    parts: Vec<Compound>,
    spec: (u32, u32),
    order: usize,
    color: Option<(String, (f64, f64, f64), Span)>,
    bg: Option<(String, (f64, f64, f64))>,
}

fn compound(s: &str) -> Option<Compound> {
    if s.is_empty() || s.contains([':', '[', '#', '&', '*', '(']) {
        return None;
    }
    let mut it = s.split('.');
    let tag = it.next().filter(|t| !t.is_empty()).map(|t| t.to_ascii_lowercase());
    let classes: Vec<String> = it.map(str::to_string).collect();
    if classes.iter().any(|c| c.is_empty() || !c.chars().all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_')) {
        return None;
    }
    Some(Compound { tag, classes })
}

fn rules(items: &[CssItem], tokens: &[(String, String)], out: &mut Vec<Rule>) {
    for it in items {
        match it {
            CssItem::Rule { selector, items: inner, .. } => {
                let mut color = None;
                let mut bg = None;
                for d in inner {
                    if let CssItem::Decl { name, value, span } = d {
                        match name.as_str() {
                            "color" => color = resolve_color(value, tokens).map(|(n, c)| (n, c, *span)),
                            "background" | "background-color" => bg = resolve_color(value, tokens),
                            _ => {}
                        }
                    }
                }
                if color.is_none() && bg.is_none() {
                    continue;
                }
                for sel in selector.split(',') {
                    let parts: Option<Vec<Compound>> = sel.replace('>', " ").split_whitespace().map(compound).collect();
                    let Some(parts) = parts.filter(|p| !p.is_empty()) else { continue };
                    let spec = parts.iter().fold((0, 0), |(c, t), p| (c + p.classes.len() as u32, t + u32::from(p.tag.is_some())));
                    out.push(Rule { parts, spec, order: out.len(), color: color.clone(), bg: bg.clone() });
                }
            }
            CssItem::At { name, block: Some(b), .. } if name == "layer" => rules(b, tokens, out),
            _ => {}
        }
    }
}

#[derive(Clone)]
struct El {
    tag: String,
    classes: Vec<String>,
}

fn matches_compound(c: &Compound, e: &El) -> bool {
    c.tag.as_ref().is_none_or(|t| *t == e.tag) && c.classes.iter().all(|k| e.classes.contains(k))
}

/// The subject matches `e` and the ancestors match in order.
fn matches(r: &Rule, e: &El, ancestors: &[El]) -> bool {
    let (subject, rest) = r.parts.split_last().unwrap();
    if !matches_compound(subject, e) {
        return false;
    }
    let mut i = ancestors.len();
    for c in rest.iter().rev() {
        loop {
            if i == 0 {
                return false;
            }
            i -= 1;
            if matches_compound(c, &ancestors[i]) {
                break;
            }
        }
    }
    true
}

fn el_of(e: &Element) -> El {
    let mut classes = Vec::new();
    for a in &e.attrs {
        match (&a.kind, &a.value) {
            (AttrKind::Plain, AttrValue::Text(t)) if a.name == "class" => classes.extend(t.split_whitespace().map(str::to_string)),
            (AttrKind::Class, _) => classes.push(a.name.clone()),
            _ => {}
        }
    }
    El { tag: e.tag.to_ascii_lowercase(), classes }
}

fn has_text(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| matches!(n, Node::Text(t) if !t.value.trim().is_empty()) || matches!(n, Node::Interp(_)))
}

struct Walk<'a> {
    rules: &'a [Rule],
    /// (fg rule order, bg rule order) pairs already reported, and the reports: (span, message).
    seen: Vec<(usize, usize)>,
    hits: Vec<(Span, String)>,
}

type Fg = Option<(String, (f64, f64, f64), Span, usize, String)>;
type Bg = Option<(String, (f64, f64, f64), usize, String)>;

impl Walk<'_> {
    fn nodes(&mut self, nodes: &[Node], ancestors: &mut Vec<El>, fg: &Fg, bg: &Bg) {
        for n in nodes {
            match n {
                Node::Element(e) => {
                    let el = el_of(e);
                    let mut win_fg: Option<&Rule> = None;
                    let mut win_bg: Option<&Rule> = None;
                    for r in self.rules.iter().filter(|r| matches(r, &el, ancestors)) {
                        if r.color.is_some() && win_fg.is_none_or(|w| (r.spec, r.order) > (w.spec, w.order)) {
                            win_fg = Some(r);
                        }
                        if r.bg.is_some() && win_bg.is_none_or(|w| (r.spec, r.order) > (w.spec, w.order)) {
                            win_bg = Some(r);
                        }
                    }
                    let label = |r: &Rule| r.parts.iter().map(|p| format!("{}{}", p.tag.clone().unwrap_or_default(), p.classes.iter().map(|c| format!(".{c}")).collect::<String>())).collect::<Vec<_>>().join(" ");
                    let fg2: Fg = match win_fg {
                        Some(r) => r.color.clone().map(|(n, c, s)| (n, c, s, r.order, label(r))),
                        None => fg.clone(),
                    };
                    let bg2: Bg = match win_bg {
                        Some(r) => r.bg.clone().map(|(n, c)| (n, c, r.order, label(r))),
                        None => bg.clone(),
                    };
                    if has_text(&e.children)
                        && let (Some((fname, f, span, fo, fsel)), Some((bname, b, bo, bsel))) = (&fg2, &bg2)
                        && fo != bo
                        && !self.seen.contains(&(*fo, *bo))
                    {
                        let ratio = contrast_ratio(*f, *b);
                        if ratio < 4.5 {
                            self.seen.push((*fo, *bo));
                            let classes = if el.classes.is_empty() { String::new() } else { format!(" class=\"{}\"", el.classes.join(" ")) };
                            self.hits.push((*span, format!("in `<{}{classes}>`, the text color `{fname}` from `{fsel}` lands on `{bname}` from `{bsel}` ({ratio:.2}:1, minimum 4.5:1)", el.tag)));
                        }
                    }
                    ancestors.push(el);
                    self.nodes(&e.children, ancestors, &fg2, &bg2);
                    ancestors.pop();
                }
                Node::If(b) => {
                    for br in &b.branches {
                        self.nodes(&br.body, ancestors, fg, bg);
                    }
                    if let Some(o) = &b.otherwise {
                        self.nodes(o, ancestors, fg, bg);
                    }
                }
                Node::Each(b) => {
                    self.nodes(&b.body, ancestors, fg, bg);
                    if let Some(e) = &b.empty {
                        self.nodes(e, ancestors, fg, bg);
                    }
                }
                Node::Await(a) => {
                    self.nodes(&a.pending, ancestors, fg, bg);
                    for br in [&a.then, &a.catch].into_iter().flatten() {
                        self.nodes(&br.body, ancestors, fg, bg);
                    }
                }
                Node::ComponentRef(r) => self.nodes(&r.children, ancestors, fg, bg),
                Node::SlotScope(s) => self.nodes(&s.body, ancestors, fg, bg),
                _ => {}
            }
        }
    }
}

impl Checker<'_> {
    pub(crate) fn cascade_contrast(&mut self, items: &[CssItem], tokens: &[(String, String)]) {
        let mut rs = Vec::new();
        rules(items, tokens, &mut rs);
        if !rs.iter().any(|r| r.color.is_some()) || !rs.iter().any(|r| r.bg.is_some()) {
            return;
        }
        let mut w = Walk { rules: &rs, seen: Vec::new(), hits: Vec::new() };
        w.nodes(&self.c.template, &mut Vec::new(), &None, &None);
        for (span, msg) in w.hits {
            self.push("B0408", span, msg, "give the element its own text color with enough contrast on that background, or make the more specific rule not reach it");
        }
    }
}
