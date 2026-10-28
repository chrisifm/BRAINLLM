//! brain-min: in-house minifier (SPEC §11, INTERNALS §6). Hand-written JS lexer, ASI-safe compaction,
//! local-name mangling and the runtime short map (`RUNTIME_MAP`).

pub mod lex;
pub mod mangle;
pub mod print;

/// Compacts JS (no renaming): strips comments and whitespace.
pub fn compact(src: &str) -> Result<String, String> {
    Ok(print::print(&lex::lex(src)?))
}

use std::collections::{HashMap, HashSet};

/// Short key for each runtime export (single source of truth, INTERNALS §6/§11). Codegen emits readable names;
/// in build, the core is published with these exports and every chunk imports the short keys.
pub const RUNTIME_MAP: &[(&str, &str)] = &[
    ("sig", "s"), ("drv", "d"), ("fx", "f"), ("batch", "b"), ("untrack", "u"), ("tpl", "t"), ("insert", "i"),
    ("attr", "a"), ("cls", "c"), ("on", "o"), ("list", "l"), ("when", "w"), ("mount", "m"), ("hydrate", "h"), ("rpc", "r"),
];
/// Internal exports used by sibling modules (brain-dev, brain-wc): they keep their name.
pub const RUNTIME_KEEP: &[&str] = &["_dev", "_ev", "_err", "_rt"];

fn runtime_map() -> HashMap<String, String> {
    RUNTIME_MAP.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}
fn runtime_keep() -> HashSet<String> {
    RUNTIME_KEEP.iter().map(|s| s.to_string()).collect()
}

/// Is this specifier the runtime core? (`/@brain/brain.js`, `./brain.js`, or the chunk that replaces it)
pub fn is_runtime_spec(spec: &str) -> bool {
    spec == "./brain.js" || spec.ends_with("/brain.js") || spec.ends_with("/brain.min.js")
}

/// Minifies the `runtime/brain.js` core with short exports. Fails if an export has no key in the map.
pub fn minify_runtime_core(src: &str) -> Result<String, String> {
    let (map, keep) = (runtime_map(), runtime_keep());
    let toks = lex::lex(src)?;
    let missing = mangle::exports_outside(&toks, &map, &keep)?;
    if !missing.is_empty() {
        return Err(format!(
            "runtime exports without a short key: {}; fix: add them to brain_min::RUNTIME_MAP (or to RUNTIME_KEEP if they are internal)",
            missing.join(", ")
        ));
    }
    let toks = mangle::drop_console_log(toks);
    let out = mangle::apply(&toks, &mangle::Plan { mangle: true, forced: &map, keep: &keep, import_map: &map, import_from: &|_| false })?;
    Ok(print::print(&out))
}

/// Minifies a module that consumes the runtime (component chunk, brain-dev, brain-wc): mangles locals,
/// rewrites runtime imports to the short key and, in prod, drops `console.log`.
pub fn minify_module(src: &str, prod: bool) -> Result<String, String> {
    let (map, keep) = (runtime_map(), runtime_keep());
    let mut toks = lex::lex(src)?;
    if prod {
        toks = mangle::drop_console_log(toks);
        toks = minify_templates(toks);
        // dead locals inside functions (QA #101 136); a module the parser cannot follow keeps its code as is
        toks = mangle::drop_unused_locals(toks.clone()).unwrap_or(toks);
    }
    let none = HashMap::new();
    let out = mangle::apply(&toks, &mangle::Plan { mangle: true, forced: &none, keep: &keep, import_map: &map, import_from: &is_runtime_spec })?;
    Ok(print::print(&out))
}

/// Shim to run the runtime tests against the minified core: re-exports both short and long keys.
pub fn runtime_test_shim(min_file: &str) -> String {
    let long: Vec<String> = RUNTIME_MAP.iter().map(|(l, s)| format!("{s} as {l}")).collect();
    format!("export * from './{min_file}'\nexport {{ {} }} from './{min_file}'\n", long.join(", "))
}


#[derive(Default, Clone)]
pub struct Options {
    /// Rename local declarations.
    pub mangle: bool,
    /// Production: `console.log(...)` → `void 0`.
    pub drop_log: bool,
    /// Forced names (runtime exports → short map).
    pub forced: HashMap<String, String>,
    /// Module-level names that are never renamed.
    pub keep: HashSet<String>,
}

pub fn minify(src: &str, o: &Options) -> Result<String, String> {
    let mut toks = lex::lex(src)?;
    if o.drop_log {
        toks = mangle::drop_console_log(toks);
    }
    if o.mangle {
        toks = mangle::mangle(&toks, &o.forced, &o.keep)?;
    }
    Ok(print::print(&toks))
}

/// Removes module-level code nothing live uses, keeping the exports in `keep` (see `mangle::tree_shake`).
pub fn tree_shake(src: &str, keep: &HashSet<String>) -> Result<String, String> {
    Ok(print::print(&mangle::tree_shake(&lex::lex(src)?, keep)?))
}

/// Names imported (`import { a, b as c }` → `a`, `b`) from specifiers that satisfy `from`.
pub fn imported_names(src: &str, from: &dyn Fn(&str) -> bool) -> Result<Vec<String>, String> {
    let t = lex::lex(src)?;
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if t[i].ident("import") && t.get(i + 1).is_some_and(|x| x.punct("{")) {
            let close = (i + 1..t.len()).find(|&j| t[j].punct("}")).ok_or("unterminated import list")?;
            let spec = t.get(close + 2).filter(|_| t.get(close + 1).is_some_and(|x| x.ident("from")));
            if let Some(s) = spec.filter(|s| s.kind == lex::Kind::Str && from(&s.text[1..s.text.len() - 1])) {
                let mut j = i + 2;
                while j < close {
                    if t[j].kind == lex::Kind::Ident {
                        out.push(t[j].text.clone());
                        // skip `as local`
                        if t.get(j + 1).is_some_and(|x| x.ident("as")) {
                            j += 2;
                        }
                    }
                    j += 1;
                }
                let _ = s;
            }
            i = close;
        }
        i += 1;
    }
    Ok(out)
}

/// Minifies the HTML inside `tpl("…")` calls of codegen output: drops quotes around attribute values that do not
/// need them (`data-b="ab12"` → `data-b=ab12`, a 4-byte win inside a JS string where quotes are escaped) and
/// collapses whitespace runs to one space. Runs before mangling, while the call is still named `tpl`.
pub fn minify_templates(toks: Vec<lex::Tok>) -> Vec<lex::Tok> {
    let mut out = toks;
    for i in 0..out.len().saturating_sub(2) {
        if !(out[i].ident("tpl") && out[i + 1].punct("(") && out[i + 2].kind == lex::Kind::Str) {
            continue;
        }
        let lit = &out[i + 2].text;
        let Some(html) = js_unquote(lit) else { continue };
        out[i + 2].text = js_quote(&minify_html(&html));
    }
    out
}

fn js_unquote(lit: &str) -> Option<String> {
    let q = lit.chars().next()?;
    let inner = lit.strip_prefix(q)?.strip_suffix(q)?;
    let mut s = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            s.push(c);
            continue;
        }
        match it.next()? {
            'n' => s.push('\n'),
            't' => s.push('\t'),
            'r' => s.push('\r'),
            '\\' => s.push('\\'),
            '"' => s.push('"'),
            '\'' => s.push('\''),
            // anything else (\u, \x, line continuations): leave the literal alone
            _ => return None,
        }
    }
    Some(s)
}

fn js_quote(s: &str) -> String {
    let dq = s.matches('"').count();
    let sq = s.matches('\'').count();
    let q = if sq < dq { '\'' } else { '"' };
    let mut o = String::with_capacity(s.len() + 2);
    o.push(q);
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c if c == q => {
                o.push('\\');
                o.push(c);
            }
            c => o.push(c),
        }
    }
    o.push(q);
    o
}

/// Conservative HTML minifier for template strings: inside tags, unquote safe attribute values; outside tags,
/// collapse whitespace runs to a single space (never removes it: inline layout depends on it). Inside `<pre>` and
/// `<textarea>` whitespace is content and is kept byte for byte (nested `<pre><code>` included).
pub fn minify_html(h: &str) -> String {
    let b = h.as_bytes();
    let mut out = String::with_capacity(h.len());
    let mut i = 0;
    let mut keep_ws = 0usize; // open <pre>/<textarea> elements
    while i < b.len() {
        if b[i] == b'<' && h[i..].starts_with("<!--") {
            let end = h[i..].find("-->").map_or(h.len(), |e| i + e + 3);
            out.push_str(&h[i..end]);
            i = end;
        } else if b[i] == b'<' && i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'/') {
            let end = match h[i..].find('>') {
                Some(e) => i + e + 1,
                None => h.len(),
            };
            let tag = &h[i..end];
            let close = tag.starts_with("</");
            let name: String = tag[if close { 2 } else { 1 }..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
            if name == "pre" || name == "textarea" {
                if close {
                    keep_ws = keep_ws.saturating_sub(1);
                } else if !tag.ends_with("/>") {
                    keep_ws += 1;
                }
            }
            out.push_str(&minify_tag(tag));
            i = end;
        } else if (b[i] as char).is_ascii_whitespace() && keep_ws > 0 {
            out.push(b[i] as char);
            i += 1;
        } else if (b[i] as char).is_ascii_whitespace() {
            while i < b.len() && (b[i] as char).is_ascii_whitespace() {
                i += 1;
            }
            out.push(' ');
        } else {
            let ch = h[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn minify_tag(tag: &str) -> String {
    let safe = |v: &str| !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '#' | '%' | '+' | ','));
    let mut out = String::with_capacity(tag.len());
    let mut rest = tag;
    while let Some(p) = rest.find("=\"") {
        out.push_str(&rest[..p + 1]);
        let after = &rest[p + 2..];
        let Some(end) = after.find('"') else {
            out.push_str(&rest[p + 1..]);
            return out;
        };
        let v = &after[..end];
        // the value must not end the tag early: `/` right before `>` would turn into a self-closing slash
        let next = after[end + 1..].chars().next();
        if safe(v) && !(v.ends_with('/') && matches!(next, Some('>'))) {
            out.push_str(v);
        } else {
            out.push('"');
            out.push_str(v);
            out.push('"');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod template_tests {
    use super::*;

    #[test]
    fn unquotes_safe_attributes_only() {
        assert_eq!(minify_html("<button data-b=\"rs62\" class=\"a b\" type=\"button\"><!>  x</button>"), "<button data-b=rs62 class=\"a b\" type=button><!> x</button>");
        assert_eq!(minify_html("<a href=\"/x/\">y</a>"), "<a href=\"/x/\">y</a>"); // conservative: a trailing `/` before `>` keeps its quotes
        assert_eq!(minify_html("<a href=\"/x\">y</a>"), "<a href=/x>y</a>");
        assert_eq!(minify_html("<a title=\"\" data-x=\"a=b\">y</a>"), "<a title=\"\" data-x=\"a=b\">y</a>");
        // whitespace is content inside <pre> and <textarea> (COMPILER defect: code blocks came out on one line)
        let code = "<div>\n  <pre class=\"c\"><code>let n = sig(0)\n\n  &lt;b&gt;  x\n</code></pre>\n  <p>a   b</p><textarea>\n two  spaces\n</textarea> <i>c   d</i></div>";
        assert_eq!(minify_html(code), "<div> <pre class=c><code>let n = sig(0)\n\n  &lt;b&gt;  x\n</code></pre> <p>a b</p><textarea>\n two  spaces\n</textarea> <i>c d</i></div>");
    }

    #[test]
    fn rewrites_tpl_literals() {
        let t = minify_templates(lex::lex("const t = tpl(\"<p data-b=\\\"ab12\\\" class=\\\"x\\\">a\\n  b</p>\")").unwrap());
        assert_eq!(print::print(&t), "const t=tpl(\"<p data-b=ab12 class=x>a b</p>\")");
    }
}
