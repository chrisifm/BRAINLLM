//! `brain new` and `brain add`: templates that follow SPEC §1-§3.

use crate::names::{kebab, valid_ident};
use std::fs;
use std::path::{Path, PathBuf};

const TYPES: &[&str] = &["str", "num", "bool", "list", "obj", "fn", "any"];

pub fn new_app(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    new_app_named(dir, &name)
}

/// Creates the app exactly in `dir` (which may exist but must be empty); `name` only names it (title, APP.md).
pub fn new_app_named(dir: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let name = name.to_string();
    if !valid_ident(&name) {
        return Err(format!("invalid app name `{name}`; fix: use letters, digits and hyphens, starting with a letter"));
    }
    // hidden files (.git, .claude) do not make a folder "taken": an agent often starts in an initialized repo
    if dir.exists() && fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().any(|e| !e.file_name().to_string_lossy().starts_with('.')) {
        return Err(format!("`{}` already exists and is not empty; fix: pick another folder or delete its contents", dir.display()));
    }
    // a random dev-only session key (BRAIN_SECRET in [development]; production gets its own from the host)
    let secret: String = {
        let mut b = [0u8; 24];
        if let Ok(mut f) = fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = f.read_exact(&mut b);
        }
        b.iter().map(|x| format!("{x:02x}")).collect()
    };
    let fill = |t: &str| t.replace("{{name}}", &name).replace("{{secret}}", &secret);
    let text: Vec<(&str, String)> = vec![
        ("app.brain", fill(include_str!("templates/new/app.brain"))),
        (TOKENS_FILE, TOKENS_CSS.into()),
        ("brain.json", fill(include_str!("templates/new/brain.json"))),
        (".brain_env", fill(include_str!("templates/new/brain_env"))),
        (".brain_env.example", include_str!("templates/new/brain_env.example").into()),
        (".gitignore", "dist/\n.brain/\n.brain_env\nnode_modules/\n.DS_Store\n".into()),
        // the brain version this app was made with, pinned exactly: a global `brain` delegates to it (INTERNALS §23)
        ("package.json", PACKAGE_JSON.replace("{{name}}", &name.to_ascii_lowercase()).replace("{{version}}", env!("CARGO_PKG_VERSION"))),
        ("APP.md", APP_MD.replace("{{name}}", &name)),
        ("AGENTS.md", AGENTS_MD.into()),
        ("views/home.brain", HOME.into()),
        ("views/about.brain", fill(include_str!("templates/new/about.brain"))),
        ("views/404.brain", include_str!("templates/new/404.brain").into()),
        ("views/_offline.brain", include_str!("templates/new/_offline.brain").into()),
        ("views/_layout.brain", include_str!("templates/new/views/_layout.brain").into()),
        ("views/_loading.brain", include_str!("templates/new/views/_loading.brain").into()),
        ("views/_error.brain", include_str!("templates/new/views/_error.brain").into()),
        ("views/403.brain", include_str!("templates/new/views/403.brain").into()),
        ("views/posts/index.brain", include_str!("templates/new/views/posts/index.brain").into()),
        ("views/posts/[id].brain", include_str!("templates/new/views/posts/[id].brain").into()),
        ("views/account/_guard.jsbrain", include_str!("templates/new/views/account/_guard.jsbrain").into()),
        ("views/account/index.brain", include_str!("templates/new/views/account/index.brain").into()),
        ("views/login.brain", include_str!("templates/new/views/login.brain").into()),
        ("views/login.test.brain", include_str!("templates/new/views/login.test.brain").into()),
        ("lib/posts.jsbrain", include_str!("templates/new/lib/posts.jsbrain").into()),
        ("channels/session.jsbrain", include_str!("templates/new/channels/session.jsbrain").into()),
        ("server/auth.jsbrain", include_str!("templates/new/server/auth.jsbrain").into()),
        ("comp/counter.brain", COUNTER.into()),
        ("comp/counter.test.brain", include_str!("templates/new/counter.test.brain").into()),
        ("ui/badge.brain", BADGE.into()),
        ("public/favicon.svg", crate::icons::FAVICON_SVG.into()),
    ];
    // folders without code yet hold only a README: what goes there and the command that creates it (§29)
    let readmes = FOLDERS.iter().map(|(d, body)| (format!("{d}/README.md"), body.to_string()));
    let mut written = Vec::new();
    for (rel, body) in text.into_iter().map(|(a, b)| (a.to_string(), b)).chain(readmes) {
        let p = dir.join(&rel);
        fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&p, body).map_err(|e| e.to_string())?;
        written.push(p);
    }
    for (rel, bytes) in [("public/icon-192.png", crate::icons::icon(192)), ("public/icon-512.png", crate::icons::icon(512)), ("public/og.png", crate::icons::og())] {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&p, bytes).map_err(|e| e.to_string())?;
        written.push(p);
    }
    Ok(written)
}

/// Folders of the layout that start empty (INTERNALS §29): one README each.
const FOLDERS: &[(&str, &str)] = &[
    ("channels", "# channels/\n\nState shared between components: context keys, stores and buses (one `.jsbrain` each). Create one with `brain add channel <name>`.\n"),
    ("services", "# services/\n\nClient-side services that talk to the outside world (an API client, storage, analytics), provided to components through a channel. Create one with `brain add service <name>`.\n"),
    ("api", "# api/\n\nTyped API contracts: `api('/path', { get: { out: '[Post]' } })` with the request and response shapes. Create one with `brain add api <name>`.\n"),
    ("lib", "# lib/\n\nPure helpers shared by views and components (formatting, parsing, math): one export per `.jsbrain`. Create one with `brain add lib <name>`.\n"),
    ("server", "# server/\n\nServer-only modules (`<meta server>`): database access, secrets, auth. They never reach the browser. Create one with `brain add server <name>`.\n"),
    ("content", "# content/\n\nMarkdown collections with frontmatter (a blog, docs, a changelog), one folder per collection. Create one with `brain add collection <name>`.\n"),
];

/// Tokens file that marks the root of an app (SPEC §10).
pub const TOKENS_FILE: &str = "tokens.cssbrain";
/// Previous name; accepted while examples/ and brain-check migrate.
const TOKENS_LEGACY: &str = "tokens.brain.css";

/// An app root has its tokens file or app.brain (an app without tokens yet is still an app).
pub fn is_app_dir(d: &Path) -> bool {
    d.join(TOKENS_FILE).is_file() || d.join(TOKENS_LEGACY).is_file() || d.join("app.brain").is_file()
}

/// App root: the nearest ancestor with `tokens.cssbrain`.
pub fn find_root(from: &Path) -> Option<PathBuf> {
    from.ancestors().find(|d| is_app_dir(d)).map(Path::to_path_buf)
}

/// HIGHEST ancestor with `tokens.cssbrain`: base of `Diag.file` when apps are nested (e.g. examples/invalid).
pub fn outer_root(from: &Path) -> Option<PathBuf> {
    from.ancestors().filter(|d| is_app_dir(d)).last().map(Path::to_path_buf)
}

/// Options shared by the CLI and MCP generators. Flags outside their kind are rejected.
#[derive(Default)]
pub struct AddOptions {
    pub props: String,
    pub store: Option<String>,
    pub bus: bool,
    pub key: bool,
    pub get: Option<String>,
    pub post: Option<String>,
    pub fields: Option<String>,
}

pub fn add_with_options(root: &Path, kind: &str, name: &str, o: &AddOptions) -> Result<PathBuf, String> {
    if (o.store.is_some() || o.bus || o.key) && kind != "channel"
        || (o.get.is_some() || o.post.is_some()) && kind != "api"
        || o.fields.is_some() && kind != "type"
        || !o.props.is_empty() && !matches!(kind, "view" | "comp" | "ui" | "micro")
    { return Err("options do not match the file kind; fix: use channel store/bus/key, api get/post, type fields or component props".into()); }
    if !matches!(kind, "channel" | "service" | "api" | "type" | "guard" | "server") {
        return add(root, kind, name, &o.props);
    }
    if !valid_ident(name) { return Err("invalid name; fix: letters, digits, underscores or hyphens, starting with a letter".into()); }
    let stem = kebab(name);
    let (rel, body) = match kind {
        "channel" => {
            if usize::from(o.store.is_some()) + usize::from(o.bus) + usize::from(o.key) > 1 { return Err("choose only one of --store, --bus or --key".into()); }
            let expr = if let Some(value) = &o.store {
                validate_store(value)?;
                format!("store({value})")
            } else if o.bus { format!("bus('{}')", stem) } else { format!("key('{}')", stem) };
            (format!("channels/{stem}.jsbrain"), format!("export default {expr}\n"))
        }
        "service" => (format!("services/{stem}.jsbrain"), format!("// Import this key, provide its implementation in app.brain, then use it in components.\nexport default key('{stem}')\n")),
        "server" => (format!("server/{stem}.jsbrain"), "<meta server>\n// Server-only helpers. Add functions to this object.\nexport default {}\n".into()),
        "guard" => (format!("views/{stem}/_guard.jsbrain"), "// Deny anonymous access; adapt this condition to the route's authorization rules.\nexport default (ctx) => (ctx.user ? true : deny('Sign in to continue'))\n".into()),
        "api" => {
            if o.get.is_none() && o.post.is_none() { return Err("api needs --get <type> or --post <input>:<output>".into()); }
            let mut methods = Vec::new();
            if let Some(out) = &o.get { validate_shape(root, out)?; methods.push(format!("get: {{ out: '{}' }}", out)); }
            if let Some(post) = &o.post {
                let (input, out) = post.split_once(':').ok_or("--post needs input:output, e.g. User:User")?;
                validate_shape(root, input)?; validate_shape(root, out)?;
                methods.push(format!("post: {{ in: '{input}', out: '{out}' }}"));
            }
            (format!("api/{stem}.jsbrain"), format!("export default api('/{stem}', {{ {} }})\n", methods.join(", ")))
        }
        "type" => {
            if !name.starts_with(|c: char| c.is_ascii_uppercase()) || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("type names must be PascalCase identifiers, e.g. User".into());
            }
            let fields = o.fields.as_ref().filter(|s| !s.trim().is_empty()).ok_or("type needs --fields, e.g. id:num! email:email!")?;
            let mut seen = std::collections::HashSet::new();
            let mut pairs = Vec::new();
            for field in fields.split_whitespace() {
                let (key, shape) = field.split_once(':').ok_or("fields must be name:type pairs")?;
                if !valid_ident(key) || key.contains('-') || !seen.insert(key) || matches!(key, "__proto__" | "constructor" | "prototype") { return Err("invalid or duplicate field name".into()); }
                validate_shape(root, shape)?;
                pairs.push(format!("{key}: '{shape}'"));
            }
            ("types.jsbrain".into(), format!("export const {name} = {{ {} }}\n", pairs.join(", ")))
        }
        _ => unreachable!(),
    };
    let path = root.join(&rel);
    // Refuse directory/file symlinks before reading or writing; the new-file operation is exclusive.
    let mut cursor = root.to_path_buf();
    for part in Path::new(&rel).components() {
        cursor.push(part);
        if fs::symlink_metadata(&cursor).is_ok_and(|m| m.file_type().is_symlink()) { return Err("generator paths cannot cross symlinks".into()); }
    }
    if kind == "type" && path.exists() {
        return append_type(root, &path, name, &body);
    }
    validate_generated(root, &path, &body)?;
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("cannot create {rel}: {e}; fix: choose a new name"))?;
    file.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

fn validate_generated(_root: &Path, path: &Path, body: &str) -> Result<(), String> {
    let c = brain_parse::parse(body, path).map_err(|d| format!("invalid generated source: {d:?}"))?;
    let (ctx, _) = brain_check::load_ctx(path, &c);
    let errors: Vec<_> = brain_check::check(&c, &ctx).into_iter().filter(|d| !d.is_warning() && !d.is_info()).collect();
    if errors.is_empty() { Ok(()) } else { Err(format!("generated file does not pass check: {errors:?}")) }
}

fn validate_shape(root: &Path, shape: &str) -> Result<(), String> {
    // Supported schema tokens only; quotes, code and additional declarations can never enter a template.
    let plain = shape.strip_suffix('!').unwrap_or(shape);
    let plain = plain.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(plain);
    let valid = matches!(plain, "str" | "num" | "bool" | "email" | "url" | "date" | "obj" | "list" | "any")
        || brain_check::types::exported(root).is_some_and(|names| names.contains(plain));
    if !valid { return Err(format!("unsupported or undeclared schema `{shape}`; fix: use a primitive or an existing types.jsbrain export")); }
    Ok(())
}

fn validate_store(value: &str) -> Result<(), String> {
    use brain_parse::ast::{Expr, ObjProp, Stmt};
    fn safe_key(key: &brain_parse::ast::PropKey) -> bool {
        use brain_parse::ast::PropKey;
        match key {
            PropKey::Ident(i) => !matches!(i.name.as_str(), "__proto__" | "constructor" | "prototype"),
            PropKey::Str { value, .. } => !matches!(value.as_str(), "__proto__" | "constructor" | "prototype"),
            PropKey::Num { .. } => true,
            PropKey::Computed(_) => false,
        }
    }
    fn literal(e: &Expr) -> bool {
        match e {
            Expr::Str { .. } | Expr::Num { .. } | Expr::Bool { .. } | Expr::Null(_) => true,
            Expr::Unary { op: "-", arg, .. } => matches!(arg.as_ref(), Expr::Num { .. }),
            Expr::Array { elems, .. } => elems.iter().all(|e| e.as_ref().is_some_and(literal)),
            Expr::Object { props, .. } => props.iter().all(|p| matches!(p, ObjProp::KeyValue { key, value, .. } if safe_key(key) && literal(value))),
            _ => false,
        }
    }
    if value.len() > 65536 { return Err("store initializer exceeds 64 KiB".into()); }
    let c = brain_parse::parse(&format!("export default {value}"), Path::new("channels/validate.jsbrain")).map_err(|_| "--store must be a literal object")?;
    match c.script.as_slice() {
        [Stmt::ExportDefault { expr: Expr::Object { .. }, .. }] if matches!(&c.script[0], Stmt::ExportDefault { expr, .. } if literal(expr)) => Ok(()),
        _ => Err("--store must be a literal object, with no functions, references or extra statements".into()),
    }
}

fn append_type(root: &Path, path: &Path, name: &str, body: &str) -> Result<PathBuf, String> {
    use std::io::{Read, Write};
    // Kernel-owned lock releases even if the generating agent is killed; no stale sidecar can block future work.
    let mut file = fs::OpenOptions::new().read(true).append(true).open(path).map_err(|e| e.to_string())?;
    file.try_lock().map_err(|_| "another type generator is active; retry after it finishes")?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 1024 * 1024 { return Err("types.jsbrain exceeds generator limit".into()); }
    let mut previous = String::new();
    file.read_to_string(&mut previous).map_err(|e| e.to_string())?;
    let parsed = brain_parse::parse(&previous, path).map_err(|_| "fix existing types.jsbrain before adding another type")?;
    if brain_check::scope::top_bindings(&parsed).contains_key(name) { return Err(format!("type `{name}` already exists")); }
    let next = format!("{previous}\n{body}");
    validate_generated(root, path, &next)?;
    file.write_all(format!("\n{body}").as_bytes()).map_err(|e| e.to_string())?;
    Ok(path.to_path_buf())
}

pub fn add(root: &Path, kind: &str, name: &str, props: &str) -> Result<PathBuf, String> {
    match kind {
        "test" => return add_test(root, name),
        "layout" => return add_layout(root, name),
        "lib" => return add_lib(root, name),
        "collection" => return add_collection(root, name),
        "locale" => return add_locale(root, name),
        _ => {}
    }
    let dir = match kind {
        "view" => "views",
        "comp" => "comp",
        "ui" | "micro" => "ui",
        _ => return Err(format!("unknown kind `{kind}`; fix: use view, comp, ui, layout, lib, collection, locale or test")),
    };
    // views take a route path (`about`, `users/[id]`, `404`, `blog/index`); components a single name
    let stem = if kind == "view" {
        view_path(name)?
    } else if valid_ident(name) {
        kebab(name)
    } else {
        return Err(format!("invalid name `{name}`; fix: letters, digits and hyphens, starting with a letter"));
    };
    let props = validate_props(props)?;
    let rel = format!("{dir}/{stem}.brain");
    let path = root.join(&rel);
    if path.exists() {
        return Err(format!("`{}` already exists; fix: pick another name or edit the file", path.display()));
    }
    // the same name the compiler gives the component (`views/users/[id].brain` → UsersId)
    let comp = brain_parse::component_name(Path::new(&rel));
    let class = kebab(&comp);
    let meta = if props.is_empty() { String::new() } else { format!("<meta props=\"{props}\">\n\n") };
    // every declared prop is rendered once (a placeholder to replace): a generated file passes check with no B1005
    let names: Vec<&str> = props.split_whitespace().filter_map(|p| p.split(':').next()).filter(|n| !n.is_empty()).collect();
    let uses = if names.is_empty() { String::new() } else { format!("  <p>{}</p>\n", names.iter().map(|n| format!("{{{n}}}")).collect::<Vec<_>>().join(" · ")) };
    let route = crate::map::route_of(&rel);
    let body = match kind {
        "view" => view_body(&meta, &uses, &class, &comp, route.as_deref(), &stem),
        "comp" => format!(
            "{meta}<script>\n  let open = sig(false)\n  let query = sig('')\n</script>\n\n<section class=\"{class}\">\n  <button type=\"button\" on:click={{open = !open}}>{comp}</button>\n  <label>Search <input bind:value={{query}} placeholder=\"Type to filter\"></label>\n{uses}  {{#if open}}<slot />{{/if}}\n</section>\n\n<style>\n  .{class} {{ display: grid; gap: var(--space-2); }}\n</style>\n"
        ),
        _ => format!("{meta}<span class=\"{class}\">{}<slot /></span>\n\n<style>\n  .{class} {{ display: inline-flex; }}\n</style>\n", names.iter().map(|n| format!("{{{n}}} ")).collect::<String>()),
    };
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&path, body).map_err(|e| e.to_string())?;
    // a component or static view is born with its colocated test (SPEC: `*.test.brain` next to it); a dynamic view
    // needs its route's props, layouts only wrap a slot and lib/ has no markup, so those are left to `brain add test`
    if kind != "view" || route.as_deref().is_some_and(|r| !r.contains(':') && r != "*") {
        write_test(root, &path)?;
    }
    // static routes go into the main nav (dynamic ones are reached from a list; the 404 from anywhere)
    if let Some(r) = route.filter(|r| !r.contains(':') && r != "*" && r != "/") {
        let label = r.rsplit('/').next().filter(|x| !x.is_empty()).map(crate::names::pascal).unwrap_or_else(|| comp.clone());
        link_in_nav(root, &r, &label);
    }
    Ok(path)
}

/// `Users/[id]` → `users/[id]`; each segment is a kebab name, a number (`404`) or a `[param]`.
fn view_path(name: &str) -> Result<String, String> {
    let segs: Vec<String> = name.trim_matches('/').split('/').map(|s| if s.starts_with('[') { s.to_string() } else { kebab(s) }).collect();
    let ok = |s: &str| match s.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        Some(p) => valid_ident(p.trim_start_matches("...")),
        None => !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !s.starts_with('-'),
    };
    match segs.iter().find(|s| !ok(s)) {
        Some(bad) => Err(format!("invalid view path segment `{bad}` in `{name}`; fix: lowercase words, numbers or a `[param]`, e.g. `users/[id]`")),
        None => Ok(segs.join("/")),
    }
}

fn view_body(meta: &str, uses: &str, class: &str, comp: &str, route: Option<&str>, stem: &str) -> String {
    let style = format!("\n<style>\n  .{class} {{ padding: var(--space-4); }}\n</style>\n");
    match route {
        Some("*") => format!(
            "{meta}<meta seo title=\"Page not found\" description=\"The page you are looking for does not exist or was moved. Go back to the home page.\" robots=\"noindex\">\n\n<main class=\"{class}\">\n  <h1>Page not found</h1>\n  <p><a href=\"/\">Back to the home page</a></p>\n{uses}</main>\n{style}"
        ),
        Some(r) if r.contains(':') => {
            // dynamic route: load() receives the params; with paths() it is prerendered, without it served by SSR
            let params: Vec<&str> = r.split('/').filter_map(|s| s.strip_prefix(':')).collect();
            let fields = params.iter().map(|p| format!("{p}: ctx.params.{p}")).collect::<Vec<_>>().join(", ");
            let shown = params.iter().map(|p| format!("{{props.{p}}}")).collect::<Vec<_>>().join(" · ");
            format!(
                "{meta}<meta seo title=\"{comp}\" description=\"TODO: describe this page in one sentence (70 to 155 characters).\">\n\n<script>\n  export async function load(ctx) {{\n    return {{ {fields} }}\n  }}\n</script>\n\n<main class=\"{class}\">\n  <h1>{comp} {shown}</h1>\n{uses}</main>\n{style}"
            )
        }
        Some(r) => {
            let title = crate::names::pascal(r.rsplit('/').next().filter(|x| !x.is_empty()).unwrap_or(stem));
            format!(
                "{meta}<meta seo title=\"{title}\" description=\"TODO: describe this page in one sentence (70 to 155 characters).\" canonical=\"{r}\">\n\n<main class=\"{class}\">\n  <h1>{title}</h1>\n  <p>TODO: write this page and link related pages, e.g. the <a href=\"/\">home page</a>.</p>\n{uses}</main>\n{style}"
            )
        }
        None => format!("{meta}<main class=\"{class}\">\n  <h1>{comp}</h1>\n{uses}</main>\n{style}"),
    }
}

/// `brain add layout <folder>` → `views/<folder>/_layout.brain` (`.` = the root of views/): wraps every view below.
fn add_layout(root: &Path, dir: &str) -> Result<PathBuf, String> {
    let dir = dir.trim_matches('/');
    let rel = if dir.is_empty() || dir == "." { "views/_layout.brain".to_string() } else { format!("views/{}/_layout.brain", view_path(dir)?) };
    let path = root.join(&rel);
    if path.exists() {
        return Err(format!("`{rel}` already exists; fix: edit it"));
    }
    let class = kebab(&brain_parse::component_name(Path::new(&rel))).trim_start_matches('-').to_string();
    let class = if class.is_empty() { "layout".to_string() } else { class };
    let body = format!("<section class=\"{class}\">\n  <slot />\n</section>\n\n<style>\n  .{class} {{ display: grid; gap: var(--space-4); }}\n</style>\n");
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path)
}

/// `brain add locale <code>` (§37): declares the language in brain.json (creating the `i18n` block with `en` as the
/// default when there is none) and writes `i18n/<code>.json` with the default locale's keys and texts, so the site
/// works at once and B1402 lists every text still to translate.
fn add_locale(root: &Path, code: &str) -> Result<PathBuf, String> {
    if !brain_check::i18n::valid_locale(code) {
        return Err(format!("`{code}` is not a language tag; fix: e.g. es, pt-BR, ar"));
    }
    let bj = root.join("brain.json");
    let src = fs::read_to_string(&bj).unwrap_or_else(|_| "{}\n".into());
    let (default, locales) = brain_pack::read_i18n(root).unwrap_or_else(|| ("en".into(), vec!["en".into()]));
    if locales.iter().any(|l| l == code) {
        return Err(format!("`{code}` is already in brain.json i18n.locales; fix: translate i18n/{code}.json"));
    }
    let list = locales.iter().chain(std::iter::once(&code.to_string())).map(|l| format!("\"{l}\"")).collect::<Vec<_>>().join(", ");
    let block = format!("\"i18n\": {{ \"default\": \"{default}\", \"locales\": [{list}] }}");
    let out = match src.find("\"i18n\"") {
        // replace the existing block (from its key to the end of its object)
        Some(i) => {
            let open = i + src[i..].find('{').ok_or("brain.json: `i18n` is not an object")?;
            let mut depth = 0;
            let mut end = open;
            for (k, ch) in src[open..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + k + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            format!("{}{block}{}", &src[..i], &src[end..])
        }
        None => match src.find('{') {
            Some(o) => {
                let rest = src[o + 1..].trim_start();
                let comma = if rest.starts_with('}') { "" } else { "," };
                format!("{}{{\n  {block}{comma}{}", &src[..o], &src[o + 1..])
            }
            None => format!("{{\n  {block}\n}}\n"),
        },
    };
    fs::write(&bj, out).map_err(|e| e.to_string())?;
    let dir = root.join("i18n");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let def_file = dir.join(format!("{default}.json"));
    if !def_file.exists() {
        fs::write(&def_file, "{\n}\n").map_err(|e| e.to_string())?;
    }
    let path = dir.join(format!("{code}.json"));
    let fresh = !path.exists();
    // the first extra language: the fixed text of the templates moves to keys (B1403's edits: `{t('key')}` in the
    // markup, the text in the default file), so the app stays clean; the new file then starts as a copy to translate
    if locales.len() == 1 {
        crate::run_fix_only(Some(root.to_path_buf()), Some(&["B1403"]), &mut |_, _| {})?;
    }
    if fresh || fs::read_to_string(&path).is_ok_and(|t| t.trim() == "{}" || t.trim() == "{\n}") {
        fs::write(&path, fs::read_to_string(&def_file).unwrap_or_else(|_| "{\n}\n".into())).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// `brain add collection <name>` (INTERNALS §33): `content/<name>/hello-world.md` plus the four views that read it
/// through `brain:content` — the listing (`/<name>`), its pages (`/<name>/page/<n>`), each entry (`/<name>/<slug>`) and
/// each tag (`/<name>/tag/<tag>`) — all prerendered with `paths()`, and a nav link. Passes SEO's minimum untouched.
fn add_collection(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_ident(name) {
        return Err(format!("invalid name `{name}`; fix: letters, digits and hyphens, starting with a letter"));
    }
    let name = kebab(name);
    let title = {
        let mut c = name.replace('-', " ").chars().collect::<Vec<_>>();
        if let Some(f) = c.first_mut() {
            *f = f.to_ascii_uppercase();
        }
        c.into_iter().collect::<String>()
    };
    // the entry's author: the first one declared in brain.json (its BlogPosting then has `author`); none → no line
    let author = fs::read_to_string(root.join("brain.json"))
        .ok()
        .and_then(|t| brain_mcp::Json::parse(&t).ok())
        .and_then(|j| match j.get("authors") {
            Some(brain_mcp::Json::Obj(kv)) => kv.first().map(|(n, _)| format!("author: {n}\n")),
            _ => None,
        })
        .unwrap_or_default();
    let files: Vec<(String, &str)> = vec![
        (format!("content/{name}/hello-world.md"), COLLECTION_MD),
        (format!("views/{name}/index.brain"), COLLECTION_INDEX),
        (format!("views/{name}/page/[n].brain"), COLLECTION_PAGE),
        (format!("views/{name}/[slug].brain"), COLLECTION_ENTRY),
        (format!("views/{name}/tag/[tag].brain"), COLLECTION_TAG),
    ];
    if let Some((f, _)) = files.iter().find(|(f, _)| root.join(f).exists()) {
        return Err(format!("`{f}` already exists; fix: pick another collection name"));
    }
    for (f, body) in &files {
        let p = root.join(f);
        fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&p, body.replace("{{author}}", &author).replace("{{name}}", &name).replace("{{Title}}", &title)).map_err(|e| e.to_string())?;
    }
    link_in_nav(root, &format!("/{name}"), &title);
    Ok(root.join(&files[1].0))
}

const COLLECTION_MD: &str = r#"---
title: First entry of {{Title}}
description: The first entry of {{Title}}. Write in Markdown below the frontmatter; the page, its feed item and its sitemap date come from this file.
date: 2026-10-13
tags: [welcome]
{{author}}---
This entry lives in `content/{{name}}/hello-world.md`. Add more `.md` files next to it: each one becomes a page at
`/{{name}}/<file name>`, joins the listing and the tag pages, and appears in the RSS and Atom feeds.

## Frontmatter

`title` and `description` are required. `date`, `updated`, `tags`, `cover`, `author` and `draft: true` are optional.
"#;

const COLLECTION_INDEX: &str = r#"<meta seo title="{{Title}}" description="Every entry of {{Title}}, newest first, with a short summary, its date and its tags. Each title opens the full entry.">

<script>
  import { page } from 'brain:content'
  export async function load() {
    return page('{{name}}', 1)
  }
</script>

<main class="page">
  <h1>{{Title}}</h1>
  <ul class="entries">
    {#each props.entries as e (e.slug)}
      <li>
        <h2><a href={e.url}>{e.title}</a></h2>
        <p>{e.description}</p>
        {#if e.date}<time datetime={e.date}>{e.date}</time>{/if}
      </li>
    {/each}
  </ul>
  {#if props.next}<p><a href={props.next} rel="next">Older entries</a></p>{/if}
</main>

<style>
  .entries { display: grid; gap: var(--space-5); padding: 0; list-style: none; }
  h2 { margin: 0; font-size: 1.25rem; }
  time { color: var(--muted); font-size: .875rem; }
</style>
"#;

const COLLECTION_PAGE: &str = r#"<meta seo title={`{{Title}}, page ${props.n}`} description={`Page ${props.n} of ${props.pages} of {{Title}}: older entries with their summaries, dates and tags, newest first.`}>

<script>
  import { page, pages } from 'brain:content'
  export function paths() {
    return pages('{{name}}')
  }
  export async function load(ctx) {
    return page('{{name}}', ctx.params.n)
  }
</script>

<main class="page">
  <h1>{{Title}}, page {props.n}</h1>
  <ul class="entries">
    {#each props.entries as e (e.slug)}
      <li>
        <h2><a href={e.url}>{e.title}</a></h2>
        <p>{e.description}</p>
      </li>
    {/each}
  </ul>
  <nav aria-label="Pages">
    {#if props.prev}<a href={props.prev} rel="prev">Newer entries</a>{/if}
    {#if props.next}<a href={props.next} rel="next">Older entries</a>{/if}
  </nav>
</main>

<style>
  .entries { display: grid; gap: var(--space-5); padding: 0; list-style: none; }
  h2 { margin: 0; font-size: 1.25rem; }
  nav { display: flex; gap: var(--space-4); }
</style>
"#;

const COLLECTION_ENTRY: &str = r#"<meta seo title={props.entry.title} description={props.entry.description}>

<script>
  import { entries, entry } from 'brain:content'
  export function paths() {
    return entries('{{name}}').map((e) => ({ slug: e.slug }))
  }
  export async function load(ctx) {
    return { entry: entry('{{name}}', ctx.params.slug) }
  }
</script>

<main class="page">
  <article>
    <h1>{props.entry.title}</h1>
    <p class="meta">
      {#if props.entry.date}<time datetime={props.entry.date}>{props.entry.date}</time>{/if}
      <span>{props.entry.readingMinutes} min read</span>
    </p>
    {@html sanitize(props.entry.html)}
    {#if props.entry.tags.length}
      <p>Tags:
        {#each props.entry.tags as t (t)}
          <a href={`/{{name}}/tag/${t.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`}>{t}</a>
        {/each}
      </p>
    {/if}
  </article>
  <p><a href="/{{name}}">All entries of {{Title}}</a></p>
</main>

<style>
  .meta { display: flex; gap: var(--space-3); color: var(--muted); font-size: .875rem; }
</style>
"#;

const COLLECTION_TAG: &str = r#"<meta seo title={`{{Title}} tagged ${props.tag}`} description={`Every entry of {{Title}} tagged ${props.tag}, newest first, with a short summary of each one and a link to read it in full.`}>

<script>
  import { tags, tagged } from 'brain:content'
  export function paths() {
    return tags('{{name}}').map((t) => ({ tag: t.slug }))
  }
  export async function load(ctx) {
    const t = tags('{{name}}').find((x) => x.slug === ctx.params.tag)
    return { tag: t ? t.tag : ctx.params.tag, entries: tagged('{{name}}', ctx.params.tag) }
  }
</script>

<main class="page">
  <h1>Tagged {props.tag}</h1>
  <ul class="entries">
    {#each props.entries as e (e.slug)}
      <li><a href={e.url}>{e.title}</a> · {e.description}</li>
    {/each}
  </ul>
  <p><a href="/{{name}}">All entries of {{Title}}</a></p>
</main>

<style>
  .entries { display: grid; gap: var(--space-3); padding-left: var(--space-5); }
</style>
"#;

/// `brain add lib <name>` → `lib/<name>.jsbrain`: logic without markup, a single default export (B0124).
fn add_lib(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_ident(name) {
        return Err(format!("invalid name `{name}`; fix: letters, digits and hyphens, starting with a letter"));
    }
    let rel = format!("lib/{}.jsbrain", kebab(name));
    let path = root.join(&rel);
    if path.exists() {
        return Err(format!("`{rel}` already exists; fix: edit it"));
    }
    let body = format!("// {}: helpers shared by views and components. One export per module: add functions to this object.\nexport default {{}}\n", kebab(name));
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path)
}

/// A new view is linked from app.brain's main nav (before `</nav>`), so it is never born orphaned (SEO B1813).
fn link_in_nav(root: &Path, href: &str, label: &str) {
    let app = root.join("app.brain");
    let Ok(src) = fs::read_to_string(&app) else { return };
    if src.contains(&format!("href=\"{href}\"")) {
        return;
    }
    let Some(nav) = src.find("<nav") else { return };
    let Some(close) = src[nav..].find("</nav>").map(|i| nav + i) else { return };
    let indent = src[..close].rsplit('\n').next().filter(|l| l.trim().is_empty()).map_or("", |l| l);
    let out = format!("{}  <a href=\"{href}\">{label}</a>\n{indent}{}", &src[..close], &src[close..]);
    let _ = fs::write(&app, out);
}

/// `brain add test <name>`: `<dir>/<name>.test.brain` next to the component (comp/, ui/ or views/).
fn add_test(root: &Path, name: &str) -> Result<PathBuf, String> {
    let file = format!("{}.brain", kebab(name));
    let comp = ["comp", "ui", "views"].iter().map(|d| root.join(d).join(&file)).find(|p| p.is_file()).ok_or_else(|| {
        format!("no component `{file}` in comp/, ui/ or views/; fix: create it first with `brain add comp {name}`")
    })?;
    write_test(root, &comp)
}

/// `<file>.test.brain` next to `comp`: mounts it and expects it to render.
fn write_test(root: &Path, comp: &Path) -> Result<PathBuf, String> {
    let file = comp.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let path = comp.with_extension("test.brain");
    if path.exists() {
        return Err(format!("`{}` already exists; fix: edit it", path.display()));
    }
    let comp_name = brain_parse::component_name(&comp.strip_prefix(root).unwrap_or(comp).to_path_buf());
    let body = format!(
        "<script>\n  import {comp_name} from './{file}'\n\n  test('renders', async () => {{\n    await mount({comp_name}, {{}})\n    expect(count('*') > 0).toBeTruthy()\n  }})\n</script>\n"
    );
    fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Validates `name:type[?][=default] ...` (SPEC §1) and returns it normalized.
pub fn validate_props(props: &str) -> Result<String, String> {
    let mut out = Vec::new();
    for p in props.split_whitespace() {
        let (decl, default) = p.split_once('=').map_or((p, None), |(a, b)| (a, Some(b)));
        let Some((name, ty)) = decl.split_once(':') else {
            return Err(format!("prop `{p}` has no type; fix: `{decl}:str` (types: {})", TYPES.join(" ")));
        };
        let base = ty.trim_end_matches('?');
        if !TYPES.contains(&base) {
            return Err(format!("unknown type `{base}` in `{p}`; fix: use one of {}", TYPES.join(" ")));
        }
        if ty.ends_with('?') && default.is_some() {
            return Err(format!("`{p}`: a prop with a default is already optional; fix: `{name}:{base}={}`", default.unwrap_or("")));
        }
        if !valid_ident(name) || name.contains('-') {
            return Err(format!("invalid prop name `{name}`; fix: camelCase, starting with a letter"));
        }
        out.push(match default {
            Some(d) => format!("{name}:{ty}={d}"),
            None => format!("{name}:{ty}"),
        });
    }
    Ok(out.join(" "))
}

const TOKENS_CSS: &str = include_str!("templates/tokens.cssbrain");

/// Welcome page: hero, the starter prompt for any LLM and a copy button (white-glass starter theme).
const HOME: &str = include_str!("templates/home.brain");

const COUNTER: &str = r#"<meta props="start:num=0 label:str='Clicks'">

<script>
  let n = sig(start)
  let dbl = drv(() => n * 2)
  fx(() => log(n))
</script>

<button on:click={n++} class:hot={n > 10}>{label}: {n} × 2 = {dbl}</button>

<style>
  button { padding: var(--space-2) var(--space-4); border-radius: var(--radius); &.hot { color: var(--bad); } }
</style>
"#;

const BADGE: &str = r#"<span class="badge"><slot /></span>

<style>
  .badge { padding: 0 var(--space-2); border-radius: var(--radius); background: var(--accent); color: var(--on-accent); font-size: .75em; }
</style>
"#;

/// For any agent that opens the project (Claude Code, Codex, Cursor…): what to read and which loop to follow.
const AGENTS_MD: &str = r#"# AGENTS.md — app BRAINLLM
1. Before writing code read `brain spec` (or `brain spec --topic <topic>`); what is not there does not exist.
2. Read `brain map --json` to see existing routes, components, props and modules; import only what it lists. To use a component read `brain card <name>` (its contract), never its body; `brain map --reads cart.total` lists who reads a store field. A component over 150 lines gets B1010: run the `brain split` it suggests.
3. Create files with `brain add view|comp|ui <name> --props "…"` and complete them; do not write them from scratch.
4. Loop: edit → `brain check --json` → apply `brain fix`, then each remaining error's `fix` → repeat until `[]`.
5. A diag with `"stuck": true` failed the same way 3 checks in a row: stop patching, read its `explain` (why + correct example) or `brain explain <code>`, and rewrite that part.
6. Done means `brain doctor` exits 0 (check + tests + accessibility + leaks). Not before; never claim a task done with doctor failing.
7. `brain run` serves the app and prints one `impact {…}` line per save: new/fixed diagnostics and Δ bytes per view. Per-environment values (API URLs, tokens) go in `.brain_env` through MCP `brain_env_set {key, section, value}`; `brain_env_keys` lists the keys, never the values. Use the CLI or discover the available tools through `brain mcp`. Prefer structured output where supported. Inspect the exit status and result; unified exit codes are still planned.
"#;

const PACKAGE_JSON: &str = r#"{
  "name": "{{name}}",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "brain run",
    "build": "brain build",
    "doctor": "brain doctor",
    "start": "brain start"
  },
  "devDependencies": {
    "@christianirack/brainllm": "{{version}}"
  }
}
"#;

const APP_MD: &str = r#"# {{name}}

BRAINLLM app. Rules: `brain spec` · check: `brain check --json` · development: `brain run` · production: `brain build`.

| Path | What goes there |
|---|---|
| `app.brain` | root layout: `<head>`, site `<meta seo>`, nav (every view is linked from it) |
| `views/` | routes (`about.brain` → `/about`, `404.brain`, `_offline.brain`); may export `load` |
| `comp/` · `ui/` | stateful components (with a colocated `.test.brain`) · stateless micro components |
| `channels/` `services/` `api/` `lib/` `server/` `content/` | see each README and its `brain add` command |
| `public/` | files served as is (icons, og.png); referenced ones get a content hash at build |
| `tokens.cssbrain` | design tokens: a `var(--x)` that does not exist is a compile error |
| `brain.json` | name, site URL, cache, SEO minimum score, budgets |
| `.brain_env` | variables per environment (never committed; `.brain_env.example` lists the keys) |

## Phases
- [ ] Describe the app in one paragraph (replace this list with your own phases)
- [ ] Views and navigation
- [ ] Data and server functions
- [ ] Tests and `brain doctor` exit 0
- [ ] `brain build` and deploy
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("brain-cli-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn new_then_add() {
        let app = tmp("demo");
        let files = new_app(&app).unwrap();
        assert_eq!(files.len(), 39);
        assert!(app.join("AGENTS.md").is_file());
        assert!(app.join("app.brain").is_file() && !app.join("index.html").exists());
        assert!(new_app(&app).is_err(), "must not overwrite a non-empty folder");
        assert_eq!(find_root(&app.join("views")), Some(app.clone()));
        let p = add(&app, "comp", "UserCard", "user:obj active:bool=false").unwrap();
        assert!(p.ends_with("comp/user-card.brain"));
        let body = fs::read_to_string(&p).unwrap();
        assert!(body.starts_with("<meta props=\"user:obj active:bool=false\">"));
        assert!(add(&app, "comp", "UserCard", "").is_err());
        fs::remove_dir_all(&app).unwrap();
    }

    /// What the CLI generates must pass its own `brain check`: the LLM copies the example's defects (QA, Phase 0).
    #[test]
    fn templates_pass_check() {
        let app = tmp("clean");
        new_app(&app).unwrap();
        add(&app, "view", "settings", "id:num?").unwrap();
        let shell = fs::read_to_string(app.join("app.brain")).unwrap();
        assert!(shell.contains("<a href=\"/settings\">Settings</a>\n    </nav>"), "the new view is in the nav: {shell}");
        add(&app, "comp", "UserCard", "user:obj active:bool=false").unwrap();
        assert!(app.join("comp/user-card.test.brain").is_file(), "brain add comp creates its colocated test");
        add(&app, "micro", "chip", "label:str").unwrap();
        let mut files = Vec::new();
        crate::brain_files(&app, &mut files);
        files.retain(|f| !f.to_string_lossy().ends_with(".test.brain")); // tests are not components (brain check skips them too)
        assert_eq!(files.len(), 18, "app.brain + the scaffold's views/components + 3 added");
        for f in &files {
            let src = fs::read_to_string(f).unwrap();
            let rel = f.strip_prefix(&app).unwrap();
            let (diags, _) = crate::compiler::check(&src, rel, f);
            assert!(diags.is_empty(), "{}: {}", rel.display(), crate::compiler::Diag::list_to_json(&diags));
        }
        fs::remove_dir_all(&app).unwrap();
    }

    #[test]
    fn module_generators_validate_and_preserve_files() {
        let root = tmp("modules"); new_app(&root).unwrap();
        let make = |kind, name, o| add_with_options(&root, kind, name, &o).unwrap_or_else(|e| panic!("{kind} {name}: {e}"));
        make("type", "User", AddOptions { fields: Some("id:num! email:email!".into()), ..Default::default() });
        let original = fs::read_to_string(root.join("types.jsbrain")).unwrap();
        make("type", "Order", AddOptions { fields: Some("id:num!".into()), ..Default::default() });
        assert!(fs::read_to_string(root.join("types.jsbrain")).unwrap().starts_with(&original));
        make("channel", "cart", AddOptions { store: Some("{items:[], count:0}".into()), ..Default::default() });
        make("channel", "events", AddOptions { bus: true, ..Default::default() });
        make("channel", "theme", AddOptions::default());
        make("service", "payments", AddOptions::default());
        make("server", "users-db", AddOptions::default());
        make("guard", "admin", AddOptions::default());
        make("api", "users", AddOptions { get: Some("[User]".into()), post: Some("User:User".into()), ..Default::default() });
        assert!(add_with_options(&root, "api", "users", &AddOptions { get: Some("list".into()), ..Default::default() }).is_err());
        assert!(add_with_options(&root, "type", "User", &AddOptions { fields: Some("id:num!".into()), ..Default::default() }).is_err());
        assert!(fs::read_to_string(root.join("api/users.jsbrain")).unwrap().contains("'[User]'"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn module_generators_reject_untrusted_options_without_writing() {
        let root = tmp("module-invalid"); new_app(&root).unwrap();
        for value in ["run()", "{}; export default {}", "{x: fetch('/private')}", "{[run()]:1}", "{__proto__:{}}", "{nested:{constructor:1}}"] {
            assert!(add_with_options(&root, "channel", "bad", &AddOptions { store: Some(value.into()), ..Default::default() }).is_err(), "{value}");
        }
        for name in ["../outside", "x/y", "..", "/tmp/escape"] {
            assert!(add_with_options(&root, "server", name, &AddOptions::default()).is_err());
        }
        assert!(add_with_options(&root, "service", "bad", &AddOptions { bus: true, ..Default::default() }).is_err());
        assert!(add_with_options(&root, "channel", "bad", &AddOptions { bus: true, key: true, ..Default::default() }).is_err());
        assert!(add_with_options(&root, "api", "bad", &AddOptions { get: Some("NoSuchType".into()), ..Default::default() }).is_err());
        assert!(add_with_options(&root, "type", "Bad", &AddOptions { fields: Some("x:num x:str".into()), ..Default::default() }).is_err());
        assert!(!root.join("channels/bad.jsbrain").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn module_type_lock_is_nonblocking_and_released() {
        let root = tmp("module-lock"); new_app(&root).unwrap();
        add_with_options(&root, "type", "First", &AddOptions { fields: Some("id:num!".into()), ..Default::default() }).unwrap();
        let lock = fs::OpenOptions::new().read(true).write(true).open(root.join("types.jsbrain")).unwrap();
        lock.try_lock().unwrap();
        let options = AddOptions { fields: Some("id:num!".into()), ..Default::default() };
        assert!(add_with_options(&root, "type", "Second", &options).unwrap_err().contains("active"));
        drop(lock);
        add_with_options(&root, "type", "Second", &options).unwrap();
        assert!(!root.join(".brain-add-type.lock").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn module_generator_refuses_symlink_directories() {
        let root = tmp("module-symlink"); new_app(&root).unwrap();
        fs::remove_dir_all(root.join("services")).unwrap();
        std::os::unix::fs::symlink(root.join("lib"), root.join("services")).unwrap();
        assert!(add_with_options(&root, "service", "escape", &AddOptions::default()).unwrap_err().contains("symlinks"));
        assert!(!root.join("lib/escape.jsbrain").exists());
        fs::remove_dir_all(root).unwrap();
    }

    /// Every reasonable text/background pair of the scaffold tokens passes B0408 (QA: the LLM could not find a valid pair).
    #[test]
    fn scaffold_tokens_contrast() {
        // Exactly the names from SPEC.md §CSS: the LLM only knows those.
        for t in ["space-1", "space-2", "space-3", "space-4", "space-5", "space-6", "radius", "font", "fg", "muted", "bg", "surface", "line", "accent", "on-accent", "ok", "bad", "radius-sm", "radius-lg", "glass", "glass-edge", "shadow", "scrim"] {
            assert!(TOKENS_CSS.contains(&format!("--{t}:")), "missing --{t}");
        }
        assert!(!TOKENS_CSS.contains("--sp-"));
        let tok = |name: &str| {
            let start = TOKENS_CSS.find(&format!("--{name}:")).unwrap() + name.len() + 3;
            let v = TOKENS_CSS[start..].split(';').next().unwrap().trim();
            brain_check::parse_hex(v).unwrap_or_else(|| panic!("--{name} is not hex: {v}"))
        };
        // every text token on every surface it can sit on; --line is a border, but body text (fg, muted) still
        // passes on it because a muted hint inside a bordered chip lands there (QA gate regression, #164)
        let text = ["fg", "muted", "accent", "ok", "bad"];
        let mut pairs: Vec<(&str, &str)> = text.iter().flat_map(|t| [(*t, "bg"), (*t, "surface")]).collect();
        pairs.extend([("fg", "line"), ("muted", "line"), ("on-accent", "accent"), ("on-accent", "ok"), ("on-accent", "bad")]);
        for (fg, bg) in pairs {
            let r = brain_check::contrast_ratio(tok(fg), tok(bg));
            assert!(r >= 4.5, "--{fg} on --{bg}: {r:.2}");
        }
    }

    #[test]
    fn new_app_in_exact_folder() {
        let dir = tmp("exact");
        fs::create_dir_all(dir.join(".git")).unwrap();
        new_app_named(&dir, "habits").unwrap();
        assert!(dir.join("app.brain").is_file(), "the app root is `dir` itself");
        assert!(fs::read_to_string(dir.join("app.brain")).unwrap().contains("<title>habits</title>"));
        assert!(new_app_named(&dir, "habits").unwrap_err().contains("not empty"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn props_errors_have_fix() {
        assert!(validate_props("n:int").unwrap_err().contains("fix"));
        assert!(validate_props("n").unwrap_err().contains("n:str"));
        assert_eq!(validate_props(" a:num=0  b:str? ").unwrap(), "a:num=0 b:str?");
        assert!(validate_props("a:bool?=false").unwrap_err().contains("a:bool=false"));
    }
}
