//! `.brain` → JS module over `runtime/brain.js` (SPEC §6) + scoped CSS (SPEC §7-§8).
//!
//! Public contract (SPEC §4): [`compile`] and [`Output`]. [`compile_with`] adds dev mode.

pub mod blocks;
mod img;
mod inline;
mod srcmap;
mod js;
mod template;

pub use js::js_str;

use brain_check::scope::{self, Kind, Scopes};
use brain_parse::ast::*;
use brain_parse::Diag;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub js: String,
    pub css: String,
    /// Files read to compile this one (tokens, imported components) — for watch/HMR.
    pub deps: Vec<PathBuf>,
    /// Non-blocking diagnostics (`Diag::is_warning`).
    pub warnings: Vec<Diag>,
    /// Names this component owns, for CSS renaming in brain-min (SPEC §11 `--protect`).
    pub names: Names,
    /// Server-only ESM: `load` + `server:` functions (with `ctx` first) + the imports they use (SPEC §11.1).
    /// Empty when the file has no server code. A `<meta server>` module lives here entirely and `js` is empty.
    pub server_js: String,
    /// Names of the `server:` functions, reachable as `POST /_r/<hash>/<name>`.
    pub server_fns: Vec<String>,
    /// Per-block hashes of the compiled source (INTERNALS §28): compare with [`blocks::change`].
    pub blocks: blocks::Blocks,
    /// Source Map v3 (JSON) from `js` lines to `.brain` lines (§18). In dev it is also inlined in `js`.
    pub map: String,
    /// i18n keys this file still resolves at runtime (plurals, dynamic args); `"*"` when a key is not literal.
    /// TOOLCHAIN ships only these per view and locale (§37).
    pub i18n_keys: Vec<String>,
}

/// Class names and custom properties a component owns, and what must not be renamed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    /// Classes in the component's `<style>` selectors, static `class="…"` and `class:x`. Sorted.
    pub classes: Vec<String>,
    /// Custom properties the component declares in its `<style>` (tokens are global). Sorted.
    pub vars: Vec<String>,
    /// Classes built at runtime (`class={expr}`, `classList` in script): the string literals found
    /// in those expressions (e.g. `"a-"` for `class={'a-' + x}`), or `"*"` when nothing is known.
    /// brain-min leaves matching classes untouched.
    pub dynamic: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Dev build: `sig(v, 'name')` + `export default dev(Comp)` for HMR (SPEC §6.2).
    pub dev: bool,
    /// #143: hydration by adoption. Node references use `fc`/`nx` (RUNTIME brain-hydrate.js), which step over a
    /// server-rendered dynamic region; on a fresh clone they equal `.firstChild`/`.nextSibling`.
    pub adopt: bool,
    /// `brain.json` `"lint"` (INTERNALS §20): per-code level for the B09–B13 families; `None` turns the code off.
    pub lint: Vec<(String, Option<brain_parse::Severity>)>,
    /// Import specifier of the runtime core.
    pub runtime: String,
    pub runtime_dev: String,
    pub runtime_wc: String,
    /// Appended to every `.brain`/`.jsbrain` import specifier (e.g. `".js"` → `./a.brain.js`).
    /// Empty = specifiers are emitted as written and the server resolves them.
    pub import_suffix: String,
    /// Directory that `path` is relative to, to read tokens and imported files from disk.
    /// `None` = the current directory.
    pub root: Option<PathBuf>,
    /// Base36 chars of the component hash in `data-b`, `@scope`, `Comp.h` and `rpc("<hash>/f")`: 6 in dev,
    /// 4 in `brain build` (5 for a colliding pair).
    pub hash_len: usize,
    /// `PUBLIC_*` values of the selected environment (`.brain_env`, §36): `import.meta.env.PUBLIC_X` and
    /// `process.env.PUBLIC_X` compile to these string constants. Empty = left as written.
    pub public_env: Vec<(String, String)>,
    /// Locale being built and its plain (non-plural) texts (`i18n/<locale>.json`, §37): `t('key')` and
    /// `t('key', { n })` with literal keys compile to the text itself. `None` = every `t()` stays a runtime call.
    pub i18n: Option<(String, std::collections::BTreeMap<String, String>)>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            dev: false,
            adopt: false,
            lint: Vec::new(),
            runtime: "/@brain/brain.js".into(),
            runtime_dev: "/@brain/brain-dev.js".into(),
            runtime_wc: "/@brain/brain-wc.js".into(),
            import_suffix: String::new(),
            root: None,
            hash_len: 6,
            public_env: Vec::new(),
            i18n: None,
        }
    }
}

/// SPEC §4 entry point: production output.
pub fn compile(src: &str, path: &Path) -> Result<Output, Vec<Diag>> {
    compile_with(src, path, &Options::default())
}

pub fn compile_with(src: &str, path: &Path, opts: &Options) -> Result<Output, Vec<Diag>> {
    compile_normalized(src, path, opts, 0)
}

/// INTERNALS §21: when the only findings are `info` normalizations (plus warnings), their edits are applied to the
/// source and it is compiled again; the infos are reported with the positions of the file as written.
fn compile_normalized(src: &str, path: &Path, opts: &Options, depth: u32) -> Result<Output, Vec<Diag>> {
    let c = brain_parse::parse(src, path)?;
    let disk = match &opts.root {
        Some(r) => r.join(path),
        None => path.to_path_buf(),
    };
    let (ctx, deps) = brain_check::load_ctx(&disk, &c);
    let diags = apply_lint(brain_check::check(&c, &ctx), &opts.lint);
    if depth < 4 && !diags.iter().any(|d| !d.is_warning()) && diags.iter().any(|d| d.is_info() && !d.edits.is_empty()) {
        let mut edits: Vec<brain_parse::Edit> = diags.iter().filter(|d| d.is_info()).flat_map(|d| d.edits.clone()).collect();
        edits.sort_by_key(|e| (e.start, e.end));
        let mut kept: Vec<brain_parse::Edit> = Vec::new();
        for e in edits {
            if kept.last().is_none_or(|k| k.end <= e.start && !(k.start == k.end && k.start == e.start)) {
                kept.push(e);
            }
        }
        if let Some(fixed) = Diag::apply(src, &kept)
            && fixed != src
        {
            let infos: Vec<Diag> = diags.into_iter().filter(|d| d.is_info()).collect();
            return match compile_normalized(&fixed, path, opts, depth + 1) {
                Ok(mut o) => {
                    let mut w = infos;
                    w.extend(o.warnings);
                    o.warnings = w;
                    Ok(o)
                }
                Err(mut e) => {
                    e.splice(0..0, infos);
                    Err(e)
                }
            };
        }
    }
    compile_once(&c, opts, &ctx, deps, diags, &disk)
}

/// `brain.json` `"lint"`: a level per code, only for the families a project may tune (B09–B13: effects, quality,
/// data, compatibility, practices); everything else keeps its catalog level.
fn apply_lint(diags: Vec<Diag>, lint: &[(String, Option<brain_parse::Severity>)]) -> Vec<Diag> {
    if lint.is_empty() {
        return diags;
    }
    diags
        .into_iter()
        .filter_map(|mut d| {
            let tunable = matches!(&d.code[..3], "B09" | "B10" | "B11" | "B12" | "B13");
            match lint.iter().find(|(c, _)| c == d.code).filter(|_| tunable) {
                Some((_, None)) => None,
                Some((_, Some(level))) => {
                    d.level = *level;
                    Some(d)
                }
                None => Some(d),
            }
        })
        .collect()
}

fn compile_once(c: &Component, opts: &Options, ctx: &brain_check::Ctx, deps: Vec<PathBuf>, diags: Vec<Diag>, disk: &Path) -> Result<Output, Vec<Diag>> {
    if diags.iter().any(|d| !d.is_warning()) {
        return Err(diags);
    }
    let server_srcs: HashSet<String> =
        ctx.imports.iter().filter(|(_, v)| matches!(v, brain_check::Imported::Server)).map(|(k, _)| k.clone()).collect();
    let inline = if opts.dev || c.kind != FileKind::Component { HashMap::new() } else { inline::candidates(c, disk, opts.hash_len) };
    let title_template = if c.level == Level::View && c.kind == FileKind::Component { title_template(disk) } else { None };
    // §22: built-in `<Img>` becomes `<img>` with decoding/loading/fetchpriority set by the compiler.
    let lowered = img::lowered(c);
    let gc = lowered.as_ref().unwrap_or(c);
    let (js, css, server_js, server_fns, i18n_keys) = generate(gc, opts, &server_srcs, inline, title_template);
    let blocks = blocks::blocks(&c.src, Path::new(&c.file));
    let (js, map) = srcmap::finish(&js, &c.file, &c.src, opts.dev && !js.is_empty());
    let (server_js, _) = srcmap::finish(&server_js, &c.file, &c.src, false);
    Ok(Output { js, css, deps, warnings: diags, names: names(c), server_js, server_fns, blocks, map, i18n_keys })
}

/// Collects [`Names`] from the style, the markup and the script.
pub fn names(c: &Component) -> Names {
    use std::collections::BTreeSet;
    let mut classes = BTreeSet::new();
    let mut vars = BTreeSet::new();
    let mut dynamic = BTreeSet::new();
    fn css_walk(items: &[brain_css::CssItem], classes: &mut BTreeSet<String>, vars: &mut BTreeSet<String>) {
        for it in items {
            match it {
                brain_css::CssItem::Rule { selector, items, .. } => {
                    let b = selector.as_bytes();
                    let mut i = 0;
                    while i < b.len() {
                        if b[i] == b'.' && (i == 0 || !b[i - 1].is_ascii_digit()) {
                            let n: String = selector[i + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
                            if !n.is_empty() && !n.starts_with(|c: char| c.is_ascii_digit()) {
                                i += n.len();
                                classes.insert(n);
                            }
                        }
                        i += 1;
                    }
                    css_walk(items, classes, vars);
                }
                brain_css::CssItem::Decl { name, .. } if name.starts_with("--") => {
                    vars.insert(name.clone());
                }
                brain_css::CssItem::At { block: Some(b), .. } => css_walk(b, classes, vars),
                _ => {}
            }
        }
    }
    if let Some(s) = &c.style {
        css_walk(&s.items, &mut classes, &mut vars);
    }
    fn literals(e: &Expr, out: &mut BTreeSet<String>) -> bool {
        // Collects string pieces; returns false when the expression has no literal at all.
        let before = out.len();
        fn walk(e: &Expr, out: &mut BTreeSet<String>) {
            match e {
                Expr::Str { value, .. } => {
                    for w in value.split_whitespace() {
                        out.insert(w.to_string());
                    }
                }
                Expr::Template { quasis, exprs, .. } => {
                    for q in quasis {
                        for w in q.split_whitespace() {
                            out.insert(w.to_string());
                        }
                    }
                    exprs.iter().for_each(|x| walk(x, out));
                }
                Expr::Binary { left, right, .. } => {
                    walk(left, out);
                    walk(right, out);
                }
                Expr::Cond { cons, alt, .. } => {
                    walk(cons, out);
                    walk(alt, out);
                }
                Expr::Paren { expr, .. } => walk(expr, out),
                Expr::Array { elems, .. } => elems.iter().flatten().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        walk(e, out);
        out.len() > before
    }
    fn markup(nodes: &[Node], classes: &mut BTreeSet<String>, dynamic: &mut BTreeSet<String>) {
        for n in nodes {
            match n {
                Node::Element(e) => {
                    for a in &e.attrs {
                        match (a.kind, &a.value) {
                            (AttrKind::Class, _) => {
                                classes.insert(a.name.clone());
                            }
                            (AttrKind::Plain, AttrValue::Text(t)) if a.name == "class" => {
                                classes.extend(t.split_whitespace().map(str::to_string));
                            }
                            (AttrKind::Plain, AttrValue::Expr(x)) if a.name == "class" => {
                                if !literals(x, dynamic) {
                                    dynamic.insert("*".into());
                                }
                            }
                            (AttrKind::Plain, AttrValue::Mixed(parts)) if a.name == "class" => {
                                for p in parts {
                                    match p {
                                        AttrPart::Text(t) => {
                                            dynamic.extend(t.split_whitespace().map(str::to_string));
                                        }
                                        AttrPart::Expr(x) => {
                                            if !literals(x, dynamic) {
                                                dynamic.insert("*".into());
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    markup(&e.children, classes, dynamic);
                }
                Node::ComponentRef(r) => markup(&r.children, classes, dynamic),
                Node::If(b) => {
                    for br in &b.branches {
                        markup(&br.body, classes, dynamic);
                    }
                    if let Some(o) = &b.otherwise {
                        markup(o, classes, dynamic);
                    }
                }
                Node::Each(b) => {
                    markup(&b.body, classes, dynamic);
                    if let Some(e) = &b.empty {
                        markup(e, classes, dynamic);
                    }
                }
                Node::SlotScope(b) => markup(&b.body, classes, dynamic),
                Node::Await(b) => {
                    markup(&b.pending, classes, dynamic);
                    for br in [&b.then, &b.catch].into_iter().flatten() {
                        markup(&br.body, classes, dynamic);
                    }
                }
                _ => {}
            }
        }
    }
    markup(&c.template, &mut classes, &mut dynamic);
    // `classList`/`className` in the script: nothing can be proven about class names.
    let mut script = String::new();
    for s in &c.script {
        brain_parse::dump::stmt(s, 0, &mut script);
    }
    if script.contains(".classList") || script.contains(".className") {
        dynamic.insert("*".into());
    }
    Names { classes: classes.into_iter().collect(), vars: vars.into_iter().collect(), dynamic: dynamic.into_iter().collect() }
}

/// Stable 6-char scope hash of a component path (FNV-1a 64 → base36).
pub fn component_hash(file: &str) -> String {
    component_hash_len(file, 6)
}

/// Same hash with `len` base36 chars (1..=12); a shorter one is a prefix of a longer one.
/// `brain build` uses 4 and recompiles a colliding pair with 5.
pub fn component_hash_len(file: &str, len: usize) -> String {
    let len = len.clamp(1, 12);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in file.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let mut s = String::new();
    while s.len() < len {
        let d = (h % 36) as u8;
        s.push(if d < 10 { (b'0' + d) as char } else { (b'a' + d - 10) as char });
        h /= 36;
    }
    s
}

/// Every name the codegen may import from the runtime, in import order.
pub const RUNTIME_NAMES: &[&str] = &["sig", "drv", "fx", "batch", "untrack", "tpl", "insert", "attr", "cls", "on", "list", "when", "rpc", "_rt"];

pub(crate) struct Gen<'a> {
    pub(crate) opts: &'a Options,
    /// Top-level `resource()`/`stream()` locals: their node methods (`refetch`, `more`…) are read without `.v`.
    pub(crate) node_vars: HashSet<String>,
    /// `let f = form(schema, { submit: save })` with `save` a `server:` function: `f` → `save` (§38, real POST forms).
    pub(crate) forms: HashMap<String, String>,
    /// Component name → the specifier `lazy` imports it from (§22 `<Chart lazy />`).
    pub(crate) lazy_src: HashMap<String, String>,
    pub(crate) scopes: Scopes,
    pub(crate) indent: usize,
    pub(crate) hash: String,
    /// Prop name → JS of its default (only props that have one).
    pub(crate) prop_defaults: HashMap<String, String>,
    /// Static `id`s rewritten per instance (SPEC §8).
    pub(crate) ids: HashSet<String>,
    /// Some `id` is an expression (`id={'faq-' + it.id}`): ids and ID references computed at run time get the
    /// per-instance suffix too (#308).
    pub(crate) dyn_ids: bool,
    /// An ID reference or id computed at run time goes through the module helper `$sx`.
    pub(crate) uses_sx: bool,
    used: BTreeSet<&'static str>,
    /// Runtime names that collide with user names get a `$` alias.
    aliased: HashSet<&'static str>,
    templates: Vec<String>,
    counter: u32,
    /// Printing a `.jsbrain` module: statements live at module level and keep `export`.
    pub(crate) module: bool,
    /// `~` props: a write becomes `$p["bind:x"]?.(v)` (INTERNALS §12, model channel).
    pub(crate) bindable: HashSet<String>,
    /// `channels/cart.jsbrain` → `"channels/cart"`: third argument of `store()` (sync key, brain-ch.js).
    pub(crate) store_id: Option<String>,
    /// Names imported from `channels/*.jsbrain`: stores read reactively (their fields are signal getters).
    pub(crate) channel_names: HashSet<String>,
    /// Production folding (INTERNALS §13): `sig`/`drv` that compile to plain constants, `fx` run directly.
    pub(crate) folded: HashSet<String>,
    pub(crate) folded_fx: HashSet<u32>,
    /// Stateless `ui/` components inlined into this one (INTERNALS §13), by local name.
    pub(crate) inline: HashMap<String, inline::Inline>,
    /// Inlined uses being generated (innermost last).
    pub(crate) frames: Vec<inline::Frame>,
    /// Import sources whose every use was inlined (their scoped CSS travels in this component's CSS).
    pub(crate) inlined_sources: BTreeSet<String>,
    /// `titleTemplate` of the app's `<meta seo>` (`"%s · Acme"`): applied to a view's title at compile time.
    pub(crate) title_template: Option<String>,
    /// Type names referenced by `api()` schemas (imported from the app's types.jsbrain, §15).
    pub(crate) type_refs: BTreeSet<String>,
    /// i18n keys left for the runtime (see [`Output::i18n_keys`]).
    pub(crate) i18n_keys: BTreeSet<String>,
    /// A runtime `t()`/`locale()` call was emitted: only then are they imported (a local `t` is common).
    pub(crate) uses_i18n: bool,
}

/// Signals never written, derived values and effects that read nothing reactive (fixed point, conservative:
/// shadowing is ignored and any call to a user function counts as reactive).
fn fold_plan(c: &Component) -> (HashSet<String>, HashSet<u32>) {
    use brain_parse::visit::Uses;
    let all = Uses::component(c);
    let top = scope::top_bindings(c);
    let mut user_fns: HashSet<String> = HashSet::new();
    let mut sigs = Vec::new();
    let mut drvs: Vec<(String, &Expr)> = Vec::new();
    for s in &c.script {
        match s {
            Stmt::Func(f) | Stmt::Server { func: f, .. } => user_fns.extend(f.name.iter().map(|n| n.name.clone())),
            Stmt::Var(v) => {
                for d in &v.decls {
                    let Pat::Ident(id) = &d.pat else { continue };
                    match (scope::init_kind(d.init.as_ref()), d.init.as_ref().map(Expr::unparen)) {
                        (Some(Kind::Sig), _) => sigs.push(id.name.clone()),
                        (Some(Kind::Drv), Some(Expr::Call { callee, args, .. })) if matches!(callee.unparen(), Expr::Ident(k) if k.name == "drv") => {
                            if let Some(a) = args.first() {
                                drvs.push((id.name.clone(), a));
                            }
                        }
                        (None, Some(Expr::Func(_))) => {
                            user_fns.insert(id.name.clone());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let provided_raw = |n: &str| all.calls.contains("provide") && all.reads.contains(n);
    let mut folded: HashSet<String> = sigs.into_iter().filter(|n| !all.writes.contains(n) && !provided_raw(n)).collect();
    // Names whose reads are reactive once folding is applied.
    let reactive = |folded: &HashSet<String>| -> HashSet<String> {
        top.iter()
            .filter(|(n, k)| !folded.contains(*n) && (k.is_reactive_cell() || matches!(k, Kind::Prop | Kind::LoadProps | Kind::Form)))
            .map(|(n, _)| n.clone())
            .chain(c.script.iter().filter_map(|s| match s {
                Stmt::Import(i) if i.source.contains("channels/") => Some(import_names(i)),
                _ => None,
            }).flatten())
            .collect()
    };
    let pure = |e: &Expr, reactive: &HashSet<String>| {
        let mut u = Uses::default();
        u.expr(e);
        u.reads.is_disjoint(reactive) && u.calls.is_disjoint(&user_fns) && !u.calls.contains("use")
    };
    loop {
        let r = reactive(&folded);
        let more: Vec<String> = drvs.iter().filter(|(n, f)| !folded.contains(n) && pure(f, &r)).map(|(n, _)| n.clone()).collect();
        if more.is_empty() {
            break;
        }
        folded.extend(more);
    }
    let r = reactive(&folded);
    let mut fxs = HashSet::new();
    for s in &c.script {
        if let Stmt::Expr { expr, .. } = s
            && let Expr::Call { callee, args, span, .. } = expr.unparen()
            && matches!(callee.unparen(), Expr::Ident(k) if k.name == "fx")
            && let Some(Expr::Func(f)) = args.first().map(Expr::unparen)
            && f.params.is_empty()
            && !returns(&f.body)
            && pure(&args[0], &r)
        {
            fxs.insert(span.start);
        }
    }
    (folded, fxs)
}

/// Whether a function body returns anything (an effect's cleanup).
fn returns(b: &FnBody) -> bool {
    fn s(st: &Stmt) -> bool {
        match st {
            Stmt::Return { arg, .. } => arg.is_some(),
            Stmt::If { cons, alt, .. } => s(cons) || alt.as_deref().is_some_and(s),
            Stmt::Block { body, .. } => body.iter().any(s),
            Stmt::ForOf { body, .. } => s(body),
            Stmt::Try { block, handler, finalizer, .. } => block.iter().chain(handler.iter().flatten()).chain(finalizer.iter().flatten()).any(s),
            _ => false,
        }
    }
    match b {
        FnBody::Expr(_) => true,
        FnBody::Block(v) => v.iter().any(s),
    }
}

impl<'a> Gen<'a> {
    /// Marks a runtime export as used and returns the local name to call it by.
    pub(crate) fn rt(&mut self, name: &str) -> String {
        let n = RUNTIME_NAMES.iter().copied().find(|x| *x == name).expect("runtime name");
        self.used.insert(n);
        if self.aliased.contains(n) { format!("${n}") } else { n.to_string() }
    }

    pub(crate) fn fresh(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("${prefix}{}", self.counter)
    }

    /// Registers a module-level `tpl(html)` and returns its name.
    pub(crate) fn template_decl(&mut self, html: &str) -> String {
        if let Some(i) = self.templates.iter().position(|h| h == html) {
            return format!("$t{i}");
        }
        self.rt("tpl");
        self.templates.push(html.to_string());
        format!("$t{}", self.templates.len() - 1)
    }
}

fn collect_ids(nodes: &[Node], out: &mut HashSet<String>, dyn_ids: &mut bool) {
    for n in nodes {
        match n {
            Node::Element(e) => {
                for a in &e.attrs {
                    if a.kind == AttrKind::Plain
                        && a.name == "id"
                        && let AttrValue::Text(t) = &a.value
                    {
                        out.insert(t.clone());
                    }
                    // #308: `id={'faq-' + it.id}` / `id="faq-{it.id}"` needs the per-instance suffix too.
                    if a.kind == AttrKind::Plain
                        && a.name == "id"
                        && (matches!(&a.value, AttrValue::Mixed(_)) || matches!(&a.value, AttrValue::Expr(x) if !matches!(x.unparen(), Expr::Str { .. } | Expr::Num { .. } | Expr::Bool { .. } | Expr::Null(_))))
                    {
                        *dyn_ids = true;
                    }
                }
                collect_ids(&e.children, out, dyn_ids);
            }
            Node::ComponentRef(r) => collect_ids(&r.children, out, dyn_ids),
            Node::If(b) => {
                for br in &b.branches {
                    collect_ids(&br.body, out, dyn_ids);
                }
                if let Some(o) = &b.otherwise {
                    collect_ids(o, out, dyn_ids);
                }
            }
            Node::Each(b) => {
                collect_ids(&b.body, out, dyn_ids);
                if let Some(e) = &b.empty {
                    collect_ids(e, out, dyn_ids);
                }
            }
            Node::SlotScope(b) => collect_ids(&b.body, out, dyn_ids),
            Node::Await(b) => {
                collect_ids(&b.pending, out, dyn_ids);
                for br in [&b.then, &b.catch].into_iter().flatten() {
                    collect_ids(&br.body, out, dyn_ids);
                }
            }
            _ => {}
        }
    }
}

/// Every name bound anywhere in the template (each items, await bindings).
fn template_names(nodes: &[Node], out: &mut HashSet<String>) {
    for n in nodes {
        match n {
            Node::Element(e) => template_names(&e.children, out),
            Node::ComponentRef(r) => template_names(&r.children, out),
            Node::If(b) => {
                for br in &b.branches {
                    template_names(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    template_names(o, out);
                }
            }
            Node::Each(b) => {
                let mut v = Vec::new();
                b.item.bound_names(&mut v);
                out.extend(v.into_iter().map(|i| i.name.clone()));
                if let Some(i) = &b.index {
                    out.insert(i.name.clone());
                }
                template_names(&b.body, out);
            }
            Node::SlotScope(b) => {
                out.extend(b.names.iter().map(|i| i.name.clone()));
                template_names(&b.body, out);
            }
            Node::Await(b) => {
                for br in [&b.then, &b.catch].into_iter().flatten() {
                    if let Some(p) = &br.binding {
                        let mut v = Vec::new();
                        p.bound_names(&mut v);
                        out.extend(v.into_iter().map(|i| i.name.clone()));
                    }
                    template_names(&br.body, out);
                }
            }
            _ => {}
        }
    }
}

fn is_literal(e: &Expr) -> bool {
    matches!(e.unparen(), Expr::Num { .. } | Expr::Str { .. } | Expr::Bool { .. } | Expr::Null(_))
        || matches!(e.unparen(), Expr::Unary { op: "-", arg, .. } if matches!(arg.unparen(), Expr::Num { .. }))
}

/// `import` lines of the user's script, with `import_suffix` applied to `.brain`/`.jsbrain`.
/// `keep` decides which imports go in (client: all but server modules; server: only the ones used there).
fn user_imports(c: &Component, opts: &Options, keep: &dyn Fn(&Import) -> bool) -> String {
    let mut js = String::new();
    let mut workers = 0;
    for s in &c.script {
        if let Stmt::Import(i) = s
            && keep(i)
            && brain_check::worker::is_worker(&i.source)
        {
            let dir = Path::new(&c.file).parent().unwrap_or(Path::new(""));
            let disk = opts.root.as_ref().map(|r| r.join(dir).join(&i.source)).unwrap_or_else(|| dir.join(&i.source));
            let exports = std::fs::read_to_string(&disk).ok().and_then(|src| brain_parse::parse(&src, &disk).ok()).map(|m| worker_exports(&m));
            js.push_str(&worker_stub(i, opts, workers, exports.as_ref()));
            workers += 1;
            continue;
        }
        if let Stmt::Import(i) = s
            && keep(i)
        {
            let mut what = Vec::new();
            if let Some(d) = &i.default {
                what.push(d.name.clone());
            }
            if let Some(n) = &i.namespace {
                what.push(format!("* as {}", n.name));
            }
            if !i.named.is_empty() {
                let v: Vec<String> = i.named.iter().map(|(a, b)| if a.name == b.name { a.name.clone() } else { format!("{} as {}", a.name, b.name) }).collect();
                what.push(format!("{{ {} }}", v.join(", ")));
            }
            let src = if i.source.ends_with(".brain") || i.source.ends_with(".jsbrain") { format!("{}{}", i.source, opts.import_suffix) } else { i.source.clone() };
            if what.is_empty() {
                js.push_str(&format!("import {};\n", js_str(&src)));
            } else {
                js.push_str(&format!("import {} from {};\n", what.join(", "), js_str(&src)));
            }
        }
    }
    js
}

fn runtime_import(g: &Gen, opts: &Options) -> String {
    let names: Vec<String> = RUNTIME_NAMES
        .iter()
        .copied()
        .filter(|n| g.used.contains(n))
        .map(|n| if g.aliased.contains(n) { format!("{n} as ${n}") } else { n.to_string() })
        .collect();
    if names.is_empty() { String::new() } else { format!("import {{ {} }} from {};\n", names.join(", "), js_str(&opts.runtime)) }
}

/// `.jsbrain`: the script at module level, same rewrites, no component wrapper.
fn generate_module(g: &mut Gen, c: &Component, opts: &Options) -> String {
    g.indent = 0;
    let worker = brain_check::worker::is_worker(&c.file);
    let mut body = String::new();
    for s in &c.script {
        match s {
            // A worker keeps a handle on its default export to serve it.
            Stmt::ExportDefault { expr, .. } if worker => body.push_str(&format!("const $default = {};\nexport default $default;\n", g.expr(expr))),
            _ => {
                body.push_str(&srcmap::mark(s.span().line));
                body.push_str(&g.stmt(s));
            }
        }
    }
    if worker {
        // §35: the module runs in a dedicated Web Worker and answers calls from its importers (brain-worker.js).
        let ex = worker_exports(c);
        let mut fns: Vec<String> = ex.named.clone();
        if ex.has_default {
            fns.push("default: $default".into());
        }
        let dir = opts.runtime.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
        body.push_str(&format!("import {{ serve as $serve }} from {};\n$serve({{ {} }});\n", js_str(&format!("{dir}/brain-worker.js")), fns.join(", ")));
    }
    let declared: HashSet<String> = scope::top_bindings(c).into_keys().collect();
    let auto = auto_imports(&body, &declared, &BTreeSet::new(), opts, g.uses_i18n, &author_names(c));
    let types = types_import(g, c, &declared, opts);
    format!("{}{auto}{types}{}{body}", runtime_import(g, opts), user_imports(c, opts, &|_| true))
}

/// `import { User, Post } from "../types.jsbrain"` for the type names `api()` schemas use (§15).
fn types_import(g: &Gen, c: &Component, declared: &HashSet<String>, opts: &Options) -> String {
    let names: Vec<&String> = g.type_refs.iter().filter(|n| !declared.contains(*n)).collect();
    if names.is_empty() || c.file == "types.jsbrain" {
        return String::new();
    }
    let depth = Path::new(&c.file).parent().map(|p| p.components().count()).unwrap_or(0);
    let spec = if depth == 0 { "./types.jsbrain".to_string() } else { format!("{}types.jsbrain", "../".repeat(depth)) };
    format!("import {{ {} }} from {};\n", names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "), js_str(&format!("{spec}{}", opts.import_suffix)))
}

/// Names a worker module exports: named exports and, for `export default { a, b }`, the keys of that object.
pub(crate) struct WorkerExports {
    pub named: Vec<String>,
    pub has_default: bool,
    pub default_keys: Vec<String>,
}

pub(crate) fn worker_exports(c: &Component) -> WorkerExports {
    let mut w = WorkerExports { named: Vec::new(), has_default: false, default_keys: Vec::new() };
    for s in &c.script {
        match s {
            Stmt::Export { decl, .. } => match decl.as_ref() {
                Stmt::Var(v) => {
                    for d in &v.decls {
                        if let Pat::Ident(i) = &d.pat {
                            w.named.push(i.name.clone());
                        }
                    }
                }
                Stmt::Func(Func { name: Some(n), .. }) => w.named.push(n.name.clone()),
                _ => {}
            },
            Stmt::ExportDefault { expr, .. } => {
                w.has_default = true;
                if let Expr::Object { props, .. } = expr.unparen() {
                    for p in props {
                        match p {
                            ObjProp::Shorthand(i) => w.default_keys.push(i.name.clone()),
                            ObjProp::KeyValue { key: PropKey::Ident(k), .. } | ObjProp::Method { key: PropKey::Ident(k), .. } => w.default_keys.push(k.name.clone()),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    w
}

/// Importer side of a worker module (§35): every import becomes async functions that post to the worker, which
/// `brain-worker.js` starts on first use. `exports` are read from the worker file when it is on disk.
fn worker_stub(i: &Import, opts: &Options, n: usize, exports: Option<&WorkerExports>) -> String {
    let dir = opts.runtime.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
    let src = if i.source.ends_with(".jsbrain") { format!("{}{}", i.source, opts.import_suffix) } else { i.source.clone() };
    let w = format!("$w{n}");
    let mut js = if n == 0 { format!("import {{ worker as $worker }} from {};\n", js_str(&format!("{dir}/brain-worker.js"))) } else { String::new() };
    js.push_str(&format!("const {w} = $worker(new URL({}, import.meta.url));\n", js_str(&src)));
    let call = |name: &str| format!("(...a) => {w}.call({}, a)", js_str(name));
    for (imported, local) in &i.named {
        js.push_str(&format!("const {} = {};\n", local.name, call(&imported.name)));
    }
    let keys: Vec<String> = exports.map(|e| e.default_keys.clone()).unwrap_or_default();
    if let Some(d) = &i.default {
        let fns: Vec<String> = keys.iter().map(|k| format!("{k}: {}", call(&format!("default.{k}")))).collect();
        js.push_str(&format!("const {} = {{ {} }};\n", d.name, fns.join(", ")));
    }
    if let Some(ns) = &i.namespace {
        let mut fns: Vec<String> = exports.map(|e| e.named.iter().map(|k| format!("{k}: {}", call(k))).collect()).unwrap_or_default();
        if !keys.is_empty() {
            let d: Vec<String> = keys.iter().map(|k| format!("{k}: {}", call(&format!("default.{k}")))).collect();
            fns.push(format!("default: {{ {} }}", d.join(", ")));
        }
        js.push_str(&format!("const {} = {{ {} }};\n", ns.name, fns.join(", ")));
    }
    js
}

/// `brain/ui` components referenced in markup.
fn ui_refs(nodes: &[Node], out: &mut BTreeSet<String>) {
    for n in nodes {
        match n {
            Node::Element(e) => ui_refs(&e.children, out),
            Node::ComponentRef(r) => {
                if scope::UI_KIT.contains(&r.name.as_str()) {
                    out.insert(r.name.clone());
                }
                ui_refs(&r.children, out);
            }
            Node::If(b) => {
                for br in &b.branches {
                    ui_refs(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    ui_refs(o, out);
                }
            }
            Node::Each(b) => {
                ui_refs(&b.body, out);
                if let Some(e) = &b.empty {
                    ui_refs(e, out);
                }
            }
            Node::SlotScope(b) => ui_refs(&b.body, out),
            Node::Await(b) => {
                ui_refs(&b.pending, out);
                for br in [&b.then, &b.catch].into_iter().flatten() {
                    ui_refs(&br.body, out);
                }
            }
            _ => {}
        }
    }
}

/// Imports for primitives outside the core and for `brain/ui` components, none of which the user imports
/// (SPEC). Modules sit next to `opts.runtime`; the kit under `<dir>/ui/<name>.brain`.
/// Names the author wrote (script, markup, `<head>`, `<meta seo>` and prop defaults). An author-facing primitive is
/// imported only when it is in here *and* in the generated code: a word in a string or a comment is not a use.
fn author_names(c: &Component) -> HashSet<String> {
    let mut u = brain_parse::visit::Uses::component(c);
    u.nodes(&c.head);
    for m in &c.meta {
        if let Some(e) = &m.expr {
            u.expr(e);
        }
    }
    let mut names: HashSet<String> = u.reads.into_iter().chain(u.calls).chain(u.writes).collect();
    // Written as `<meta http>`, emitted as `http({ … })`.
    if c.kind == FileKind::App && c.meta.iter().any(|m| m.name == "http") {
        names.insert("http".into());
    }
    names
}

fn auto_imports(code: &str, declared: &HashSet<String>, ui: &BTreeSet<String>, opts: &Options, i18n: bool, author: &HashSet<String>) -> String {
    let dir = opts.runtime.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
    let used = |name: &str| {
        let w = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
        code.match_indices(name).any(|(i, _)| {
            let before = code[..i].chars().next_back();
            let after = code[i + name.len()..].chars().next();
            !before.is_some_and(|c| w(c) || matches!(c, '.' | ':' | '"' | '\'' | '-')) && !after.is_some_and(|c| w(c) || matches!(c, ':' | '"' | '\''))
        })
    };
    let mut by_mod: Vec<(&str, Vec<&str>)> = Vec::new();
    for (name, module) in scope::EXT_PRIMITIVES {
        if declared.contains(*name) || !used(name) || (matches!(*name, "t" | "locale") && !i18n) || (scope::PRIMITIVES.contains(name) && !author.contains(*name))
            // Emitted by the codegen as calls only; the word in template text is not a use.
            || (matches!(*name, "dyn" | "portal" | "transition" | "lazy" | "fc" | "nx") && !code.contains(&format!("{name}("))) {
            continue;
        }
        match by_mod.iter_mut().find(|(m, _)| m == module) {
            Some((_, v)) => v.push(name),
            None => by_mod.push((module, vec![name])),
        }
    }
    let mut js = String::new();
    for (module, names) in by_mod {
        let names: Vec<String> = names.iter().map(|n| match n.strip_prefix('$') { Some(b) => format!("{b} as {n}"), None => n.to_string() }).collect();
        js.push_str(&format!("import {{ {} }} from {};\n", names.join(", "), js_str(&format!("{dir}/{module}"))));
    }
    for name in ui {
        if !declared.contains(name) {
            js.push_str(&format!("import {name} from {};\n", js_str(&format!("{dir}/ui/{}.brain{}", kebab(name), opts.import_suffix))));
        }
    }
    js
}

fn import_names(i: &Import) -> Vec<String> {
    i.default.iter().chain(i.namespace.iter()).chain(i.named.iter().map(|(_, l)| l)).map(|x| x.name.clone()).collect()
}

/// Whole-word occurrence of `name` in generated code.
fn mentions(code: &str, name: &str) -> bool {
    let w = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    code.match_indices(name).any(|(i, _)| {
        let before = &code[..i];
        // `ctx.session` is a property, not the import `session` (a spread `...session` still is).
        let member = before.ends_with('.') && !before.ends_with("...");
        !member && !before.chars().next_back().is_some_and(w) && !code[i + name.len()..].chars().next().is_some_and(w)
    })
}

/// A top-level `function f` or `const f = (…) => …`.
fn as_fn(s: &Stmt) -> bool {
    matches!(s, Stmt::Func(_)) || matches!(s, Stmt::Var(v) if v.decls.len() == 1 && matches!(v.decls[0].init.as_ref().map(Expr::unparen), Some(Expr::Func(_))))
}

/// `let f = form(schema, { submit: save })` where `save` is a `server:` function of the file: (var, fn, schema).
/// The `server:` function a `submit: async (v) => { … await save(v) … }` hands its values to.
pub(crate) fn submit_target(func: &Func, server: &HashSet<&str>) -> Option<String> {
    let Some(Pat::Ident(param)) = func.params.first() else { return None };
    let mut hit = None;
    let mut look = |e: &Expr| {
        if let Expr::Call { callee, args, .. } = e
            && let Expr::Ident(f) = callee.unparen()
            && server.contains(f.name.as_str())
            && matches!(args.first().map(Expr::unparen), Some(Expr::Ident(a)) if a.name == param.name)
            && hit.is_none()
        {
            hit = Some(f.name.clone());
        }
    };
    brain_check::each_expr(&Expr::Func(Box::new(func.clone())), &mut look);
    hit
}

pub(crate) fn server_forms(c: &Component) -> Vec<(String, String, &Expr)> {
    let server: HashSet<&str> = c.script.iter().filter_map(|s| if let Stmt::Server { func, .. } = s { func.name.as_ref().map(|n| n.name.as_str()) } else { None }).collect();
    let mut out = Vec::new();
    for s in &c.script {
        let Stmt::Var(v) = s else { continue };
        for d in &v.decls {
            let (Pat::Ident(id), Some(Expr::Call { callee, args, .. })) = (&d.pat, d.init.as_ref().map(Expr::unparen)) else { continue };
            if !matches!(callee.unparen(), Expr::Ident(k) if k.name == "form") {
                continue;
            }
            let (Some(schema), Some(Expr::Object { props, .. })) = (args.first(), args.get(1).map(Expr::unparen)) else { continue };
            let submit = props.iter().find_map(|p| match p {
                ObjProp::KeyValue { key: PropKey::Ident(k), value: Expr::Ident(f), .. } if k.name == "submit" => Some(f.name.clone()),
                ObjProp::Shorthand(f) if f.name == "submit" => Some(f.name.clone()),
                // `submit: async (v) => { … await login(v) … }`: the server function it hands the values to.
                ObjProp::KeyValue { key: PropKey::Ident(k), value: Expr::Func(func), .. } if k.name == "submit" => submit_target(func, &server),
                _ => None,
            });
            if let Some(f) = submit.filter(|f| server.contains(f.as_str())) {
                out.push((id.name.clone(), f, schema));
            }
        }
    }
    out
}

/// `load` and `server:` functions as a server ESM; they see only the imports and each other.
fn generate_server(g: &mut Gen, c: &Component, opts: &Options) -> (String, Vec<String>) {
    let mut fns = Vec::new();
    let mut scope_top = HashMap::new();
    for s in &c.script {
        if let Stmt::Import(_) = s {
            scope::declare_stmt(s, &mut scope_top);
        }
    }
    let mut body = String::new();
    let saved = std::mem::replace(&mut g.scopes, Scopes::new(scope_top));
    let saved_indent = std::mem::replace(&mut g.indent, 0);
    for s in &c.script {
        match s {
            Stmt::Server { func, .. } => {
                if let Some(n) = &func.name {
                    g.scopes.declare(&n.name, Kind::Func);
                    fns.push(n.name.clone());
                }
            }
            Stmt::Export { decl, .. } if is_load(decl) => {
                if let Stmt::Func(Func { name: Some(n), .. }) = decl.as_ref() {
                    g.scopes.declare(&n.name, Kind::Func);
                }
            }
            _ => {}
        }
    }
    // #276: pure constants and server-only helpers, copied in source order before the server functions.
    let shared = brain_check::servershare::shared(c);
    for s in &c.script {
        let name = match s {
            Stmt::Func(f) => f.name.as_ref().map(|n| n.name.clone()),
            Stmt::Var(v) if v.decls.len() == 1 => match &v.decls[0].pat {
                Pat::Ident(i) => Some(i.name.clone()),
                _ => None,
            },
            _ => None,
        };
        let Some(name) = name.filter(|n| shared.contains(n)) else { continue };
        g.scopes.declare(&name, if as_fn(s) { Kind::Func } else { Kind::Const });
        body.push_str(&g.stmt(s));
    }
    for s in &c.script {
        match s {
            Stmt::Server { func, .. } => body.push_str(&format!("export {}\n", g.func_expr(func))),
            Stmt::Export { decl, .. } if is_load(decl) => {
                let Stmt::Func(f) = decl.as_ref() else { unreachable!() };
                body.push_str(&format!("export {}\n", g.func_expr(f)));
            }
            _ => {}
        }
    }
    // §38: a form posted without JS runs its `server:` function with the same schema as `form()` (server.mjs callForm).
    let forms = server_forms(c);
    if !forms.is_empty() {
        let entries: Vec<String> = forms.iter().map(|(_, f, schema)| format!("{f}: {}", g.expr(schema))).collect();
        body.push_str(&format!("export const $forms = {{ {} }};\n", entries.join(", ")));
    }
    // §32: `<meta seo>` values and the `<head>` block, resolved at SSR/prerender with the `load` result.
    if c.level == Level::View && c.kind == FileKind::Component {
        if let Some(seo) = seo_object(g, c) {
            body.push_str(&format!("export const seo = (props) => ({seo});\n"));
        }
        if !c.head.is_empty() {
            body.push_str(&format!("export const head = (props) => {};\n", head_html(g, &c.head)));
        }
    }
    g.scopes = saved;
    g.indent = saved_indent;
    if body.is_empty() {
        return (String::new(), fns);
    }
    let imports = user_imports(c, opts, &|i| import_names(i).iter().any(|n| mentions(&body, n)));
    // §33: collection readers used by `load`/`paths`/`server:` come from the build's `brain:content` module.
    let declared: HashSet<String> = scope::top_bindings(c).into_keys().collect();
    let content: Vec<&str> = scope::CONTENT.iter().copied().filter(|n| !declared.contains(*n) && body.contains(&format!("{n}("))).filter(|n| mentions(&body, n)).collect();
    let content = if content.is_empty() { String::new() } else { format!("import {{ {} }} from \"brain:content\";\n", content.join(", ")) };
    // §37: `t()`/`locale()`/`fmt` left in server code (`<meta seo title={t('…')}>`, `load`) read the request's locale.
    let dir = opts.runtime.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
    let i18n: Vec<&str> = ["t", "locale", "fmt"].into_iter().filter(|n| !declared.contains(*n) && mentions(&body, n) && (body.contains(&format!("{n}(")) || body.contains(&format!("{n}.")))).collect();
    let i18n = if i18n.is_empty() { String::new() } else { format!("import {{ {} }} from {};\n", i18n.join(", "), js_str(&format!("{dir}/brain-i18n.js"))) };
    (format!("{content}{i18n}{imports}{body}"), fns)
}

/// `load` and `paths` (§32) run on the server only.
fn is_load(decl: &Stmt) -> bool {
    matches!(decl, Stmt::Func(f) if f.name.as_ref().is_some_and(|n| n.name == "load" || n.name == "paths"))
}

/// `{ "title": …, "og:image": …, "jsonld": … }` from `<meta seo>` (the `seo` flag itself excluded).
fn seo_object(g: &mut Gen, c: &Component) -> Option<String> {
    let start = c.meta.iter().position(|m| m.name == "seo")?;
    let mut parts = Vec::new();
    for m in &c.meta[start + 1..] {
        if matches!(m.name.as_str(), "props" | "emits" | "expose" | "export" | "tools" | "server" | "fallback" | "cache" | "http" | "client" | "budget" | "route") {
            break;
        }
        // `keyword` is for the audit only: never rendered.
        if m.name == "keyword" {
            continue;
        }
        let v = match (&m.expr, &m.value) {
            (Some(x), _) => g.expr(x),
            (None, Some(t)) => js_str(t),
            (None, None) => "true".into(),
        };
        // The app's `titleTemplate` is resolved here, so SSR and client navigation show the same title.
        let v = match (&g.title_template, m.name.as_str()) {
            (Some(tpl), "title") if tpl.contains("%s") => {
                let (a, b) = tpl.split_once("%s").unwrap_or((tpl, ""));
                match (&m.expr, &m.value) {
                    (None, Some(t)) => js_str(&format!("{a}{t}{b}")),
                    _ => format!("{} + ({v}) + {}", js_str(a), js_str(b)),
                }
            }
            _ => v,
        };
        parts.push(format!("{}: {v}", js_str(&m.name)));
    }
    Some(format!("{{ {} }}", parts.join(", ")))
}

/// The `<head>` block as a template literal; interpolations are HTML-escaped.
fn head_html(g: &mut Gen, nodes: &[Node]) -> String {
    fn esc_tpl(s: &str) -> String {
        s.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${")
    }
    fn walk(g: &mut Gen, nodes: &[Node], out: &mut String) {
        for n in nodes {
            match n {
                Node::Text(t) if t.value.trim().is_empty() => {}
                Node::Text(t) => out.push_str(&esc_tpl(&t.value)),
                Node::Interp(i) => out.push_str(&format!("${{$esc({})}}", g.expr(&i.expr))),
                Node::Element(e) => {
                    out.push('<');
                    out.push_str(&e.tag);
                    for a in &e.attrs {
                        match &a.value {
                            AttrValue::None => out.push_str(&format!(" {}", a.name)),
                            AttrValue::Text(t) => out.push_str(&format!(" {}=\"{}\"", a.name, esc_tpl(&t.replace('"', "&quot;")))),
                            AttrValue::Expr(x) => out.push_str(&format!(" {}=\"${{$esc({})}}\"", a.name, g.expr(x))),
                            AttrValue::Mixed(parts) => {
                                out.push_str(&format!(" {}=\"", a.name));
                                for p in parts {
                                    match p {
                                        AttrPart::Text(t) => out.push_str(&esc_tpl(&t.replace('"', "&quot;"))),
                                        AttrPart::Expr(x) => out.push_str(&format!("${{$esc({})}}", g.expr(x))),
                                    }
                                }
                                out.push('"');
                            }
                        }
                    }
                    out.push('>');
                    if !matches!(e.tag.as_str(), "meta" | "link" | "base") {
                        walk(g, &e.children, out);
                        out.push_str(&format!("</{}>", e.tag));
                    }
                }
                _ => {}
            }
        }
    }
    let mut s = String::new();
    walk(g, nodes, &mut s);
    format!("(($esc) => `{}`)((v) => String(v ?? \"\").replace(/[&<>\"]/g, (c) => \"&#\" + c.charCodeAt(0) + \";\"))", s.trim())
}

fn generate(c: &Component, opts: &Options, server_srcs: &HashSet<String>, inline: HashMap<String, inline::Inline>, title_template: Option<String>) -> (String, String, String, Vec<String>, Vec<String>) {
    let mut top = scope::top_bindings(c);
    let (folded, folded_fx) = if opts.dev || c.kind != FileKind::Component { (HashSet::new(), HashSet::new()) } else { fold_plan(c) };
    for n in &folded {
        top.insert(n.clone(), Kind::Const);
    }
    let mut user_names: HashSet<String> = top.keys().cloned().collect();
    template_names(&c.template, &mut user_names);
    let aliased = RUNTIME_NAMES.iter().copied().filter(|n| user_names.contains(*n)).collect();
    let mut ids = HashSet::new();
    let mut dyn_ids = false;
    // Views, layouts and app.brain render once per document: their ids are public URL fragments (`/pricing#faq`)
    // and keep their name. Only reusable components (`comp/`, `ui/`) get a per-instance suffix.
    if c.level != Level::View && c.kind != FileKind::App {
        collect_ids(&c.template, &mut ids, &mut dyn_ids);
    }
    let mut g = Gen {
        opts,
        forms: server_forms(c).into_iter().map(|(v, f, _)| (v, f)).collect(),
        lazy_src: {
            let dir = opts.runtime.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
            let mut m: HashMap<String, String> = scope::UI_KIT.iter().map(|n| (n.to_string(), format!("{dir}/ui/{}.brain{}", kebab(n), opts.import_suffix))).collect();
            for st in &c.script {
                if let Stmt::Import(i) = st
                    && let Some(d) = &i.default
                    && i.source.ends_with(".brain")
                {
                    m.insert(d.name.clone(), format!("{}{}", i.source, opts.import_suffix));
                }
            }
            m
        },
        node_vars: c
            .script
            .iter()
            .filter_map(|s| if let Stmt::Var(v) = s { Some(v) } else { None })
            .flat_map(|v| v.decls.iter())
            .filter_map(|d| match (&d.pat, d.init.as_ref().map(Expr::unparen)) {
                (Pat::Ident(id), Some(Expr::Call { callee, .. })) if matches!(callee.unparen(), Expr::Ident(k) if matches!(k.name.as_str(), "resource" | "stream")) => Some(id.name.clone()),
                _ => None,
            })
            .collect(),
        scopes: Scopes::new(top),
        indent: 1,
        hash: component_hash_len(&c.file, opts.hash_len),
        prop_defaults: HashMap::new(),
        ids,
        dyn_ids,
        uses_sx: false,
        used: BTreeSet::new(),
        aliased,
        templates: Vec::new(),
        counter: 0,
        module: c.kind == FileKind::Module,
        bindable: c.props.iter().filter(|p| p.bindable).map(|p| p.name.clone()).collect(),
        folded,
        folded_fx,
        inline,
        frames: Vec::new(),
        inlined_sources: BTreeSet::new(),
        title_template,
        type_refs: BTreeSet::new(),
        i18n_keys: BTreeSet::new(),
        uses_i18n: false,
        channel_names: c
            .script
            .iter()
            .filter_map(|s| match s {
                Stmt::Import(i) if i.source.contains("channels/") => Some(import_names(i)),
                _ => None,
            })
            .flatten()
            .collect(),
        store_id: {
            let f = c.file.trim_end_matches(".jsbrain");
            f.find("channels/").filter(|_| c.kind == FileKind::Module).map(|i| f[i..].to_string())
        },
    };
    if g.module {
        let js = generate_module(&mut g, c, opts);
        if c.meta.iter().any(|m| m.name == "server") {
            return (String::new(), String::new(), js, Vec::new(), g.i18n_keys.into_iter().collect());
        }
        return (js, String::new(), String::new(), Vec::new(), g.i18n_keys.into_iter().collect());
    }
    let (server_js, server_fns) = generate_server(&mut g, c, opts);
    let name = if c.name.is_empty() { "Component".to_string() } else { c.name.clone() };

    // Per-instance prelude: non-literal prop defaults are created once per instance so that
    // `$p.items ?? []` does not allocate a new array on every read.
    let mut body = String::new();
    for p in &c.props {
        if let Some(d) = &p.default {
            let v = g.expr(d);
            if is_literal(d) {
                g.prop_defaults.insert(p.name.clone(), v);
            } else {
                let var = format!("$d_{}", p.name);
                body.push_str(&format!("  const {var} = {v};\n"));
                g.prop_defaults.insert(p.name.clone(), var);
            }
        }
    }
    if !g.ids.is_empty() || g.dyn_ids {
        body.push_str("  // #268: the first instance keeps the ids as written (`/#pricing` works); later ones get `~2`, `~3`…\n  const $u = $n++ ? \"~\" + $n : \"\";\n");
    }
    let server_helpers = brain_check::servershare::shared(c).helpers;
    // `load` and `server:` bodies live in `server_js`; the client only gets an RPC stub per server function.
    let mut module_fns = String::new();
    // §40.3: `<meta http …>` in app.brain is the app-wide policy, applied once when the shell module loads.
    if let Some(h) = brain_check::http::policy_js(c) {
        module_fns.push_str(&h);
    }
    for f in &server_fns {
        let rpc = g.rt("rpc");
        module_fns.push_str(&format!("const {f} = (...a) => {rpc}({}, a);\n{f}.id = {};\n", js_str(&format!("{}/{f}", g.hash)), js_str(&format!("{}/{f}", g.hash))));
    }
    for s in &c.script {
        match s {
            Stmt::Import(_) | Stmt::Server { .. } => {}
            // #276: helpers only server code reaches live in the server module.
            Stmt::Func(Func { name: Some(n), .. }) if server_helpers.contains(&n.name) => {}
            Stmt::Var(v) if v.decls.len() == 1 && matches!(&v.decls[0].pat, Pat::Ident(i) if server_helpers.contains(&i.name)) => {}
            Stmt::Export { decl, .. } if is_load(decl) => {}
            Stmt::Var(v) => body.push_str(&format!("{}  {};\n", srcmap::mark(v.span.line), g.var_decl(v, true))),
            other => {
                body.push_str(&srcmap::mark(other.span().line));
                body.push_str(&g.stmt(other));
            }
        }
    }
    // `<meta expose="open close">`: hand the parent its imperative API at the end of setup.
    let exposed = brain_check::meta_names(c, "expose");
    if !exposed.is_empty() {
        body.push_str(&format!("  expose($p, {{ {} }});\n", exposed.join(", ")));
    }
    // `<meta tools>` (WebMCP, §32): registered while the component is mounted (brain-mcp.js unregisters on dispose).
    let tools: Vec<brain_check::tools::Tool> = c.meta.iter().filter(|m| m.name == "tools").flat_map(|m| brain_check::tools::parse_tools(m.value.as_deref().unwrap_or(""))).collect();
    if !tools.is_empty() {
        let list: Vec<String> = tools.iter().map(tool_js).collect();
        body.push_str(&format!("  registerTools([{}]);\n", list.join(", ")));
    }
    body.push_str(&g.fragment(&c.template, true));

    // §38 `<meta client>`: on the server (SSR, prerender) the component is a sized placeholder; the browser mounts it.
    if brain_check::http::meta_tag(c, "client").is_some() {
        let t = g.template_decl(&format!("<div data-b=\"{}\" data-client aria-busy=\"true\"></div>", g.hash));
        body.insert_str(0, &format!("  if (globalThis.__brainSSR || globalThis.__brainRoute) return {t}();\n"));
    }
    // ── assemble the module ──
    let mut js = String::new();
    let wc = c.meta.iter().any(|m| m.name == "export" && m.value.as_deref() == Some("wc"));
    js.push_str(&runtime_import(&g, opts));
    if opts.dev {
        js.push_str(&format!("import {{ dev }} from {};\n", js_str(&opts.runtime_dev)));
    }
    if wc {
        js.push_str(&format!("import {{ define }} from {};\n", js_str(&opts.runtime_wc)));
    }
    let mut ui = BTreeSet::new();
    ui_refs(&c.template, &mut ui);
    // A kit component used only with `lazy` is loaded by `lazy()`, never imported up front.
    ui.retain(|n| mentions(&body, n));
    let declared: HashSet<String> = scope::top_bindings(c).into_keys().collect();
    // `scope()` wraps the export when the component provides context (see below); `expose()` is in the body.
    let author = author_names(c);
    let provides = if mentions(&body, "provide") && author.contains("provide") { "scope()" } else { "" };
    js.push_str(&auto_imports(&format!("{module_fns}{body}{provides}"), &declared, &ui, opts, g.uses_i18n, &author));
    js.push_str(&types_import(&g, c, &declared, opts));
    // Imports used only by `load`/`server:` stay on the server; side-effect imports always stay.
    let client_code = format!("{module_fns}{body}");
    js.push_str(&user_imports(c, opts, &|i| {
        let names = import_names(i);
        !server_srcs.contains(&i.source) && (names.is_empty() || names.iter().any(|n| mentions(&client_code, n)))
    }));
    for (i, h) in g.templates.iter().enumerate() {
        let tpl = if g.aliased.contains("tpl") { "$tpl" } else { "tpl" };
        js.push_str(&format!("const $t{i} = {tpl}({});\n", js_str(h)));
    }
    if !g.ids.is_empty() || g.dyn_ids {
        js.push_str("let $n = 0;\n");
    }
    if g.uses_sx {
        // #308: suffix the ids an ID reference computed at run time names. With computed ids every token is local;
        // otherwise only the component's static ids are (a reference to an id outside it stays as written).
        if g.dyn_ids {
            js.push_str("const $sx = (v, u) => u && v != null && v !== false ? String(v).split(\" \").map((x) => x && x + u).join(\" \") : v;\n");
        } else {
            let mut list: Vec<&String> = g.ids.iter().collect();
            list.sort();
            let items: Vec<String> = list.iter().map(|x| js_str(x)).collect();
            js.push_str(&format!("const $ix = new Set([{}]);\nconst $sx = (v, u) => u && v != null && v !== false ? String(v).split(\" \").map((x) => $ix.has(x) ? x + u : x).join(\" \") : v;\n", items.join(", ")));
        }
    }
    js.push_str(&module_fns);
    js.push_str(&format!("function {name}($p = {{}}) {{\n{body}}}\n"));
    js.push_str(&format!("{name}.h = {};\n", js_str(&g.hash)));
    // §32: the router swaps the head on client navigation with the view's `load` props.
    if c.level == Level::View && c.kind == FileKind::Component {
        if let Some(seo) = seo_object(&mut g, c) {
            js.push_str(&format!("{name}.seo = ($p) => ({seo});\n"));
        }
        if !c.head.is_empty() {
            js.push_str(&format!("{name}.head = ($p) => {};\n", head_html(&mut g, &c.head)));
        }
    }
    if wc {
        let props: Vec<String> = c.props.iter().map(|p| js_str(&p.name)).collect();
        js.push_str(&format!("{name}.p = [{}];\n", props.join(", ")));
        if c.meta.iter().any(|m| m.name == "form") {
            js.push_str(&format!("{name}.form = true;\n"));
        }
    }
    // A component that provides context gets its own owner so only its descendants see it.
    let wrapped = if !provides.is_empty() { format!("scope({name})") } else { name.clone() };
    // §42: every comp/, views/ and app.brain component is its own error boundary (RUNTIME brain-guard.js); `ui/`
    // falls back to its parent's. `<meta fallback="…">` only customizes the text. dev() stays outside so HMR finds `.h`.
    let wrapped = if c.level != Level::Micro && c.kind != FileKind::Module && scope::runtime_has("brain-guard.js", "guard") {
        let dir = opts.runtime.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
        js.insert_str(0, &format!("import {{ guard }} from {};\n", js_str(&format!("{dir}/brain-guard.js"))));
        js.push_str(&format!("{name}.f = {};\n", js_str(&c.file)));
        let fb = c.meta.iter().find(|m| m.name == "fallback").and_then(|m| m.value.as_deref()).map(js_str);
        // §30.3: `<meta budget="rows:200000">` raises the runtime fuse (B0908) for this component.
        let budget = brain_check::http::budget(c);
        let limits = if budget.is_empty() { String::new() } else { format!(", {{ {} }}", budget.iter().map(|(k, n)| format!("{k}: {n}")).collect::<Vec<_>>().join(", ")) };
        match (fb, limits.is_empty()) {
            (Some(t), _) => format!("guard({wrapped}, {t}{limits})"),
            (None, true) => format!("guard({wrapped})"),
            (None, false) => format!("guard({wrapped}, undefined{limits})"),
        }
    } else {
        wrapped
    };
    if opts.dev {
        js.push_str(&format!("export default dev({wrapped});\n"));
    } else {
        js.push_str(&format!("export default {wrapped};\n"));
    }

    // ── CSS ──
    let mut css = component_css(c, &g.hash);
    if let Some(p) = brain_check::http::client_placeholder_css(c, &g.hash) {
        css.push_str(&p);
    }
    // Inlined `ui/` components carry their scoped CSS here: no chunk or import is left for them.
    let mut inlined: Vec<&inline::Inline> = g.inline.values().filter(|i| i.has_style && g.inlined_sources.contains(&i.source)).collect();
    inlined.sort_by(|a, b| a.source.cmp(&b.source));
    for i in inlined {
        let child = component_css(&i.comp, &i.hash);
        if !child.is_empty() {
            if !css.is_empty() && !css.ends_with('\n') {
                css.push('\n');
            }
            css.push_str(&child);
        }
    }
    if wc && let Some(tag) = c.meta.iter().find(|m| m.name == "tag").and_then(|m| m.value.clone()) {
        let mut plain = String::new();
        if let Some(sheet) = &c.style
            && let Ok(sheet) = brain_css::expand_mixins(sheet, &c.file)
        {
            brain_css::print_items(&sheet.items, &mut plain, 0);
        }
        js.push_str(&format!("define({}, {name}, {});\n", js_str(&tag), js_str(&plain)));
    }
    (js, css, server_js, server_fns, g.i18n_keys.into_iter().collect())
}

/// Scoped CSS of a component: its `<style>` with mixins expanded, inside `@layer <level>` and `@scope` by `hash`.
pub(crate) fn component_css(c: &Component, hash: &str) -> String {
    let Some(sheet) = &c.style else { return String::new() };
    let Ok(sheet) = brain_css::expand_mixins(sheet, &c.file) else { return String::new() };
    let layer = match c.level {
        Level::Micro => "ui",
        Level::Comp => "comp",
        Level::View => "view",
    };
    brain_css::scoped(&sheet, hash, layer)
}

/// One WebMCP tool descriptor: `{ name, description, inputSchema, execute }` (JSON Schema from the `tools` mini-language).
fn tool_js(t: &brain_check::tools::Tool) -> String {
    let props: Vec<String> = t
        .params
        .iter()
        .map(|p| {
            let schema = match p.ty.as_deref().unwrap_or("any") {
                "str" => "{ type: \"string\" }",
                "num" => "{ type: \"number\" }",
                "bool" => "{ type: \"boolean\" }",
                "list" => "{ type: \"array\" }",
                "obj" => "{ type: \"object\" }",
                "email" => "{ type: \"string\", format: \"email\" }",
                "url" => "{ type: \"string\", format: \"uri\" }",
                "date" => "{ type: \"string\", format: \"date\" }",
                _ => "{}",
            };
            format!("{}: {schema}", js_str(&p.name))
        })
        .collect();
    let required: Vec<String> = t.params.iter().filter(|p| p.required).map(|p| js_str(&p.name)).collect();
    format!(
        "{{ name: {}, description: {}, inputSchema: {{ type: \"object\", properties: {{ {} }}, required: [{}] }}, execute: {} }}",
        js_str(&t.name),
        js_str(&t.description),
        props.join(", "),
        required.join(", "),
        t.name
    )
}

/// `titleTemplate` of the nearest `app.brain` above `disk` (the view file).
fn title_template(disk: &Path) -> Option<String> {
    let app = disk.ancestors().skip(1).map(|d| d.join("app.brain")).find(|p| p.exists())?;
    let src = std::fs::read_to_string(&app).ok()?;
    let c = brain_parse::parse(&src, Path::new("app.brain")).ok()?;
    c.meta.iter().find(|m| m.name == "titleTemplate").and_then(|m| m.value.clone())
}

/// `LangSwitch` → `lang-switch` (kit file names).
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
