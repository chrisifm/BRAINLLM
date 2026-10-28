//! Compact, stable text dump of the AST for snapshot tests and `brain ast` debugging.
//! Each node prints as `Kind@line:col` plus its essentials; expressions print as JS-ish source.

use crate::ast::*;
use std::fmt::Write;

pub fn dump(c: &Component) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "{:?} {} level={:?} file={}", c.kind, c.name, c.level, c.file);
    for p in &c.props {
        let d = p.default.as_ref().map(|e| format!(" = {}", expr(e))).unwrap_or_default();
        let _ = writeln!(o, "  prop@{} {}:{}{}{}", at(p.span), p.name, p.ty.as_str(), format!("{}{}", if p.bindable { "~" } else { "" }, if p.optional { "?" } else { "" }), d);
    }
    for m in &c.meta {
        match &m.expr {
            Some(x) => {
                let _ = writeln!(o, "  meta@{} {}={{{}}}", at(m.span), m.name, expr(x));
            }
            None => {
                let _ = writeln!(o, "  meta@{} {}={:?}", at(m.span), m.name, m.value);
            }
        }
    }
    if !c.script.is_empty() {
        o.push_str("  script\n");
        for s in &c.script {
            stmt(s, 2, &mut o);
        }
    }
    o.push_str("  template\n");
    for n in &c.template {
        node(n, 2, &mut o);
    }
    if !c.head.is_empty() {
        o.push_str("  head\n");
        for n in &c.head {
            node(n, 2, &mut o);
        }
    }
    if let Some(s) = &c.style {
        let mut css = String::new();
        brain_css::print_items(&s.items, &mut css, 0);
        let _ = writeln!(o, "  style@{}", at(s.span));
        for l in css.lines() {
            let _ = writeln!(o, "    {l}");
        }
    }
    o
}

fn at(s: Span) -> String {
    format!("{}:{}", s.line, s.col)
}

fn pad(n: usize) -> String {
    "  ".repeat(n)
}

fn node(n: &Node, d: usize, o: &mut String) {
    let p = pad(d);
    match n {
        Node::Text(t) => {
            if t.value.trim().is_empty() {
                return;
            }
            let _ = writeln!(o, "{p}text@{} {:?}", at(t.span), t.value.trim());
        }
        Node::Interp(i) => {
            let _ = writeln!(o, "{p}interp@{} {{{}{}}}", at(i.span), if i.html { "@html " } else { "" }, expr(&i.expr));
        }
        Node::Element(e) => {
            let _ = writeln!(o, "{p}<{}>@{}{}", e.tag, at(e.span), attrs(&e.attrs));
            for c in &e.children {
                node(c, d + 1, o);
            }
        }
        Node::ComponentRef(e) => {
            let _ = writeln!(o, "{p}component {}@{}{}", e.name, at(e.span), attrs(&e.attrs));
            for c in &e.children {
                node(c, d + 1, o);
            }
        }
        Node::Slot(s) => {
            let _ = writeln!(o, "{p}slot@{}{}", at(s.span), s.name.as_ref().map(|n| format!(" name={n}")).unwrap_or_default());
            for a in &s.attrs {
                let _ = writeln!(o, "{p}  {}={}", a.name, match &a.value {
                    AttrValue::Expr(x) => expr(x),
                    other => format!("{other:?}"),
                });
            }
        }
        Node::SlotScope(b) => {
            let _ = writeln!(o, "{p}slot-scope@{} {}", at(b.span), b.names.iter().map(|n| n.name.as_str()).collect::<Vec<_>>().join(", "));
            for c in &b.body {
                node(c, d + 1, o);
            }
        }
        Node::If(b) => {
            for (i, br) in b.branches.iter().enumerate() {
                let kw = if i == 0 { "if" } else { "else if" };
                let _ = writeln!(o, "{p}{kw}@{} {}", at(br.span), expr(&br.test));
                for c in &br.body {
                    node(c, d + 1, o);
                }
            }
            if let Some(e) = &b.otherwise {
                let _ = writeln!(o, "{p}else");
                for c in e {
                    node(c, d + 1, o);
                }
            }
        }
        Node::Each(b) => {
            let idx = b.index.as_ref().map(|i| format!(", {}", i.name)).unwrap_or_default();
            let key = b.key.as_ref().map(|k| format!(" ({})", expr(k))).unwrap_or_else(|| " (NO KEY)".into());
            let _ = writeln!(o, "{p}each@{} {} as {}{}{}", at(b.span), expr(&b.list), pat(&b.item), idx, key);
            for c in &b.body {
                node(c, d + 1, o);
            }
            if let Some(e) = &b.empty {
                let _ = writeln!(o, "{p}empty");
                for c in e {
                    node(c, d + 1, o);
                }
            }
        }
        Node::Await(b) => {
            let _ = writeln!(o, "{p}await@{} {}", at(b.span), expr(&b.promise));
            for c in &b.pending {
                node(c, d + 1, o);
            }
            for (kw, br) in [("then", &b.then), ("catch", &b.catch)] {
                if let Some(br) = br {
                    let bind = br.binding.as_ref().map(|b| format!(" {}", pat(b))).unwrap_or_default();
                    let _ = writeln!(o, "{p}{kw}{bind}");
                    for c in &br.body {
                        node(c, d + 1, o);
                    }
                }
            }
        }
    }
}

fn attrs(a: &[Attr]) -> String {
    let mut s = String::new();
    for at in a {
        let prefix = match at.kind {
            AttrKind::Plain => "",
            AttrKind::On => "on:",
            AttrKind::Bind => "bind:",
            AttrKind::Class => "class:",
            AttrKind::Style => "style:",
            AttrKind::Transition => "transition:",
        };
        s.push(' ');
        s.push_str(prefix);
        s.push_str(&at.name);
        match &at.value {
            AttrValue::None => {}
            AttrValue::Text(t) => {
                let _ = write!(s, "={t:?}");
            }
            AttrValue::Expr(e) => {
                let _ = write!(s, "={{{}}}", expr(e));
            }
            AttrValue::Mixed(parts) => {
                s.push_str("=\"");
                for p in parts {
                    match p {
                        AttrPart::Text(t) => s.push_str(t),
                        AttrPart::Expr(e) => {
                            let _ = write!(s, "{{{}}}", expr(e));
                        }
                    }
                }
                s.push('"');
            }
        }
    }
    s
}

pub fn stmt(s: &Stmt, d: usize, o: &mut String) {
    let p = pad(d);
    match s {
        Stmt::Var(v) => {
            let _ = writeln!(o, "{p}{}@{}", var(v), at(v.span));
            for dcl in &v.decls {
                if let Some(i) = &dcl.init {
                    nested(i, d + 1, o);
                }
            }
        }
        Stmt::Expr { expr: e, span } => {
            let _ = writeln!(o, "{p}expr@{} {}", at(*span), expr(e));
            nested(e, d + 1, o);
        }
        Stmt::If { test, cons, alt, span } => {
            let _ = writeln!(o, "{p}if@{} {}", at(*span), expr(test));
            stmt(cons, d + 1, o);
            if let Some(a) = alt {
                let _ = writeln!(o, "{p}else");
                stmt(a, d + 1, o);
            }
        }
        Stmt::ForOf { kind, pat: pt, iter, body, span } => {
            let _ = writeln!(o, "{p}for@{} {} {} of {}", at(*span), kind_str(*kind), pat(pt), expr(iter));
            stmt(body, d + 1, o);
        }
        Stmt::Return { arg, span } => {
            let _ = writeln!(o, "{p}return@{} {}", at(*span), arg.as_ref().map(expr).unwrap_or_default());
            if let Some(a) = arg {
                nested(a, d + 1, o);
            }
        }
        Stmt::Block { body, span } => {
            let _ = writeln!(o, "{p}block@{}", at(*span));
            for s in body {
                stmt(s, d + 1, o);
            }
        }
        Stmt::Func(f) => {
            let _ = writeln!(o, "{p}{}@{}", func_head(f), at(f.span));
            func_body(f, d + 1, o);
        }
        Stmt::Server { func: f, span } => {
            let _ = writeln!(o, "{p}server {}@{}", func_head(f), at(*span));
            func_body(f, d + 1, o);
        }
        Stmt::Try { block, param, handler, finalizer, span } => {
            let _ = writeln!(o, "{p}try@{}", at(*span));
            for s in block {
                stmt(s, d + 1, o);
            }
            if let Some(h) = handler {
                let _ = writeln!(o, "{p}catch {}", param.as_ref().map(pat).unwrap_or_default());
                for s in h {
                    stmt(s, d + 1, o);
                }
            }
            if let Some(f) = finalizer {
                let _ = writeln!(o, "{p}finally");
                for s in f {
                    stmt(s, d + 1, o);
                }
            }
        }
        Stmt::Throw { arg, span } => {
            let _ = writeln!(o, "{p}throw@{} {}", at(*span), expr(arg));
        }
        Stmt::Break { span } => {
            let _ = writeln!(o, "{p}break@{}", at(*span));
        }
        Stmt::Continue { span } => {
            let _ = writeln!(o, "{p}continue@{}", at(*span));
        }
        Stmt::Import(i) => {
            let mut what = Vec::new();
            if let Some(d) = &i.default {
                what.push(d.name.clone());
            }
            if let Some(n) = &i.namespace {
                what.push(format!("* as {}", n.name));
            }
            if !i.named.is_empty() {
                let n: Vec<String> = i.named.iter().map(|(a, b)| if a.name == b.name { a.name.clone() } else { format!("{} as {}", a.name, b.name) }).collect();
                what.push(format!("{{ {} }}", n.join(", ")));
            }
            let _ = writeln!(o, "{p}import@{} {} from {:?}", at(i.span), what.join(", "), i.source);
        }
        Stmt::Export { decl, span } => {
            let _ = writeln!(o, "{p}export@{}", at(*span));
            stmt(decl, d + 1, o);
        }
        Stmt::ExportDefault { expr: e, span } => {
            let _ = writeln!(o, "{p}export default@{} {}", at(*span), expr(e));
            nested(e, d + 1, o);
        }
        Stmt::Empty { .. } => {}
    }
}

/// Dumps the bodies of block-bodied functions found inside an expression.
fn nested(e: &Expr, d: usize, o: &mut String) {
    let p = pad(d);
    match e {
        Expr::Func(f) => {
            if let FnBody::Block(b) = &f.body {
                let _ = writeln!(o, "{p}fn@{}", at(f.span));
                for s in b {
                    stmt(s, d + 1, o);
                }
            } else if let FnBody::Expr(x) = &f.body {
                nested(x, d, o);
            }
        }
        Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
            nested(callee, d, o);
            for a in args {
                nested(a, d, o);
            }
        }
        Expr::Array { elems, .. } => elems.iter().flatten().for_each(|x| nested(x, d, o)),
        Expr::Object { props, .. } => {
            for pr in props {
                match pr {
                    ObjProp::KeyValue { value, .. } => nested(value, d, o),
                    ObjProp::Method { func, .. } => nested(&Expr::Func(Box::new(func.clone())), d, o),
                    ObjProp::Spread { arg, .. } => nested(arg, d, o),
                    ObjProp::Shorthand(_) => {}
                }
            }
        }
        Expr::Unary { arg, .. } | Expr::Update { arg, .. } | Expr::Spread { arg, .. } | Expr::Await { arg, .. } => nested(arg, d, o),
        Expr::Paren { expr: x, .. } => nested(x, d, o),
        Expr::Binary { left, right, .. } => {
            nested(left, d, o);
            nested(right, d, o);
        }
        Expr::Assign { target, value, .. } => {
            nested(target, d, o);
            nested(value, d, o);
        }
        Expr::Cond { test, cons, alt, .. } => {
            nested(test, d, o);
            nested(cons, d, o);
            nested(alt, d, o);
        }
        Expr::Member { obj, prop, .. } => {
            nested(obj, d, o);
            if let MemberProp::Computed(x) = prop {
                nested(x, d, o);
            }
        }
        Expr::Seq { exprs, .. } => exprs.iter().for_each(|x| nested(x, d, o)),
        Expr::Template { tag, exprs, .. } => {
            if let Some(t) = tag {
                nested(t, d, o);
            }
            exprs.iter().for_each(|x| nested(x, d, o));
        }
        _ => {}
    }
}

fn kind_str(k: VarKind) -> &'static str {
    match k {
        VarKind::Let => "let",
        VarKind::Const => "const",
        VarKind::Var => "var",
    }
}

fn var(v: &VarDecl) -> String {
    let ds: Vec<String> = v
        .decls
        .iter()
        .map(|d| match &d.init {
            Some(i) => format!("{} = {}", pat(&d.pat), expr(i)),
            None => pat(&d.pat),
        })
        .collect();
    format!("{} {}", kind_str(v.kind), ds.join(", "))
}

fn func_head(f: &Func) -> String {
    let params: Vec<String> = f.params.iter().map(pat).collect();
    format!(
        "{}function {}({})",
        if f.is_async { "async " } else { "" },
        f.name.as_ref().map(|n| n.name.as_str()).unwrap_or(""),
        params.join(", ")
    )
}

fn func_body(f: &Func, d: usize, o: &mut String) {
    match &f.body {
        FnBody::Expr(e) => {
            let _ = writeln!(o, "{}=> {}", pad(d), expr(e));
        }
        FnBody::Block(b) => {
            for s in b {
                stmt(s, d, o);
            }
        }
    }
}

pub fn pat(p: &Pat) -> String {
    match p {
        Pat::Ident(i) => i.name.clone(),
        Pat::Object { props, rest, .. } => {
            let mut v: Vec<String> = props
                .iter()
                .map(|pp| if pp.shorthand { pat(&pp.value) } else { format!("{}: {}", key(&pp.key), pat(&pp.value)) })
                .collect();
            if let Some(r) = rest {
                v.push(format!("...{}", pat(r)));
            }
            format!("{{ {} }}", v.join(", "))
        }
        Pat::Array { elems, rest, .. } => {
            let mut v: Vec<String> = elems.iter().map(|e| e.as_ref().map(pat).unwrap_or_default()).collect();
            if let Some(r) = rest {
                v.push(format!("...{}", pat(r)));
            }
            format!("[{}]", v.join(", "))
        }
        Pat::Default { pat: p, default, .. } => format!("{} = {}", pat(p), expr(default)),
        Pat::Rest { arg, .. } => format!("...{}", pat(arg)),
    }
}

fn key(k: &PropKey) -> String {
    match k {
        PropKey::Ident(i) => i.name.clone(),
        PropKey::Str { raw, .. } => raw.clone(),
        PropKey::Num { raw, .. } => raw.clone(),
        PropKey::Computed(e) => format!("[{}]", expr(e)),
    }
}

/// Fully parenthesized JS-ish rendering (shows the parse tree's grouping).
pub fn expr(e: &Expr) -> String {
    match e {
        Expr::Ident(i) => i.name.clone(),
        Expr::This(_) => "this".into(),
        Expr::Num { raw, .. } | Expr::Regex { raw, .. } | Expr::Str { raw, .. } => raw.clone(),
        Expr::Bool { value, .. } => value.to_string(),
        Expr::Null(_) => "null".into(),
        Expr::Template { tag, quasis, exprs, .. } => {
            let mut s = tag.as_ref().map(|t| expr(t)).unwrap_or_default();
            s.push('`');
            for (i, q) in quasis.iter().enumerate() {
                s.push_str(q);
                if let Some(x) = exprs.get(i) {
                    s.push_str("${");
                    s.push_str(&expr(x));
                    s.push('}');
                }
            }
            s.push('`');
            s
        }
        Expr::Array { elems, .. } => format!("[{}]", elems.iter().map(|e| e.as_ref().map(expr).unwrap_or_default()).collect::<Vec<_>>().join(", ")),
        Expr::Object { props, .. } => {
            let v: Vec<String> = props
                .iter()
                .map(|p| match p {
                    ObjProp::KeyValue { key: k, value, .. } => format!("{}: {}", key(k), expr(value)),
                    ObjProp::Shorthand(i) => i.name.clone(),
                    ObjProp::Method { key: k, func, .. } => format!("{}{}", key(k), &func_head(func)["function ".len()..]),
                    ObjProp::Spread { arg, .. } => format!("...{}", expr(arg)),
                })
                .collect();
            format!("{{ {} }}", v.join(", "))
        }
        Expr::Func(f) => {
            if f.is_arrow {
                let params: Vec<String> = f.params.iter().map(pat).collect();
                let body = match &f.body {
                    FnBody::Expr(e) => expr(e),
                    FnBody::Block(b) => format!("{{ {} stmts }}", b.len()),
                };
                format!("{}({}) => {}", if f.is_async { "async " } else { "" }, params.join(", "), body)
            } else {
                format!("{} {{…}}", func_head(f))
            }
        }
        Expr::Unary { op, arg, .. } => {
            let sp = if op.chars().all(|c| c.is_alphabetic()) { " " } else { "" };
            format!("({op}{sp}{})", expr(arg))
        }
        Expr::Update { op, prefix, arg, .. } => {
            if *prefix { format!("({op}{})", expr(arg)) } else { format!("({}{op})", expr(arg)) }
        }
        Expr::Binary { op, left, right, .. } => format!("({} {op} {})", expr(left), expr(right)),
        Expr::Assign { op, target, value, .. } => format!("({} {op} {})", expr(target), expr(value)),
        Expr::Cond { test, cons, alt, .. } => format!("({} ? {} : {})", expr(test), expr(cons), expr(alt)),
        Expr::Call { callee, args, optional, .. } => {
            format!("{}{}({})", expr(callee), if *optional { "?." } else { "" }, args.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
        Expr::New { callee, args, .. } => format!("new {}({})", expr(callee), args.iter().map(expr).collect::<Vec<_>>().join(", ")),
        Expr::Member { obj, prop, optional, .. } => match prop {
            MemberProp::Name(n) => format!("{}{}{}", expr(obj), if *optional { "?." } else { "." }, n.name),
            MemberProp::Computed(p) => format!("{}{}[{}]", expr(obj), if *optional { "?." } else { "" }, expr(p)),
        },
        Expr::Seq { exprs, .. } => format!("({})", exprs.iter().map(expr).collect::<Vec<_>>().join(", ")),
        Expr::Spread { arg, .. } => format!("...{}", expr(arg)),
        Expr::Await { arg, .. } => format!("(await {})", expr(arg)),
        Expr::Paren { expr: e, .. } => expr(e),
    }
}
