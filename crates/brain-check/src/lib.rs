//! Semantic checks for `.brain` components (SPEC §2, §3, §7, §8). Every diagnostic carries the exact
//! fix in text so an LLM can apply it without reasoning.
//!
//! Code families: B01xx BrainScript · B02xx reactivity, DOM and markup · B03xx CSS ·
//! B04xx accessibility · B05xx SEO. (B00xx are syntax errors from brain-parse.)

pub mod catalog;
pub mod contract;
pub mod servershare;
mod cascade;
pub mod http;
pub mod split;
mod literals;
mod runaway;
mod security;
pub mod seo;
pub mod tools;
pub mod worker;
mod quality;
pub mod doctor;
pub mod types;
pub mod env;
pub mod i18n;
mod i18n_rules;
mod planned;
mod structure;
pub mod outline;
pub mod scope;

pub use catalog::{explain, CATALOG};

use brain_parse::ast::*;
use brain_parse::{Diag, Span};
use scope::{Kind, Scopes, KNOWN_GLOBALS, PRIMITIVES};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Token file names, preferred first (`tokens.brain.css` is the pre-§10 name).
pub const TOKEN_FILES: &[&str] = &["tokens.cssbrain", "tokens.brain.css"];

/// What the checker knows beyond the file itself.
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// A view whose `app.brain` or a `_layout.brain` above it already renders `<main>` (B0405 normalizes the view's).
    pub wrapper_main: bool,
    /// Custom properties from `tokens.cssbrain` (`None` = no token file found: skip B0302/B0408).
    pub tokens: Option<Vec<(String, String)>>,
    /// Import source → what it resolved to.
    pub imports: HashMap<String, Imported>,
    /// Import source → fix text for B0119 built from the files that do exist (closest name first).
    pub missing_hint: HashMap<String, String>,
    /// Import source → the one existing file it most likely meant (B0119 edit).
    pub missing_best: HashMap<String, String>,
    /// `.brain` import source → its declared channels (§12: emits, expose).
    pub children: HashMap<String, Child>,
    /// Context keys imported from `channels/*.jsbrain` (local name → has a default value).
    pub keys: HashMap<String, bool>,
    /// Names passed as the first argument of `provide(…)` anywhere in the app (`None` = app not scanned): B0807.
    pub provided: Option<HashSet<String>>,
    /// `brain.json` → `"redirects"`: external hosts `redirect()` may target (B0710).
    pub redirect_hosts: Vec<String>,
    /// brain.json `"cache": { "data": false }`: `cache:` in `api()` is ignored (B1102 warning, §35).
    pub cache_data_off: bool,
    /// Languages of the app (`None` = no i18n setup): B1401, B1403, B1404, B0306 (§37).
    pub i18n: Option<i18n::I18n>,
    /// The app's `.brain_env` keys (`None` = no such file): B0721 (§36). Never values.
    pub env: Option<env::EnvInfo>,
    /// Names exported by the app's `types.jsbrain` (`None` = no such file), for `api()` schemas (B1103).
    pub types: Option<HashSet<String>>,
    /// Static `canonical` of every view in the app → the view files declaring it (B1604). Empty when not scanned.
    pub canonicals: HashMap<String, Vec<String>>,
}

/// What a parent may use of a child component besides its props (INTERNALS §12).
#[derive(Clone, Debug, Default)]
pub struct Child {
    /// `<meta emits="select:obj close">` → `["select", "close"]`.
    pub emits: Vec<String>,
    /// `<meta expose="open close">`.
    pub expose: Vec<String>,
}

/// Names in a space-separated meta list, without their `:type`.
/// Every expression inside `e` (itself included, function bodies too), depth first.
pub fn each_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    security::expr(e, false, &[], &mut |x, _, _| f(x));
}

pub fn meta_names(c: &Component, attr: &str) -> Vec<String> {
    c.meta
        .iter()
        .filter(|m| m.name == attr)
        .filter_map(|m| m.value.as_deref())
        .flat_map(|v| v.split_whitespace().map(|w| w.split(':').next().unwrap_or("").to_string()))
        .filter(|w| !w.is_empty())
        .collect()
}

#[derive(Clone, Debug)]
pub enum Imported {
    /// A `.brain` file and its declared props.
    Brain(Vec<Prop>),
    /// A `.jsbrain` headed by `<meta server>`: usable only from `load()` and `server:` functions (SPEC §11.1).
    Server,
    /// Plain JS or anything not compiled.
    Other,
    Missing,
}

/// Reads what `check` needs from disk: the nearest `tokens.cssbrain` (or the legacy
/// `tokens.brain.css` while apps migrate, SPEC §10) and the props of imported
/// `.brain` files. Returns the context and the files read (for watch/HMR dependencies).
pub fn load_ctx(disk_path: &Path, c: &Component) -> (Ctx, Vec<PathBuf>) {
    let mut deps = Vec::new();
    let mut ctx = Ctx::default();
    let dir = disk_path.parent().unwrap_or(Path::new("."));
    let project = app_root(dir);
    'up: for anc in dir.ancestors() {
        for name in TOKEN_FILES {
            let t = anc.join(name);
            deps.push(t.clone()); // Absence matters: creating a nearer token file changes the context.
            if let Ok(src) = std::fs::read_to_string(&t) {
                if let Ok(sheet) = brain_css::parse_str(&src, name) {
                    ctx.tokens = Some(brain_css::custom_props(&sheet));
                }
                break 'up;
            }
        }
        if project.as_deref() == Some(anc) { break; }
    }
    for anc in dir.ancestors() {
        deps.push(anc.join("brain.json"));
        if let Ok(json) = std::fs::read_to_string(anc.join("brain.json")) {
            ctx.redirect_hosts = json_string_list(&json, "redirects");
            break;
        }
        if project.as_deref() == Some(anc) { break; }
    }
    // B0405: the shell or a layout above this view already has `<main>`.
    if c.level == Level::View && c.kind == FileKind::Component {
        let has_main = |p: &Path| {
            std::fs::read_to_string(p).ok().and_then(|src| brain_parse::parse(&src, p).ok()).is_some_and(|w| element_tags(&w.template).is_some_and(|t| t.contains("main")))
        };
        for anc in dir.ancestors() {
            let layout = anc.join("_layout.brain");
            deps.push(layout.clone());
            deps.push(anc.join("app.brain"));
            if layout != disk_path && has_main(&layout) {
                ctx.wrapper_main = true;
                break;
            }
            if anc.join("app.brain").is_file() || anc.join("brain.json").is_file() {
                ctx.wrapper_main = has_main(&anc.join("app.brain"));
                break;
            }
        }
    }
    for s in &c.script {
        let Stmt::Import(i) = s else { continue };
        let target = dir.join(&i.source);
        let info = if !(i.source.starts_with("./") || i.source.starts_with("../")) {
            Imported::Other
        } else if i.source.ends_with(".brain") {
            match std::fs::read_to_string(&target) {
                Ok(src) => {
                    deps.push(target.clone());
                    match brain_parse::parse(&src, &target) {
                        Ok(child) => {
                            ctx.children.insert(i.source.clone(), Child { emits: meta_names(&child, "emits"), expose: meta_names(&child, "expose") });
                            Imported::Brain(child.props)
                        }
                        Err(_) => Imported::Brain(Vec::new()),
                    }
                }
                Err(_) => Imported::Missing,
            }
        } else if i.source.ends_with(".jsbrain") && target.exists() {
            let module = std::fs::read_to_string(&target).ok().and_then(|src| brain_parse::parse(&src, &target).ok());
            if let Some(m) = &module {
                // `export const Theme = key('theme', default?)`
                for s in &m.script {
                    let Stmt::Export { decl, .. } = s else { continue };
                    let Stmt::Var(v) = decl.as_ref() else { continue };
                    for d in &v.decls {
                        if let (Pat::Ident(id), Some(Expr::Call { callee, args, .. })) = (&d.pat, d.init.as_ref().map(Expr::unparen))
                            && matches!(callee.unparen(), Expr::Ident(k) if k.name == "key")
                        {
                            let local = i.named.iter().find(|(imp, _)| imp.name == id.name).map(|(_, l)| l.name.clone());
                            if let Some(local) = local {
                                ctx.keys.insert(local, args.len() > 1);
                            }
                        }
                    }
                }
            }
            let server = module.is_some_and(|m| m.meta.iter().any(|a| a.name == "server"));
            deps.push(target);
            if server { Imported::Server } else { Imported::Other }
        } else if target.exists() {
            deps.push(target);
            Imported::Other
        } else {
            Imported::Missing
        };
        if matches!(info, Imported::Missing) {
            let (hint, best) = missing_hint(dir, &i.source);
            ctx.missing_hint.insert(i.source.clone(), hint);
            if let Some(b) = best {
                ctx.missing_best.insert(i.source.clone(), b);
            }
        }
        ctx.imports.insert(i.source.clone(), info);
    }
    if !ctx.keys.is_empty() {
        // B0807: what the render tree guarantees above this file; the app-wide set when the file is not reachable.
        ctx.provided = app_root(dir).map(|root| {
            let rel = disk_path.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let mut set = doctor::ancestor_provided(&root, &rel).unwrap_or_else(|| provided_names(&root));
            // A component may use what it provides itself.
            set.extend(provided_names_in(&c.src));
            set
        });
    }
    if let Some(root) = app_root(dir) {
        deps.extend([root.join("types.jsbrain"), root.join(env::FILE), root.join("brain.json")]);
        ctx.types = types::exported(&root);
        ctx.env = env::load(&root);
        ctx.i18n = i18n::load(&root);
        if let Some(i18n) = &ctx.i18n {
            for locale in &i18n.locales { deps.push(root.join("i18n").join(format!("{locale}.json"))); }
        }
        ctx.cache_data_off = std::fs::read_to_string(root.join("brain.json")).ok().is_some_and(|j| cache_data_off(&j));
    }
    if c.level == Level::View
        && c.meta.iter().any(|m| m.name == "canonical")
        && let Some(root) = app_root(dir)
    {
        ctx.canonicals = seo::canonicals(&root);
    }
    // Provider ancestry and canonical uniqueness read other components. Track the graph (including directory
    // membership) only for those checks; ordinary components keep their small direct dependency set.
    if (!ctx.keys.is_empty() || c.meta.iter().any(|m| m.name == "canonical")) && let Some(root) = app_root(dir) {
        fn graph(dir: &Path, deps: &mut Vec<PathBuf>) {
            deps.push(dir.to_path_buf());
            if let Ok(entries) = std::fs::read_dir(dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with('.') || matches!(name.as_ref(), "node_modules" | "dist" | "target") { continue; }
                    if e.file_type().is_ok_and(|t| t.is_dir()) { graph(&p, deps); }
                    else if p.extension().is_some_and(|x| x == "brain" || x == "jsbrain") { deps.push(p); }
                }
            }
        }
        graph(&root, &mut deps);
    }
    for dep in &mut deps {
        if let Ok(path) = dep.canonicalize() { *dep = path; }
    }
    deps.sort();
    deps.dedup();
    (ctx, deps)
}

/// The app root: nearest ancestor with `app.brain` or a token file.
fn app_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|a| a.join("app.brain").exists() || a.join("brain.json").exists() || a.join("types.jsbrain").exists() || TOKEN_FILES.iter().any(|t| a.join(t).exists()))
        .map(Path::to_path_buf)
}

/// First arguments of every `provide(X, …)` in the app's `.brain` files (textual scan, B0807).
fn provided_names(root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "dist" | "target") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if name.ends_with(".brain")
                && let Ok(src) = std::fs::read_to_string(&p)
            {
                for (i, _) in src.match_indices("provide(") {
                    let rest = src[i + 8..].trim_start();
                    let n: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect();
                    if !n.is_empty() {
                        out.insert(n);
                    }
                }
            }
        }
    }
    out
}

/// B0119 fix: the closest existing file in the target folder, or what exists there, or the folders that exist.
fn missing_hint(dir: &Path, source: &str) -> (String, Option<String>) {
    let (folder, file) = match source.rfind('/') {
        Some(i) => (&source[..i], &source[i + 1..]),
        None => (".", source),
    };
    let ext = file.find('.').map(|i| &file[i..]).unwrap_or("");
    let list = |d: &Path, dirs: bool| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir() == dirs)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.') && (dirs || n.ends_with(ext)) && !n.ends_with(".test.brain"))
            .collect();
        v.sort();
        v
    };
    let target_dir = dir.join(folder);
    if target_dir.is_dir() {
        let files = list(&target_dir, false);
        let stem = |n: &str| n.split('.').next().unwrap_or("").to_string();
        let want = stem(file);
        let best = files
            .iter()
            .map(|f| (scope::edit_distance(&want, &stem(f)), f))
            .filter(|(d, f)| *d <= 3.max(want.len() / 3) && *d < want.len().max(stem(f).len()))
            .min_by_key(|(d, _)| *d)
            .map(|(_, f)| f.clone());
        return match best {
            Some(f) => (format!("did you mean `{folder}/{f}`? (run `brain map --json` to see every file)"), Some(format!("{folder}/{f}"))),
            None if files.is_empty() => (format!("`{folder}/` has no `{ext}` files: create `{source}` or remove the import"), None),
            None => (format!("`{folder}/` contains: {}; create `{source}` or import one of those", files.iter().take(8).map(|f| format!("`{folder}/{f}`")).collect::<Vec<_>>().join(", ")), None),
        };
    }
    let parent = target_dir.parent().unwrap_or(dir);
    let up = folder.rfind('/').map(|i| &folder[..i]).unwrap_or(".");
    let dirs = list(parent, true);
    let hint = if dirs.is_empty() {
        format!("create `{source}` or remove the import")
    } else {
        format!("folder `{folder}/` does not exist; available folders: {}. Create `{source}` or remove the import", dirs.iter().map(|d| format!("`{up}/{d}/`")).collect::<Vec<_>>().join(", "))
    };
    (hint, None)
}

/// Runs every rule. Diagnostics come sorted by position.
pub fn check(c: &Component, ctx: &Ctx) -> Vec<Diag> {
    let mut server_names = HashSet::new();
    for s in &c.script {
        if let Stmt::Import(i) = s
            && matches!(ctx.imports.get(&i.source), Some(Imported::Server))
        {
            for id in i.default.iter().chain(i.namespace.iter()).chain(i.named.iter().map(|(_, l)| l)) {
                server_names.insert(id.name.clone());
            }
        }
    }
    let mut server_fns: HashSet<String> = HashSet::new();
    for s in &c.script {
        match s {
            Stmt::Server { func, .. } => server_fns.extend(func.name.iter().map(|n| n.name.clone())),
            Stmt::Export { decl, .. } if matches!(decl.as_ref(), Stmt::Func(f) if f.name.as_ref().is_some_and(|n| n.name == "load")) => {
                server_fns.insert("load".into());
            }
            _ => {}
        }
    }
    let server_ctx = c.kind == FileKind::Module && c.meta.iter().any(|m| m.name == "server");
    let mut api_vars = HashMap::new();
    collect_api_vars(c, ctx, &c.template, &mut api_vars);
    let mut ck = Checker { c, ctx, diags: Vec::new(), scopes: Scopes::new(scope::top_bindings(c)), fn_depth: 0, server_names, server_ctx, server_fns, api_vars, scoped_stores: HashSet::new(), shared: servershare::shared(c) };
    ck.structure();
    ck.runaway();
    ck.security();
    ck.seo_rules();
    ck.tools();
    ck.worker_rules();
    ck.quality();
    ck.http_rules();
    ck.api_types();
    ck.i18n_rules();
    ck.planned();
    ck.script();
    ck.template();
    ck.shell_static();
    ck.aria_refs();
    ck.css();
    ck.a11y();
    ck.seo();
    let mut d = ck.diags;
    // B0721: keys read here but missing from .brain_env; the diagnostic and its edit belong to .brain_env.
    if let Some(env) = &ctx.env {
        let mut seen = HashSet::new();
        let idx = brain_parse::LineIndex::new(&c.src);
        for (k, at) in env::keys_used(&c.src) {
            if env.keys.contains(&k) || !seen.insert(k.clone()) {
                continue;
            }
            let (line, col) = idx.line_col(&c.src, at);
            let eidx = brain_parse::LineIndex::new(&env.src);
            let (eline, _) = eidx.line_col(&env.src, env.insert_at.min(env.src_len));
            let span = Span { start: env.insert_at as u32, end: env.insert_at as u32, line: eline, col: 1 };
            let mut diag = Diag::new("B0721", env::FILE, span, format!("`{k}` is read in `{}:{line}:{col}` but not declared in .brain_env", c.file), format!("add `{k}=` to the shared block of .brain_env (and a value per environment)"));
            let text = if env.src.is_empty() || env.src[..env.insert_at.min(env.src_len)].ends_with('\n') || env.insert_at == 0 { format!("{k}=\n") } else { format!("\n{k}=") };
            diag = diag.edit(span.start, span.end, text);
            diag.locate(&env.src);
            d.push(diag);
        }
    }
    for x in &mut d {
        if x.file == c.file {
            x.locate(&c.src);
        }
    }
    d.sort_by_key(|d| (d.line, d.col));
    d.dedup();
    d
}

struct Checker<'a> {
    c: &'a Component,
    ctx: &'a Ctx,
    diags: Vec<Diag>,
    scopes: Scopes,
    fn_depth: u32,
    /// Bindings imported from `<meta server>` modules (B0701).
    server_names: HashSet<String>,
    /// Inside `load()`, a `server:` function or a `<meta server>` module: env and server imports allowed.
    server_ctx: bool,
    /// `load` and the `server:` functions of this component: the only top-level names server code sees.
    server_fns: HashSet<String>,
    /// `<Child bind:api={m} />` → `m` → what the child exposes (B0804).
    api_vars: HashMap<String, (String, Vec<String>)>,
    /// Start offsets of `store(…)` calls that are the value of a `provide(K, store(…))`: a subtree-scoped store (§41.2).
    scoped_stores: HashSet<u32>,
    /// #276: pure constants and server-only helpers the server module also gets.
    shared: servershare::Shared,
}

/// Import source of a component used as `<Name>`.
fn component_source<'a>(c: &'a Component, name: &str) -> Option<&'a str> {
    c.script.iter().find_map(|s| match s {
        Stmt::Import(i) if i.default.as_ref().is_some_and(|d| d.name == name) => Some(i.source.as_str()),
        _ => None,
    })
}

fn collect_api_vars(c: &Component, ctx: &Ctx, nodes: &[Node], out: &mut HashMap<String, (String, Vec<String>)>) {
    for n in nodes {
        match n {
            Node::Element(e) => collect_api_vars(c, ctx, &e.children, out),
            Node::ComponentRef(r) => {
                for a in &r.attrs {
                    if a.kind == AttrKind::Bind
                        && a.name == "api"
                        && let AttrValue::Expr(e) = &a.value
                        && let Expr::Ident(id) = e.unparen()
                        && let Some(child) = component_source(c, &r.name).and_then(|src| ctx.children.get(src))
                    {
                        out.insert(id.name.clone(), (r.name.clone(), child.expose.clone()));
                    }
                }
                collect_api_vars(c, ctx, &r.children, out);
            }
            Node::If(b) => {
                for br in &b.branches {
                    collect_api_vars(c, ctx, &br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    collect_api_vars(c, ctx, o, out);
                }
            }
            Node::Each(b) => {
                collect_api_vars(c, ctx, &b.body, out);
                if let Some(e) = &b.empty {
                    collect_api_vars(c, ctx, e, out);
                }
            }
            Node::SlotScope(b) => collect_api_vars(c, ctx, &b.body, out),
            Node::Await(b) => {
                collect_api_vars(c, ctx, &b.pending, out);
                for br in [&b.then, &b.catch].into_iter().flatten() {
                    collect_api_vars(c, ctx, &br.body, out);
                }
            }
            _ => {}
        }
    }
}

fn in_channels(file: &str) -> bool {
    file.starts_with("channels/") || file.contains("/channels/")
}

const MUTATING: &[&str] = &["push", "pop", "shift", "unshift", "splice", "sort", "reverse", "fill", "copyWithin", "set", "add", "delete", "clear"];

fn root_ident(e: &Expr) -> Option<&Ident> {
    match e.unparen() {
        Expr::Ident(i) => Some(i),
        Expr::Member { obj, .. } => root_ident(obj),
        _ => None,
    }
}

impl<'a> Checker<'a> {
    fn push(&mut self, code: &'static str, span: Span, msg: impl Into<String>, fix: impl Into<String>) {
        self.diags.push(Diag::new(code, &self.c.file, span, msg, fix));
    }

    /// Like [`push`] with exact edits `(start, end, text)` over the source (SPEC §5 `edits`).
    fn push_edits(&mut self, code: &'static str, span: Span, msg: impl Into<String>, fix: impl Into<String>, edits: Vec<(u32, u32, String)>) {
        let mut d = Diag::new(code, &self.c.file, span, msg, fix);
        for (a, b, t) in edits {
            d = d.edit(a, b, t);
        }
        self.diags.push(d);
    }

    /// A deterministic normalization (INTERNALS §21): `info`, never blocks, and the compiler applies its edits.
    fn push_info(&mut self, code: &'static str, span: Span, msg: impl Into<String>, fix: impl Into<String>, edits: Vec<(u32, u32, String)>) {
        self.push_edits(code, span, msg, fix, edits);
        if let Some(d) = self.diags.pop() {
            self.diags.push(d.info());
        }
    }

    /// Source text of a span.
    fn text(&self, span: Span) -> &str {
        self.c.src.get(span.start as usize..span.end as usize).unwrap_or("")
    }

    // ───────────────────────── script ─────────────────────────

    fn script(&mut self) {
        let c = self.c;
        for s in &c.script {
            // Top-level declarations initialised with a primitive are the legal place for sig/drv/ref.
            let module_var = match s {
                Stmt::Var(v) => Some(v),
                Stmt::Export { decl, .. } => match decl.as_ref() {
                    Stmt::Var(v) => Some(v),
                    _ => None,
                },
                _ => None,
            };
            if c.kind == FileKind::Module
                && !self.server_ctx
                && let Some(v) = module_var
                && v.kind != VarKind::Const
            {
                for d in &v.decls {
                    let channel = matches!(d.init.as_ref().map(Expr::unparen), Some(Expr::Call { callee, .. }) if matches!(callee.unparen(), Expr::Ident(k) if matches!(k.name.as_str(), "store" | "bus" | "key")));
                    if !channel {
                        self.push("B0801", d.pat.span(), "mutable module-level state in a `.jsbrain` is hidden global state", "make it `const`, keep it inside a function, or share it through `export default store({ … })` in `channels/`");
                    }
                }
            }
            if let Stmt::Var(v) = s {
                if v.kind == VarKind::Var {
                    self.push_info("B0103", v.span, "`var` does not exist in BrainScript (normalized to `let`)", "use `let` (or `const` if it is never reassigned)", vec![(v.span.start, v.span.start + 3, "let".into())]);
                }
                for d in &v.decls {
                    if let Some(init) = &d.init {
                        if let (Some(k), Expr::Call { callee, args, .. }) = (scope::init_kind(Some(init)), init.unparen()) {
                            if matches!(callee.unparen(), Expr::Ident(u) if u.name == "use") {
                                self.call(callee, args, init.span());
                            }
                            // The embedded brain/ui kit is stateful by design (INTERNALS §14): Table, Toast, Menu…
                            let kit = c.file.contains(".brain/kit/") || c.file.starts_with("runtime/ui/") || scope::kit_source(&c.src);
                            if k == Kind::Sig && c.level == Level::Micro && !kit {
                                self.push("B0116", init.span(), "a `ui/` component has no state of its own", "receive the value as a prop (`<meta props=\"x:num=0\">`) or move the file to `comp/`");
                            }
                            for a in args {
                                self.expr(a);
                            }
                        } else {
                            self.expr(init);
                        }
                    }
                    self.pat_defaults(&d.pat);
                }
                continue;
            }
            if let Stmt::Server { func, span } = s {
                if c.kind != FileKind::Component {
                    self.push("B0704", *span, "`server:` only goes in the <script> of a `.brain`", "in a `.jsbrain`, put `<meta server>` on the first line and export the function");
                } else if !func.is_async {
                    self.push_edits("B0704", *span, "a `server:` function must be `async`", "write `server: async function name(ctx, data) { … }`", vec![(func.span.start, func.span.start, "async ".into())]);
                }
                self.server_fn(func);
                continue;
            }
            if let Stmt::Export { decl, span } = s
                && let Stmt::Func(f) = decl.as_ref()
                && let Some(name) = f.name.as_ref().map(|n| n.name.as_str()).filter(|n| matches!(*n, "paths" | "sitemap"))
            {
                // §32: the params a dynamic route is prerendered for (`paths`) or listed in the sitemap at request
                // time (`sitemap`, served by server.mjs).
                if c.level != Level::View || !c.file.contains('[') {
                    self.push("B0117", *span, &format!("`{name}()` only exists in dynamic views (`views/…/[id].brain`)"), "remove it, or move it to the `[param].brain` view whose pages it lists");
                } else if name == "sitemap" && c.script.iter().any(|t| exports_fn(t, "paths")) {
                    // paths() prerenders the pages and lists them in the build's sitemap; sitemap() is for views that
                    // render at request time. Both would list the same pages twice under two contracts.
                    self.push("B0117", *span, "a view exports both `paths()` and `sitemap()`", "keep `paths()` to prerender the pages (they are already in the sitemap), or remove it and keep `sitemap()` to render them at request time");
                }
                self.server_fn(f);
                continue;
            }
            if let Stmt::Export { decl, span } = s
                && let Stmt::Func(f) = decl.as_ref()
                    && f.name.as_ref().is_some_and(|n| n.name == "load")
                {
                    if c.level != Level::View {
                        self.push("B0117", *span, "`load()` only exists in `views/`", "move the data loading to the view and pass the result as a prop");
                    } else if !f.is_async {
                        self.push("B0117", *span, "`load()` must be `async`", "write `export async function load(ctx) { … }`");
                    }
                    self.server_fn(f);
                    continue;
                }
            // #276: a helper only server code reaches runs on the server (private env is fine there).
            if let Some(name) = match s {
                Stmt::Func(f) => f.name.as_ref().map(|n| n.name.clone()),
                Stmt::Var(v) if v.decls.len() == 1 && matches!(v.decls[0].init.as_ref().map(Expr::unparen), Some(Expr::Func(_))) => match &v.decls[0].pat {
                    Pat::Ident(i) => Some(i.name.clone()),
                    _ => None,
                },
                _ => None,
            } && self.shared.helpers.contains(&name)
            {
                let saved = std::mem::replace(&mut self.server_ctx, true);
                self.stmt(s);
                self.server_ctx = saved;
                continue;
            }
            self.stmt(s);
        }
    }

    /// `load()` and `server:` bodies run on the server.
    fn server_fn(&mut self, f: &Func) {
        let saved = std::mem::replace(&mut self.server_ctx, true);
        self.func(f);
        self.server_ctx = saved;
    }

    /// B0703 on a literal that reaches the client.
    fn secrets(&mut self, text: &str, span: Span) {
        if self.server_ctx {
            return;
        }
        if let Some((_, kind)) = scan_secrets(text).into_iter().next() {
            self.push("B0703", span, format!("a literal shaped like a secret key ({kind}) is in code that reaches the browser"), "remove the key from the code: store it in `.env` without the `PUBLIC_` prefix and use it only inside `load()` or `server: async function`");
        }
    }

    /// B0702: `process.env.X` / `import.meta.env.X` in client code. Returns true when `e` was an env access.
    fn env_access(&mut self, e: &Expr) -> bool {
        let Expr::Member { obj, prop, .. } = e.unparen() else { return false };
        let is_env = |x: &Expr| matches!(x.unparen(), Expr::Member { obj, prop: MemberProp::Name(n), .. }
            if n.name == "env" && matches!(obj.unparen(), Expr::Ident(i) if i.name == "process" || i.name == "import.meta"));
        let key: Option<Option<String>> = if is_env(obj) {
            Some(match prop {
                MemberProp::Name(n) => Some(n.name.clone()),
                MemberProp::Computed(k) => match k.unparen() {
                    Expr::Str { value, .. } => Some(value.clone()),
                    other => {
                        self.expr(other);
                        None
                    }
                },
            })
        } else if is_env(e) {
            Some(None)
        } else {
            return false;
        };
        if self.server_ctx {
            return true;
        }
        let root = if dump_root(e) == "import.meta" { "import.meta.env" } else { "process.env" };
        match key.flatten() {
            Some(k) if k.starts_with("PUBLIC_") => {}
            Some(k) => self.push("B0702", e.span(), format!("`{root}.{k}` is a private variable and this code reaches the browser"), format!("read it on the server (`load()` or `server: async function`) and pass only the value; if it really is public, rename it to `PUBLIC_{k}`")),
            None => self.push("B0702", e.span(), format!("dynamic or whole access to `{root}` in code that reaches the browser"), "read each variable by name (`import.meta.env.PUBLIC_X`); private ones only in `load()` or `server: async function`"),
        }
        true
    }

    fn pat_defaults(&mut self, p: &Pat) {
        match p {
            Pat::Default { pat, default, .. } => {
                self.expr(default);
                self.pat_defaults(pat);
            }
            Pat::Object { props, rest, .. } => {
                for pp in props {
                    if let PropKey::Computed(e) = &pp.key {
                        self.expr(e);
                    }
                    self.pat_defaults(&pp.value);
                }
                if let Some(r) = rest {
                    self.pat_defaults(r);
                }
            }
            Pat::Array { elems, rest, .. } => {
                for e in elems.iter().flatten() {
                    self.pat_defaults(e);
                }
                if let Some(r) = rest {
                    self.pat_defaults(r);
                }
            }
            Pat::Rest { arg, .. } => self.pat_defaults(arg),
            Pat::Ident(_) => {}
        }
    }

    fn block(&mut self, body: &[Stmt]) {
        self.scopes.push();
        let mut m = HashMap::new();
        for s in body {
            scope::declare_stmt(s, &mut m);
        }
        for (k, v) in m {
            // Inner declarations shadow, and are never signals outside the top level.
            let v = if v.is_reactive_cell() { Kind::Let } else { v };
            self.scopes.declare(&k, v);
        }
        for s in body {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Var(v) => {
                if v.kind == VarKind::Var {
                    self.push_info("B0103", v.span, "`var` does not exist in BrainScript (normalized to `let`)", "use `let` (or `const` if it is never reassigned)", vec![(v.span.start, v.span.start + 3, "let".into())]);
                }
                for d in &v.decls {
                    if let Some(i) = &d.init {
                        self.expr(i);
                    }
                    self.pat_defaults(&d.pat);
                }
            }
            Stmt::Expr { expr, .. } | Stmt::Throw { arg: expr, .. } => self.expr(expr),
            Stmt::If { test, cons, alt, .. } => {
                self.expr(test);
                self.stmt(cons);
                if let Some(a) = alt {
                    self.stmt(a);
                }
            }
            Stmt::ForOf { pat, iter, body, .. } => {
                self.expr(iter);
                self.scopes.push();
                self.scopes.declare_pat(pat, Kind::Local);
                self.stmt(body);
                self.scopes.pop();
            }
            Stmt::Return { arg, .. } => {
                if let Some(a) = arg {
                    self.expr(a);
                }
            }
            Stmt::Block { body, .. } => self.block(body),
            Stmt::Func(f) => self.func(f),
            Stmt::Try { block, param, handler, finalizer, .. } => {
                self.block(block);
                if let Some(h) = handler {
                    self.scopes.push();
                    if let Some(p) = param {
                        self.scopes.declare_pat(p, Kind::Local);
                    }
                    self.block(h);
                    self.scopes.pop();
                }
                if let Some(f) = finalizer {
                    self.block(f);
                }
            }
            Stmt::Export { decl, span } => {
                let is_load = matches!(decl.as_ref(), Stmt::Func(f) if f.name.as_ref().is_some_and(|n| matches!(n.name.as_str(), "load" | "paths" | "sitemap")));
                // `load` outside a view is B0117.
                if self.c.kind != FileKind::Module && !is_load {
                    self.push_edits("B0120", *span, "`export` only exists in `.jsbrain` modules", "remove `export`: a .brain only exports its component; shared code goes in `lib/x.jsbrain`", vec![(span.start, decl.span().start, String::new())]);
                }
                self.stmt(decl)
            }
            Stmt::ExportDefault { expr, span } => {
                if self.c.kind != FileKind::Module {
                    self.push("B0120", *span, "`export default` only exists in `.jsbrain` modules", "in a .brain the component already is the default export; remove `export default` or move the logic to `lib/x.jsbrain`");
                }
                self.expr(expr);
            }
            Stmt::Import(i) => {
                if i.source.ends_with(".brain") {
                    for id in i.named.iter().map(|(_, l)| l).chain(i.namespace.iter()) {
                        self.push("B0805", id.span, format!("`{}` is internal to `{}`: a component only exports itself", id.name, i.source), "import the component (`import Name from '…'`) and talk to it through props, `bind:`, events or `bind:api`; shared state goes in `channels/`");
                    }
                }
                if is_framework_import(i) {
                    let mut end = i.span.end as usize;
                    let rest = &self.c.src[end.min(self.c.src.len())..];
                    let trail = rest.len() - rest.trim_start_matches([' ', '\t', ';']).len();
                    end += trail;
                    if self.c.src[end.min(self.c.src.len())..].starts_with('\n') {
                        end += 1;
                    }
                    self.push_info(
                        "B0121",
                        i.span,
                        format!("`{}` is not imported: the primitives and `brain/ui` are already available (import removed)", i.source),
                        "delete the import line",
                        vec![(i.span.start, end as u32, String::new())],
                    );
                    return;
                }
                if let Some(Imported::Missing) = self.ctx.imports.get(&i.source) {
                    let fix = self.ctx.missing_hint.get(&i.source).cloned().unwrap_or_else(|| "fix the relative path (start with `./` or `../`) or create the file".into());
                    let mut edits = Vec::new();
                    if let Some(best) = self.ctx.missing_best.get(&i.source)
                        && let Some(at) = self.text(i.span).rfind(i.source.as_str())
                    {
                        let a = i.span.start + at as u32;
                        edits.push((a, a + i.source.len() as u32, best.clone()));
                    }
                    self.push_edits("B0119", i.span, format!("imported file `{}` does not exist", i.source), fix, edits);
                }
            }
            Stmt::Server { func, span } => {
                self.push("B0704", *span, "`server:` only goes at the top level of the <script>", "move it out of the function or block: `server: async function name(ctx, data) { … }` at the very top");
                self.server_fn(func);
            }
            Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Empty { .. } => {}
        }
    }

    fn func(&mut self, f: &Func) {
        self.fn_depth += 1;
        self.scopes.push();
        for p in &f.params {
            self.scopes.declare_pat(p, Kind::Local);
        }
        for p in &f.params {
            self.pat_defaults(p);
        }
        match &f.body {
            FnBody::Expr(e) => self.expr(e),
            FnBody::Block(b) => self.block(b),
        }
        self.scopes.pop();
        self.fn_depth -= 1;
    }

    fn ident_ref(&mut self, id: &Ident) {
        let name = id.name.as_str();
        if let Some(k) = self.scopes.lookup(name) {
            if k == Kind::Import && !self.server_ctx && self.server_names.contains(name) {
                self.push(
                    "B0701",
                    id.span,
                    format!("`{name}` comes from a server module (`<meta server>`) and this code reaches the browser"),
                    format!("use `{name}` only inside `export async function load(ctx)` or `server: async function f(ctx, …)` and call that function from the client"),
                );
            }
            // Server code of a component runs outside any instance: it sees imports and other server functions only.
            if self.server_ctx
                && self.c.kind == FileKind::Component
                && k != Kind::Import
                && !self.server_fns.contains(name)
                && !self.shared.contains(name)
                && self.scopes.frames[1..].iter().all(|f| !f.contains_key(name))
            {
                self.push(
                    "B0704",
                    id.span,
                    format!("`{name}` belongs to the component and server code cannot see it"),
                    format!("pass it as an argument: `save({name})` on the client and `server: async function save(ctx, {name})` on the server"),
                );
            }
            return;
        }
        // `redirect(url, status?)` / `deny(reason)` (§22): route guards, middleware and server code; `notFound()` in
        // server code (`load` answers a real 404).
        let route_file = self.c.file.rsplit('/').next().is_some_and(|f| f.starts_with("_guard.") || f.starts_with("_middleware."));
        if matches!(name, "redirect" | "deny") && (self.server_ctx || route_file) {
            return;
        }
        if name == "notFound" && self.server_ctx {
            return;
        }
        if name == "import.meta" || (name == "process" && self.server_ctx) {
            return;
        }
        if name == "document" || name == "window" {
            if worker::is_worker(&self.c.file) {
                return; // B0127 reports it: there is no DOM at all in a worker.
            }
            self.push("B0201", id.span, format!("direct access to `{name}` from BrainScript"), "create `let el = ref()`, put `bind:ref={el}` on the node and use `el`");
            return;
        }
        // §33 collections: read on the server, where the content is; the page gets the result as props.
        if scope::CONTENT.contains(&name) {
            if !self.server_ctx {
                self.push("B0705", id.span, format!("`{name}()` reads content/ and only runs on the server"), format!("move it into `export async function load(ctx)` and return what the page needs: `return {{ posts: {name}('blog') }}`"));
            }
            return;
        }
        // Other frameworks' names for the same readers (Astro): normalized.
        if let Some(to) = match name { "getCollection" => Some("entries"), "getEntry" => Some("entry"), _ => None } {
            self.push_info("B0706", id.span, format!("`{name}` is `{to}` in BRAINLLM (normalized)"), format!("write `{to}(…)`"), vec![(id.span.start, id.span.end, to.to_string())]);
            return;
        }
        if PRIMITIVES.contains(&name) || KNOWN_GLOBALS.contains(&name) {
            return;
        }
        if name == "eval" {
            self.push("B0105", id.span, "`eval` does not exist in BrainScript", "write the code directly or use `JSON.parse` for data");
            return;
        }
        let names = self.scopes.all_names();
        let suggestion = scope::closest(name, names.iter().map(String::as_str).chain(PRIMITIVES.iter().copied()));
        if suggestion.is_none() && name.starts_with(|c: char| c.is_ascii_uppercase()) {
            return; // probably a Web API constructor we don't list
        }
        let (fix, edits) = match suggestion {
            Some(s) => (format!("did you mean `{s}`?"), vec![(id.span.start, id.span.end, s.to_string())]),
            None => (format!("declare it in the <script> (`let {name} = sig(…)`) or add it to `<meta props=\"{name}:any\">`"), Vec::new()),
        };
        self.push_edits("B0111", id.span, format!("`{name}` is not declared"), fix, edits);
    }

    /// Checks the target of `=`/`+=`/`++`.
    fn assign_target(&mut self, target: &Expr) {
        match target.unparen() {
            Expr::Ident(id) => match self.scopes.lookup(&id.name) {
                Some(Kind::Drv) => self.push("B0112", id.span, format!("`{}` is a `drv` and is read-only", id.name), format!("change the `sig`s that `{}` depends on, or turn it into a `sig`", id.name)),
                Some(Kind::Prop) if self.c.props.iter().any(|p| p.name == id.name && p.bindable) => {}
                Some(Kind::Prop) | Some(Kind::LoadProps) => self.push("B0112", id.span, format!("prop `{}` is read-only", id.name), format!("copy the prop into local state: `let {0}Local = sig({0})` and modify `{0}Local`", id.name)),
                Some(Kind::Const) => self.push("B0112", id.span, format!("`{}` is `const` and cannot be reassigned", id.name), format!("declare it with `let {}`", id.name)),
                Some(Kind::Func) | Some(Kind::Import) => self.push("B0112", id.span, format!("`{}` is a function or an import and cannot be reassigned", id.name), "use another name for the new value"),
                Some(Kind::EachItem) => self.push("B0114", id.span, format!("`{}` is the item of an `{{#each}}` and cannot be reassigned", id.name), "update the source list with `list = list.map(x => x.id === id ? updated : x)`"),
                Some(_) => {}
                None => self.ident_ref(id),
            },
            Expr::Member { obj, prop, .. } => {
                if let Some(root) = root_ident(obj) {
                    match self.scopes.lookup(&root.name) {
                        Some(Kind::Sig) => {
                            let field = match prop {
                                MemberProp::Name(n) => n.name.clone(),
                                MemberProp::Computed(_) => "field".into(),
                            };
                            self.push(
                                "B0113",
                                target.span(),
                                format!("mutating `{}` in place is not reactive", root.name),
                                format!("reassign the whole sig: `{0} = {{ ...{0}, {field}: value }}` (or `{0} = {0}.map(…)` if it is a list)", root.name),
                            );
                        }
                        Some(Kind::EachItem) => self.push(
                            "B0114",
                            target.span(),
                            format!("`{}` is the item of an `{{#each}}`; mutating it does not update the view", root.name),
                            format!("update the source list: `list = list.map(x => x.id === {0}.id ? {{ ...x, field: value }} : x)`", root.name),
                        ),
                        Some(Kind::Prop) => self.push("B0112", target.span(), format!("prop `{}` is read-only", root.name), format!("emit an event to the parent: `emit('change', {{ ...{0}, field: value }})`", root.name)),
                        _ => {}
                    }
                    if self.scopes.lookup(&root.name).is_none() {
                        self.ident_ref(root);
                    }
                }
                self.member_inner(obj, prop);
            }
            other => self.expr(other),
        }
    }

    fn member_inner(&mut self, obj: &Expr, prop: &MemberProp) {
        if !matches!(obj.unparen(), Expr::Ident(_)) {
            self.expr(obj);
        }
        if let MemberProp::Computed(p) = prop {
            self.expr(p);
        }
    }

    /// B0129: `location.assign('/x')`, `location.replace('/x')` or `location.href = '/x'` with an app path leaves the
    /// base path and reloads the page; `navigate('/x')` does neither.
    fn location_nav(&mut self, e: &Expr) {
        let is_loc = |x: &Expr, scopes: &scope::Scopes| match x.unparen() {
            Expr::Ident(i) => i.name == "location" && scopes.lookup("location").is_none(),
            Expr::Member { obj, prop: MemberProp::Name(p), .. } => p.name == "location" && matches!(obj.unparen(), Expr::Ident(w) if w.name == "window"),
            _ => false,
        };
        let app_path = |x: &Expr| match x.unparen() {
            Expr::Str { value, .. } => value.starts_with('/') && !value.starts_with("//"),
            Expr::Template { quasis, .. } => quasis.first().is_some_and(|q| q.starts_with('/') && !q.starts_with("//")),
            _ => false,
        };
        let (arg, replace, span) = match e {
            Expr::Call { callee, args, span, .. } => match callee.unparen() {
                Expr::Member { obj, prop: MemberProp::Name(m), .. } if matches!(m.name.as_str(), "assign" | "replace") && is_loc(obj, &self.scopes) => match args.first() {
                    Some(a) if app_path(a) => (a, m.name == "replace", *span),
                    _ => return,
                },
                _ => return,
            },
            Expr::Assign { op: "=", target, value, span } => match target.unparen() {
                Expr::Member { obj, prop: MemberProp::Name(p), .. } if p.name == "href" && is_loc(obj, &self.scopes) && app_path(value) => (value.as_ref(), false, *span),
                _ => return,
            },
            _ => return,
        };
        let path = self.text(arg.span()).to_string();
        let call = if replace { format!("navigate({path}, {{ replace: true }})") } else { format!("navigate({path})") };
        let msg = "an absolute app path through `location` leaves the base path and reloads the page";
        if scope::runtime_has("brain-router.js", "navigate") && self.scopes.lookup("navigate").is_none() {
            self.push_edits("B0129", span, msg, format!("use `{call}`"), vec![(span.start, span.end, call)]);
        } else {
            self.push("B0129", span, msg, "use a path relative to the base, or the router once `navigate()` ships (B0128)");
        }
    }

    /// B0133: `f.v = { … }` / `f.err = { … }` on a `form()`. Those objects hold the live fields the inputs are bound
    /// to; replacing them disconnects the form (the runtime throws, F10). Values load with `f.reset(values)`.
    fn form_replace(&mut self, e: &Expr) {
        let Expr::Assign { op, target, value, span } = e else { return };
        let Expr::Member { obj, prop: MemberProp::Name(p), .. } = target.unparen() else { return };
        let Expr::Ident(id) = obj.unparen() else { return };
        if !matches!(p.name.as_str(), "v" | "err") || self.scopes.lookup(&id.name) != Some(Kind::Form) {
            return;
        }
        let f = &id.name;
        if p.name == "v" && *op == "=" {
            let call = format!("{f}.reset({})", self.text(value.span()));
            self.push_edits(
                "B0133",
                *span,
                format!("`{f}.v` cannot be replaced: the inputs stay bound to the old fields"),
                format!("load the values with `{call}`, or start from them with `form(schema, {{ init: … }})`"),
                vec![(span.start, span.end, call)],
            );
        } else {
            let fix = if p.name == "v" { format!("assign one field at a time (`{f}.v.email = …`) or call `{f}.reset(values)`") } else { format!("mark server errors with `{f}.errors({{ field: 'message' }})`") };
            self.push("B0133", *span, format!("`{f}.{}` cannot be replaced", p.name), fix);
        }
    }

    /// B0130: `wide.v` on a reactive cell. Signals have one form: `{wide}`, `wide + 1`; the compiler adds `.v`.
    fn explicit_v(&mut self, e: &Expr) {
        let Expr::Member { obj, prop: MemberProp::Name(p), span, .. } = e else { return };
        let Expr::Ident(id) = obj.unparen() else { return };
        if p.name != "v" || !matches!(self.scopes.lookup(&id.name), Some(Kind::Sig | Kind::Drv | Kind::Ref)) {
            return;
        }
        let start = obj.span().end;
        self.push_edits("B0130", *span, format!("`{0}.v`: read the signal directly as `{0}`", id.name), format!("write `{}`", id.name), vec![(start, span.end, String::new())]);
    }

    fn expr(&mut self, e: &Expr) {
        self.location_nav(e);
        self.form_replace(e);
        self.explicit_v(e);
        match e {
            Expr::Ident(id) => self.ident_ref(id),
            Expr::This(span) => self.push("B0102", *span, "`this` does not exist in BrainScript", "use the variable directly: `n` instead of `this.n`"),
            Expr::Str { value, span, .. } => self.secrets(value, *span),
            Expr::Num { .. } | Expr::Regex { .. } | Expr::Bool { .. } | Expr::Null(_) => {}
            Expr::Template { tag, exprs, quasis, span } => {
                self.secrets(&quasis.concat(), *span);
                if let Some(t) = tag {
                    self.expr(t);
                }
                for x in exprs {
                    self.expr(x);
                }
            }
            Expr::Array { elems, .. } => elems.iter().flatten().for_each(|x| self.expr(x)),
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjProp::KeyValue { key, value, .. } => {
                            if let PropKey::Computed(k) = key {
                                self.expr(k);
                            }
                            self.expr(value);
                        }
                        ObjProp::Shorthand(id) => self.ident_ref(id),
                        ObjProp::Method { func, .. } => self.func(func),
                        ObjProp::Spread { arg, .. } => self.expr(arg),
                    }
                }
            }
            Expr::Func(f) => self.func(f),
            Expr::Unary { arg, op, .. } => {
                if *op == "delete" {
                    self.assign_target(arg);
                } else {
                    self.expr(arg);
                }
            }
            Expr::Update { arg, .. } => self.assign_target(arg),
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Assign { target, value, .. } => {
                self.assign_target(target);
                self.expr(value);
            }
            Expr::Cond { test, cons, alt, .. } => {
                self.expr(test);
                self.expr(cons);
                self.expr(alt);
            }
            Expr::Call { callee, args, .. } => {
                self.call(callee, args, e.span());
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::New { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Member { .. } if self.env_access(e) => {}
            Expr::Member { obj, prop: MemberProp::Name(m), .. }
                if matches!(obj.unparen(), Expr::Ident(o) if self.api_vars.contains_key(&o.name) && matches!(self.scopes.lookup(&o.name), Some(Kind::Sig | Kind::Let))) =>
            {
                let Expr::Ident(o) = obj.unparen() else { unreachable!() };
                let (child, expose) = self.api_vars[&o.name].clone();
                if !expose.contains(&m.name) {
                    let fix = if expose.is_empty() { format!("`<{child}>` exposes nothing: declare `<meta expose=\"{}\">` in it", m.name) } else { format!("`<{child}>` exposes: {}; or add `{}` to its `<meta expose>`", expose.join(", "), m.name) };
                    self.push("B0804", m.span, format!("`{}.{}` is not exposed by `<{child}>`", o.name, m.name), fix);
                }
                self.ident_ref(o);
            }
            Expr::Member { obj, prop, .. } => {
                self.expr(obj);
                if let MemberProp::Computed(p) = prop {
                    self.expr(p);
                }
            }
            Expr::Seq { exprs, .. } => exprs.iter().for_each(|x| self.expr(x)),
            Expr::Spread { arg, .. } | Expr::Await { arg, .. } | Expr::Paren { expr: arg, .. } => self.expr(arg),
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], span: Span) {
        if let Expr::Ident(p) = callee.unparen()
            && p.name == "provide"
            && self.scopes.lookup("provide").is_none()
            && let Some(Expr::Call { callee: sc, span: ss, .. }) = args.get(1).map(Expr::unparen)
            && matches!(sc.unparen(), Expr::Ident(k) if k.name == "store")
        {
            self.scoped_stores.insert(ss.start);
        }
        // `console.x(…)` → `log(…)` (B1308, normalized): `log` is removed from production builds.
        if let Expr::Member { obj, prop: MemberProp::Name(m), .. } = callee.unparen()
            && let Expr::Ident(o) = obj.unparen()
            && o.name == "console"
            && self.scopes.lookup("console").is_none()
            && self.scopes.lookup("log").is_none()
        {
            let args_src = match (args.first(), args.last()) {
                (Some(a), Some(b)) => self.c.src.get(a.span().start as usize..b.span().end as usize).unwrap_or("").to_string(),
                _ => String::new(),
            };
            let cs = callee.span();
            self.push_info("B1308", span, format!("`console.{}` in BrainScript (normalized to `log`)", m.name), format!("use `log({args_src})`: it is stripped in build"), vec![(cs.start, cs.end, "log".into())]);
        }
        match callee.unparen() {
            Expr::Ident(id) if self.scopes.lookup(&id.name).is_none() => match id.name.as_str() {
                "sig" | "drv" | "ref" => {
                    // Reached only for calls that are not a top-level `let x = sig(…)`.
                    self.push(
                        "B0115",
                        span,
                        format!("`{}()` can only be declared at the top level of the <script>", id.name),
                        format!("move it up: `let name = {}(…)` outside functions, ifs and loops", id.name),
                    );
                }
                "emit"
                    if !matches!(args.first().map(Expr::unparen), Some(Expr::Str { .. })) => {
                        self.push("B0118", span, "the first argument of `emit` must be the event name in quotes", "write `emit('save', data)`");
                    }
                "emit" => {
                    if let Some(Expr::Str { value, span: es, .. }) = args.first().map(Expr::unparen) {
                        let declared = meta_names(self.c, "emits");
                        if !declared.contains(value) {
                            let fix = if declared.is_empty() {
                                format!("declare it: `<meta emits=\"{value}\">` (add `:type` if it carries data, e.g. `{value}:obj`)")
                            } else {
                                format!("add it to the declaration: `<meta emits=\"{} {value}\">`, or emit one of: {}", declared.join(" "), declared.join(", "))
                            };
                            // §21: adding the declaration is mechanical, so it is a normalization.
                            let edit = match self.c.meta.iter().find(|m| m.name == "emits") {
                                Some(m) => {
                                    let text = self.text(m.span);
                                    let q = text.ends_with('"') || text.ends_with('\'');
                                    let at = if q { m.span.end - 1 } else { m.span.end };
                                    let sep = if m.value.as_deref().is_some_and(|v| !v.trim().is_empty()) { " " } else { "" };
                                    (at, at, format!("{sep}{value}"))
                                }
                                None => (0, 0, format!("<meta emits=\"{value}\">\n")),
                            };
                            self.push_info("B0802", *es, format!("event `{value}` is not declared in `<meta emits>` (declared)"), fix, vec![edit]);
                        }
                    }
                }
                "store" | "bus" if !in_channels(&self.c.file) && !(id.name == "store" && self.scoped_stores.contains(&span.start)) => {
                    let fix = if id.name == "store" {
                        "for state shared by one subtree write `provide(Key, store({ … }))` in its root; for app-wide state move it to `channels/<name>.jsbrain` as `export default store(…)`".to_string()
                    } else {
                        format!("move it to `channels/<name>.jsbrain` as `export default {}(…)` and import it here", id.name)
                    };
                    self.push("B0806", span, format!("`{}()` only exists in `channels/*.jsbrain`", id.name), fix);
                }
                "use" => {
                    if let Some(Expr::Ident(k)) = args.first().map(Expr::unparen)
                        && let Some(has_default) = self.ctx.keys.get(&k.name)
                        && !has_default
                        && self.ctx.provided.as_ref().is_some_and(|p| !p.contains(&k.name))
                    {
                        let mut d = Diag::new("B0807", &self.c.file, span, format!("`use({})` but no component provides `{}`", k.name, k.name), format!("call `provide({0}, value)` in an ancestor, or give the key a default: `key('…', default)`", k.name));
                        d.level = brain_parse::Severity::Warning;
                        self.diags.push(d);
                    }
                }
                _ => {}
            },
            Expr::Member { obj, prop: MemberProp::Name(m), .. } if MUTATING.contains(&m.name.as_str()) => {
                if let Expr::Ident(root) = obj.unparen()
                    && self.scopes.lookup(&root.name) == Some(Kind::Sig)
                {
                    let r = &root.name;
                    let fix = match m.name.as_str() {
                        "push" => format!("reassign a copy: `{r} = [...{r}, item]`"),
                        "unshift" => format!("reassign a copy: `{r} = [item, ...{r}]`"),
                        "pop" => format!("reassign a copy: `{r} = {r}.slice(0, -1)`"),
                        "shift" => format!("reassign a copy: `{r} = {r}.slice(1)`"),
                        "splice" => format!("reassign a copy: `{r} = {r}.toSpliced(i, n)` or `{r} = {r}.filter(…)`"),
                        "sort" => format!("reassign a copy: `{r} = {r}.toSorted(cmp)`"),
                        "reverse" => format!("reassign a copy: `{r} = {r}.toReversed()`"),
                        _ => format!("reassign a new copy with the change: `{r} = new Map({r})` / `[...{r}]`"),
                    };
                    // push/unshift with arguments rewrite mechanically; the rest needs judgment.
                    let mut edits = Vec::new();
                    if let (Some(first), Some(last)) = (args.first(), args.last()) {
                        let items = self.c.src.get(first.span().start as usize..last.span().end as usize).unwrap_or("").to_string();
                        match m.name.as_str() {
                            "push" => edits.push((span.start, span.end, format!("{r} = [...{r}, {items}]"))),
                            "unshift" => edits.push((span.start, span.end, format!("{r} = [{items}, ...{r}]"))),
                            _ => {}
                        }
                    }
                    self.push_edits("B0113", span, format!("`{r}.{}()` mutates the sig in place and the view is not notified", m.name), fix, edits);
                }
            }
            _ => {}
        }
    }

    // ───────────────────────── template ─────────────────────────

    /// B0410: `aria-controls`/`aria-describedby`/`aria-labelledby` pointing at an id that only exists inside an `{#if}` or
    /// `{#each}`: while it is closed the reference is dangling (QA #21 F9).
    fn aria_refs(&mut self) {
        // Each node carries the stack of conditional blocks (`{#if}` branch, `{#each}`, `{#await}` branch) it renders
        // under. A reference is safe when the target's stack is a prefix of its own: wherever the reference renders,
        // the target does too (same `{#each}` iteration included). #308 N2: computed values pair by their source shape
        // (`aria-controls={'faq-' + it.id}` ↔ `id={'faq-' + it.id}`), and an `{#if}` nested in the same `{#each}`
        // as the reference counts as conditional.
        type Def = (String, Vec<u32>);
        type Ref = (String, String, Span, Vec<u32>);
        struct W {
            next: u32,
            defs: Vec<Def>,
            refs: Vec<Ref>,
        }
        impl W {
            fn block(&mut self, nodes: &[Node], stack: &[u32]) {
                self.next += 1;
                let mut s = stack.to_vec();
                s.push(self.next);
                self.walk(nodes, &s);
            }
            fn walk(&mut self, nodes: &[Node], stack: &[u32]) {
                for n in nodes {
                    match n {
                        Node::Element(e) => {
                            for a in &e.attrs {
                                if a.kind != AttrKind::Plain {
                                    continue;
                                }
                                if a.name == "id" {
                                    if let Some(k) = label_key(&a.value) {
                                        self.defs.push((k, stack.to_vec()));
                                    }
                                } else if matches!(a.name.as_str(), "aria-controls" | "aria-describedby" | "aria-labelledby") {
                                    match &a.value {
                                        AttrValue::Text(t) => {
                                            for r in t.split_whitespace() {
                                                self.refs.push((a.name.clone(), r.to_string(), a.span, stack.to_vec()));
                                            }
                                        }
                                        v => {
                                            if let Some(k) = label_key(v) {
                                                self.refs.push((a.name.clone(), k, a.span, stack.to_vec()));
                                            }
                                        }
                                    }
                                }
                            }
                            self.walk(&e.children, stack);
                        }
                        Node::ComponentRef(r) => self.walk(&r.children, stack),
                        Node::If(b) => {
                            for br in &b.branches {
                                self.block(&br.body, stack);
                            }
                            if let Some(o) = &b.otherwise {
                                self.block(o, stack);
                            }
                        }
                        Node::Each(b) => {
                            self.block(&b.body, stack);
                            if let Some(e) = &b.empty {
                                self.block(e, stack);
                            }
                        }
                        Node::Await(a) => {
                            self.block(&a.pending, stack);
                            for br in [&a.then, &a.catch].into_iter().flatten() {
                                self.block(&br.body, stack);
                            }
                        }
                        Node::SlotScope(b) => self.walk(&b.body, stack),
                        _ => {}
                    }
                }
            }
        }
        let mut w = W { next: 0, defs: Vec::new(), refs: Vec::new() };
        w.walk(&self.c.template, &[]);
        for (attr, id, span, ref_stack) in w.refs {
            let found: Vec<&Vec<u32>> = w.defs.iter().filter(|(d, _)| *d == id).map(|(_, s)| s).collect();
            if !found.is_empty() && !found.iter().any(|s| ref_stack.starts_with(s)) {
                let shown = if id.starts_with('{') { format!("{attr}={id}") } else { format!("{attr}=\"{id}\"") };
                let target = if id.starts_with('{') { format!("the element with `id={id}`") } else { format!("`#{id}`") };
                self.push("B0410", span, format!("`{shown}` points at an element that only exists inside `{{#if}}`/`{{#each}}`"), format!("keep {target} in the DOM and hide it with the `hidden` attribute (`hidden={{!open}}`), so the reference never dangles"));
            }
        }
    }

    /// B0132: `app.brain` is the static document shell (read as HTML by the build and the dev server); expressions,
    /// blocks and components in it are never compiled per page (QA #153).
    fn shell_static(&mut self) {
        fn go(nodes: &[Node], out: &mut Vec<(Span, String)>) {
            for n in nodes {
                match n {
                    Node::Element(e) => go(&e.children, out),
                    Node::Interp(i) => out.push((i.span, format!("`{{{}}}`", brain_parse::dump::expr(&i.expr)))),
                    Node::ComponentRef(r) => out.push((r.span, format!("`<{}>`", r.name))),
                    Node::If(b) => out.push((b.span, "`{#if}`".into())),
                    Node::Each(b) => out.push((b.span, "`{#each}`".into())),
                    Node::Await(b) => out.push((b.span, "`{#await}`".into())),
                    _ => {}
                }
            }
        }
        if self.c.kind != FileKind::App {
            return;
        }
        let mut hits = Vec::new();
        go(&self.c.template, &mut hits);
        for (span, what) in hits {
            self.push("B0132", span, format!("{what} in `app.brain`: the shell is static HTML, never compiled per page"), "move it into `views/_layout.brain`, which wraps every view and renders with the page's locale and data");
        }
    }

    fn template(&mut self) {
        let nodes = &self.c.template;
        self.nodes(nodes);
    }

    fn nodes(&mut self, nodes: &[Node]) {
        for n in nodes {
            self.node(n);
        }
    }

    fn node(&mut self, n: &Node) {
        match n {
            Node::Text(t) => self.secrets(&t.value, t.span),
            Node::Slot(sl) => self.attrs(&sl.attrs, None),
            // Slot props are read-only values the child passes (§41.6).
            Node::SlotScope(b) => {
                self.scopes.push();
                for n in &b.names {
                    self.scopes.declare(&n.name, Kind::Drv);
                }
                self.nodes(&b.body);
                self.scopes.pop();
            }
            Node::Interp(i) => {
                if i.html && !matches!(i.expr.unparen(), Expr::Call { callee, .. } if matches!(callee.as_ref(), Expr::Ident(id) if id.name == "sanitize")) {
                    self.push("B0210", i.span, "raw HTML without `sanitize`", "wrap the value: `{@html sanitize(value)}`");
                }
                self.expr(&i.expr)
            }
            Node::Element(e) => {
                // B0216: `transition:` kinds the runtime knows (brain-anim.js throws on any other).
                for a in e.attrs.iter().filter(|a| a.kind == AttrKind::Transition) {
                    if !matches!(a.name.as_str(), "fade" | "slide" | "scale") {
                        let near = scope::closest(&a.name, ["fade", "slide", "scale"]).unwrap_or("fade");
                        let at = a.span.start + "transition:".len() as u32;
                        self.push_edits("B0216", a.span, format!("`transition:{}` does not exist", a.name), format!("use `transition:{near}` (fade, slide or scale)"), vec![(at, at + a.name.len() as u32, near.to_string())]);
                    }
                }
                // B0217: `<component is={C}>` renders C; without `is` it renders nothing.
                if e.tag == "component" && !e.attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == "is" && matches!(a.value, AttrValue::Expr(_))) {
                    self.push("B0217", e.span, "`<component>` without `is={Component}`", "write `<component is={Current} …props />` with a component imported or held in a signal");
                }
                self.attrs(&e.attrs, Some(&e.tag));
                self.nodes(&e.children);
            }
            Node::ComponentRef(r) => {
                // A `{#slot}` block is the default slot itself: it cannot share it with other content.
                let rest = r.children.iter().filter(|n| !matches!(n, Node::Text(t) if t.value.trim().is_empty()) && !matches!(n, Node::SlotScope(_)));
                if r.children.iter().any(|n| matches!(n, Node::SlotScope(_))) && rest.clone().any(|n| !matches!(n, Node::Element(e) if e.attrs.iter().any(|a| a.name == "slot"))) {
                    self.push("B0218", r.span, format!("`<{}>` mixes `{{#slot}}` with other default-slot content", r.name), "put all the default content inside the `{#slot …}…{/slot}` block");
                }
                self.component_ref(r);
                self.attrs(&r.attrs, None);
                self.nodes(&r.children);
            }
            Node::If(b) => {
                for br in &b.branches {
                    self.expr(&br.test);
                    self.nodes(&br.body);
                }
                if let Some(o) = &b.otherwise {
                    self.nodes(o);
                }
            }
            Node::Each(b) => {
                self.expr(&b.list);
                self.scopes.push();
                self.scopes.declare_pat(&b.item, Kind::EachItem);
                if let Some(i) = &b.index {
                    self.scopes.declare(&i.name, Kind::EachItem);
                }
                match &b.key {
                    Some(k) => self.expr(k),
                    None => {
                        let item = brain_parse::dump::pat(&b.item);
                        let list = brain_parse::dump::expr(&b.list);
                        // A list of primitives (the item is never used as an object) is keyed by value.
                        let mut used = String::new();
                        node_exprs(&b.body, &mut used);
                        let object = matches!(b.item, Pat::Object { .. }) || used.contains(&format!("{item}.")) || used.contains(&format!("{item}?.")) || used.contains(&format!("{item}["));
                        let fix = if object {
                            format!("add the key after the item: `{{#each {list} as {item} ({item}.id)}}`")
                        } else {
                            format!("key by value: `{{#each {list} as {item} ({item})}}`")
                        };
                        // Insert right after the item (or the index): ` (it.id)` / ` (it)`; a destructured item
                        // only when it has `id`.
                        let at = b.index.as_ref().map(|i| i.span.end).unwrap_or(b.item.span().end);
                        let key = match &b.item {
                            Pat::Ident(_) if object => Some(format!("{item}.id")),
                            Pat::Ident(_) => Some(item.clone()),
                            Pat::Object { props, .. } if props.iter().any(|p| matches!(&p.key, PropKey::Ident(n) if n.name == "id")) => Some("id".into()),
                            _ => None,
                        };
                        match key {
                            Some(k) => self.push_info("B0202", b.span, format!("`{{#each}}` without a key (normalized to `({k})`)"), fix, vec![(at, at, format!(" ({k})"))]),
                            None => self.push("B0202", b.span, "`{#each}` without a key", fix),
                        }
                    }
                }
                self.nodes(&b.body);
                self.scopes.pop();
                if let Some(e) = &b.empty {
                    self.nodes(e);
                }
            }
            Node::Await(b) => {
                self.expr(&b.promise);
                self.nodes(&b.pending);
                for br in [&b.then, &b.catch].into_iter().flatten() {
                    self.scopes.push();
                    if let Some(p) = &br.binding {
                        self.scopes.declare_pat(p, Kind::Local);
                    }
                    self.nodes(&br.body);
                    self.scopes.pop();
                }
            }
        }
    }

    /// `class={c ? 'a' : ''}` / `class="x {c ? 'a' : 'b'}"` → `class="x" class:a={c} class:b={!(c)}` (B0212, normalized).
    fn class_ternary(&mut self, a: &Attr, all: &[Attr]) {
        if a.kind != AttrKind::Plain || a.name != "class" {
            return;
        }
        let parts: Vec<AttrPart> = match &a.value {
            AttrValue::Expr(e) => vec![AttrPart::Expr(e.clone())],
            AttrValue::Mixed(p) => p.clone(),
            _ => return,
        };
        let valid = |c: &str| c.split_whitespace().all(|w| w.chars().all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_'));
        let mut fixed = Vec::new();
        let mut toggles: Vec<(String, String)> = Vec::new();
        for p in &parts {
            match p {
                AttrPart::Text(t) => fixed.push(t.trim().to_string()),
                AttrPart::Expr(e) => {
                    let Expr::Cond { test, cons, alt, .. } = e.unparen() else { return };
                    let (Expr::Str { value: on, .. }, Expr::Str { value: off, .. }) = (cons.unparen(), alt.unparen()) else { return };
                    if !valid(on) || !valid(off) {
                        return;
                    }
                    let t = self.text(test.span()).to_string();
                    for c in on.split_whitespace() {
                        toggles.push((c.to_string(), t.clone()));
                    }
                    for c in off.split_whitespace() {
                        toggles.push((c.to_string(), format!("!({t})")));
                    }
                }
            }
        }
        if toggles.is_empty() || toggles.iter().any(|(c, _)| all.iter().any(|x| x.kind == AttrKind::Class && &x.name == c)) {
            return;
        }
        let stat = fixed.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
        let mut text = if stat.is_empty() { String::new() } else { format!("class=\"{stat}\"") };
        for (c, t) in &toggles {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&format!("class:{c}={{{t}}}"));
        }
        self.push_info("B0212", a.span, "conditional inside `class=` (normalized to `class:x`)", format!("write `{text}`"), vec![(a.span.start, a.span.end, text)]);
    }

    fn attrs(&mut self, attrs: &[Attr], tag: Option<&str>) {
        for a in attrs {
            self.class_ternary(a, attrs);
        }
        let mut seen = HashSet::new();
        for a in attrs {
            let full = match a.kind {
                AttrKind::Plain => a.name.clone(),
                AttrKind::On => format!("on:{}", a.name),
                AttrKind::Bind => format!("bind:{}", a.name),
                AttrKind::Class => format!("class:{}", a.name),
                AttrKind::Style => format!("style:{}", a.name),
                AttrKind::Transition => format!("transition:{}", a.name),
            };
            if !seen.insert(full.clone()) {
                self.push("B0209", a.span, format!("duplicate attribute `{full}`"), "keep a single occurrence of the attribute");
            }
            match (&a.kind, &a.value) {
                (AttrKind::Bind, v) => self.bind(a, v, tag),
                (AttrKind::On, AttrValue::Expr(e)) => self.expr(e),
                (AttrKind::On, _) => self.push("B0208", a.span, format!("`on:{}` needs a handler in braces", a.name), format!("write `on:{}={{handler}}` or `on:{}={{n++}}`", a.name, a.name)),
                (_, AttrValue::Expr(e)) => self.expr(e),
                (_, AttrValue::Mixed(parts)) => {
                    for p in parts {
                        match p {
                            AttrPart::Expr(e) => self.expr(e),
                            AttrPart::Text(t) => self.secrets(t, a.span),
                        }
                    }
                }
                (_, AttrValue::Text(t)) => self.secrets(t, a.span),
                _ => {}
            }
        }
    }

    fn bind(&mut self, a: &Attr, v: &AttrValue, tag: Option<&str>) {
        if a.name == "api" && tag.is_none() {
            match v {
                AttrValue::Expr(e) if matches!(e.unparen(), Expr::Ident(_)) => self.expr(e),
                _ => self.push("B0204", a.span, "`bind:api` needs a variable", "declare `let m = sig(null)` and write `bind:api={m}`; then call `m.open()`"),
            }
            return;
        }
        // On a component, `bind:<prop>` binds a `~` model prop (checked in `component_ref`); on an element only
        // value/checked/ref exist.
        if tag.is_some() && !matches!(a.name.as_str(), "value" | "checked" | "ref") {
            self.push("B0205", a.span, format!("`bind:{}` does not exist", a.name), "use `bind:value`, `bind:checked` or `bind:ref`");
            return;
        }
        let AttrValue::Expr(e) = v else {
            self.push("B0204", a.span, format!("`bind:{}` needs a sig in braces", a.name), format!("write `bind:{}={{name}}` with `let name = sig(…)`", a.name));
            return;
        };
        match e.unparen() {
            Expr::Ident(id) => match self.scopes.lookup(&id.name) {
                Some(Kind::Sig) | Some(Kind::Ref) => {}
                // A `~` model prop: the child writes it back to the parent's binding (INTERNALS §12).
                Some(Kind::Prop) if a.name != "ref" && self.c.props.iter().any(|p| p.name == id.name && p.bindable) => {}
                None => self.ident_ref(id),
                Some(k) => {
                    let what = match k {
                        Kind::Drv => "a `drv` (read-only)",
                        Kind::Prop | Kind::LoadProps => "a prop (read-only)",
                        Kind::EachItem => "the item of an `{#each}`",
                        _ => "a plain variable",
                    };
                    self.push("B0204", e.span(), format!("`bind:{}` needs a `sig`, and `{}` is {what}", a.name, id.name), format!("declare `let {0} = sig(…)` in the <script> and use `bind:{1}={{{0}}}`", id.name, a.name));
                }
            },
            // `bind:checked={prefs.email}` writes `prefs = { ...prefs, email: v }` (SPEC: field of a sig object).
            Expr::Member { .. } if a.name != "ref" && root_ident(e).is_some_and(|r| matches!(self.scopes.lookup(&r.name), Some(Kind::Sig | Kind::Form))) => self.expr(e),
            Expr::Member { .. } if a.name != "ref" && root_ident(e).is_some_and(|r| self.scopes.lookup(&r.name).is_none()) => self.expr(e),
            _ => self.push("B0204", e.span(), format!("`bind:{}` needs a `sig` or a field of a `sig`", a.name), format!("declare `let data = sig({{ field: '' }})` and use `bind:{}={{data.field}}`", a.name)),
        }
    }

    /// B0219: spacing props of the kit layout components take a step of the `--space-N` scale (0–6), and
    /// `<Container size>` one of sm, md, lg, xl, full (RUNTIME #82).
    fn kit_spacing(&mut self, r: &ComponentRef) {
        let spacing: &[&str] = match r.name.as_str() {
            "Stack" | "Row" | "Grid" | "Sidebar" => &["gap"],
            "Center" | "Container" => &["gutter"],
            "Spacer" => &["size"],
            _ => return,
        };
        for a in r.attrs.iter().filter(|a| a.kind == AttrKind::Plain) {
            let AttrValue::Text(v) = &a.value else { continue };
            let v = v.trim();
            if r.name == "Container" && a.name == "size" {
                if !matches!(v, "sm" | "md" | "lg" | "xl" | "full") {
                    self.push("B0219", a.span, format!("`<Container size=\"{v}\">` is not a container size"), "use sm, md, lg, xl or full");
                }
                continue;
            }
            if !spacing.contains(&a.name.as_str()) || (v.len() == 1 && ('0'..='6').contains(&v.chars().next().unwrap_or('x'))) {
                continue;
            }
            // `var(--space-3)` or a length equal to a token value → that step.
            let step = v.strip_prefix("var(--space-").and_then(|x| x.strip_suffix(')')).map(str::to_string).or_else(|| {
                self.ctx.tokens.as_ref().and_then(|t| t.iter().find(|(n, val)| n.starts_with("--space-") && val.trim() == v).map(|(n, _)| n.trim_start_matches("--space-").to_string()))
            }).filter(|s| s.len() == 1 && ('0'..='6').contains(&s.chars().next().unwrap_or('x')));
            let msg = format!("`<{} {}=\"{v}\">`: spacing props take a step of the scale (0–6)", r.name, a.name);
            match step {
                Some(n) => {
                    let text = self.text(a.span).to_string();
                    let at = text.find(v).map(|i| a.span.start + i as u32);
                    match at {
                        Some(at) => self.push_edits("B0219", a.span, msg, format!("write `{}=\"{n}\"` (var(--space-{n}))", a.name), vec![(at, at + v.len() as u32, n)]),
                        None => self.push("B0219", a.span, msg, format!("write `{}=\"{n}\"`", a.name)),
                    }
                }
                None => self.push("B0219", a.span, msg, format!("write `{}=\"3\"`: steps 0–6 map to var(--space-0…6)", a.name)),
            }
        }
    }

    fn component_ref(&mut self, r: &ComponentRef) {
        // B0220: `lazy` takes no value (visible), `idle` or `interaction`.
        if let Some(a) = r.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == "lazy") {
            match &a.value {
                AttrValue::None => {}
                AttrValue::Text(t) if matches!(t.as_str(), "idle" | "interaction" | "visible") => {}
                AttrValue::Text(t) => {
                    let near = scope::closest(t, ["idle", "interaction"]).unwrap_or("idle");
                    let text = self.text(a.span).to_string();
                    let edits = text.find(t.as_str()).map(|i| vec![(a.span.start + i as u32, a.span.start + (i + t.len()) as u32, near.to_string())]).unwrap_or_default();
                    self.push_edits("B0220", a.span, format!("`lazy=\"{t}\"` is not a loading mode"), format!("use `lazy` (when visible), `lazy=\"idle\"` or `lazy=\"interaction\"`; did you mean `{near}`?"), edits);
                }
                _ => self.push("B0220", a.span, "`lazy` takes a literal mode", "use `lazy`, `lazy=\"idle\"` or `lazy=\"interaction\"`"),
            }
        }
        // `<Img src alt width height>` (§22) is built in: dimensions prevent layout shift, alt is required.
        if r.name == "Img" && self.scopes.lookup("Img").is_none() {
            let has = |n: &str| r.attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == n);
            if !has("alt") {
                self.push("B0401", r.span, "`<Img>` without `alt`", "add `alt=\"description\"` (or `alt=\"\"` if it is decorative)");
            }
            let missing: Vec<&str> = ["width", "height"].into_iter().filter(|n| !has(n)).collect();
            if !missing.is_empty() {
                self.push("B0221", r.span, format!("`<Img>` without `{}`", missing.join("` and `")), "add the intrinsic size, e.g. `width=\"800\" height=\"600\"`: the browser reserves the space and nothing shifts");
            }
            return;
        }
        let Some(kind) = self.scopes.lookup(&r.name) else {
            // `<Portal to="#layer">` is built in (§41.6, brain-env.js).
            if scope::UI_KIT.contains(&r.name.as_str()) || r.name == "Portal" {
                self.kit_spacing(r);
                return;
            }
            self.push("B0203", r.span, format!("component `<{}>` is not imported", r.name), format!("add `import {} from './{}.brain'` to the <script>", r.name, kebab(&r.name)));
            return;
        };
        if kind != Kind::Import {
            return;
        }
        let source = self.c.script.iter().find_map(|s| match s {
            Stmt::Import(i) if i.default.as_ref().is_some_and(|d| d.name == r.name) => Some(i.source.clone()),
            _ => None,
        });
        if let Some(child) = source.as_ref().and_then(|s| self.ctx.children.get(s)) {
            for a in &r.attrs {
                if a.kind == AttrKind::On && !child.emits.contains(&a.name) {
                    let fix = if child.emits.is_empty() { format!("`<{}>` emits no events; declare `<meta emits=\"{}\">` in it or remove `on:{}`", r.name, a.name, a.name) } else { format!("`<{}>` emits: {}", r.name, child.emits.join(", ")) };
                    self.push("B0803", a.span, format!("`<{}>` does not declare the event `{}`", r.name, a.name), fix);
                }
            }
        }
        let Some(Imported::Brain(props)) = source.and_then(|s| self.ctx.imports.get(&s)) else { return };
        let props = props.clone();
        for a in &r.attrs {
            // `lazy` is a loading directive of the framework, not a prop of the child (§22).
            if a.kind != AttrKind::Plain || a.name == "lazy" {
                continue;
            }
            if !props.iter().any(|p| p.name == a.name) {
                let known: Vec<&str> = props.iter().map(|p| p.name.as_str()).collect();
                let fix = match scope::closest(&a.name, known.iter().copied()) {
                    Some(s) => format!("did you mean `{s}`?"),
                    None if known.is_empty() => format!("`<{}>` declares no props; remove the attribute", r.name),
                    None => format!("props of `<{}>`: {}", r.name, known.join(", ")),
                };
                self.push("B0206", a.span, format!("`<{}>` has no prop `{}`", r.name, a.name), fix);
            }
        }
        for a in &r.attrs {
            if a.kind != AttrKind::Bind || matches!(a.name.as_str(), "ref" | "api") {
                continue;
            }
            match props.iter().find(|p| p.name == a.name) {
                Some(p) if p.bindable => {}
                Some(p) => self.push(
                    "B0205",
                    a.span,
                    format!("prop `{}` of `<{}>` is not bindable", a.name, r.name),
                    format!("declare it as a model prop in `<{}>`: `{}:{}~`, or pass it with `{}={{…}}` and listen with `on:event`", r.name, p.name, p.ty.as_str(), p.name),
                ),
                None => self.push("B0206", a.span, format!("`<{}>` has no prop `{}`", r.name, a.name), format!("declare `{}:str~` in `<{}>` to bind it", a.name, r.name)),
            }
        }
        for p in &props {
            if !p.optional && p.default.is_none() && !r.attrs.iter().any(|a| matches!(a.kind, AttrKind::Plain | AttrKind::Bind) && a.name == p.name) {
                self.push("B0207", r.span, format!("missing required prop `{}` on `<{}>`", p.name, r.name), format!("add `{}={{…}}` (type {})", p.name, p.ty.as_str()));
            }
        }
    }

    // ───────────────────────── CSS ─────────────────────────

    fn css(&mut self) {
        let Some(sheet) = &self.c.style else { return };
        for (sel, span) in brain_css::global_selectors(sheet) {
            self.push("B0301", span, format!("global selector `{sel}` in a component <style>"), "move the global style to the `reset` layer or use a component class");
        }
        let tags = element_tags(&self.c.template);
        for (sel, why, span) in brain_css::dead_scoped_selectors(sheet, tags.as_ref()) {
            self.push("B0307", span, format!("`{sel}` never matches: {why}"), "remove the selector, or style a class this component writes (page-level styles go in the `reset` layer)");
        }
        let sheet = match brain_css::expand_mixins(sheet, &self.c.file) {
            Ok(s) => s,
            Err(ds) => {
                self.diags.extend(ds);
                return;
            }
        };
        // B0308: `--line` is the border token (SPEC tokens); as a fill or text color it breaks contrast and themes.
        fn decls<'a>(items: &'a [brain_css::CssItem], out: &mut Vec<(&'a str, &'a str, Span)>) {
            for it in items {
                match it {
                    brain_css::CssItem::Decl { name, value, span } => out.push((name, value, *span)),
                    brain_css::CssItem::Rule { items, .. } => decls(items, out),
                    brain_css::CssItem::At { block: Some(b), .. } => decls(b, out),
                    _ => {}
                }
            }
        }
        let mut ds = Vec::new();
        decls(&sheet.items, &mut ds);
        for (name, value, span) in ds {
            let border = ["border", "outline", "column-rule", "box-shadow", "text-decoration"].iter().any(|p| name.starts_with(p)) || name.starts_with("--");
            if border || !value.contains("var(--line)") {
                continue;
            }
            let text = self.text(span).to_string();
            let fill = name.starts_with("background");
            match text.find("--line").filter(|_| fill) {
                Some(at) => {
                    let start = span.start + at as u32;
                    self.push_edits("B0308", span, format!("`--line` as `{name}`: it is the border token"), "use `--surface` for a fill (or `--bg`)", vec![(start, start + 6, "--surface".to_string())]);
                }
                None => self.push("B0308", span, format!("`--line` as `{name}`: it is the border token"), "use `--fg` or `--muted` for text and `--surface` for fills; keep `--line` for borders"),
            }
        }
        let local: HashSet<String> = brain_css::custom_props(&sheet).into_iter().map(|p| p.0).collect();
        if let Some(tokens) = &self.ctx.tokens {
            let names: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();
            for (name, span) in brain_css::var_refs(&sheet) {
                if local.contains(&name) || names.contains(&name.as_str()) {
                    continue;
                }
                // `--space-0` (and its synonyms) is not a token: zero spacing is `0` (§21 normalization).
                let zero = {
                    let t = name.trim_start_matches("--");
                    t.rsplit_once('-').is_some_and(|(g, n)| n == "0" && matches!(g, "space" | "sp" | "spacing" | "gap" | "gutter" | "pad" | "s"))
                };
                if zero {
                    let decl = self.text(span).to_string();
                    let mut edits = Vec::new();
                    let mut from = 0;
                    while let Some(i) = decl[from..].find("var(") {
                        let at = from + i;
                        let inner = decl[at + 4..].trim_start();
                        if inner.starts_with(&name) && !inner[name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_') {
                            let close = decl[at..].find(')').map(|e| at + e + 1).unwrap_or(decl.len());
                            edits.push((span.start + at as u32, span.start + close as u32, "0".to_string()));
                            from = close;
                        } else {
                            from = at + 4;
                        }
                    }
                    if !edits.is_empty() {
                        self.push_info("B0302", span, format!("token `{name}` does not exist (normalized to `0`)"), "write `0` for no spacing", edits);
                        continue;
                    }
                }
                let suggestion = token_suggestion(&name, &names);
                let best = suggestion.map(|(t, _)| t);
                // `--scrim` (§ Tokens, #164) is missing in apps scaffolded before it existed: say exactly what to add.
                let scrim_like = matches!(name.trim_start_matches("--"), "scrim" | "overlay" | "backdrop" | "veil" | "shade");
                let fix = match best {
                    _ if scrim_like && !names.contains(&"--scrim") => {
                        "declare it in tokens.cssbrain: `--scrim: rgb(15 23 42 / .45);` in `:root` and `:root { --scrim: rgb(0 0 0 / .6); }` inside `@media (prefers-color-scheme: dark)`".to_string()
                    }
                    Some(s) => format!("did you mean `{s}`?"),
                    None => format!("declare it in tokens.cssbrain or use an existing one: {}", names.join(", ")),
                };
                let mut edits = Vec::new();
                if let Some(best) = best {
                    // Every `var(<name>` of this declaration.
                    let decl = self.text(span);
                    let mut from = 0;
                    while let Some(i) = decl[from..].find("var(") {
                        let after = from + i + 4;
                        let ws = decl[after..].len() - decl[after..].trim_start().len();
                        let n0 = after + ws;
                        let end = n0 + name.len();
                        if decl[n0..].starts_with(&name) && !decl[end..].starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_') {
                            edits.push((span.start + n0 as u32, span.start + end as u32, best.to_string()));
                        }
                        from = after;
                    }
                }
                if scrim_like && !names.contains(&"--scrim") {
                    // No token to point at: the fix says what to declare; no edit to an unrelated token.
                    edits.clear();
                }
                if suggestion.is_some_and(|(_, exact)| exact) && !edits.is_empty() {
                    let t = best.unwrap_or_default();
                    self.push_info("B0302", span, format!("token `{name}` does not exist (normalized to `{t}`)"), fix, edits);
                } else {
                    self.push_edits("B0302", span, format!("token `{name}` does not exist"), fix, edits);
                }
            }
            self.contrast(&sheet.items, tokens);
            self.cascade_contrast(&sheet.items, tokens);
        }
        let tokens = self.ctx.tokens.clone();
        self.literals(&sheet.items, tokens.as_deref());
        self.motion(&sheet);
    }

    fn contrast(&mut self, items: &[brain_css::CssItem], tokens: &[(String, String)]) {
        use brain_css::CssItem;
        for it in items {
            match it {
                CssItem::Rule { items: inner, .. } => {
                    let mut fg = None;
                    let mut bg = None;
                    for d in inner {
                        if let CssItem::Decl { name, value, span } = d {
                            match name.as_str() {
                                "color" => fg = Some((value.clone(), resolve_color(value, tokens), *span)),
                                "background" | "background-color" => bg = resolve_color(value, tokens),
                                _ => {}
                            }
                        }
                    }
                    if let (Some((raw, fg, span)), Some((bname, b))) = (fg, bg) {
                        let better = tokens
                            .iter()
                            .filter_map(|(n, v)| parse_hex(v).map(|c| (n, contrast_ratio(c, b))))
                            .filter(|(_, r)| *r >= 4.5)
                            .max_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(n, _)| format!("use a direct color token with enough contrast on `{bname}`, for example `{n}`"))
                            .unwrap_or_else(|| format!("darken the text color or lighten `{bname}` until it reaches 4.5:1"));
                        match fg {
                            Some((fname, f)) => {
                                let ratio = contrast_ratio(f, b);
                                if ratio < 4.5 {
                                    self.push("B0408", span, format!("insufficient contrast between `{fname}` and `{bname}` ({:.2}:1, minimum 4.5:1)", ratio), better);
                                }
                            }
                            // Unverifiable text color over a known background is not a pass (QA: the
                            // checker must not teach how to hide the problem).
                            None => self.push("B0408", span, format!("cannot verify the contrast of `{raw}` on `{bname}`"), better),
                        }
                    }
                    self.contrast(inner, tokens);
                }
                CssItem::At { block: Some(b), .. } => self.contrast(b, tokens),
                _ => {}
            }
        }
    }

    fn motion(&mut self, sheet: &brain_css::CssSheet) {
        use brain_css::CssItem;
        fn has_reduced(items: &[CssItem]) -> bool {
            items.iter().any(|it| match it {
                CssItem::At { name, prelude, block, .. } => (name == "media" && prelude.contains("prefers-reduced-motion")) || block.as_deref().is_some_and(has_reduced),
                CssItem::Rule { items, .. } => has_reduced(items),
                _ => false,
            })
        }
        fn first_anim(items: &[CssItem], in_media: bool) -> Option<(Span, String)> {
            for it in items {
                match it {
                    CssItem::Decl { name, value, span } if (name == "animation" || name == "animation-name") && value.trim() != "none" && !in_media => return Some((*span, String::new())),
                    CssItem::Rule { selector, items, .. } => {
                        if let Some((s, sel)) = first_anim(items, in_media) {
                            return Some((s, if sel.is_empty() { selector.clone() } else { sel }));
                        }
                    }
                    CssItem::At { name, block: Some(b), .. } if name != "keyframes" => {
                        if let Some(x) = first_anim(b, in_media || name == "media") {
                            return Some(x);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        if has_reduced(&sheet.items) {
            return;
        }
        if let Some((span, sel)) = first_anim(&sheet.items, false) {
            let sel = if sel.is_empty() { ".x".to_string() } else { sel };
            self.push("B0409", span, "animation without `@media (prefers-reduced-motion: reduce)`", format!("add `@media (prefers-reduced-motion: reduce) {{ {sel} {{ animation: none; }} }}`"));
        }
    }

    // ───────────────────────── accessibility ─────────────────────────

    fn a11y(&mut self) {
        let mut st = A11y::default();
        collect_label_targets(&self.c.template, &mut st.label_for);
        let nodes = &self.c.template;
        self.a11y_nodes(nodes, &mut st, false);
    }

    fn a11y_nodes(&mut self, nodes: &[Node], st: &mut A11y, in_label: bool) {
        for n in nodes {
            match n {
                Node::Element(e) => self.a11y_el(e, st, in_label),
                Node::ComponentRef(r) => self.a11y_nodes(&r.children, st, in_label),
                Node::If(b) => {
                    for br in &b.branches {
                        self.a11y_nodes(&br.body, st, in_label);
                    }
                    if let Some(o) = &b.otherwise {
                        self.a11y_nodes(o, st, in_label);
                    }
                }
                Node::Each(b) => {
                    self.a11y_nodes(&b.body, st, in_label);
                    if let Some(e) = &b.empty {
                        self.a11y_nodes(e, st, in_label);
                    }
                }
                Node::SlotScope(b) => self.a11y_nodes(&b.body, st, in_label),
                Node::Await(b) => {
                    self.a11y_nodes(&b.pending, st, in_label);
                    for br in [&b.then, &b.catch].into_iter().flatten() {
                        self.a11y_nodes(&br.body, st, in_label);
                    }
                }
                _ => {}
            }
        }
    }

    /// A raw `<input>` the kit `Input` can replace: (label from placeholder or name, the other attributes as written).
    fn kit_input(&self, e: &Element) -> Option<(String, String)> {
        const OK: &[&str] = &["type", "name", "placeholder", "autocomplete", "required", "disabled", "value"];
        if !e.children.is_empty() || e.attrs.iter().any(|a| !(matches!(a.kind, AttrKind::Plain) && OK.contains(&a.name.as_str()) || a.kind == AttrKind::Bind && a.name == "value")) {
            return None;
        }
        if matches!(e.attrs.iter().find(|a| a.name == "type").map(|a| &a.value), Some(AttrValue::Text(t)) if matches!(t.as_str(), "checkbox" | "radio" | "file" | "range" | "color")) {
            return None;
        }
        let text = |n: &str| e.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == n).and_then(|a| if let AttrValue::Text(t) = &a.value { Some(t.trim().to_string()) } else { None }).filter(|t| !t.is_empty());
        let label = text("placeholder").map(|p| p.trim_end_matches(['…', '.']).trim_end_matches("...").to_string()).or_else(|| {
            text("name").map(|n| {
                let words = n.split(['_', '-', '.']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
                let mut cs = words.chars();
                cs.next().map(|f| f.to_uppercase().collect::<String>() + cs.as_str()).unwrap_or_default()
            })
        })?;
        let rest: String = e.attrs.iter().map(|a| format!(" {}", self.text(a.span))).collect();
        Some((label, rest))
    }

    fn a11y_el(&mut self, e: &Element, st: &mut A11y, in_label: bool) {
        let tag = e.tag.to_ascii_lowercase();
        let has = |n: &str| e.attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == n);
        let text_attr = |n: &str| {
            e.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == n).and_then(|a| match &a.value {
                AttrValue::Text(t) => Some(t.clone()),
                _ => None,
            })
        };
        let labelled = has("aria-label") || has("aria-labelledby") || has("title");
        match tag.as_str() {
            "img" if !has("alt") => self.push("B0401", e.span, "`<img>` without `alt`", "add `alt=\"description\"` (or `alt=\"\"` if it is decorative)"),
            "input" | "select" | "textarea" => {
                let ty = text_attr("type").unwrap_or_default();
                let exempt = matches!(ty.as_str(), "hidden" | "submit" | "button" | "reset" | "image");
                let by_for = e.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == "id").and_then(|a| label_key(&a.value)).is_some_and(|id| st.label_for.contains(&id));
                if !exempt && !in_label && !labelled && !by_for {
                    match (tag.as_str(), self.kit_input(e)) {
                        // §21: a raw `<input>` whose attributes the kit `Input` accepts becomes `<Input label=…>`,
                        // with the label taken from the placeholder or the name (a placeholder is not a label).
                        ("input", Some((label, rest))) => {
                            let new = format!("<Input label=\"{}\"{rest} />", label.replace('"', "&quot;"));
                            self.push_info("B0403", e.span, format!("`<input>` without a label (normalized to `<Input label=\"{label}\">`)"), "check the label text; the kit `Input` renders the `<label>` and the error", vec![(e.span.start, e.span.end, new)]);
                        }
                        _ => self.push("B0403", e.span, format!("`<{tag}>` without `<label>` or `aria-label`"), format!("wrap it in `<label>Text <{tag} …/></label>` or add `aria-label=\"…\"`")),
                    }
                }
            }
            "img" => {
                if let Some(alt) = text_attr("alt")
                    && poor_alt(&alt)
                {
                    self.push("B0413", e.span, format!("`alt=\"{}\"` is a file name or a generic word, not a description", alt.trim()), "describe what the image shows (`alt=\"Team at the 2026 offsite\"`), or use `alt=\"\"` if it is decorative");
                }
            }
            "button" if !labelled && !has_accessible_text(&e.children) => {
                self.push("B0406", e.span, "`<button>` without accessible text", "add visible text or `aria-label=\"…\"`");
            }
            "a" if has("href") => {
                let aria = has("aria-label") || has("aria-labelledby");
                if !labelled && !has_accessible_text(&e.children) {
                    self.push("B0411", e.span, "`<a href>` without an accessible name", "add link text, `aria-label=\"…\"` or an `<img alt=\"…\">` inside the link");
                } else if !aria && let Some(t) = static_text(&e.children) {
                    let norm = t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
                    let norm = norm.trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace() || matches!(c, '…' | '→' | '›' | '»'));
                    if GENERIC_LINK_TEXT.contains(&norm) {
                        self.push("B0412", e.span, format!("link text `{}` does not say where the link goes", t.trim()), "name the destination (`Read the pricing guide`) or add `aria-label=\"…\"`");
                    }
                }
            }
            "iframe" if !has("title") && !has("aria-label") && !has("aria-labelledby") => {
                self.push("B0414", e.span, "`<iframe>` without `title`", "add `title=\"…\"` describing the embedded content");
            }
            "video" if (has("controls") || !has("muted")) && !has_caption_track(&e.children) => {
                self.push("B0415", e.span, "`<video>` without a captions track", "add `<track kind=\"captions\" src=\"/captions.vtt\" srclang=\"en\" label=\"English\">` (a muted autoplay loop without `controls` needs none)");
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = tag.as_bytes()[1] - b'0';
                if let Some(prev) = st.last_heading
                    && level > prev + 1
                {
                    self.push("B0404", e.span, format!("skipped heading level: `<h{level}>` after `<h{prev}>`"), format!("use `<h{}>`", prev + 1));
                }
                st.last_heading = Some(level);
                if level == 1 {
                    st.h1 += 1;
                    // §21: deterministic, so it is a normalization: every `<h1>` after the first becomes `<h2>`.
                    if st.h1 >= 2 && self.c.level == Level::View {
                        let text = self.text(e.span);
                        let mut edits = vec![(e.span.start + 1, e.span.start + 3, "h2".to_string())];
                        if text.ends_with("</h1>") {
                            edits.push((e.span.end - 3, e.span.end - 1, "h2".to_string()));
                        }
                        self.push_info("B0405", e.span, "the view has more than one `<h1>` (normalized to `<h2>`)", "keep a single `<h1>`; the others are `<h2>`", edits);
                    }
                }
            }
            "main" => {
                st.main += 1;
                let view = self.c.level == Level::View && self.c.kind == FileKind::Component;
                // §21: the layout already renders `<main>`, so every `<main>` of the view becomes `<section>`; the
                // same happens to a second `<main>` in the view itself.
                if view && (self.ctx.wrapper_main || st.main >= 2) {
                    let text = self.text(e.span);
                    let mut edits = vec![(e.span.start + 1, e.span.start + 5, "section".to_string())];
                    if text.ends_with("</main>") {
                        edits.push((e.span.end - 5, e.span.end - 1, "section".to_string()));
                    }
                    let why = if self.ctx.wrapper_main { "the layout already has `<main>`" } else { "the view has more than one `<main>`" };
                    self.push_info("B0405", e.span, format!("{why} (normalized to `<section>`)"), "keep a single `<main>` per page; the rest are `<section>`", edits);
                }
            }
            _ => {}
        }
        if e.attrs.iter().any(|a| a.kind == AttrKind::On && a.name == "click") && !is_interactive(&tag, e) {
            self.push("B0402", e.span, format!("`on:click` on `<{tag}>`, which is not interactive"), format!("use `<button type=\"button\">` instead of `<{tag}>`"));
        }
        for a in &e.attrs {
            if a.kind == AttrKind::Plain && a.name == "tabindex" {
                let v = match &a.value {
                    AttrValue::Text(t) => t.trim().parse::<i64>().ok(),
                    AttrValue::Expr(Expr::Num { raw, .. }) => raw.parse::<i64>().ok(),
                    _ => None,
                };
                if v.is_some_and(|v| v > 0) {
                    self.push("B0407", a.span, "`tabindex` greater than 0 breaks the focus order", "use `tabindex=\"0\"` or remove it");
                }
            }
        }
        self.a11y_nodes(&e.children, st, in_label || tag == "label");
    }

    // ───────────────────────── SEO ─────────────────────────

    fn seo(&mut self) {
        // `_layout.brain` (and any `_x.brain`) wraps a folder of views; it is not a route.
        let partial = self.c.file.rsplit('/').next().is_some_and(|f| f.starts_with('_'));
        if self.c.kind == FileKind::Component && self.c.level == Level::View && !partial && !self.c.meta.iter().any(|m| m.name == "seo") {
            let s = Span { start: 0, end: 0, line: 1, col: 1 };
            self.push("B0501", s, "the view does not declare `<meta seo …>`", "add `<meta seo title=\"…\" description=\"…\" canonical=\"/route\">` at the top of the file");
        }
    }
}

/// Every expression of a markup subtree, dumped as text (for cheap usage checks).
/// Lowercased element names this component writes, including inside component children (B0307). `None` when it
/// renders `{@html}`, whose tags are unknown.
fn element_tags(nodes: &[Node]) -> Option<HashSet<String>> {
    fn go(nodes: &[Node], out: &mut HashSet<String>) -> bool {
        nodes.iter().all(|n| match n {
            Node::Element(e) => {
                out.insert(e.tag.to_ascii_lowercase());
                go(&e.children, out)
            }
            Node::ComponentRef(r) => go(&r.children, out),
            Node::Interp(i) => !i.html,
            Node::If(b) => b.branches.iter().all(|br| go(&br.body, out)) && b.otherwise.as_ref().is_none_or(|o| go(o, out)),
            Node::Each(b) => go(&b.body, out) && b.empty.as_ref().is_none_or(|e| go(e, out)),
            Node::SlotScope(b) => go(&b.body, out),
            Node::Await(b) => go(&b.pending, out) && [&b.then, &b.catch].into_iter().flatten().all(|br| go(&br.body, out)),
            _ => true,
        })
    }
    let mut out = HashSet::new();
    go(nodes, &mut out).then_some(out)
}

fn node_exprs(nodes: &[Node], out: &mut String) {
    use brain_parse::dump::expr;
    for n in nodes {
        match n {
            Node::Interp(i) => out.push_str(&expr(&i.expr)),
            Node::Element(Element { attrs, children, .. }) | Node::ComponentRef(ComponentRef { attrs, children, .. }) => {
                for a in attrs {
                    match &a.value {
                        AttrValue::Expr(e) => out.push_str(&expr(e)),
                        AttrValue::Mixed(parts) => parts.iter().for_each(|p| {
                            if let AttrPart::Expr(e) = p {
                                out.push_str(&expr(e));
                            }
                        }),
                        _ => {}
                    }
                    out.push(' ');
                }
                node_exprs(children, out);
            }
            Node::If(b) => {
                for br in &b.branches {
                    out.push_str(&expr(&br.test));
                    node_exprs(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    node_exprs(o, out);
                }
            }
            Node::Each(b) => {
                out.push_str(&expr(&b.list));
                node_exprs(&b.body, out);
            }
            Node::SlotScope(b) => node_exprs(&b.body, out),
            Node::Slot(s) => {
                for a in &s.attrs {
                    match &a.value {
                        AttrValue::Expr(e) => out.push_str(&expr(e)),
                        AttrValue::Mixed(parts) => parts.iter().for_each(|p| {
                            if let AttrPart::Expr(e) = p {
                                out.push_str(&expr(e));
                            }
                        }),
                        _ => {}
                    }
                    out.push(' ');
                }
            }
            Node::Await(b) => {
                out.push_str(&expr(&b.promise));
                node_exprs(&b.pending, out);
                for br in [&b.then, &b.catch].into_iter().flatten() {
                    node_exprs(&br.body, out);
                }
            }
            _ => {}
        }
        out.push(' ');
    }
}

#[derive(Default)]
struct A11y {
    last_heading: Option<u8>,
    h1: u32,
    main: u32,
    label_for: HashSet<String>,
}

/// How a `for` and an `id` are matched: the text when static, the expression's source shape when dynamic
/// (`for={'name-' + r.id}` pairs with `id={'name-' + r.id}`, QA B0403).
fn label_key(v: &AttrValue) -> Option<String> {
    match v {
        AttrValue::Text(t) => Some(t.clone()),
        AttrValue::Expr(x) => Some(format!("{{{}}}", brain_parse::dump::expr(x))),
        AttrValue::Mixed(parts) => Some(
            parts
                .iter()
                .map(|p| match p {
                    AttrPart::Text(t) => t.clone(),
                    AttrPart::Expr(x) => format!("{{{}}}", brain_parse::dump::expr(x)),
                })
                .collect(),
        ),
        AttrValue::None => None,
    }
}

fn collect_label_targets(nodes: &[Node], out: &mut HashSet<String>) {
    for n in nodes {
        match n {
            Node::Element(e) => {
                if e.tag == "label" {
                    for a in &e.attrs {
                        if a.kind == AttrKind::Plain
                            && a.name == "for"
                            && let Some(k) = label_key(&a.value)
                        {
                            out.insert(k);
                        }
                    }
                }
                collect_label_targets(&e.children, out);
            }
            Node::ComponentRef(r) => collect_label_targets(&r.children, out),
            Node::If(b) => {
                for br in &b.branches {
                    collect_label_targets(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    collect_label_targets(o, out);
                }
            }
            Node::Each(b) => {
                collect_label_targets(&b.body, out);
                if let Some(e) = &b.empty {
                    collect_label_targets(e, out);
                }
            }
            // QA: labels inside `{:then}`/`{:catch}` count too (B0403 false positive in `{#await}`).
            Node::Await(a) => {
                collect_label_targets(&a.pending, out);
                for br in [&a.then, &a.catch].into_iter().flatten() {
                    collect_label_targets(&br.body, out);
                }
            }
            Node::SlotScope(b) => collect_label_targets(&b.body, out),
            _ => {}
        }
    }
}

fn has_accessible_text(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Text(t) => !t.value.trim().is_empty(),
        Node::Interp(_) | Node::ComponentRef(_) | Node::Slot(_) => true,
        Node::SlotScope(b) => has_accessible_text(&b.body),
        Node::Element(e) => {
            (e.tag == "img" && e.attrs.iter().any(|a| a.name == "alt" && !matches!(&a.value, AttrValue::Text(t) if t.is_empty())))
                || e.attrs.iter().any(|a| a.name == "aria-label")
                || has_accessible_text(&e.children)
        }
        Node::If(b) => b.branches.iter().any(|br| has_accessible_text(&br.body)),
        Node::Each(b) => has_accessible_text(&b.body),
        Node::Await(_) => true,
    })
}

/// Link texts that do not say where the link goes (B0412), compared lowercased and without trailing punctuation.
pub const GENERIC_LINK_TEXT: &[&str] = &["click here", "here", "read more", "more", "link", "learn more"];

/// Extensions that make an `alt` a file name (B0413).
const IMAGE_EXTENSIONS: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".avif"];
/// Words that alone describe nothing (B0413).
const GENERIC_ALT: &[&str] = &["image", "photo", "picture", "logo", "icon", "img"];

/// True when a non-empty `alt` is a file name (`hero.png`) or only a generic word (`image`), B0413. `alt=""` is fine.
pub fn poor_alt(alt: &str) -> bool {
    let a = alt.trim().to_lowercase();
    if a.is_empty() {
        return false;
    }
    let word = a.trim_end_matches(|c: char| c.is_ascii_punctuation());
    GENERIC_ALT.contains(&word)
        || a.split_whitespace().any(|w| {
            let w = w.trim_end_matches([',', ';', ':', ')', '"', '\'']);
            IMAGE_EXTENSIONS.iter().any(|x| w.ends_with(x) && w.len() > x.len())
        })
}

/// The static text of a subtree, or `None` when any part is dynamic (an expression, a component, a slot or a block).
fn static_text(nodes: &[Node]) -> Option<String> {
    let mut out = String::new();
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(&t.value),
            Node::Element(e) => {
                if e.attrs.iter().any(|a| a.name == "aria-label") {
                    return None;
                }
                if e.tag == "img" {
                    match e.attrs.iter().find(|a| a.name == "alt").map(|a| &a.value) {
                        Some(AttrValue::Text(t)) => out.push_str(t),
                        Some(AttrValue::None) | None => {}
                        Some(_) => return None,
                    }
                }
                out.push(' ');
                out.push_str(&static_text(&e.children)?);
                out.push(' ');
            }
            _ => return None,
        }
    }
    Some(out)
}

/// A `<track kind="captions|subtitles">` among the children of a `<video>` (B0415).
fn has_caption_track(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Element(e) => {
            (e.tag == "track" && e.attrs.iter().any(|a| a.name == "kind" && matches!(&a.value, AttrValue::Text(t) if matches!(t.trim(), "captions" | "subtitles"))))
                || has_caption_track(&e.children)
        }
        Node::If(b) => b.branches.iter().any(|br| has_caption_track(&br.body)) || b.otherwise.as_ref().is_some_and(|o| has_caption_track(o)),
        Node::Each(b) => has_caption_track(&b.body),
        _ => false,
    })
}

fn is_interactive(tag: &str, e: &Element) -> bool {
    matches!(tag, "button" | "input" | "select" | "textarea" | "summary" | "details" | "label" | "option")
        || (tag == "a" && e.attrs.iter().any(|a| a.name == "href"))
        || e.attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == "role")
}

fn kebab(name: &str) -> String {
    let mut s = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                s.push('-');
            }
            s.push(c.to_ascii_lowercase());
        } else {
            s.push(c);
        }
    }
    s
}

/// Resolves a CSS color value to a label and sRGB components: `var(--token)`, a hex literal, or
/// `color-mix(in srgb, A [p%], B [q%])` over those (linear per-channel interpolation).
pub fn resolve_color(v: &str, tokens: &[(String, String)]) -> Option<(String, (f64, f64, f64))> {
    let v = v.trim();
    if let Some(name) = v.strip_prefix("var(").and_then(|x| x.strip_suffix(')')) {
        let name = name.trim().split(',').next()?.trim().to_string();
        let val = tokens.iter().find(|t| t.0 == name)?.1.clone();
        return Some((name, parse_hex(&val)?));
    }
    if let Some(c) = parse_hex(v) {
        return Some((v.to_string(), c));
    }
    let inner = v.strip_prefix("color-mix(")?.strip_suffix(')')?;
    let mut parts = Vec::new();
    let (mut depth, mut last) = (0i32, 0usize);
    for (i, ch) in inner.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(inner[last..i].trim());
                last = i + 1;
            }
            _ => {}
        }
    }
    parts.push(inner[last..].trim());
    if parts.len() != 3 || parts[0].replace(' ', "") != "insrgb" {
        return None;
    }
    let split = |p: &str| -> Option<((f64, f64, f64), Option<f64>)> {
        let (color, pct) = match p.rsplit_once(' ') {
            Some((c, w)) if w.ends_with('%') => (c, w.trim_end_matches('%').parse::<f64>().ok()),
            _ => (p, None),
        };
        Some((resolve_color(color, tokens)?.1, pct))
    };
    let (a, pa) = split(parts[1])?;
    let (b, pb) = split(parts[2])?;
    let (wa, wb) = match (pa, pb) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, 100.0 - x),
        (None, Some(y)) => (100.0 - y, y),
        (None, None) => (50.0, 50.0),
    };
    let t = wa + wb;
    if t <= 0.0 {
        return None;
    }
    let mix = |x: f64, y: f64| ((x * wa + y * wb) / t * 255.0).round() / 255.0;
    let c = (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2));
    let hex = format!("#{:02x}{:02x}{:02x}", (c.0 * 255.0) as u8, (c.1 * 255.0) as u8, (c.2 * 255.0) as u8);
    Some((format!("{v} ({hex})"), c))
}

/// `#rgb` / `#rrggbb` → linear-ready sRGB components in 0..1.
pub fn parse_hex(v: &str) -> Option<(f64, f64, f64)> {
    let h = v.trim().strip_prefix('#')?;
    let (r, g, b) = match h.len() {
        3 | 4 => {
            let p = |i: usize| u8::from_str_radix(&h[i..i + 1].repeat(2), 16).ok();
            (p(0)?, p(1)?, p(2)?)
        }
        6 | 8 => {
            let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
            (p(0)?, p(2)?, p(4)?)
        }
        _ => return None,
    };
    Some((r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0))
}

/// WCAG 2.x contrast ratio.
pub fn contrast_ratio(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    fn lin(c: f64) -> f64 {
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }
    fn lum((r, g, b): (f64, f64, f64)) -> f64 {
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }
    let (la, lb) = (lum(a), lum(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn dump_root(e: &Expr) -> String {
    match e.unparen() {
        Expr::Ident(i) => i.name.clone(),
        Expr::Member { obj, .. } => dump_root(obj),
        _ => String::new(),
    }
}

/// Byte offsets and kinds of substrings shaped like credentials (SPEC §11, B0703). Used by the checker on
/// client literals and by brain-pack on the final output.
pub fn scan_secrets(text: &str) -> Vec<(usize, &'static str)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'-';
    let b64u = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'-';
    let run = |from: usize, ok: &dyn Fn(u8) -> bool| b[from.min(b.len())..].iter().take_while(|c| ok(**c)).count();
    let mut i = 0;
    while i < b.len() {
        // Only at a token boundary: `task-1234` must not look like `sk-`.
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let boundary = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        let rest = &text[i..];
        let hit: Option<(&'static str, usize)> = if !boundary {
            None
        } else if rest.starts_with("-----BEGIN") {
            Some(("PEM private key", 10))
        } else if rest.starts_with("sk-") && run(i + 3, &word) >= 20 {
            Some(("sk-…", 3 + run(i + 3, &word)))
        } else if let Some(p) = ["sk_live_", "sk_test_", "rk_live_", "rk_test_", "whsec_"].iter().find(|p| rest.starts_with(**p))
            && run(i + p.len(), &|c: u8| c.is_ascii_alphanumeric()) >= 20
        {
            Some(("Stripe key", p.len() + run(i + p.len(), &|c: u8| c.is_ascii_alphanumeric())))
        } else if rest.starts_with("AKIA") && run(i + 4, &|c: u8| c.is_ascii_uppercase() || c.is_ascii_digit()) >= 16 {
            Some(("AWS AKIA…", 20))
        } else if rest.starts_with("AIza") && run(i + 4, &word) >= 35 {
            Some(("Google API key", 39))
        } else if (rest.starts_with("ghp_") || rest.starts_with("gho_") || rest.starts_with("ghs_")) && run(i + 4, &|c: u8| c.is_ascii_alphanumeric()) >= 36 {
            Some(("GitHub token", 40))
        } else if rest.starts_with("github_pat_") && run(i + 11, &word) >= 40 {
            Some(("GitHub token", 11 + run(i + 11, &word)))
        } else if (rest.starts_with("xoxb-") || rest.starts_with("xoxp-") || rest.starts_with("xoxa-")) && run(i + 5, &word) >= 20 {
            Some(("Slack token", 5 + run(i + 5, &word)))
        } else if rest.starts_with("eyJ") {
            let a = run(i, &b64u);
            let ok = a >= 13 && b.get(i + a) == Some(&b'.') && text[i + a + 1..].starts_with("eyJ") && {
                let p = run(i + a + 1, &b64u);
                p >= 13 && b.get(i + a + 1 + p) == Some(&b'.')
            };
            if ok { Some(("JWT", a)) } else { None }
        } else {
            None
        };
        match hit {
            Some((kind, len)) => {
                out.push((i, kind));
                i += len.max(1);
            }
            None => i += 1,
        }
    }
    out
}

/// B0302: closest token by spelling, then by abbreviation with the same step number (`--sp-2` → `--space-2`),
/// then by meaning (`--border` → `--line`).
pub fn closest_token<'a>(name: &str, tokens: &[&'a str]) -> Option<&'a str> {
    token_suggestion(name, tokens).map(|(t, _)| t)
}

/// [`closest_token`] plus whether it is an exact synonym (same name without hyphens/case, an abbreviation with
/// the same step, or a listed synonym): those are normalized as `info` (INTERNALS §21); a mere typo stays an error.
pub fn token_suggestion<'a>(name: &str, tokens: &[&'a str]) -> Option<(&'a str, bool)> {
    let squash = |t: &str| t.trim_start_matches("--").replace(['-', '_'], "").to_ascii_lowercase();
    if let Some(t) = tokens.iter().find(|t| squash(t) == squash(name)) {
        return Some((t, true));
    }
    if let Some(t) = by_meaning(name, tokens) {
        return Some((t, true));
    }
    scope::closest(name, tokens.iter().copied()).map(|t| (t, false))
}

fn by_meaning<'a>(name: &str, tokens: &[&'a str]) -> Option<&'a str> {
    let split = |t: &str| -> (String, String) {
        let t = t.trim_start_matches("--");
        match t.rfind('-') {
            Some(i) if t[i + 1..].chars().all(|c| c.is_ascii_digit()) && i + 1 < t.len() => (t[..i].to_string(), t[i + 1..].to_string()),
            _ => (t.to_string(), String::new()),
        }
    };
    const GROUPS: &[&[&str]] = &[
        &["space", "sp", "spacing", "gap", "gutter", "pad", "s"],
        &["line", "border", "stroke", "divider", "outline", "rule", "hairline"],
        &["surface", "card", "panel", "bg-2", "bg-alt", "elevated", "raised", "paper"],
        &["fg", "text", "color", "ink", "foreground", "fg-1"],
        &["muted", "secondary", "subtle", "fg-2", "text-muted", "dim", "faint"],
        &["bg", "background", "page", "canvas", "base"],
        &["accent", "primary", "brand", "link", "highlight"],
        &["on-accent", "on-primary", "accent-fg", "primary-fg", "on-brand", "contrast"],
        &["ok", "success", "good", "positive", "green"],
        &["bad", "danger", "error", "negative", "red", "warn", "warning"],
        &["radius", "rounded", "round", "r", "corner", "br"],
        &["scrim", "overlay", "backdrop", "veil", "shade", "dim-bg"],
        &["font", "font-family", "family", "sans", "body"],
    ];
    let (stem, num) = split(name);
    let group = |s: &str| GROUPS.iter().position(|g| g.contains(&s));
    let mut best: Option<(u8, &'a str)> = None;
    for &t in tokens {
        let (ts, tn) = split(t);
        if tn != num {
            continue;
        }
        let score = if !stem.is_empty() && !ts.is_empty() && (stem.starts_with(&ts) || ts.starts_with(&stem)) {
            1
        } else if group(&stem).is_some() && group(&stem) == group(&ts) {
            2
        } else {
            continue;
        };
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, t));
        }
    }
    best.map(|(_, t)| t)
}

/// `import { sig } from 'brain'`, `import { Button } from 'brain/ui'`…: every binding is a primitive, a runtime
/// export or a kit component and the source is not a file (B0121, normalized away).
fn is_framework_import(i: &Import) -> bool {
    let src = i.source.as_str();
    let framework_src = !src.starts_with('.') && !src.starts_with('/') && (src.starts_with("brain") || src.starts_with("@brain"));
    let names: Vec<&str> = i.named.iter().map(|(imp, _)| imp.name.as_str()).chain(i.default.iter().map(|d| d.name.as_str())).collect();
    framework_src
        && i.namespace.is_none()
        && i.named.iter().all(|(a, b)| a.name == b.name)
        && names.iter().all(|n| scope::PRIMITIVES.contains(n) || scope::UI_KIT.contains(n) || matches!(*n, "batch" | "untrack" | "mount" | "hydrate"))
}

/// `"key": ["a", "b"]` anywhere in a JSON text (no escapes inside the strings). Missing key → empty.
fn json_string_list(json: &str, key: &str) -> Vec<String> {
    let Some(i) = json.find(&format!("\"{key}\"")) else { return Vec::new() };
    let rest = &json[i + key.len() + 2..];
    let Some(open) = rest.find('[') else { return Vec::new() };
    if rest[..open].trim().trim_start_matches(':').trim() != "" {
        return Vec::new();
    }
    let Some(close) = rest[open..].find(']') else { return Vec::new() };
    rest[open + 1..open + close].split(',').map(|x| x.trim().trim_matches('"').to_string()).filter(|x| !x.is_empty()).collect()
}

/// `"cache": { … "data": false … }` in brain.json text.
fn cache_data_off(json: &str) -> bool {
    let Some(i) = json.find("\"cache\"") else { return false };
    let rest = &json[i..];
    let Some(open) = rest.find('{') else { return false };
    let Some(close) = rest[open..].find('}') else { return false };
    let obj = &rest[open..open + close];
    obj.find("\"data\"").is_some_and(|d| obj[d + 6..].trim_start().trim_start_matches(':').trim_start().starts_with("false"))
}

fn provided_names_in(src: &str) -> HashSet<String> {
    src.match_indices("provide(")
        .filter_map(|(i, _)| {
            let n: String = src[i + 8..].trim_start().chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect();
            (!n.is_empty()).then_some(n)
        })
        .collect()
}

/// `export [async] function <name>(…)` at the top of a script.
fn exports_fn(s: &Stmt, name: &str) -> bool {
    matches!(s, Stmt::Export { decl, .. } if matches!(decl.as_ref(), Stmt::Func(f) if f.name.as_ref().is_some_and(|n| n.name == name)))
}
