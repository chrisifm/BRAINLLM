//! Per-scope mangling of local names. Analyzes token structure (statements, functions,
//! arrows, classes, objects, destructuring patterns, import/export) just enough to know which
//! identifier is a declaration, which a reference and which a property name. Never renames
//! properties, undeclared globals or exported names (except those pinned by `forced`).

use crate::lex::{Kind, Tok};
use crate::print::{next_continues, prev_continues};
use std::collections::{HashMap, HashSet};

const KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "let",
    "static",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
    "await",
    "arguments",
    "eval",
    "undefined",
    "NaN",
    "Infinity",
    "async",
    "of",
    "get",
    "set",
];

fn reserved(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

/// Words that are never names. Contextual ones (`get`, `set`, `of`, `async`, `static`, `let`,
/// `await`, `arguments`…) can be variables and are treated as references.
const HARD: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

fn hard(s: &str) -> bool {
    HARD.contains(&s)
}

#[derive(Debug)]
struct Scope {
    parent: Option<usize>,
    start: usize,
    end: usize,
    is_fn: bool,
    decls: Vec<(String, usize)>,
}

#[derive(Clone)]
struct Part {
    start: usize,
    pattern_end: usize,
    init: Option<usize>,
    end: usize,
    names: Vec<usize>,
}
#[derive(Clone)]
struct Declaration {
    keyword: usize,
    end: usize,
    scope: usize,
    parts: Vec<Part>,
}
#[derive(Default)]
struct Marks {
    statements: HashSet<usize>,
    declarations: Vec<Declaration>,
    functions: Vec<(usize, usize, usize, usize)>,
    import_spans: Vec<(usize, usize)>,
    /// Identifiers that are references to resolve.
    refs: Vec<usize>,
    /// Declarations: token index → scope.
    decl_at: HashMap<usize, usize>,
    /// Object/pattern shorthand: when renamed it prints `orig:new`.
    short_obj: HashSet<usize>,
    /// Import shorthand: prints `orig as new`.
    short_import: HashSet<usize>,
    /// Exported local names: not renamed (except `forced`).
    exported: HashSet<String>,
    /// Named import: (external-name token, specifier token).
    imports: Vec<(usize, usize)>,
}

struct P<'a> {
    t: &'a [Tok],
    i: usize,
    scopes: Vec<Scope>,
    cur: usize,
    m: Marks,
}

pub struct Error(pub String);

impl<'a> P<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.t.get(self.i)
    }
    fn at(&self, k: usize) -> Option<&'a Tok> {
        self.t.get(self.i + k)
    }
    fn p(&self, s: &str) -> bool {
        self.peek().is_some_and(|t| t.punct(s))
    }
    fn w(&self, s: &str) -> bool {
        self.peek().is_some_and(|t| t.ident(s))
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.p(s) || self.w(s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<(), Error> {
        if self.eat(s) {
            Ok(())
        } else {
            let ctx: Vec<&str> = self.t[self.i.saturating_sub(8)..(self.i + 4).min(self.t.len())].iter().map(|t| t.text.as_str()).collect();
            Err(Error(format!("expected `{s}` at token {} near `{}`", self.i, ctx.join(" "))))
        }
    }
    fn open(&mut self, is_fn: bool) -> usize {
        self.scopes.push(Scope { parent: Some(self.cur), start: self.i, end: self.t.len(), is_fn, decls: vec![] });
        self.cur = self.scopes.len() - 1;
        self.cur
    }
    fn close(&mut self, s: usize) {
        self.scopes[s].end = self.i;
        self.cur = self.scopes[s].parent.unwrap_or(0);
    }
    fn fn_scope(&self) -> usize {
        let mut s = self.cur;
        while !self.scopes[s].is_fn {
            s = self.scopes[s].parent.unwrap_or(0);
            if s == 0 {
                break;
            }
        }
        s
    }
    fn declare(&mut self, idx: usize, scope: usize) {
        let name = self.t[idx].text.clone();
        self.scopes[scope].decls.push((name, idx));
        self.m.decl_at.insert(idx, scope);
    }
    fn is_name(&self) -> bool {
        self.peek().is_some_and(|t| t.kind == Kind::Ident)
    }
    /// Index of the `)`/`]`/`}` that closes the one at `self.i`.
    fn matching(&self, from: usize) -> Option<usize> {
        let mut d = 0i32;
        for j in from..self.t.len() {
            let t = &self.t[j];
            match t.kind {
                Kind::Punct if matches!(t.text.as_str(), "(" | "[" | "{") => d += 1,
                Kind::Punct if matches!(t.text.as_str(), ")" | "]" | "}") => {
                    d -= 1;
                    if d == 0 {
                        return Some(j);
                    }
                }
                Kind::TplHead => d += 1,
                Kind::TplTail => d -= 1,
                _ => {}
            }
        }
        None
    }

    // ---------- statements ----------

    fn statements(&mut self) -> Result<(), Error> {
        while let Some(t) = self.peek() {
            if t.punct("}") {
                return Ok(());
            }
            let before = self.i;
            self.m.statements.insert(self.i);
            self.statement()?;
            if self.i == before {
                return Err(Error(format!("no progress at `{}` (token {})", t.text, self.i)));
            }
        }
        Ok(())
    }

    fn statement(&mut self) -> Result<(), Error> {
        let t = self.peek().ok_or_else(|| Error("unexpected end of JavaScript structure".into()))?;
        if t.kind == Kind::Punct {
            match t.text.as_str() {
                "{" => return self.block(),
                ";" => {
                    self.i += 1;
                    return Ok(());
                }
                _ => {}
            }
        }
        if t.kind == Kind::Ident {
            let next = self.at(1);
            // label `x:`
            if next.is_some_and(|n| n.punct(":")) && !reserved(&t.text) {
                self.i += 2;
                return Ok(());
            }
            match t.text.as_str() {
                "let" | "const" | "var" if next.is_some_and(|n| n.kind == Kind::Ident || n.punct("[") || n.punct("{")) => {
                    let kind = t.text.clone();
                    self.i += 1;
                    self.decl(&kind)?;
                    self.eat(";");
                    return Ok(());
                }
                "function" => {
                    let (start, scope) = (self.i, self.cur);
                    self.function_decl()?;
                    if let Some((_, name)) = self.scopes[scope].decls.iter().find(|(_, at)| *at > start && *at < self.i) {
                        self.m.functions.push((start, self.i, scope, *name));
                    }
                    return Ok(());
                }
                "async" if next.is_some_and(|n| n.ident("function")) && !next.unwrap().nl_before => {
                    let (start, scope) = (self.i, self.cur);
                    self.i += 1;
                    self.function_decl()?;
                    if let Some((_, name)) = self.scopes[scope].decls.iter().find(|(_, at)| *at > start && *at < self.i) {
                        self.m.functions.push((start, self.i, scope, *name));
                    }
                    return Ok(());
                }
                "class" => {
                    self.i += 1;
                    let s = self.cur;
                    if self.is_name() && !self.w("extends") {
                        let idx = self.i;
                        self.declare(idx, s);
                        self.i += 1;
                    }
                    return self.class_tail();
                }
                "import" if !next.is_some_and(|n| n.punct("(") || n.punct(".")) => return self.import(),
                "export" => return self.export(),
                "if" => {
                    self.i += 1;
                    self.paren_expr()?;
                    self.statement()?;
                    if self.eat("else") {
                        self.statement()?;
                    }
                    return Ok(());
                }
                "while" => {
                    self.i += 1;
                    self.paren_expr()?;
                    return self.statement();
                }
                "do" => {
                    self.i += 1;
                    self.statement()?;
                    self.expect("while")?;
                    self.paren_expr()?;
                    self.eat(";");
                    return Ok(());
                }
                "for" => return self.for_stmt(),
                "try" => {
                    self.i += 1;
                    self.block()?;
                    if self.eat("catch") {
                        let s = self.open(false);
                        if self.eat("(") {
                            self.pattern(s)?;
                            self.expect(")")?;
                        }
                        self.expect("{")?;
                        self.statements()?;
                        self.expect("}")?;
                        self.close(s);
                    }
                    if self.eat("finally") {
                        self.block()?;
                    }
                    return Ok(());
                }
                "switch" => {
                    self.i += 1;
                    self.paren_expr()?;
                    self.expect("{")?;
                    let s = self.open(false);
                    while !self.p("}") && self.peek().is_some() {
                        if self.eat("case") {
                            self.expr(false, false)?;
                            self.expect(":")?;
                        } else if self.eat("default") {
                            self.expect(":")?;
                        } else {
                            self.statement()?;
                        }
                    }
                    self.close(s);
                    self.expect("}")?;
                    return Ok(());
                }
                "return" | "throw" => {
                    self.i += 1;
                    if !self.stmt_ends_here() {
                        self.expr(false, true)?;
                    }
                    self.eat(";");
                    return Ok(());
                }
                "break" | "continue" => {
                    self.i += 1;
                    if self.is_name() && !self.peek().unwrap().nl_before {
                        self.i += 1; // label
                    }
                    self.eat(";");
                    return Ok(());
                }
                "with" => return Err(Error("`with` prevents safe mangling".into())),
                _ => {}
            }
        }
        self.expr(false, true)?;
        self.eat(";");
        Ok(())
    }

    fn stmt_ends_here(&self) -> bool {
        match self.peek() {
            None => true,
            Some(t) => t.punct(";") || t.punct("}") || t.nl_before,
        }
    }

    fn block(&mut self) -> Result<(), Error> {
        self.expect("{")?;
        let s = self.open(false);
        self.statements()?;
        self.close(s);
        self.expect("}")
    }

    fn paren_expr(&mut self) -> Result<(), Error> {
        self.expect("(")?;
        self.expr(false, false)?;
        self.expect(")")
    }

    fn decl(&mut self, kind: &str) -> Result<(), Error> {
        let target = if kind == "var" { self.fn_scope() } else { self.cur };
        let keyword = self.i.saturating_sub(1);
        let mut parts = vec![];
        loop {
            let start = self.i;
            let before = self.scopes[target].decls.len();
            self.pattern(target)?;
            let pattern_end = self.i;
            let names = self.scopes[target].decls[before..].iter().map(|(_, at)| *at).collect();
            let init = if self.eat("=") {
                let at = self.i;
                self.expr(true, true)?;
                Some(at)
            } else {
                None
            };
            parts.push(Part { start, pattern_end, init, end: self.i, names });
            if !self.eat(",") {
                self.m.declarations.push(Declaration { keyword, end: self.i, scope: target, parts });
                return Ok(());
            }
        }
    }

    fn for_stmt(&mut self) -> Result<(), Error> {
        self.i += 1;
        self.eat("await");
        self.expect("(")?;
        let s = self.open(false);
        if self.w("let") || self.w("const") || self.w("var") {
            let k = self.peek().unwrap().text.clone();
            self.i += 1;
            self.decl(&k)?;
        } else if self.is_name() && self.at(1).is_some_and(|n| n.ident("of") || n.ident("in")) {
            // for (x of y) with x already declared
            self.m.refs.push(self.i);
            self.i += 1;
        } else if !self.p(";") {
            self.expr(false, false)?;
        }
        // for (x of y) / for (x in y) / for (;;)
        if self.eat("of") || self.eat("in") {
            self.expr(false, false)?;
        } else {
            self.expect(";")?;
            if !self.p(";") {
                self.expr(false, false)?;
            }
            self.expect(";")?;
            if !self.p(")") {
                self.expr(false, false)?;
            }
        }
        self.expect(")")?;
        self.statement()?;
        self.close(s);
        Ok(())
    }

    fn function_decl(&mut self) -> Result<(), Error> {
        self.expect("function")?;
        self.eat("*");
        if self.is_name() {
            let idx = self.i;
            let s = self.cur;
            self.declare(idx, s);
            self.i += 1;
        }
        self.function_rest()
    }

    /// From the parameters' `(` to the body's `}`.
    fn function_rest(&mut self) -> Result<(), Error> {
        let s = self.open(true);
        self.params()?;
        self.expect("{")?;
        self.statements()?;
        self.expect("}")?;
        self.close(s);
        Ok(())
    }

    fn params(&mut self) -> Result<(), Error> {
        let s = self.cur;
        self.expect("(")?;
        while !self.p(")") {
            self.eat("...");
            self.pattern(s)?;
            if self.eat("=") {
                self.expr(true, false)?;
            }
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")
    }

    /// Binding pattern: name, `[..]` or `{..}` (with defaults and rest).
    fn pattern(&mut self, s: usize) -> Result<(), Error> {
        if self.peek().is_none() {
            return Err(Error("unterminated binding pattern".into()));
        }
        if self.is_name() {
            let idx = self.i;
            self.declare(idx, s);
            self.i += 1;
            return Ok(());
        }
        if self.eat("[") {
            while !self.p("]") {
                if self.eat(",") {
                    continue;
                }
                self.eat("...");
                self.pattern(s)?;
                if self.eat("=") {
                    self.expr(true, false)?;
                }
                if !self.eat(",") {
                    break;
                }
            }
            return self.expect("]");
        }
        if self.eat("{") {
            while !self.p("}") {
                if self.eat("...") {
                    self.pattern(s)?;
                } else if self.eat("[") {
                    self.expr(false, false)?;
                    self.expect("]")?;
                    self.expect(":")?;
                    self.pattern(s)?;
                } else {
                    let key = self.i;
                    if self.peek().is_none() {
                        return Err(Error("unterminated binding property".into()));
                    }
                    self.i += 1;
                    if self.eat(":") {
                        self.pattern(s)?;
                    } else if self.t[key].kind == Kind::Ident {
                        self.declare(key, s);
                        self.m.short_obj.insert(key);
                    }
                }
                if self.eat("=") {
                    self.expr(true, false)?;
                }
                if !self.eat(",") {
                    break;
                }
            }
            return self.expect("}");
        }
        Err(Error(format!("unexpected pattern `{}` (token {})", self.peek().map_or("EOF", |t| &t.text), self.i)))
    }

    fn class_tail(&mut self) -> Result<(), Error> {
        if self.eat("extends") {
            // Heritage expression up to the body's `{`.
            while !self.p("{") {
                self.primary()?;
            }
        }
        self.expect("{")?;
        while !self.p("}") {
            if self.eat(";") {
                continue;
            }
            if self.w("static") && self.at(1).is_some_and(|n| n.punct("{")) {
                self.i += 1;
                self.block()?;
                continue;
            }
            self.member(true)?;
        }
        self.expect("}")
    }

    /// Class or object member: method, getter/setter, field or property.
    fn member(&mut self, class: bool) -> Result<(), Error> {
        if self.peek().is_none() {
            return Err(Error("unterminated member".into()));
        }
        loop {
            let mod_word = self.peek().is_some_and(|t| t.kind == Kind::Ident && matches!(t.text.as_str(), "static" | "get" | "set" | "async"));
            let next_is_key = self.at(1).is_some_and(|n| matches!(n.kind, Kind::Ident | Kind::Str | Kind::Num) || n.punct("[") || n.punct("*") || n.punct("#"));
            if mod_word && next_is_key {
                self.i += 1;
            } else {
                break;
            }
        }
        self.eat("*");
        self.eat("#");
        // key
        let key = self.i;
        if self.eat("[") {
            self.expr(false, false)?;
            self.expect("]")?;
        } else if self.eat("...") {
            return self.expr(true, false);
        } else {
            self.i += 1;
        }
        if self.p("(") {
            return self.function_rest();
        }
        if self.eat(":") {
            return self.expr(true, false);
        }
        if self.eat("=") {
            // class field or default in a pattern
            return self.expr(!class, class);
        }
        // shorthand `{a}` (object) or field without a value (class)
        if !class && self.t[key].kind == Kind::Ident {
            self.m.refs.push(key);
            self.m.short_obj.insert(key);
        }
        if class {
            self.eat(";");
        }
        Ok(())
    }

    fn object(&mut self) -> Result<(), Error> {
        self.expect("{")?;
        while !self.p("}") {
            self.member(false)?;
            if !self.eat(",") {
                break;
            }
        }
        self.expect("}")
    }

    fn import_tail(&mut self) -> Result<(), Error> {
        if self.eat("with") || self.eat("assert") {
            if !self.p("{"){return Err(Error("import attributes require a literal object".into()));}
            let close=self.matching(self.i).ok_or_else(||Error("unterminated import attributes".into()))?;
            self.i=close+1;
        }
        self.eat(";");Ok(())
    }
    fn import(&mut self) -> Result<(), Error> {
        let start = self.i;
        self.i += 1;
        let s = 0;
        if self.peek().is_some_and(|t| t.kind == Kind::Str) {
            self.i += 1;
            self.import_tail()?;
            self.m.import_spans.push((start, self.i));
            return Ok(());
        }
        let first_ext = self.m.imports.len();
        loop {
            if self.eat("*") {
                self.expect("as")?;
                let idx = self.i;
                self.declare(idx, s);
                self.i += 1;
            } else if self.eat("{") {
                while !self.p("}") {
                    if self.peek().is_none() {
                        return Err(Error("unterminated import list".into()));
                    }
                    let a = self.i;
                    self.m.imports.push((a, 0));
                    self.i += 1;
                    if self.eat("as") {
                        let b = self.i;
                        self.declare(b, s);
                        self.i += 1;
                    } else {
                        self.declare(a, s);
                        self.m.short_import.insert(a);
                    }
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("}")?;
            } else if self.is_name() && !self.w("from") {
                let idx = self.i;
                self.declare(idx, s);
                self.i += 1;
            }
            if !self.eat(",") {
                break;
            }
        }
        self.expect("from")?;
        let src = self.i;
        for e in &mut self.m.imports[first_ext..] {
            e.1 = src;
        }
        self.i += 1; // specifier
        self.import_tail()?;
        self.m.import_spans.push((start, self.i));
        Ok(())
    }

    fn export(&mut self) -> Result<(), Error> {
        self.i += 1;
        if self.eat("default") {
            if self.w("function") || (self.w("async") && self.at(1).is_some_and(|n| n.ident("function"))) {
                self.eat("async");
                return self.function_decl();
            }
            if self.eat("class") {
                if self.is_name() && !self.w("extends") {
                    let idx = self.i;
                    let s = self.cur;
                    self.declare(idx, s);
                    self.i += 1;
                }
                return self.class_tail();
            }
            self.expr(false, true)?;
            self.eat(";");
            return Ok(());
        }
        if self.eat("*") {
            if self.eat("as") {
                self.i += 1;
            }
            self.expect("from")?;
            self.i += 1;
            self.eat(";");
            return Ok(());
        }
        if self.p("{") {
            let close = self.matching(self.i).ok_or_else(|| Error("unterminated export {}".into()))?;
            let from = self.t.get(close + 1).is_some_and(|t| t.ident("from"));
            self.i += 1;
            while !self.p("}") {
                if self.peek().is_none() {
                    return Err(Error("unterminated export list".into()));
                }
                let a = self.i;
                if !from {
                    self.m.refs.push(a);
                    self.m.exported.insert(self.t[a].text.clone());
                }
                self.i += 1;
                if self.eat("as") {
                    self.i += 1;
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("}")?;
            if self.eat("from") {
                self.i += 1;
            }
            self.eat(";");
            return Ok(());
        }
        // export const/let/var/function/class: declared names stay fixed.
        let before = self.scopes[self.cur].decls.len();
        self.statement()?;
        let names: Vec<String> = self.scopes[self.cur].decls[before..].iter().map(|(n, _)| n.clone()).collect();
        self.m.exported.extend(names);
        Ok(())
    }

    // ---------- expressions ----------

    /// Walks an expression. `stop_comma`: a `,` at this level ends it. `stmt`: a line break that
    /// would trigger ASI ends it (statement level).
    fn expr(&mut self, stop_comma: bool, stmt: bool) -> Result<(), Error> {
        let start = self.i;
        while let Some(t) = self.peek() {
            if self.i > start && stmt && t.nl_before {
                let prev = &self.t[self.i - 1];
                if !(prev_continues(prev) || next_continues(t)) {
                    return Ok(());
                }
            }
            if t.kind == Kind::Punct {
                match t.text.as_str() {
                    ")" | "]" | "}" | ";" | ":" => return Ok(()),
                    "," if stop_comma => return Ok(()),
                    "?" => {
                        self.i += 1;
                        self.expr(true, false)?;
                        self.expect(":")?;
                        continue;
                    }
                    _ => {}
                }
            }
            if matches!(t.kind, Kind::TplMiddle | Kind::TplTail) {
                return Ok(());
            }
            self.primary()?;
        }
        Ok(())
    }

    /// One atom or punctuator of an expression.
    fn primary(&mut self) -> Result<(), Error> {
        let t = self.peek().ok_or_else(|| Error("unexpected end of JavaScript structure".into()))?;
        match t.kind {
            Kind::Punct => match t.text.as_str() {
                "(" => {
                    let close = self.matching(self.i).ok_or_else(|| Error("unterminated parenthesis".into()))?;
                    if self.t.get(close + 1).is_some_and(|n| n.punct("=>")) {
                        return self.arrow();
                    }
                    self.i += 1;
                    self.expr(false, false)?;
                    self.expect(")")
                }
                "[" => {
                    self.i += 1;
                    while !self.p("]") {
                        if self.eat(",") {
                            continue;
                        }
                        self.eat("...");
                        self.expr(true, false)?;
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect("]")
                }
                "{" => self.object(),
                "." | "?." => {
                    self.i += 1;
                    self.eat("#");
                    if self.is_name() {
                        self.i += 1; // property
                    }
                    Ok(())
                }
                _ => {
                    self.i += 1;
                    Ok(())
                }
            },
            Kind::TplHead => {
                self.i += 1;
                loop {
                    self.expr(false, false)?;
                    match self.peek() {
                        Some(t) if t.kind == Kind::TplMiddle => self.i += 1,
                        Some(t) if t.kind == Kind::TplTail => {
                            self.i += 1;
                            return Ok(());
                        }
                        _ => return Err(Error("unterminated template".into())),
                    }
                }
            }
            Kind::Ident => {
                let next = self.at(1);
                match t.text.as_str() {
                    "function" => {
                        self.i += 1;
                        self.eat("*");
                        let s = self.open(true);
                        if self.is_name() {
                            let idx = self.i;
                            self.declare(idx, s);
                            self.i += 1;
                        }
                        self.params()?;
                        self.expect("{")?;
                        self.statements()?;
                        self.expect("}")?;
                        self.close(s);
                        Ok(())
                    }
                    "class" => {
                        self.i += 1;
                        let s = self.open(false);
                        if self.is_name() && !self.w("extends") {
                            let idx = self.i;
                            self.declare(idx, s);
                            self.i += 1;
                        }
                        self.class_tail()?;
                        self.close(s);
                        Ok(())
                    }
                    "async" if next.is_some_and(|n| n.punct("(") || (n.kind == Kind::Ident && self.at(2).is_some_and(|m| m.punct("=>")))) => {
                        self.i += 1;
                        if self.p("(") {
                            let close = self.matching(self.i).ok_or_else(|| Error("unterminated parenthesis".into()))?;
                            if self.t.get(close + 1).is_some_and(|n| n.punct("=>")) {
                                return self.arrow();
                            }
                            // call to a function named async
                            return Ok(());
                        }
                        self.arrow()
                    }
                    _ if next.is_some_and(|n| n.punct("=>")) && !reserved(&t.text) => self.arrow(),
                    w if hard(w) => {
                        self.i += 1;
                        Ok(())
                    }
                    _ => {
                        self.m.refs.push(self.i);
                        self.i += 1;
                        Ok(())
                    }
                }
            }
            _ => {
                self.i += 1;
                Ok(())
            }
        }
    }

    fn arrow(&mut self) -> Result<(), Error> {
        let s = self.open(true);
        if self.p("(") {
            self.params()?;
        } else {
            let idx = self.i;
            self.declare(idx, s);
            self.i += 1;
        }
        self.expect("=>")?;
        if self.p("{") {
            self.i += 1;
            self.statements()?;
            self.expect("}")?;
        } else {
            self.expr(true, true)?;
        }
        self.close(s);
        Ok(())
    }
}

/// Short-name generator: a..z A..Z $ _ and then two characters.
struct Names {
    n: usize,
}
impl Names {
    fn nth(mut i: usize) -> String {
        const FIRST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ$_";
        const REST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ$_0123456789";
        let mut s = String::new();
        s.push(FIRST[i % FIRST.len()] as char);
        i /= FIRST.len();
        while i > 0 {
            i -= 1;
            s.push(REST[i % REST.len()] as char);
            i /= REST.len();
        }
        s
    }
    fn next_free(&mut self, taken: &HashSet<String>) -> String {
        loop {
            let s = Self::nth(self.n);
            self.n += 1;
            if !taken.contains(&s) && !reserved(&s) && !matches!(s.as_str(), "do" | "if" | "in" | "of") {
                return s;
            }
        }
    }
}

/// Renames local declarations. `forced` pins names (e.g. runtime exports → short map);
/// `keep` are names that are never touched (internal exports). Returns the tokens with the new text.
/// What to rename: `forced` pins module-level names (runtime exports → short map); `keep` are never
/// touched; `imports` renames the EXTERNAL name of imports whose specifier satisfies `matches`.
pub struct Plan<'a> {
    pub mangle: bool,
    pub forced: &'a HashMap<String, String>,
    pub keep: &'a HashSet<String>,
    pub import_map: &'a HashMap<String, String>,
    pub import_from: &'a dyn Fn(&str) -> bool,
}

/// Read-only lexical ownership query for a small set of native identifiers. No names are assigned or rewritten.
/// Returns original byte offsets and names only; properties and declaration keys are not references.
pub fn free_identifier_sites(toks:&[Tok], names:&HashSet<String>)->Result<Vec<(usize,String)>,String> {
    if toks.len()>250000 || names.len()>16 || names.iter().any(|name|name.len()>80) {return Err("identifier ownership input budget exceeded".into());}
    if toks.windows(2).any(|pair|pair[0].ident("with")&&pair[1].punct("(")){return Err("identifier ownership cannot be proven with dynamic with scope".into());}
    // Bound parser recursion before entering it, including arrow/conditional chains without parentheses.
    let mut nesting=vec![(0usize,0usize)];
    for token in toks {
        if token.kind==Kind::TplHead || ["(","[","{"].iter().any(|p|token.punct(p)) {nesting.push((0,0));}
        else if token.kind==Kind::TplTail || [")","]","}"].iter().any(|p|token.punct(p)) {if nesting.len()>1 {nesting.pop();}}
        else if let Some((arrows,questions))=nesting.last_mut() {
            if token.punct("=>"){*arrows+=1;}else if token.punct("?"){*questions+=1;}else if token.punct(":"){*questions=questions.saturating_sub(1);}else if token.punct(";")||token.punct(",")||token.kind==Kind::TplMiddle{*arrows=0;*questions=0;}
        }
        if nesting.len()+nesting.iter().map(|(a,q)|a+q).sum::<usize>()>256 {return Err("identifier ownership nesting budget exceeded".into());}
    }
    let mut parser=P {t:toks,i:0,scopes:vec![Scope{parent:None,start:0,end:toks.len(),is_fn:true,decls:vec![]}],cur:0,m:Marks::default()};
    parser.statements().map_err(|_|"identifier ownership structure could not be proven")?;
    if parser.i!=toks.len() || parser.scopes.len()>10000 {return Err("identifier ownership scope budget exceeded".into());}
    let mut ranges:HashMap<String,Vec<(usize,usize)>>=HashMap::new();
    for scope in &parser.scopes {for(name,_)in &scope.decls {if names.contains(name)||name=="eval" {ranges.entry(name.clone()).or_default().push((scope.start,scope.end.max(scope.start+1)));}}}
    let bound=|name:&str,at:usize|ranges.get(name).is_some_and(|ranges|ranges.iter().any(|(start,end)|*start<=at&&at<*end));
    let mut result=Vec::new();
    for index in parser.m.refs {
        let token=&toks[index];
        if token.ident("eval") && toks.get(index+1).is_some_and(|n|n.punct("(")) && !bound("eval",index){return Err("identifier ownership cannot be proven with direct eval".into());}
        if names.contains(&token.text) && !bound(&token.text,index){result.push((token.start,token.text.clone()));}
    }
    result.sort();result.dedup();Ok(result)
}

pub fn mangle(toks: &[Tok], forced: &HashMap<String, String>, keep: &HashSet<String>) -> Result<Vec<Tok>, String> {
    let none = HashMap::new();
    apply(toks, &Plan { mangle: true, forced, keep, import_map: &none, import_from: &|_| false })
}

/// Module-level exports that have no pinned name in `forced` and are not in `keep`.
pub fn exports_outside(toks: &[Tok], forced: &HashMap<String, String>, keep: &HashSet<String>) -> Result<Vec<String>, String> {
    let mut p = P { t: toks, i: 0, scopes: vec![Scope { parent: None, start: 0, end: toks.len(), is_fn: true, decls: vec![] }], cur: 0, m: Marks::default() };
    p.statements().map_err(|e| e.0)?;
    let mut v: Vec<String> = p.m.exported.into_iter().filter(|n| !forced.contains_key(n) && !keep.contains(n)).collect();
    v.sort();
    Ok(v)
}

pub fn apply(toks: &[Tok], plan: &Plan) -> Result<Vec<Tok>, String> {
    let (forced, keep) = (plan.forced, plan.keep);
    let mut p = P { t: toks, i: 0, scopes: vec![Scope { parent: None, start: 0, end: toks.len(), is_fn: true, decls: vec![] }], cur: 0, m: Marks::default() };
    p.statements().map_err(|e| e.0)?;
    if p.i != toks.len() {
        return Err(format!("unrecognized structure near token {} (`{}`)", p.i, toks[p.i].text));
    }
    let P { scopes, m, .. } = p;
    // External name of each import that points at the runtime → short key.
    let mut ext_new: HashMap<usize, String> = HashMap::new();
    for &(ext, src) in &m.imports {
        let spec = toks[src].text.trim_matches(|c| c == '"' || c == '\'' || c == '`');
        if (plan.import_from)(spec) {
            let name = &toks[ext].text;
            match plan.import_map.get(name) {
                Some(n) => {
                    ext_new.insert(ext, n.clone());
                }
                None if keep.contains(name) => {}
                None => return Err(format!("`{name}` is imported from `{spec}` but is not in RUNTIME_MAP nor in the fixed-names list")),
            }
        }
    }

    // Resolve references to the innermost scope that declares the name.
    let lookup = |name: &str, at: usize| -> Option<usize> {
        let mut best: Option<usize> = None;
        for (si, s) in scopes.iter().enumerate() {
            if s.start <= at && at < s.end.max(s.start + 1) && s.decls.iter().any(|(n, _)| n == name) && best.is_none_or(|b| scopes[b].start <= s.start) {
                best = Some(si);
            }
        }
        best
    };
    let mut globals: HashSet<String> = HashSet::new();
    // (scope, name) → reference indices
    let mut uses: HashMap<(usize, String), Vec<usize>> = HashMap::new();
    for &r in &m.refs {
        let name = &toks[r].text;
        match lookup(name, r) {
            Some(s) => uses.entry((s, name.clone())).or_default().push(r),
            None => {
                globals.insert(name.clone());
            }
        }
    }
    // Direct `eval` makes it impossible to know what is used: nothing is renamed.
    if globals.contains("eval") {
        return Ok(toks.to_vec());
    }

    // Assign names top-down (scopes are in opening order: parents before children).
    let mut final_name: HashMap<(usize, String), String> = HashMap::new();
    let contains = |outer: usize, at: usize| scopes[outer].start <= at && at < scopes[outer].end;
    for (si, s) in scopes.iter().enumerate() {
        let mut taken: HashSet<String> = globals.clone();
        // final names of outer bindings referenced inside this scope
        for ((os, on), refs) in &uses {
            if *os != si && refs.iter().any(|r| contains(si, *r)) {
                if let Some(f) = final_name.get(&(*os, on.clone())) {
                    taken.insert(f.clone());
                } else {
                    taken.insert(on.clone());
                }
            }
        }
        // Declaration order without duplicates: output must be deterministic (same build → same bytes).
        let mut seen = HashSet::new();
        let mut names: Vec<&String> = s.decls.iter().map(|(n, _)| n).filter(|n| seen.insert(*n)).collect();
        // pinned names first (they stay reserved), then by use count
        names.sort_by_key(|n| std::cmp::Reverse(uses.get(&(si, (*n).clone())).map_or(0, Vec::len)));
        names.sort_by_key(|n| (si != 0 || !(m.exported.contains(*n) || keep.contains(*n) || forced.contains_key(*n))) as u8);
        let mut namer = Names { n: 0 };
        for n in names {
            let fixed = if si == 0 {
                if let Some(f) = forced.get(n) {
                    Some(f.clone())
                } else if m.exported.contains(n) || keep.contains(n) {
                    Some(n.clone())
                } else {
                    None
                }
            } else {
                None
            };
            let new = match fixed {
                Some(f) => f,
                None if plan.mangle => namer.next_free(&taken),
                None => n.clone(),
            };
            taken.insert(new.clone());
            final_name.insert((si, n.clone()), new);
        }
    }

    let mut out = toks.to_vec();
    let rename = |idx: usize, scope: usize, out: &mut Vec<Tok>| {
        let orig = toks[idx].text.clone();
        let new = final_name.get(&(scope, orig.clone())).cloned().unwrap_or_else(|| orig.clone());
        if m.short_import.contains(&idx) {
            let ext = ext_new.get(&idx).cloned().unwrap_or_else(|| orig.clone());
            out[idx].text = if ext == new { new } else { format!("{ext} as {new}") };
            return;
        }
        if new == orig {
            return;
        }
        out[idx].text = if m.short_obj.contains(&idx) { format!("{orig}:{new}") } else { new };
    };
    for (&idx, &scope) in &m.decl_at {
        rename(idx, scope, &mut out);
    }
    for ((scope, _), refs) in &uses {
        for &r in refs {
            rename(r, *scope, &mut out);
        }
    }
    // `{sig as x}`: the external name changes to the short key.
    for (&idx, n) in &ext_new {
        if !m.short_import.contains(&idx) {
            out[idx].text = n.clone();
        }
    }
    Ok(out)
}

/// Replaces `console.log(...)` with `void 0` (SPEC: `log()` never reaches production).
pub fn drop_console_log(toks: Vec<Tok>) -> Vec<Tok> {
    cleanup_engine::drop_logs(toks.clone(), None).map(|(t, _)| t).unwrap_or(toks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::lex;
    use crate::print::print;

    fn m(src: &str) -> String {
        print(&mangle(&lex(src).unwrap(), &HashMap::new(), &HashSet::new()).unwrap())
    }

    #[test]
    fn renames_locals_not_props_or_globals() {
        assert_eq!(
            m("function outer(longName, other) { const total = longName + other.value; return total + window.x }"),
            "function a(a,b){const c=a+b.value;return c+window.x}"
        );
    }

    #[test]
    fn shorthand_and_destructuring() {
        assert_eq!(m("const f = (value) => ({ value, k: value })"), "const a=(a)=>({value:a,k:a})");
        assert_eq!(
            m("function g({ alpha, beta: gamma = 1 }, [x, , ...rest]) { return alpha + gamma + x + rest }"),
            "function a({alpha:a,beta:b=1},[c,,...d]){return a+b+c+d}"
        );
    }

    #[test]
    fn exports_and_imports() {
        assert_eq!(
            m("import { sig, drv as derive } from './b.js'\nexport function make(seed) { return derive(() => sig(seed)) }"),
            "import{sig as a,drv as b}from'./b.js'\nexport function make(c){return b(()=>a(c))}"
        );
    }

    #[test]
    fn shadowing_is_respected() {
        // The parameter may share the function's name (unused inside), but never `a`, which is used.
        let out = m("const first = 1; function f(second) { return first + second }");
        assert_eq!(out, "const a=1;function b(b){return a+b}");
    }

    #[test]
    fn classes_keep_members() {
        assert_eq!(
            m("class Node { constructor(value) { this.value = value } get v() { return this.value } m(s) { return s } }\nnew Node(1)"),
            "class a{constructor(a){this.value=a}get v(){return this.value}m(a){return a}}\nnew a(1)"
        );
    }

    #[test]
    fn for_of_with_existing_binding() {
        assert_eq!(m("function f(list) { let item; for (item of list) use(item) }"), "function a(b){let a;for(a of b)use(a)}");
    }

    #[test]
    fn contextual_words_are_names() {
        assert_eq!(m("function f() { const set = new Set(); set.add(1); return set }"), "function a(){const a=new Set();a.add(1);return a}");
    }

    #[test]
    fn runtime_map_on_exports_and_imports() {
        let map: HashMap<String, String> = [("sig", "s"), ("mount", "m")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let keep: HashSet<String> = ["_ev".to_string()].into();
        // the runtime: exports renamed to the map
        let rt = apply(
            &lex("export const sig = v => v\nexport function mount(C) { return sig(C) }\nexport function _ev() {}").unwrap(),
            &Plan { mangle: true, forced: &map, keep: &keep, import_map: &map, import_from: &|_| false },
        )
        .unwrap();
        assert_eq!(print(&rt), "export const s=a=>a\nexport function m(a){return s(a)}\nexport function _ev(){}");
        // a consumer: imports rewritten even without mangling
        let none = HashMap::new();
        let from = |s: &str| s.ends_with("brain.js");
        let c = apply(
            &lex("import { sig, mount as mo, _ev } from '/@brain/brain.js'\nsig(mo)").unwrap(),
            &Plan { mangle: false, forced: &none, keep: &keep, import_map: &map, import_from: &from },
        )
        .unwrap();
        assert_eq!(print(&c), "import{s as sig,m as mo,_ev}from'/@brain/brain.js'\nsig(mo)");
        // unmapped export → error naming it
        assert_eq!(exports_outside(&lex("export const fresh = 1").unwrap(), &map, &keep).unwrap(), vec!["fresh".to_string()]);
        let bad = apply(
            &lex("import { fresh } from './brain.js'").unwrap(),
            &Plan { mangle: false, forced: &none, keep: &keep, import_map: &map, import_from: &from },
        );
        assert!(bad.unwrap_err().contains("fresh"));
    }

    #[test]
    fn drop_log() {
        let t = drop_console_log(lex("fx(() => console.log(n.v, f(x)))").unwrap());
        assert_eq!(print(&t), "fx(()=>void(n.v,f(x)))");
    }
}

// ---------- tree shaking ----------

/// Removes unreferenced module exports using the same conservative scope/effect
/// analysis as production cleanup. Imports retain module evaluation by default.
pub fn tree_shake(toks: &[Tok], keep_exports: &HashSet<String>) -> Result<Vec<Tok>, String> {
    cleanup_engine::shake_tokens(toks, None, keep_exports).map(|(t, _)| t).map_err(|e| e.to_string())
}

/// Legacy local-only entry point; shares the production cleanup engine. Unused
/// bindings with effectful initializers become initializer expression statements.
pub fn drop_unused_locals(toks: Vec<Tok>) -> Result<Vec<Tok>, String> {
    cleanup_engine::cleanup_tokens(toks, None, true).map(|(t, _)| t).map_err(|e| e.to_string())
}

#[cfg(test)]
mod dead_locals_tests {
    use super::*;
    use crate::lex::lex;
    use crate::print::print;

    fn run(src: &str) -> String {
        print(&drop_unused_locals(lex(src).unwrap()).unwrap())
    }

    #[test]
    fn drops_pure_unused_locals_only() {
        let src = "function C($p) { const unused = 5; const label = 'a' + 'b'; let keep = 1; const side = load(); const chain = [keep]; return label; }";
        // Proven initialized reads can disappear; the unused `side` binding is removed while load() still executes.
        assert_eq!(run(src), "function C($p){const label='a'+'b';(load());return label;}");
    }

    #[test]
    fn keeps_shadowed_written_and_module_names() {
        let src = "const top = 1;\nfunction f() { let n = 0; n = 2; const x = 1; const g = () => { const x = 2; return x }; return g }";
        assert_eq!(run(src), "const top=1;function f(){let n=0;n=2;const g=()=>{const x=2;return x};return g}");
    }
}

#[cfg(test)]
mod shake_tests {
    use super::*;
    use crate::lex::lex;
    use crate::print::print;

    fn shake(src: &str, keep: &[&str]) -> String {
        let keep: HashSet<String> = keep.iter().map(|s| s.to_string()).collect();
        print(&tree_shake(&lex(src).unwrap(), &keep).unwrap())
    }

    #[test]
    fn drops_unused_exports_and_their_helpers() {
        let src = "let state = null, other = 0\nconst helper = (x) => x + 1\nconst lonely = () => 2\nexport const a = (v) => helper(v) + state\nexport function b() { return lonely() }\nexport class C {}\ndocument.title = 'x'\n";
        assert_eq!(shake(src, &["a"]), "let state=null;const helper=(x)=>x+1;export const a=(v)=>helper(v)+state;document.title='x'");
    }

    #[test]
    fn keeps_side_effects_and_transitive_uses() {
        let src = "const registry = new Map()\nconst run = () => registry.size\nconst boot = init()\nexport const x = () => run()\n";
        // Initializer effects remain even when bindings disappear. `new Map` cannot assume an unmodified global constructor.
        assert_eq!(shake(src, &[]), "(new Map());(init());");
        assert_eq!(shake(src, &["x"]), "const registry=new Map()\nconst run=()=>registry.size;(init());export const x=()=>run()");
    }
}

#[path = "cleanup_engine.rs"]
mod cleanup_engine;
pub(crate) use cleanup_engine::{cleanup_tokens, drop_logs, shake_tokens};

#[cfg(test)]
mod ownership_query_tests {
    use super::*;
    use crate::lex::lex;
    #[test] fn queries_only_free_native_refs_without_rewriting_local_bindings() {
        let source="function local(exports,URL){return exports.malloc(new URL('x',import.meta.url))} const wasm={exports:{}};wasm.exports.memory;new URL('x',import.meta.url);process.env.NODE_ENV";
        let tokens=lex(source).unwrap();let names=["exports","URL","process"].into_iter().map(str::to_owned).collect();
        let result=free_identifier_sites(&tokens,&names).unwrap();
        assert_eq!(result.iter().filter(|(_,name)|name=="URL").count(),1);
        assert_eq!(result.iter().filter(|(_,name)|name=="exports").count(),0);
        assert_eq!(result.iter().filter(|(_,name)|name=="process").count(),1);
    }
    #[test] fn ownership_queries_bound_deep_inputs_and_direct_eval() {
        let names=["URL".to_owned()].into();
        assert!(free_identifier_sites(&lex(&format!("{}x{}", "(".repeat(300),")".repeat(300))).unwrap(),&names).is_err());
        assert!(free_identifier_sites(&lex("eval('let URL');new URL('x',import.meta.url)").unwrap(),&names).is_err());
        assert!(free_identifier_sites(&lex("const URL=custom;new URL('x',import.meta.url)").unwrap(),&names).unwrap().is_empty());
    }
}
