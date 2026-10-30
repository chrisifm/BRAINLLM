//! File structure (SPEC «Component», «Architecture»):
//! - B0124 a `.jsbrain` module with more than one export.
//! - B0125 blocks out of order (`<meta>`, `<script>`, markup, `<style>`).
//! - B0126 a component without markup (tests, `*.test.brain`, are script only).

use crate::Checker;
use brain_parse::ast::*;
use brain_parse::Span;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Block {
    Meta,
    Script,
    Markup,
    Style,
}

impl Block {
    fn name(self) -> &'static str {
        match self {
            Block::Meta => "`<meta>`",
            Block::Script => "`<script>`",
            Block::Markup => "the markup",
            Block::Style => "`<style>`",
        }
    }
}

// Parsed markup expressions and attribute values are content, never file-level blocks.
fn expression_spans(nodes: &[Node], out: &mut Vec<Span>) {
    fn attrs(items: &[Attr], out: &mut Vec<Span>) { out.extend(items.iter().map(|attr| attr.span)); }
    for node in nodes {
        match node {
            Node::Interp(node) => out.push(node.span),
            Node::Element(node) => { attrs(&node.attrs, out); expression_spans(&node.children, out); }
            Node::ComponentRef(node) => { attrs(&node.attrs, out); expression_spans(&node.children, out); }
            Node::Slot(node) => attrs(&node.attrs, out),
            Node::If(node) => {
                for branch in &node.branches { out.push(branch.test.span()); expression_spans(&branch.body, out); }
                if let Some(body) = &node.otherwise { expression_spans(body, out); }
            }
            Node::Each(node) => {
                out.push(node.list.span());
                if let Some(key) = &node.key { out.push(key.span()); }
                expression_spans(&node.body, out);
                if let Some(body) = &node.empty { expression_spans(body, out); }
            }
            Node::Await(node) => {
                out.push(node.promise.span()); expression_spans(&node.pending, out);
                for branch in [&node.then, &node.catch].into_iter().flatten() { expression_spans(&branch.body, out); }
            }
            Node::SlotScope(node) => expression_spans(&node.body, out),
            Node::Text(_) => {}
        }
    }
}

/// Top-level blocks of a `.brain` source in order of appearance, with their byte offset.
fn blocks(src: &str, protected: &[Span]) -> Vec<(Block, usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    let b = src.as_bytes();
    while i < b.len() {
        if let Some(span) = protected.get(protected.partition_point(|span| span.start as usize <= i).wrapping_sub(1)).filter(|span| i < span.end as usize) {
            if out.last().is_none_or(|(kind, _, _)| *kind != Block::Markup) { out.push((Block::Markup, i, i)); }
            i = span.end as usize;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").map(|e| e + 3).unwrap_or(rest.len());
            continue;
        }
        let tag = |t: &str| rest.starts_with(t) && rest[t.len()..].starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/');
        let head_end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
        let head = &rest[..head_end];
        // HTML `<meta charset|name>` and `<script src|type>` inside `app.brain`'s `<head>` are markup.
        let brain_meta = tag("<meta") && !head.contains("charset") && !head.contains("name=") && !head.contains("http-equiv") && !head.contains("property=");
        let brain_script = tag("<script") && !head.contains("src=") && !head.contains("type=");
        if brain_meta {
            out.push((Block::Meta, i, i + head_end));
            i += head_end;
        } else if brain_script || tag("<style") {
            let (kind, close) = if brain_script { (Block::Script, "</script>") } else { (Block::Style, "</style>") };
            let end = i + if head.trim_end().ends_with("/>") {head_end} else {rest.find(close).map(|e| e + close.len()).unwrap_or(rest.len())};
            out.push((kind, i, end));
            i = end;
        } else if (b[i] as char).is_whitespace() {
            i += 1;
        } else if let Some(len) = brain_parse::raw_element_len(rest) {
            // `<code raw>…</code>`: a code sample, never blocks of this file.
            if out.last().is_none_or(|(k, _, _)| *k != Block::Markup) {
                out.push((Block::Markup, i, i));
            }
            i += len;
        } else {
            if out.last().is_none_or(|(k, _, _)| *k != Block::Markup) {
                out.push((Block::Markup, i, i));
            }
            // Up to the next tag that could start another block.
            let next = rest[1..].find('<').map(|e| e + 1).unwrap_or(rest.len());
            i += next;
        }
    }
    out
}

impl Checker<'_> {
    pub(crate) fn structure(&mut self) {
        let c = self.c;
        // `types.jsbrain` is the app's type catalog: one `export const` per type (SPEC «Data»).
        if c.kind == FileKind::Module && c.file.rsplit('/').next() == Some("types.jsbrain") {
            return;
        }
        if c.kind == FileKind::Module {
            let exports: Vec<Span> = c
                .script
                .iter()
                .filter_map(|s| match s {
                    Stmt::Export { span, .. } | Stmt::ExportDefault { span, .. } => Some(*span),
                    _ => None,
                })
                .collect();
            if let Some(second) = exports.get(1) {
                self.push(
                    "B0124",
                    *second,
                    format!("a `.jsbrain` module has one export; this one has {}", exports.len()),
                    "export a single object (`export default { a, b }`) or move the extra export to its own file",
                );
            }
            return;
        }
        let src = c.src.as_str();
        let mut protected = Vec::new();
        expression_spans(&c.template, &mut protected);
        expression_spans(&c.head, &mut protected);
        protected.sort_by_key(|span| (span.start, span.end));
        let found = blocks(src, &protected);
        let mut max = Block::Meta;
        for (k, at, _) in &found {
            if *k < max {
                let mut span = Span { start: *at as u32, end: *at as u32 + 1, line: 0, col: 0 };
                let idx = brain_parse::LineIndex::new(src);
                (span.line, span.col) = idx.line_col(src, span.start as usize);
                // §21: the order is mechanical, so the compiler rewrites the file in place.
                let fixed = reorder(src, &found);
                self.push_info(
                    "B0125",
                    span,
                    format!("{} after {}: blocks go in the order `<meta>`, `<script>`, markup, `<style>` (reordered)", k.name(), max.name()),
                    format!("move {} before {}", k.name(), max.name()),
                    vec![(0, src.len() as u32, fixed)],
                );
                break;
            }
            max = *k;
        }
        if !c.file.ends_with(".test.brain") && !found.iter().any(|(k, _, _)| *k == Block::Markup) {
            let span = Span { start: 0, end: 0, line: 1, col: 1 };
            self.push("B0126", span, "the component has no markup", "add the markup it renders after `<script>`; logic without markup goes in `lib/x.jsbrain`");
        }
    }
}

/// The same source with its blocks in canonical order: every `<meta>`, the `<script>`, the markup, the `<style>`.
fn reorder(src: &str, found: &[(Block, usize, usize)]) -> String {
    let mut metas = Vec::new();
    let mut scripts = Vec::new();
    let mut styles = Vec::new();
    let mut markup = String::new();
    let mut last = 0;
    for (k, a, b) in found.iter().filter(|(k, _, _)| *k != Block::Markup) {
        markup.push_str(&src[last..*a]);
        last = *b;
        let text = src[*a..*b].to_string();
        match k {
            Block::Meta => metas.push(text),
            Block::Script => scripts.push(text),
            Block::Style => styles.push(text),
            Block::Markup => {}
        }
    }
    markup.push_str(&src[last..]);
    let parts: Vec<String> = [metas.join("\n"), scripts.join("\n"), markup.trim().to_string(), styles.join("\n")].into_iter().filter(|p| !p.is_empty()).collect();
    format!("{}\n", parts.join("\n"))
}
