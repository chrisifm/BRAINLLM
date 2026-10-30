//! Custom-property values are token streams: preserve them before both whitespace and value shortening.
use brain_css::CssItem;

/// Mask only custom-property values. Malformed/opaque custom blocks fail closed instead of changing tokens.
pub(crate) fn preserve(css: &str, normalize: impl FnOnce(&str) -> String) -> String {
    if !css.contains("--") {
        return normalize(css);
    }
    if !brain_css::intern::within_budget(css) {
        return css.into();
    }
    let Ok(sheet) = brain_css::parse_str(css, "generated.css") else {
        return css.into();
    };
    let mut ranges = Vec::new();
    if !values(&sheet.items, css, &mut ranges) {
        return css.into();
    }
    if ranges.is_empty() {
        return normalize(css);
    }
    ranges.sort_unstable();
    let identity = crate::content_hash(css);
    let Some(prefix) = (0..16)
        .map(|attempt| format!("__brain_css_{identity}_{attempt}__"))
        .find(|prefix| !css.contains(prefix))
    else {
        return css.into();
    };
    let mut masked = String::with_capacity(css.len());
    let mut original = Vec::new();
    let mut last = 0;
    for (index, (start, end)) in ranges.into_iter().enumerate() {
        masked.push_str(&css[last..start]);
        let token = format!("{prefix}{index}__");
        masked.push_str(&token);
        original.push(&css[start..end]);
        last = end;
    }
    masked.push_str(&css[last..]);
    let normalized = normalize(&masked);
    let mut output = String::with_capacity(css.len());
    let mut rest = normalized.as_str();
    let mut restored = 0;
    while let Some(at) = rest.find(&prefix) {
        output.push_str(&rest[..at]);
        let after = &rest[at + prefix.len()..];
        let Some(end) = after.find("__") else {
            return css.into();
        };
        let Ok(index) = after[..end].parse::<usize>() else {
            return css.into();
        };
        let Some(value) = original.get(index) else {
            return css.into();
        };
        output.push_str(value);
        rest = &after[end + 2..];
        restored += 1;
    }
    if restored != original.len() {
        return css.into();
    }
    output.push_str(rest);
    output
}
fn values(items: &[CssItem], css: &str, ranges: &mut Vec<(usize, usize)>) -> bool {
    for item in items {
        match item {
            CssItem::Decl { name, span, .. } if name.starts_with("--") => {
                if name.contains('\\') {
                    return false;
                }
                let start = span.start as usize;
                let mut end = span.end as usize;
                let raw = &css[start..end];
                let Some(colon) = raw.find(':') else {
                    return false;
                };
                if raw.ends_with(';') {
                    end -= 1;
                }
                ranges.push((start + colon + 1, end));
            }
            CssItem::Rule {
                selector, items, ..
            } => {
                if selector.trim_start().starts_with("--") {
                    return false;
                }
                if !values(items, css, ranges) {
                    return false;
                }
            }
            CssItem::At {
                block: Some(items), ..
            } => {
                if !values(items, css, ranges) {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

pub(crate) fn shorten(css: &str) -> String {
    let Ok(sheet) = brain_css::parse_str(css, "generated.css") else {
        return css.into();
    };
    let mut ranges = Vec::new();
    ordinary_values(&sheet.items, css, &mut ranges);
    ranges.sort_unstable();
    let mut output = String::with_capacity(css.len());
    let mut last = 0;
    for (start, end) in ranges {
        output.push_str(&css[last..start]);
        output.push_str(&shorten_value(&css[start..end]));
        last = end;
    }
    output.push_str(&css[last..]);
    output
}
fn ordinary_values(items: &[CssItem], css: &str, ranges: &mut Vec<(usize, usize)>) {
    for item in items {
        match item {
            CssItem::Decl { name, span, .. } if !name.starts_with("--") => {
                let start = span.start as usize;
                let mut end = span.end as usize;
                let raw = &css[start..end];
                if let Some(colon) = raw.find(':') {
                    if raw.ends_with(';') {
                        end -= 1;
                    }
                    ranges.push((start + colon + 1, end));
                }
            }
            CssItem::Rule { items, .. }
            | CssItem::At {
                block: Some(items), ..
            } => ordinary_values(items, css, ranges),
            _ => {}
        }
    }
}

fn shorten_value(css: &str) -> String {
    let b: Vec<char> = css.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    let mut depth = 0u32;
    let is_num_start = |prev: Option<char>| {
        prev.is_none_or(|p| matches!(p, ':' | ' ' | ',' | '(' | '-' | '+' | '/' | '*'))
    };
    while i < b.len() {
        let c = b[i];
        if i + 4 <= b.len()
            && b[i].eq_ignore_ascii_case(&'u')
            && b[i + 1].eq_ignore_ascii_case(&'r')
            && b[i + 2].eq_ignore_ascii_case(&'l')
            && b[i + 3] == '('
        {
            let start = i;
            i += 4;
            let mut level = 1;
            while i < b.len() && level > 0 {
                if b[i] == '\\' {
                    i = (i + 2).min(b.len());
                    continue;
                }
                if matches!(b[i], '\'' | '"') {
                    let quote = b[i];
                    i += 1;
                    while i < b.len() && b[i] != quote {
                        i += if b[i] == '\\' { 2 } else { 1 };
                    }
                    i = (i + 1).min(b.len());
                    continue;
                }
                if b[i] == '(' {
                    level += 1;
                } else if b[i] == ')' {
                    level -= 1;
                }
                i += 1;
            }
            out.extend(&b[start..i]);
            continue;
        }
        if c == '"' || c == '\'' {
            let start = i;
            i += 1;
            while i < b.len() && b[i] != c {
                if b[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            out.extend(&b[start..i]);
            continue;
        }
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth = depth.saturating_sub(1);
        }
        // #aabbcc → #abc (only in values: the previous char is `:`, space, `,` or `(`)
        if c == '#'
            && i + 7 <= b.len()
            && matches!(out.chars().last(), None | Some(':' | ' ' | ',' | '('))
        {
            let h: Vec<char> = b[i + 1..i + 7].to_vec();
            let ends = b.get(i + 7).is_none_or(|n| !n.is_ascii_hexdigit());
            if ends
                && h.iter().all(|x| x.is_ascii_hexdigit())
                && h[0] == h[1]
                && h[2] == h[3]
                && h[4] == h[5]
            {
                out.push('#');
                out.push(h[0].to_ascii_lowercase());
                out.push(h[2].to_ascii_lowercase());
                out.push(h[4].to_ascii_lowercase());
                i += 7;
                continue;
            }
        }
        if c == '0' && is_num_start(out.chars().last()) {
            // 0.5 → .5
            if b.get(i + 1) == Some(&'.') && b.get(i + 2).is_some_and(|d| d.is_ascii_digit()) {
                i += 1;
                continue;
            }
            // 0px → 0 (not inside parentheses, not 0.x, not part of a longer number)
            if depth == 0 {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let unit: String = b[i + 1..j].iter().collect();
                let ends = b
                    .get(j)
                    .is_none_or(|n| matches!(n, ';' | '}' | ' ' | ',' | '!' | ')'));
                if ends
                    && matches!(
                        unit.as_str(),
                        "px" | "em" | "rem" | "vh" | "vw" | "pt" | "cm" | "mm" | "in" | "ex" | "ch"
                    )
                {
                    out.push('0');
                    i = j;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}
