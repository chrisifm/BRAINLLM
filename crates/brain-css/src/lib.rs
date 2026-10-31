//! Hand-written CSS for `.brain`: a tolerant block parser, `@mixin`/`@include`, token (`var(--x)`)
//! extraction, global-selector detection and the `@layer` + `@scope` wrapper of SPEC §7-§8.
//! Native nesting is kept as-is (every target browser supports it); nothing is flattened.

pub mod intern;

use brain_lex::{Diag, LineIndex, Span};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct CssSheet {
    pub items: Vec<CssItem>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CssItem {
    Rule { selector: String, items: Vec<CssItem>, span: Span },
    Decl { name: String, value: String, span: Span },
    /// `name` without the `@`. `block` is `None` for statements like `@include x;`.
    At { name: String, prelude: String, block: Option<Vec<CssItem>>, span: Span },
}

impl CssItem {
    pub fn span(&self) -> Span {
        match self {
            CssItem::Rule { span, .. } | CssItem::Decl { span, .. } | CssItem::At { span, .. } => *span,
        }
    }
}

struct Parser<'a> {
    src: &'a str,
    file: &'a str,
    idx: &'a LineIndex,
    pos: usize,
    end: usize,
    /// Open blocks around `pos` (bounded by [`MAX_CSS_DEPTH`]).
    depth: u32,
}

/// Blocks nested deeper than this are a diagnostic, not a stack overflow in the parser or later passes.
const MAX_CSS_DEPTH: u32 = 256;

/// Parses `src[start..end]` (the inside of a `<style>`), with spans absolute in the file.
pub fn parse(src: &str, file: &str, idx: &LineIndex, start: usize, end: usize) -> Result<CssSheet, Diag> {
    let mut p = Parser { src, file, idx, pos: start, end, depth: 0 };
    let items = p.block_items(false)?;
    Ok(CssSheet { items, span: idx.span(src, start, end) })
}

/// Parses a standalone stylesheet such as `tokens.cssbrain`.
pub fn parse_str(src: &str, file: &str) -> Result<CssSheet, Diag> {
    let idx = LineIndex::new(src);
    parse(src, file, &idx, 0, src.len())
}

impl<'a> Parser<'a> {
    fn span(&self, a: usize, b: usize) -> Span {
        self.idx.span(self.src, a, b)
    }
    fn err(&self, at: usize, msg: &str, fix: &str) -> Diag {
        Diag::new("B0003", self.file, self.span(at, at + 1), msg, fix)
    }

    fn skip_ws_comments(&mut self) -> Result<(), Diag> {
        loop {
            let rest = &self.src[self.pos..self.end];
            let trimmed = rest.trim_start();
            self.pos += rest.len() - trimmed.len();
            if trimmed.starts_with("/*") {
                match trimmed[2..].find("*/") {
                    Some(i) => self.pos += i + 4,
                    None => return Err(self.err(self.pos, "unclosed CSS comment", "close the comment with `*/`")),
                }
            } else {
                return Ok(());
            }
        }
    }

    /// Reads raw text until `{`, `;` or `}` at depth 0, skipping strings, parens and comments.
    /// Returns the text (comments removed) and the stop char (`None` at the end of input).
    fn read_until_stop(&mut self) -> Result<(String, Option<char>), Diag> {
        let mut out = String::new();
        let mut depth = 0i32;
        while self.pos < self.end {
            let rest = &self.src[self.pos..self.end];
            let c = rest.chars().next().unwrap_or(' ');
            if rest.starts_with("/*") {
                match rest[2..].find("*/") {
                    Some(i) => {
                        self.pos += i + 4;
                        out.push(' ');
                        continue;
                    }
                    None => return Err(self.err(self.pos, "unclosed CSS comment", "close the comment with `*/`")),
                }
            }
            match c {
                '"' | '\'' => {
                    let start = self.pos;
                    self.pos += 1;
                    loop {
                        let Some(d) = self.src[self.pos..self.end].chars().next() else {
                            return Err(self.err(start, "unclosed CSS string", "close the string with the same quote"));
                        };
                        self.pos += d.len_utf8();
                        if d == '\\' {
                            if let Some(e) = self.src[self.pos..self.end].chars().next() {
                                self.pos += e.len_utf8();
                            }
                        } else if d == c {
                            break;
                        }
                    }
                    out.push_str(&self.src[start..self.pos]);
                    continue;
                }
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                '{' | ';' | '}' if depth <= 0 => return Ok((out, Some(c))),
                _ => {}
            }
            out.push(c);
            self.pos += c.len_utf8();
        }
        Ok((out, None))
    }

    fn block_items(&mut self, nested: bool) -> Result<Vec<CssItem>, Diag> {
        let mut items = Vec::new();
        loop {
            self.skip_ws_comments()?;
            if self.pos >= self.end {
                if nested {
                    return Err(self.err(self.end.saturating_sub(1), "unclosed CSS block", "add the missing `}`"));
                }
                return Ok(items);
            }
            let start = self.pos;
            let (text, stop) = self.read_until_stop()?;
            let text = collapse_ws(&text);
            match stop {
                Some('{') => {
                    if self.depth >= MAX_CSS_DEPTH {
                        return Err(self.err(self.pos, "the CSS is nested too deeply to compile", "flatten the nested rules"));
                    }
                    self.pos += 1;
                    self.depth += 1;
                    let inner = self.block_items(true);
                    self.depth -= 1;
                    let inner = inner?;
                    let span = self.span(start, self.pos);
                    if let Some(at) = text.strip_prefix('@') {
                        let (name, prelude) = split_at_rule(at);
                        items.push(CssItem::At { name, prelude, block: Some(inner), span });
                    } else {
                        items.push(CssItem::Rule { selector: text, items: inner, span });
                    }
                }
                Some(';') | Some('}') | None => {
                    if stop == Some(';') {
                        self.pos += 1;
                    }
                    let span = self.span(start, self.pos);
                    if !text.is_empty() {
                        if let Some(at) = text.strip_prefix('@') {
                            let (name, prelude) = split_at_rule(at);
                            items.push(CssItem::At { name, prelude, block: None, span });
                        } else if let Some(colon) = text.find(':') {
                            let name = text[..colon].trim().to_string();
                            let value = if name.starts_with("--") {
                                // Custom properties carry raw token streams, including whitespace/comments.
                                let end = if stop == Some(';') { self.pos - 1 } else { self.pos };
                                let raw = &self.src[start..end];
                                raw.find(':').map(|at| raw[at + 1..].to_string()).unwrap_or_default()
                            } else { text[colon + 1..].trim().to_string() };
                            items.push(CssItem::Decl { name, value, span });
                        } else {
                            return Err(Diag::new(
                                "B0003",
                                self.file,
                                span,
                                format!("invalid CSS declaration `{text}`"),
                                "write `property: value;` or open a block with `{ }`",
                            ));
                        }
                    }
                    if stop == Some('}') {
                        if !nested {
                            return Err(self.err(self.pos, "stray `}` in the CSS", "remove the unmatched closing brace"));
                        }
                        self.pos += 1;
                        return Ok(items);
                    }
                    if stop.is_none() && nested {
                        return Err(self.err(self.end.saturating_sub(1), "unclosed CSS block", "add the missing `}`"));
                    }
                }
                Some(_) => unreachable!(),
            }
        }
    }
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut ws = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            ws = true;
        } else {
            if ws {
                out.push(' ');
            }
            ws = false;
            out.push(c);
        }
    }
    out
}

fn split_at_rule(s: &str) -> (String, String) {
    let i = s.find(|c: char| c.is_whitespace() || c == '(').unwrap_or(s.len());
    (s[..i].to_string(), s[i..].trim().to_string())
}

/// Replaces every `@include name;` with the body of the matching top-level `@mixin name { }`,
/// and removes the mixin definitions. Unknown mixins are B0303.
pub fn expand_mixins(sheet: &CssSheet, file: &str) -> Result<CssSheet, Vec<Diag>> {
    let mut mixins: HashMap<String, Vec<CssItem>> = HashMap::new();
    for it in &sheet.items {
        if let CssItem::At { name, prelude, block: Some(b), .. } = it
            && name == "mixin"
        {
            mixins.insert(prelude.clone(), b.clone());
        }
    }
    let mut diags = Vec::new();
    let items = expand_list(&sheet.items, &mixins, file, &mut diags, 0);
    if diags.is_empty() { Ok(CssSheet { items, span: sheet.span }) } else { Err(diags) }
}

fn expand_list(items: &[CssItem], mixins: &HashMap<String, Vec<CssItem>>, file: &str, diags: &mut Vec<Diag>, depth: u32) -> Vec<CssItem> {
    let mut out = Vec::new();
    for it in items {
        match it {
            CssItem::At { name, .. } if name == "mixin" => {}
            CssItem::At { name, prelude, block: None, span } if name == "include" => match mixins.get(prelude.as_str()) {
                Some(body) if depth < 16 => out.extend(expand_list(body, mixins, file, diags, depth + 1)),
                Some(_) => diags.push(Diag::new("B0303", file, *span, format!("`@include {prelude}` is recursive"), "a mixin cannot include itself")),
                None => {
                    let known: Vec<&str> = mixins.keys().map(String::as_str).collect();
                    let fix = if known.is_empty() {
                        format!("define `@mixin {prelude} {{ ... }}` in this <style>")
                    } else {
                        format!("available mixins: {}", known.join(", "))
                    };
                    diags.push(Diag::new("B0303", file, *span, format!("mixin `{prelude}` does not exist"), fix));
                }
            },
            CssItem::Rule { selector, items, span } => out.push(CssItem::Rule {
                selector: selector.clone(),
                items: expand_list(items, mixins, file, diags, depth),
                span: *span,
            }),
            CssItem::At { name, prelude, block: Some(b), span } => out.push(CssItem::At {
                name: name.clone(),
                prelude: prelude.clone(),
                block: Some(expand_list(b, mixins, file, diags, depth)),
                span: *span,
            }),
            other => out.push(other.clone()),
        }
    }
    out
}

/// Every `var(--name)` used in declaration values, with the span of its declaration.
pub fn var_refs(sheet: &CssSheet) -> Vec<(String, Span)> {
    fn walk(items: &[CssItem], out: &mut Vec<(String, Span)>) {
        for it in items {
            match it {
                CssItem::Decl { value, span, .. } => {
                    let mut rest = value.as_str();
                    while let Some(i) = rest.find("var(") {
                        rest = &rest[i + 4..];
                        let name: String = rest
                            .trim_start()
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                            .collect();
                        if name.starts_with("--") {
                            out.push((name, *span));
                        }
                    }
                }
                CssItem::Rule { items, .. } => walk(items, out),
                CssItem::At { block: Some(b), .. } => walk(b, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&sheet.items, &mut out);
    out
}

/// Custom properties declared anywhere in a sheet (the token file), in order.
pub fn custom_props(sheet: &CssSheet) -> Vec<(String, String)> {
    fn walk(items: &[CssItem], out: &mut Vec<(String, String)>) {
        for it in items {
            match it {
                CssItem::Decl { name, value, .. } if name.starts_with("--") => out.push((name.clone(), value.clone())),
                CssItem::Rule { items, .. } => walk(items, out),
                CssItem::At { block: Some(b), .. } => walk(b, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&sheet.items, &mut out);
    out
}

/// Top-level rules whose selector escapes the component (`:root`, `html`, `body`, `*`, `:global`).
pub fn global_selectors(sheet: &CssSheet) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    for it in &sheet.items {
        if let CssItem::Rule { selector, span, .. } = it {
            for part in selector.split(',') {
                let first = part.trim().split(|c: char| c.is_whitespace() || c == '>' || c == '+' || c == '~').next().unwrap_or("");
                let base = first.split([':', '.', '[', '#']).next().unwrap_or("");
                if first.starts_with(":root") || part.contains(":global(") || first.starts_with(":global") || base == "html" || base == "body" || first == "*" {
                    out.push((part.trim().to_string(), *span));
                }
            }
        }
    }
    out
}

/// B0307: selectors that `@scope ([data-b]) to ([data-b])` can never match. `tags` = the element names written by
/// this component (`None` = unknown, e.g. it renders `{@html}`; the absent-tag check is skipped). Leading
/// `html`/`body`/`:root` at the top level is B0301 and is not repeated here. Returns (selector, reason, span).
pub fn dead_scoped_selectors(sheet: &CssSheet, tags: Option<&std::collections::HashSet<String>>) -> Vec<(String, String, Span)> {
    let mut out = Vec::new();
    dead_in(&sheet.items, tags, true, false, &mut out);
    out
}

fn dead_in(items: &[CssItem], tags: Option<&std::collections::HashSet<String>>, top: bool, at: bool, out: &mut Vec<(String, String, Span)>) {
    for it in items {
        match it {
            CssItem::Rule { selector, items, span } => {
                for part in split_top(selector, ',') {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    let types = type_selectors(part);
                    let lead = types.first().is_some_and(|(_, i)| *i == 0) || part.starts_with(":root");
                    let page = types.iter().map(|(t, _)| t.as_str()).find(|t| matches!(*t, "html" | "body")).map(str::to_string).or_else(|| part.contains(":root").then(|| ":root".to_string()));
                    if let Some(p) = page {
                        let b0301 = top && !at && lead && {
                            let first = part.split(|c: char| c.is_whitespace() || matches!(c, '>' | '+' | '~')).next().unwrap_or("");
                            let base = first.split([':', '.', '[', '#']).next().unwrap_or("");
                            base == "html" || base == "body" || first.starts_with(":root")
                        };
                        if !b0301 {
                            out.push((part.to_string(), format!("`{p}` is outside every component, so `@scope` never matches it"), *span));
                        }
                        continue;
                    }
                    if top {
                        let (first, rest) = first_compound(part);
                        let rest = rest.trim_start();
                        if (first == ":scope" || first == "&") && (rest.starts_with('~') || rest.starts_with('+')) {
                            out.push((part.to_string(), "siblings of the component root are outside its `@scope`".to_string(), *span));
                            continue;
                        }
                    }
                    if let Some(tags) = tags {
                        if let Some((t, _)) = types.iter().find(|(t, _)| !tags.contains(t)) {
                            out.push((part.to_string(), format!("this component writes no `<{t}>`; child components and slot content are outside its `@scope`"), *span));
                        }
                    }
                }
                dead_in(items, tags, false, at, out);
            }
            CssItem::At { name, block: Some(b), .. } if matches!(name.as_str(), "media" | "supports" | "container" | "layer") => dead_in(b, tags, top, true, out),
            _ => {}
        }
    }
}

/// Type selectors of one complex selector with their byte offset, lowercased. Arguments of `:where()`, `:is()`
/// and `:has()` count; those of `:not()`, `:nth-*()` and the rest do not.
fn type_selectors(sel: &str) -> Vec<(String, usize)> {
    let b = sel.as_bytes();
    let mut out = Vec::new();
    let mut stack: Vec<bool> = Vec::new(); // per open paren: do type selectors inside count?
    let mut i = 0;
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'-' || c == b'_';
    while i < b.len() {
        let c = b[i];
        match c {
            b'[' => {
                while i < b.len() && b[i] != b']' {
                    i += 1;
                }
            }
            b'"' | b'\'' => {
                i += 1;
                while i < b.len() && b[i] != c {
                    i += 1;
                }
            }
            b':' => {
                while i < b.len() && b[i] == b':' {
                    i += 1;
                }
                let s = i;
                while i < b.len() && ident(b[i]) {
                    i += 1;
                }
                if i < b.len() && b[i] == b'(' {
                    stack.push(matches!(&sel[s..i], "where" | "is" | "has" | "matches"));
                    i += 1;
                }
                continue;
            }
            b'.' | b'#' => {
                i += 1;
                while i < b.len() && ident(b[i]) {
                    i += 1;
                }
                continue;
            }
            b'(' => stack.push(false),
            b')' => {
                stack.pop();
            }
            c if c.is_ascii_alphabetic() && stack.iter().all(|x| *x) => {
                let s = i;
                while i < b.len() && ident(b[i]) {
                    i += 1;
                }
                // `ns|tag` and `n` of `2n+1` never reach here (`|` and digits are skipped below).
                out.push((sel[s..i].to_ascii_lowercase(), s));
                continue;
            }
            c if ident(c) => {
                while i < b.len() && ident(b[i]) {
                    i += 1;
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Prints items compactly (one rule per line; the minifier owns byte-level squeezing).
pub fn print_items(items: &[CssItem], out: &mut String, indent: usize) {
    let pad = "  ".repeat(indent);
    for it in items {
        match it {
            CssItem::Decl { name, value, .. } => {
                if name.starts_with("--") { out.push_str(&format!("{pad}{name}:{value};\n")); }
                else { out.push_str(&format!("{pad}{name}: {value};\n")); }
            },
            CssItem::Rule { selector, items, .. } => {
                out.push_str(&format!("{pad}{selector} {{\n"));
                print_items(items, out, indent + 1);
                out.push_str(&format!("{pad}}}\n"));
            }
            CssItem::At { name, prelude, block, .. } => {
                let sep = if prelude.is_empty() { "" } else { " " };
                match block {
                    Some(b) => {
                        out.push_str(&format!("{pad}@{name}{sep}{prelude} {{\n"));
                        print_items(b, out, indent + 1);
                        out.push_str(&format!("{pad}}}\n"));
                    }
                    None => out.push_str(&format!("{pad}@{name}{sep}{prelude};\n")),
                }
            }
        }
    }
}

/// At-rules that must not live inside `@scope`.
fn is_hoisted(it: &CssItem) -> bool {
    matches!(it, CssItem::At { name, .. } if matches!(name.as_str(), "keyframes" | "font-face" | "property" | "import" | "charset"))
}

/// Wraps an (already mixin-expanded) sheet as SPEC §7-§8:
/// `@layer <layer> { @scope ([data-b="hash"]) to ([data-b]) { ... } }`.
pub fn scoped(sheet: &CssSheet, hash: &str, layer: &str) -> String {
    let (hoisted, scoped): (Vec<CssItem>, Vec<CssItem>) = sheet.items.iter().cloned().partition(is_hoisted);
    let scoped: Vec<CssItem> = scoped.into_iter().map(root_anchor).collect();
    let mut out = format!("@layer {layer} {{\n");
    print_items(&hoisted, &mut out, 1);
    if !scoped.is_empty() {
        out.push_str(&format!("  @scope ([data-b=\"{hash}\"]) to ([data-b]) {{\n"));
        print_items(&scoped, &mut out, 2);
        out.push_str("  }\n");
    }
    out.push_str("}\n");
    out
}

/// Inside `@scope`, a selector without `:scope`/`&` gets an implicit `:scope ` descendant
/// combinator, so it never matches the component's root element (verified in Chrome 154).
/// Each top-level selector is therefore also emitted anchored to the root:
/// `.todo li` → `:where(:scope):is(.todo) li, .todo li` (`:where` keeps the specificity).
fn root_anchor(it: CssItem) -> CssItem {
    match it {
        CssItem::Rule { selector, items, span } => {
            let parts: Vec<String> = split_top(&selector, ',')
                .into_iter()
                .flat_map(|part| {
                    let part = part.trim().to_string();
                    if part.starts_with('&') || part.starts_with(":scope") || part.is_empty() {
                        return vec![part];
                    }
                    let (first, rest) = first_compound(&part);
                    vec![format!(":where(:scope):is({first}){rest}"), part]
                })
                .collect();
            CssItem::Rule { selector: parts.join(", "), items, span }
        }
        CssItem::At { name, prelude, block: Some(b), span } if matches!(name.as_str(), "media" | "supports" | "container" | "layer") => {
            CssItem::At { name, prelude, block: Some(b.into_iter().map(root_anchor).collect()), span }
        }
        other => other,
    }
}

/// Splits on `sep` outside parentheses, brackets and strings.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut last) = (0i32, None::<char>, 0usize);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => depth -= 1,
            (None, c) if c == sep && depth == 0 => {
                out.push(&s[last..i]);
                last = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[last..]);
    out
}

/// `li.done > span` → (`li.done`, ` > span`): the first compound selector and the rest.
fn first_compound(s: &str) -> (&str, &str) {
    let (mut depth, mut quote) = (0i32, None::<char>);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => depth -= 1,
            (None, c) if depth == 0 && (c.is_whitespace() || c == '>' || c == '+' || c == '~') => return (&s[..i], &s[i..]),
            _ => {}
        }
    }
    (s, "")
}

/// Order of the cascade layers every build declares first.
pub const LAYER_ORDER: &str = "@layer reset, tokens, ui, comp, view;";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nesting_and_decls() {
        let s = parse_str("button { padding: var(--space-2); &.hot { color: var(--bad); } }", "c.css").unwrap();
        let CssItem::Rule { selector, items, .. } = &s.items[0] else { panic!() };
        assert_eq!(selector, "button");
        assert_eq!(items.len(), 2);
        assert_eq!(var_refs(&s).iter().map(|v| v.0.as_str()).collect::<Vec<_>>(), vec!["--space-2", "--bad"]);
    }

    #[test]
    fn mixins_expand_and_unknown_is_b0303() {
        let s = parse_str("@mixin card { padding: 1px; } .a { @include card; color: red }", "c.css").unwrap();
        let e = expand_mixins(&s, "c.css").unwrap();
        let mut out = String::new();
        print_items(&e.items, &mut out, 0);
        assert_eq!(out, ".a {\n  padding: 1px;\n  color: red;\n}\n");
        let bad = parse_str(".a { @include nope; }", "c.css").unwrap();
        assert_eq!(expand_mixins(&bad, "c.css").unwrap_err()[0].code, "B0303");
    }

    #[test]
    fn strings_with_braces_and_semicolons() {
        let s = parse_str(".a::before { content: \"{;}\"; }", "c.css").unwrap();
        let CssItem::Rule { items, .. } = &s.items[0] else { panic!() };
        assert_eq!(items[0], CssItem::Decl { name: "content".into(), value: "\"{;}\"".into(), span: items[0].span() });
    }

    #[test]
    fn global_selectors_are_found() {
        let s = parse_str(":root { --x: 1 } body, .a { margin: 0 } .b { }", "c.css").unwrap();
        let g: Vec<String> = global_selectors(&s).into_iter().map(|g| g.0).collect();
        assert_eq!(g, vec![":root", "body"]);
        // `:global(…)` is not supported anywhere in a selector; past the first compound the browser dropped the rule
        let s = parse_str(".x :global(a) { color: red } .y > :global(.z) { } .w { }", "c.css").unwrap();
        let g: Vec<String> = global_selectors(&s).into_iter().map(|g| g.0).collect();
        assert_eq!(g, vec![".x :global(a)", ".y > :global(.z)"]);
    }

    #[test]
    fn scoped_output_hoists_keyframes() {
        let s = parse_str("@keyframes k { from { opacity: 0 } } .a { animation: k 1s }", "c.css").unwrap();
        let out = scoped(&s, "abc123", "comp");
        assert!(out.starts_with("@layer comp {\n  @keyframes k {"));
        assert!(out.contains("@scope ([data-b=\"abc123\"]) to ([data-b]) {\n    :where(:scope):is(.a), .a {"));
    }

    #[test]
    fn root_anchoring_keeps_combinators_and_lists() {
        let s = parse_str(".todo li, button.on > span:is(.a, .b), & .x { color: red } @media (width > 1px) { p { color: blue } }", "c.css").unwrap();
        let out = scoped(&s, "h", "comp");
        assert!(out.contains(":where(:scope):is(.todo) li, .todo li, :where(:scope):is(button.on) > span:is(.a, .b), button.on > span:is(.a, .b), & .x {"), "{out}");
        assert!(out.contains(":where(:scope):is(p), p {"), "{out}");
    }
}
