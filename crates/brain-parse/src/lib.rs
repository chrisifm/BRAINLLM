//! `.brain` → AST (SPEC §1-§4). Entry point: [`parse`].

pub mod ast;
pub mod dump;
pub mod js;
pub mod visit;
mod markup;

pub use brain_lex::{codes, push_json_str, Diag, Edit, Json, LineIndex, Severity, Span};

use ast::{Component, FileKind, Level};
use std::path::Path;

/// Parses a whole `.brain` file. `path` is used for the component name, its level and diagnostics.
pub fn parse(src: &str, path: &Path) -> Result<Component, Vec<Diag>> {
    let file = display_path(path);
    let idx = LineIndex::new(src);
    let kind = file_kind(path);
    if kind == FileKind::Module {
        let (meta, body) = module_meta(src, &file, &idx)?;
        let script = brain_lex::Lexer::new(src, &file, &idx, body, src.len());
        let script = js::JsParser::new(script).and_then(|mut p| p.parse_program()).map_err(|d| vec![d])?;
        return Ok(Component {
            name: component_name(path),
            kind,
            level: level_of(path),
            file,
            props: Vec::new(),
            meta,
            script,
            template: Vec::new(),
            head: Vec::new(),
            style: None,
            span: idx.span(src, 0, src.len()),
            src: src.to_string(),
        });
    }
    let mut parsed = markup::Markup::new(src, &file, &idx).run()?;
    // A view's top-level `<head>` holds extra head tags (§32); `app.brain` keeps its `<html><head>` as markup.
    let mut head = Vec::new();
    if kind == FileKind::Component {
        parsed.template.retain(|n| match n {
            ast::Node::Element(e) if e.tag == "head" => {
                head.extend(e.children.iter().cloned());
                false
            }
            _ => true,
        });
    }
    Ok(Component {
        name: component_name(path),
        kind,
        level: level_of(path),
        file: file.clone(),
        props: parsed.props,
        meta: parsed.meta,
        script: parsed.script,
        template: parsed.template,
        head,
        style: parsed.style,
        span: idx.span(src, 0, src.len()),
        src: src.to_string(),
    })
}

/// Optional `<meta server protect …>` line heading a `.jsbrain` (SPEC §11). Returns the attributes and
/// the byte offset where the JS starts.
fn module_meta(src: &str, file: &str, idx: &LineIndex) -> Result<(Vec<ast::MetaAttr>, usize), Vec<Diag>> {
    let start = src.len() - src.trim_start().len();
    if !src[start..].starts_with("<meta") || !src[start + 5..].starts_with(|c: char| c.is_whitespace() || c == '>') {
        return Ok((Vec::new(), 0));
    }
    let Some(close) = src[start..].find('>').map(|i| start + i) else {
        return Err(vec![Diag::new("B0003", file, idx.span(src, start, start + 5), "unclosed `<meta`", "close the line with `>`: `<meta server>`")]);
    };
    let b = src.as_bytes();
    let mut meta = Vec::new();
    let mut i = start + 5;
    while i < close {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let ns = i;
        while i < close && !b[i].is_ascii_whitespace() && b[i] != b'=' {
            i += 1;
        }
        let name = src[ns..i].to_string();
        let mut value = None;
        if i < close && b[i] == b'=' {
            i += 1;
            let q = b.get(i).copied().filter(|q| *q == b'"' || *q == b'\'');
            let vs = if q.is_some() { i + 1 } else { i };
            let mut j = vs;
            while j < close && (q.map_or(!b[j].is_ascii_whitespace(), |q| b[j] != q)) {
                j += 1;
            }
            value = Some(src[vs..j].to_string());
            i = if q.is_some() { j + 1 } else { j };
        }
        meta.push(ast::MetaAttr { name, value, expr: None, span: idx.span(src, ns, i.min(close)) });
    }
    Ok((meta, close + 1))
}

/// `.jsbrain` → module, `app.brain` → app shell, anything else → component.
pub fn file_kind(path: &Path) -> FileKind {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if name.ends_with(".jsbrain") {
        FileKind::Module
    } else if name == "app.brain" {
        FileKind::App
    } else {
        FileKind::Component
    }
}

/// Path with forward slashes, as written into diagnostics.
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `user-card.brain` → `UserCard`.
/// Dynamic routes take the folder as prefix so `users/[id]` and `posts/[id]` never collide:
/// `views/users/[id].brain` → `UsersId`, `[...slug].brain` → `Slug`; `404.brain` → `V404`.
pub fn component_name(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let dynamic = stem.starts_with('[');
    let stem = if dynamic {
        stem.trim_start_matches('[').trim_start_matches('.').split(']').next().unwrap_or("").to_string()
    } else {
        stem.split('.').next().unwrap_or("").to_string()
    };
    let stem = match path.parent().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()) {
        Some(dir) if dynamic && !dir.starts_with('[') && dir != "views" && !stem.is_empty() && !path.to_string_lossy().contains("...") => format!("{dir}-{stem}"),
        _ => stem,
    };
    let name = pascal(&stem);
    if name.starts_with(|c: char| c.is_ascii_digit()) { format!("V{name}") } else { name }
}

fn pascal(stem: &str) -> String {
    stem.split(['-', '_', ' ', '.'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Nearest known folder wins: `ui/` micro, `views/` view, anything else comp.
pub fn level_of(path: &Path) -> Level {
    for c in path.parent().into_iter().flat_map(|p| p.components().rev()) {
        match c.as_os_str().to_str() {
            Some("ui") => return Level::Micro,
            Some("views") => return Level::View,
            Some("comp") => return Level::Comp,
            _ => {}
        }
    }
    Level::Comp
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn names_and_levels() {
        assert_eq!(component_name(&PathBuf::from("comp/user-card.brain")), "UserCard");
        assert_eq!(component_name(&PathBuf::from("comp/counter.test.brain")), "Counter");
        assert_eq!(component_name(&PathBuf::from("views/users/[id].brain")), "UsersId");
        assert_eq!(component_name(&PathBuf::from("views/docs/[...slug].brain")), "Slug");
        assert_eq!(component_name(&PathBuf::from("views/[id].brain")), "Id");
        assert_eq!(component_name(&PathBuf::from("views/404.brain")), "V404");
        assert_eq!(component_name(&PathBuf::from("views/users/_layout.brain")), "Layout");
        assert_eq!(level_of(&PathBuf::from("app/ui/badge.brain")), Level::Micro);
        assert_eq!(level_of(&PathBuf::from("app/views/admin/users.brain")), Level::View);
        assert_eq!(level_of(&PathBuf::from("x.brain")), Level::Comp);
    }
}

/// When `s` starts with an element marked `raw` (`<code raw>…</code>`), the byte length of the whole element:
/// text scanners (block order, block hashes) must treat its content as opaque.
pub fn raw_element_len(s: &str) -> Option<usize> {
    let rest = s.strip_prefix('<')?;
    let name_len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))?;
    if name_len == 0 {
        return None;
    }
    let name = &rest[..name_len];
    let head_end = rest.find('>')?;
    let head = &rest[name_len..head_end];
    let is_raw = head.split(|c: char| c.is_whitespace() || c == '/').any(|w| w == "raw");
    if !is_raw {
        return None;
    }
    let close = format!("</{name}>");
    let body = 1 + head_end + 1;
    let end = s[body..].find(&close)? + body + close.len();
    Some(end)
}
