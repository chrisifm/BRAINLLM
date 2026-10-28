//! Prints tokens with minimal whitespace. A source line break is kept only when removing it
//! could change the meaning through ASI (automatic `;` insertion).

use crate::lex::{Kind, Tok};

const RESTRICTED: &[&str] = &["return", "break", "continue", "throw", "yield", "async", "let"];

fn word_end(t: &Tok) -> bool {
    matches!(t.kind, Kind::Ident | Kind::Num) || (t.kind == Kind::Regex && t.text.ends_with(|c: char| c.is_alphanumeric()))
}
fn word_start(t: &Tok) -> bool {
    matches!(t.kind, Kind::Ident | Kind::Num) || t.text.starts_with('\\')
}

/// Is a space needed between `a` and `b` so they do not merge into another token?
fn needs_space(a: &Tok, b: &Tok) -> bool {
    if word_end(a) && word_start(b) {
        return true;
    }
    let (x, y) = (a.text.as_str(), b.text.as_str());
    // `a + +b`, `a - -b`, `a + ++b`, `a / /re/` (would be a comment), `1 .toString()`
    (x.ends_with('+') && y.starts_with('+'))
        || (x.ends_with('-') && y.starts_with('-'))
        || (x.ends_with('/') && (y.starts_with('/') || y.starts_with('*')))
        || (a.kind == Kind::Num && y.starts_with('.') && !x.contains(['.', 'e', 'E', 'x', 'X']))
        || (x.ends_with('<') && y.starts_with('!')) // `<!--`
        || (x.ends_with("--") && y.starts_with('>')) // `-->`
}

/// Can the statement end right at `prev`? If not, the following line break does not trigger ASI.
pub fn prev_continues(prev: &Tok) -> bool {
    prev.kind == Kind::Punct && !matches!(prev.text.as_str(), ")" | "]" | "}" | "++" | "--")
        || matches!(prev.kind, Kind::TplHead | Kind::TplMiddle)
}

/// Does `next` always continue the previous expression (never an ASI "offending" token)?
pub fn next_continues(next: &Tok) -> bool {
    match next.kind {
        Kind::Punct => !matches!(next.text.as_str(), "++" | "--" | "{" | "!" | "~" | "@" | "#"),
        Kind::Template | Kind::TplHead | Kind::TplMiddle | Kind::TplTail => true,
        Kind::Ident => matches!(next.text.as_str(), "in" | "instanceof" | "of"),
        _ => false,
    }
}

pub fn print(toks: &[Tok]) -> String {
    let mut out = String::with_capacity(toks.iter().map(|t| t.text.len() + 1).sum());
    for (i, t) in toks.iter().enumerate() {
        if i > 0 {
            let p = &toks[i - 1];
            let restricted = p.kind == Kind::Ident && RESTRICTED.contains(&p.text.as_str());
            let keep_nl = t.nl_before
                && (restricted || matches!(t.text.as_str(), "++" | "--") || !(prev_continues(p) || next_continues(t)));
            if keep_nl {
                out.push('\n');
            } else if needs_space(p, t) {
                out.push(' ');
            }
        }
        out.push_str(&t.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::lex;

    fn min(s: &str) -> String {
        print(&lex(s).unwrap())
    }

    #[test]
    fn spaces_only_where_needed() {
        assert_eq!(min("const  a = b  +  +c ; let x = y - -z"), "const a=b+ +c;let x=y- -z");
        assert_eq!(min("x = a / /re/.source; n = 1 .toString()"), "x=a/ /re/.source;n=1 .toString()");
        assert_eq!(min("if ( a ) { return b }"), "if(a){return b}");
    }

    #[test]
    fn asi_is_preserved() {
        // Without `;`: line breaks separating statements are kept.
        assert_eq!(min("let a = 1\nlet b = 2\nfoo()\nbar()"), "let a=1\nlet b=2\nfoo()\nbar()");
        // return + line break = return; (restricted production)
        assert_eq!(min("function f() {\n  return\n  1\n}"), "function f(){return\n1}");
        // a\n++b: the ++ is a prefix of b
        assert_eq!(min("a\n++b"), "a\n++b");
        // line breaks inside expressions are removed
        assert_eq!(min("const x = [\n  1,\n  2\n]\n  .map(f)"), "const x=[1,2].map(f)");
        assert_eq!(min("a = b\n  ? c\n  : d"), "a=b?c:d");
    }
}
