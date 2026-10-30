//! Attribute cleanup facts to unique original declarations, never to guessed source instructions.
use brain_parse::Span;
use brain_parse::ast::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Component as PathComponent, Path};

const MAX_FACTS: usize = 4096;
const MAX_SOURCE_BYTES: usize = 262144;
const MAX_VISITS: usize = 65536;
const MAX_DECLARATIONS: usize = 8192;
const MAX_DEPTH: usize = 128;
#[derive(Clone, Copy)]
struct Declaration {
    kind: &'static str,
    span: Span,
    callable: bool,
}
#[derive(Default)]
struct Declarations {
    names: BTreeMap<String, Vec<Declaration>>,
    visits: usize,
    declarations: usize,
    truncated: bool,
    capture_references: bool,
    references: Vec<SourceReference>,
    methods: Vec<SourceSymbol>,
}
impl Declarations {
    fn enter(&mut self, depth: usize) -> bool {
        self.visits += 1;
        if depth > MAX_DEPTH || self.visits > MAX_VISITS {
            self.truncated = true;
            false
        } else {
            true
        }
    }
    fn record(&mut self, name: &str, kind: &'static str, span: Span) {
        if !safe_binding(name) {
            return;
        }
        if self.declarations >= MAX_DECLARATIONS {
            self.truncated = true;
            return;
        }
        self.declarations += 1;
        self.names
            .entry(name.into())
            .or_default()
            .push(Declaration {
                kind,
                span,
                callable: kind == "function",
            });
    }
    fn reference(&mut self, i: &Ident) {
        if !self.capture_references || !safe_binding(&i.name) {
            return;
        }
        if self.references.len() >= MAX_VISITS {
            self.truncated = true;
            return;
        }
        self.references.push(SourceReference {
            name: i.name.clone(),
            span: i.span,
        });
    }
    fn method(&mut self, key: &PropKey) {
        if !self.capture_references {
            return;
        }
        if let PropKey::Ident(i) = key {
            if safe_binding(&i.name) {
                if self.methods.len() >= MAX_DECLARATIONS {
                    self.truncated = true;
                } else {
                    self.methods.push(SourceSymbol {
                        name: i.name.clone(),
                        kind: "method",
                        span: i.span,
                        callable: true,
                    });
                }
            }
        }
    }
    fn key(&mut self, key: &PropKey, depth: usize) {
        if let PropKey::Computed(e) = key {
            self.expr(e, depth + 1);
        }
    }
    fn pat(&mut self, p: &Pat, kind: &'static str, depth: usize) {
        if !self.enter(depth) {
            return;
        }
        match p {
            Pat::Ident(i) => self.record(&i.name, kind, i.span),
            Pat::Object { props, rest, .. } => {
                for p in props {
                    self.key(&p.key, depth);
                    self.pat(&p.value, kind, depth + 1);
                }
                if let Some(p) = rest {
                    self.pat(p, kind, depth + 1);
                }
            }
            Pat::Array { elems, rest, .. } => {
                for p in elems.iter().flatten() {
                    self.pat(p, kind, depth + 1);
                }
                if let Some(p) = rest {
                    self.pat(p, kind, depth + 1);
                }
            }
            Pat::Default { pat, default, .. } => {
                self.pat(pat, kind, depth + 1);
                self.expr(default, depth + 1);
            }
            Pat::Rest { arg, .. } => self.pat(arg, kind, depth + 1),
        }
    }
    fn func(&mut self, f: &Func, named: bool, depth: usize) {
        if !self.enter(depth) {
            return;
        }
        if named {
            if let Some(i) = &f.name {
                self.record(&i.name, "function", i.span);
            }
        }
        for p in &f.params {
            self.pat(p, "variable", depth + 1);
        }
        match &f.body {
            FnBody::Expr(e) => self.expr(e, depth + 1),
            FnBody::Block(body) => {
                for s in body {
                    self.stmt(s, depth + 1);
                }
            }
        }
    }
    fn stmt(&mut self, s: &Stmt, depth: usize) {
        if !self.enter(depth) {
            return;
        }
        match s {
            Stmt::Import(i) => {
                for n in i
                    .default
                    .iter()
                    .chain(i.namespace.iter())
                    .chain(i.named.iter().map(|(_, local)| local))
                {
                    self.record(&n.name, "import", n.span);
                }
            }
            Stmt::Var(v) => {
                for d in &v.decls {
                    self.pat(&d.pat, "variable", depth + 1);
                    if let Some(e) = &d.init {
                        if matches!(e.unparen(), Expr::Func(_)) {
                            if let Pat::Ident(i) = &d.pat {
                                if let Some(decl) = self
                                    .names
                                    .get_mut(&i.name)
                                    .and_then(|items| items.last_mut())
                                {
                                    decl.callable = true;
                                }
                            }
                        }
                        self.expr(e, depth + 1);
                    }
                }
            }
            Stmt::Expr { expr, .. }
            | Stmt::ExportDefault { expr, .. }
            | Stmt::Throw { arg: expr, .. } => self.expr(expr, depth + 1),
            Stmt::If {
                test, cons, alt, ..
            } => {
                self.expr(test, depth + 1);
                self.stmt(cons, depth + 1);
                if let Some(s) = alt {
                    self.stmt(s, depth + 1);
                }
            }
            Stmt::ForOf {
                pat, iter, body, ..
            } => {
                self.pat(pat, "variable", depth + 1);
                self.expr(iter, depth + 1);
                self.stmt(body, depth + 1);
            }
            Stmt::Return { arg, .. } => {
                if let Some(e) = arg {
                    self.expr(e, depth + 1);
                }
            }
            Stmt::Block { body, .. } => {
                for s in body {
                    self.stmt(s, depth + 1);
                }
            }
            Stmt::Func(f) | Stmt::Server { func: f, .. } => self.func(f, true, depth + 1),
            Stmt::Try {
                block,
                param,
                handler,
                finalizer,
                ..
            } => {
                if let Some(p) = param {
                    self.pat(p, "variable", depth + 1);
                }
                for s in block
                    .iter()
                    .chain(handler.iter().flatten())
                    .chain(finalizer.iter().flatten())
                {
                    self.stmt(s, depth + 1);
                }
            }
            Stmt::Export { decl, .. } => self.stmt(decl, depth + 1),
            Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Empty { .. } => {}
        }
    }
    fn expr(&mut self, e: &Expr, depth: usize) {
        if !self.enter(depth) {
            return;
        }
        match e {
            Expr::Template { tag, exprs, .. } => {
                if let Some(e) = tag {
                    self.expr(e, depth + 1);
                }
                for e in exprs {
                    self.expr(e, depth + 1);
                }
            }
            Expr::Array { elems, .. } => {
                for e in elems.iter().flatten() {
                    self.expr(e, depth + 1);
                }
            }
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjProp::KeyValue { key, value, .. } => {
                            if matches!(value.unparen(), Expr::Func(_)) {
                                self.method(key);
                            }
                            self.key(key, depth);
                            self.expr(value, depth + 1);
                        }
                        ObjProp::Method { key, func, .. } => {
                            self.method(key);
                            self.key(key, depth);
                            self.func(func, false, depth + 1);
                        }
                        ObjProp::Spread { arg, .. } => self.expr(arg, depth + 1),
                        ObjProp::Shorthand(i) => self.reference(i),
                    }
                }
            }
            Expr::Func(f) => self.func(f, true, depth + 1),
            Expr::Unary { arg, .. }
            | Expr::Update { arg, .. }
            | Expr::Await { arg, .. }
            | Expr::Spread { arg, .. }
            | Expr::Paren { expr: arg, .. } => self.expr(arg, depth + 1),
            Expr::Binary { left, right, .. } => {
                self.expr(left, depth + 1);
                self.expr(right, depth + 1);
            }
            Expr::Assign { target, value, .. } => {
                self.expr(target, depth + 1);
                self.expr(value, depth + 1);
            }
            Expr::Cond {
                test, cons, alt, ..
            } => {
                self.expr(test, depth + 1);
                self.expr(cons, depth + 1);
                self.expr(alt, depth + 1);
            }
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                self.expr(callee, depth + 1);
                for e in args {
                    self.expr(e, depth + 1);
                }
            }
            Expr::Member { obj, prop, .. } => {
                self.expr(obj, depth + 1);
                if let MemberProp::Computed(e) = prop {
                    self.expr(e, depth + 1);
                }
            }
            Expr::Seq { exprs, .. } => {
                for e in exprs {
                    self.expr(e, depth + 1);
                }
            }
            Expr::Ident(i) => self.reference(i),
            Expr::This(_)
            | Expr::Num { .. }
            | Expr::Str { .. }
            | Expr::Regex { .. }
            | Expr::Bool { .. }
            | Expr::Null(_) => {}
        }
    }
    fn attrs(&mut self, attrs: &[Attr], depth: usize) {
        for attr in attrs {
            match &attr.value {
                AttrValue::Expr(e) => self.expr(e, depth + 1),
                AttrValue::Mixed(parts) => {
                    for p in parts {
                        if let AttrPart::Expr(e) = p {
                            self.expr(e, depth + 1);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn nodes(&mut self, nodes: &[Node], depth: usize) {
        if !self.enter(depth) {
            return;
        }
        for n in nodes {
            match n {
                Node::Element(e) => {
                    self.attrs(&e.attrs, depth);
                    self.nodes(&e.children, depth + 1);
                }
                Node::ComponentRef(e) => {
                    self.attrs(&e.attrs, depth);
                    self.nodes(&e.children, depth + 1);
                }
                Node::Interp(e) => self.expr(&e.expr, depth + 1),
                Node::If(b) => {
                    for branch in &b.branches {
                        self.expr(&branch.test, depth + 1);
                        self.nodes(&branch.body, depth + 1);
                    }
                    if let Some(body) = &b.otherwise {
                        self.nodes(body, depth + 1);
                    }
                }
                Node::Each(b) => {
                    self.expr(&b.list, depth + 1);
                    self.pat(&b.item, "variable", depth + 1);
                    if let Some(i) = &b.index {
                        self.record(&i.name, "variable", i.span);
                    }
                    if let Some(e) = &b.key {
                        self.expr(e, depth + 1);
                    }
                    self.nodes(&b.body, depth + 1);
                    if let Some(body) = &b.empty {
                        self.nodes(body, depth + 1);
                    }
                }
                Node::Await(b) => {
                    self.expr(&b.promise, depth + 1);
                    self.nodes(&b.pending, depth + 1);
                    for branch in b.then.iter().chain(b.catch.iter()) {
                        if let Some(p) = &branch.binding {
                            self.pat(p, "variable", depth + 1);
                        }
                        self.nodes(&branch.body, depth + 1);
                    }
                }
                Node::Slot(s) => self.attrs(&s.attrs, depth),
                Node::SlotScope(s) => {
                    for i in &s.names {
                        self.record(&i.name, "variable", i.span);
                    }
                    self.nodes(&s.body, depth + 1);
                }
                Node::Text(_) => {}
            }
        }
    }
    fn collect(c: &Component) -> Self {
        Self::collect_with_references(c, false)
    }
    fn collect_with_references(c: &Component, capture_references: bool) -> Self {
        let mut out = Self {
            capture_references,
            ..Self::default()
        };
        for p in &c.props {
            out.record(&p.name, "prop", p.span);
            if let Some(e) = &p.default {
                out.expr(e, 1);
            }
        }
        for meta in &c.meta {
            if let Some(e) = &meta.expr {
                out.expr(e, 1);
            }
        }
        for s in &c.script {
            out.stmt(s, 1);
        }
        out.nodes(&c.template, 1);
        out.nodes(&c.head, 1);
        out
    }
}
/// Literal-free declaration metadata shared with native symbol search.
pub(crate) struct SourceSymbol {
    pub name: String,
    pub kind: &'static str,
    pub span: Span,
    pub callable: bool,
}
pub(crate) struct SourceReference {
    pub name: String,
    pub span: Span,
}
pub(crate) fn source_index(
    c: &Component,
) -> Result<(Vec<SourceSymbol>, Vec<SourceReference>), String> {
    let found = Declarations::collect_with_references(c, true);
    if found.truncated {
        return Err("source declaration budget exceeded".into());
    }
    let mut symbols: Vec<SourceSymbol> = found
        .names
        .into_iter()
        .flat_map(|(name, items)| {
            items.into_iter().map(move |d| SourceSymbol {
                name: name.clone(),
                kind: d.kind,
                span: d.span,
                callable: d.callable,
            })
        })
        .collect();
    symbols.extend(found.methods);
    Ok((symbols, found.references))
}
fn safe_binding(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_alphabetic() || b == b'_' || b == b'$' || i > 0 && b.is_ascii_digit()
        })
}
pub(crate) fn public_path(file: &str) -> bool {
    !file.is_empty()
        && file.len() <= 240
        && brain_check::scan_secrets(file).is_empty()
        && !file.contains(['\\', ':', '?', '#'])
        && !file.chars().any(char::is_control)
        && Path::new(file)
            .components()
            .all(|p| matches!(p, PathComponent::Normal(_)))
        && file.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.starts_with('.')
                && !p.to_ascii_lowercase().contains(".env")
                && !["secrets", "credentials", "node_modules", "dist", "target"]
                    .contains(&p.to_ascii_lowercase().as_str())
        })
}
fn positive(value: &Value) -> Value {
    value
        .as_u64()
        .filter(|n| *n > 0)
        .map(Value::from)
        .unwrap_or(Value::Null)
}

/// Annotates a bounded cleanup report. Read/parse uncertainty is reported in metadata, never guessed.
pub fn annotate(root: &Path, source_file: &str, report: &mut Value) -> Result<(), String> {
    let object = report
        .as_object()
        .ok_or("optimization feedback must be an object")?;
    let facts = object
        .get("facts")
        .and_then(Value::as_array)
        .ok_or("optimization feedback needs a facts array")?;
    if facts.len() > MAX_FACTS || facts.iter().any(|f| !f.is_object()) {
        return Err("optimization feedback exceeds 4096 facts or contains a malformed fact".into());
    }
    let expected_revision = report
        .get("originalSourceHash")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let safe = public_path(source_file);
    let eligible = safe
        && !source_file.starts_with("runtime/")
        && !source_file.starts_with("compiler/")
        && [".brain", ".jsbrain", ".js", ".mjs"]
            .iter()
            .any(|ext| source_file.ends_with(ext));
    let mut declarations = None;
    let mut revision = Value::Null;
    let mut status = if !safe {
        "unsafe-source"
    } else if !eligible {
        "non-app-source"
    } else {
        "source-unavailable"
    };
    if eligible {
        if let Ok(snapshot) = crate::changes::read(root, source_file) {
            if let Some(src) = snapshot.get("content").and_then(Value::as_str) {
                revision = snapshot["revision"].clone();
                if expected_revision.is_none() {
                    status = "source-revision-unverified";
                } else if expected_revision.as_deref() != snapshot["revision"].as_str() {
                    status = "stale-source";
                } else if src.len() > MAX_SOURCE_BYTES {
                    status = "source-too-large";
                } else {
                    let parse_path =
                        if source_file.ends_with(".js") || source_file.ends_with(".mjs") {
                            Path::new("source.jsbrain")
                        } else {
                            Path::new(source_file)
                        };
                    match brain_parse::parse(src, parse_path) {
                        Ok(c) => {
                            let found = Declarations::collect(&c);
                            status = if found.truncated {
                                "declaration-limit"
                            } else {
                                "collected"
                            };
                            if !found.truncated {
                                declarations = Some(found);
                                revision = snapshot["revision"].clone();
                            }
                        }
                        Err(_) => status = "parse-unavailable",
                    }
                }
            }
        }
    }
    let mut mapped = 0usize;
    let mut ambiguous = 0usize;
    for fact in report["facts"].as_array_mut().unwrap() {
        // Keep minifier coordinates unchanged and label their input-JS coordinate space explicitly.
        fact["generatedLocation"] = json!({"file":if safe{Some(source_file)}else{None},"line":positive(&fact["line"]),"column":positive(&fact["column"]),"coordinateSpace":"input-js"});
        let candidate = fact["binding"]
            .as_str()
            .filter(|name| safe_binding(name))
            .and_then(|name| declarations.as_ref().and_then(|d| d.names.get(name)));
        let matched = match candidate {
            Some(items)
                if items.len() == 1
                    && fact["kind"].as_str() == Some(items[0].kind)
                    && items[0].span.line > 0
                    && items[0].span.col > 0 =>
            {
                Some(items[0])
            }
            Some(items) if items.len() > 1 => {
                ambiguous += 1;
                None
            }
            _ => None,
        };
        if let Some(decl) = matched {
            fact["location"] = json!({"file":source_file,"line":decl.span.line,"column":decl.span.col,"precision":"source-declaration"});
            fact["originalLine"] = json!(decl.span.line);
            fact["mapped"] = json!(true);
            fact["mappingReason"] = json!("unique-binding-declaration");
            mapped += 1;
        } else {
            fact["location"] = json!({"file":if safe{Some(source_file)}else{None},"line":Value::Null,"column":Value::Null,"precision":if fact["generatedLocation"]["line"].is_number(){"generated"}else{"module-only"}});
            fact["originalLine"] = Value::Null;
            fact["mapped"] = json!(false);
            fact["mappingReason"] = json!(if status != "collected" {
                status
            } else if candidate.is_some_and(|items| items.len() > 1) {
                "ambiguous-binding"
            } else if candidate.is_some() {
                "kind-mismatch"
            } else {
                "binding-unavailable"
            });
        }
    }
    let total = report["facts"].as_array().unwrap().len();
    report["sourceMapping"] = json!({"schemaVersion":1,"strategy":"unique-binding-declaration","status":status,"sourceRevision":revision,"mapped":mapped,"unmapped":total-mapped,"ambiguous":ambiguous,"truncated":status=="declaration-limit","limits":{"sourceBytes":MAX_SOURCE_BYTES,"visits":MAX_VISITS,"declarations":MAX_DECLARATIONS,"depth":MAX_DEPTH,"facts":MAX_FACTS},"qualification":"Source declaration only; originalSourceHash matched the guarded read snapshot when mapped. Instruction/source-map equivalence and later filesystem changes are not inferred."});
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new(source: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "brain-feedback-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(root.join("comp")).unwrap();
            std::fs::write(root.join("comp/test.brain"), source).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fact(name: &str, kind: &str) -> Value {
        json!({"binding":name,"kind":kind,"line":99,"column":8,"action":"removed"})
    }
    fn run(source: &str, facts: Vec<Value>) -> Value {
        let f = Fixture::new(source);
        let revision = crate::changes::read(&f.0, "comp/test.brain").unwrap()["revision"].clone();
        let mut report =
            json!({"facts":facts,"coordinateSpace":"input-js","originalSourceHash":revision});
        annotate(&f.0, "comp/test.brain", &mut report).unwrap();
        report
    }
    #[test]
    fn unique_declaration_preserves_generated_coordinates() {
        let r = run(
            "<script>\nconst unused = 1\n</script><p>OK</p>",
            vec![fact("unused", "variable")],
        );
        assert_eq!(r["facts"][0]["location"]["line"], 2);
        assert_eq!(r["facts"][0]["generatedLocation"]["line"], 99);
        assert_eq!(r["facts"][0]["location"]["precision"], "source-declaration");
        assert!(r["sourceMapping"]["sourceRevision"].is_string());
    }
    #[test]
    fn nested_shadowing_and_kind_mismatch_stay_unknown() {
        let r = run(
            "<script>const value = 1; function nested(value) { return value } const imported = 2</script><p>OK</p>",
            vec![fact("value", "variable"), fact("imported", "import")],
        );
        assert!(r["facts"][0]["originalLine"].is_null());
        assert_eq!(r["sourceMapping"]["ambiguous"], 1);
        assert!(r["facts"][1]["originalLine"].is_null());
    }
    #[test]
    fn imports_destructuring_defaults_and_named_function_expressions_are_collected() {
        let r = run(
            "<script>import root, { a as local } from './module.jsbrain'; const { item: renamed = (() => { const hidden = 1; return hidden })() } = root; const fn = function inner(arg = 1) { return arg }</script><p>OK</p>",
            vec![
                fact("root", "import"),
                fact("local", "import"),
                fact("renamed", "variable"),
                fact("hidden", "variable"),
                fact("inner", "function"),
                fact("arg", "variable"),
            ],
        );
        assert_eq!(r["sourceMapping"]["mapped"], 6, "{r}");
    }
    #[test]
    fn template_bindings_and_attribute_functions_prevent_false_unique_matches() {
        let r = run(
            "<script>const item = 1; const result = 2</script>{#each [] as item (item)}<button on:click={() => { const inside = 1 }}>OK</button>{/each}{#await pending}{:then result}<p>{result}</p>{/await}",
            vec![
                fact("item", "variable"),
                fact("result", "variable"),
                fact("inside", "variable"),
            ],
        );
        assert_eq!(r["sourceMapping"]["ambiguous"], 2);
        assert_eq!(r["sourceMapping"]["mapped"], 1, "{r}");
    }
    #[test]
    fn secret_and_unsafe_sources_never_return_body_or_invent_locations() {
        let f = Fixture::new("<script>const untouched = 1</script><p>OK</p>");
        let mut r = json!({"facts":[fact("untouched","variable")]});
        annotate(&f.0, "../private.env", &mut r).unwrap();
        assert_eq!(r["sourceMapping"]["status"], "unsafe-source");
        assert!(r["facts"][0]["location"]["file"].is_null());
        assert!(!r.to_string().contains("<script>"));
    }
    #[test]
    fn metadata_head_slots_and_nested_branches_are_visited() {
        let c = brain_parse::parse(
            "<meta seo title={(() => { const metaOnly = 1; return metaOnly })()}><script>if (true) { const branchOnly = () => { const bodyOnly = 1 } }</script><head><title>{(() => { const headOnly = 1; return headOnly })()}</title></head><Thing>{#slot slotValue}<slot item={() => { const slotOnly = 1 }} />{/slot}</Thing>",
            Path::new("views/test.brain"),
        );
        if let Ok(c) = c {
            let found = Declarations::collect(&c);
            for name in [
                "metaOnly",
                "branchOnly",
                "bodyOnly",
                "headOnly",
                "slotValue",
                "slotOnly",
            ] {
                assert!(found.names.contains_key(name), "{name}");
            }
        } else {
            panic!("fixture must parse: {c:?}");
        }
    }
    #[test]
    fn changed_source_and_missing_revision_never_relocate_old_artifacts() {
        let f = Fixture::new("<script>const same = 1</script><p>OK</p>");
        let hash = crate::changes::read(&f.0, "comp/test.brain").unwrap()["revision"].clone();
        std::fs::write(
            f.0.join("comp/test.brain"),
            "<script>\n\nconst same = 1</script><p>OK</p>",
        )
        .unwrap();
        let mut r = json!({"facts":[fact("same","variable")],"originalSourceHash":hash});
        annotate(&f.0, "comp/test.brain", &mut r).unwrap();
        assert_eq!(r["sourceMapping"]["status"], "stale-source");
        assert_eq!(r["facts"][0]["mapped"], false);
        assert!(r["facts"][0]["originalLine"].is_null());
        let mut r = json!({"facts":[fact("same","variable")]});
        annotate(&f.0, "comp/test.brain", &mut r).unwrap();
        assert_eq!(r["sourceMapping"]["status"], "source-revision-unverified");
        assert!(r["facts"][0]["originalLine"].is_null());
    }
    #[test]
    fn detected_source_secrets_and_runtime_origins_are_not_read_into_feedback() {
        let source = format!(
            "<script>const key = '{}{}{}'</script><p>OK</p>",
            "sk_",
            "live_",
            "a".repeat(32)
        );
        assert!(!brain_check::scan_secrets(&source).is_empty());
        let f = Fixture::new(&source);
        let hash = brain_pack::sha::sha256(source.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let mut r = json!({"facts":[fact("key","variable")],"originalSourceHash":hash});
        annotate(&f.0, "comp/test.brain", &mut r).unwrap();
        assert_eq!(r["sourceMapping"]["status"], "source-unavailable");
        assert_eq!(r["facts"][0]["mapped"], false);
        assert!(!r.to_string().contains(&"a".repeat(32)));
        let mut r = json!({"facts":[fact("key","variable")]});
        annotate(&f.0, "runtime/brain-data.js", &mut r).unwrap();
        assert_eq!(r["sourceMapping"]["status"], "non-app-source");
        assert!(r["facts"][0]["originalLine"].is_null());
    }
}
