//! `<Img src alt width height>` (INTERNALS §22): lowered to a plain `<img>` before code generation. Every image gets
//! `decoding="async"`. In document order, the first large image of a view is the likely LCP element and gets
//! `fetchpriority="high"`; images before it stay eager (up to three), the rest get `loading="lazy"`. A component cannot see
//! its page, so its first image with large literal dimensions only avoids `lazy`. In `{#each}` the same rule is decided per
//! iteration: priority for the first, the first three eager. An author's own `Img` (imported or declared) is left alone.

use brain_parse::ast::*;
use brain_parse::Span;

/// Below this area (`width * height`) an image is an icon or a thumbnail, not the LCP candidate.
const LARGE_AREA: u64 = 50_000;
/// Images that may load eagerly before the LCP candidate, and iterations of a list that stay eager.
const EAGER: usize = 3;
/// The index given to an `{#each}` that needs one and did not name it.
const IMG_INDEX: &str = "$imgIndex";

#[derive(Clone)]
struct Budget {
    view: bool,
    /// The LCP candidate has not been seen yet.
    open: bool,
    eager: usize,
    /// Inside an `{#each}` that decides per iteration: the name of its index.
    each: Option<String>,
}

impl Budget {
    /// Alternatives (`{#if}` branches, `{#await}` states) each get the budget they started with: only one renders.
    fn alternatives(&mut self, branches: Vec<Budget>, exhaustive: bool) {
        let start = self.clone();
        self.open = branches.iter().any(|b| b.open) || (!exhaustive && start.open);
        self.eager = branches.iter().map(|b| b.eager).max().unwrap_or(start.eager);
    }
}

fn plain(name: &str, value: &str, span: Span) -> Attr {
    Attr { kind: AttrKind::Plain, name: name.into(), value: AttrValue::Text(value.into()), span }
}

fn has(attrs: &[Attr], name: &str) -> bool {
    attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == name)
}

fn literal(attrs: &[Attr], name: &str) -> Option<u64> {
    attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == name).and_then(|a| match &a.value {
        AttrValue::Text(t) => t.trim().parse().ok(),
        AttrValue::Expr(Expr::Num { raw, .. }) => raw.parse().ok(),
        _ => None,
    })
}

/// `width * height` when both are written as numbers; a dynamic size is unknown.
fn area(attrs: &[Attr]) -> Option<u64> {
    Some(literal(attrs, "width")?.saturating_mul(literal(attrs, "height")?))
}

fn num(value: usize, span: Span) -> Box<Expr> {
    Box::new(Expr::Num { raw: value.to_string(), span })
}

/// `{index <op> limit ? then : otherwise}`; the runtime removes an attribute whose value is null.
fn by_index(name: &str, index: &str, op: &'static str, limit: usize, then: Option<&str>, otherwise: Option<&str>, span: Span) -> Attr {
    let value = |v: Option<&str>| {
        Box::new(v.map_or(Expr::Null(span), |t| Expr::Str { value: t.into(), raw: format!("'{t}'"), span }))
    };
    let test = Expr::Binary { op, left: Box::new(Expr::Ident(Ident { name: index.into(), span })), right: num(limit, span), span };
    let cond = Expr::Cond { test: Box::new(test), cons: value(then), alt: value(otherwise), span };
    Attr { kind: AttrKind::Plain, name: name.into(), value: AttrValue::Expr(cond), span }
}

fn lower_img(attrs: &mut Vec<Attr>, b: &mut Budget, span: Span) {
    if !has(attrs, "decoding") {
        attrs.push(plain("decoding", "async", span));
    }
    let explicit_lazy = attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == "loading" && matches!(&a.value, AttrValue::Text(t) if t == "lazy"));
    let area = area(attrs);
    // A component's image may repeat in a list the component cannot see: only a declared large size earns eagerness there.
    let large = if b.view { area.is_none_or(|a| a >= LARGE_AREA) } else { area.is_some_and(|a| a >= LARGE_AREA) };
    if !b.open || explicit_lazy {
        if !has(attrs, "loading") {
            attrs.push(plain("loading", "lazy", span));
        }
    } else if let Some(index) = b.each.clone() {
        if b.view && large && !has(attrs, "fetchpriority") {
            attrs.push(by_index("fetchpriority", &index, "===", 0, Some("high"), None, span));
        }
        if !has(attrs, "loading") {
            attrs.push(by_index("loading", &index, "<", EAGER, None, Some("lazy"), span));
        }
        b.open = false;
    } else if large {
        if b.view && !has(attrs, "fetchpriority") {
            attrs.push(plain("fetchpriority", "high", span));
        }
        b.open = false;
    } else if b.view {
        b.eager += 1;
        b.open = b.eager < EAGER;
    } else if !has(attrs, "loading") {
        attrs.push(plain("loading", "lazy", span));
    }
}

fn lower(nodes: &mut [Node], b: &mut Budget) {
    for n in nodes.iter_mut() {
        let replace = match n {
            Node::ComponentRef(r) if r.name == "Img" => {
                let mut attrs = r.attrs.clone();
                lower_img(&mut attrs, b, r.span);
                Some(Node::Element(Element { tag: "img".into(), attrs, children: Vec::new(), span: r.span }))
            }
            Node::Element(e) => {
                lower(&mut e.children, b);
                None
            }
            Node::ComponentRef(r) => {
                lower(&mut r.children, b);
                None
            }
            Node::If(x) => {
                let mut seen = Vec::new();
                for br in &mut x.branches {
                    let mut own = b.clone();
                    lower(&mut br.body, &mut own);
                    seen.push(own);
                }
                let exhaustive = x.otherwise.is_some();
                if let Some(o) = &mut x.otherwise {
                    let mut own = b.clone();
                    lower(o, &mut own);
                    seen.push(own);
                }
                b.alternatives(seen, exhaustive);
                None
            }
            Node::Each(x) => {
                if uses_img(&x.body) {
                    if b.open && b.each.is_none() {
                        let index = x.index.get_or_insert_with(|| Ident { name: IMG_INDEX.into(), span: x.span }).name.clone();
                        let mut own = Budget { each: Some(index), ..b.clone() };
                        lower(&mut x.body, &mut own);
                    } else {
                        // A list inside a list has no first iteration of the page; the priority is settled by the outer one.
                        lower(&mut x.body, &mut Budget { open: false, ..b.clone() });
                    }
                    b.open = false;
                }
                if let Some(e) = &mut x.empty {
                    lower(e, &mut Budget { open: false, ..b.clone() });
                }
                None
            }
            Node::Await(a) => {
                let mut seen = Vec::new();
                let mut own = b.clone();
                lower(&mut a.pending, &mut own);
                seen.push(own);
                for t in [&mut a.then, &mut a.catch].into_iter().flatten() {
                    let mut own = b.clone();
                    lower(&mut t.body, &mut own);
                    seen.push(own);
                }
                b.alternatives(seen, true);
                None
            }
            Node::SlotScope(s) => {
                lower(&mut s.body, b);
                None
            }
            _ => None,
        };
        if let Some(r) = replace {
            *n = r;
        }
    }
}

fn uses_img(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::ComponentRef(r) => r.name == "Img" || uses_img(&r.children),
        Node::Element(e) => uses_img(&e.children),
        Node::If(b) => b.branches.iter().any(|br| uses_img(&br.body)) || b.otherwise.as_deref().is_some_and(uses_img),
        Node::Each(b) => uses_img(&b.body) || b.empty.as_deref().is_some_and(uses_img),
        Node::Await(a) => uses_img(&a.pending) || [&a.then, &a.catch].into_iter().flatten().any(|t| uses_img(&t.body)),
        Node::SlotScope(s) => uses_img(&s.body),
        _ => false,
    })
}

/// The component with every built-in `<Img>` lowered, or `None` when there is nothing to lower.
pub(crate) fn lowered(c: &Component) -> Option<Component> {
    if !uses_img(&c.template) || brain_check::scope::top_bindings(c).contains_key("Img") {
        return None;
    }
    let mut out = c.clone();
    let mut budget = Budget { view: c.level == Level::View && c.kind == FileKind::Component, open: true, eager: 0, each: None };
    lower(&mut out.template, &mut budget);
    Some(out)
}
