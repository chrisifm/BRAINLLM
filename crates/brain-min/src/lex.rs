//! In-house JavaScript lexer, just enough to minify: comments, strings, nested templates,
//! regex vs division, numbers and identifiers. It does not validate the grammar; it keeps each token's text.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ident,
    Punct,
    Str,
    Num,
    Regex,
    /// Template without substitutions: `` `...` ``.
    Template,
    /// `` `...${ `` · `}...${` · `` }...` ``
    TplHead,
    TplMiddle,
    TplTail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tok {
    pub kind: Kind,
    pub text: String,
    /// There was a line break (or a multi-line comment containing one) between the previous token and this one.
    pub nl_before: bool,
    /// Byte offset of the token in the source (0 for synthesized tokens).
    pub start: usize,
}

impl Tok {
    pub fn is(&self, k: Kind, t: &str) -> bool {
        self.kind == k && self.text == t
    }
    pub fn punct(&self, t: &str) -> bool {
        self.is(Kind::Punct, t)
    }
    pub fn ident(&self, t: &str) -> bool {
        self.is(Kind::Ident, t)
    }
}

/// Punctuators ordered from longest to shortest (longest match wins).
const PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=", "<=", ">=", "&&", "||",
    "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**", "<<", ">>", "{", "}", "(", ")", "[", "]",
    ";", ",", "<", ">", "+", "-", "*", "/", "%", "&", "|", "^", "!", "~", "?", ":", "=", ".", "@", "#",
];

/// Words after which a `/` starts a regex.
const REGEX_AFTER_WORD: &[&str] =
    &["return", "typeof", "instanceof", "in", "of", "new", "delete", "void", "throw", "case", "do", "else", "yield", "await"];

fn is_id_start(c: char) -> bool {
    c == '$' || c == '_' || c.is_alphabetic()
}
fn is_id_part(c: char) -> bool {
    is_id_start(c) || c.is_ascii_digit() || c == '\u{200c}' || c == '\u{200d}' || c.is_alphanumeric()
}

pub fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    // byte offset of each char index (plus one extra at the end)
    let byte_at: Vec<usize> = src.char_indices().map(|(b, _)| b).chain(std::iter::once(src.len())).collect();
    let mut i = 0;
    let mut out: Vec<Tok> = Vec::new();
    let mut nl = false;
    // Brace depth inside each open `${` of a template.
    let mut tpl_stack: Vec<u32> = Vec::new();
    let line_of = |i: usize| chars[..i.min(chars.len())].iter().filter(|c| **c == '\n').count() + 1;

    while i < chars.len() {
        let c = chars[i];
        if c == '\n' || c == '\r' || c == '\u{2028}' || c == '\u{2029}' {
            nl = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() || c == '\u{feff}' {
            i += 1;
            continue;
        }
        // Comments
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let start = i;
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                if chars[i] == '\n' {
                    nl = true;
                }
                i += 1;
            }
            if i + 1 >= chars.len() {
                return Err(format!("unterminated comment at line {}", line_of(start)));
            }
            i += 2;
            continue;
        }
        let start = i;
        let at = byte_at[start];
        let push = |out: &mut Vec<Tok>, kind, text: String, nl: &mut bool| {
            out.push(Tok { kind, text, nl_before: *nl, start: at });
            *nl = false;
        };
        // Template (start, or continuation after `}`)
        let closes_sub = c == '}' && tpl_stack.last() == Some(&0);
        if c == '`' || closes_sub {
            if closes_sub {
                tpl_stack.pop();
            }
            i += 1;
            let mut has_sub = false;
            loop {
                match chars.get(i) {
                    None => return Err(format!("unterminated template at line {}", line_of(start))),
                    Some('\\') => i += 2,
                    Some('`') => {
                        i += 1;
                        break;
                    }
                    Some('$') if chars.get(i + 1) == Some(&'{') => {
                        i += 2;
                        has_sub = true;
                        break;
                    }
                    Some(_) => i += 1,
                }
            }
            let text: String = chars[start..i].iter().collect();
            let kind = match (closes_sub, has_sub) {
                (false, false) => Kind::Template,
                (false, true) => Kind::TplHead,
                (true, true) => Kind::TplMiddle,
                (true, false) => Kind::TplTail,
            };
            if has_sub {
                tpl_stack.push(0);
            }
            push(&mut out, kind, text, &mut nl);
            continue;
        }
        if c == '"' || c == '\'' {
            i += 1;
            loop {
                match chars.get(i) {
                    None | Some('\n') => return Err(format!("unterminated string at line {}", line_of(start))),
                    Some('\\') => i += 2,
                    Some(q) if *q == c => {
                        i += 1;
                        break;
                    }
                    Some(_) => i += 1,
                }
            }
            push(&mut out, Kind::Str, chars[start..i].iter().collect(), &mut nl);
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                // 1e-5 / 1e+5
                if (chars[i] == 'e' || chars[i] == 'E')
                    && !chars[start..i].iter().any(|c| *c == 'x' || *c == 'X')
                    && matches!(chars.get(i + 1), Some('+') | Some('-'))
                {
                    i += 1;
                }
                i += 1;
            }
            push(&mut out, Kind::Num, chars[start..i].iter().collect(), &mut nl);
            continue;
        }
        if is_id_start(c) || c == '\\' {
            while i < chars.len() && (is_id_part(chars[i]) || chars[i] == '\\') {
                i += 1;
            }
            push(&mut out, Kind::Ident, chars[start..i].iter().collect(), &mut nl);
            continue;
        }
        if c == '/' && regex_allowed(out.last()) {
            i += 1;
            let mut in_class = false;
            loop {
                match chars.get(i) {
                    None | Some('\n') => return Err(format!("unterminated regex at line {}", line_of(start))),
                    Some('\\') => i += 2,
                    Some('[') => {
                        in_class = true;
                        i += 1
                    }
                    Some(']') => {
                        in_class = false;
                        i += 1
                    }
                    Some('/') if !in_class => {
                        i += 1;
                        break;
                    }
                    Some(_) => i += 1,
                }
            }
            while i < chars.len() && is_id_part(chars[i]) {
                i += 1;
            }
            push(&mut out, Kind::Regex, chars[start..i].iter().collect(), &mut nl);
            continue;
        }
        let rest: String = chars[i..(i + 4).min(chars.len())].iter().collect();
        let Some(p) = PUNCTS.iter().find(|p| rest.starts_with(**p)) else {
            return Err(format!("unexpected character `{c}` at line {}", line_of(start)));
        };
        // `?.` followed by a digit is `? .5`
        let p = if *p == "?." && chars.get(i + 2).is_some_and(|d| d.is_ascii_digit()) { "?" } else { p };
        i += p.chars().count();
        if let Some(top) = tpl_stack.last_mut() {
            if p == "{" {
                *top += 1;
            } else if p == "}" {
                *top -= 1;
            }
        }
        push(&mut out, Kind::Punct, p.to_string(), &mut nl);
    }
    if !tpl_stack.is_empty() {
        return Err("unterminated template at end of file".into());
    }
    Ok(out)
}

fn regex_allowed(prev: Option<&Tok>) -> bool {
    match prev {
        None => true,
        Some(t) => match t.kind {
            Kind::Num | Kind::Str | Kind::Regex | Kind::Template | Kind::TplTail => false,
            Kind::Ident => REGEX_AFTER_WORD.contains(&t.text.as_str()),
            Kind::Punct => !matches!(t.text.as_str(), ")" | "]" | "++" | "--"),
            Kind::TplHead | Kind::TplMiddle => true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(Kind, String)> {
        lex(src).unwrap().into_iter().map(|t| (t.kind, t.text)).collect()
    }

    #[test]
    fn regex_vs_division() {
        let k = kinds("a = b / c / d; r = /x\\/[/]y/g.test(s); return /z/");
        assert_eq!(k.iter().filter(|(k, _)| *k == Kind::Regex).count(), 2);
        assert!(k.contains(&(Kind::Regex, "/x\\/[/]y/g".into())));
        assert!(k.contains(&(Kind::Regex, "/z/".into())));
    }

    #[test]
    fn nested_templates() {
        let k = kinds("`a${ b ? `c${d}` : {e:1}.e }f`");
        let texts: Vec<_> = k.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, ["`a${", "b", "?", "`c${", "d", "}`", ":", "{", "e", ":", "1", "}", ".", "e", "}f`"]);
        assert_eq!(k[0].0, Kind::TplHead);
        assert_eq!(k[5].0, Kind::TplTail);
        assert_eq!(k[14].0, Kind::TplTail);
    }

    #[test]
    fn comments_strings_numbers_newlines() {
        let t = lex("x = 'a//b' // c\n/* d\n */ y = 1e-3 + .5 + 0x1F + 1_000").unwrap();
        assert_eq!(t[2].text, "'a//b'");
        assert!(t[3].nl_before && t[3].text == "y");
        let nums: Vec<_> = t.iter().filter(|t| t.kind == Kind::Num).map(|t| t.text.as_str()).collect();
        assert_eq!(nums, ["1e-3", ".5", "0x1F", "1_000"]);
        assert!(lex("'abc").is_err() && lex("`a${b").is_err());
    }

    #[test]
    fn byte_offsets() {
        let src = "ü = 'ß' /* x */ + b";
        for t in lex(src).unwrap() {
            assert_eq!(&src[t.start..t.start + t.text.len()], t.text);
        }
    }

    #[test]
    fn longest_punct() {
        let k = kinds("a ??= b?.c >>>= d ... e => f");
        let p: Vec<_> = k.iter().filter(|(k, _)| *k == Kind::Punct).map(|(_, t)| t.as_str()).collect();
        assert_eq!(p, ["??=", "?.", ">>>=", "...", "=>"]);
    }
}
