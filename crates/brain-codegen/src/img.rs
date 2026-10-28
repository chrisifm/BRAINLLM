//! `<Img src alt width height>` (INTERNALS §22): lowered to a plain `<img>` before code generation. Every image gets
//! `decoding="async"`; the first `<Img>` of a view (document order) is the likely LCP element and gets
//! `fetchpriority="high"`, the rest `loading="lazy"`. An author's own `Img` (imported or declared) is left alone.

use brain_parse::ast::*;

fn plain(name: &str, value: &str, span: brain_parse::Span) -> Attr {
    Attr { kind: AttrKind::Plain, name: name.into(), value: AttrValue::Text(value.into()), span }
}

fn lower(nodes: &mut [Node], view: bool, first: &mut bool) {
    for n in nodes.iter_mut() {
        let replace = match n {
            Node::ComponentRef(r) if r.name == "Img" => {
                let mut attrs = r.attrs.clone();
                let has = |a: &[Attr], k: &str| a.iter().any(|x| x.kind == AttrKind::Plain && x.name == k);
                if !has(&attrs, "decoding") {
                    attrs.push(plain("decoding", "async", r.span));
                }
                if view && *first {
                    if !has(&attrs, "fetchpriority") {
                        attrs.push(plain("fetchpriority", "high", r.span));
                    }
                } else if !has(&attrs, "loading") {
                    attrs.push(plain("loading", "lazy", r.span));
                }
                *first = false;
                Some(Node::Element(Element { tag: "img".into(), attrs, children: Vec::new(), span: r.span }))
            }
            Node::Element(e) => {
                lower(&mut e.children, view, first);
                None
            }
            Node::ComponentRef(r) => {
                lower(&mut r.children, view, first);
                None
            }
            Node::If(b) => {
                for br in &mut b.branches {
                    lower(&mut br.body, view, first);
                }
                if let Some(o) = &mut b.otherwise {
                    lower(o, view, first);
                }
                None
            }
            Node::Each(b) => {
                lower(&mut b.body, view, first);
                if let Some(e) = &mut b.empty {
                    lower(e, view, first);
                }
                None
            }
            Node::Await(a) => {
                lower(&mut a.pending, view, first);
                if let Some(t) = &mut a.then {
                    lower(&mut t.body, view, first);
                }
                if let Some(t) = &mut a.catch {
                    lower(&mut t.body, view, first);
                }
                None
            }
            Node::SlotScope(s) => {
                lower(&mut s.body, view, first);
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
    let mut first = true;
    lower(&mut out.template, c.level == Level::View && c.kind == FileKind::Component, &mut first);
    Some(out)
}
