//! INTERNALS §15/§18, rules inside one file:
//! - B0901 a signal cycle through effects: an `fx` that writes what it reads, or sig → fx → sig → fx loops.
//! - B0903 a component that renders itself outside any `{#if}` (no base case).
//! - B1101 `{#await}` on a `resource()` without `{:catch}`, or whose `{#each}` lacks `{:empty}`.
//! - B1105 (warning) a `{:catch}` on a resource that neither renders `<ErrorState>` nor reads the error's kind/status.
//! - B1001 (warning) a `drv` that only returns another signal.
//! - B1004 (warning) an `fx` that only computes one signal from others (it is a `drv`).
//! - B1005 (warning) a prop never used.
//! - B1009 (warning) a top-level function never used.

use crate::scope::{top_bindings, Kind};
use crate::Checker;
use brain_parse::ast::*;
use brain_parse::visit::Uses;
use brain_parse::Span;
use std::collections::{HashMap, HashSet};

/// Reads and writes of an effect body, ignoring nested functions (callbacks run later, untracked) and
/// anything inside `untrack(…)`.
#[derive(Default)]
struct Rw {
    reads: HashSet<String>,
    writes: HashSet<String>,
    pure: bool,
}

fn rw_expr(e: &Expr, rw: &mut Rw) {
    match e {
        Expr::Ident(i) => {
            rw.reads.insert(i.name.clone());
        }
        Expr::Func(_) => {}
        Expr::Call { callee, args, .. } => {
            rw.pure = false;
            if matches!(callee.unparen(), Expr::Ident(i) if i.name == "untrack") {
                return;
            }
            rw_expr(callee, rw);
            args.iter().for_each(|a| rw_expr(a, rw));
        }
        Expr::New { callee, args, .. } => {
            rw.pure = false;
            rw_expr(callee, rw);
            args.iter().for_each(|a| rw_expr(a, rw));
        }
        Expr::Assign { op, target, value, .. } => {
            rw.pure = false;
            if let Expr::Ident(t) = target.unparen() {
                rw.writes.insert(t.name.clone());
                if *op != "=" {
                    rw.reads.insert(t.name.clone());
                }
            } else {
                rw_expr(target, rw);
            }
            rw_expr(value, rw);
        }
        Expr::Update { arg, .. } => {
            rw.pure = false;
            if let Expr::Ident(t) = arg.unparen() {
                rw.writes.insert(t.name.clone());
                rw.reads.insert(t.name.clone());
            }
        }
        Expr::Await { arg, .. } => {
            rw.pure = false;
            rw_expr(arg, rw);
        }
        Expr::Template { tag, exprs, .. } => {
            if let Some(t) = tag {
                rw_expr(t, rw);
            }
            exprs.iter().for_each(|x| rw_expr(x, rw));
        }
        Expr::Array { elems, .. } => elems.iter().flatten().for_each(|x| rw_expr(x, rw)),
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    ObjProp::KeyValue { value, .. } => rw_expr(value, rw),
                    ObjProp::Spread { arg, .. } => rw_expr(arg, rw),
                    ObjProp::Shorthand(i) => {
                        rw.reads.insert(i.name.clone());
                    }
                    ObjProp::Method { .. } => {}
                }
            }
        }
        Expr::Unary { arg, .. } | Expr::Spread { arg, .. } | Expr::Paren { expr: arg, .. } => rw_expr(arg, rw),
        Expr::Binary { left, right, .. } => {
            rw_expr(left, rw);
            rw_expr(right, rw);
        }
        Expr::Cond { test, cons, alt, .. } => {
            rw_expr(test, rw);
            rw_expr(cons, rw);
            rw_expr(alt, rw);
        }
        Expr::Member { obj, prop, .. } => {
            rw_expr(obj, rw);
            if let MemberProp::Computed(p) = prop {
                rw_expr(p, rw);
            }
        }
        Expr::Seq { exprs, .. } => exprs.iter().for_each(|x| rw_expr(x, rw)),
        _ => {}
    }
}

fn rw_stmt(s: &Stmt, rw: &mut Rw) {
    match s {
        Stmt::Expr { expr, .. } => rw_expr(expr, rw),
        Stmt::Var(v) => {
            rw.pure = false;
            for d in &v.decls {
                if let Some(e) = &d.init {
                    rw_expr(e, rw);
                }
            }
        }
        Stmt::If { test, cons, alt, .. } => {
            rw.pure = false;
            rw_expr(test, rw);
            rw_stmt(cons, rw);
            if let Some(a) = alt {
                rw_stmt(a, rw);
            }
        }
        Stmt::ForOf { iter, body, .. } => {
            rw.pure = false;
            rw_expr(iter, rw);
            rw_stmt(body, rw);
        }
        Stmt::Block { body, .. } => body.iter().for_each(|x| rw_stmt(x, rw)),
        Stmt::Return { arg, .. } => {
            rw.pure = false;
            if let Some(e) = arg {
                rw_expr(e, rw);
            }
        }
        Stmt::Try { block, handler, finalizer, .. } => {
            rw.pure = false;
            for x in block.iter().chain(handler.iter().flatten()).chain(finalizer.iter().flatten()) {
                rw_stmt(x, rw);
            }
        }
        _ => rw.pure = false,
    }
}

fn rw_of(f: &Func) -> Rw {
    let mut rw = Rw { pure: true, ..Rw::default() };
    match &f.body {
        FnBody::Expr(e) => rw_expr(e, &mut rw),
        FnBody::Block(b) => b.iter().for_each(|s| rw_stmt(s, &mut rw)),
    }
    rw
}

/// Top-level `fx(() => …)` calls with their span.
fn effects(c: &Component) -> Vec<(&Func, Span)> {
    c.script
        .iter()
        .filter_map(|s| match s {
            Stmt::Expr { expr: Expr::Call { callee, args, span, .. }, .. } if matches!(callee.unparen(), Expr::Ident(i) if i.name == "fx") => match args.first().map(Expr::unparen) {
                Some(Expr::Func(f)) => Some((f.as_ref(), *span)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn has_self(nodes: &[Node], name: &str, guarded: bool, out: &mut Vec<Span>) {
    for n in nodes {
        match n {
            Node::ComponentRef(r) => {
                if r.name == name && !guarded {
                    out.push(r.span);
                }
                has_self(&r.children, name, guarded, out);
            }
            Node::Element(e) => has_self(&e.children, name, guarded, out),
            Node::If(b) => {
                for br in &b.branches {
                    has_self(&br.body, name, true, out);
                }
                if let Some(o) = &b.otherwise {
                    has_self(o, name, true, out);
                }
            }
            // An empty list ends the recursion too.
            Node::Each(b) => has_self(&b.body, name, true, out),
            Node::SlotScope(b) => has_self(&b.body, name, true, out),
            Node::Await(a) => {
                has_self(&a.pending, name, guarded, out);
                for br in [&a.then, &a.catch].into_iter().flatten() {
                    has_self(&br.body, name, guarded, out);
                }
            }
            _ => {}
        }
    }
}

fn each_without_empty(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Each(b) => b.empty.is_none() || each_without_empty(&b.body),
        Node::SlotScope(b) => each_without_empty(&b.body),
        Node::Element(e) => each_without_empty(&e.children),
        Node::ComponentRef(r) => each_without_empty(&r.children),
        Node::If(b) => b.branches.iter().any(|br| each_without_empty(&br.body)) || b.otherwise.as_deref().is_some_and(each_without_empty),
        _ => false,
    })
}

fn awaits<'a>(nodes: &'a [Node], out: &mut Vec<&'a AwaitBlock>) {
    for n in nodes {
        match n {
            Node::Await(a) => {
                out.push(a);
                awaits(&a.pending, out);
                for br in [&a.then, &a.catch].into_iter().flatten() {
                    awaits(&br.body, out);
                }
            }
            Node::Element(e) => awaits(&e.children, out),
            Node::ComponentRef(r) => awaits(&r.children, out),
            Node::If(b) => {
                for br in &b.branches {
                    awaits(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    awaits(o, out);
                }
            }
            Node::Each(b) => awaits(&b.body, out),
            Node::SlotScope(b) => awaits(&b.body, out),
            _ => {}
        }
    }
}

impl Checker<'_> {
    pub(crate) fn quality(&mut self) {
        let c = self.c;
        if c.kind == FileKind::Module {
            return;
        }
        let top = top_bindings(c);
        let cells: HashSet<String> = top.iter().filter(|(_, k)| matches!(k, Kind::Sig | Kind::Drv)).map(|(n, _)| n.clone()).collect();
        let sigs: HashSet<String> = top.iter().filter(|(_, k)| **k == Kind::Sig).map(|(n, _)| n.clone()).collect();

        // B0901: direct self-cycles, then cycles through several effects (drv nodes forward their reads).
        let fxs = effects(c);
        let rws: Vec<Rw> = fxs.iter().map(|(f, _)| rw_of(f)).collect();
        let mut drv_reads: HashMap<String, HashSet<String>> = HashMap::new();
        for s in &c.script {
            let Stmt::Var(v) = s else { continue };
            for d in &v.decls {
                if let (Pat::Ident(id), Some(Expr::Call { callee, args, .. })) = (&d.pat, d.init.as_ref().map(Expr::unparen))
                    && matches!(callee.unparen(), Expr::Ident(k) if k.name == "drv")
                    && let Some(Expr::Func(f)) = args.first().map(Expr::unparen)
                {
                    drv_reads.insert(id.name.clone(), rw_of(f).reads);
                }
            }
        }
        // Signals an effect depends on, through derived values.
        let deps = |reads: &HashSet<String>| -> HashSet<String> {
            let mut out = HashSet::new();
            let mut stack: Vec<String> = reads.iter().cloned().collect();
            while let Some(n) = stack.pop() {
                if !out.insert(n.clone()) {
                    continue;
                }
                if let Some(r) = drv_reads.get(&n) {
                    stack.extend(r.iter().cloned());
                }
            }
            out
        };
        let dep_sets: Vec<HashSet<String>> = rws.iter().map(|rw| deps(&rw.reads)).collect();
        let mut reported = HashSet::new();
        for (i, rw) in rws.iter().enumerate() {
            if let Some(x) = rw.writes.iter().filter(|w| sigs.contains(*w) && dep_sets[i].contains(*w)).min() {
                reported.insert(i);
                self.push("B0901", fxs[i].1, format!("`fx` writes `{x}`, which it also reads: direct cycle"), format!("if it counts runs read with `untrack(() => {x})`; if `{x}` derives from something else, make it a `drv`"));
            }
        }
        // fx_i → fx_j when fx_i writes a signal fx_j depends on; a cycle among effects loops forever.
        let n = fxs.len();
        for start in 0..n {
            if reported.contains(&start) {
                continue;
            }
            let mut stack = vec![(start, vec![start])];
            let mut found = None;
            let mut seen = HashSet::new();
            while let Some((cur, path)) = stack.pop() {
                for next in 0..n {
                    if rws[cur].writes.iter().any(|w| sigs.contains(w) && dep_sets[next].contains(w)) {
                        if next == start && path.len() > 1 {
                            found = Some(path.clone());
                            break;
                        }
                        if seen.insert(next) {
                            let mut p = path.clone();
                            p.push(next);
                            stack.push((next, p));
                        }
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            if let Some(path) = found {
                let names: Vec<String> = path.iter().map(|i| rws[*i].writes.iter().filter(|w| sigs.contains(*w)).cloned().collect::<Vec<_>>().join("/")).collect();
                for i in &path {
                    reported.insert(*i);
                }
                self.push("B0901", fxs[start].1, format!("effects write each other's inputs in a loop ({})", names.join(" → ")), "derive the values with `drv` instead of writing them from `fx`, or break the loop with `untrack(…)`");
            }
        }

        // B0903
        let mut selfs = Vec::new();
        has_self(&c.template, &c.name, false, &mut selfs);
        if let Some(s) = selfs.first() {
            self.push("B0903", *s, "the component renders itself with no base case", "wrap the call in `{#if depth < max}…{/if}`");
        }

        // B1101
        let resources: HashSet<String> = c
            .script
            .iter()
            .filter_map(|s| if let Stmt::Var(v) = s { Some(v) } else { None })
            .flat_map(|v| v.decls.iter())
            .filter_map(|d| match (&d.pat, d.init.as_ref().map(Expr::unparen)) {
                (Pat::Ident(id), Some(Expr::Call { callee, .. })) if matches!(callee.unparen(), Expr::Ident(k) if k.name == "resource") => Some(id.name.clone()),
                _ => None,
            })
            .collect();
        let mut aw = Vec::new();
        awaits(&c.template, &mut aw);
        let uses_error_state = crate::outline::outline(c).children.iter().any(|t| t == "ErrorState");
        for a in aw {
            // `{#await posts}`, but also `{#await posts ?? null}`, `{#await cond ? a : b}` (QA #176): the first resource
            // the expression reads.
            let mut found: Option<Ident> = None;
            crate::each_expr(&a.promise, &mut |e| {
                if let Expr::Ident(i) = e
                    && found.is_none()
                    && resources.contains(&i.name)
                {
                    found = Some(i.clone());
                }
            });
            let Some(id) = found else { continue };
            let id = &id;
            let no_empty = a.then.as_ref().is_some_and(|t| each_without_empty(&t.body));
            // Name the piece that is missing: the agent fixes exactly that and does not touch the rest (QA #277).
            let add_catch = format!("add `{{:catch e}}<ErrorState error={{e}} retry={{{}.refetch}} />`", id.name);
            let missing = match (a.catch.is_none(), no_empty) {
                (true, true) => Some(("`{:catch}` and an empty state (`{:empty}` in its `{#each}`)", format!("{add_catch} and `{{:empty}}` in the `{{#each}}`"))),
                (true, false) => Some(("`{:catch}`", add_catch)),
                (false, true) => Some(("an empty state (`{:empty}` in its `{#each}`)", "add `{:empty}<p>…</p>` at the end of the `{#each}` inside `{:then}`".to_string())),
                (false, false) => None,
            };
            if let Some((what, fix)) = missing {
                self.push("B1101", a.span, format!("`{{#await {}}}` on a resource without {what}", id.name), fix);
            }
            // B1105 (§40.5): a `{:catch}` that shows the same thing for every failure: 401 and 403 need a sign-in link
            // and the missing permission, which `<ErrorState>` renders. A file that uses `<ErrorState>` is handled.
            if let Some(br) = &a.catch
                && !uses_error_state
            {
                let bind = match &br.binding {
                    Some(Pat::Ident(b)) => Some(b.name.clone()),
                    _ => None,
                };
                let mut text = String::new();
                crate::node_exprs(&br.body, &mut text);
                let reads_kind = bind.as_ref().is_some_and(|b| [".status", ".kind", ".is("].iter().any(|k| text.contains(&format!("{b}{k}"))));
                if !reads_kind {
                    let el = format!("<ErrorState error={{{}}} retry={{{}.refetch}} />", bind.as_deref().unwrap_or("e"), id.name);
                    let msg = format!("`{{:catch}}` of `{}` shows one message for every failure (offline, 401, 403, 5xx…)", id.name);
                    match (&bind, br.body.first()) {
                        (Some(_), Some(first)) => {
                            let at = first.span().start;
                            let end = br.body.last().map_or(at, |l| l.span().end);
                            self.push_edits("B1105", br.span, msg, format!("render `{el}` (it names the missing permission and links to sign in)"), vec![(at, end, el)]);
                        }
                        _ => self.push("B1105", br.span, msg, format!("write `{{:catch e}}{el}`")),
                    }
                }
            }
        }

        // B1001
        for s in &c.script {
            let Stmt::Var(v) = s else { continue };
            for d in &v.decls {
                if let (Pat::Ident(id), Some(call @ Expr::Call { callee, args, .. })) = (&d.pat, d.init.as_ref().map(Expr::unparen))
                    && matches!(callee.unparen(), Expr::Ident(k) if k.name == "drv")
                    && let Some(Expr::Func(f)) = args.first().map(Expr::unparen)
                {
                    let only = match &f.body {
                        FnBody::Expr(e) => Some(e.as_ref()),
                        FnBody::Block(b) if b.len() == 1 => match &b[0] {
                            Stmt::Return { arg: Some(e), .. } => Some(e),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(Expr::Ident(src)) = only.map(Expr::unparen)
                        && (cells.contains(&src.name) || top.get(&src.name) == Some(&Kind::Prop))
                    {
                        self.push("B1001", call.span(), format!("`drv` that only returns the signal `{}`", src.name), format!("use `{}` directly and delete `{}`", src.name, id.name));
                    }
                }
            }
        }

        // B1004
        for (i, (f, span)) in fxs.iter().enumerate() {
            let rw = &rws[i];
            let body: Vec<&Stmt> = match &f.body {
                FnBody::Block(b) => b.iter().collect(),
                FnBody::Expr(_) => Vec::new(),
            };
            if reported.contains(&i) || body.len() != 1 {
                continue;
            }
            if let Stmt::Expr { expr: Expr::Assign { op: "=", target, value, .. }, .. } = body[0]
                && let Expr::Ident(t) = target.unparen()
                && sigs.contains(&t.name)
            {
                let mut v = Rw { pure: true, ..Rw::default() };
                rw_expr(value, &mut v);
                if v.pure && !v.reads.is_empty() && v.reads.iter().all(|r| cells.contains(r) || top.get(r) == Some(&Kind::Prop) || top.get(r) == Some(&Kind::Const)) && rw.writes.len() == 1 {
                    let src = brain_parse::dump::expr(value);
                    let _ = f;
                    self.push("B1004", *span, format!("`fx` that only computes `{}` from other signals", t.name), format!("`let {} = drv(() => {src})`", t.name));
                }
            }
        }

        // B1005 / B1009: names never read anywhere in the file.
        let mut uses = Uses::component(c);
        for m in &c.meta {
            if let Some(x) = &m.expr {
                uses.expr(x);
            }
        }
        uses.nodes(&c.head);
        let used = |n: &str| uses.reads.contains(n) || uses.calls.contains(n) || uses.writes.contains(n);
        let mentioned: HashSet<String> = c.meta.iter().filter(|m| matches!(m.name.as_str(), "expose" | "tools")).flat_map(|m| m.value.clone()).flat_map(|v| v.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')).map(str::to_string).collect::<Vec<_>>()).collect();
        for p in &c.props {
            if !used(&p.name) && !mentioned.contains(&p.name) {
                self.push("B1005", p.span, format!("prop `{}` is never used", p.name), "remove it from `<meta props>` or render it");
            }
        }
        // A file with `<meta tools>` reports a misnamed tool function as B1701, not as dead code.
        let has_tools = c.meta.iter().any(|m| m.name == "tools");
        for s in c.script.iter().filter(|_| !has_tools) {
            let (name, span) = match s {
                Stmt::Func(Func { name: Some(n), .. }) => (n.name.clone(), n.span),
                Stmt::Var(v) => match v.decls.as_slice() {
                    [d] if matches!(d.init.as_ref().map(Expr::unparen), Some(Expr::Func(_))) => match &d.pat {
                        Pat::Ident(i) => (i.name.clone(), i.span),
                        _ => continue,
                    },
                    _ => continue,
                },
                _ => continue,
            };
            if !used(&name) && !mentioned.contains(&name) {
                self.push("B1009", span, format!("`{name}` is never used"), "delete it or wire it to a control");
            }
        }

        // B1010: a component over the size limit; the fix is the exact `brain split` that extracts its biggest block.
        let (n_lines, bytes) = crate::contract::size(c);
        if n_lines > crate::split::MAX_LINES || bytes > crate::split::MAX_BYTES {
            let span = c.template.first().map_or(c.span, |n| n.span());
            let what = if n_lines > crate::split::MAX_LINES { format!("{n_lines} lines of markup and script (limit {})", crate::split::MAX_LINES) } else { format!("{} KB (limit {} KB)", bytes / 1024, crate::split::MAX_BYTES / 1024) };
            let fix = match crate::split::suggest(c) {
                Some(crate::split::Suggestion::Split { selector, name, line }) => format!("extract the block at line {line}: `brain split {} \"{selector}\" {name}`", c.file),
                Some(crate::split::Suggestion::Repeated { line, count }) => format!("the block at line {line} repeats {count} times: make it a component and render it with `{{#each}}` or once per use"),
                None => "split the markup into components under comp/ (`brain split <file> \"<selector>\" <Name>`)".to_string(),
            };
            self.push("B1010", span, format!("`{}` is too large: {what}", c.name), fix);
        }
    }
}
