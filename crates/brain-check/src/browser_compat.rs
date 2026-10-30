//! Conservative application compatibility checks. Unsupported declarations are warnings; no polyfills are inferred.
use crate::{Ctx, baseline_data as data, security};
use brain_css::CssItem;
use brain_parse::ast::*;
use brain_parse::{Diag, Span};
use std::collections::{BTreeSet, HashSet};

fn warning(mut d: Diag) -> Diag {
    d.level = brain_parse::Severity::Warning;
    d
}

type Row = (
    &'static str,
    u16,
    bool,
    u8,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);
/// Read-only dataset identity for doctor and agent tooling; counts do not imply implemented match coverage.
#[derive(Clone, Copy, Debug)]
pub struct DatasetMetadata {
    pub source: &'static str,
    pub published: &'static str,
    pub data_sha256: &'static str,
    pub archive_integrity: &'static str,
    pub globals_sha256: &'static str,
    pub license: &'static str,
    pub browsers: &'static [&'static str],
    pub features: usize,
    pub exact_paths: usize,
    pub receiver_members: usize,
    pub raw_compat_keys: usize,
    pub unclassified_keys: usize,
}
pub fn dataset_metadata() -> DatasetMetadata {
    DatasetMetadata {
        source: data::SOURCE,
        published: data::PUBLISHED,
        data_sha256: data::DATA_SHA256,
        archive_integrity: data::ARCHIVE_INTEGRITY,
        globals_sha256: data::GLOBALS_SHA256,
        license: "Apache-2.0",
        browsers: data::BROWSERS,
        features: data::FEATURES.len(),
        exact_paths: data::PATHS.len(),
        receiver_members: data::MEMBERS.len(),
        raw_compat_keys: data::COMPAT_KEYS.len(),
        unclassified_keys: data::UNCLASSIFIED.len(),
    }
}
#[derive(Clone, Copy)]
enum Target {
    Widely,
    Newly,
    Year(u16),
}
impl Target {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "" | "widely" | "baseline-widely" => Some(Self::Widely),
            "newly" | "baseline-newly" => Some(Self::Newly),
            other => {
                let year = other.strip_prefix("baseline-")?.parse::<u16>().ok()?;
                (2015..=data::PUBLISHED
                    .get(..4)
                    .and_then(|s| s.parse::<u16>().ok())
                    .unwrap_or(2026))
                    .contains(&year)
                    .then_some(Self::Year(year))
            }
        }
    }
    fn supports(self, row: &Row) -> bool {
        match self {
            Self::Widely => row.2,
            Self::Newly => row.1 > 0,
            Self::Year(year) => row.1 > 0 && row.1 <= year,
        }
    }
}
fn find(table: &'static [Row], key: &str) -> Option<&'static Row> {
    table
        .binary_search_by_key(&key, |row| row.0)
        .ok()
        .map(|i| &table[i])
}
fn path(e: &Expr) -> Option<String> {
    match e.unparen() {
        Expr::Ident(i) => Some(i.name.clone()),
        Expr::Member { obj, prop, .. } => {
            let member = match prop {
                MemberProp::Name(i) => i.name.clone(),
                MemberProp::Computed(e) => match e.unparen() {
                    Expr::Str { value, .. } => value.clone(),
                    _ => return None,
                },
            };
            Some(format!("{}.{}", path(obj)?, member))
        }
        _ => None,
    }
}
fn normalize(value: &str) -> &str {
    value
        .strip_prefix("globalThis.")
        .or_else(|| value.strip_prefix("window."))
        .unwrap_or(value)
}
fn positive(e: &Expr) -> HashSet<String> {
    match e.unparen() {
        Expr::Ident(_) | Expr::Member { .. } => path(e)
            .map(|p| HashSet::from([normalize(&p).into()]))
            .unwrap_or_default(),
        Expr::Binary {
            op: "in",
            left,
            right,
            ..
        } => {
            if let Expr::Str { value, .. } = left.unparen() {
                if let Some(parent) = path(right) {
                    return HashSet::from([normalize(&format!("{parent}.{value}")).into()]);
                }
            }
            HashSet::new()
        }
        Expr::Binary {
            op: "&&",
            left,
            right,
            ..
        } => {
            let mut out = positive(left);
            out.extend(positive(right));
            out
        }
        Expr::Binary {
            op, left, right, ..
        } if matches!(*op, "===" | "==" | "!==" | "!=") => {
            for (test, value) in [(left, right), (right, left)] {
                if let Expr::Unary {
                    op: "typeof", arg, ..
                } = test.unparen()
                {
                    if let Expr::Str { value, .. } = value.unparen() {
                        let accepted = if matches!(*op, "===" | "==") {
                            matches!(value.as_str(), "function" | "object")
                        } else {
                            value == "undefined"
                        };
                        if accepted {
                            return path(arg)
                                .map(|p| HashSet::from([normalize(&p).into()]))
                                .unwrap_or_default();
                        }
                    }
                }
            }
            HashSet::new()
        }
        _ => HashSet::new(),
    }
}
fn probes(e: &Expr) -> HashSet<String> {
    let mut out = positive(e);
    match e.unparen() {
        Expr::Unary { op: "!", arg, .. } => out.extend(probes(arg)),
        Expr::Binary {
            op: "||" | "&&",
            left,
            right,
            ..
        } => {
            out.extend(probes(left));
            out.extend(probes(right));
        }
        Expr::Binary { left, right, .. } => {
            for candidate in [left, right] {
                if let Expr::Unary {
                    op: "typeof", arg, ..
                } = candidate.unparen()
                {
                    if let Some(p) = path(arg) {
                        out.insert(normalize(&p).into());
                    }
                }
            }
        }
        _ => {}
    }
    out
}
#[derive(Default)]
struct Facts {
    shadowed: HashSet<String>,
    guards: Vec<(Span, HashSet<String>)>,
    writes: HashSet<(u32, u32)>,
}
impl Facts {
    fn pat(&mut self, p: &Pat) {
        let mut names = vec![];
        p.bound_names(&mut names);
        self.shadowed
            .extend(names.into_iter().map(|i| i.name.clone()));
    }
    fn func(&mut self, f: &Func) {
        for p in &f.params {
            self.pat(p);
        }
        if let Some(n) = &f.name {
            self.shadowed.insert(n.name.clone());
        }
        if let FnBody::Block(body) = &f.body {
            for s in body {
                self.stmt(s);
            }
        }
    }
    fn guard(&mut self, test: &Expr, body: Span) {
        let probes = probes(test);
        if !probes.is_empty() {
            self.guards.push((test.span(), probes));
        }
        let known = positive(test);
        if !known.is_empty() {
            self.guards.push((body, known));
        }
    }
    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Var(v) => {
                for d in &v.decls {
                    self.pat(&d.pat);
                }
            }
            Stmt::Func(f) | Stmt::Server { func: f, .. } => self.func(f),
            Stmt::If {
                test, cons, alt, ..
            } => {
                self.guard(test, cons.span());
                self.stmt(cons);
                if let Some(a) = alt {
                    self.stmt(a);
                }
            }
            Stmt::ForOf { pat, body, .. } => {
                self.pat(pat);
                self.stmt(body);
            }
            Stmt::Block { body, .. } => {
                for s in body {
                    self.stmt(s);
                }
            }
            Stmt::Try {
                block,
                param,
                handler,
                finalizer,
                ..
            } => {
                if let Some(p) = param {
                    self.pat(p);
                }
                for s in block
                    .iter()
                    .chain(handler.iter().flatten())
                    .chain(finalizer.iter().flatten())
                {
                    self.stmt(s);
                }
            }
            Stmt::Export { decl, .. } => self.stmt(decl),
            _ => {}
        }
    }
    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Assign { target, .. } => {
                let span = target.span();
                self.writes.insert((span.start, span.end));
            }
            Expr::Func(f) => self.func(f),
            Expr::Object { props, .. } => {
                for p in props {
                    if let ObjProp::Method { func, .. } = p {
                        self.func(func);
                    }
                }
            }
            Expr::Binary {
                op: "&&",
                left,
                right,
                ..
            } => self.guard(left, right.span()),
            Expr::Cond { test, cons, .. } => self.guard(test, cons.span()),
            Expr::Call {
                callee,
                optional: true,
                span,
                ..
            } => {
                if let Some(p) = path(callee) {
                    self.guards
                        .push((*span, HashSet::from([normalize(&p).into()])));
                }
            }
            _ => {}
        }
    }
    fn nodes(&mut self, nodes: &[Node]) {
        for n in nodes {
            match n {
                Node::If(b) => {
                    for branch in &b.branches {
                        self.guard(&branch.test, branch.span);
                        self.nodes(&branch.body);
                    }
                    if let Some(body) = &b.otherwise {
                        self.nodes(body);
                    }
                }
                Node::Each(b) => {
                    self.pat(&b.item);
                    if let Some(i) = &b.index {
                        self.shadowed.insert(i.name.clone());
                    }
                    self.nodes(&b.body);
                    if let Some(body) = &b.empty {
                        self.nodes(body);
                    }
                }
                Node::Await(b) => {
                    self.nodes(&b.pending);
                    for branch in b.then.iter().chain(b.catch.iter()) {
                        if let Some(p) = &branch.binding {
                            self.pat(p);
                        }
                        self.nodes(&branch.body);
                    }
                }
                Node::SlotScope(b) => {
                    for n in &b.names {
                        self.shadowed.insert(n.name.clone());
                    }
                    self.nodes(&b.body);
                }
                Node::Element(e) => self.nodes(&e.children),
                Node::ComponentRef(e) => self.nodes(&e.children),
                _ => {}
            }
        }
    }
    fn guarded(&self, key: &str, span: Span) -> bool {
        self.guards
            .iter()
            .any(|(s, keys)| s.start <= span.start && s.end >= span.end && keys.contains(key))
    }
}
struct Analyzer<'a> {
    c: &'a Component,
    target: Target,
    facts: Facts,
    diags: Vec<Diag>,
    seen: BTreeSet<(u32, String)>,
}
impl Analyzer<'_> {
    fn feature(&mut self, table: &'static [Row], key: &str, span: Span, code: &'static str) {
        let Some(row) = find(table, key) else { return };
        if self.target.supports(row)
            || self.diags.len() >= 100
            || !self.seen.insert((span.start, key.into()))
        {
            return;
        }
        let state = if row.1 == 0 {
            "limited availability"
        } else if row.2 {
            "widely available"
        } else {
            "newly available"
        };
        let feature_ids: Vec<_> = row
            .4
            .split(',')
            .filter_map(|i| i.parse::<usize>().ok())
            .filter_map(|i| data::FEATURES.get(i))
            .copied()
            .take(4)
            .collect();
        self.diags.push(warning(Diag::new(code,&self.c.file,span,format!("`{key}` is outside the configured browser target ({state}; {}; features: {})",data::SOURCE,feature_ids.join(", ")),format!("use a supported alternative, or guard `{key}` with a feature test / matching @supports and provide the fallback outside that branch; this checker does not inject polyfills"))));
    }
    fn js(&mut self, e: &Expr, server: bool) {
        if server {
            return;
        }
        let Some(raw) = path(e) else { return };
        let root = raw.split('.').next().unwrap_or("");
        if self.facts.shadowed.contains(root) || !crate::scope::KNOWN_GLOBALS.contains(&root) {
            return;
        }
        let key = normalize(&raw);
        if self.facts.writes.contains(&(e.span().start, e.span().end))
            || self.facts.guarded(key, e.span())
        {
            return;
        }
        self.feature(data::PATHS, key, e.span(), "B2101");
    }
    fn css(
        &mut self,
        items: &[CssItem],
        props: &HashSet<String>,
        selectors: &HashSet<String>,
        location: Option<Span>,
    ) {
        for item in items {
            let span = location.unwrap_or_else(|| item.span());
            match item {
                CssItem::Decl { name, .. } => {
                    if !props.contains(name) {
                        self.feature(data::CSS_PROPERTIES, name, span, "B2102");
                    }
                }
                CssItem::Rule {
                    selector, items, ..
                } => {
                    for key in selector_names(selector) {
                        if !selectors.contains(&key) {
                            self.feature(data::CSS_SELECTORS, &key, span, "B2102");
                        }
                    }
                    self.css(items, props, selectors, location);
                }
                CssItem::At {
                    name,
                    prelude,
                    block,
                    ..
                } => {
                    self.feature(data::CSS_AT_RULES, name, span, "B2102");
                    if let Some(body) = block {
                        let (mut p, mut s) = (props.clone(), selectors.clone());
                        if name == "supports" {
                            let (extra_p, extra_s) = supports(prelude);
                            p.extend(extra_p);
                            s.extend(extra_s);
                        }
                        self.css(body, &p, &s, location);
                    }
                }
            }
        }
    }
    fn nodes(&mut self, nodes: &[Node]) {
        for n in nodes {
            match n {
                Node::Element(e) => {
                    self.feature(data::HTML_ELEMENTS, &e.tag, e.span, "B2102");
                    for attr in &e.attrs {
                        if attr.kind == AttrKind::Plain {
                            self.feature(
                                data::HTML_ATTRIBUTES,
                                &format!("*.{}", attr.name),
                                attr.span,
                                "B2102",
                            );
                            self.feature(
                                data::HTML_ATTRIBUTES,
                                &format!("{}.{}", e.tag, attr.name),
                                attr.span,
                                "B2102",
                            );
                            if e.tag == "input" && attr.name == "type" {
                                if let AttrValue::Text(value) = &attr.value {
                                    self.feature(
                                        data::HTML_ATTRIBUTES,
                                        &format!("input[type={value}]"),
                                        attr.span,
                                        "B2102",
                                    );
                                }
                            }
                            if attr.name == "style" {
                                if let AttrValue::Text(value) = &attr.value {
                                    if let Ok(sheet) = brain_css::parse_str(value, &self.c.file) {
                                        self.css(
                                            &sheet.items,
                                            &HashSet::new(),
                                            &HashSet::new(),
                                            Some(attr.span),
                                        );
                                    }
                                }
                            }
                        } else if attr.kind == AttrKind::Style {
                            self.feature(data::CSS_PROPERTIES, &attr.name, attr.span, "B2102");
                        }
                    }
                    self.nodes(&e.children);
                }
                Node::ComponentRef(e) => self.nodes(&e.children),
                Node::If(b) => {
                    for branch in &b.branches {
                        self.nodes(&branch.body);
                    }
                    if let Some(body) = &b.otherwise {
                        self.nodes(body);
                    }
                }
                Node::Each(b) => {
                    self.nodes(&b.body);
                    if let Some(body) = &b.empty {
                        self.nodes(body);
                    }
                }
                Node::Await(b) => {
                    self.nodes(&b.pending);
                    for branch in b.then.iter().chain(b.catch.iter()) {
                        self.nodes(&branch.body);
                    }
                }
                Node::Slot(_) => {}
                Node::SlotScope(s) => self.nodes(&s.body),
                _ => {}
            }
        }
    }
}
/// Recognize pseudo names while ignoring strings and attribute-selector contents.
fn selector_names(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut chars = text.chars().peekable();
    let mut quoted = None;
    let mut brackets = 0;
    while let Some(c) = chars.next() {
        if c == '\\' {
            chars.next();
            continue;
        }
        if let Some(q) = quoted {
            if c == q {
                quoted = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quoted = Some(c);
            continue;
        }
        if c == '[' {
            brackets += 1;
        } else if c == ']' {
            brackets -= 1;
        } else if c == ':' && brackets == 0 {
            if chars.peek() == Some(&':') {
                chars.next();
            }
            let mut name = String::new();
            while chars
                .peek()
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == '-')
            {
                name.push(chars.next().unwrap());
            }
            if !name.is_empty() {
                out.insert(name);
            }
        }
    }
    out
}
fn single_css_test(text: &str) -> bool {
    let Some(start) = text.find('(') else {
        return false;
    };
    let suffix = &text[start..];
    let mut depth = 0i32;
    let mut quote = None;
    let mut escaped = false;
    for (offset, c) in suffix.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            continue;
        }
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                return offset + 1 == suffix.len();
            }
            if depth < 0 {
                return false;
            }
        }
    }
    false
}
/// Only a single positive property or selector test is proof; OR/NOT and unknown syntax remain unguarded.
fn supports(text: &str) -> (HashSet<String>, HashSet<String>) {
    let text = text.trim();
    let mut props = HashSet::new();
    let mut selectors = HashSet::new();
    if !single_css_test(text) {
        return (props, selectors);
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains(" or ") || lower.contains(" and ") || lower.starts_with("not ") {
        return (props, selectors);
    }
    if let Some(inner) = text
        .strip_prefix("selector(")
        .and_then(|s| s.strip_suffix(')'))
    {
        selectors = selector_names(inner);
    } else if let Some(inner) = text.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        if let Some((name, _)) = inner.split_once(':') {
            let name = name.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
                props.insert(name.into());
            }
        }
    }
    (props, selectors)
}

pub fn check(c: &Component, ctx: &Ctx) -> Vec<Diag> {
    if c.meta
        .iter()
        .any(|m| m.name == "server" || m.name == "protect")
    {
        return vec![];
    }
    let Some(target) = Target::parse(&ctx.browser_target) else {
        return vec![warning(Diag::new(
            "B2103",
            &c.file,
            Span {
                line: 1,
                col: 1,
                ..Span::default()
            },
            "invalid browser target",
            "set brain.json targets to baseline-widely, baseline-newly, or baseline-YYYY (2015 through 2026)",
        ))];
    };
    let mut facts = Facts::default();
    facts
        .shadowed
        .extend(crate::scope::top_bindings(c).into_keys());
    for s in &c.script {
        facts.stmt(s);
    }
    facts.nodes(&c.template);
    security::walk(&c.script, false, &mut |e, _, _| facts.expr(e));
    crate::planned::collect_calls(&c.template, &mut |e, _, _| facts.expr(e));
    let mut analyzer = Analyzer {
        c,
        target,
        facts,
        diags: vec![],
        seen: BTreeSet::new(),
    };
    security::walk(&c.script, false, &mut |e, server, _| analyzer.js(e, server));
    crate::planned::collect_calls(&c.template, &mut |e, server, _| analyzer.js(e, server));
    analyzer.nodes(&c.template);
    if let Some(sheet) = &c.style {
        analyzer.css(&sheet.items, &HashSet::new(), &HashSet::new(), None);
    }
    analyzer.diags.sort_by_key(|d| (d.line, d.col, d.code));
    analyzer.diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn check_source(source: &str, target: &str) -> Vec<Diag> {
        let c = brain_parse::parse(source, Path::new("comp/test.brain")).unwrap();
        check(
            &c,
            &Ctx {
                browser_target: target.into(),
                ..Ctx::default()
            },
        )
    }
    #[test]
    fn unsupported_global_has_precise_location() {
        let ds = check_source(
            "<script>\nconst p = Promise.try(() => 1)\n</script><p>OK</p>",
            "baseline-widely",
        );
        assert!(
            ds.iter()
                .any(|d| d.code == "B2101" && d.line == 2 && d.msg.contains("Promise.try"))
        );
        assert!(ds.iter().all(Diag::is_warning));
    }
    #[test]
    fn guards_are_branch_scoped() {
        let ds = check_source(
            "<script>if ('try' in Promise) { Promise.try(() => 1) } else { Promise.try(() => 2) }</script><p>OK</p>",
            "baseline-widely",
        );
        assert_eq!(
            ds.iter().filter(|d| d.msg.contains("Promise.try")).count(),
            1
        );
    }
    #[test]
    fn true_typeof_and_optional_call_are_progressive() {
        let ds = check_source(
            "<script>if (typeof Promise.try === 'function') { Promise.try(() => 1) } Promise.try?.(() => 2)</script><p>OK</p>",
            "baseline-widely",
        );
        assert!(ds.is_empty(), "{ds:?}");
    }
    #[test]
    fn shadowed_globals_and_user_members_are_not_guessed() {
        let ds = check_source(
            "<script>function work(Promise) { return Promise.try() } const own = { moveBefore() {} }; own.moveBefore()</script><p>OK</p>",
            "baseline-widely",
        );
        assert!(ds.is_empty(), "{ds:?}");
    }
    #[test]
    fn server_code_is_excluded() {
        let ds = check_source(
            "<script>server: async function save(ctx) { return Promise.try(() => 1) }</script><p>OK</p>",
            "baseline-widely",
        );
        assert!(ds.is_empty(), "{ds:?}");
    }
    #[test]
    fn css_supports_only_suppresses_matching_positive_test() {
        let ds = check_source(
            "<p class=\"x\">OK</p><style>@supports (field-sizing: content) { .x { field-sizing: content; interpolate-size: allow-keywords; } }</style>",
            "baseline-widely",
        );
        assert!(!ds.iter().any(|d| d.msg.contains("`field-sizing`")));
        assert!(ds.iter().any(|d| d.msg.contains("interpolate-size")));
    }
    #[test]
    fn css_selector_strings_are_not_features() {
        assert!(!selector_names("[data-text=\":has\"] .x").contains("has"));
        assert!(selector_names(".x:has(input)::before").contains("has"));
    }
    #[test]
    fn negative_feature_probe_is_safe_but_or_branch_does_not_prove_availability() {
        let ds = check_source(
            "<script>if (!Promise.try) { log('fallback') } if (Promise.try || other) { Promise.try(() => 1) }</script><p>OK</p>",
            "baseline-widely",
        );
        assert_eq!(
            ds.iter().filter(|d| d.msg.contains("Promise.try")).count(),
            1
        );
    }
    #[test]
    fn disjunctive_supports_does_not_hide_unsupported_properties() {
        assert!(
            supports("(field-sizing:content)or(display:block)")
                .0
                .is_empty()
        );
        assert!(
            supports("(field-sizing: content) or (display: block)")
                .0
                .is_empty()
        );
        assert!(
            supports("selector(:has(*)) OR selector(:is(*))")
                .1
                .is_empty()
        );
    }
    #[test]
    fn template_guards_and_bindings_preserve_provenance() {
        let ds = check_source(
            "{#if 'try' in Promise}<p>{Promise.try(() => 1)}</p>{:else}<p>Fallback</p>{/if}",
            "baseline-widely",
        );
        assert!(ds.is_empty(), "{ds:?}");
        let ds = check_source(
            "<script>const items = []</script>{#each items as Promise (Promise.id)}<p>{Promise.try()}</p>{/each}",
            "baseline-widely",
        );
        assert!(ds.is_empty(), "{ds:?}");
        let info = dataset_metadata();
        assert!(!info.data_sha256.is_empty());
        assert!(info.raw_compat_keys >= info.exact_paths);
    }
    #[test]
    fn invalid_target_is_actionable_warning() {
        let ds = check_source("<p>OK</p>", "all-browsers");
        assert_eq!(ds[0].code, "B2103");
        assert!(ds[0].is_warning());
    }
    #[test]
    fn newly_target_accepts_supported_new_features() {
        assert!(
            check_source(
                "<script>Promise.try(() => 1)</script><p>OK</p>",
                "baseline-newly"
            )
            .is_empty()
        );
    }
}
