//! BrainScript lexer. Works on a byte range of the whole `.brain` file so every span is absolute,
//! and stops wherever the parser stops (the markup parser resumes right after a `}`).

use crate::{Diag, LineIndex, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// Identifiers and keywords alike; the parser decides what a word means.
    Ident(String),
    Num(String),
    /// `value` is the cooked string, `raw` the source text including quotes.
    Str { value: String, raw: String },
    Regex(String),
    /// Template pieces. `raw` excludes the delimiters (`` ` ``, `${`, `}`).
    TplFull(String),
    TplHead(String),
    TplMiddle(String),
    TplTail(String),
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// A line terminator appeared between the previous token and this one (for ASI).
    pub nl_before: bool,
}

impl Token {
    pub fn is_punct(&self, p: &str) -> bool {
        matches!(&self.tok, Tok::Punct(q) if *q == p)
    }
    pub fn is_word(&self, w: &str) -> bool {
        matches!(&self.tok, Tok::Ident(q) if q == w)
    }
}

/// Snapshot used by the parser to backtrack (arrow-function detection).
#[derive(Clone, Debug)]
pub struct LexerState {
    pos: usize,
    braces: Vec<bool>,
    regex_ok: bool,
}

pub struct Lexer<'a> {
    src: &'a str,
    file: &'a str,
    idx: &'a LineIndex,
    pos: usize,
    end: usize,
    /// One entry per open `{` or `${`; `true` marks a template substitution.
    braces: Vec<bool>,
    regex_ok: bool,
}

const PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=", "<=", ">=",
    "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**", "<<", ">>", "{", "}",
    "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%", "&", "|", "^", "!", "~", "?", ":", "=", ".",
    "@", "#",
];

const REGEX_AFTER_WORDS: &[&str] = &[
    "return", "typeof", "instanceof", "in", "of", "new", "delete", "void", "throw", "case", "do", "else", "await",
    "yield",
];

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, file: &'a str, idx: &'a LineIndex, start: usize, end: usize) -> Self {
        Lexer { src, file, idx, pos: start, end: end.min(src.len()), braces: Vec::new(), regex_ok: true }
    }

    pub fn src(&self) -> &'a str {
        self.src
    }
    pub fn file(&self) -> &'a str {
        self.file
    }
    pub fn line_index(&self) -> &'a LineIndex {
        self.idx
    }
    pub fn pos(&self) -> usize {
        self.pos
    }
    pub fn end(&self) -> usize {
        self.end
    }

    pub fn save(&self) -> LexerState {
        LexerState { pos: self.pos, braces: self.braces.clone(), regex_ok: self.regex_ok }
    }
    pub fn restore(&mut self, s: LexerState) {
        self.pos = s.pos;
        self.braces = s.braces;
        self.regex_ok = s.regex_ok;
    }

    pub fn span(&self, start: usize, end: usize) -> Span {
        self.idx.span(self.src, start, end)
    }

    fn err(&self, start: usize, msg: &str, fix: &str) -> Diag {
        Diag::new("B0001", self.file, self.span(start, start + 1), msg, fix)
    }

    fn peek(&self) -> Option<char> {
        if self.pos >= self.end {
            return None;
        }
        self.src[self.pos..self.end].chars().next()
    }
    fn peek_at(&self, n: usize) -> Option<char> {
        if self.pos >= self.end {
            return None;
        }
        self.src[self.pos..self.end].chars().nth(n)
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Skips whitespace and comments; returns whether a newline was crossed.
    fn skip_trivia(&mut self) -> Result<bool, Diag> {
        let mut nl = false;
        loop {
            match self.peek() {
                Some('\n') | Some('\u{2028}') | Some('\u{2029}') => {
                    nl = true;
                    self.bump();
                }
                Some(c) if c.is_whitespace() || c == '\u{feff}' => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        match self.bump() {
                            None => return Err(self.err(start, "unclosed `/*` comment", "close the comment with `*/`")),
                            Some('\n') => nl = true,
                            Some('*') if self.peek() == Some('/') => {
                                self.bump();
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                _ => return Ok(nl),
            }
        }
    }

    pub fn next_token(&mut self) -> Result<Token, Diag> {
        self.next_token_located(true)
    }

    /// The same tokenization with byte ranges only: successful token spans have zero line/column.
    /// Consumers that only need token boundaries avoid repeatedly counting Unicode columns on long lines.
    /// Errors still carry the normal exact line and column; lexer state and snapshots are unchanged.
    pub fn next_token_bytes(&mut self) -> Result<Token, Diag> {
        self.next_token_located(false)
    }

    fn next_token_located(&mut self, located: bool) -> Result<Token, Diag> {
        let nl_before = self.skip_trivia()?;
        let start = self.pos;
        let Some(c) = self.peek() else {
            let span = if located { self.span(start, start) } else { Span { start: start as u32, end: start as u32, line: 0, col: 0 } };
            return Ok(Token { tok: Tok::Eof, span, nl_before });
        };
        let tok = if is_ident_start(c) {
            self.ident()
        } else if c.is_ascii_digit() || (c == '.' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit())) {
            self.number(start)?
        } else if c == '"' || c == '\'' {
            self.string(start, c)?
        } else if c == '`' {
            self.bump();
            self.template_part(start, true)?
        } else if c == '}' && self.braces.last() == Some(&true) {
            self.braces.pop();
            self.bump();
            self.template_part(start, false)?
        } else if c == '/' && self.regex_ok {
            self.regex(start)?
        } else {
            self.punct(start)?
        };
        self.regex_ok = match &tok {
            Tok::Ident(w) => REGEX_AFTER_WORDS.contains(&w.as_str()),
            Tok::Punct(p) => !matches!(*p, ")" | "]" | "}" | "++" | "--"),
            Tok::TplHead(_) | Tok::TplMiddle(_) => true,
            _ => false,
        };
        let span = if located { self.span(start, self.pos) } else { Span { start: start as u32, end: self.pos as u32, line: 0, col: 0 } };
        Ok(Token { tok, span, nl_before })
    }

    fn ident(&mut self) -> Tok {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_ident_part(c) {
                self.bump();
            } else {
                break;
            }
        }
        Tok::Ident(self.src[start..self.pos].to_string())
    }

    fn number(&mut self, start: usize) -> Result<Tok, Diag> {
        let radix_prefix = self.peek() == Some('0') && matches!(self.peek_at(1), Some('x' | 'X' | 'b' | 'B' | 'o' | 'O'));
        if radix_prefix {
            self.pos += 2;
            while self.peek().is_some_and(|c| c.is_ascii_hexdigit() || c == '_') {
                self.bump();
            }
        } else {
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '_') {
                self.bump();
            }
            if self.peek() == Some('.') {
                self.bump();
                while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '_') {
                    self.bump();
                }
            }
            if matches!(self.peek(), Some('e' | 'E')) {
                self.bump();
                if matches!(self.peek(), Some('+' | '-')) {
                    self.bump();
                }
                if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    return Err(self.err(start, "incomplete numeric exponent", "write digits after the `e`, e.g. `1e3`"));
                }
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.bump();
                }
            }
        }
        if self.peek() == Some('n') {
            self.bump();
        }
        if self.peek().is_some_and(is_ident_start) {
            return Err(self.err(self.pos, "an identifier cannot start right after a number", "separate the number from the name with a space or an operator"));
        }
        Ok(Tok::Num(self.src[start..self.pos].to_string()))
    }

    fn escape(&mut self, out: &mut String, start: usize) -> Result<(), Diag> {
        let Some(e) = self.bump() else {
            return Err(self.err(start, "unclosed string", "close the string with the same quote it opens with"));
        };
        match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'b' => out.push('\u{08}'),
            'f' => out.push('\u{0c}'),
            'v' => out.push('\u{0b}'),
            '0' if !self.peek().is_some_and(|c| c.is_ascii_digit()) => out.push('\0'),
            '\n' => {}
            '\r' => {
                if self.peek() == Some('\n') {
                    self.bump();
                }
            }
            'x' => {
                let h: String = (0..2).filter_map(|_| self.bump()).collect();
                let v = u32::from_str_radix(&h, 16).map_err(|_| self.err(start, "invalid `\\x` escape", "use two hex digits: `\\x41`"))?;
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            'u' => {
                let v = if self.peek() == Some('{') {
                    self.bump();
                    let mut h = String::new();
                    while let Some(c) = self.bump() {
                        if c == '}' {
                            break;
                        }
                        h.push(c);
                    }
                    u32::from_str_radix(&h, 16).ok()
                } else {
                    let h: String = (0..4).filter_map(|_| self.bump()).collect();
                    u32::from_str_radix(&h, 16).ok()
                };
                let v = v.ok_or_else(|| self.err(start, "invalid `\\u` escape", "use `\\u00e1` or `\\u{1F600}`"))?;
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            c => out.push(c),
        }
        Ok(())
    }

    fn string(&mut self, start: usize, q: char) -> Result<Tok, Diag> {
        self.bump();
        let mut value = String::new();
        loop {
            match self.bump() {
                None | Some('\n') => {
                    return Err(self.err(start, "unclosed string", &format!("close the string with {q} on the same line; for multiple lines use a template string `...`")));
                }
                Some(c) if c == q => break,
                Some('\\') => self.escape(&mut value, start)?,
                Some(c) => value.push(c),
            }
        }
        Ok(Tok::Str { value, raw: self.src[start..self.pos].to_string() })
    }

    /// Scans template text after `` ` `` or after the `}` closing a substitution.
    fn template_part(&mut self, start: usize, head: bool) -> Result<Tok, Diag> {
        let text_start = self.pos;
        loop {
            match self.peek() {
                None => return Err(self.err(start, "unclosed template string", "close the template string with `")),
                Some('\\') => {
                    self.bump();
                    self.bump();
                }
                Some('`') => {
                    let raw = self.src[text_start..self.pos].to_string();
                    self.bump();
                    return Ok(if head { Tok::TplFull(raw) } else { Tok::TplTail(raw) });
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    let raw = self.src[text_start..self.pos].to_string();
                    self.pos += 2;
                    self.braces.push(true);
                    return Ok(if head { Tok::TplHead(raw) } else { Tok::TplMiddle(raw) });
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    fn regex(&mut self, start: usize) -> Result<Tok, Diag> {
        self.bump();
        let mut in_class = false;
        loop {
            match self.bump() {
                None | Some('\n') => return Err(self.err(start, "unclosed regular expression", "close the regex with `/`")),
                Some('\\') => {
                    self.bump();
                }
                Some('[') => in_class = true,
                Some(']') => in_class = false,
                Some('/') if !in_class => break,
                _ => {}
            }
        }
        while self.peek().is_some_and(is_ident_part) {
            self.bump();
        }
        Ok(Tok::Regex(self.src[start..self.pos].to_string()))
    }

    fn punct(&mut self, start: usize) -> Result<Tok, Diag> {
        let rest = &self.src[self.pos..self.end];
        for p in PUNCTS {
            if rest.starts_with(p) {
                // `a?.5:1` is a conditional, not optional chaining.
                if *p == "?." && rest[2..].starts_with(|c: char| c.is_ascii_digit()) {
                    continue;
                }
                self.pos += p.len();
                match *p {
                    "{" => self.braces.push(false),
                    "}" => {
                        self.braces.pop();
                    }
                    _ => {}
                }
                return Ok(Tok::Punct(p));
            }
        }
        let c = rest.chars().next().unwrap_or(' ');
        Err(self.err(start, &format!("unexpected character `{c}`"), "remove the character or write it inside a string"))
    }
}

pub fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$' || (!c.is_ascii() && c.is_alphabetic())
}

pub fn is_ident_part(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit() || (!c.is_ascii() && c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(src: &str) -> Vec<Tok> {
        let idx = LineIndex::new(src);
        let mut lx = Lexer::new(src, "t.brain", &idx, 0, src.len());
        let mut out = Vec::new();
        loop {
            let t = lx.next_token().unwrap();
            if t.tok == Tok::Eof {
                return out;
            }
            out.push(t.tok);
        }
    }

    #[test]
    fn punctuators_and_words() {
        assert_eq!(
            lex("let n = a?.b ?? 1 >>>= x"),
            vec![
                Tok::Ident("let".into()),
                Tok::Ident("n".into()),
                Tok::Punct("="),
                Tok::Ident("a".into()),
                Tok::Punct("?."),
                Tok::Ident("b".into()),
                Tok::Punct("??"),
                Tok::Num("1".into()),
                Tok::Punct(">>>="),
                Tok::Ident("x".into()),
            ]
        );
    }

    #[test]
    fn template_with_nested_braces() {
        let toks = lex("`a${ {x:1}.x }b${y}c`");
        assert_eq!(toks[0], Tok::TplHead("a".into()));
        assert_eq!(toks[toks.len() - 1], Tok::TplTail("c".into()));
        assert!(toks.contains(&Tok::TplMiddle("b".into())));
    }

    #[test]
    fn regex_vs_division() {
        assert_eq!(lex("a / b / c").iter().filter(|t| **t == Tok::Punct("/")).count(), 2);
        assert_eq!(lex("x = /ab+c/gi")[2], Tok::Regex("/ab+c/gi".into()));
        assert_eq!(lex("return /[/]/")[1], Tok::Regex("/[/]/".into()));
    }

    #[test]
    fn strings_are_cooked() {
        assert_eq!(lex(r"'a\nbπ'")[0], Tok::Str { value: "a\nbπ".into(), raw: r"'a\nbπ'".into() });
    }

    #[test]
    fn spans_are_absolute_with_line_and_col() {
        let src = "<x>\n  let π = 1";
        let idx = LineIndex::new(src);
        let mut lx = Lexer::new(src, "t.brain", &idx, 3, src.len());
        lx.next_token().unwrap();
        let t = lx.next_token().unwrap();
        assert_eq!(t.tok, Tok::Ident("π".into()));
        assert_eq!((t.span.line, t.span.col), (2, 7));
    }

    #[test]
    fn byte_only_tokens_preserve_ranges_grammar_and_newlines() {
        let src = "const π = {label: 'naïve 😀'};\n/* note */const value = `text ${(() => { return /[/*@L71*/]/.source })()} tail`;";
        let idx = LineIndex::new(src);
        let mut normal = Lexer::new(src, "t.brain", &idx, 0, src.len());
        let mut bytes = Lexer::new(src, "t.brain", &idx, 0, src.len());
        loop {
            let located = normal.next_token().unwrap();
            let unlocated = bytes.next_token_bytes().unwrap();
            assert_eq!(located.tok, unlocated.tok);
            assert_eq!(located.nl_before, unlocated.nl_before);
            assert_eq!((located.span.start, located.span.end), (unlocated.span.start, unlocated.span.end));
            assert_eq!((unlocated.span.line, unlocated.span.col), (0, 0));
            if located.tok == Tok::Eof { break; }
        }
    }

    #[test]
    fn byte_only_token_errors_keep_exact_unicode_locations() {
        let src = "const π = 'naïve';\nconst obj = {π: 'bad";
        let idx = LineIndex::new(src);
        let mut normal = Lexer::new(src, "t.brain", &idx, 0, src.len());
        let mut bytes = Lexer::new(src, "t.brain", &idx, 0, src.len());
        loop {
            match (normal.next_token(), bytes.next_token_bytes()) {
                (Ok(a), Ok(b)) => { assert_eq!(a.tok, b.tok); assert_ne!(a.tok, Tok::Eof); }
                (Err(a), Err(b)) => {
                    assert_eq!(a, b);
                    assert_eq!((b.line, b.col), (2, 17));
                    break;
                }
                tokens => panic!("byte-only lexing diverged: {tokens:?}"),
            }
        }
    }

    #[test]
    fn unterminated_string_is_b0001() {
        let src = "'abc";
        let idx = LineIndex::new(src);
        let e = Lexer::new(src, "t.brain", &idx, 0, src.len()).next_token().unwrap_err();
        assert_eq!(e.code, "B0001");
    }
}
