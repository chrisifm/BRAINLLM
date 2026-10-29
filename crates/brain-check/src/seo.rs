//! INTERNALS §32 metadata rules, and what the SEO module (`brain-seo`, §33) reads from a view.
//! - B1601 view whose `<meta seo>` has no `description`.
//! - B1602 (warning) static `title` longer than 60 or `description` longer than 160 characters.
//! - B1603 `jsonld={{…}}` object without `@type`.
//! - B1604 the same static `canonical` in two views.
//! - B0502 (warning) indexable view without `keyword` (the search query the page targets).

use crate::Checker;
use brain_parse::ast::*;
use brain_parse::Span;
use std::collections::HashMap;
use std::path::Path;

/// A `<meta seo>` value: fixed text or an expression over the `load` props (JS source as written).
#[derive(Clone, Debug, PartialEq)]
pub enum SeoValue {
    Text(String),
    Expr(String),
}

/// What a view says about itself, for the SEO audit (§33). Static text only: interpolations are left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewSeo {
    /// `<meta seo …>` attributes in order (`seo` itself excluded).
    pub attrs: Vec<(String, SeoValue)>,
    pub canonical: Option<String>,
    pub keyword: Option<String>,
    /// `jsonld={…}` as JS source.
    pub jsonld: Option<String>,
    pub has_head: bool,
    /// The view exports `paths()` (dynamic route prerendered per param).
    pub has_paths: bool,
    /// `(level, static text, span)` of every heading, in document order.
    pub headings: Vec<(u8, String, Span)>,
    /// Static text of the markup, whitespace-collapsed.
    pub text: String,
    /// `(src, alt, span)` of every `<img>` (`alt` = `None` when absent or dynamic).
    pub images: Vec<(String, Option<String>, Span)>,
    /// `(href, static text, external, span)` of every `<a>` with a static `href`.
    pub links: Vec<(String, String, bool, Span)>,
}

/// [`ViewSeo`] of a parsed view.
pub fn view_info(c: &Component) -> ViewSeo {
    let mut v = ViewSeo::default();
    for m in c.meta.iter().skip_while(|m| m.name != "seo").skip(1) {
        // Attributes of the same `<meta seo>` tag come right after `seo` until another tag's first attribute
        // (`props`, `tools`…); the parser keeps them in source order.
        if matches!(m.name.as_str(), "props" | "emits" | "expose" | "export" | "tools" | "server" | "fallback" | "cache" | "http" | "client" | "budget" | "route") {
            break;
        }
        let val = match (&m.expr, &m.value) {
            (Some(x), _) => SeoValue::Expr(brain_parse::dump::expr(x)),
            (None, Some(t)) => SeoValue::Text(t.clone()),
            (None, None) => SeoValue::Text(String::new()),
        };
        match (m.name.as_str(), &val) {
            ("canonical", SeoValue::Text(t)) => v.canonical = Some(t.clone()),
            ("keyword", SeoValue::Text(t)) => v.keyword = Some(t.clone()),
            ("jsonld", SeoValue::Expr(x)) => v.jsonld = Some(x.clone()),
            _ => {}
        }
        v.attrs.push((m.name.clone(), val));
    }
    v.has_head = !c.head.is_empty();
    v.has_paths = c.script.iter().any(|s| matches!(s, Stmt::Export { decl, .. } if matches!(decl.as_ref(), Stmt::Func(f) if f.name.as_ref().is_some_and(|n| n.name == "paths"))));
    walk(&c.template, &mut v);
    v.text = v.text.split_whitespace().collect::<Vec<_>>().join(" ");
    v
}

fn static_text(nodes: &[Node]) -> String {
    let mut s = String::new();
    for n in nodes {
        match n {
            Node::Text(t) => s.push_str(&t.value),
            Node::Element(e) => s.push_str(&static_text(&e.children)),
            _ => {}
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_attr(e: &Element, name: &str) -> Option<String> {
    e.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == name).and_then(|a| match &a.value {
        AttrValue::Text(t) => Some(t.clone()),
        AttrValue::Expr(Expr::Str { value, .. }) => Some(value.clone()),
        _ => None,
    })
}

fn walk(nodes: &[Node], v: &mut ViewSeo) {
    for n in nodes {
        match n {
            Node::Text(t) => {
                v.text.push(' ');
                v.text.push_str(&t.value);
            }
            Node::Element(e) => {
                let tag = e.tag.as_str();
                if let Some(l) = tag.strip_prefix('h').and_then(|d| d.parse::<u8>().ok()).filter(|l| (1..=6).contains(l)) {
                    v.headings.push((l, static_text(&e.children), e.span));
                }
                if tag == "img" {
                    v.images.push((text_attr(e, "src").unwrap_or_default(), text_attr(e, "alt"), e.span));
                }
                if tag == "a"
                    && let Some(h) = text_attr(e, "href")
                {
                    let external = h.starts_with("http://") || h.starts_with("https://") || h.starts_with("//");
                    v.links.push((h, static_text(&e.children), external, e.span));
                }
                walk(&e.children, v);
            }
            Node::ComponentRef(r) => walk(&r.children, v),
            Node::If(b) => {
                for br in &b.branches {
                    walk(&br.body, v);
                }
                if let Some(o) = &b.otherwise {
                    walk(o, v);
                }
            }
            Node::Each(b) => walk(&b.body, v),
            Node::SlotScope(b) => walk(&b.body, v),
            Node::Await(a) => walk(&a.pending, v),
            _ => {}
        }
    }
}

/// Static canonicals of every view under `<root>/views` (B1604).
pub fn canonicals(root: &Path) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut stack = vec![root.join("views")];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "brain") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&p) else { continue };
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            if let Ok(c) = brain_parse::parse(&src, Path::new(&rel))
                && let Some(can) = view_info(&c).canonical
            {
                out.entry(can).or_default().push(rel);
            }
        }
    }
    out
}

impl Checker<'_> {
    pub(crate) fn seo_rules(&mut self) {
        let c = self.c;
        // Expressions in `<meta seo>` and the `<head>` block read the view's scope.
        for m in &c.meta {
            if let Some(x) = &m.expr {
                self.expr(x);
            }
        }
        if !c.head.is_empty() {
            self.nodes(&c.head);
        }
        if c.kind != FileKind::Component || c.level != Level::View || !c.meta.iter().any(|m| m.name == "seo") {
            return;
        }
        let info = view_info(c);
        let seo_span = c.meta.iter().find(|m| m.name == "seo").map(|m| m.span).unwrap_or_default();
        let attr = |name: &str| c.meta.iter().find(|m| m.name == name);
        // B0502: an indexable page names the search query it should rank for (`keyword`, audit-only, never rendered).
        // Skipped for pages not meant to rank: noindex, partials (`_x.brain`), dynamic routes, guarded folders and error pages.
        let file_name = c.file.rsplit('/').next().unwrap_or("");
        let noindex = matches!(info.attrs.iter().find(|(n, _)| n == "robots"), Some((_, SeoValue::Text(r))) if r.to_ascii_lowercase().contains("noindex"));
        // dynamic routes (`[slug].brain`) take their query from the data of each page: not asked for one static keyword
        let special = file_name.starts_with('_') || c.file.contains('[') || ["404.brain", "500.brain", "error.brain"].contains(&file_name);
        if !info.attrs.iter().any(|(n, _)| n == "keyword") && !noindex && !special && !self.ctx.guarded {
            self.push(
                "B0502",
                seo_span,
                "the view has no `keyword`: the page does not say which search query it should rank for",
                "pick the query a reader would type to find this page (2-4 words, e.g. \"offline task planner\"), add `keyword=\"<that query>\"` to `<meta seo>`, and use those exact words in the title, the H1, the first sentence and the description; for a page that should not rank, add `robots=\"noindex\"` instead",
            );
        }
        if !info.attrs.iter().any(|(n, _)| n == "description") {
            self.push("B1601", seo_span, "the view has no `description`", "add `description=\"…\"` (one sentence, ≤ 160 characters) or `description={props.…}` to `<meta seo>`");
        }
        for (name, max) in [("title", 60usize), ("description", 160)] {
            if let Some((_, SeoValue::Text(t))) = info.attrs.iter().find(|(n, _)| n == name)
                && t.chars().count() > max
            {
                let span = attr(name).map(|m| m.span).unwrap_or(seo_span);
                self.push("B1602", span, format!("`{name}` has {} characters; search results cut it after {max}", t.chars().count()), format!("shorten the {name} to {max} characters or fewer"));
            }
        }
        if let Some(m) = attr("jsonld")
            && let Some(Expr::Object { props, .. }) = m.expr.as_ref().map(Expr::unparen)
        {
            let has_type = props.iter().any(|p| match p {
                ObjProp::KeyValue { key: PropKey::Str { value, .. }, .. } => value == "@type",
                _ => false,
            });
            if !has_type {
                self.push("B1603", m.span, "JSON-LD without `@type`: search engines ignore it", "add the schema.org type: `jsonld={{ '@type': 'Article', headline: props.post.title }}`");
            }
        }
        if let Some(can) = &info.canonical
            && let Some(files) = self.ctx.canonicals.get(can)
        {
            let others: Vec<&String> = files.iter().filter(|f| **f != c.file && !c.file.ends_with(f.as_str())).collect();
            if !others.is_empty() {
                let span: Span = attr("canonical").map(|m| m.span).unwrap_or(seo_span);
                self.push("B1604", span, format!("canonical `{can}` is also declared by {}", others.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")), "give each view its own canonical URL (the route it is served at)");
            }
        }
    }
}
