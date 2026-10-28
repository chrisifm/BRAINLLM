//! INTERNALS §28: per-block content hashes so the dev server and the build redo only what an edit touched.
//!
//! A `.brain` file is split into its `meta`, `script`, `markup` and `style` blocks, each hashed on its exact text, plus
//! a `contract` hash of what importers depend on (props, emits, expose, slots). Comparing two [`Blocks`] gives a
//! [`Change`]: nothing, style only (recompile CSS, hot-swap the sheet), this component only, or its contract (also
//! re-check its direct importers). [`compile_style`] is the style-only fast path.

use crate::{component_hash_len, Options};
use brain_parse::ast::*;
use brain_parse::Diag;
use std::path::Path;

/// Version of the compiler: package version + a fingerprint of the compiler sources (build.rs), part of every cache key, so any compiler change invalidates the cache.
pub const COMPILER_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("BRAIN_COMPILER_FP"));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocks {
    /// The `<meta …>` tags, in order.
    pub meta: String,
    /// The bodies of `<script>` elements (a `.jsbrain` file is all script).
    pub script: String,
    /// Everything else.
    pub markup: String,
    /// The body of `<style>` (a `.cssbrain` file is all style).
    pub style: String,
    /// What importers depend on: props (name, type, optional, default, `~`), emits, expose and slot names.
    /// Empty when the file does not parse.
    pub contract: String,
}

/// What an edit requires, from cheapest to most expensive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// Same blocks: nothing to do.
    None,
    /// Only `<style>` changed: [`compile_style`] and swap the stylesheet.
    Style,
    /// Script, markup or non-contract meta changed: recompile this file only.
    Component,
    /// Props, emits, expose or slots changed: recompile this file and re-check its direct importers.
    Contract,
}

/// 16 hex chars of FNV-1a 64 over `parts`, separated so that block boundaries are part of the key.
pub fn hash(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in p.bytes().chain([0xff]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("{h:016x}")
}

/// Splits `src` (the file at `path`) into hashed blocks. Cheap: a text scan plus, for the contract, a parse.
pub fn blocks(src: &str, path: &Path) -> Blocks {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let (meta, script, markup, style) = match ext {
        "jsbrain" => (String::new(), src.to_string(), String::new(), String::new()),
        "cssbrain" => (String::new(), String::new(), String::new(), src.to_string()),
        _ => split(src),
    };
    let contract = brain_parse::parse(src, path).map(|c| contract_of(&c)).unwrap_or_default();
    Blocks { meta: hash(&[&meta]), script: hash(&[&script]), markup: hash(&[&markup]), style: hash(&[&style]), contract: hash(&[&contract]) }
}

/// The cheapest [`Change`] that turns `old` into `new`.
pub fn change(old: &Blocks, new: &Blocks) -> Change {
    if old == new {
        Change::None
    } else if old.contract != new.contract {
        Change::Contract
    } else if old.meta == new.meta && old.script == new.script && old.markup == new.markup {
        Change::Style
    } else {
        Change::Component
    }
}

/// Cache key of a full compile: source blocks + compiler version + every option that changes the output.
pub fn cache_key(b: &Blocks, path: &Path, opts: &Options) -> String {
    hash(&[COMPILER_VERSION, &path.to_string_lossy(), &options_key(opts), &b.meta, &b.script, &b.markup, &b.style])
}

/// Persistent lookup needs the exact source, not an AST or a public-contract parse. Block classification remains
/// available for HMR, but must not make every warm cache hit parse the component again.
pub fn source_cache_key(src: &str, path: &Path, opts: &Options) -> String {
    hash(&[COMPILER_VERSION, "source-v1", &path.to_string_lossy(), &options_key(opts), src])
}

fn options_key(opts: &Options) -> String {
    // PUBLIC_* values enter as a hash: the cache key (a file name) never holds a value (§36).
    // Keep field boundaries: ("A", "B=C") must differ from ("A=B", "C"). Preserve order too, since duplicate
    // keys can be interpreted in order by codegen. Extra misses are safer than output from different options.
    let env_refs: Vec<&str> = opts.public_env.iter().flat_map(|(k, v)| [k.as_str(), v.as_str()]).collect();
    // The locale being built and its texts (static `t()` is resolved into the output, §37).
    let i18n = opts.i18n.as_ref().map(|(loc, m)| {
        let refs: Vec<&str> = m.iter().flat_map(|(k, v)| [k.as_str(), v.as_str()]).collect();
        hash(&[loc, &hash(&refs)])
    });
    let lint: Vec<&str> = opts.lint.iter().flat_map(|(code, level)| [code.as_str(), level.map_or("off", |l| l.as_str())]).collect();
    hash(&[if opts.dev { "dev" } else { "prod" }, if opts.adopt { "adopt" } else { "create" },
        &opts.runtime, &opts.runtime_dev, &opts.runtime_wc, &opts.import_suffix, &opts.hash_len.to_string(),
        &hash(&env_refs), &i18n.unwrap_or_default(), &hash(&lint),
        &opts.root.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()])
}

/// Style-only fast path: the scoped CSS of `src`, exactly as [`crate::compile_with`] would emit it.
/// Errors are the style diagnostics (`B03xx`, color contrast `B0408`) of the whole check.
pub fn compile_style(src: &str, path: &Path, opts: &Options) -> Result<String, Vec<Diag>> {
    let c = brain_parse::parse(src, path)?;
    let disk = match &opts.root {
        Some(r) => r.join(path),
        None => path.to_path_buf(),
    };
    let (ctx, _) = brain_check::load_ctx(&disk, &c);
    let errors: Vec<Diag> =
        brain_check::check(&c, &ctx).into_iter().filter(|d| !d.is_warning() && (d.code.starts_with("B03") || d.code == "B0408")).collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(crate::component_css(&c, &component_hash_len(&c.file, opts.hash_len)))
}

/// `src` with the content of `raw` elements blanked (same length), so block tags inside code samples are ignored.
fn mask_raw(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if src.as_bytes()[i] == b'<'
            && let Some(len) = brain_parse::raw_element_len(&src[i..])
        {
            out.extend(src[i..i + len].chars().map(|c| if c == '\n' { '\n' } else { ' ' }).flat_map(|c| std::iter::repeat_n(c, 1)));
            // Keep byte offsets: pad multi-byte characters.
            let produced: usize = src[i..i + len].chars().map(|_| 1).sum();
            out.extend(std::iter::repeat_n(' ', len - produced));
            i += len;
            continue;
        }
        let ch = src[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn split(src: &str) -> (String, String, String, String) {
    let (mut meta, mut script, mut markup, mut style) = (String::new(), String::new(), String::new(), String::new());
    let masked = mask_raw(src);
    let mut rest = src;
    loop {
        let base = src.len() - rest.len();
        let rest_masked = &masked[base..];
        let next = [("<meta", 0), ("<script", 1), ("<style", 2)]
            .iter()
            .filter_map(|(tag, k)| find_tag(rest_masked, tag).map(|i| (i, *tag, *k)))
            .min_by_key(|(i, _, _)| *i);
        let Some((i, tag, kind)) = next else {
            markup.push_str(rest);
            break;
        };
        markup.push_str(&rest[..i]);
        rest = &rest[i..];
        if kind == 0 {
            let end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
            meta.push_str(&rest[..end]);
            meta.push('\n');
            rest = &rest[end..];
            continue;
        }
        let close = if kind == 1 { "</script>" } else { "</style>" };
        let open_end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
        let body_end = rest[open_end..].find(close).map(|e| open_end + e).unwrap_or(rest.len());
        let out = if kind == 1 { &mut script } else { &mut style };
        // The opening tag (attributes such as `lang` or `global`) belongs to the block.
        out.push_str(&rest[..body_end]);
        let _ = tag;
        rest = &rest[(body_end + close.len()).min(rest.len())..];
    }
    (meta, script, markup, style)
}

/// Byte offset of `<tag` followed by whitespace, `>` or `/`.
fn find_tag(s: &str, tag: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = s[from..].find(tag) {
        let at = from + i;
        match s[at + tag.len()..].chars().next() {
            Some(c) if c.is_whitespace() || c == '>' || c == '/' => return Some(at),
            None => return None,
            _ => from = at + tag.len(),
        }
    }
    None
}

/// Canonical text of what importers see.
fn contract_of(c: &Component) -> String {
    let mut s = String::new();
    for p in &c.props {
        s.push_str(&format!("p {}:{:?}{}{}={};", p.name, p.ty, if p.optional { "?" } else { "" }, if p.bindable { "~" } else { "" }, p.default.as_ref().map(dump_expr).unwrap_or_default()));
    }
    for m in &c.meta {
        if matches!(m.name.as_str(), "emits" | "expose" | "export" | "tag") {
            s.push_str(&format!("m {}={};", m.name, m.value.as_deref().unwrap_or("")));
        }
    }
    // Modules: the exported names (and their declared form: `key(…)`, `api(…)`, function arity).
    for st in &c.script {
        match st {
            Stmt::Export { decl, .. } => match decl.as_ref() {
                Stmt::Var(v) => {
                    for d in &v.decls {
                        let init = d.init.as_ref().map(|e| match e.unparen() {
                            Expr::Call { callee, .. } => format!("call {}", brain_parse::dump::expr(callee)),
                            Expr::Func(f) => format!("fn/{}", f.params.len()),
                            _ => String::new(),
                        });
                        s.push_str(&format!("x {}:{};", brain_parse::dump::pat(&d.pat), init.unwrap_or_default()));
                    }
                }
                Stmt::Func(f) => s.push_str(&format!("x {}:fn/{};", f.name.as_ref().map(|n| n.name.as_str()).unwrap_or(""), f.params.len())),
                _ => {}
            },
            Stmt::ExportDefault { .. } => s.push_str("x default;"),
            Stmt::Server { func, .. } => s.push_str(&format!("r {}/{};", func.name.as_ref().map(|n| n.name.as_str()).unwrap_or(""), func.params.len())),
            _ => {}
        }
    }
    let mut slots = Vec::new();
    slot_names(&c.template, &mut slots);
    for n in slots {
        s.push_str(&format!("s {n};"));
    }
    s
}

fn dump_expr(e: &Expr) -> String {
    // Spans differ when only whitespace moves; the printed form does not.
    brain_parse::dump::expr(e)
}

fn slot_names(nodes: &[Node], out: &mut Vec<String>) {
    for n in nodes {
        match n {
            Node::Slot(s) => out.push(s.name.clone().unwrap_or_default()),
            Node::Element(e) => slot_names(&e.children, out),
            Node::ComponentRef(r) => slot_names(&r.children, out),
            Node::If(b) => {
                for br in &b.branches {
                    slot_names(&br.body, out);
                }
                if let Some(o) = &b.otherwise {
                    slot_names(o, out);
                }
            }
            Node::Each(b) => slot_names(&b.body, out),
            Node::SlotScope(b) => slot_names(&b.body, out),
            _ => {}
        }
    }
}
