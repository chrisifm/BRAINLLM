//! Conservative rule interning. No property/value algebra or cross-scope equivalence is inferred.
use crate::{CssItem, parse_str};

#[derive(Clone, Debug)]
pub struct Change {
    pub line: u32,
    pub column: u32,
    pub kind: &'static str,
}
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub rules_merged: usize,
    pub groups_merged: usize,
    pub duplicate_rules_removed: usize,
    pub scope_bodies_interned: usize,
    pub changes: Vec<Change>,
    pub skipped: Option<&'static str>,
}
impl Report {
    /// Metadata only. CSS values, selector text and local paths never enter feedback.
    pub fn to_json(&self) -> String {
        let changes = self
            .changes
            .iter()
            .map(|change| {
                format!(
                    "{{\"line\":{},\"column\":{},\"kind\":\"{}\"}}",
                    change.line, change.column, change.kind
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"schemaVersion\":1,\"coordinateSpace\":\"generated-css\",\"inputBytes\":{},\"outputBytes\":{},\"rulesMerged\":{},\"groupsMerged\":{},\"duplicateRulesRemoved\":{},\"scopeBodiesInterned\":{},\"skipped\":{},\"changesTruncated\":{},\"changes\":[{}]}}",
            self.input_bytes,
            self.output_bytes,
            self.rules_merged,
            self.groups_merged,
            self.duplicate_rules_removed,
            self.scope_bodies_interned,
            self.skipped
                .map(|reason| format!("\"{reason}\""))
                .unwrap_or_else(|| "null".into()),
            self.rules_merged + self.groups_merged > self.changes.len(),
            changes
        )
    }
    fn changed(&mut self, item: &CssItem, kind: &'static str) {
        let span = item.span();
        if self.changes.len() < 256 {
            self.changes.push(Change {
                line: span.line,
                column: span.col,
                kind,
            });
        }
    }
}
struct Node<'a> {
    item: &'a CssItem,
    leading: String,
}
struct Optimizer<'a> {
    source: &'a str,
    report: Report,
}

/// All coordinates refer to the generated CSS input. No transform is attempted on malformed/oversized input.
pub fn dedupe(source: &str) -> (String, Report) {
    let mut optimizer = Optimizer {
        source,
        report: Report {
            input_bytes: source.len(),
            output_bytes: source.len(),
            ..Default::default()
        },
    };
    if !within_budget(source) {
        optimizer.report.skipped = Some("css-budget");
        return (source.into(), optimizer.report);
    }
    let sheet = match parse_str(source, "generated.css") {
        Ok(sheet) => sheet,
        Err(_) => {
            optimizer.report.skipped = Some("unparsed-css");
            return (source.into(), optimizer.report);
        }
    };
    let (nodes, tail) = optimizer.nodes(&sheet.items, 0, source.len());
    let output = optimizer.render(&nodes, &tail, true);
    optimizer.report.output_bytes = output.len();
    (output, optimizer.report)
}
#[doc(hidden)]
pub fn within_budget(source: &str) -> bool {
    if source.len() > 4 * 1024 * 1024 {
        return false;
    }
    let bytes = source.as_bytes();
    let (mut cursor, mut depth) = (0, 0usize);
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' => {
                let quote = bytes[cursor];
                cursor += 1;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += if bytes[cursor] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                let Some(end) = source[cursor + 2..].find("*/") else {
                    return false;
                };
                cursor += end + 3;
            }
            b'{' => {
                depth += 1;
                if depth > 64 {
                    return false;
                }
            }
            b'}' => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
            }
            _ => {}
        }
        cursor += 1;
    }
    depth == 0
}
impl<'a> Optimizer<'a> {
    fn raw(&self, item: &CssItem) -> &'a str {
        let span = item.span();
        &self.source[span.start as usize..span.end as usize]
    }
    fn nodes<'b>(&self, items: &'b [CssItem], start: usize, end: usize) -> (Vec<Node<'b>>, String) {
        let mut at = start;
        let mut nodes = Vec::with_capacity(items.len());
        for item in items {
            let span = item.span();
            nodes.push(Node {
                item,
                leading: self.source[at..span.start as usize].into(),
            });
            at = span.end as usize;
        }
        (nodes, self.source[at..end].into())
    }
    fn parts(&self, item: &CssItem) -> Option<(&'a str, &'a str, usize, usize)> {
        let span = item.span();
        let raw = self.raw(item);
        let bytes = raw.as_bytes();
        let mut cursor = 0;
        let mut depth = 0;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\'' | b'"' => {
                    let quote = bytes[cursor];
                    cursor += 1;
                    while cursor < bytes.len() && bytes[cursor] != quote {
                        cursor += if bytes[cursor] == b'\\' { 2 } else { 1 };
                    }
                }
                b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                    cursor += raw[cursor + 2..].find("*/")? + 3;
                }
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b'{' if depth == 0 => {
                    if !raw.ends_with('}') {
                        return None;
                    }
                    let start = span.start as usize + cursor + 1;
                    let end = span.end as usize - 1;
                    return Some((&raw[..cursor], &self.source[start..end], start, end));
                }
                _ => {}
            }
            cursor += 1;
        }
        None
    }
    fn rule(&self, item: &CssItem) -> Option<(&'a str, &'a str)> {
        let CssItem::Rule { items, .. } = item else {
            return None;
        };
        if items.is_empty()
            || !items
                .iter()
                .all(|item| matches!(item, CssItem::Decl { .. }))
        {
            return None;
        }
        let (head, body, _, _) = self.parts(item)?;
        if head.contains("/*") {
            return None;
        }
        Some((head.trim(), body))
    }
    fn group<'b>(&self, item: &'b CssItem) -> Option<(&'a str, &'b [CssItem], usize, usize)> {
        let CssItem::At {
            name,
            prelude,
            block: Some(items),
            ..
        } = item
        else {
            return None;
        };
        if !matches!(name.as_str(), "media" | "supports" | "scope" | "layer")
            || (name == "layer" && prelude.is_empty())
        {
            return None;
        }
        let (head, _, start, end) = self.parts(item)?;
        // Header comments may carry metadata and are not canonicalized away.
        if head.contains("/*") {
            return None;
        }
        Some((head.trim(), items, start, end))
    }
    // A data-b value is singular on an element; the shared lower boundary cuts every nested component.
    // Only simple qualified bodies are admitted: '&' and nested scopes can change matching/specificity.
    fn private_scope<'b>(
        &self,
        item: &'b CssItem,
    ) -> Option<(Vec<String>, &'a str, &'b [CssItem], usize, usize)> {
        let CssItem::At {
            name,
            prelude,
            block: Some(items),
            ..
        } = item
        else {
            return None;
        };
        if name != "scope"
            || items.is_empty()
            || items.iter().any(|item| {
                let CssItem::Rule {
                    selector, items, ..
                } = item
                else {
                    return true;
                };
                !safe_selectors(selector)
                    || selector.contains('&')
                    || items.is_empty()
                    || !items
                        .iter()
                        .all(|item| matches!(item, CssItem::Decl { .. }))
            })
        {
            return None;
        }
        let text = prelude.trim().strip_prefix('(')?;
        let close = text.find(')')?;
        if text[close + 1..].trim().strip_prefix("to")?.trim() != "([data-b])" {
            return None;
        }
        let mut roots = Vec::new();
        for root in text[..close].split(',') {
            let id = root
                .trim()
                .strip_prefix("[data-b=\"")?
                .strip_suffix("\"]")?;
            if id.is_empty()
                || id.len() > 32
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                || roots.iter().any(|root| root == id)
            {
                return None;
            }
            roots.push(id.to_string());
        }
        let (_, body, start, stop) = self.parts(item)?;
        Some((roots, body, items, start, stop))
    }
    fn render(&mut self, nodes: &[Node<'_>], tail: &str, allow_scope_union: bool) -> String {
        let mut output = String::new();
        let mut index = 0;
        while index < nodes.len() {
            let node = &nodes[index];
            output.push_str(&node.leading);
            if let Some((selector, body)) = self.rule(node.item) {
                let mut selectors = vec![selector];
                let mut end = index + 1;
                while end < nodes.len() && nodes[end].leading.trim().is_empty() {
                    let Some((next, next_body)) = self.rule(nodes[end].item) else {
                        break;
                    };
                    // Trailing whitespace can belong to a custom-property token stream; do not trim it.
                    if body != next_body {
                        break;
                    }
                    if !selectors.contains(&next)
                        && !(safe_selectors(selector) && safe_selectors(next))
                    {
                        break;
                    }
                    self.report.rules_merged += 1;
                    if selectors.contains(&next) {
                        self.report.duplicate_rules_removed += 1;
                    } else {
                        selectors.push(next);
                    }
                    self.report
                        .changed(nodes[end].item, "adjacent-identical-declarations");
                    end += 1;
                }
                if end > index + 1 {
                    output.push_str(&selectors.join(","));
                    output.push('{');
                    output.push_str(body);
                    output.push('}');
                } else {
                    output.push_str(self.raw(node.item));
                }
                index = end;
                continue;
            }
            if allow_scope_union {
                if let Some((mut roots, body, items, start, stop)) = self.private_scope(node.item) {
                    let mut end = index + 1;
                    while end < nodes.len() && nodes[end].leading.trim().is_empty() {
                        let Some((next, next_body, _, _, _)) = self.private_scope(nodes[end].item)
                        else {
                            break;
                        };
                        if body != next_body || next.iter().any(|root| roots.contains(root)) {
                            break;
                        }
                        roots.extend(next);
                        end += 1;
                    }
                    if end > index + 1 {
                        let (children, tail) = self.nodes(items, start, stop);
                        let body = self.render(&children, &tail, false);
                        self.report.groups_merged += end - index - 1;
                        self.report.scope_bodies_interned += end - index - 1;
                        for member in &nodes[index + 1..end] {
                            self.report
                                .changed(member.item, "adjacent-private-scope-body");
                        }
                        let roots = roots
                            .iter()
                            .map(|root| format!("[data-b=\"{root}\"]"))
                            .collect::<Vec<_>>()
                            .join(",");
                        output.push_str(&format!("@scope ({roots}) to ([data-b]){{{body}}}"));
                        index = end;
                        continue;
                    }
                }
            }
            if let Some((header, _, _, _)) = self.group(node.item) {
                let mut end = index + 1;
                while end < nodes.len()
                    && nodes[end].leading.trim().is_empty()
                    && self
                        .group(nodes[end].item)
                        .is_some_and(|(next, _, _, _)| next == header)
                {
                    end += 1;
                }
                let mut children = Vec::new();
                let mut trailing = String::new();
                for member in &nodes[index..end] {
                    let (_, items, start, stop) = self.group(member.item).unwrap();
                    let (mut next, tail) = self.nodes(items, start, stop);
                    if let Some(first) = next.first_mut() {
                        first.leading.insert_str(0, &trailing);
                        trailing = tail;
                    } else {
                        trailing.push_str(&tail);
                    }
                    children.extend(next);
                }
                let nested_scope = matches!(node.item,CssItem::At{name,..} if name=="scope");
                let body = self.render(&children, &trailing, allow_scope_union && !nested_scope);
                if end > index + 1 {
                    self.report.groups_merged += end - index - 1;
                    for member in &nodes[index + 1..end] {
                        self.report
                            .changed(member.item, "adjacent-identical-context");
                    }
                    output.push_str(header);
                    output.push('{');
                    output.push_str(&body);
                    output.push('}');
                } else {
                    let raw = self.raw(node.item);
                    let (_, old, start, _) = self.parts(node.item).unwrap();
                    if body == old {
                        output.push_str(raw);
                    } else {
                        let prefix = start - node.item.span().start as usize;
                        output.push_str(&raw[..prefix]);
                        output.push_str(&body);
                        output.push('}');
                    }
                }
                index = end;
                continue;
            }
            // Qualified nesting and unknown/anonymous groups remain completely opaque.
            output.push_str(self.raw(node.item));
            index += 1;
        }
        output.push_str(tail);
        output
    }
}
// Keep a plain selector list's individual specificity. Never wrap unrelated selectors in :is().
fn safe_selectors(selectors: &str) -> bool {
    selectors.split(',').all(|selector| {
        let mut selector = selector.trim().to_string();
        if let Some(rest) = selector.strip_prefix(":where(:scope):is(") {
            let Some(close) = rest.find(')') else {
                return false;
            };
            selector = format!("{}{}", &rest[..close], &rest[close + 1..]);
        }
        selector = selector.replace(":scope", "*");
        if selector.is_empty()
            || selector.chars().any(|c| {
                !(c.is_ascii_alphanumeric()
                    || matches!(
                        c,
                        '.' | '#' | '-' | '_' | '*' | '>' | '+' | '~' | ' ' | '\t' | '\n' | '\r'
                    ))
            })
        {
            return false;
        }
        let bytes = selector.as_bytes();
        let (mut at, mut need_compound) = (0, true);
        while at < bytes.len() {
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            if at == bytes.len() {
                return !need_compound;
            }
            if matches!(bytes[at], b'>' | b'+' | b'~') {
                if need_compound {
                    return false;
                }
                need_compound = true;
                at += 1;
                continue;
            }
            let mut atoms = 0;
            if bytes[at] == b'*' {
                at += 1;
                atoms += 1;
            } else if ident_start(bytes, at) {
                at = identifier(bytes, at);
                atoms += 1;
            }
            while at < bytes.len() && matches!(bytes[at], b'.' | b'#') {
                at += 1;
                if !ident_start(bytes, at) {
                    return false;
                }
                at = identifier(bytes, at);
                atoms += 1;
            }
            if atoms == 0 {
                return false;
            }
            need_compound = false;
            if at < bytes.len()
                && !bytes[at].is_ascii_whitespace()
                && !matches!(bytes[at], b'>' | b'+' | b'~')
            {
                return false;
            }
        }
        !need_compound
    })
}
fn ident_start(bytes: &[u8], at: usize) -> bool {
    bytes
        .get(at)
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        || (bytes.get(at) == Some(&b'-')
            && bytes
                .get(at + 1)
                .is_some_and(|byte| byte.is_ascii_alphabetic() || matches!(*byte, b'_' | b'-')))
}
fn identifier(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len()
        && (bytes[at].is_ascii_alphanumeric() || matches!(bytes[at], b'_' | b'-'))
    {
        at += 1;
    }
    at
}
