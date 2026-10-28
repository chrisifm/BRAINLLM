//! Own Markdown renderer for content collections (no third-party parser, CONTRIBUTING invariant 1). CommonMark subset
//! plus GFM tables, task lists and strikethrough: ATX/setext headings with unique ids, paragraphs, emphasis, code spans
//! and fences, block quotes, nested lists, links and images (inline and reference), autolinks, hard breaks, thematic
//! breaks. Raw HTML is escaped except a small allowlist (`details`, `summary`, `section faq`, `figure`…) with
//! attributes reduced to `id`, `open` and `faq`; `javascript:`-style URLs are dropped. Indented code blocks are not
//! supported (use fences).

use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rendered {
    pub html: String,
    /// `(level, text, id)` of every heading.
    pub headings: Vec<(u8, String, String)>,
    /// Plain text (for word count, reading time and the excerpt).
    pub text: String,
    /// Plain text of the first paragraph.
    pub excerpt: String,
    /// `src` of the first image.
    pub first_image: Option<String>,
}

const ALLOWED_TAGS: &[&str] = &[
    "details",
    "summary",
    "section",
    "figure",
    "figcaption",
    "kbd",
    "sup",
    "sub",
    "br",
    "mark",
    "abbr",
    "small",
    "u",
];
const ALLOWED_ATTRS: &[&str] = &["id", "open", "faq"];

#[derive(Default)]
struct Ctx {
    refs: HashMap<String, (String, Option<String>)>,
    ids: HashMap<String, usize>,
    out: Rendered,
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Heading slug: lowercase letters and digits, words joined by `-`.
pub fn slug(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            o.push(c);
        } else if (c.is_whitespace() || c == '-' || c == '_') && !o.ends_with('-') && !o.is_empty()
        {
            o.push('-');
        }
    }
    o.trim_end_matches('-').to_string()
}

/// Drops URLs with a script-capable scheme.
fn safe_url(u: &str) -> String {
    let lower: String = u
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    if ["javascript:", "vbscript:", "data:text/html"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        "#".into()
    } else {
        u.trim().to_string()
    }
}

fn indent(l: &str) -> usize {
    l.chars().take_while(|c| *c == ' ').count()
        + l.chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .filter(|c| *c == '\t')
            .count()
            * 3
}

fn is_blank(l: &str) -> bool {
    l.trim().is_empty()
}

fn fence_open(l: &str) -> Option<(char, usize, String)> {
    if indent(l) > 3 {
        return None;
    }
    let t = l.trim_start();
    let c = t.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = t.chars().take_while(|x| *x == c).count();
    let info = t[n..].trim().to_string();
    (n >= 3 && !(c == '`' && info.contains('`'))).then_some((c, n, info))
}

fn atx(l: &str) -> Option<(u8, String)> {
    if indent(l) > 3 {
        return None;
    }
    let t = l.trim_start();
    let n = t.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&n) {
        return None;
    }
    let rest = &t[n..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let mut text = rest.trim();
    let stripped = text.trim_end_matches('#');
    if stripped.is_empty() || stripped.ends_with([' ', '\t']) {
        text = stripped.trim_end();
    }
    Some((n as u8, text.to_string()))
}

fn hr(l: &str) -> bool {
    if indent(l) > 3 {
        return false;
    }
    let t: String = l.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

/// List marker: `(ordered, start number, content column, marker char)`.
fn marker(l: &str) -> Option<(bool, u32, usize, char)> {
    let ind = indent(l);
    if ind > 3 {
        return None;
    }
    let t = l.trim_start();
    let first = t.chars().next()?;
    if matches!(first, '-' | '*' | '+') {
        let rest = &t[1..];
        if rest.is_empty() || rest.starts_with([' ', '\t']) {
            let pad = rest.chars().take_while(|c| *c == ' ').count().clamp(1, 4);
            return Some((
                false,
                0,
                ind + 1 + if rest.trim().is_empty() { 1 } else { pad },
                first,
            ));
        }
        return None;
    }
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 9 {
        return None;
    }
    let after = &t[digits.len()..];
    let d = after.chars().next()?;
    if d != '.' && d != ')' {
        return None;
    }
    let rest = &after[1..];
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let pad = rest.chars().take_while(|c| *c == ' ').count().clamp(1, 4);
    Some((
        true,
        digits.parse().unwrap_or(1),
        ind + digits.len() + 1 + pad,
        d,
    ))
}

fn table_cells(l: &str) -> Vec<String> {
    let t = l.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn delimiter_row(l: &str) -> Option<Vec<&'static str>> {
    if !l.contains('-') {
        return None;
    }
    let cells = table_cells(l);
    cells
        .iter()
        .map(|c| {
            let ok =
                !c.is_empty() && c.trim_matches(':').chars().all(|x| x == '-') && c.contains('-');
            ok.then(|| match (c.starts_with(':'), c.ends_with(':')) {
                (true, true) => "center",
                (false, true) => "right",
                (true, false) => "left",
                _ => "",
            })
        })
        .collect()
}

/// A block-level start that interrupts a paragraph.
fn interrupts(l: &str) -> bool {
    atx(l).is_some()
        || fence_open(l).is_some()
        || hr(l)
        || l.trim_start().starts_with('>')
        || html_block(l)
        || marker(l).is_some_and(|(ord, n, _, _)| !ord || n == 1)
            && !l.trim().ends_with(['-', '*', '+'])
}

fn html_block(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with('<') && tag_name(&t[1..]).is_some_and(|n| ALLOWED_TAGS.contains(&n.as_str()))
}

fn tag_name(s: &str) -> Option<String> {
    let s = s.strip_prefix('/').unwrap_or(s);
    let n: String = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (!n.is_empty()).then(|| n.to_ascii_lowercase())
}

/// Re-emits an allowlisted tag (`<…>` at the start of `s`) with only the allowed attributes; `None` if not allowed.
fn safe_tag(s: &str) -> Option<(String, usize)> {
    let end = s.find('>')?;
    let inner = &s[1..end];
    let closing = inner.starts_with('/');
    let name = tag_name(inner)?;
    if !ALLOWED_TAGS.contains(&name.as_str()) {
        return None;
    }
    if closing {
        return Some((format!("</{name}>"), end + 1));
    }
    let attrs =
        crate::html::parse_attrs(&inner[1 + name.len().min(inner.len().saturating_sub(1))..]);
    let mut o = format!("<{name}");
    for (k, v) in attrs
        .iter()
        .filter(|(k, _)| ALLOWED_ATTRS.contains(&k.as_str()))
    {
        if v.is_empty() {
            o.push_str(&format!(" {k}"));
        } else {
            o.push_str(&format!(" {k}=\"{}\"", esc(v)));
        }
    }
    o.push('>');
    Some((o, end + 1))
}

/// Renders Markdown (without frontmatter) to HTML.
pub fn render(src: &str) -> Rendered {
    let src = src.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = src.split('\n').collect();
    let mut ctx = Ctx::default();
    // pass 1: link reference definitions (outside fences)
    let mut in_fence: Option<(char, usize)> = None;
    lines.retain(|l| {
        if let Some((c, n)) = in_fence {
            let t = l.trim();
            if t.chars().take_while(|x| *x == c).count() >= n && t.trim_start_matches(c).is_empty()
            {
                in_fence = None;
            }
            return true;
        }
        if let Some((c, n, _)) = fence_open(l) {
            in_fence = Some((c, n));
            return true;
        }
        if indent(l) <= 3
            && let Some(rest) = l.trim_start().strip_prefix('[')
            && let Some((label, def)) = rest.split_once("]:")
            && !label.is_empty()
            && !label.contains(']')
        {
            let def = def.trim();
            let (url, title) = match def.split_once(char::is_whitespace) {
                Some((u, t)) => (
                    u,
                    Some(t.trim().trim_matches(['"', '\'', '(', ')']).to_string()),
                ),
                None => (def, None),
            };
            if !url.is_empty() {
                ctx.refs
                    .entry(label.trim().to_lowercase())
                    .or_insert((url.trim_matches(['<', '>']).to_string(), title));
                return false;
            }
        }
        true
    });
    let mut html = String::new();
    blocks(&lines, &mut html, &mut ctx, false);
    ctx.out.html = html.trim_end().to_string();
    ctx.out.text = ctx
        .out
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    ctx.out
}

fn plain_push(ctx: &mut Ctx, html: &str) {
    let d = crate::html::parse(&format!("<main>{html}</main>"));
    ctx.out.text.push(' ');
    ctx.out.text.push_str(&d.text);
    if ctx.out.first_image.is_none() {
        ctx.out.first_image = d
            .images
            .first()
            .and_then(|(a, _)| crate::html::attr(a, "src"))
            .map(str::to_string);
    }
}

fn heading(ctx: &mut Ctx, out: &mut String, level: u8, text: &str) {
    let inner = inline(text, ctx);
    let plain = crate::html::parse(&format!("<main><p>{inner}</p></main>")).text;
    let base = {
        let s = slug(&plain);
        if s.is_empty() {
            "section".to_string()
        } else {
            s
        }
    };
    let n = ctx.ids.entry(base.clone()).or_insert(0);
    *n += 1;
    let id = if *n == 1 { base } else { format!("{base}-{n}") };
    out.push_str(&format!("<h{level} id=\"{id}\">{inner}</h{level}>\n"));
    ctx.out.text.push(' ');
    ctx.out.text.push_str(&plain);
    ctx.out.headings.push((level, plain, id));
}

fn blocks(lines: &[&str], out: &mut String, ctx: &mut Ctx, tight: bool) {
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if is_blank(l) {
            i += 1;
            continue;
        }
        if let Some((c, n, info)) = fence_open(l) {
            let ind = indent(l);
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() {
                let t = lines[i].trim();
                if t.chars().take_while(|x| *x == c).count() >= n
                    && t.trim_start_matches(c).is_empty()
                    && indent(lines[i]) <= 3
                {
                    i += 1;
                    break;
                }
                let raw = lines[i];
                let strip = raw.chars().take(ind).take_while(|x| *x == ' ').count();
                body.push(&raw[strip..]);
                i += 1;
            }
            let lang = info.split_whitespace().next().unwrap_or("");
            let class = if lang.is_empty() {
                String::new()
            } else {
                format!(" class=\"language-{}\"", esc(lang))
            };
            let code = body.join("\n");
            out.push_str(&format!(
                "<pre><code{class}>{}{}</code></pre>\n",
                esc(&code),
                if code.is_empty() { "" } else { "\n" }
            ));
            ctx.out.text.push(' ');
            ctx.out.text.push_str(&code);
            continue;
        }
        if let Some((lvl, text)) = atx(l) {
            heading(ctx, out, lvl, &text);
            i += 1;
            continue;
        }
        if hr(l) {
            out.push_str("<hr>\n");
            i += 1;
            continue;
        }
        if l.trim_start().starts_with('>') && indent(l) <= 3 {
            let mut inner: Vec<&str> = Vec::new();
            while i < lines.len() && !is_blank(lines[i]) {
                let t = lines[i].trim_start();
                match t.strip_prefix('>') {
                    Some(r) => inner.push(r.strip_prefix(' ').unwrap_or(r)),
                    None if !interrupts(lines[i]) && !inner.is_empty() => inner.push(lines[i]),
                    None => break,
                }
                i += 1;
            }
            out.push_str("<blockquote>\n");
            blocks(&inner, out, ctx, false);
            out.push_str("</blockquote>\n");
            continue;
        }
        if let Some((ordered, start, _, ch)) = marker(l) {
            i = list(lines, i, ordered, start, ch, out, ctx);
            continue;
        }
        if l.contains('|')
            && let Some(aligns) = lines.get(i + 1).and_then(|n| delimiter_row(n))
        {
            let head = table_cells(l);
            if head.len() == aligns.len() {
                let style = |k: usize| {
                    aligns
                        .get(k)
                        .filter(|a| !a.is_empty())
                        .map_or(String::new(), |a| format!(" style=\"text-align:{a}\""))
                };
                let mut t = String::from("<table>\n<thead>\n<tr>");
                for (k, c) in head.iter().enumerate() {
                    t.push_str(&format!("<th{}>{}</th>", style(k), inline(c, ctx)));
                }
                t.push_str("</tr>\n</thead>\n");
                i += 2;
                let mut rows = String::new();
                while i < lines.len()
                    && !is_blank(lines[i])
                    && lines[i].contains('|')
                    && !interrupts(lines[i])
                {
                    let cells = table_cells(lines[i]);
                    rows.push_str("<tr>");
                    for k in 0..head.len() {
                        rows.push_str(&format!(
                            "<td{}>{}</td>",
                            style(k),
                            inline(cells.get(k).map_or("", String::as_str), ctx)
                        ));
                    }
                    rows.push_str("</tr>\n");
                    i += 1;
                }
                if !rows.is_empty() {
                    t.push_str(&format!("<tbody>\n{rows}</tbody>\n"));
                }
                t.push_str("</table>\n");
                plain_push(ctx, &t);
                out.push_str(&t);
                continue;
            }
        }
        if html_block(l) {
            while i < lines.len() && !is_blank(lines[i]) {
                let t = lines[i].trim();
                // a line that is only an allowed tag is emitted as is; text between tags is Markdown inline
                let mut rest = t;
                let mut line_out = String::new();
                while let Some(lt) = rest.find('<') {
                    line_out.push_str(&inline(&rest[..lt], ctx));
                    match safe_tag(&rest[lt..]) {
                        Some((tag, used)) => {
                            line_out.push_str(&tag);
                            rest = &rest[lt + used..];
                        }
                        None => {
                            line_out.push_str("&lt;");
                            rest = &rest[lt + 1..];
                        }
                    }
                }
                line_out.push_str(&inline(rest, ctx));
                plain_push(ctx, &line_out);
                out.push_str(&line_out);
                out.push('\n');
                i += 1;
            }
            continue;
        }
        // paragraph (or setext heading)
        let mut para: Vec<&str> = vec![l.trim_start()];
        i += 1;
        let mut setext = 0u8;
        while i < lines.len() && !is_blank(lines[i]) {
            let t = lines[i].trim();
            if indent(lines[i]) <= 3 && !t.is_empty() && t.chars().all(|c| c == '=') {
                setext = 1;
                i += 1;
                break;
            }
            if indent(lines[i]) <= 3 && !t.is_empty() && t.chars().all(|c| c == '-') {
                setext = 2;
                i += 1;
                break;
            }
            if interrupts(lines[i]) {
                break;
            }
            para.push(lines[i].trim_start());
            i += 1;
        }
        let text = para.join("\n");
        if setext > 0 {
            heading(ctx, out, setext, text.trim());
            continue;
        }
        let inner = inline(text.trim_end(), ctx);
        let plain = crate::html::parse(&format!("<main><p>{inner}</p></main>"));
        if ctx.out.excerpt.is_empty() {
            ctx.out.excerpt = plain.first_paragraph.clone();
        }
        plain_push(ctx, &format!("<p>{inner}</p>"));
        if tight {
            out.push_str(&inner);
            out.push('\n');
        } else {
            out.push_str(&format!("<p>{inner}</p>\n"));
        }
    }
}

/// Parses one list starting at `lines[i]`; returns the index after it. Lines indented to an item's content column
/// belong to it (nested lists included); a blank line between items makes the list loose (`<p>` per item).
fn list(
    lines: &[&str],
    mut i: usize,
    ordered: bool,
    start: u32,
    ch: char,
    out: &mut String,
    ctx: &mut Ctx,
) -> usize {
    let mut items: Vec<Vec<String>> = Vec::new();
    let mut loose = false;
    let sibling = |l: &str| marker(l).is_some_and(|(o, _, _, c)| o == ordered && c == ch);
    while i < lines.len() {
        let l = lines[i];
        let Some((_, _, col, _)) = marker(l).filter(|_| sibling(l)) else {
            break;
        };
        let t = l.trim_start();
        let first = t
            .get((col - indent(l)).min(t.len())..)
            .unwrap_or("")
            .trim_start_matches(' ');
        let mut item = vec![first.to_string()];
        i += 1;
        while i < lines.len() {
            let n = lines[i];
            if is_blank(n) {
                item.push(String::new());
            } else if indent(n) >= col {
                let k = n.chars().take(col).take_while(|c| *c == ' ').count();
                item.push(n[k..].to_string());
            } else if item.last().is_some_and(|x| !x.is_empty())
                && marker(n).is_none()
                && !interrupts(n)
            {
                item.push(n.trim_start().to_string()); // lazy paragraph continuation
            } else {
                break;
            }
            i += 1;
        }
        let trailing = item.iter().rev().take_while(|x| x.is_empty()).count();
        item.truncate(item.len() - trailing);
        // a blank line followed by top-level content inside the item → loose
        loose |= item
            .windows(2)
            .any(|w| w[0].is_empty() && !w[1].is_empty() && !w[1].starts_with(' '));
        items.push(item);
        if trailing > 0 {
            if i < lines.len() && sibling(lines[i]) {
                loose = true;
            } else {
                break;
            }
        }
    }
    finish_list(items, ordered, start, loose, out, ctx, i)
}

fn finish_list(
    items: Vec<Vec<String>>,
    ordered: bool,
    start: u32,
    loose: bool,
    out: &mut String,
    ctx: &mut Ctx,
    i: usize,
) -> usize {
    let tag = if ordered { "ol" } else { "ul" };
    if ordered && start != 1 {
        out.push_str(&format!("<ol start=\"{start}\">\n"));
    } else {
        out.push_str(&format!("<{tag}>\n"));
    }
    for item in items {
        let mut lines: Vec<&str> = item.iter().map(String::as_str).collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        out.push_str("<li>");
        // GFM task list item
        let task = lines.first().and_then(|f| {
            let t = f.strip_prefix("[ ] ").map(|r| (false, r)).or_else(|| {
                f.strip_prefix("[x] ")
                    .or_else(|| f.strip_prefix("[X] "))
                    .map(|r| (true, r))
            });
            t.map(|(done, rest)| (done, rest.to_string()))
        });
        let owned;
        if let Some((done, rest)) = task {
            out.push_str(if done {
                "<input type=\"checkbox\" checked disabled> "
            } else {
                "<input type=\"checkbox\" disabled> "
            });
            owned = rest;
            lines[0] = &owned;
        }
        let mut inner = String::new();
        blocks(&lines, &mut inner, ctx, !loose);
        out.push_str(inner.trim_end());
        out.push_str("</li>\n");
    }
    out.push_str(&format!("</{tag}>\n"));
    i
}

/// Bracketed text starting after `[` at `s[i..]`: index of the matching `]`.
fn close_bracket(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 1;
    let mut k = i;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 1,
            b'`' => {
                let n = b[k..].iter().take_while(|c| **c == b'`').count();
                let close = s[k + n..].find(&"`".repeat(n)).map(|x| k + n + x + n)?;
                k = close;
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// `(url, title, end index)` of an inline destination starting at `s[i] == '('`.
fn destination(s: &str, i: usize) -> Option<(String, Option<String>, usize)> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'(') {
        return None;
    }
    let mut depth = 0;
    let mut k = i;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 1,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            b'\n' if depth == 1 && s[i + 1..k].trim().is_empty() => return None,
            _ => {}
        }
        k += 1;
    }
    if k >= b.len() {
        return None;
    }
    let inner = s[i + 1..k].trim();
    let (url, title) = match inner.find([' ', '\n', '\t']) {
        Some(sp) => {
            let t = inner[sp..].trim();
            let title = t
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| t.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .or_else(|| t.strip_prefix('(').and_then(|x| x.strip_suffix(')')));
            title?;
            (&inner[..sp], title.map(str::to_string))
        }
        None => (inner, None),
    };
    Some((url.trim_matches(['<', '>']).to_string(), title, k + 1))
}

fn unescape(s: &str) -> String {
    let mut o = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' && it.peek().is_some_and(|n| n.is_ascii_punctuation()) {
            o.push(it.next().unwrap());
        } else {
            o.push(c);
        }
    }
    o
}

/// Inline Markdown → HTML.
fn inline(s: &str, ctx: &Ctx) -> String {
    let b = s.as_bytes();
    let mut o = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\\' if i + 1 < b.len() && b[i + 1] == b'\n' => {
                o.push_str("<br>\n");
                i += 2;
            }
            b'\\' if i + 1 < b.len() && b[i + 1].is_ascii_punctuation() => {
                o.push_str(&esc(&(b[i + 1] as char).to_string()));
                i += 2;
            }
            b'`' => {
                let n = b[i..].iter().take_while(|x| **x == b'`').count();
                let fence = "`".repeat(n);
                let mut k = i + n;
                let mut found = None;
                while let Some(x) = s[k..].find(&fence) {
                    let at = k + x;
                    let run = b[at..].iter().take_while(|y| **y == b'`').count();
                    if run == n {
                        found = Some(at);
                        break;
                    }
                    k = at + run;
                }
                match found {
                    Some(end) => {
                        let mut code = s[i + n..end].replace('\n', " ");
                        if code.len() >= 2
                            && code.starts_with(' ')
                            && code.ends_with(' ')
                            && !code.trim().is_empty()
                        {
                            code = code[1..code.len() - 1].to_string();
                        }
                        o.push_str(&format!("<code>{}</code>", esc(&code)));
                        i = end + n;
                    }
                    None => {
                        o.push_str(&fence);
                        i += n;
                    }
                }
            }
            b'!' if b.get(i + 1) == Some(&b'[') => match link_at(s, i + 1, ctx) {
                Some((url, title, text, end)) => {
                    let alt =
                        crate::html::parse(&format!("<main><p>{}</p></main>", inline(&text, ctx)))
                            .text;
                    let t = title.map_or(String::new(), |t| format!(" title=\"{}\"", esc(&t)));
                    o.push_str(&format!(
                        "<img src=\"{}\" alt=\"{}\"{t}>",
                        esc(&safe_url(&url)),
                        esc(&alt)
                    ));
                    i = end;
                }
                None => {
                    o.push('!');
                    i += 1;
                }
            },
            b'[' => match link_at(s, i, ctx) {
                Some((url, title, text, end)) => {
                    let t = title.map_or(String::new(), |t| format!(" title=\"{}\"", esc(&t)));
                    o.push_str(&format!(
                        "<a href=\"{}\"{t}>{}</a>",
                        esc(&safe_url(&url)),
                        inline(&text, ctx)
                    ));
                    i = end;
                }
                None => {
                    o.push('[');
                    i += 1;
                }
            },
            b'<' => {
                let rest = &s[i + 1..];
                let end = rest.find('>');
                let auto = end.map(|e| &rest[..e]).filter(|u| {
                    !u.contains(char::is_whitespace)
                        && (u.contains("://") || (u.contains('@') && !u.contains(':')))
                });
                if let (Some(u), Some(e)) = (auto, end) {
                    let href = if u.contains("://") {
                        safe_url(u)
                    } else {
                        format!("mailto:{u}")
                    };
                    o.push_str(&format!("<a href=\"{}\">{}</a>", esc(&href), esc(u)));
                    i += e + 2;
                } else if let Some((tag, used)) = safe_tag(&s[i..]) {
                    o.push_str(&tag);
                    i += used;
                } else {
                    o.push_str("&lt;");
                    i += 1;
                }
            }
            b'*' | b'_' | b'~' => {
                let n = b[i..].iter().take_while(|x| **x == c).count();
                let prev = s[..i].chars().last();
                let next = s[i + n..].chars().next();
                let left_flanking = next.is_some_and(|x| !x.is_whitespace());
                let intraword_underscore = c == b'_' && prev.is_some_and(char::is_alphanumeric);
                let want = if c == b'~' {
                    (n == 2).then_some(2)
                } else {
                    Some(n.min(3))
                };
                match want
                    .filter(|_| left_flanking && !intraword_underscore)
                    .and_then(|w| close_delim(s, i + n, c, w).map(|e| (w, e)))
                {
                    Some((w, end)) => {
                        let inner = inline(&s[i + n..end], ctx);
                        let (open, close) = match (c, w) {
                            (b'~', _) => ("<del>", "</del>"),
                            (_, 1) => ("<em>", "</em>"),
                            (_, 2) => ("<strong>", "</strong>"),
                            _ => ("<em><strong>", "</strong></em>"),
                        };
                        o.push_str(open);
                        o.push_str(&inner);
                        o.push_str(close);
                        i = end + w;
                    }
                    None => {
                        o.push_str(&s[i..i + n]);
                        i += n;
                    }
                }
            }
            b'&' => {
                let ent = s[i + 1..].find(';').filter(|&e| {
                    e > 0
                        && e <= 10
                        && s[i + 1..i + 1 + e]
                            .chars()
                            .all(|x| x.is_ascii_alphanumeric() || x == '#')
                });
                match ent {
                    Some(e) => {
                        o.push_str(&s[i..i + e + 2]);
                        i += e + 2;
                    }
                    None => {
                        o.push_str("&amp;");
                        i += 1;
                    }
                }
            }
            b'>' => {
                o.push_str("&gt;");
                i += 1;
            }
            b'"' => {
                o.push_str("&quot;");
                i += 1;
            }
            b'\n' => {
                if o.ends_with("  ") {
                    while o.ends_with(' ') {
                        o.pop();
                    }
                    o.push_str("<br>");
                }
                o.push('\n');
                i += 1;
            }
            b'h' if (s[i..].starts_with("https://") || s[i..].starts_with("http://"))
                && !s[..i].ends_with(['(', '"', '\'', '=']) =>
            {
                let len = s[i..]
                    .find(|x: char| x.is_whitespace() || x == '<')
                    .unwrap_or(s.len() - i);
                let url =
                    s[i..i + len].trim_end_matches(['.', ',', ':', ';', '!', '?', ')', '\'', '"']);
                o.push_str(&format!("<a href=\"{}\">{}</a>", esc(url), esc(url)));
                i += url.len();
            }
            _ => {
                let ch = s[i..].chars().next().unwrap();
                o.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    o
}

/// `(url, title, link text, end)` of `[text](url)` or `[text][ref]`/`[ref]` starting at `s[i] == '['`.
fn link_at(s: &str, i: usize, ctx: &Ctx) -> Option<(String, Option<String>, String, usize)> {
    let close = close_bracket(s, i + 1)?;
    let text = &s[i + 1..close];
    if let Some((url, title, end)) = destination(s, close + 1) {
        return Some((unescape(&url), title, text.to_string(), end));
    }
    // reference: [text][ref], [text][], [ref]
    let (label, end) = if s[close + 1..].starts_with('[') {
        let c2 = s[close + 2..].find(']').map(|x| close + 2 + x)?;
        let l = &s[close + 2..c2];
        (if l.is_empty() { text } else { l }, c2 + 1)
    } else {
        (text, close + 1)
    };
    let (url, title) = ctx.refs.get(&label.trim().to_lowercase())?;
    Some((url.clone(), title.clone(), text.to_string(), end))
}

/// Index of a closing run of exactly `w` `c` chars (right-flanking) after `from`, skipping code spans.
fn close_delim(s: &str, from: usize, c: u8, w: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut k = from;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 2,
            b'`' => {
                let n = b[k..].iter().take_while(|x| **x == b'`').count();
                k = s[k + n..]
                    .find(&"`".repeat(n))
                    .map_or(k + n, |x| k + n + x + n);
            }
            x if x == c => {
                let n = b[k..].iter().take_while(|y| **y == c).count();
                let prev = s[..k].chars().last();
                let next = s[k + n..].chars().next();
                let right_flanking = prev.is_some_and(|p| !p.is_whitespace());
                let ok_underscore = c != b'_' || !next.is_some_and(char::is_alphanumeric);
                if n >= w && k > from && right_flanking && ok_underscore {
                    return Some(k + (n - w));
                }
                k += n;
            }
            _ => k += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> String {
        render(s).html
    }

    #[test]
    fn blocks_and_inline() {
        assert_eq!(
            h("# Hello *world*\n\nSome **bold** and `co<de>` text."),
            "<h1 id=\"hello-world\">Hello <em>world</em></h1>\n<p>Some <strong>bold</strong> and <code>co&lt;de&gt;</code> text.</p>"
        );
        assert_eq!(
            h("Title\n=====\n\nSub\n---"),
            "<h1 id=\"title\">Title</h1>\n<h2 id=\"sub\">Sub</h2>"
        );
        assert_eq!(h("a\n\n---\n\nb"), "<p>a</p>\n<hr>\n<p>b</p>");
        assert_eq!(
            h("> quote\n> more\n\nafter"),
            "<blockquote>\n<p>quote\nmore</p>\n</blockquote>\n<p>after</p>"
        );
        assert_eq!(
            h("```rust\nlet x = 1 < 2;\n```"),
            "<pre><code class=\"language-rust\">let x = 1 &lt; 2;\n</code></pre>"
        );
        assert_eq!(
            h("line one  \nline two\\\nthree"),
            "<p>line one<br>\nline two<br>\nthree</p>"
        );
        assert_eq!(
            h("snake_case_name and ~~old~~ and ***both***"),
            "<p>snake_case_name and <del>old</del> and <em><strong>both</strong></em></p>"
        );
    }

    #[test]
    fn lists() {
        assert_eq!(
            h("- a\n- b\n  - c\n- d"),
            "<ul>\n<li>a</li>\n<li>b\n<ul>\n<li>c</li>\n</ul></li>\n<li>d</li>\n</ul>"
        );
        assert_eq!(
            h("1. one\n2. two\n\n3. three"),
            "<ol>\n<li><p>one</p></li>\n<li><p>two</p></li>\n<li><p>three</p></li>\n</ol>"
        );
        assert_eq!(
            h("3) x\n4) y"),
            "<ol start=\"3\">\n<li>x</li>\n<li>y</li>\n</ol>"
        );
        assert_eq!(
            h("- [x] done\n- [ ] todo"),
            "<ul>\n<li><input type=\"checkbox\" checked disabled> done</li>\n<li><input type=\"checkbox\" disabled> todo</li>\n</ul>"
        );
        assert_eq!(h("Text\n- item"), "<p>Text</p>\n<ul>\n<li>item</li>\n</ul>");
    }

    #[test]
    fn links_and_images() {
        assert_eq!(
            h("[Docs](/docs \"Read\") and ![Logo](/l.png)"),
            "<p><a href=\"/docs\" title=\"Read\">Docs</a> and <img src=\"/l.png\" alt=\"Logo\"></p>"
        );
        assert_eq!(
            h("See [the guide][g] or [g].\n\n[g]: https://x.dev/g \"Guide\""),
            "<p>See <a href=\"https://x.dev/g\" title=\"Guide\">the guide</a> or <a href=\"https://x.dev/g\" title=\"Guide\">g</a>.</p>"
        );
        assert_eq!(
            h("<https://a.dev> https://b.dev/x. <me@a.dev>"),
            "<p><a href=\"https://a.dev\">https://a.dev</a> <a href=\"https://b.dev/x\">https://b.dev/x</a>. <a href=\"mailto:me@a.dev\">me@a.dev</a></p>"
        );
        assert_eq!(h("[x](javascript:alert(1))"), "<p><a href=\"#\">x</a></p>");
    }

    #[test]
    fn html_is_escaped_except_allowlist() {
        assert_eq!(
            h("<script>alert(1)</script>"),
            "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>"
        );
        assert_eq!(
            h(
                "<section faq onclick=\"x()\">\n<details open>\n<summary>Is it *free*?</summary>\nYes.\n</details>\n</section>"
            ),
            "<section faq>\n<details open>\n<summary>Is it <em>free</em>?</summary>\nYes.\n</details>\n</section>"
        );
        assert_eq!(
            h("a <img src=x onerror=y> b"),
            "<p>a &lt;img src=x onerror=y&gt; b</p>"
        );
    }

    #[test]
    fn tables() {
        assert_eq!(
            h("| Plan | Price |\n|:-----|------:|\n| Free | $0 |\n| Team | $9 \\| mo |"),
            "<table>\n<thead>\n<tr><th style=\"text-align:left\">Plan</th><th style=\"text-align:right\">Price</th></tr>\n</thead>\n<tbody>\n<tr><td style=\"text-align:left\">Free</td><td style=\"text-align:right\">$0</td></tr>\n<tr><td style=\"text-align:left\">Team</td><td style=\"text-align:right\">$9 | mo</td></tr>\n</tbody>\n</table>"
        );
    }

    #[test]
    fn metadata() {
        let r =
            render("# Intro\n\nFirst para with ![Cover](/c.png) here.\n\n## Intro\n\nMore words.");
        assert_eq!(
            r.headings,
            vec![
                (1, "Intro".into(), "intro".into()),
                (2, "Intro".into(), "intro-2".into())
            ]
        );
        assert_eq!(r.excerpt, "First para with here.");
        assert_eq!(r.first_image.as_deref(), Some("/c.png"));
        assert_eq!(r.text, "Intro First para with here. Intro More words.");
        assert_eq!(slug("Hello, World — 2026!"), "hello-world-2026");
    }
}
