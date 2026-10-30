//! Conservative native evidence. Unknown calls, mutations and escapes stay unknown.
use crate::{Ctx, Imported, security};
use brain_parse::ast::*;
use brain_parse::{Diag, Span};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
type Shape = HashMap<String, String>;
#[derive(Clone, Debug, Default)]
pub struct MethodContract {
    pub input: Option<String>,
    pub output: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct ApiContract {
    pub path: String,
    pub methods: HashMap<String, MethodContract>,
    pub schemas: HashMap<String, Shape>,
}
#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Unknown,
    Null,
    Str,
    Num,
    Numeric,
    BoolText,
    Bool,
    List(Box<Ty>),
    Object(BTreeMap<String, Ty>),
    Record,
    Api,
    Form,
    Chain(Box<Ty>),
    Func,
    Function(Box<Ty>),
    Promise(Box<Ty>),
    Maybe(Box<Ty>, u8),
}
fn maybe(ty: Ty, flags: u8) -> Ty {
    match ty {
        Ty::Unknown => Ty::Unknown,
        Ty::Maybe(t, f) => Ty::Maybe(t, f | flags),
        t => Ty::Maybe(Box::new(t), flags),
    }
}
fn solid(ty: &Ty) -> &Ty {
    match ty {
        Ty::Maybe(t, _) | Ty::Chain(t) => solid(t),
        t => t,
    }
}
fn join(a: Ty, b: Ty) -> Ty {
    if a == b {
        a
    } else {
        match (a, b) {
            (Ty::Null, t) | (t, Ty::Null) => maybe(t, 1),
            (Ty::Maybe(t, f), u) | (u, Ty::Maybe(t, f)) if *t == u => Ty::Maybe(t, f),
            _ => Ty::Unknown,
        }
    }
}
fn rule(value: &str) -> Ty {
    if value.len() > 256 {
        return Ty::Unknown;
    }
    let value = value.trim();
    if value.starts_with('[') {
        if let Some(end) = value.find(']') {
            let inner = solid(&rule(&value[1..end])).clone();
            let ty = Ty::List(Box::new(inner));
            return if value.contains('!') {
                ty
            } else {
                maybe(ty, 3)
            };
        }
        return Ty::Unknown;
    }
    let ty = match value
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("")
    {
        "str" | "email" | "url" | "date" => Ty::Str,
        "num" => Ty::Num,
        "bool" => Ty::Bool,
        "obj" => Ty::Record,
        _ => Ty::Unknown,
    };
    if value.contains('!') {
        ty
    } else {
        maybe(ty, 3)
    }
}
fn key(key: &PropKey) -> Option<&str> {
    match key {
        PropKey::Ident(i) => Some(&i.name),
        PropKey::Str { value, .. } => Some(value),
        _ => None,
    }
}
fn object(e: &Expr) -> Option<BTreeMap<String, &Expr>> {
    let Expr::Object { props, .. } = e.unparen() else {
        return None;
    };
    if props.len() > 128 {
        return None;
    }
    let mut out = BTreeMap::new();
    for p in props {
        let ObjProp::KeyValue { key: k, value, .. } = p else {
            return None;
        };
        if out.insert(key(k)?.to_owned(), value).is_some() {
            return None;
        }
    }
    Some(out)
}
fn decl(s: &Stmt) -> &Stmt {
    if let Stmt::Export { decl, .. } = s {
        decl
    } else {
        s
    }
}
fn declarations(c: &Component) -> Vec<(&str, &Expr)> {
    c.script
        .iter()
        .filter_map(|s| {
            if let Stmt::Var(v) = decl(s) {
                Some(v)
            } else {
                None
            }
        })
        .flat_map(|v| &v.decls)
        .filter_map(|d| match (&d.pat, &d.init) {
            (Pat::Ident(i), Some(e)) => Some((i.name.as_str(), e)),
            _ => None,
        })
        .collect()
}
pub(crate) fn shapes(c: &Component) -> HashMap<String, Shape> {
    declarations(c)
        .into_iter()
        .filter_map(|(name, e)| {
            let values = object(e)?;
            if values.len() > 64 {
                return None;
            }
            let shape: Option<Shape> = values
                .into_iter()
                .map(|(k, v)| match v.unparen() {
                    Expr::Str { value, .. } if rule(value) != Ty::Unknown => {
                        Some((k, value.clone()))
                    }
                    _ => None,
                })
                .collect();
            Some((name.to_owned(), shape?))
        })
        .take(128)
        .collect()
}
fn api_contract(e: &Expr, schemas: HashMap<String, Shape>) -> Option<ApiContract> {
    let Expr::Call {
        callee,
        args,
        optional: false,
        ..
    } = e.unparen()
    else {
        return None;
    };
    if !matches!(callee.unparen(),Expr::Ident(i)if i.name=="api") {
        return None;
    }
    let Expr::Str { value: path, .. } = args.first()?.unparen() else {
        return None;
    };
    if path.len() > 2048 {
        return None;
    }
    let configs = object(args.get(1)?)?;
    if configs.len() > 32 {
        return None;
    }
    let mut methods = HashMap::new();
    for (name, config) in configs {
        let fields = object(config)?;
        let text = |key: &str| {
            fields.get(key).and_then(|e| {
                if let Expr::Str { value, .. } = e.unparen() {
                    Some(if value.len() <= 256 {
                        value.clone()
                    } else {
                        String::new()
                    })
                } else {
                    Some(String::new())
                }
            })
        };
        methods.insert(
            name,
            MethodContract {
                input: text("in"),
                output: text("out"),
            },
        );
    }
    Some(ApiContract {
        path: path.clone(),
        methods,
        schemas,
    })
}
pub(crate) fn module_api(c: &Component) -> Option<ApiContract> {
    let mut unsafe_flow = false;
    security::walk(&c.script, false, &mut |e, _, _| match e {
        Expr::Assign { .. } | Expr::Update { .. } => unsafe_flow = true,
        Expr::Call { callee, .. } if !matches!(callee.unparen(),Expr::Ident(i)if i.name=="api") => {
            unsafe_flow = true
        }
        _ => {}
    });
    if unsafe_flow {
        return None;
    }
    if crate::scope::top_bindings(c).contains_key("api") {
        return None;
    }
    let values = declarations(c);
    let mut exported = c.script.iter().filter_map(|s| {
        if let Stmt::ExportDefault { expr, .. } = s {
            Some(expr)
        } else {
            None
        }
    });
    let e = exported.next()?;
    if exported.next().is_some() {
        return None;
    }
    let e = if let Expr::Ident(i) = e.unparen() {
        values.iter().find(|(n, _)| *n == i.name)?.1
    } else {
        e
    };
    api_contract(e, shapes(c))
}
fn lexical(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    absolute
        .components()
        .try_fold(PathBuf::new(), |mut out, part| {
            match part {
                std::path::Component::ParentDir => {
                    if !out.pop() {
                        return None;
                    }
                }
                std::path::Component::CurDir => {}
                part => out.push(part.as_os_str()),
            };
            Some(out)
        })
}
fn module_location(root: Option<&Path>, path: &Path) -> Option<(PathBuf, String)> {
    if let Some(root) = root {
        let base = lexical(root)?;
        let file = lexical(path)?;
        let relative = file.strip_prefix(&base).ok()?.to_str()?.replace('\\', "/");
        if relative.len() > 2048 || relative.split('/').count() > 32 {
            return None;
        }
        Some((base.canonicalize().ok()?, relative))
    } else {
        Some((
            path.parent()?.canonicalize().ok()?,
            path.file_name()?.to_str()?.to_owned(),
        ))
    }
}
/// Standalone fixtures/components still have a project-relative source identity.
/// Derive its boundary rather than treating an arbitrary import parent as a root.
pub(crate) fn component_root(disk: &Path, logical: &str) -> Option<PathBuf> {
    let relative=Path::new(logical);
    if relative.is_absolute() || logical.contains('\\') || relative.components().count()>32 || !relative.components().all(|part|matches!(part,std::path::Component::Normal(_))) {return None;}
    let mut path=lexical(disk)?;
    if !path.ends_with(relative) {return None;}
    for _ in relative.components() {path.pop();}
    let canonical=path.canonicalize().ok()?;
    // Keep the source spelling for relative-path checks (/var versus /private/var
    // on macOS); the guarded reader canonicalizes the root before opening files.
    canonical.parent().is_some().then_some(path)
}
pub(crate) fn read_module(root: Option<&Path>, path: &Path) -> Option<Component> {
    let (base, relative) = module_location(root, path)?;
    let bytes = crate::remote_imports::read(&base, &relative, 256 * 1024)?;
    let source = String::from_utf8(bytes).ok()?;
    brain_parse::parse(&source, Path::new(&relative)).ok()
}
pub(crate) fn shared_types(root: &Path) -> (Option<HashSet<String>>, HashMap<String, Shape>) {
    let Some(module) = read_module(Some(root), &root.join("types.jsbrain")) else {
        return (None, HashMap::new());
    };
    let mut names = HashSet::new();
    for statement in &module.script {
        if let Stmt::Export { decl, .. } = statement {
            match decl.as_ref() {
                Stmt::Var(v) => {
                    for d in &v.decls {
                        if let Pat::Ident(i) = &d.pat {
                            names.insert(i.name.clone());
                        }
                    }
                }
                Stmt::Func(f) => {
                    if let Some(name) = &f.name {
                        names.insert(name.name.clone());
                    }
                }
                _ => {}
            }
        }
    }
    (Some(names), shapes(&module))
}
pub(crate) fn watched_module(root: Option<&Path>, path: &Path) -> PathBuf {
    let observed = module_location(root, path)
        .map(|(base, relative)| crate::remote_imports::watched(&base, &relative))
        .unwrap_or_else(|| {
            root.unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")))
                .to_path_buf()
        });
    if std::fs::symlink_metadata(&observed).is_ok_and(|m| !m.is_file() && !m.is_dir()) {
        observed.parent().unwrap_or(&observed).to_path_buf()
    } else {
        observed
    }
}

fn observe(root: &Path, path: &Path, deps: &mut Vec<PathBuf>) {
    let mut current = path;
    for _ in 0..32 {
        deps.push(watched_module(Some(root), current));
        if current == root {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if !parent.starts_with(root) {
            break;
        }
        current = parent;
    }
}
pub(crate) fn module_hint_safe(root: Option<&Path>, path: &Path) -> bool {
    let Some((base, relative)) = module_location(root, path) else {
        return false;
    };
    std::fs::symlink_metadata(base.join(&relative))
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        && crate::remote_imports::watched(&base, &relative) == base.join(&relative)
}
/// Read only directly imported API modules and resources referenced by a literal API path.
/// Every observed path participates in normal context invalidation, including absent schema files.
pub(crate) fn load_contracts(
    c: &Component,
    ctx: &mut Ctx,
    root: &Path,
    dir: &Path,
    deps: &mut Vec<PathBuf>,
) {
    fn read(path: &Path, root: &Path, budget: &mut u64) -> Option<Component> {
        let (base, relative) = module_location(Some(root), path)?;
        let bytes =
            crate::remote_imports::read(&base, &relative, (*budget).min(256 * 1024) as usize)?;
        *budget -= bytes.len() as u64;
        let source = String::from_utf8(bytes).ok()?;
        brain_parse::parse(&source, Path::new(&relative)).ok()
    }
    fn root_types(path: &Path, root: &Path) -> bool {
        let Ok(expected) = std::fs::canonicalize(root.join("types.jsbrain")) else {
            return false;
        };
        std::fs::canonicalize(path)
            .ok()
            .is_some_and(|actual| actual == expected)
    }
    let mut budget = 8 * 1024 * 1024;
    let mut paths = Vec::new();
    for s in c.script.iter().take(128) {
        if let Stmt::Import(i) = s {
            if i.source.ends_with(".jsbrain")
                && (i.source.starts_with("./") || i.source.starts_with("../"))
            {
                let path = dir.join(&i.source);
                observe(root, &path, deps);
                if let Some(module) = read(&path, root, &mut budget) {
                    if root_types(&path, root) {
                        for (original, local) in &i.named {
                            if let Some(shape) = ctx.type_shapes.get(&original.name) {
                                ctx.schema_aliases.insert(local.name.clone(), shape.clone());
                            }
                        }
                    }
                    if let Some(mut contract) = module_api(&module) {
                        for stmt in &module.script {
                            if let Stmt::Import(import) = stmt {
                                let imported = path.parent().unwrap_or(root).join(&import.source);
                                if import.source.ends_with(".jsbrain")
                                    && read(&imported, root, &mut budget).is_some()
                                    && root_types(&imported, root)
                                {
                                    observe(root, &imported, deps);
                                    for (original, local) in &import.named {
                                        if let Some(shape) = ctx.type_shapes.get(&original.name) {
                                            contract
                                                .schemas
                                                .insert(local.name.clone(), shape.clone());
                                        }
                                    }
                                }
                            }
                        }
                        paths.push(contract.path.clone());
                        ctx.api_contracts.insert(i.source.clone(), contract);
                    }
                }
            }
        }
    }
    if !crate::scope::top_bindings(c).contains_key("api") {
        for (_, e) in declarations(c) {
            if let Some(contract) = api_contract(e, HashMap::new()) {
                paths.push(contract.path);
            }
        }
    }
    paths.sort();
    paths.dedup();
    for path in paths.into_iter().take(128) {
        let Some(name) = path.strip_prefix("/api/").and_then(|p| p.split('/').next()) else {
            continue;
        };
        if name.is_empty()
            || !name.as_bytes()[0].is_ascii_lowercase()
            || matches!(
                name,
                "constructor"
                    | "prototype"
                    | "__proto__"
                    | "id"
                    | "owner"
                    | "created_at"
                    | "updated_at"
                    | "search"
            )
            || name.len() > 48
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            continue;
        }
        let schema = root
            .join("server/resources")
            .join(name)
            .join("schema.jsbrain");
        observe(root, &schema, deps);
        if let Some(module) = read(&schema, root, &mut budget) {
            if let Some(Stmt::ExportDefault { expr, .. }) = module
                .script
                .iter()
                .find(|s| matches!(s, Stmt::ExportDefault { .. }))
            {
                if let Some(fields) = object(expr) {
                    let shape: Option<Shape> = fields
                        .into_iter()
                        .map(|(k, v)| {
                            if let Expr::Str { value, .. } = v.unparen() {
                                Some((k, value.clone()))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if let Some(shape) = shape.filter(|s| {
                        !s.is_empty()
                            && s.len() <= 64
                            && s.iter().all(|(k, v)| {
                                k.len() <= 48
                                    && k.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                                    && k.bytes().all(|b| {
                                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'
                                    })
                                    && !matches!(
                                        k.as_str(),
                                        "constructor"
                                            | "prototype"
                                            | "__proto__"
                                            | "id"
                                            | "owner"
                                            | "created_at"
                                            | "updated_at"
                                            | "search"
                                    )
                                    && matches!(
                                        v.trim_end_matches('!'),
                                        "str" | "email" | "num" | "bool" | "obj"
                                    )
                            })
                    }) {
                        ctx.resource_shapes.insert(name.to_owned(), shape);
                    }
                }
            }
        }
    }
}

fn incompatible(expected: &Ty, actual: &Ty) -> bool {
    match (solid(expected), solid(actual)) {
        (Ty::Unknown, _) | (_, Ty::Unknown) => false,
        (Ty::Numeric, Ty::Num | Ty::Str | Ty::Numeric) | (Ty::Num | Ty::Str, Ty::Numeric) => false,
        (Ty::BoolText, Ty::Bool | Ty::Str | Ty::BoolText) | (Ty::Bool | Ty::Str, Ty::BoolText) => {
            false
        }
        (Ty::List(w), Ty::List(a)) => incompatible(w, a),
        (
            Ty::Object(_) | Ty::Record | Ty::Api | Ty::Form,
            Ty::Object(_) | Ty::Record | Ty::Api | Ty::Form,
        )
        | (Ty::Func | Ty::Function(_), Ty::Func | Ty::Function(_)) => false,
        (a, b) => a != b,
    }
}
fn label(ty: &Ty) -> &'static str {
    match solid(ty) {
        Ty::Unknown => "unknown",
        Ty::Null => "null",
        Ty::Str => "str",
        Ty::Num | Ty::Numeric => "num",
        Ty::Bool | Ty::BoolText => "bool",
        Ty::List(_) => "list",
        Ty::Object(_) | Ty::Record | Ty::Api | Ty::Form => "obj",
        Ty::Func | Ty::Function(_) => "fn",
        Ty::Promise(_) => "promise",
        Ty::Maybe(..) | Ty::Chain(_) => "nullable value",
    }
}
fn prop(ty: PropType) -> Ty {
    match ty {
        PropType::Str => Ty::Str,
        PropType::Num => Ty::Num,
        PropType::Bool => Ty::Bool,
        PropType::List => Ty::List(Box::new(Ty::Unknown)),
        PropType::Obj => Ty::Record,
        PropType::Fn => Ty::Func,
        PropType::Any => Ty::Unknown,
    }
}
fn materialize(ty: Ty) -> Ty {
    match ty {
        Ty::Chain(t) => maybe(*t, 2),
        t => t,
    }
}
fn underlying(ty: &Ty) -> &Ty {
    match ty {
        Ty::Maybe(t, _) | Ty::Chain(t) => underlying(t),
        t => t,
    }
}
fn nullable(ty: &Ty) -> bool {
    match ty {
        Ty::Maybe(..) | Ty::Null => true,
        Ty::Chain(t) => nullable(t),
        _ => false,
    }
}
fn root(e: &Expr) -> Option<&str> {
    match e.unparen() {
        Expr::Ident(i) => Some(&i.name),
        Expr::Member { obj, .. } => root(obj),
        _ => None,
    }
}
fn escape(e: &Expr, out: &mut HashSet<String>) {
    match e.unparen() {
        Expr::Ident(_) | Expr::Member { .. } => {
            if let Some(n) = root(e) {
                out.insert(n.to_owned());
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    ObjProp::KeyValue { value, .. } => escape(value, out),
                    ObjProp::Spread { arg, .. } => escape(arg, out),
                    ObjProp::Shorthand(i) => {
                        out.insert(i.name.clone());
                    }
                    _ => {}
                }
            }
        }
        Expr::Array { elems, .. } => {
            for e in elems.iter().flatten() {
                escape(e, out)
            }
        }
        Expr::Cond { cons, alt, .. } => {
            escape(cons, out);
            escape(alt, out)
        }
        Expr::Spread { arg, .. } => escape(arg, out),
        Expr::Func(f) => {
            if let FnBody::Expr(e) = &f.body {
                escape(e, out)
            }
        }
        _ => {}
    }
}
// Generic API validation accepts numeric strings without coercing the JSON value.
fn output_ty(ty: Ty) -> Ty {
    match ty {
        Ty::Num => Ty::Numeric,
        Ty::Record => Ty::Unknown,
        Ty::Object(fields) => {
            Ty::Object(fields.into_iter().map(|(k, v)| (k, output_ty(v))).collect())
        }
        Ty::List(t) => Ty::List(Box::new(output_ty(*t))),
        Ty::Maybe(t, f) if *t == Ty::Bool => maybe(Ty::BoolText, f),
        Ty::Maybe(t, f) => maybe(output_ty(*t), f),
        t => t,
    }
}
fn fields_ty(shape: &Shape) -> Ty {
    Ty::Object(shape.iter().map(|(k, v)| (k.clone(), rule(v))).collect())
}
fn field_span(body: &Expr, name: &str) -> Option<Span> {
    let Expr::Object { props, .. } = body.unparen() else {
        return None;
    };
    props.iter().find_map(|p| match p {
        ObjProp::KeyValue {
            key: PropKey::Ident(i),
            ..
        } if i.name == name => Some(i.span),
        _ => None,
    })
}
fn closest(name: &str, fields: impl Iterator<Item = String>) -> Option<String> {
    let mut c: Vec<_> = fields
        .map(|f| (distance(name, &f), f))
        .filter(|(d, _)| *d <= 2)
        .collect();
    c.sort();
    c.first()
        .filter(|a| c.get(1).is_none_or(|b| a.0 < b.0))
        .map(|a| a.1.clone())
}
#[derive(Clone)]
struct Endpoint {
    input: Option<Shape>,
    output: Ty,
    body: bool,
}
struct Analysis<'a> {
    c: &'a Component,
    ctx: &'a Ctx,
    env: HashMap<String, Ty>,
    schemas: HashMap<String, Shape>,
    forms: HashMap<String, Shape>,
    apis: HashMap<String, HashMap<String, Endpoint>>,
    shadowed: HashSet<String>,
    diagnostics: Vec<Diag>,
    poisoned: HashSet<String>,
}
impl Analysis<'_> {
    fn push(&mut self, code: &'static str, span: Span, msg: String, fix: String) -> bool {
        if self.diagnostics.len() >= 128 {
            return false;
        }
        let mut d = Diag::new(code, &self.c.file, span, msg, fix);
        d.level = brain_parse::Severity::Warning;
        d.locate(&self.c.src);
        self.diagnostics.push(d);
        true
    }
    fn edit_last(&mut self, span: Span, text: String) {
        if let Some(d) = self.diagnostics.last_mut() {
            *d = d.clone().edit(span.start, span.end, text);
            d.locate(&self.c.src)
        }
    }
    fn schema(&self, e: &Expr) -> Option<Shape> {
        match e.unparen() {
            Expr::Ident(i) => self.schemas.get(&i.name).cloned(),
            Expr::Object { .. } => object(e)?
                .into_iter()
                .map(|(k, v)| match v.unparen() {
                    Expr::Str { value, .. } if rule(value) != Ty::Unknown => {
                        Some((k, value.clone()))
                    }
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }
    fn schema_type(&self, name: &str, schemas: &HashMap<String, Shape>) -> Ty {
        if let Some((name, list)) = crate::types::type_ref(name) {
            let t = schemas
                .get(&name)
                .map(|s| output_ty(fields_ty(s)))
                .unwrap_or(Ty::Unknown);
            if list { Ty::List(Box::new(t)) } else { t }
        } else {
            output_ty(rule(name))
        }
    }
    fn register_api(&mut self, name: &str, contract: ApiContract) {
        let mut schemas = self.schemas.clone();
        schemas.extend(contract.schemas);
        let mut methods = HashMap::new();
        let native = contract
            .path
            .strip_prefix("/api/")
            .and_then(|p| p.split('/').next())
            .and_then(|n| self.ctx.resource_shapes.get(n));
        for (method, config) in contract.methods {
            let lower = method.to_ascii_lowercase();
            let input = config
                .input
                .as_ref()
                .and_then(|n| schemas.get(n))
                .cloned()
                .or_else(|| {
                    if matches!(lower.as_str(), "post" | "put") {
                        native.cloned()
                    } else {
                        None
                    }
                });
            let output = config
                .output
                .as_ref()
                .map(|s| self.schema_type(s, &schemas))
                .unwrap_or_else(|| {
                    let Some(shape) = native else {
                        return Ty::Unknown;
                    };
                    let row = Ty::Object(BTreeMap::from([
                        ("id".into(), Ty::Str),
                        ("owner".into(), Ty::Str),
                        ("data".into(), fields_ty(shape)),
                    ]));
                    if lower == "get" {
                        if contract.path.trim_end_matches('/').split('/').count() == 3 {
                            Ty::Object(BTreeMap::from([
                                ("items".into(), Ty::List(Box::new(row))),
                                ("limit".into(), Ty::Num),
                                ("offset".into(), Ty::Num),
                            ]))
                        } else if contract.path.split('/').count() == 4 {
                            row
                        } else {
                            Ty::Unknown
                        }
                    } else if matches!(lower.as_str(), "post" | "put") {
                        row
                    } else {
                        Ty::Unknown
                    }
                });
            methods.insert(
                method,
                Endpoint {
                    input,
                    output,
                    body: !contract.path.contains(':') && !matches!(lower.as_str(), "get" | "head"),
                },
            );
        }
        self.apis.insert(name.to_owned(), methods);
        self.env.insert(name.to_owned(), Ty::Api);
    }
    fn value(&self, e: &Expr) -> Ty {
        self.value_in(e, &self.env, 0)
    }
    fn value_in(&self, e: &Expr, env: &HashMap<String, Ty>, depth: usize) -> Ty {
        if depth > 32 {
            return Ty::Unknown;
        }
        let nested = |e: &Expr| self.value_in(e, env, depth + 1);
        if let Expr::Paren { expr, .. } = e {
            return materialize(nested(expr));
        }
        match e.unparen() {
            Expr::Null(_) => Ty::Null,
            Expr::Str { .. } | Expr::Template { tag: None, .. } => Ty::Str,
            Expr::Num { .. } => Ty::Num,
            Expr::Bool { .. } => Ty::Bool,
            Expr::Ident(i) => match env.get(&i.name) {
                Some(Ty::Api) => self
                    .apis
                    .get(&i.name)
                    .map(|methods| {
                        Ty::Object(
                            methods
                                .iter()
                                .map(|(k, v)| {
                                    (
                                        k.clone(),
                                        Ty::Function(Box::new(Ty::Promise(Box::new(
                                            v.output.clone(),
                                        )))),
                                    )
                                })
                                .collect(),
                        )
                    })
                    .unwrap_or(Ty::Record),
                Some(Ty::Form) => Ty::Record,
                Some(t) => t.clone(),
                None => Ty::Unknown,
            },
            Expr::Array { elems, .. } => {
                let mut ts = elems.iter().flatten().map(nested);
                let first = ts.next().unwrap_or(Ty::Unknown);
                let ty = if ts.all(|t| t == first) {
                    first
                } else {
                    Ty::Unknown
                };
                Ty::List(Box::new(ty))
            }
            Expr::Object { .. } => object(e)
                .map(|o| {
                    Ty::Object(
                        o.into_iter()
                            .map(|(k, v)| (k, materialize(nested(v))))
                            .collect(),
                    )
                })
                .unwrap_or(Ty::Unknown),
            Expr::Func(f) => {
                if f.params.is_empty() {
                    Ty::Function(Box::new(self.function_value(f, env, depth + 1)))
                } else {
                    Ty::Func
                }
            }
            Expr::Await { arg, .. } => match materialize(nested(arg)) {
                Ty::Promise(t) => *t,
                _ => Ty::Unknown,
            },
            Expr::Member {
                obj,
                prop: MemberProp::Name(p),
                optional,
                ..
            } => {
                if let Expr::Ident(i) = obj.unparen() {
                    if p.name == "v" && matches!(env.get(&i.name), Some(Ty::Form)) {
                        if let Some(shape) = self.forms.get(&i.name) {
                            return Ty::Object(
                                shape.keys().map(|k| (k.clone(), Ty::Unknown)).collect(),
                            );
                        }
                    }
                }
                let receiver = nested(obj);
                let value = match underlying(&receiver) {
                    Ty::Object(fields) => fields.get(&p.name).cloned().unwrap_or(Ty::Unknown),
                    Ty::List(_) | Ty::Str if p.name == "length" => Ty::Num,
                    _ => Ty::Unknown,
                };
                if (*optional && nullable(&receiver)) || matches!(receiver, Ty::Chain(_)) {
                    Ty::Chain(Box::new(value))
                } else {
                    value
                }
            }
            Expr::Unary { op: "!", .. } => Ty::Bool,
            Expr::Unary {
                op: "+" | "-", arg, ..
            } if nested(arg) == Ty::Num => Ty::Num,
            Expr::Cond {
                test, cons, alt, ..
            } => {
                let mut yes = env.clone();
                narrow(test, true, &mut yes);
                let mut no = env.clone();
                narrow(test, false, &mut no);
                join(
                    self.value_in(cons, &yes, depth + 1),
                    self.value_in(alt, &no, depth + 1),
                )
            }
            Expr::Binary {
                op: "??",
                left,
                right,
                ..
            } => {
                let a = nested(left);
                match a {
                    Ty::Maybe(t, _) => join(*t, nested(right)),
                    Ty::Null => nested(right),
                    t => t,
                }
            }
            Expr::Binary {
                op: "+" | "-" | "*" | "/" | "%",
                left,
                right,
                ..
            } if nested(left) == Ty::Num && nested(right) == Ty::Num => Ty::Num,
            Expr::Call {
                callee,
                args,
                optional: false,
                ..
            } => {
                if let Expr::Ident(i) = callee.unparen() {
                    if !self.shadowed.contains(&i.name) {
                        match i.name.as_str() {
                            "api" => return Ty::Api,
                            "form" => return Ty::Form,
                            "sig" => return args.first().map(nested).unwrap_or(Ty::Unknown),
                            "drv" => {
                                if let Some(Expr::Func(f)) = args.first().map(Expr::unparen) {
                                    if f.params.is_empty() && !f.is_async {
                                        return self.function_value(f, env, depth + 1);
                                    }
                                }
                                return Ty::Unknown;
                            }
                            "resource" => {
                                let ty = args.first().map(nested).unwrap_or(Ty::Unknown);
                                return match ty {
                                    Ty::Function(t) => match *t {
                                        Ty::Promise(t) => Ty::Promise(t),
                                        v => Ty::Promise(Box::new(v)),
                                    },
                                    _ => Ty::Unknown,
                                };
                            }
                            _ => {}
                        }
                    }
                }
                match nested(callee) {
                    Ty::Function(t) => *t,
                    _ => Ty::Unknown,
                }
            }
            _ => Ty::Unknown,
        }
    }
    fn function_value(&self, f: &Func, env: &HashMap<String, Ty>, depth: usize) -> Ty {
        if depth > 32 || !f.params.is_empty() {
            return Ty::Unknown;
        }
        let result = match &f.body {
            FnBody::Expr(e) => self.value_in(e, env, depth + 1),
            FnBody::Block(stmts) => {
                let mut local = env.clone();
                let mut result = Ty::Unknown;
                for s in stmts {
                    match decl(s) {
                        Stmt::Var(v) => {
                            for d in &v.decls {
                                if let (Pat::Ident(i), Some(e)) = (&d.pat, &d.init) {
                                    local.insert(
                                        i.name.clone(),
                                        materialize(self.value_in(e, &local, depth + 1)),
                                    );
                                } else {
                                    return Ty::Unknown;
                                }
                            }
                        }
                        Stmt::Return { arg: Some(e), .. } => {
                            result = self.value_in(e, &local, depth + 1);
                            break;
                        }
                        _ => return Ty::Unknown,
                    }
                }
                result
            }
        };
        if f.is_async && !matches!(result, Ty::Promise(_) | Ty::Unknown) {
            Ty::Promise(Box::new(result))
        } else {
            result
        }
    }
    fn expression(&mut self, e: &Expr) {
        if self.diagnostics.len() >= 128 {
            return;
        }
        match e.unparen() {
            Expr::Member {
                obj,
                prop,
                optional,
                ..
            } => {
                self.expression(obj);
                if let MemberProp::Computed(e) = prop {
                    self.expression(e)
                }
                let receiver = self.value(obj);
                if !*optional && nullable(&receiver) {
                    self.push(
                        "B2005",
                        obj.span(),
                        "nullable value is dereferenced without a proven check".into(),
                        "guard this value with `if (value != null)` or use optional chaining"
                            .into(),
                    );
                }
                if let (MemberProp::Name(p), Ty::Object(fields)) = (prop, underlying(&receiver)) {
                    if !fields.contains_key(&p.name)
                        && ![
                            "toString",
                            "toLocaleString",
                            "valueOf",
                            "hasOwnProperty",
                            "isPrototypeOf",
                            "propertyIsEnumerable",
                            "constructor",
                            "__proto__",
                        ]
                        .contains(&p.name.as_str())
                    {
                        let best = closest(&p.name, fields.keys().cloned());
                        if self.push(
                            "B2002",
                            p.span,
                            format!("known object has no field `{}`", p.name),
                            best.as_ref()
                                .map(|s| format!("use `{s}`"))
                                .unwrap_or_else(|| {
                                    "use a declared field or keep the receiver dynamic".into()
                                }),
                        ) {
                            if let Some(best) = best {
                                self.edit_last(p.span, best)
                            }
                        }
                    }
                }
            }
            Expr::Binary {
                op: "&&" | "||" | "??",
                left,
                right,
                ..
            } => {
                self.expression(left);
                let saved = self.env.clone();
                match e.unparen() {
                    Expr::Binary { op: "&&", .. } => narrow(left, true, &mut self.env),
                    Expr::Binary { op: "||", .. } => narrow(left, false, &mut self.env),
                    _ => {}
                }
                self.expression(right);
                self.env = saved;
            }
            Expr::Cond {
                test, cons, alt, ..
            } => {
                self.expression(test);
                let saved = self.env.clone();
                narrow(test, true, &mut self.env);
                self.expression(cons);
                self.env = saved.clone();
                narrow(test, false, &mut self.env);
                self.expression(alt);
                self.env = saved;
            }
            Expr::Call {
                callee,
                args,
                optional: false,
                ..
            } => {
                self.expression(callee);
                if let Expr::Member {
                    obj,
                    prop: MemberProp::Name(method),
                    ..
                } = callee.unparen()
                {
                    if let Expr::Ident(api) = obj.unparen() {
                        if !matches!(self.env.get(&api.name), Some(Ty::Api)) {
                            for arg in args {
                                self.expression(arg)
                            }
                            return;
                        }
                        if let Some(endpoint) = self
                            .apis
                            .get(&api.name)
                            .and_then(|m| m.get(&method.name))
                            .cloned()
                        {
                            if endpoint.body {
                                if let (Some(body), Some(shape)) =
                                    (args.first(), endpoint.input.as_ref())
                                {
                                    self.body(body, shape, "B2003")
                                }
                            }
                        }
                    }
                }
                for arg in args {
                    self.expression(arg)
                }
            }
            Expr::Func(f) => {
                let saved = self.env.clone();
                for p in &f.params {
                    self.bind(p, Ty::Unknown)
                }
                match &f.body {
                    FnBody::Expr(e) => self.expression(e),
                    FnBody::Block(body) => {
                        let poisoned = self.poisoned.clone();
                        self.stmts(body, &poisoned)
                    }
                }
                self.env = saved;
            }
            _ => {
                // Descend one AST level so guarded subexpressions retain their own facts.
                match e.unparen() {
                    Expr::Array { elems, .. } => {
                        for e in elems.iter().flatten() {
                            self.expression(e)
                        }
                    }
                    Expr::Object { props, .. } => {
                        for p in props {
                            match p {
                                ObjProp::KeyValue { value, .. } => self.expression(value),
                                ObjProp::Spread { arg, .. } => self.expression(arg),
                                _ => {}
                            }
                        }
                    }
                    Expr::Unary { arg, .. }
                    | Expr::Await { arg, .. }
                    | Expr::Spread { arg, .. } => self.expression(arg),
                    Expr::Binary { left, right, .. } => {
                        self.expression(left);
                        self.expression(right)
                    }
                    Expr::Template { exprs, .. } | Expr::Seq { exprs, .. } => {
                        for e in exprs {
                            self.expression(e)
                        }
                    }
                    Expr::Assign { value, .. } => self.expression(value),
                    _ => {}
                }
            }
        }
    }
    fn body(&mut self, body: &Expr, shape: &Shape, code: &'static str) {
        let Some(fields) = object(body) else { return };
        for (name, value) in &fields {
            let Some(expected) = shape.get(name) else {
                let best = closest(
                    name,
                    shape.keys().filter(|k| !fields.contains_key(*k)).cloned(),
                );
                let span = field_span(body, name).unwrap_or(value.span());
                if self.push(
                    code,
                    span,
                    format!("schema has no field `{name}`"),
                    best.as_ref()
                        .map(|s| format!("use `{s}`"))
                        .unwrap_or_else(|| {
                            "remove the undeclared field or update the shared schema".into()
                        }),
                ) {
                    if let (Some(best), Some(span)) = (best, field_span(body, name)) {
                        self.edit_last(span, best)
                    }
                }
                continue;
            };
            let expected = rule(expected);
            let actual = self.value(value);
            if actual == Ty::Null && (matches!(expected, Ty::Maybe(..)) || code == "B2004") {
                continue;
            }
            if matches!(expected,Ty::Maybe(ref t,_)if !matches!(t.as_ref(),Ty::List(_)))
                && matches!(value.unparen(),Expr::Str{value,..}if value.trim().is_empty())
            {
                continue;
            }
            if code == "B2004"
                && matches!(value.unparen(),Expr::Str{value,..}if value.trim().is_empty())
            {
                continue;
            }
            if matches!(solid(&expected), Ty::Num)
                && matches!(value.unparen(),Expr::Str{value,..}if value.trim().parse::<f64>().is_ok_and(f64::is_finite))
            {
                continue;
            }
            if matches!(solid(&expected), Ty::Num)
                && actual == Ty::Str
                && !matches!(value.unparen(), Expr::Str { .. })
            {
                continue;
            }
            if let (Ty::List(w), Ty::List(a)) = (solid(&expected), solid(&actual)) {
                if matches!(solid(w), Ty::Num) && solid(a) == &Ty::Str {
                    if let Expr::Array { elems, .. } = value.unparen() {
                        if elems.iter().flatten().all(|e|matches!(e.unparen(),Expr::Str{value,..}if value.trim().parse::<f64>().is_ok_and(f64::is_finite))){continue}
                    } else {
                        continue;
                    }
                }
            }
            if incompatible(&expected, &actual) {
                self.push(
                    code,
                    value.span(),
                    format!(
                        "field `{name}` expects {}, received {}",
                        label(&expected),
                        label(&actual)
                    ),
                    format!("pass a {} value for `{name}`", label(&expected)),
                );
            }
        }
        for (name, rule) in shape {
            if code == "B2003" && rule.contains('!') && !fields.contains_key(name) {
                self.push(
                    code,
                    body.span(),
                    format!("required schema field `{name}` is missing"),
                    format!("supply `{name}` from the shared schema"),
                );
            }
        }
    }
    fn attrs(&mut self, attrs: &[Attr], expected: Option<&[Prop]>, tag: &str) {
        for a in attrs {
            match &a.value {
                AttrValue::Expr(e) => self.expression(e),
                AttrValue::Mixed(parts) => {
                    for part in parts {
                        if let AttrPart::Expr(e) = part {
                            self.expression(e)
                        }
                    }
                }
                _ => {}
            }
            let actual = match &a.value {
                AttrValue::Expr(e) => self.value(e),
                AttrValue::Text(_) | AttrValue::Mixed(_) => Ty::Str,
                AttrValue::None => Ty::Bool,
            };
            if matches!(a.kind, AttrKind::Plain | AttrKind::Bind) {
                if let Some(p) = expected.and_then(|ps| ps.iter().find(|p| p.name == a.name)) {
                    let want = prop(p.ty);
                    if actual == Ty::Null && p.default.is_some() {
                        continue;
                    }
                    if incompatible(&want, &actual) {
                        if self.push(
                            "B2001",
                            a.span,
                            format!(
                                "prop `{}` expects {}, received {}",
                                a.name,
                                label(&want),
                                label(&actual)
                            ),
                            format!("pass a {} value for `{}`", label(&want), a.name),
                        ) {
                            if a.kind == AttrKind::Plain && want == Ty::Num {
                                if let AttrValue::Text(text) = &a.value {
                                    if text
                                        .parse::<f64>()
                                        .is_ok_and(|n| n.is_finite() && n.to_string() == *text)
                                    {
                                        self.edit_last(a.span, format!("{}={{{text}}}", a.name));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if a.kind == AttrKind::Bind
                && matches!(tag, "input" | "select" | "textarea")
                && matches!(a.name.as_str(), "checked" | "value")
            {
                let AttrValue::Expr(e) = &a.value else {
                    continue;
                };
                let Expr::Member {
                    obj,
                    prop: MemberProp::Name(field),
                    ..
                } = e.unparen()
                else {
                    continue;
                };
                let Expr::Member {
                    obj: form,
                    prop: MemberProp::Name(v),
                    ..
                } = obj.unparen()
                else {
                    continue;
                };
                let Expr::Ident(form) = form.unparen() else {
                    continue;
                };
                if v.name != "v" || !matches!(self.env.get(&form.name), Some(Ty::Form)) {
                    continue;
                }
                let Some(declared) = self.forms.get(&form.name).and_then(|f| f.get(&field.name))
                else {
                    continue;
                };
                let ty = rule(declared);
                if (a.name == "checked" && solid(&ty) != &Ty::Bool)
                    || (a.name == "value" && solid(&ty) == &Ty::Bool)
                {
                    self.push("B2004",a.span,format!("bind:{} conflicts with form field `{}` ({})",a.name,field.name,label(&ty)),"use bind:checked for boolean fields and bind:value for scalar value fields".into());
                }
            }
        }
    }
    fn bind(&mut self, p: &Pat, ty: Ty) {
        match p {
            Pat::Ident(i) => {
                self.env.insert(i.name.clone(), materialize(ty));
            }
            Pat::Object { props, rest, .. } => {
                for p in props {
                    let field = key(&p.key)
                        .and_then(|k| {
                            if let Ty::Object(fields) = underlying(&ty) {
                                fields.get(k).cloned()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(Ty::Unknown);
                    self.bind(&p.value, field)
                }
                if let Some(p) = rest {
                    self.bind(p, Ty::Unknown)
                }
            }
            Pat::Array { elems, rest, .. } => {
                for p in elems.iter().flatten() {
                    self.bind(p, Ty::Unknown)
                }
                if let Some(p) = rest {
                    self.bind(p, Ty::Unknown)
                }
            }
            Pat::Default { pat, .. } | Pat::Rest { arg: pat, .. } => self.bind(pat, Ty::Unknown),
        }
    }
    fn nodes(&mut self, nodes: &[Node]) {
        for node in nodes {
            if self.diagnostics.len() >= 128 {
                break;
            }
            match node {
                Node::Interp(i) => self.expression(&i.expr),
                Node::Element(e) => {
                    self.attrs(&e.attrs, None, &e.tag);
                    self.nodes(&e.children)
                }
                Node::ComponentRef(r) => {
                    let expected = self.c.script.iter().find_map(|s| match decl(s) {
                        Stmt::Import(i) if i.default.as_ref().is_some_and(|n| n.name == r.name) => {
                            match self.ctx.imports.get(&i.source) {
                                Some(Imported::Brain(p)) => Some(p.clone()),
                                _ => None,
                            }
                        }
                        _ => None,
                    });
                    self.attrs(&r.attrs, expected.as_deref(), &r.name);
                    self.nodes(&r.children)
                }
                Node::If(i) => {
                    let saved = self.env.clone();
                    let mut remaining = saved.clone();
                    for b in &i.branches {
                        self.env = remaining.clone();
                        self.expression(&b.test);
                        narrow(&b.test, true, &mut self.env);
                        self.nodes(&b.body);
                        narrow(&b.test, false, &mut remaining);
                    }
                    self.env = remaining;
                    if let Some(other) = &i.otherwise {
                        self.nodes(other)
                    }
                    self.env = saved;
                }
                Node::Each(e) => {
                    self.expression(&e.list);
                    let ty = self.value(&e.list);
                    let item = match underlying(&ty) {
                        Ty::List(t) => *t.clone(),
                        _ => Ty::Unknown,
                    };
                    let saved = self.env.clone();
                    self.bind(&e.item, item);
                    if let Some(i) = &e.index {
                        self.env.insert(i.name.clone(), Ty::Num);
                    }
                    if let Some(key) = &e.key {
                        self.expression(key)
                    }
                    self.nodes(&e.body);
                    self.env = saved;
                    if let Some(empty) = &e.empty {
                        self.nodes(empty)
                    }
                }
                Node::Await(a) => {
                    self.expression(&a.promise);
                    self.nodes(&a.pending);
                    let ty = match self.value(&a.promise) {
                        Ty::Promise(t) => *t,
                        _ => Ty::Unknown,
                    };
                    for (branch, ty) in [(&a.then, ty), (&a.catch, Ty::Unknown)] {
                        if let Some(b) = branch {
                            let saved = self.env.clone();
                            if let Some(p) = &b.binding {
                                self.bind(p, ty)
                            }
                            self.nodes(&b.body);
                            self.env = saved;
                        }
                    }
                }
                Node::SlotScope(slot) => {
                    let saved = self.env.clone();
                    for n in &slot.names {
                        self.env.insert(n.name.clone(), Ty::Unknown);
                    }
                    self.nodes(&slot.body);
                    self.env = saved
                }
                _ => {}
            }
        }
    }
    fn stmts(&mut self, stmts: &[Stmt], poisoned: &HashSet<String>) {
        for s in stmts {
            match decl(s) {
                Stmt::Var(v) => {
                    for d in &v.decls {
                        if let Some(e) = &d.init {
                            self.expression(e);
                            let ty = if matches!(&d.pat,Pat::Ident(i)if poisoned.contains(&i.name))
                            {
                                Ty::Unknown
                            } else {
                                materialize(self.value(e))
                            };
                            self.bind(&d.pat, ty);
                        }
                    }
                }
                Stmt::Expr { expr, .. } => self.expression(expr),
                Stmt::Return { arg: Some(e), .. } | Stmt::Throw { arg: e, .. } => {
                    self.expression(e)
                }
                Stmt::If {
                    test, cons, alt, ..
                } => {
                    self.expression(test);
                    let saved = self.env.clone();
                    narrow(test, true, &mut self.env);
                    self.stmts(std::slice::from_ref(cons.as_ref()), poisoned);
                    self.env = saved.clone();
                    narrow(test, false, &mut self.env);
                    if let Some(alt) = alt {
                        self.stmts(std::slice::from_ref(alt.as_ref()), poisoned)
                    }
                    if !terminates(cons) {
                        self.env = saved
                    }
                }
                Stmt::Block { body, .. } => {
                    let saved = self.env.clone();
                    self.stmts(body, poisoned);
                    self.env = saved;
                }
                Stmt::Func(f) | Stmt::Server { func: f, .. } => {
                    let saved = self.env.clone();
                    for p in &f.params {
                        self.bind(p, Ty::Unknown)
                    }
                    match &f.body {
                        FnBody::Expr(e) => self.expression(e),
                        FnBody::Block(body) => self.stmts(body, poisoned),
                    }
                    self.env = saved;
                }
                Stmt::Try {
                    block,
                    param,
                    handler,
                    finalizer,
                    ..
                } => {
                    let saved = self.env.clone();
                    self.stmts(block, poisoned);
                    self.env = saved.clone();
                    if let Some(body) = handler {
                        if let Some(p) = param {
                            self.bind(p, Ty::Unknown)
                        }
                        self.stmts(body, poisoned);
                    }
                    self.env = saved.clone();
                    if let Some(body) = finalizer {
                        self.stmts(body, poisoned)
                    }
                    self.env = saved;
                }
                Stmt::ForOf {
                    pat, iter, body, ..
                } => {
                    self.expression(iter);
                    let item = match self.value(iter) {
                        Ty::List(t) => *t,
                        _ => Ty::Unknown,
                    };
                    let saved = self.env.clone();
                    self.bind(pat, item);
                    self.stmts(std::slice::from_ref(body.as_ref()), poisoned);
                    self.env = saved;
                }
                _ => {}
            }
        }
    }
}
fn terminates(s: &Stmt) -> bool {
    match s {
        Stmt::Return { .. } | Stmt::Throw { .. } => true,
        Stmt::Block { body, .. } => body.last().is_some_and(terminates),
        _ => false,
    }
}
fn path(e: &Expr) -> Option<Vec<String>> {
    match e.unparen() {
        Expr::Ident(i) => Some(vec![i.name.clone()]),
        Expr::Member {
            obj,
            prop: MemberProp::Name(p),
            ..
        } => {
            let mut path = path(obj)?;
            path.push(p.name.clone());
            Some(path)
        }
        _ => None,
    }
}
fn narrow_path(e: &Expr, mask: u8, env: &mut HashMap<String, Ty>) {
    let Some(path) = path(e) else { return };
    let Some(mut target) = env.get_mut(&path[0]) else {
        return;
    };
    for name in &path[1..] {
        let receiver = match target {
            Ty::Maybe(t, _) | Ty::Chain(t) => t.as_mut(),
            t => t,
        };
        let Ty::Object(fields) = receiver else { return };
        let Some(next) = fields.get_mut(name) else {
            return;
        };
        target = next;
    }
    if let Ty::Maybe(t, flags) = target {
        let remaining = *flags & !mask;
        *target = if remaining == 0 {
            *t.clone()
        } else {
            Ty::Maybe(t.clone(), remaining)
        }
    }
}
fn narrow_ancestors(test: &Expr, env: &mut HashMap<String, Ty>) {
    if let Expr::Member { obj, .. } = test.unparen() {
        narrow_ancestors(obj, env);
        narrow_path(obj, 3, env);
    }
}
fn narrow(test: &Expr, truth: bool, env: &mut HashMap<String, Ty>) {
    match test.unparen() {
        Expr::Unary { op: "!", arg, .. } => narrow(arg, !truth, env),
        Expr::Binary {
            op: "&&",
            left,
            right,
            ..
        } if truth => {
            narrow(left, true, env);
            narrow(right, true, env)
        }
        Expr::Binary {
            op: "||",
            left,
            right,
            ..
        } if !truth => {
            narrow(left, false, env);
            narrow(right, false, env)
        }
        Expr::Binary {
            op, left, right, ..
        } if matches!(*op, "!=" | "!==" | "==" | "===") => {
            let nullish = |e: &Expr| match e.unparen() {
                Expr::Null(_) => Some(1),
                Expr::Ident(i) if i.name == "undefined" && !env.contains_key("undefined") => {
                    Some(2)
                }
                _ => None,
            };
            let pair = if let Some(flag) = nullish(right) {
                Some((left.as_ref(), flag))
            } else {
                nullish(left).map(|flag| (right.as_ref(), flag))
            };
            if let Some((value, flag)) = pair {
                if truth == matches!(*op, "!=" | "!==") {
                    let mask = if matches!(*op, "!=" | "==") { 3 } else { flag };
                    if mask & 2 != 0 {
                        narrow_ancestors(value, env);
                    }
                    narrow_path(value, mask, env)
                }
            }
        }
        _ if truth => {
            narrow_ancestors(test, env);
            narrow_path(test, 3, env)
        }
        _ => {}
    }
}

#[rustfmt::skip]
fn distance(a: &str, b: &str) -> usize {
    if a.len() > 80 || b.len() > 80 { return usize::MAX; }
    let mut row: Vec<usize> = (0..=b.chars().count()).collect();
    for (i,x) in a.chars().enumerate() {
        let mut prev=row[0]; row[0]=i+1;
        for (j,y) in b.chars().enumerate() {
            let old=row[j+1]; row[j+1]=(row[j+1]+1).min(row[j]+1).min(prev+usize::from(x!=y)); prev=old;
        }
    }
    row.last().copied().unwrap_or(usize::MAX)
}
/// Emit warning-only findings from proven literals and existing contracts.
pub(crate) fn check(c: &Component, ctx: &Ctx) -> Vec<Diag> {
    let declarations = declarations(c);
    if declarations.len() > 512 {
        return vec![];
    }
    let mut shadowed: HashSet<String> = crate::scope::top_bindings(c).into_keys().collect();
    let mut poisoned = HashSet::new();
    let mut counts = HashMap::new();
    for (name, _) in &declarations {
        *counts.entry(*name).or_insert(0) += 1;
    }
    security::walk(&c.script, false, &mut |e, _, _| match e {
        Expr::Assign { target, .. } | Expr::Update { arg: target, .. } => {
            if let Some(n) = root(target) {
                poisoned.insert(n.to_owned());
            }
        }
        Expr::Call { callee, .. } if matches!(callee.unparen(), Expr::Ident(i) if matches!(i.name.as_str(),"form"|"api"|"resource"|"sig"|"drv") && !shadowed.contains(&i.name)) =>
            {}
        Expr::Call { args, .. } | Expr::New { args, .. } => {
            for a in args {
                escape(a, &mut poisoned);
            }
        }
        _ => {}
    });
    // Any nested binding shadows conservatively. Counting every script identifier
    // declaration avoids pretending a callback's local value is the outer one.
    #[rustfmt::skip]
    fn bindings(stmts: &[Stmt], top: bool, poisoned: &mut HashSet<String>, globals:&HashSet<String>) {
        fn pat(p: &Pat, out: &mut HashSet<String>, globals:&HashSet<String>) {
            let mut names=vec![]; p.bound_names(&mut names); out.extend(names.into_iter().filter(|n|globals.contains(&n.name)).map(|n|n.name.clone()));
        }
        for s in stmts { match decl(s) {
            Stmt::Var(v) => for d in &v.decls {
                if !top { pat(&d.pat,poisoned,globals); }
                if let Some(e)=&d.init { escape(e,poisoned); }
            },
            Stmt::Return { arg: Some(e), .. } => {let mut escaped=HashSet::new();escape(e,&mut escaped);poisoned.extend(escaped.into_iter().filter(|n|globals.contains(n)));},
            Stmt::Func(f) | Stmt::Server { func:f, .. } => {
                for p in &f.params { pat(p,poisoned,globals); }
                if let FnBody::Block(b)=&f.body { bindings(b,false,poisoned,globals); }
            },
            Stmt::If { cons,alt,.. } => {
                bindings(std::slice::from_ref(cons.as_ref()),false,poisoned,globals);
                if let Some(a)=alt { bindings(std::slice::from_ref(a.as_ref()),false,poisoned,globals); }
            },
            Stmt::Block { body,.. } => bindings(body,false,poisoned,globals),
            Stmt::ForOf { pat:p,body,.. } => { pat(p,poisoned,globals); bindings(std::slice::from_ref(body.as_ref()),false,poisoned,globals); },
            Stmt::Try { block,param,handler,finalizer,.. } => {
                if let Some(p)=param { pat(p,poisoned,globals); } bindings(block,false,poisoned,globals);
                if let Some(b)=handler { bindings(b,false,poisoned,globals); } if let Some(b)=finalizer { bindings(b,false,poisoned,globals); }
            }, _=>{}
        }}
    }
    // A template binding is a write by the runtime/child, not a permanent value
    // from its initializer. In particular bind:api replaces sig(null) before events.
    // form() fields are a fixed validated facade: retain their schema evidence.
    fn template_bindings(nodes:&[Node],out:&mut HashSet<String>) {
        fn attrs(attrs:&[Attr],out:&mut HashSet<String>) {
            for attr in attrs {
                if attr.kind==AttrKind::Bind && let AttrValue::Expr(value)=&attr.value && let Some(name)=root(value) {out.insert(name.into());}
            }
        }
        for node in nodes {match node {
            Node::Element(node)=>{attrs(&node.attrs,out);template_bindings(&node.children,out)},
            Node::ComponentRef(node)=>{attrs(&node.attrs,out);template_bindings(&node.children,out)},
            Node::Slot(node)=>attrs(&node.attrs,out),
            Node::SlotScope(node)=>template_bindings(&node.body,out),
            Node::If(node)=>{for branch in &node.branches {template_bindings(&branch.body,out)} if let Some(body)=&node.otherwise {template_bindings(body,out)}},
            Node::Each(node)=>{template_bindings(&node.body,out);if let Some(body)=&node.empty {template_bindings(body,out)}},
            Node::Await(node)=>{template_bindings(&node.pending,out);for branch in [&node.then,&node.catch].into_iter().flatten(){template_bindings(&branch.body,out)}},
            _=>{}
        }}
    }
    let mut writes=HashSet::new();
    template_bindings(&c.head,&mut writes);
    template_bindings(&c.template,&mut writes);
    for name in writes {
        let form=declarations.iter().any(|(declared,value)| *declared==name && matches!(value.unparen(),Expr::Call {callee,..} if matches!(callee.unparen(),Expr::Ident(id) if id.name=="form" && !shadowed.contains("form"))));
        if !form {poisoned.insert(name);}
    }
    let globals = shadowed.clone();
    bindings(&c.script, true, &mut poisoned, &globals);
    security::walk(&c.script, false, &mut |e, _, _| {
        if let Expr::Func(f) = e {
            for p in &f.params {
                let mut names = vec![];
                p.bound_names(&mut names);
                poisoned.extend(names.into_iter().map(|n| n.name.clone()));
            }
            if let FnBody::Block(b) = &f.body {
                bindings(b, false, &mut poisoned, &globals);
            }
        }
    });
    for (name, count) in &counts {
        if *count > 1 {
            poisoned.insert((*name).to_owned());
        }
    }
    shadowed.extend(poisoned.iter().cloned());
    let mut schemas = ctx.type_shapes.clone();
    schemas.retain(|name, _| !shadowed.contains(name));
    schemas.extend(
        shapes(c)
            .into_iter()
            .filter(|(name, _)| !poisoned.contains(name)),
    );
    let mut a = Analysis {
        c,
        ctx,
        env: shadowed
            .iter()
            .map(|name| (name.clone(), Ty::Unknown))
            .collect(),
        schemas,
        forms: HashMap::new(),
        apis: HashMap::new(),
        shadowed,
        diagnostics: vec![],
        poisoned: poisoned.clone(),
    };
    for statement in &c.script {
        if let Stmt::Import(import) = statement {
            if let Some(name) = &import.default {
                if !poisoned.contains(&name.name) {
                    if let Some(contract) = ctx.api_contracts.get(&import.source) {
                        a.register_api(&name.name, contract.clone());
                    }
                }
            }
            for (_, local) in &import.named {
                if !poisoned.contains(&local.name) {
                    if let Some(shape) = ctx.schema_aliases.get(&local.name) {
                        a.schemas.insert(local.name.clone(), shape.clone());
                    }
                }
            }
        }
    }
    for (name, e) in &declarations {
        if poisoned.contains(*name) || counts[name] > 1 {
            continue;
        }
        if let Some(contract) = api_contract(e, HashMap::new()) {
            if !a.shadowed.contains("api") {
                a.register_api(name, contract);
            }
        }
        let ty = materialize(a.value(e));
        a.env.insert((*name).to_owned(), ty);
        if let Expr::Call { callee, args, .. } = e.unparen() {
            if matches!(callee.unparen(),Expr::Ident(i)if i.name=="form"&&!a.shadowed.contains("form"))
            {
                if let Some(shape) = args.first().and_then(|e| a.schema(e)) {
                    if let Some(init) = args
                        .get(1)
                        .and_then(object)
                        .and_then(|o| o.get("init").copied())
                    {
                        a.body(init, &shape, "B2004");
                    }
                    a.forms.insert((*name).to_owned(), shape);
                }
            }
        }
    }
    a.stmts(&c.script, &poisoned);
    a.nodes(&c.head);
    a.nodes(&c.template);
    a.diagnostics
}
