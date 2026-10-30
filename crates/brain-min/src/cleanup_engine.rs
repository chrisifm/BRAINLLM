//! Cleanup uses the existing scope parser and binding/reference marks, not a second JS parser.
use super::*;
use crate::cleanup::{Action, CleanupError, CleanupReport, Kind as CleanupKind};

struct Model {
    scopes: Vec<Scope>,
    marks: Marks,
    top: Vec<(usize, usize)>,
    uses: HashMap<usize, HashMap<String, Vec<usize>>>,
    counts: HashMap<usize, HashMap<String, usize>>,
    resolved: HashMap<usize, Option<usize>>,
    references: Vec<usize>,
    function_ranges: Vec<(usize, usize)>,
}
fn failure(source: Option<&str>, toks: &[Tok], at: usize) -> CleanupError {
    CleanupError::new(
        "CLEANUP_UNSUPPORTED_STRUCTURE",
        "scope-analysis",
        source,
        toks.get(at).map(|t| t.start),
        "The in-house scope model cannot safely analyze this structure",
        "Run brain check; simplify the named input location without weakening checks",
    )
}
fn model(toks: &[Tok], source: Option<&str>) -> Result<Model, CleanupError> {
    if toks.len() > 200_000 {
        return Err(CleanupError::new(
            "CLEANUP_TOKEN_LIMIT",
            "scope-analysis",
            source,
            None,
            "Module exceeds 200000 tokens",
            "Split the module before production cleanup",
        ));
    }
    let (mut depth, mut arrows) = (0usize, 0usize);
    for (i, t) in toks.iter().enumerate() {
        if matches!(t.kind, Kind::Punct) && matches!(t.text.as_str(), "(" | "[" | "{") {
            depth += 1;
        }
        if matches!(t.kind, Kind::Punct) && matches!(t.text.as_str(), ")" | "]" | "}") {
            depth = depth.saturating_sub(1);
        }
        if t.punct("=>") {
            arrows += 1;
        }
        if t.punct(";") {
            arrows = 0;
        }
        if depth > 256 || arrows > 256 {
            return Err(CleanupError::new(
                "CLEANUP_DEPTH_LIMIT",
                "scope-analysis",
                source,
                Some(toks[i].start),
                "Module structure exceeds the bounded analysis depth",
                "Split nested expressions or generated statements",
            ));
        }
    }
    let mut p = P { t: toks, i: 0, scopes: vec![Scope { parent: None, start: 0, end: toks.len(), is_fn: true, decls: vec![] }], cur: 0, m: Marks::default() };
    let mut top = vec![];
    while p.i < toks.len() {
        let start = p.i;
        p.m.statements.insert(start);
        if p.statement().is_err() || p.i == start {
            return Err(failure(source, toks, p.i));
        }
        top.push((start, p.i));
    }
    if p.scopes.len() > 8192 {
        return Err(CleanupError::new(
            "CLEANUP_SCOPE_LIMIT",
            "scope-analysis",
            source,
            None,
            "Module exceeds 8192 scopes",
            "Split the module before production cleanup",
        ));
    }
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    let mut counts: HashMap<usize, HashMap<String, usize>> = HashMap::new();
    for (si, scope) in p.scopes.iter().enumerate() {
        for (name, _) in &scope.decls {
            *counts.entry(si).or_default().entry(name.clone()).or_default() += 1;
            let owners = by_name.entry(name.clone()).or_default();
            if owners.last() != Some(&si) {
                owners.push(si);
            }
        }
    }
    let mut uses: HashMap<usize, HashMap<String, Vec<usize>>> = HashMap::new();
    let mut resolved = HashMap::new();
    for &at in &p.m.refs {
        let name = &toks[at].text;
        let owner = by_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&s| p.scopes[s].start <= at && at < p.scopes[s].end.max(p.scopes[s].start + 1))
            .max_by_key(|&s| (p.scopes[s].start, s));
        if let Some(scope) = owner {
            uses.entry(scope).or_default().entry(name.clone()).or_default().push(at);
        }
        resolved.insert(at, owner);
    }
    let mut references = p.m.refs.clone();
    references.sort_unstable();
    references.dedup();
    let mut function_ranges = p.scopes.iter().skip(1).filter(|s| s.is_fn).map(|s| (s.start, s.end)).collect::<Vec<_>>();
    function_ranges.sort_unstable();
    Ok(Model { scopes: p.scopes, marks: p.m, top, uses, counts, resolved, references, function_ranges })
}
fn resolve(m: &Model, _name: &str, at: usize) -> Option<usize> {
    m.resolved.get(&at).copied().flatten()
}
fn used(m: &Model, _t: &[Tok], scope: usize, name: &str, except: Option<(usize, usize)>) -> bool {
    m.uses.get(&scope).and_then(|s| s.get(name)).is_some_and(|refs| refs.iter().any(|&r| !except.is_some_and(|(a, b)| a <= r && r < b)))
}
fn dynamic(m: &Model, t: &[Tok]) -> Option<usize> {
    m.marks.refs.iter().copied().find(|&r| t[r].ident("eval") && resolve(m, "eval", r).is_none())
}
fn duplicates(m: &Model, scope: usize, name: &str) -> bool {
    m.counts.get(&scope).and_then(|s| s.get(name)).copied().unwrap_or(0) > 1
}
fn nested_function(m: &Model, a: usize, b: usize, at: usize) -> bool {
    let start = m.function_ranges.partition_point(|(s, _)| *s < a);
    let end = m.function_ranges.partition_point(|(s, _)| *s <= at);
    m.function_ranges[start..end].iter().any(|(_, e)| *e <= b && at < *e)
}
fn safe_read(m: &Model, t: &[Tok], at: usize) -> bool {
    let Some(scope) = resolve(m, &t[at].text, at) else { return false };
    if duplicates(m, scope, &t[at].text) {
        return false;
    }
    let owner_fn = |mut s: usize| {
        while !m.scopes[s].is_fn {
            s = m.scopes[s].parent.unwrap_or(0);
        }
        s
    };
    let read_fn = m.scopes.iter().enumerate().filter(|(_, s)| s.is_fn && s.start <= at && at < s.end).max_by_key(|(_, s)| s.start).map(|(i, _)| i).unwrap_or(0);
    if owner_fn(scope) != read_fn {
        return false;
    }
    m.marks.declarations.iter().filter(|d| d.scope == scope).any(|d| {
        d.parts.iter().any(|p| {
            p.names.iter().any(|&n| t[n].text == t[at].text) && (t[d.keyword].ident("var") || (p.end <= at && m.marks.statements.contains(&d.keyword)))
        })
    })
}
/// Only discard allocations/primitive literal work proven not to execute user code or throw.
fn pure_expression(m: &Model, t: &[Tok], a: usize, b: usize) -> bool {
    if a >= b {
        return false;
    }
    let lo = m.references.partition_point(|&r| r < a);
    let hi = m.references.partition_point(|&r| r < b);
    let references = m.references[lo..hi].iter().copied().filter(|&r| !nested_function(m, a, b, r)).collect::<Vec<_>>();
    if references.iter().any(|&r| !safe_read(m, t, r)) {
        return false;
    }
    let coercion = (a..b).any(|i| {
        !nested_function(m, a, b, i)
            && matches!(
                t[i].text.as_str(),
                "+" | "-" | "*" | "/" | "%" | "**" | "<" | ">" | "<=" | ">=" | "==" | "!=" | "~" | "|" | "&" | "^" | "<<" | ">>" | ">>>"
            )
    });
    if coercion && !references.is_empty() {
        return false;
    }
    let container = (a..b)
        .any(|i| !nested_function(m, a, b, i) && (t[i].punct("{") || t[i].punct("[") || t[i].kind == crate::lex::Kind::Regex || t[i].text.ends_with('n')));
    for i in a..b {
        if nested_function(m, a, b, i) {
            continue;
        }
        let x = &t[i];
        if x.ident("new")
            || x.ident("extends")
            || x.ident("await")
            || x.ident("yield")
            || x.ident("delete")
            || x.ident("throw")
            || x.ident("this")
            || x.ident("super")
            || x.ident("in")
            || x.ident("instanceof")
            || x.punct("...")
        {
            return false;
        }
        if x.punct("[") && i > a && !matches!(t[i - 1].text.as_str(), "(" | "=" | ":" | "=>") {
            return false;
        }
        if matches!(x.kind, crate::lex::Kind::TplHead | crate::lex::Kind::TplMiddle | crate::lex::Kind::TplTail) {
            return false;
        }
        if x.punct(".") || x.punct("?.") || matches!(x.text.as_str(), "=" | "++" | "--" | "+=" | "-=" | "*=" | "/=" | "||=" | "&&=" | "??=") {
            return false;
        }
        if x.punct("(")
            && i > a
            && !matches!(t[i - 1].text.as_str(), "(" | "=" | "," | ":" | "=>" | "return" | "void" | "typeof" | "!" | "~" | "+" | "-" | "?" | "&&" | "||" | "??")
        {
            return false;
        }
        if x.kind == crate::lex::Kind::Template && i > a && !matches!(t[i - 1].text.as_str(), "(" | "=" | "," | ":" | "=>") {
            return false;
        }
        if container
            && matches!(
                x.text.as_str(),
                "+" | "-" | "*" | "/" | "%" | "**" | "<" | ">" | "<=" | ">=" | "==" | "!=" | "===" | "!==" | "~" | "!" | "|" | "&" | "^" | "<<" | ">>" | ">>>"
            )
        {
            return false;
        }
    }
    true
}
fn pure_statement(m: &Model, t: &[Tok], a: usize, b: usize) -> bool {
    let mut head = a;
    if t.get(head).is_some_and(|t| t.ident("export")) {
        head += 1;
    }
    if t.get(head).is_some_and(|t| t.ident("function"))
        || (t.get(head).is_some_and(|t| t.ident("async")) && t.get(head + 1).is_some_and(|t| t.ident("function")))
    {
        return true;
    }
    if t.get(head).is_some_and(|t| t.ident("class")) {
        return t.get(head + 2).is_some_and(|t| t.punct("{")) && t.get(head + 3).is_some_and(|t| t.punct("}"));
    }
    let Some(d) = m.marks.declarations.iter().find(|d| d.keyword == head) else { return false };
    d.parts.iter().all(|p| {
        p.pattern_end == p.start + 1
            && t[p.start].kind == crate::lex::Kind::Ident
            && match p.init {
                Some(i) => pure_expression(m, t, i, p.end),
                None => !t[head].ident("const"),
            }
    }) && d.end <= b
}
fn synth(text: &str, at: usize) -> Tok {
    Tok { kind: crate::lex::Kind::Punct, text: text.into(), nl_before: false, start: at }
}
fn apply(t: &[Tok], mut edits: Vec<(usize, usize, Vec<Tok>)>, statements: bool) -> Vec<Tok> {
    edits.sort_by_key(|(a, b, _)| (*a, std::cmp::Reverse(*b)));
    let mut out: Vec<Tok> = vec![];
    let mut cursor = 0;
    for (a, b, replacement) in edits {
        if a < cursor {
            continue;
        }
        out.extend_from_slice(&t[cursor..a]);
        if statements && replacement.first().is_some_and(|t| t.punct("(")) && out.last().is_some_and(|t| !t.punct(";") && !t.punct("{") && !t.punct(":")) {
            out.push(synth(";", t[a].start));
        }
        for token in replacement {
            let generated_semi = token.punct(";") && token.start == t[a].start;
            if generated_semi && (out.is_empty() || out.last().is_some_and(|t| t.punct("{") || t.punct(";"))) {
                continue;
            }
            out.push(token);
        }
        cursor = b;
    }
    out.extend_from_slice(&t[cursor..]);
    out
}

fn part_tokens(t: &[Tok], d: &Declaration, p: &Part) -> Vec<Tok> {
    let mut out = vec![t[d.keyword].clone()];
    out.extend_from_slice(&t[p.start..p.end]);
    out.push(synth(";", t[d.keyword].start));
    out
}
fn import_edit(m: &Model, t: &[Tok], a: usize, b: usize, source: Option<&str>, report: &mut CleanupReport) -> Result<Option<Vec<Tok>>, CleanupError> {
    if t.get(a + 1).is_some_and(|t| t.kind == crate::lex::Kind::Str) {
        return Ok(None);
    }
    let Some(from) = (a + 1..b).find(|&i| t[i].ident("from") && t.get(i + 1).is_some_and(|t| t.kind == crate::lex::Kind::Str)) else { return Ok(None) };
    let Some(spec) = t.get(from + 1).filter(|t| t.kind == crate::lex::Kind::Str) else { return Ok(None) };
    let mut default = None;
    let mut namespace = None;
    let mut named = vec![];
    let mut removed = vec![];
    let mut i = a + 1;
    if t[i].kind == crate::lex::Kind::Ident {
        if used(m, t, 0, &t[i].text, None) {
            default = Some(t[i].clone());
        } else {
            removed.push(i);
        }
        i += 1;
        if t.get(i).is_some_and(|t| t.punct(",")) {
            i += 1;
        }
    }
    if t.get(i).is_some_and(|t| t.punct("*")) {
        let local = i + 2;
        if used(m, t, 0, &t[local].text, None) {
            namespace = Some(t[i..local + 1].to_vec());
        } else {
            removed.push(local);
        }
    } else if t.get(i).is_some_and(|t| t.punct("{")) {
        i += 1;
        while i < from && !t[i].punct("}") {
            let start = i;
            while i < from && !t[i].punct(",") && !t[i].punct("}") {
                i += 1;
            }
            let binding = (start..i).find(|n| m.marks.decl_at.contains_key(n));
            if let Some(at) = binding {
                if used(m, t, 0, &t[at].text, None) {
                    named.push(t[start..i].to_vec());
                } else {
                    removed.push(at);
                }
            } else {
                return Ok(None);
            }
            if t.get(i).is_some_and(|t| t.punct(",")) {
                i += 1;
            }
        }
    }
    if removed.is_empty() {
        return Ok(None);
    }
    let bare = default.is_none() && namespace.is_none() && named.is_empty();
    for at in removed {
        report.record(
            source,
            &t[at],
            if bare { Action::SideEffectImportRetained } else { Action::Removed },
            CleanupKind::Import,
            "unused-import-binding; module evaluation preserved",
            "Remove the unused source import binding; retain module evaluation unless its purity is proven",
        )?;
    }
    let mut out = vec![t[a].clone()];
    if let Some(local) = default {
        out.push(local);
        if namespace.is_some() || !named.is_empty() {
            out.push(synth(",", t[a].start));
        }
    }
    if let Some(ns) = namespace {
        out.extend(ns);
    }
    if !named.is_empty() {
        out.push(synth("{", t[a].start));
        for (j, n) in named.into_iter().enumerate() {
            if j > 0 {
                out.push(synth(",", t[a].start));
            }
            out.extend(n);
        }
        out.push(synth("}", t[a].start));
    }
    if !bare {
        out.push(t[from].clone());
    }
    out.push(spec.clone());
    let tail_end = b - usize::from(t.get(b.saturating_sub(1)).is_some_and(|t| t.punct(";")));
    out.extend_from_slice(&t[from + 2..tail_end]);
    out.push(synth(";", t[a].start));
    Ok(Some(out))
}
/// Existing local-only API and production module cleanup share this engine.
pub(crate) fn cleanup_tokens(mut t: Vec<Tok>, source: Option<&str>, only_locals: bool) -> Result<(Vec<Tok>, CleanupReport), CleanupError> {
    let mut report = CleanupReport::default();
    let mut retained = HashSet::new();
    let mut work = 0usize;
    for _ in 0..256 {
        work = work.saturating_add(t.len());
        if work > 8_000_000 {
            return Err(CleanupError::new(
                "CLEANUP_WORK_LIMIT",
                "cleanup",
                source,
                None,
                "Cleanup exceeded its bounded work budget",
                "Split long dependency chains before building",
            ));
        }
        let m = model(&t, source)?;
        if let Some(at) = dynamic(&m, &t) {
            report.record(
                source,
                &t[at],
                Action::Retained,
                CleanupKind::Scope,
                "dynamic-evaluation prevents a sound unused-binding proof",
                "Remove direct eval before requesting lexical source cleanup",
            )?;
            return Ok((t, report));
        }
        let mut edits = vec![];
        let mut functions = if only_locals {
            vec![]
        } else {
            m.marks
                .functions
                .iter()
                .copied()
                .filter(|&(a, b, scope, name_at)| {
                    let name = &t[name_at].text;
                    m.marks.statements.contains(&a)
                        && !(scope == 0 && m.marks.exported.contains(name))
                        && !duplicates(&m, scope, name)
                        && !used(&m, &t, scope, name, Some((a, b)))
                })
                .collect::<Vec<_>>()
        };
        functions.sort_by_key(|f| f.0);
        let mut modified_ranges = vec![];
        let mut declarations = m.marks.declarations.iter().collect::<Vec<_>>();
        declarations.sort_by_key(|d| d.keyword);
        for d in declarations {
            if only_locals && d.scope == 0
                || functions.iter().any(|(a, b, _, _)| *a <= d.keyword && d.end <= *b)
                || modified_ranges.iter().any(|(a, b)| *a <= d.keyword && d.end <= *b)
            {
                continue;
            }
            let any_unused = d.parts.iter().flat_map(|p| &p.names).any(|&at| !used(&m, &t, d.scope, &t[at].text, None));
            if !any_unused {
                continue;
            }
            let standalone = m.marks.statements.contains(&d.keyword);
            let mut replacement = vec![];
            let mut changed = false;
            for p in &d.parts {
                let simple = p.names.len() == 1 && p.pattern_end == p.start + 1 && t[p.start].kind == crate::lex::Kind::Ident;
                let unused = p.names.iter().all(|&at| !used(&m, &t, d.scope, &t[at].text, None));
                let exported = d.scope == 0 && p.names.iter().any(|&at| m.marks.exported.contains(&t[at].text));
                let ambiguous = p.names.iter().any(|&at| duplicates(&m, d.scope, &t[at].text));
                if unused && !exported && simple && standalone && !ambiguous && !(p.init.is_none() && t[d.keyword].ident("const")) {
                    changed = true;
                    let at = p.names[0];
                    if let Some(init) = p.init {
                        if pure_expression(&m, &t, init, p.end) {
                            report.record(
                                source,
                                &t[at],
                                Action::Removed,
                                CleanupKind::Variable,
                                "unused-binding with effect-free initializer",
                                "Remove the unused source declaration",
                            )?;
                        } else {
                            report.record(
                                source,
                                &t[at],
                                Action::InitializerRetained,
                                CleanupKind::Variable,
                                "unused-binding; initializer may execute effects or throw",
                                "Replace the unused source declaration with its initializer expression after reviewing side effects",
                            )?;
                            replacement.push(synth("(", t[d.keyword].start));
                            replacement.extend_from_slice(&t[init..p.end]);
                            replacement.push(synth(")", t[d.keyword].start));
                            replacement.push(synth(";", t[d.keyword].start));
                        }
                    } else {
                        report.record(
                            source,
                            &t[at],
                            Action::Removed,
                            CleanupKind::Variable,
                            "unused-binding without initializer",
                            "Remove the unused source declaration",
                        )?;
                    }
                } else {
                    replacement.extend(part_tokens(&t, d, p));
                    if unused && !exported {
                        for &at in &p.names {
                            if retained.insert(t[at].start) {
                                report.record(
                                    source,
                                    &t[at],
                                    Action::Retained,
                                    CleanupKind::Variable,
                                    if !simple {
                                        "destructuring/default/iterator semantics retained"
                                    } else if !standalone {
                                        "control-body or loop-header binding retained"
                                    } else {
                                        "ambiguous or invalid declaration retained"
                                    },
                                    "Review this unused binding in source; cleanup cannot discard its binding semantics safely",
                                )?;
                            }
                        }
                    }
                }
            }
            if changed {
                if replacement.is_empty() {
                    replacement.push(synth(";", t[d.keyword].start));
                }
                let end = d.end + usize::from(t.get(d.end).is_some_and(|t| t.punct(";")));
                modified_ranges.push((d.keyword, end));
                edits.push((d.keyword, end, replacement));
            }
        }
        let mut removed_functions = vec![];
        for (a, b, _scope, name_at) in functions {
            if modified_ranges.iter().any(|(x, y)| *x <= a && b <= *y) || removed_functions.iter().any(|(x, y)| *x <= a && b <= *y) {
                continue;
            }
            report.record(
                source,
                &t[name_at],
                Action::Removed,
                CleanupKind::Function,
                "unreferenced-function declaration",
                "Remove the unused source function after reviewing its public contract",
            )?;
            removed_functions.push((a, b));
            edits.push((a, b, vec![synth(";", t[a].start)]));
        }
        if !only_locals {
            for &(a, b) in &m.marks.import_spans {
                if let Some(out) = import_edit(&m, &t, a, b, source, &mut report)? {
                    edits.push((a, b, out));
                }
            }
        }
        if edits.is_empty() {
            return Ok((t, report));
        }
        t = apply(&t, edits, true);
    }
    Err(CleanupError::new(
        "CLEANUP_ITERATION_LIMIT",
        "cleanup",
        source,
        None,
        "Cleanup did not converge within 256 rounds",
        "Split the generated module or shorten its declaration chains",
    ))
}
/// A local export clause has no evaluation step. Prune only proven local,
/// identifier-named exports; foreign reexports/namespace exports stay untouched.
/// Rebuild the binding model afterwards so removed export roots cannot pin imports.
fn prune_local_exports(t: &[Tok], m: &Model, source: Option<&str>, keep: &HashSet<String>, report: &mut CleanupReport) -> Result<Option<Vec<Tok>>, CleanupError> {
    if keep.contains("*") { return Ok(None); }
    let mut exported = HashSet::new();
    let mut lists = Vec::new();
    for &(a,b) in &m.top {
        if !t[a].ident("export") { continue; }
        if t.get(a+1).is_some_and(|token|token.punct("{")) {
            let Some(close)=(a+2..b).find(|&i|t[i].punct("}")) else { return Ok(None); };
            if t.get(close+1).is_some_and(|token|token.ident("from")) { return Ok(None); }
            let mut fields=Vec::new();
            let mut at=a+2;
            while at<close {
                if t[at].kind!=Kind::Ident || m.counts.get(&0).and_then(|names|names.get(&t[at].text))!=Some(&1) { return Ok(None); }
                let start=at;
                let mut external=t[at].text.as_str();
                at+=1;
                if at<close && t[at].ident("as") {
                    at+=1;
                    if at>=close || t[at].kind!=Kind::Ident { return Ok(None); }
                    external=t[at].text.as_str();at+=1;
                }
                if !exported.insert(external) { return Ok(None); }
                fields.push((start,at,external));
                if at<close { if !t[at].punct(",") {return Ok(None);} at+=1; }
            }
            lists.push((a,b,close,fields));
        } else if t.get(a+1).is_some_and(|token|token.ident("default")) {
            if !exported.insert("default") {return Ok(None);}
        } else {
            let names:Vec<_>=m.scopes[0].decls.iter().filter(|(_,at)|a<=*at&&*at<b).collect();
            // Includes export-star and unsupported clauses: no speculative rewrites.
            if names.is_empty() {return Ok(None);}
            for (name,_) in names { if !exported.insert(name.as_str()) {return Ok(None);} }
        }
    }
    let mut edits=Vec::new();
    for (a,b,close,fields) in lists {
        if fields.iter().all(|(_,_,name)|keep.contains(*name)) {continue;}
        let mut replacement=vec![t[a].clone(),t[a+1].clone()];
        let mut added=false;
        for (start,end,name) in fields {
            if keep.contains(name) {
                if added {replacement.push(synth(",",t[start].start));}
                replacement.extend_from_slice(&t[start..end]);added=true;
            } else {
                report.record(source,&t[start],Action::Removed,CleanupKind::Scope,
                    "unused local export specifier removed; local reads and module evaluation remain protected",
                    "Remove this unused export specifier or retain its explicit public contract")?;
            }
        }
        replacement.push(t[close].clone());
        replacement.extend_from_slice(&t[close+1..b]);
        if !replacement.last().is_some_and(|token|token.punct(";")) {replacement.push(synth(";",t[a].start));}
        edits.push((a,b,replacement));
    }
    Ok((!edits.is_empty()).then(||apply(t,edits,true)))
}

/// Tree shaking uses the same model/purity rules; retained module evaluation is a root.
pub(crate) fn shake_tokens(t: &[Tok], source: Option<&str>, keep: &HashSet<String>) -> Result<(Vec<Tok>, CleanupReport), CleanupError> {
    let m = model(t, source)?;
    let mut report = CleanupReport::default();
    if let Some(at) = dynamic(&m, t) {
        report.record(
            source,
            &t[at],
            Action::Retained,
            CleanupKind::Scope,
            "dynamic-evaluation prevents tree shaking",
            "Remove direct eval before requesting lexical source cleanup",
        )?;
        return Ok((t.to_vec(), report));
    }
    let pruned=prune_local_exports(t,&m,source,keep,&mut report)?;
    let t=pruned.as_deref().unwrap_or(t);
    let m=if pruned.is_some(){model(t,source)?}else{m};
    let names: Vec<Vec<(String, usize)>> =
        m.top.iter().map(|&(a, b)| m.scopes[0].decls.iter().filter(|(_, at)| a <= *at && *at < b).cloned().collect()).collect();
    let mut uses = vec![HashSet::new(); m.top.len()];
    for &at in &m.marks.refs {
        if resolve(&m, &t[at].text, at) == Some(0) {
            if let Some(index) = m.top.iter().position(|&(a, b)| a <= at && at < b) {
                uses[index].insert(t[at].text.clone());
            }
        }
    }
    let mut live = vec![false; m.top.len()];
    let mut queue = vec![];
    for (index, &(a, b)) in m.top.iter().enumerate() {
        let exported = t[a].ident("export");
        if !pure_statement(&m, t, a, b) || (exported && (names[index].is_empty() || names[index].iter().any(|(n, _)| keep.contains(n)))) {
            live[index] = true;
            queue.push(index);
        }
    }
    while let Some(index) = queue.pop() {
        for name in &uses[index] {
            for (j, declared) in names.iter().enumerate() {
                if !live[j] && declared.iter().any(|(n, _)| n == name) {
                    live[j] = true;
                    queue.push(j);
                }
            }
        }
    }
    let mut edits = vec![];
    for (index, &(a, b)) in m.top.iter().enumerate() {
        if !live[index] {
            let mut head = a + usize::from(t[a].ident("export"));
            if t[head].ident("async") {
                head += 1;
            }
            let kind = if t[head].ident("function") {
                CleanupKind::Function
            } else if t[head].ident("class") {
                CleanupKind::Class
            } else {
                CleanupKind::Variable
            };
            for (_, at) in &names[index] {
                report.record(
                    source,
                    &t[*at],
                    Action::Removed,
                    kind,
                    "tree-shaken declaration has no live consumer",
                    "Remove the unreferenced source declaration or declare an intentional public contract",
                )?;
            }
            edits.push((a, b, vec![synth(";", t[a].start)]));
        }
    }
    let shaken = apply(t, edits, true);
    let (out, next) = cleanup_tokens(shaken, source, false)?;
    report.append(next)?;
    Ok((out, report))
}

/// Production debug removal preserves evaluation order, throwing reads, spreads
/// and argument side effects. Lexically shadowed console objects are application code.
pub(crate) fn drop_logs(mut t: Vec<Tok>, source: Option<&str>) -> Result<(Vec<Tok>, CleanupReport), CleanupError> {
    let mut report = CleanupReport::default();
    let mut work = 0usize;
    for _ in 0..256 {
        work = work.saturating_add(t.len());
        if work > 8_000_000 {
            return Err(CleanupError::new(
                "CLEANUP_WORK_LIMIT",
                "debug-cleanup",
                source,
                None,
                "Debug cleanup exceeded its bounded work budget",
                "Split deeply nested debug expressions",
            ));
        }
        let m = model(&t, source)?;
        if dynamic(&m, &t).is_some() {
            return Ok((t, report));
        }
        let mut edits = vec![];
        let mut covered = 0;
        for i in 0..t.len() {
            if i < covered
                || !t[i].ident("console")
                || !m.marks.refs.contains(&i)
                || resolve(&m, "console", i).is_some()
                || !t.get(i + 1).is_some_and(|t| t.punct("."))
                || !t.get(i + 2).is_some_and(|t| t.ident("log"))
                || !t.get(i + 3).is_some_and(|t| t.punct("("))
            {
                continue;
            }
            let parser = P { t: &t, i: 0, scopes: vec![], cur: 0, m: Marks::default() };
            let Some(end) = parser.matching(i + 3) else {
                return Err(failure(source, &t, i));
            };
            let (mut depth, mut start) = (0isize, i + 4);
            let mut args = vec![];
            for j in i + 4..end {
                match t[j].kind {
                    crate::lex::Kind::Punct if matches!(t[j].text.as_str(), "(" | "[" | "{") => depth += 1,
                    crate::lex::Kind::Punct if matches!(t[j].text.as_str(), ")" | "]" | "}") => depth -= 1,
                    crate::lex::Kind::TplHead => depth += 1,
                    crate::lex::Kind::TplTail => depth -= 1,
                    crate::lex::Kind::Punct if t[j].punct(",") && depth == 0 => {
                        if start < j {
                            args.push((start, j));
                        }
                        start = j + 1;
                    }
                    _ => {}
                }
            }
            if start < end {
                args.push((start, end));
            }
            let impure: Vec<_> = args.into_iter().filter(|&(a, b)| !pure_expression(&m, &t, a, b)).collect();
            let spread = impure.iter().any(|&(a, b)| t[a..b].iter().any(|t| t.punct("...")));
            let follow = t.get(end + 1);
            let wrap = follow.is_some_and(|t| {
                t.punct(".")
                    || t.punct("?.")
                    || t.punct("(")
                    || t.punct("[")
                    || t.punct("**")
                    || matches!(t.kind, crate::lex::Kind::Template | crate::lex::Kind::TplHead)
            });
            let mut out = vec![];
            if wrap {
                if m.marks.statements.contains(&i) && i > 0 && !matches!(t[i - 1].text.as_str(), ";" | "{" | ":") {
                    out.push(synth(";", t[i].start));
                }
                out.push(synth("(", t[i].start));
            }
            out.push(Tok { kind: crate::lex::Kind::Ident, text: "void".into(), nl_before: t[i].nl_before, start: t[i].start });
            if impure.is_empty() {
                out.push(Tok { kind: crate::lex::Kind::Num, text: "0".into(), nl_before: false, start: t[i].start });
            } else {
                out.push(synth(if spread { "[" } else { "(" }, t[i].start));
                for (index, (a, b)) in impure.iter().enumerate() {
                    if index > 0 {
                        out.push(synth(",", t[i].start));
                    }
                    out.extend_from_slice(&t[*a..*b]);
                }
                out.push(synth(if spread { "]" } else { ")" }, t[i].start));
            }
            if wrap {
                out.push(synth(")", t[i].start));
            }
            report.record(
                source,
                &t[i],
                if impure.is_empty() { Action::Removed } else { Action::InitializerRetained },
                CleanupKind::DebugLog,
                "production debug call removed; observable argument evaluation retained",
                "Remove or gate the source debug call; keep application effects outside logging arguments",
            )?;
            edits.push((i, end + 1, out));
            covered = end + 1;
        }
        if edits.is_empty() {
            return Ok((t, report));
        }
        t = apply(&t, edits, false);
    }
    Err(CleanupError::new("CLEANUP_ITERATION_LIMIT", "debug-cleanup", source, None, "Debug cleanup did not converge", "Split deeply nested debug expressions"))
}
