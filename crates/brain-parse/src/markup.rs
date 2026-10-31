//! Markup parser: HTML superset with `{expr}`, `{#if}`, `{#each}`, `{#await}`, directives and
//! component references. It also lifts the top-level `<meta>`, `<script>` and `<style>` blocks.

use crate::ast::*;
use crate::js::JsParser;
use brain_lex::{Diag, LineIndex, Lexer, Span};

type R<T> = Result<T, Diag>;

const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];

pub struct Parsed {
    pub props: Vec<Prop>,
    pub meta: Vec<MetaAttr>,
    pub script: Vec<Stmt>,
    pub template: Vec<Node>,
    pub style: Option<CssSheet>,
}

enum End {
    Eof,
    Close(String, Span),
    ElseIf(Expr, Span),
    Else(Span),
    Empty(Span),
    Then(Option<Pat>, Span),
    Catch(Option<Pat>, Span),
    EndIf(Span),
    EndEach(Span),
    EndAwait(Span),
    EndSlot(Span),
}

impl End {
    fn describe(&self) -> String {
        match self {
            End::Eof => "the end of the file".into(),
            End::Close(t, _) => format!("`</{t}>`"),
            End::ElseIf(..) => "`{:else if}`".into(),
            End::Else(_) => "`{:else}`".into(),
            End::Empty(_) => "`{:empty}`".into(),
            End::Then(..) => "`{:then}`".into(),
            End::Catch(..) => "`{:catch}`".into(),
            End::EndIf(_) => "`{/if}`".into(),
            End::EndEach(_) => "`{/each}`".into(),
            End::EndSlot(_) => "`{/slot}`".into(),
            End::EndAwait(_) => "`{/await}`".into(),
        }
    }
    fn span(&self, fallback: Span) -> Span {
        match self {
            End::Eof => fallback,
            End::Close(_, s) | End::ElseIf(_, s) | End::Else(s) | End::Empty(s) | End::Then(_, s) | End::Catch(_, s) | End::EndIf(s) | End::EndEach(s) | End::EndAwait(s) | End::EndSlot(s) => *s,
        }
    }
}

struct RawAttr {
    attr: Attr,
    /// Byte range of a quoted value's contents.
    value_range: Option<(usize, usize)>,
}

pub struct Markup<'a> {
    src: &'a str,
    file: &'a str,
    idx: &'a LineIndex,
    pos: usize,
    out: Parsed,
    diags: Vec<Diag>,
    seen_script: bool,
    /// Open tags and blocks around `pos` (bounded by [`MAX_MARKUP_DEPTH`]).
    depth: u32,
}

/// Tags and `{#…}` blocks nested deeper than this are a diagnostic, not a stack overflow in the parser or later passes.
const MAX_MARKUP_DEPTH: u32 = 256;

impl<'a> Markup<'a> {
    pub fn new(src: &'a str, file: &'a str, idx: &'a LineIndex) -> Self {
        Markup {
            src,
            file,
            idx,
            pos: 0,
            out: Parsed { props: Vec::new(), meta: Vec::new(), script: Vec::new(), template: Vec::new(), style: None },
            diags: Vec::new(),
            seen_script: false,
            depth: 0,
        }
    }

    pub fn run(mut self) -> Result<Parsed, Vec<Diag>> {
        match self.children(true) {
            Ok((nodes, End::Eof)) => self.out.template = nodes,
            Ok((_, end)) => {
                let s = end.span(self.span(self.pos, self.pos));
                self.diags.push(self.err(s, &format!("{} has no block to close here", end.describe()), "remove the stray closing tag or open the matching block"));
            }
            Err(d) => self.diags.push(d),
        }
        if self.diags.is_empty() { Ok(self.out) } else { Err(self.diags) }
    }

    fn span(&self, a: usize, b: usize) -> Span {
        self.idx.span(self.src, a, b)
    }
    fn err(&self, span: Span, msg: &str, fix: &str) -> Diag {
        Diag::new("B0003", self.file, span, msg, fix)
    }
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }
    fn skip_ws(&mut self) {
        let r = self.rest();
        self.pos += r.len() - r.trim_start().len();
    }

    fn js_at(&self, pos: usize, end: usize) -> R<JsParser<'a>> {
        JsParser::new(Lexer::new(self.src, self.file, self.idx, pos, end))
    }

    /// Parses `expr}` starting at `self.pos` and moves past the `}`.
    fn expr_until_brace(&mut self) -> R<Expr> {
        let mut p = self.js_at(self.pos, self.src.len())?;
        if p.current().is_punct("}") {
            return Err(self.err(p.current().span, "the braces `{}` are empty", "write an expression inside: `{name}`"));
        }
        let e = p.expression()?;
        self.close_brace(&p)?;
        Ok(e)
    }

    fn close_brace(&mut self, p: &JsParser<'_>) -> R<()> {
        let t = p.current();
        if !t.is_punct("}") {
            return Err(Diag::new(
                "B0002",
                self.file,
                t.span,
                format!("expected `}}` but found {}", JsParser::describe(t)),
                "close the expression with `}`",
            ));
        }
        self.pos = t.span.end as usize;
        Ok(())
    }

    fn read_name(&mut self) -> &'a str {
        let start = self.pos;
        let r = self.rest();
        let n = r.find(|c: char| !(c.is_alphanumeric() || c == '-' || c == ':' || c == '_' || c == '.')).unwrap_or(r.len());
        self.pos += n;
        &self.src[start..start + n]
    }

    fn children(&mut self, top: bool) -> R<(Vec<Node>, End)> {
        if self.depth >= MAX_MARKUP_DEPTH {
            return Err(self.err(self.span(self.pos, self.pos), "the markup is nested too deeply to compile", "flatten it or move inner parts into components"));
        }
        self.depth += 1;
        let r = self.children_at(top);
        self.depth -= 1;
        r
    }

    fn children_at(&mut self, top: bool) -> R<(Vec<Node>, End)> {
        let mut nodes = Vec::new();
        loop {
            let r = self.rest();
            if r.is_empty() {
                return Ok((nodes, End::Eof));
            }
            let start = self.pos;
            if r.starts_with("<!--") {
                match r.find("-->") {
                    Some(i) => self.pos += i + 3,
                    None => return Err(self.err(self.span(start, start + 4), "unclosed HTML comment", "close the comment with `-->`")),
                }
            } else if r.starts_with("</") {
                self.pos += 2;
                let name = self.read_name().to_string();
                self.skip_ws();
                if !self.rest().starts_with('>') {
                    return Err(self.err(self.span(start, self.pos), &format!("incomplete closing tag `</{name}`"), &format!("write `</{name}>`")));
                }
                self.pos += 1;
                return Ok((nodes, End::Close(name, self.span(start, self.pos))));
            } else if r.starts_with('<') && r[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                if let Some(n) = self.element(top)? {
                    nodes.push(n);
                }
            } else if r.starts_with("{#") {
                nodes.push(self.block()?);
            } else if r.starts_with("{:") || r.starts_with("{/") {
                return Ok((nodes, self.block_end()?));
            } else if r.starts_with("{@html") && !r[6..].starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '$') {
                self.pos += 6;
                let expr = self.expr_until_brace()?;
                nodes.push(Node::Interp(Interp { expr, html: true, span: self.span(start, self.pos) }));
            } else if r.starts_with('{') {
                self.pos += 1;
                let expr = self.expr_until_brace()?;
                nodes.push(Node::Interp(Interp { expr, html: false, span: self.span(start, self.pos) }));
            } else {
                let n = r.find(['<', '{']).unwrap_or(r.len()).max(1);
                self.pos += n;
                nodes.push(Node::Text(Text { value: self.src[start..self.pos].to_string(), span: self.span(start, self.pos) }));
            }
        }
    }

    fn block_end(&mut self) -> R<End> {
        let start = self.pos;
        let closing = self.rest().starts_with("{/");
        self.pos += 2;
        let word = self.read_name().to_string();
        if closing {
            self.skip_ws();
            if !self.rest().starts_with('}') {
                return Err(self.err(self.span(start, self.pos), &format!("`{{/{word}` without `}}`"), &format!("write `{{/{word}}}`")));
            }
            self.pos += 1;
            let s = self.span(start, self.pos);
            return match word.as_str() {
                "if" => Ok(End::EndIf(s)),
                "each" => Ok(End::EndEach(s)),
                "await" => Ok(End::EndAwait(s)),
                "slot" => Ok(End::EndSlot(s)),
                _ => Err(self.err(s, &format!("unknown block `{{/{word}}}`"), "the blocks are `{/if}`, `{/each}`, `{/await}` and `{/slot}`")),
            };
        }
        match word.as_str() {
            "else" => {
                self.skip_ws();
                if self.rest().starts_with("if") && !self.rest()[2..].starts_with(|c: char| c.is_alphanumeric()) {
                    self.pos += 2;
                    let e = self.expr_until_brace()?;
                    return Ok(End::ElseIf(e, self.span(start, self.pos)));
                }
                if !self.rest().starts_with('}') {
                    return Err(self.err(self.span(start, self.pos), "malformed `{:else`", "write `{:else}` or `{:else if condition}`"));
                }
                self.pos += 1;
                Ok(End::Else(self.span(start, self.pos)))
            }
            "empty" => {
                self.skip_ws();
                if !self.rest().starts_with('}') {
                    return Err(self.err(self.span(start, self.pos), "malformed `{:empty`", "write `{:empty}`"));
                }
                self.pos += 1;
                Ok(End::Empty(self.span(start, self.pos)))
            }
            "then" | "catch" => {
                self.skip_ws();
                let binding = if self.rest().starts_with('}') {
                    self.pos += 1;
                    None
                } else {
                    let mut p = self.js_at(self.pos, self.src.len())?;
                    let pat = p.binding_target()?;
                    self.close_brace(&p)?;
                    Some(pat)
                };
                let s = self.span(start, self.pos);
                Ok(if word == "then" { End::Then(binding, s) } else { End::Catch(binding, s) })
            }
            _ => Err(self.err(self.span(start, self.pos), &format!("`{{:{word}}}` does not exist"), "use `{:else}`, `{:else if c}`, `{:empty}`, `{:then v}` or `{:catch e}`")),
        }
    }

    fn block(&mut self) -> R<Node> {
        let start = self.pos;
        self.pos += 2;
        let word = self.read_name().to_string();
        match word.as_str() {
            "if" => {
                let test = self.expr_until_brace()?;
                let mut branches = Vec::new();
                let mut branch_start = start;
                let mut test = Some(test);
                loop {
                    let (body, end) = self.children(false)?;
                    match end {
                        End::ElseIf(e, s) => {
                            branches.push(IfBranch { test: test.take().unwrap(), body, span: self.span(branch_start, s.start as usize) });
                            test = Some(e);
                            branch_start = s.start as usize;
                        }
                        End::Else(s) => {
                            branches.push(IfBranch { test: test.take().unwrap(), body, span: self.span(branch_start, s.start as usize) });
                            let (other, end) = self.children(false)?;
                            let End::EndIf(_) = end else {
                                return Err(self.err(end.span(self.span(start, start + 4)), &format!("expected `{{/if}}` but found {}", end.describe()), "close the block with `{/if}`"));
                            };
                            return Ok(Node::If(IfBlock { branches, otherwise: Some(other), span: self.span(start, self.pos) }));
                        }
                        End::EndIf(_) => {
                            branches.push(IfBranch { test: test.take().unwrap(), body, span: self.span(branch_start, self.pos) });
                            return Ok(Node::If(IfBlock { branches, otherwise: None, span: self.span(start, self.pos) }));
                        }
                        other => {
                            return Err(self.err(self.span(start, start + 5), &format!("unclosed `{{#if}}`: found {}", other.describe()), "close the block with `{/if}`"));
                        }
                    }
                }
            }
            "slot" => {
                // `{#slot item, i}`: names the child passes with `<slot item={…} />`.
                let mut names = Vec::new();
                loop {
                    self.skip_ws();
                    if self.rest().starts_with('}') {
                        self.pos += 1;
                        break;
                    }
                    if self.rest().starts_with(',') {
                        self.pos += 1;
                        continue;
                    }
                    let at = self.pos;
                    let n = self.read_name().to_string();
                    if n.is_empty() || !n.chars().all(|c| c.is_alphanumeric() || c == '_') || n.starts_with(|c: char| c.is_ascii_digit()) {
                        return Err(self.err(self.span(at, at + 1), "`{#slot}` takes the names of the slot props", "write `{#slot item}` or `{#slot item, index}`"));
                    }
                    names.push(Ident { name: n, span: self.span(at, self.pos) });
                }
                let (body, end) = self.children(false)?;
                let End::EndSlot(_) = end else {
                    return Err(self.err(self.span(start, start + 7), &format!("unclosed `{{#slot}}`: found {}", end.describe()), "close the block with `{/slot}`"));
                };
                Ok(Node::SlotScope(SlotScope { names, body, span: self.span(start, self.pos) }))
            }
            "each" => {
                let mut p = self.js_at(self.pos, self.src.len())?;
                let list = p.expression()?;
                if !p.current().is_word("as") {
                    return Err(Diag::new("B0002", self.file, p.current().span, "`{#each}` needs `as`", "write `{#each list as item (item.id)}`"));
                }
                // Re-enter the lexer after `as` to read the pattern.
                let after_as = p.current().span.end as usize;
                let mut p = self.js_at(after_as, self.src.len())?;
                let item = p.binding_target()?;
                let mut index = None;
                let mut key = None;
                if p.current().is_punct(",") {
                    let at = p.current().span.end as usize;
                    p = self.js_at(at, self.src.len())?;
                    let Pat::Ident(id) = p.binding_target()? else {
                        return Err(Diag::new("B0002", self.file, p.current().span, "the `{#each}` index must be a name", "write `{#each list as item, i (item.id)}`"));
                    };
                    index = Some(id);
                }
                if p.current().is_punct("(") {
                    let at = p.current().span.end as usize;
                    let mut kp = self.js_at(at, self.src.len())?;
                    key = Some(kp.expression()?);
                    if !kp.current().is_punct(")") {
                        return Err(Diag::new("B0002", self.file, kp.current().span, "missing `)` after the `{#each}` key", "write `(item.id)`"));
                    }
                    let at = kp.current().span.end as usize;
                    p = self.js_at(at, self.src.len())?;
                }
                self.close_brace(&p)?;
                let (body, end) = self.children(false)?;
                let (empty, end) = match end {
                    End::Empty(_) => {
                        let (e, end2) = self.children(false)?;
                        (Some(e), end2)
                    }
                    e => (None, e),
                };
                let End::EndEach(_) = end else {
                    return Err(self.err(self.span(start, start + 7), &format!("unclosed `{{#each}}`: found {}", end.describe()), "close the block with `{/each}`"));
                };
                Ok(Node::Each(EachBlock { list, item, index, key, body, empty, span: self.span(start, self.pos) }))
            }
            "await" => {
                let promise = self.expr_until_brace()?;
                let (pending, mut end) = self.children(false)?;
                let mut then = None;
                let mut catch = None;
                loop {
                    match end {
                        End::Then(b, s) if then.is_none() && catch.is_none() => {
                            let (body, e) = self.children(false)?;
                            then = Some(AwaitBranch { binding: b, body, span: s });
                            end = e;
                        }
                        End::Catch(b, s) if catch.is_none() => {
                            let (body, e) = self.children(false)?;
                            catch = Some(AwaitBranch { binding: b, body, span: s });
                            end = e;
                        }
                        End::EndAwait(_) => break,
                        other => {
                            return Err(self.err(self.span(start, start + 8), &format!("unclosed `{{#await}}`: found {}", other.describe()), "use `{#await p}…{:then v}…{:catch e}…{/await}`"));
                        }
                    }
                }
                Ok(Node::Await(AwaitBlock { promise, pending, then, catch, span: self.span(start, self.pos) }))
            }
            _ => Err(self.err(self.span(start, self.pos), &format!("unknown block `{{#{word}}}`"), "the blocks are `{#if}`, `{#each}`, `{#await}` and `{#slot}`")),
        }
    }

    fn attr_value(&mut self) -> R<(AttrValue, Option<(usize, usize)>)> {
        let r = self.rest();
        if r.starts_with('{') {
            self.pos += 1;
            return Ok((AttrValue::Expr(self.expr_until_brace()?), None));
        }
        if let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let vstart = self.pos + 1;
            let Some(len) = r[1..].find(q) else {
                return Err(self.err(self.span(self.pos, self.pos + 1), "unclosed attribute value", &format!("close the value with {q}")));
            };
            let vend = vstart + len;
            let raw = &self.src[vstart..vend];
            if !raw.contains('{') {
                self.pos = vend + 1;
                return Ok((AttrValue::Text(raw.to_string()), Some((vstart, vend))));
            }
            let mut parts = Vec::new();
            self.pos = vstart;
            loop {
                let rest = &self.src[self.pos..vend];
                match rest.find('{') {
                    Some(i) => {
                        if i > 0 {
                            parts.push(AttrPart::Text(rest[..i].to_string()));
                        }
                        self.pos += i + 1;
                        let e = self.expr_until_brace()?;
                        parts.push(AttrPart::Expr(e));
                        if self.pos > vend {
                            return Err(self.err(self.span(vstart, vend), "an `{expr}` crosses the attribute's closing quote", "close the expression with `}` before the quote"));
                        }
                    }
                    None => {
                        if !rest.is_empty() {
                            parts.push(AttrPart::Text(rest.to_string()));
                        }
                        break;
                    }
                }
            }
            self.pos = vend + 1;
            return Ok((AttrValue::Mixed(parts), Some((vstart, vend))));
        }
        let start = self.pos;
        let n = r.find(|c: char| c.is_whitespace() || c == '>' || c == '`' || r[..].starts_with("/>")).unwrap_or(r.len());
        let n = r[..n].find("/>").unwrap_or(n);
        self.pos += n;
        Ok((AttrValue::Text(self.src[start..self.pos].to_string()), Some((start, self.pos))))
    }

    fn attrs(&mut self) -> R<(Vec<RawAttr>, bool)> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            let r = self.rest();
            if r.starts_with("/>") {
                self.pos += 2;
                return Ok((out, true));
            }
            if r.starts_with('>') {
                self.pos += 1;
                return Ok((out, false));
            }
            if r.is_empty() {
                return Err(self.err(self.span(self.pos, self.pos), "unclosed tag", "close the tag with `>` or `/>`"));
            }
            let start = self.pos;
            if r.starts_with('{') {
                return Err(self.err(self.span(start, start + 1), "shorthand attribute `{x}` does not exist", "write the full name: `x={x}`"));
            }
            let full = self.read_name().to_string();
            if full.is_empty() {
                return Err(self.err(self.span(start, start + 1), &format!("unexpected character `{}` in the tag", r.chars().next().unwrap_or(' ')), "write attributes as `name=\"value\"` or `name={expr}`"));
            }
            let (kind, name) = match full.split_once(':') {
                Some(("on", n)) => (AttrKind::On, n.to_string()),
                Some(("bind", n)) => (AttrKind::Bind, n.to_string()),
                Some(("class", n)) => (AttrKind::Class, n.to_string()),
                Some(("style", n)) => (AttrKind::Style, n.to_string()),
                Some(("transition", n)) => (AttrKind::Transition, n.to_string()),
                _ => (AttrKind::Plain, full.clone()),
            };
            self.skip_ws();
            let (value, range) = if self.rest().starts_with('=') {
                self.pos += 1;
                self.skip_ws();
                self.attr_value()?
            } else {
                (AttrValue::None, None)
            };
            out.push(RawAttr { attr: Attr { kind, name, value, span: self.span(start, self.pos) }, value_range: range });
        }
    }

    fn raw_until_close(&mut self, tag: &str, open: usize) -> R<(usize, usize)> {
        let close = format!("</{tag}");
        let body_start = self.pos;
        let Some(i) = self.rest().find(&close) else {
            return Err(self.err(self.span(open, open + tag.len() + 1), &format!("unclosed `<{tag}>`"), &format!("add `</{tag}>`")));
        };
        let body_end = body_start + i;
        self.pos = body_end + close.len();
        self.skip_ws();
        if self.rest().starts_with('>') {
            self.pos += 1;
        }
        Ok((body_start, body_end))
    }

    fn element(&mut self, top: bool) -> R<Option<Node>> {
        let start = self.pos;
        self.pos += 1;
        let tag = self.read_name().to_string();
        let (raw_attrs, self_closing) = self.attrs()?;
        if top && (tag == "script" || tag == "style" || (tag == "meta" && raw_attrs.iter().any(|a| matches!(a.attr.name.as_str(), "props" | "seo" | "export" | "fallback" | "emits" | "expose" | "cache" | "tools" | "route" | "http" | "client" | "budget")))) {
            self.top_block(&tag, start, raw_attrs, self_closing)?;
            return Ok(None);
        }
        // A third-party `<script src>` is markup of `app.brain`'s `<head>` (the checker requires `integrity`, B0123).
        let external = tag == "script" && raw_attrs.iter().any(|a| a.attr.name == "src") && self.file.ends_with("app.brain");
        if (tag == "script" && !external) || tag == "style" {
            return Err(self.err(self.span(start, self.pos), &format!("`<{tag}>` can only go at the root level of the file"), &format!("move the `<{tag}>` outside any tag")));
        }
        let mut attrs: Vec<Attr> = raw_attrs.into_iter().map(|a| a.attr).collect();
        let is_component = tag.starts_with(|c: char| c.is_ascii_uppercase());
        // `<pre raw>` / `<code raw>`: the children are literal text (code samples with braces), escaped for HTML.
        if !is_component && !self_closing && attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == "raw" && a.value == AttrValue::None) {
            attrs.retain(|a| !(a.kind == AttrKind::Plain && a.name == "raw"));
            let close = format!("</{tag}>");
            let body_start = self.pos;
            let Some(rel) = self.src[body_start..].find(&close) else {
                let s = self.span(start, start + tag.len() + 1);
                return Err(self.err(s, &format!("unclosed `<{tag} raw>`"), &format!("add `</{tag}>`")));
            };
            let body_end = body_start + rel;
            let text = escape_raw(&self.src[body_start..body_end]);
            let tspan = self.span(body_start, body_end);
            self.pos = body_end + close.len();
            let span = self.span(start, self.pos);
            let children = if text.is_empty() { Vec::new() } else { vec![Node::Text(Text { value: text, span: tspan })] };
            return Ok(Some(Node::Element(Element { tag, attrs, children, span })));
        }
        let children = if self_closing || (!is_component && VOID.contains(&tag.to_ascii_lowercase().as_str())) {
            Vec::new()
        } else {
            let (children, end) = self.children(false)?;
            match end {
                End::Close(name, _) if name == tag => children,
                other => {
                    let s = self.span(start, start + tag.len() + 1);
                    return Err(self.err(s, &format!("unclosed `<{tag}>`: found {}", other.describe()), &format!("add `</{tag}>` or use `<{tag} />`")));
                }
            }
        };
        let span = self.span(start, self.pos);
        Ok(Some(if tag == "slot" {
            let name = attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == "name").and_then(|a| match &a.value {
                AttrValue::Text(t) => Some(t.clone()),
                _ => None,
            });
            Node::Slot(Slot { name, attrs: attrs.into_iter().filter(|a| !(a.kind == AttrKind::Plain && a.name == "name")).collect(), span })
        } else if is_component {
            Node::ComponentRef(ComponentRef { name: tag, attrs, children, span })
        } else {
            Node::Element(Element { tag, attrs, children, span })
        }))
    }

    fn top_block(&mut self, tag: &str, start: usize, attrs: Vec<RawAttr>, self_closing: bool) -> R<()> {
        match tag {
            "script" => {
                if self.seen_script {
                    return Err(self.err(self.span(start, self.pos), "there can only be one `<script>` per component", "merge all the code into a single `<script>`"));
                }
                self.seen_script = true;
                if self_closing {
                    return Ok(());
                }
                let (a, b) = self.raw_until_close("script", start)?;
                match self.js_at(a, b).and_then(|mut p| p.parse_program()) {
                    Ok(stmts) => self.out.script = stmts,
                    Err(d) => self.diags.push(d),
                }
            }
            "style" => {
                if self.out.style.is_some() {
                    return Err(self.err(self.span(start, self.pos), "there can only be one `<style>` per component", "merge all the CSS into a single `<style>`"));
                }
                if self_closing {
                    return Ok(());
                }
                let (a, b) = self.raw_until_close("style", start)?;
                match brain_css::parse(self.src, self.file, self.idx, a, b) {
                    Ok(s) => self.out.style = Some(s),
                    Err(d) => self.diags.push(d),
                }
            }
            _ => {
                let seo = attrs.iter().any(|a| a.attr.name == "seo");
                for a in attrs {
                    if a.attr.name == "props" {
                        let Some((vs, ve)) = a.value_range else {
                            return Err(self.err(a.attr.span, "`props` needs a quoted value", "write `<meta props=\"name:str='x' age:num=0\">`"));
                        };
                        if let Err(d) = self.props(vs, ve) {
                            self.diags.push(d);
                        }
                    } else {
                        let (value, expr) = match a.attr.value {
                            AttrValue::Text(t) => (Some(t), None),
                            AttrValue::None => (None, None),
                            // `<meta seo title={props.post.title}>` (§32).
                            AttrValue::Expr(x) if seo => (None, Some(x)),
                            AttrValue::Expr(Expr::Str { value, .. }) => (Some(value), None),
                            _ if seo => return Err(self.err(a.attr.span, "a `<meta seo>` value is text in quotes or one expression in braces", "write `title=\"Users\"` or ``title={`${props.user.name} · Users`}``")),
                            _ => return Err(self.err(a.attr.span, "`<meta>` attributes are fixed text", "write the value in quotes, without `{}`")),
                        };
                        self.out.meta.push(MetaAttr { name: a.attr.name, value, expr, span: a.attr.span });
                    }
                }
            }
        }
        Ok(())
    }

    /// `name:type=default name:type? name:type`
    fn props(&mut self, start: usize, end: usize) -> R<()> {
        let mut pos = start;
        let types = "str num bool list obj fn any";
        loop {
            let r = &self.src[pos..end];
            let t = r.trim_start();
            pos += r.len() - t.len();
            if pos >= end {
                return Ok(());
            }
            let pstart = pos;
            let name_len = t.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).unwrap_or(t.len());
            let name = &t[..name_len];
            pos += name_len;
            if name.is_empty() {
                return Err(Diag::new("B0004", self.file, self.span(pos, pos + 1), format!("malformed prop at `{}`", t.split_whitespace().next().unwrap_or("")), format!("each prop is `name:type=default`, `name:type?` or `name:type`; types: {types}")));
            }
            if !self.src[pos..end].starts_with(':') {
                return Err(Diag::new("B0004", self.file, self.span(pstart, pos), format!("prop `{name}` does not declare a type"), format!("write `{name}:str` (types: {types})")));
            }
            pos += 1;
            let r = &self.src[pos..end];
            let ty_len = r.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(r.len());
            let ty_str = &r[..ty_len];
            let Some(ty) = PropType::parse(ty_str) else {
                return Err(Diag::new("B0004", self.file, self.span(pos, pos + ty_len.max(1)), format!("unknown type `{ty_str}` in prop `{name}`"), format!("use one of: {types}")));
            };
            pos += ty_len;
            // `value:str~` — bindable model prop (INTERNALS §12): the child may write it, the parent uses `bind:`.
            let bindable = self.src[pos..end].starts_with('~');
            if bindable {
                pos += 1;
            }
            let mut optional = false;
            let mut default = None;
            if self.src[pos..end].starts_with('?') {
                optional = true;
                pos += 1;
            } else if self.src[pos..end].starts_with('=') {
                pos += 1;
                let mut p = self.js_at(pos, end)?;
                let e = p.assignment()?;
                pos = e.span().end as usize;
                default = Some(e);
            }
            let next = self.src[pos..end].chars().next();
            if next.is_some_and(|c| !c.is_whitespace()) {
                return Err(Diag::new("B0004", self.file, self.span(pos, pos + 1), format!("unexpected text after prop `{name}`"), "separate props with spaces: `a:num=0 b:str='x'`"));
            }
            self.out.props.push(Prop { name: name.to_string(), ty, optional, default, bindable, span: self.span(pstart, pos) });
        }
    }
}

/// Literal text for HTML: `<` and `>` escaped, `&` escaped unless it already starts an entity (`&lt;`, `&#123;`).
fn escape_raw(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => {
                let rest = &s[i + 1..];
                let end = rest.find(';').filter(|e| *e > 0 && *e <= 8);
                let entity = end.is_some_and(|e| {
                    let name = &rest[..e];
                    name.chars().all(|ch| ch.is_ascii_alphanumeric()) || (name.starts_with('#') && name[1..].trim_start_matches(['x', 'X']).chars().all(|ch| ch.is_ascii_hexdigit()))
                });
                out.push_str(if entity { "&" } else { "&amp;" });
            }
            c => out.push(c),
        }
    }
    out
}
