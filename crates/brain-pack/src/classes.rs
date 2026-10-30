//! Class names renamed in production (#260, INTERNALS §11.6, QA conformance 135): `.user-row` → `.a3` in the CSS, the
//! templates and the code, so no source name survives in dist/.
//!
//! The compiler writes classes in two fixed places: `class="…"` inside `tpl("…")` templates and the second argument
//! of `cls(el, "name", cond)` (`class:x` and conditional classes). Every class that appears in a component stylesheet
//! is a candidate; a candidate is left alone when its name also appears anywhere else in the code (a `classList`
//! call, a `querySelector('.x')`, data, a runtime module that builds markup) or in a stylesheet of public/, because
//! renaming it there could break code we cannot see through.

use std::collections::{BTreeMap, BTreeSet};

/// Class names in the selectors of a stylesheet (skipping strings, comments and `url(…)`).
pub fn css_classes(css: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    scan_css(css, &mut |name, _| {
        out.insert(name.to_string());
        None
    });
    out
}

/// Renames the `.name` selectors of `css` with `map`.
pub fn rename_css(css: &str, map: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(css.len());
    let mut last = 0;
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    scan_css(css, &mut |name, at| {
        map.get(name).map(|n| (at, at + name.len(), n.clone()))
    })
    .into_iter()
    .for_each(|e| edits.push(e));
    for (a, b, n) in edits {
        out.push_str(&css[last..a]);
        out.push_str(&n);
        last = b;
    }
    out.push_str(&css[last..]);
    out
}

/// Walks the `.class` tokens of a stylesheet: `f(name, byte offset of the name)` may return an edit.
fn scan_css(css: &str, f: &mut dyn FnMut(&str, usize) -> Option<(usize, usize, String)>) -> Vec<(usize, usize, String)> {
    let b = css.as_bytes();
    let mut edits = Vec::new();
    let mut i = 0;
    // inside a declaration block's value (after `:` up to `;`/`}`), `.5em` and `a.b` are not selectors
    let mut in_value = false;
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                continue;
            }
            b'u' if css[i..].starts_with("url(") => {
                i = css[i..].find(')').map_or(b.len(), |e| i + e + 1);
                continue;
            }
            b'{' | b';' => in_value = false,
            b'}' => in_value = false,
            b':' => {
                // `a:hover` / `:where(` in a selector vs `color: x` in a block: a value starts after `:` only inside
                // a block, which we detect by the next `;` or `}` coming before the next `{`
                let rest = &css[i + 1..];
                let next_brace = rest.find('{').unwrap_or(usize::MAX);
                let next_end = rest.find([';', '}']).unwrap_or(usize::MAX);
                if next_end < next_brace {
                    in_value = true;
                }
            }
            b'.' if !in_value => {
                let prev_digit = i > 0 && b[i - 1].is_ascii_digit();
                let name: String = css[i + 1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
                if !prev_digit && !name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit()) {
                    if let Some(e) = f(&name, i + 1) {
                        edits.push(e);
                    }
                    i += 1 + name.len();
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    edits
}

/// Short names in order: a…z, a0…z9, aa…: never one of `taken`.
pub fn short_names(count: usize, taken: &BTreeSet<String>) -> Vec<String> {
    let alpha: Vec<char> = ('a'..='z').collect();
    let tail: Vec<char> = ('0'..='9').chain('a'..='z').collect();
    let mut out = Vec::with_capacity(count);
    let mut n = 0usize;
    while out.len() < count {
        let mut s = String::new();
        s.push(alpha[n % 26]);
        let mut k = n / 26;
        while k > 0 {
            k -= 1;
            s.push(tail[k % tail.len()]);
            k /= tail.len();
        }
        n += 1;
        if !taken.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// The class map of a build: candidates (classes of the component stylesheets) minus every name that the code uses
/// outside the two compiler-owned places or that public/ styles.
pub fn plan(stylesheets: &[&str], modules: &[&str], public_css: &[&str]) -> BTreeMap<String, String> {
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    for css in stylesheets {
        candidates.extend(css_classes(css));
    }
    let mut unsafe_: BTreeSet<String> = BTreeSet::new();
    // Escaped selector names cannot be decoded by this intentionally small scanner.
    // Keeping all names is safer than assigning a short name which an opaque selector already uses.
    if stylesheets.iter().chain(public_css.iter()).any(|css|css.contains('\\')) {
        unsafe_.extend(candidates.iter().cloned());
    }
    let mut reserved = candidates.clone();
    for css in public_css {
        let names=css_classes(css);
        unsafe_.extend(names.iter().cloned());
        reserved.extend(names);
    }
    for js in modules {
        let words=foreign_strings(js);
        if words.iter().any(|word|word=="*") { unsafe_.extend(candidates.iter().cloned()); }
        for word in words {
            if candidates.contains(&word) { unsafe_.insert(word.clone()); }
            reserved.insert(word);
        }
        // Markup can carry a public/external class which has no component stylesheet declaration.
        // It must not become the destination name of another component-owned class.
        if let Ok(tokens)=brain_min::lex::lex(js) {
            for token in tokens.iter().filter(|token|matches!(token.kind,brain_min::lex::Kind::Str|brain_min::lex::Kind::Template|brain_min::lex::Kind::TplHead|brain_min::lex::Kind::TplMiddle|brain_min::lex::Kind::TplTail)) {
                let names=markup_class_names(&token.text);
                if names.contains("*") {unsafe_.extend(candidates.iter().cloned());}
                reserved.extend(names);
            }
        }
    }
    let keep: Vec<String> = candidates.iter().filter(|c| !unsafe_.contains(*c)).cloned().collect();
    let names = short_names(keep.len(), &reserved);
    keep.into_iter().zip(names).collect()
}

// Accept the same quoted/escaped/unquoted attribute forms as rename_class_attrs; never rewrite these values.
fn markup_class_names(html: &str) -> BTreeSet<String> {
    let mut names=BTreeSet::new();let mut rest=html;
    while let Some(at)=rest.find("class=") {
        let before=rest[..at].chars().next_back();rest=&rest[at+6..];
        if !before.is_some_and(char::is_whitespace) {continue;}
        let (quote,start)=if rest.starts_with("\\\""){("\\\"",2)}else if rest.starts_with('"'){("\"",1)}else if rest.starts_with('\''){("'",1)}else{("",0)};
        let body=&rest[start..];
        let end=if quote.is_empty(){body.find(|ch:char|ch.is_whitespace()||ch=='>').unwrap_or(body.len())}else{body.find(quote).unwrap_or(body.len())};
        if body[..end].contains('&') || body[..end].contains("\\u") || body[..end].contains("\\x") {names.insert("*".into());}
        names.extend(body[..end].split_whitespace().map(str::to_string));
        rest=&body[end..];
    }
    names
}

/// Words of every string in a module except the ones the compiler owns (`tpl("…")` templates and `cls(_, "x", …)`),
/// with `.` and `#` split off so `querySelector('.row')` counts as `row`.
fn foreign_strings(js: &str) -> Vec<String> {
    use brain_min::lex::{lex, Kind};
    let Ok(t) = lex(js) else { return vec!["*".into()] };
    let mut out = Vec::new();
    for i in 0..t.len() {
        if !matches!(t[i].kind, Kind::Str | Kind::Template | Kind::TplHead | Kind::TplMiddle | Kind::TplTail) {
            continue;
        }
        let owned_tpl = i >= 2 && t[i - 1].punct("(") && t[i - 2].ident("tpl");
        let owned_cls = i >= 4 && t[i - 1].punct(",") && cls_call_before(&t, i);
        if owned_tpl || owned_cls {
            continue;
        }
        // Opaque encoded names may spell a class not visible in the raw token. Fail closed.
        if t[i].text.contains("\\u") || t[i].text.contains("\\x") { out.push("*".into()); }
        for w in t[i].text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
            if !w.is_empty() {
                out.push(w.to_string());
            }
        }
    }
    out
}

/// Is the string at `i` the second argument of `cls(…)`? (`cls ( <expr> , "x"`)
fn cls_call_before(t: &[brain_min::lex::Tok], i: usize) -> bool {
    let mut depth = 0i32;
    let mut j = i - 1; // the `,`
    while j > 0 {
        j -= 1;
        let x = &t[j];
        if x.punct(")") || x.punct("]") || x.punct("}") {
            depth += 1;
        } else if x.punct("(") || x.punct("[") || x.punct("{") {
            if depth == 0 {
                return x.punct("(") && j > 0 && t[j - 1].ident("cls");
            }
            depth -= 1;
        } else if depth == 0 && x.punct(",") {
            return false; // not the second argument
        }
    }
    false
}

/// Applies the map to a compiled module: the classes of its `tpl("…")` templates and its `cls(_, "x", …)` calls.
pub fn rename_js(js: &str, map: &BTreeMap<String, String>) -> String {
    use brain_min::lex::{lex, Kind};
    if map.is_empty() {
        return js.to_string();
    }
    let Ok(t) = lex(js) else { return js.to_string() };
    let mut out = String::with_capacity(js.len());
    let mut last = 0;
    for i in 0..t.len() {
        if t[i].kind != Kind::Str {
            continue;
        }
        let new = if i >= 2 && t[i - 1].punct("(") && t[i - 2].ident("tpl") {
            Some(rename_class_attrs(&t[i].text, map))
        } else if i >= 4 && t[i - 1].punct(",") && cls_call_before(&t, i) {
            let q = &t[i].text[..1];
            let inner = &t[i].text[1..t[i].text.len() - 1];
            map.get(inner).map(|n| format!("{q}{n}{q}"))
        } else {
            None
        };
        if let Some(n) = new.filter(|n| *n != t[i].text) {
            out.push_str(&js[last..t[i].start]);
            out.push_str(&n);
            last = t[i].start + t[i].text.len();
        }
    }
    out.push_str(&js[last..]);
    out
}

/// `class="a b"` / `class=\"a b\"` / `class=a` inside HTML (a template literal or a page): every mapped word renamed.
pub fn rename_class_attrs(html: &str, map: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find("class=") {
        let before = rest[..i].chars().last();
        out.push_str(&rest[..i + 6]);
        rest = &rest[i + 6..];
        if !before.is_some_and(|c| c.is_whitespace()) {
            continue;
        }
        let (q, body_start) = if rest.starts_with("\\\"") {
            ("\\\"", 2)
        } else if rest.starts_with('"') {
            ("\"", 1)
        } else if rest.starts_with('\'') {
            ("'", 1)
        } else {
            ("", 0)
        };
        let body = &rest[body_start..];
        let end = if q.is_empty() { body.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(body.len()) } else { body.find(q).unwrap_or(body.len()) };
        let value = &body[..end];
        let renamed: Vec<String> = value.split(' ').map(|w| map.get(w).cloned().unwrap_or_else(|| w.to_string())).collect();
        out.push_str(&rest[..body_start]);
        out.push_str(&renamed.join(" "));
        rest = &body[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_selectors_only() {
        let css = "@scope ([data-b=\"x\"]){.page,.row:hover>.cell{margin:.5em;background:url(a.png);content:\".fake\"}.n1{x:1}}";
        assert_eq!(css_classes(css).into_iter().collect::<Vec<_>>(), ["cell", "n1", "page", "row"]);
        let map: BTreeMap<String, String> = [("page", "a"), ("row", "b"), ("cell", "c")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(rename_css(css, &map), "@scope ([data-b=\"x\"]){.a,.b:hover>.c{margin:.5em;background:url(a.png);content:\".fake\"}.n1{x:1}}");
    }

    #[test]
    fn compiler_places_renamed_foreign_uses_protected() {
        let js = "const $t0 = tpl(\"<main class=\\\"page wide\\\"><p class=\\\"note\\\">x</p></main>\");\ncls($e1, \"active\", () => on.v);\nel.classList.add(\"wide\");\n";
        let map = plan(&[".page{}.note{}.active{}.wide{}"], &[js], &[]);
        assert!(map.contains_key("page") && map.contains_key("note") && map.contains_key("active"), "{map:?}");
        assert!(!map.contains_key("wide"), "used by classList: kept");
        let out = rename_js(js, &map);
        assert!(out.contains(&format!("class=\\\"{} wide\\\"", map["page"])) && out.contains(&format!("cls($e1, \"{}\"", map["active"])) && out.contains("classList.add(\"wide\")"), "{out}");
        // public/ styles a class: never renamed
        assert!(!plan(&[".page{}"], &[], &[".page{color:red}"]).contains_key("page"));
    }

    #[test]
    fn short_names_skip_taken() {
        let taken: BTreeSet<String> = ["a".to_string(), "c".to_string()].into();
        assert_eq!(short_names(3, &taken), ["b", "d", "e"]);
        assert_eq!(short_names(28, &BTreeSet::new())[26..], ["a0".to_string(), "b0".to_string()]);
    }
}
