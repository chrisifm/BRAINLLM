//! Recursive-descent parser for BrainScript (the closed JS subset of SPEC §2).
//! Constructs that are outside the subset and cannot even be represented in the AST
//! (`class`, `with`, generators, getters/setters, labels, `for in`, classic loops) fail here with
//! their B01xx code and an exact fix; `this` and `var` are parsed and reported by the checker.

use crate::ast::*;
use brain_lex::{Diag, Lexer, Span, Tok, Token};

pub struct JsParser<'a> {
    lx: Lexer<'a>,
    tok: Token,
    prev_end: usize,
}

type R<T> = Result<T, Diag>;

const ASSIGN_OPS: &[&str] = &["=", "+=", "-=", "*=", "/=", "%=", "**=", "<<=", ">>=", ">>>=", "&=", "|=", "^=", "&&=", "||=", "??="];

fn binary_prec(t: &Token) -> Option<(&'static str, u8)> {
    let p = match &t.tok {
        Tok::Punct(p) => *p,
        Tok::Ident(w) if w == "instanceof" => return Some(("instanceof", 8)),
        Tok::Ident(w) if w == "in" => return Some(("in", 8)),
        _ => return None,
    };
    let prec = match p {
        "??" => 1,
        "||" => 2,
        "&&" => 3,
        "|" => 4,
        "^" => 5,
        "&" => 6,
        "==" | "!=" | "===" | "!==" => 7,
        "<" | ">" | "<=" | ">=" => 8,
        "<<" | ">>" | ">>>" => 9,
        "+" | "-" => 10,
        "*" | "/" | "%" => 11,
        "**" => 12,
        _ => return None,
    };
    Some((p, prec))
}

impl<'a> JsParser<'a> {
    pub fn new(mut lx: Lexer<'a>) -> R<Self> {
        let start = lx.pos();
        let tok = lx.next_token()?;
        Ok(JsParser { lx, tok, prev_end: start })
    }

    /// Byte offset just after the current token (used by the markup parser after `}`).
    pub fn current(&self) -> &Token {
        &self.tok
    }

    fn file(&self) -> &'a str {
        self.lx.file()
    }

    fn bump(&mut self) -> R<Token> {
        let next = self.lx.next_token()?;
        let t = std::mem::replace(&mut self.tok, next);
        self.prev_end = t.span.end as usize;
        Ok(t)
    }

    fn span_from(&self, start: Span) -> Span {
        let mut s = start;
        s.end = self.prev_end as u32;
        s
    }

    fn peek2(&mut self) -> R<Token> {
        let st = self.lx.save();
        let t = self.lx.next_token();
        self.lx.restore(st);
        t
    }

    fn is(&self, p: &str) -> bool {
        self.tok.is_punct(p)
    }
    fn is_word(&self, w: &str) -> bool {
        self.tok.is_word(w)
    }
    fn eat(&mut self, p: &str) -> R<bool> {
        if self.is(p) {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn describe(t: &Token) -> String {
        match &t.tok {
            Tok::Ident(w) => format!("`{w}`"),
            Tok::Num(n) => format!("the number `{n}`"),
            Tok::Str { raw, .. } => format!("the string {raw}"),
            Tok::Regex(r) => format!("the regex `{r}`"),
            Tok::TplFull(_) | Tok::TplHead(_) | Tok::TplMiddle(_) | Tok::TplTail(_) => "a template string".into(),
            Tok::Punct(p) => format!("`{p}`"),
            Tok::Eof => "the end of the block".into(),
        }
    }

    fn unexpected(&self, expected: &str) -> Diag {
        Diag::new(
            "B0002",
            self.file(),
            self.tok.span,
            format!("expected {expected} but found {}", Self::describe(&self.tok)),
            format!("write {expected} at this position"),
        )
    }

    fn err(&self, code: &'static str, span: Span, msg: &str, fix: &str) -> Diag {
        Diag::new(code, self.file(), span, msg, fix)
    }

    fn expect(&mut self, p: &str) -> R<Token> {
        if self.is(p) { self.bump() } else { Err(self.unexpected(&format!("`{p}`"))) }
    }

    fn ident(&mut self) -> R<Ident> {
        match &self.tok.tok {
            Tok::Ident(w) if !is_reserved(w) => {
                let t = self.bump()?;
                let Tok::Ident(name) = t.tok else { unreachable!() };
                Ok(Ident { name, span: t.span })
            }
            Tok::Ident(w) => {
                let w = w.clone();
                Err(self.err("B0002", self.tok.span, &format!("`{w}` is a reserved word and cannot be used as a name"), &format!("rename the variable, e.g. `{w}Value`")))
            }
            _ => Err(self.unexpected("a name")),
        }
    }

    /// Any word, including keywords (property names after `.`).
    fn any_word(&mut self) -> R<Ident> {
        if let Tok::Ident(_) = &self.tok.tok {
            let t = self.bump()?;
            let Tok::Ident(name) = t.tok else { unreachable!() };
            Ok(Ident { name, span: t.span })
        } else {
            Err(self.unexpected("a property name"))
        }
    }

    fn semicolon(&mut self) -> R<()> {
        if self.eat(";")? || self.is("}") || self.tok.tok == Tok::Eof || self.tok.nl_before {
            Ok(())
        } else {
            Err(self.err(
                "B0002",
                self.tok.span,
                &format!("missing `;` or line break before {}", Self::describe(&self.tok)),
                "separate statements with a line break or `;`",
            ))
        }
    }

    // ───────────────────────── statements ─────────────────────────

    pub fn parse_program(&mut self) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        while self.tok.tok != Tok::Eof {
            out.push(self.statement()?);
        }
        Ok(out)
    }

    fn block_body(&mut self) -> R<Vec<Stmt>> {
        self.expect("{")?;
        let mut out = Vec::new();
        while !self.is("}") {
            if self.tok.tok == Tok::Eof {
                return Err(self.unexpected("`}`"));
            }
            out.push(self.statement()?);
        }
        self.bump()?;
        Ok(out)
    }

    pub fn statement(&mut self) -> R<Stmt> {
        let start = self.tok.span;
        let Tok::Ident(w) = self.tok.tok.clone() else {
            if self.is("{") {
                let body = self.block_body()?;
                return Ok(Stmt::Block { body, span: self.span_from(start) });
            }
            if self.is(";") {
                self.bump()?;
                return Ok(Stmt::Empty { span: start });
            }
            return self.expr_statement();
        };
        match w.as_str() {
            "let" | "const" | "var" => {
                let d = self.var_decl()?;
                self.semicolon()?;
                Ok(Stmt::Var(d))
            }
            "function" => Ok(Stmt::Func(self.function(false, start)?)),
            "async" if self.peek2()?.is_word("function") => {
                self.bump()?;
                Ok(Stmt::Func(self.function(true, start)?))
            }
            "if" => {
                self.bump()?;
                self.expect("(")?;
                let test = self.expression()?;
                self.expect(")")?;
                let cons = Box::new(self.statement()?);
                let alt = if self.is_word("else") {
                    self.bump()?;
                    Some(Box::new(self.statement()?))
                } else {
                    None
                };
                Ok(Stmt::If { test, cons, alt, span: self.span_from(start) })
            }
            "for" => self.for_statement(start),
            "while" | "do" => Err(self.err(
                "B0110",
                start,
                &format!("`{w}` does not exist in BrainScript"),
                "iterate with `for (const x of list)`; for N repetitions use `for (const i of Array.from({ length: n }, (_, i) => i))`",
            )),
            "switch" => Err(self.err("B0110", start, "`switch` does not exist in BrainScript", "use `if (...) {} else if (...) {}` or a lookup object: `const r = { a: 1, b: 2 }[key]`")),
            "return" => {
                self.bump()?;
                let arg = if self.is(";") || self.is("}") || self.tok.tok == Tok::Eof || self.tok.nl_before { None } else { Some(self.expression()?) };
                self.semicolon()?;
                Ok(Stmt::Return { arg, span: self.span_from(start) })
            }
            "try" => {
                self.bump()?;
                let block = self.block_body()?;
                let (mut param, mut handler, mut finalizer) = (None, None, None);
                if self.is_word("catch") {
                    self.bump()?;
                    if self.eat("(")? {
                        param = Some(self.binding_target()?);
                        self.expect(")")?;
                    }
                    handler = Some(self.block_body()?);
                }
                if self.is_word("finally") {
                    self.bump()?;
                    finalizer = Some(self.block_body()?);
                }
                if handler.is_none() && finalizer.is_none() {
                    return Err(self.unexpected("`catch` or `finally`"));
                }
                Ok(Stmt::Try { block, param, handler, finalizer, span: self.span_from(start) })
            }
            "throw" => {
                self.bump()?;
                let arg = self.expression()?;
                self.semicolon()?;
                Ok(Stmt::Throw { arg, span: self.span_from(start) })
            }
            "break" | "continue" => {
                let is_break = w == "break";
                self.bump()?;
                if matches!(self.tok.tok, Tok::Ident(_)) && !self.tok.nl_before {
                    return Err(self.err("B0108", self.tok.span, "labels do not exist in BrainScript", "use `break`/`continue` without a label or extract the loop into a function with `return`"));
                }
                self.semicolon()?;
                let span = self.span_from(start);
                Ok(if is_break { Stmt::Break { span } } else { Stmt::Continue { span } })
            }
            "import" if !self.peek2()?.is_punct(".") => self.import(start),
            "export" => {
                self.bump()?;
                if self.is_word("default") {
                    self.bump()?;
                    let expr = self.assignment()?;
                    self.semicolon()?;
                    return Ok(Stmt::ExportDefault { expr, span: self.span_from(start) });
                }
                let decl = self.statement()?;
                if !matches!(decl, Stmt::Func(_) | Stmt::Var(_)) {
                    return Err(self.err("B0002", decl.span(), "`export` only accepts a declaration", "write `export const x = ...` or `export function f() {}`"));
                }
                Ok(Stmt::Export { decl: Box::new(decl), span: self.span_from(start) })
            }
            "class" => Err(self.err("B0101", start, "`class` does not exist in BrainScript", "use functions and objects: `const make = (x) => ({ x, area: () => x * x })`")),
            "with" => Err(self.err("B0104", start, "`with` does not exist in BrainScript", "access properties through the explicit object: `obj.prop`")),
            "server" if {
                let next=self.peek2()?;
                next.span.line==start.line && matches!(next.tok,Tok::Ident(ref name) if matches!(name.as_str(),"const"|"let"|"async"|"function"))
            } => Err(self.err("B0704",start,"a server handler uses the reserved `server:` function declaration, not a variable modifier","write `server: async function save(ctx, data) { ... }`; do not add a semicolon after `server`")),
            "server" if self.peek2()?.is_punct(":") => {
                // `server: async function f(ctx, …) {}` — the one reserved label (SPEC §11.1).
                self.bump()?;
                self.bump()?;
                let fs = self.tok.span;
                let is_async = self.is_word("async");
                if is_async {
                    self.bump()?;
                }
                if !self.is_word("function") {
                    return Err(self.err("B0704", start, "`server:` can only precede a function", "write `server: async function save(ctx, data) { … }`"));
                }
                let func = self.function(is_async, fs)?;
                if func.name.is_none() {
                    return Err(self.err("B0704", start, "a `server:` function needs a name", "write `server: async function save(ctx, data) { … }`"));
                }
                Ok(Stmt::Server { func, span: self.span_from(start) })
            }
            _ => {
                if !is_reserved(&w) && self.peek2()?.is_punct(":") {
                    return Err(self.err("B0108", start, "labels do not exist in BrainScript", "remove the label; if you meant an object, wrap it in parentheses: `({ a: 1 })`"));
                }
                self.expr_statement()
            }
        }
    }

    fn expr_statement(&mut self) -> R<Stmt> {
        let start = self.tok.span;
        let expr = self.expression()?;
        self.semicolon()?;
        Ok(Stmt::Expr { expr, span: self.span_from(start) })
    }

    fn var_kind(&mut self) -> R<VarKind> {
        let k = match &self.tok.tok {
            Tok::Ident(w) if w == "let" => VarKind::Let,
            Tok::Ident(w) if w == "const" => VarKind::Const,
            Tok::Ident(w) if w == "var" => VarKind::Var,
            _ => return Err(self.unexpected("`let` or `const`")),
        };
        self.bump()?;
        Ok(k)
    }

    fn var_decl(&mut self) -> R<VarDecl> {
        let start = self.tok.span;
        let kind = self.var_kind()?;
        let mut decls = Vec::new();
        loop {
            let ds = self.tok.span;
            let pat = self.binding_target()?;
            let init = if self.eat("=")? { Some(self.assignment()?) } else { None };
            decls.push(Declarator { pat, init, span: self.span_from(ds) });
            if !self.eat(",")? {
                break;
            }
        }
        Ok(VarDecl { kind, decls, span: self.span_from(start) })
    }

    fn for_statement(&mut self, start: Span) -> R<Stmt> {
        self.bump()?;
        if self.is_word("await") {
            return Err(self.err("B0110", self.tok.span, "`for await` does not exist in BrainScript", "use `for (const p of list) { const v = await p }`"));
        }
        self.expect("(")?;
        let classic = || {
            Diag::new("B0110", "", start, "only `for (const x of list)` exists in BrainScript", "rewrite the loop as `for (const x of list) { ... }`; for indices use `list.forEach((x, i) => ...)`")
        };
        if !(self.is_word("let") || self.is_word("const") || self.is_word("var")) {
            let mut d = classic();
            d.file = self.file().to_string();
            return Err(d);
        }
        let kind = self.var_kind()?;
        let pat = self.binding_target()?;
        if self.is_word("in") {
            return Err(self.err("B0109", self.tok.span, "`for in` does not exist in BrainScript", "use `for (const k of Object.keys(obj))` or `for (const [k, v] of Object.entries(obj))`"));
        }
        if !self.is_word("of") {
            let mut d = classic();
            d.file = self.file().to_string();
            return Err(d);
        }
        self.bump()?;
        let iter = self.assignment()?;
        self.expect(")")?;
        let body = Box::new(self.statement()?);
        Ok(Stmt::ForOf { kind, pat, iter, body, span: self.span_from(start) })
    }

    fn import(&mut self, start: Span) -> R<Stmt> {
        self.bump()?;
        let (mut default, mut namespace, mut named) = (None, None, Vec::new());
        if !matches!(self.tok.tok, Tok::Str { .. }) {
            if matches!(self.tok.tok, Tok::Ident(_)) {
                default = Some(self.ident()?);
                self.eat(",")?;
            }
            if self.eat("*")? {
                if !self.is_word("as") {
                    return Err(self.unexpected("`as`"));
                }
                self.bump()?;
                namespace = Some(self.ident()?);
            } else if self.eat("{")? {
                while !self.is("}") {
                    let imported = self.any_word()?;
                    let local = if self.is_word("as") {
                        self.bump()?;
                        self.ident()?
                    } else {
                        imported.clone()
                    };
                    named.push((imported, local));
                    if !self.eat(",")? {
                        break;
                    }
                }
                self.expect("}")?;
            }
            if !self.is_word("from") {
                return Err(self.unexpected("`from`"));
            }
            self.bump()?;
        }
        let Tok::Str { value, .. } = &self.tok.tok else {
            return Err(self.unexpected("the module path in quotes"));
        };
        let source = value.clone();
        self.bump()?;
        self.semicolon()?;
        Ok(Stmt::Import(Import { default, namespace, named, source, span: self.span_from(start) }))
    }

    fn function(&mut self, is_async: bool, start: Span) -> R<Func> {
        self.bump()?; // `function`
        if self.is("*") {
            return Err(self.err("B0106", self.tok.span, "generators (`function*`) do not exist in BrainScript", "return an array or use `async function` with `await`"));
        }
        let name = if matches!(self.tok.tok, Tok::Ident(_)) { Some(self.ident()?) } else { None };
        let params = self.params()?;
        let body = FnBody::Block(self.block_body()?);
        Ok(Func { name, params, body, is_arrow: false, is_async, span: self.span_from(start) })
    }

    fn params(&mut self) -> R<Vec<Pat>> {
        self.expect("(")?;
        let mut out = Vec::new();
        while !self.is(")") {
            if self.is("...") {
                let s = self.bump()?.span;
                let arg = self.binding_target()?;
                out.push(Pat::Rest { arg: Box::new(arg), span: self.span_from(s) });
                break;
            }
            out.push(self.binding_element()?);
            if !self.eat(",")? {
                break;
            }
        }
        self.expect(")")?;
        Ok(out)
    }

    // ───────────────────────── patterns ─────────────────────────

    /// Identifier, `{...}` or `[...]` (no default).
    pub fn binding_target(&mut self) -> R<Pat> {
        let start = self.tok.span;
        if self.eat("{")? {
            let mut props = Vec::new();
            let mut rest = None;
            while !self.is("}") {
                if self.eat("...")? {
                    rest = Some(Box::new(self.binding_target()?));
                    break;
                }
                let key = self.prop_key()?;
                let (value, shorthand) = if self.eat(":")? {
                    (self.binding_element()?, false)
                } else {
                    let PropKey::Ident(id) = &key else {
                        return Err(self.unexpected("`:`"));
                    };
                    if is_reserved(&id.name) {
                        return Err(self.err("B0002", id.span, &format!("`{}` is a reserved word", id.name), "rename it: `{ class: cls }`"));
                    }
                    let mut p = Pat::Ident(id.clone());
                    if self.eat("=")? {
                        let d = self.assignment()?;
                        p = Pat::Default { pat: Box::new(p), default: Box::new(d), span: self.span_from(id.span) };
                    }
                    (p, true)
                };
                props.push(PatProp { key, value, shorthand });
                if !self.eat(",")? {
                    break;
                }
            }
            self.expect("}")?;
            return Ok(Pat::Object { props, rest, span: self.span_from(start) });
        }
        if self.eat("[")? {
            let mut elems = Vec::new();
            let mut rest = None;
            while !self.is("]") {
                if self.is(",") {
                    self.bump()?;
                    elems.push(None);
                    continue;
                }
                if self.eat("...")? {
                    rest = Some(Box::new(self.binding_target()?));
                    break;
                }
                elems.push(Some(self.binding_element()?));
                if !self.eat(",")? {
                    break;
                }
            }
            self.expect("]")?;
            return Ok(Pat::Array { elems, rest, span: self.span_from(start) });
        }
        Ok(Pat::Ident(self.ident()?))
    }

    /// Target with an optional `= default`.
    fn binding_element(&mut self) -> R<Pat> {
        let start = self.tok.span;
        let p = self.binding_target()?;
        if self.eat("=")? {
            let d = self.assignment()?;
            return Ok(Pat::Default { pat: Box::new(p), default: Box::new(d), span: self.span_from(start) });
        }
        Ok(p)
    }

    // ───────────────────────── expressions ─────────────────────────

    pub fn expression(&mut self) -> R<Expr> {
        let start = self.tok.span;
        let first = self.assignment()?;
        if !self.is(",") {
            return Ok(first);
        }
        let mut exprs = vec![first];
        while self.eat(",")? {
            exprs.push(self.assignment()?);
        }
        Ok(Expr::Seq { exprs, span: self.span_from(start) })
    }

    /// Tries `(params) =>` / `ident =>` / `async ...`; restores the lexer on failure.
    fn try_arrow(&mut self) -> R<Option<Expr>> {
        let start = self.tok.span;
        let mut is_async = false;
        let saved = (self.lx.save(), self.tok.clone(), self.prev_end);
        if self.is_word("async") {
            let next = self.peek2()?;
            if next.nl_before || !(next.is_punct("(") || matches!(next.tok, Tok::Ident(_))) {
                return Ok(None);
            }
            self.bump()?;
            is_async = true;
        }
        let params = if matches!(self.tok.tok, Tok::Ident(_)) && self.peek2()?.is_punct("=>") {
            vec![Pat::Ident(self.ident()?)]
        } else if self.is("(") {
            match self.params() {
                Ok(p) if self.is("=>") && !self.tok.nl_before => p,
                _ => {
                    self.lx.restore(saved.0);
                    self.tok = saved.1;
                    self.prev_end = saved.2;
                    return Ok(None);
                }
            }
        } else {
            self.lx.restore(saved.0);
            self.tok = saved.1;
            self.prev_end = saved.2;
            return Ok(None);
        };
        self.expect("=>")?;
        let body = if self.is("{") { FnBody::Block(self.block_body()?) } else { FnBody::Expr(Box::new(self.assignment()?)) };
        Ok(Some(Expr::Func(Box::new(Func { name: None, params, body, is_arrow: true, is_async, span: self.span_from(start) }))))
    }

    pub fn assignment(&mut self) -> R<Expr> {
        if (self.is("(") || matches!(self.tok.tok, Tok::Ident(_)))
            && let Some(arrow) = self.try_arrow()?
        {
            return Ok(arrow);
        }
        let start = self.tok.span;
        let left = self.conditional()?;
        if let Tok::Punct(p) = self.tok.tok
            && ASSIGN_OPS.contains(&p)
        {
            if !matches!(left.unparen(), Expr::Ident(_) | Expr::Member { .. }) {
                return Err(self.err(
                    "B0002",
                    left.span(),
                    "you can only assign to a variable or a property",
                    "the left side of `=` must be a name (`n = 1`) or a property (`obj.x = 1`)",
                ));
            }
            self.bump()?;
            let value = self.assignment()?;
            return Ok(Expr::Assign { op: p, target: Box::new(left), value: Box::new(value), span: self.span_from(start) });
        }
        Ok(left)
    }

    fn conditional(&mut self) -> R<Expr> {
        let start = self.tok.span;
        let test = self.binary(0)?;
        if !self.eat("?")? {
            return Ok(test);
        }
        let cons = self.assignment()?;
        self.expect(":")?;
        let alt = self.assignment()?;
        Ok(Expr::Cond { test: Box::new(test), cons: Box::new(cons), alt: Box::new(alt), span: self.span_from(start) })
    }

    fn binary(&mut self, min: u8) -> R<Expr> {
        let start = self.tok.span;
        let mut left = self.unary()?;
        while let Some((op, prec)) = binary_prec(&self.tok) {
            if prec <= min {
                break;
            }
            self.bump()?;
            // `**` is right-associative.
            let right = if op == "**" { self.binary(prec - 1)? } else { self.binary(prec)? };
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), span: self.span_from(start) };
        }
        Ok(left)
    }

    fn unary(&mut self) -> R<Expr> {
        let start = self.tok.span;
        let op: Option<&'static str> = match &self.tok.tok {
            Tok::Punct(p) if matches!(*p, "!" | "-" | "+" | "~") => Some(p),
            Tok::Ident(w) if w == "typeof" => Some("typeof"),
            Tok::Ident(w) if w == "void" => Some("void"),
            Tok::Ident(w) if w == "delete" => Some("delete"),
            _ => None,
        };
        if let Some(op) = op {
            self.bump()?;
            let arg = self.unary()?;
            return Ok(Expr::Unary { op, arg: Box::new(arg), span: self.span_from(start) });
        }
        if self.is("++") || self.is("--") {
            let op = if self.is("++") { "++" } else { "--" };
            self.bump()?;
            let arg = self.unary()?;
            return Ok(Expr::Update { op, prefix: true, arg: Box::new(arg), span: self.span_from(start) });
        }
        if self.is_word("await") {
            self.bump()?;
            let arg = self.unary()?;
            return Ok(Expr::Await { arg: Box::new(arg), span: self.span_from(start) });
        }
        let e = self.call_member()?;
        if (self.is("++") || self.is("--")) && !self.tok.nl_before {
            let op = if self.is("++") { "++" } else { "--" };
            self.bump()?;
            return Ok(Expr::Update { op, prefix: false, arg: Box::new(e), span: self.span_from(start) });
        }
        Ok(e)
    }

    fn args(&mut self) -> R<Vec<Expr>> {
        self.expect("(")?;
        let mut out = Vec::new();
        while !self.is(")") {
            if self.is("...") {
                let s = self.bump()?.span;
                let arg = self.assignment()?;
                out.push(Expr::Spread { arg: Box::new(arg), span: self.span_from(s) });
            } else {
                out.push(self.assignment()?);
            }
            if !self.eat(",")? {
                break;
            }
        }
        self.expect(")")?;
        Ok(out)
    }

    fn call_member(&mut self) -> R<Expr> {
        let start = self.tok.span;
        let mut e = if self.is_word("new") {
            self.bump()?;
            if self.is(".") {
                return Err(self.err("B0002", start, "`new.target` does not exist in BrainScript", "remove it; components are not constructors"));
            }
            let callee = self.member_only()?;
            if let Expr::Ident(id) = &callee
                && id.name == "Function"
            {
                return Err(self.err("B0105", start, "`new Function` does not exist in BrainScript", "write the function directly: `const f = (a, b) => a + b`"));
            }
            let args = if self.is("(") { self.args()? } else { Vec::new() };
            Expr::New { callee: Box::new(callee), args, span: self.span_from(start) }
        } else {
            self.primary()?
        };
        loop {
            if self.eat(".")? {
                let name = self.any_word()?;
                e = Expr::Member { obj: Box::new(e), prop: MemberProp::Name(name), optional: false, span: self.span_from(start) };
            } else if self.is("?.") {
                self.bump()?;
                if self.is("(") {
                    let args = self.args()?;
                    e = Expr::Call { callee: Box::new(e), args, optional: true, span: self.span_from(start) };
                } else if self.eat("[")? {
                    let p = self.expression()?;
                    self.expect("]")?;
                    e = Expr::Member { obj: Box::new(e), prop: MemberProp::Computed(Box::new(p)), optional: true, span: self.span_from(start) };
                } else {
                    let name = self.any_word()?;
                    e = Expr::Member { obj: Box::new(e), prop: MemberProp::Name(name), optional: true, span: self.span_from(start) };
                }
            } else if self.eat("[")? {
                let p = self.expression()?;
                self.expect("]")?;
                e = Expr::Member { obj: Box::new(e), prop: MemberProp::Computed(Box::new(p)), optional: false, span: self.span_from(start) };
            } else if self.is("(") {
                let args = self.args()?;
                e = Expr::Call { callee: Box::new(e), args, optional: false, span: self.span_from(start) };
            } else if matches!(self.tok.tok, Tok::TplFull(_) | Tok::TplHead(_)) {
                let t = self.template(Some(Box::new(e)), start)?;
                e = t;
            } else {
                return Ok(e);
            }
        }
    }

    /// Callee of `new`: primary plus `.x` / `[x]`, without calls.
    fn member_only(&mut self) -> R<Expr> {
        let start = self.tok.span;
        let mut e = self.primary()?;
        loop {
            if self.eat(".")? {
                let name = self.any_word()?;
                e = Expr::Member { obj: Box::new(e), prop: MemberProp::Name(name), optional: false, span: self.span_from(start) };
            } else if self.eat("[")? {
                let p = self.expression()?;
                self.expect("]")?;
                e = Expr::Member { obj: Box::new(e), prop: MemberProp::Computed(Box::new(p)), optional: false, span: self.span_from(start) };
            } else {
                return Ok(e);
            }
        }
    }

    fn template(&mut self, tag: Option<Box<Expr>>, start: Span) -> R<Expr> {
        let first = self.bump()?;
        let mut quasis = Vec::new();
        let mut exprs = Vec::new();
        match first.tok {
            Tok::TplFull(raw) => quasis.push(raw),
            Tok::TplHead(raw) => {
                quasis.push(raw);
                loop {
                    exprs.push(self.expression()?);
                    match self.tok.tok.clone() {
                        Tok::TplMiddle(raw) => {
                            self.bump()?;
                            quasis.push(raw);
                        }
                        Tok::TplTail(raw) => {
                            self.bump()?;
                            quasis.push(raw);
                            break;
                        }
                        _ => return Err(self.unexpected("`}` to close `${`")),
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(Expr::Template { tag, quasis, exprs, span: self.span_from(start) })
    }

    fn prop_key(&mut self) -> R<PropKey> {
        match self.tok.tok.clone() {
            Tok::Ident(_) => Ok(PropKey::Ident(self.any_word()?)),
            Tok::Str { value, raw } => {
                let t = self.bump()?;
                Ok(PropKey::Str { value, raw, span: t.span })
            }
            Tok::Num(raw) => {
                let t = self.bump()?;
                Ok(PropKey::Num { raw, span: t.span })
            }
            Tok::Punct("[") => {
                self.bump()?;
                let e = self.assignment()?;
                self.expect("]")?;
                Ok(PropKey::Computed(Box::new(e)))
            }
            _ => Err(self.unexpected("a property name")),
        }
    }

    fn object(&mut self) -> R<Expr> {
        let start = self.tok.span;
        self.expect("{")?;
        let mut props = Vec::new();
        while !self.is("}") {
            let ps = self.tok.span;
            if self.eat("...")? {
                let arg = self.assignment()?;
                props.push(ObjProp::Spread { arg, span: self.span_from(ps) });
            } else if self.is("*") {
                return Err(self.err("B0106", self.tok.span, "generator methods do not exist in BrainScript", "return an array or use an `async` function"));
            } else {
                let mut is_async = false;
                if (self.is_word("get") || self.is_word("set") || self.is_word("async")) && {
                    let n = self.peek2()?;
                    !(n.is_punct(":") || n.is_punct("(") || n.is_punct(",") || n.is_punct("}"))
                } {
                    if self.is_word("async") {
                        self.bump()?;
                        is_async = true;
                    } else {
                        let w = if self.is_word("get") { "get" } else { "set" };
                        return Err(self.err(
                            "B0107",
                            self.tok.span,
                            &format!("{}ters do not exist in BrainScript", w),
                            "use a plain property or a `drv(() => ...)` for computed values",
                        ));
                    }
                }
                let key = self.prop_key()?;
                if self.eat(":")? {
                    let value = self.assignment()?;
                    props.push(ObjProp::KeyValue { key, value, span: self.span_from(ps) });
                } else if self.is("(") {
                    let params = self.params()?;
                    let body = FnBody::Block(self.block_body()?);
                    let func = Func { name: None, params, body, is_arrow: false, is_async, span: self.span_from(ps) };
                    props.push(ObjProp::Method { key, func, span: self.span_from(ps) });
                } else {
                    let PropKey::Ident(id) = key else {
                        return Err(self.unexpected("`:`"));
                    };
                    if is_reserved(&id.name) && id.name != "this" {
                        return Err(self.err("B0002", id.span, &format!("`{}` is a reserved word", id.name), &format!("write the full property: `{0}: value`", id.name)));
                    }
                    props.push(ObjProp::Shorthand(id));
                }
            }
            if !self.eat(",")? {
                break;
            }
        }
        self.expect("}")?;
        Ok(Expr::Object { props, span: self.span_from(start) })
    }

    fn primary(&mut self) -> R<Expr> {
        let start = self.tok.span;
        match self.tok.tok.clone() {
            Tok::Num(raw) => {
                self.bump()?;
                Ok(Expr::Num { raw, span: start })
            }
            Tok::Str { value, raw } => {
                self.bump()?;
                Ok(Expr::Str { value, raw, span: start })
            }
            Tok::Regex(raw) => {
                self.bump()?;
                Ok(Expr::Regex { raw, span: start })
            }
            Tok::TplFull(_) | Tok::TplHead(_) => self.template(None, start),
            Tok::Punct("(") => {
                self.bump()?;
                let e = self.expression()?;
                self.expect(")")?;
                Ok(Expr::Paren { expr: Box::new(e), span: self.span_from(start) })
            }
            Tok::Punct("[") => {
                self.bump()?;
                let mut elems = Vec::new();
                while !self.is("]") {
                    if self.is(",") {
                        self.bump()?;
                        elems.push(None);
                        continue;
                    }
                    let es = self.tok.span;
                    if self.eat("...")? {
                        let arg = self.assignment()?;
                        elems.push(Some(Expr::Spread { arg: Box::new(arg), span: self.span_from(es) }));
                    } else {
                        elems.push(Some(self.assignment()?));
                    }
                    if !self.eat(",")? {
                        break;
                    }
                }
                self.expect("]")?;
                Ok(Expr::Array { elems, span: self.span_from(start) })
            }
            Tok::Punct("{") => self.object(),
            Tok::Ident(w) => match w.as_str() {
                "true" | "false" => {
                    self.bump()?;
                    Ok(Expr::Bool { value: w == "true", span: start })
                }
                "null" => {
                    self.bump()?;
                    Ok(Expr::Null(start))
                }
                "this" => {
                    self.bump()?;
                    Ok(Expr::This(start))
                }
                "function" => Ok(Expr::Func(Box::new(self.function(false, start)?))),
                "async" if self.peek2()?.is_word("function") => {
                    self.bump()?;
                    Ok(Expr::Func(Box::new(self.function(true, start)?)))
                }
                "class" => Err(self.err("B0101", start, "`class` does not exist in BrainScript", "use functions and objects: `const make = (x) => ({ x })`")),
                "yield" => Err(self.err("B0106", start, "`yield` does not exist in BrainScript (there are no generators)", "return an array or use `await`")),
                "import" if self.peek2()?.is_punct(".") => {
                    // `import.meta` (only `.env` is meaningful: SPEC §11 env vars) — kept as one name.
                    self.bump()?;
                    self.bump()?;
                    if !self.is_word("meta") {
                        return Err(self.unexpected("`meta` after `import.`"));
                    }
                    self.bump()?;
                    Ok(Expr::Ident(Ident { name: "import.meta".into(), span: self.span_from(start) }))
                }
                "super" | "import" => Err(self.err("B0002", start, &format!("`{w}` cannot be used as an expression in BrainScript"), "use `import x from './a.js'` at the top of the <script>")),
                _ => Ok(Expr::Ident(self.ident()?)),
            },
            _ => Err(self.unexpected("an expression")),
        }
    }
}

/// Words that can never be a binding name.
pub fn is_reserved(w: &str) -> bool {
    matches!(
        w,
        "break" | "case" | "catch" | "class" | "const" | "continue" | "debugger" | "default" | "delete" | "do" | "else"
            | "export" | "extends" | "finally" | "for" | "function" | "if" | "import" | "in" | "instanceof" | "new"
            | "return" | "super" | "switch" | "this" | "throw" | "try" | "typeof" | "var" | "void" | "while" | "with"
            | "yield" | "let" | "await" | "enum" | "null" | "true" | "false"
    )
}
