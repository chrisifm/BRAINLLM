//! Conservative production checks for direct client widget bindings. No source or environment value is reported.
//! This is not whole-program evaluation: computed keys, external JavaScript and remote configuration stay
//! unknown. Only reachable source, literal strings and direct PUBLIC_* reads can prove a bad production site key.
use brain_parse::ast::*;
use std::{collections::{BTreeMap, BTreeSet, HashSet}, fs, path::{Path, PathBuf}};

pub(super) fn problem(root: &Path, environment: &str) -> Option<(String, String, bool)> {
    if environment != "production" { return None; }
    let meta = fs::symlink_metadata(root).ok()?;
    if !meta.is_dir() || meta.file_type().is_symlink() { return None; }
    let root = root.canonicalize().ok()?;
    let mut pending = vec![root.join("app.brain"), root.join("_layout.brain")];
    if !views(&root, &root.join("views"), 0, &mut ViewBudget::default(), &mut pending) { return None; }
    let mut seen = BTreeSet::new();
    let mut bytes = 0;
    while let Some(path) = pending.pop() {
        let path = path.canonicalize().ok().filter(|p| p.starts_with(&root));
        let Some(path) = path else { continue };
        if !seen.insert(path.clone()) { continue; }
        if seen.len() > 4096 { break; }
        let Ok(meta) = fs::metadata(&path) else { continue };
        if meta.len() > 1024 * 1024 { continue; }
        bytes += meta.len();
        if bytes > 32 * 1024 * 1024 { break; }
        let Ok(source) = fs::read_to_string(&path) else { continue };
        let Ok(component) = brain_parse::parse(&source, &path) else { continue };
        if component.meta.iter().any(|m| m.name == "server") { continue; }
        let file = path.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
        let local_bound = brain_parse::visit::Uses::free(&component).bound;
        let mut aliases = BTreeMap::new();
        for stmt in &component.script { constant_aliases(stmt, &local_bound, &mut aliases); }
        let mut bound = local_bound;
        for stmt in &component.script { declared_names(stmt, &mut bound); }
        let shadowed = ["process", "import", "turnstile", "hcaptcha", "grecaptcha", "window", "globalThis", "onMount", "mounted"].iter().any(|name| bound.contains(*name));
        let mut found = None;
        let mut check = |expr: &Expr, line| {
            if !shadowed && found.is_none() { found = binding(&root, environment, expr, &aliases, 0).map(|(provider, name)| (provider, name, line)); }
        };
        nodes(&component.template, &mut check);
        nodes(&component.head, &mut check);
        // Attribute checks use the same callback as render() options; the extra walk examines only render calls.
        statements(&component.script, &mut |expr| render(expr, &mut check));
        nodes_exprs(&component.template, &mut |expr| render(expr, &mut check));
        nodes_exprs(&component.head, &mut |expr| render(expr, &mut check));
        if let Some((provider, name, line)) = found {
            let location = name.map_or_else(|| "a literal site key".into(), |name| format!("{name}"));
            return Some((file.clone(), format!("{file}:{line}: [production] {location} is used as a widget site key and holds a public provider test site key ({provider}); `brain backend serve --env production` refuses to start"), false));
        }
        let mut components = BTreeSet::new();
        walk_nodes(&component.template, &mut |node| { if let Node::ComponentRef(c) = node { components.insert(c.name.clone()); } });
        let parent = path.parent().unwrap_or(&root);
        for stmt in &component.script {
            if let Stmt::Import(import) = stmt {
                if import.source.ends_with(".brain") && !import.default.as_ref().is_some_and(|id| components.contains(&id.name)) { continue; }
                if import.source.starts_with('.') && (import.source.ends_with(".brain") || import.source.ends_with(".jsbrain")) {
                    pending.push(parent.join(&import.source));
                }
            }
        }
        let (discovered, _) = brain_check::discover::load(&path, &component, Some(&root));
        for (name, resolution) in &discovered.resolutions {
            if !components.contains(name) { continue; }
            if let brain_check::discover::Resolution::Unique { source } = resolution { pending.push(parent.join(source)); }
        }
    }
    None
}

/// A provider-named environment setting is useful evidence, but does not prove which key the client receives.
/// Keep it advisory when no direct production widget binding can be resolved.
pub(super) fn unbound_problem(root: &Path, environment: &str) -> Option<(String, String)> {
    if environment != "production" || problem(root, environment).is_some() { return None; }
    let source = fs::read_to_string(root.join(crate::env::FILE)).ok()?;
    let file = brain_check::env::parse(&source);
    let values = file.public(environment, &|name| std::env::var(name).ok());
    for (name, value) in values {
        let normalized = name.replace('_', "");
        let provider_named = ["TURNSTILE", "HCAPTCHA", "RECAPTCHA", "CAPTCHA"].iter().any(|provider| normalized.contains(provider));
        if provider_named && normalized.contains("SITEKEY") && brain_backend::captcha::public_test_site_key(&value).is_some() {
            return Some((crate::env::FILE.into(), format!("[production] {name} holds a public provider test site key, but no direct production widget binding was proven; advisory only: inspect computed values, aliases across modules and external client configuration")));
        }
    }
    None
}

fn constant_aliases(stmt: &Stmt, local_bound: &HashSet<String>, aliases: &mut BTreeMap<String, Expr>) {
    match stmt {
        Stmt::Var(var) if var.kind == VarKind::Const => for decl in &var.decls {
            if let (Pat::Ident(id), Some(expr)) = (&decl.pat, &decl.init) {
                // Any inner binding of this name makes module alias resolution ambiguous, so leave it unknown.
                if !local_bound.contains(&id.name) { aliases.insert(id.name.clone(), expr.clone()); }
            }
        },
        Stmt::Export { decl, .. } => constant_aliases(decl, local_bound, aliases),
        _ => {},
    }
}

fn declared_names(stmt: &Stmt, names: &mut HashSet<String>) {
    match stmt {
        Stmt::Var(var) => for decl in &var.decls { let mut ids = Vec::new(); decl.pat.bound_names(&mut ids); names.extend(ids.into_iter().map(|id| id.name.clone())); },
        Stmt::Func(func) => names.extend(func.name.iter().map(|id| id.name.clone())),
        Stmt::Import(import) => { names.extend(import.default.iter().chain(import.namespace.iter()).map(|id| id.name.clone())); names.extend(import.named.iter().map(|(_, id)| id.name.clone())); }
        Stmt::Export { decl, .. } => declared_names(decl, names),
        _ => {},
    }
}

const MAX_VIEW_ENTRIES: usize = 8192;
const MAX_VIEW_DIRS: usize = 256;
const MAX_VIEW_DEPTH: usize = 12;
#[derive(Default)]
struct ViewBudget { entries: usize, dirs: usize, files: usize }

/// An incomplete scan proves nothing. Check confinement and symlinks before opening each directory;
/// retain at most the bounded entry budget, including names that are later excluded.
fn views(root: &Path, dir: &Path, depth: usize, budget: &mut ViewBudget, out: &mut Vec<PathBuf>) -> bool {
    if depth > MAX_VIEW_DEPTH || budget.dirs >= MAX_VIEW_DIRS { return false; }
    let meta = match fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    if !meta.is_dir() || meta.file_type().is_symlink() { return false; }
    if fs::canonicalize(dir).ok().as_deref() != Some(dir) || !dir.starts_with(root) { return false; }
    budget.dirs += 1;
    let Ok(read) = fs::read_dir(dir) else { return false };
    let mut entries = Vec::new();
    for entry in read {
        if budget.entries >= MAX_VIEW_ENTRIES { return false; }
        budget.entries += 1;
        let Ok(entry) = entry else { return false };
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else { return false };
        if name.starts_with('.') || name.ends_with(".test.brain") || matches!(name.as_str(), "tests" | "test" | "node_modules" | "dist" | "docs") { continue; }
        let Ok(kind) = entry.file_type() else { return false };
        if kind.is_symlink() { return false; }
        if kind.is_dir() {
            if !views(root, &entry.path(), depth + 1, budget, out) { return false; }
        } else if kind.is_file() && name.ends_with(".brain") {
            if budget.files >= 4096 { return false; }
            let path = entry.path();
            if fs::canonicalize(&path).ok().as_ref() != Some(&path) || !path.starts_with(root) { return false; }
            budget.files += 1;
            out.push(path);
        }
    }
    true
}

fn member(expr: &Expr) -> Option<String> {
    match expr.unparen() {
        Expr::Ident(id) => Some(id.name.clone()),
        Expr::Member { obj, prop: MemberProp::Name(prop), .. } => Some(format!("{}.{}", member(obj)?, prop.name)),
        _ => None,
    }
}
fn binding(root: &Path, environment: &str, expr: &Expr, aliases: &BTreeMap<String, Expr>, depth: usize) -> Option<(&'static str, Option<String>)> {
    if depth > 8 { return None; }
    if let Expr::Ident(id) = expr.unparen() {
        return binding(root, environment, aliases.get(&id.name)?, aliases, depth + 1);
    }
    let (value, name) = match expr.unparen() {
        Expr::Str { value, .. } => (value.clone(), None),
        expr => {
            let path = member(expr)?;
            let name = path.strip_prefix("process.env.").or_else(|| path.strip_prefix("import.meta.env."))?;
            if !name.starts_with("PUBLIC_") { return None; }
            (crate::env::value(root, environment, name)?, Some(name.to_string()))
        }
    };
    brain_backend::captcha::public_test_site_key(&value).map(|provider| (provider, name))
}
fn render(expr: &Expr, check: &mut impl FnMut(&Expr, u32)) {
    let Expr::Call { callee, args, .. } = expr else { return };
    let Some(path) = member(callee) else { return };
    let path = path.strip_prefix("window.").or_else(|| path.strip_prefix("globalThis.")).unwrap_or(&path);
    if !matches!(path, "turnstile.render" | "hcaptcha.render" | "grecaptcha.render") { return; }
    let Some(Expr::Object { props, .. }) = args.get(1).map(Expr::unparen) else { return };
    let mut site_key = None;
    for prop in props {
        match prop {
            ObjProp::KeyValue { key, value, .. } => {
                let name = match key {
                    PropKey::Ident(id) => id.name.as_str(), PropKey::Str { value, .. } => value.as_str(),
                    // Computed properties can overwrite sitekey; do not infer their runtime key.
                    _ => return,
                };
                if name == "sitekey" { site_key = Some(value); }
            }
            // Any of these can replace a previously seen key. Leave the whole options object unknown.
            ObjProp::Spread { .. } | ObjProp::Shorthand(_) | ObjProp::Method { .. } => return,
        }
    }
    if let Some(value) = site_key { check(value, value.span().line); }
}
// Only expressions executed during module initialization or a known mount callback prove configuration.
// Uncalled functions, arbitrary callbacks and conditional branches remain unknown.
fn expressions(expr: &Expr, f: &mut impl FnMut(&Expr)) {
    f(expr);
    match expr {
        Expr::Func(_) => {},
        Expr::Call { callee, args, .. } => {
            expressions(callee, f);
            for arg in args {
                if matches!(member(callee).as_deref(), Some("onMount" | "mounted")) && let Expr::Func(func) = arg.unparen() {
                    match &func.body { FnBody::Expr(expr) => expressions(expr, f), FnBody::Block(body) => statements(body, f) }
                } else { expressions(arg, f); }
            }
        }
        Expr::Cond { test, cons, alt, .. } => {
            expressions(test, f);
            if let Expr::Bool { value, .. } = test.unparen() { expressions(if *value { cons } else { alt }, f); }
        }
        Expr::Binary { op: "&&" | "||" | "??", left, .. } => expressions(left, f),
        Expr::Binary { left, right, .. } => { expressions(left, f); expressions(right, f); }
        Expr::Unary { arg, .. } | Expr::Update { arg, .. } | Expr::Spread { arg, .. } | Expr::Await { arg, .. } | Expr::Paren { expr: arg, .. } => expressions(arg, f),
        Expr::Assign { target, value, .. } => { expressions(target, f); expressions(value, f); }
        Expr::Array { elems, .. } => for expr in elems.iter().flatten() { expressions(expr, f); },
        Expr::Object { props, .. } => for prop in props {
            match prop {
                ObjProp::KeyValue { value, .. } => expressions(value, f), ObjProp::Spread { arg, .. } => expressions(arg, f), _ => {},
            }
        }
        Expr::Member { obj, prop, .. } => { expressions(obj, f); if let MemberProp::Computed(expr) = prop { expressions(expr, f); } }
        Expr::New { callee, args, .. } => { expressions(callee, f); for arg in args { expressions(arg, f); } }
        Expr::Seq { exprs, .. } | Expr::Template { exprs, .. } => for expr in exprs { expressions(expr, f); },
        _ => {},
    }
}
fn statements(stmts: &[Stmt], f: &mut impl FnMut(&Expr)) {
    for stmt in stmts {
        match stmt {
            Stmt::Export { decl, .. } => statements(std::slice::from_ref(decl), f),
            Stmt::Var(var) => for decl in &var.decls { if let Some(expr) = &decl.init { expressions(expr, f); } },
            Stmt::Expr { expr, .. } | Stmt::ExportDefault { expr, .. } | Stmt::Throw { arg: expr, .. } => expressions(expr, f),
            Stmt::Return { arg: Some(expr), .. } => expressions(expr, f),
            Stmt::Block { body, .. } => statements(body, f),
            Stmt::If { test, cons, alt, .. } => {
                expressions(test, f);
                if let Expr::Bool { value, .. } = test.unparen() {
                    if *value { statements(std::slice::from_ref(cons), f); }
                    else if let Some(alt) = alt { statements(std::slice::from_ref(alt), f); }
                }
            }
            _ => {},
        }
    }
}
fn nodes(nodes: &[Node], check: &mut impl FnMut(&Expr, u32)) {
    // Use the AST's data-sitekey attribute, not a substring search through comments or descriptive strings.
    walk_nodes(nodes, &mut |node| {
        let attrs = match node { Node::Element(e) => &e.attrs, _ => return };
        for attr in attrs.iter().filter(|a| a.name == "data-sitekey") {
            match &attr.value {
                AttrValue::Expr(expr) => check(expr, attr.span.line),
                AttrValue::Text(value) => {
                    let expr = Expr::Str { value: value.clone(), raw: String::new(), span: attr.span };
                    check(&expr, attr.span.line);
                }
                _ => {},
            }
        }
    });
}
fn nodes_exprs(nodes: &[Node], f: &mut impl FnMut(&Expr)) {
    walk_nodes(nodes, &mut |node| {
        let attrs = match node {
            Node::Element(e) => Some(&e.attrs), Node::ComponentRef(e) => Some(&e.attrs),
            Node::Interp(e) => { expressions(&e.expr, f); None }, _ => None,
        };
        if let Some(attrs) = attrs { for attr in attrs { if let AttrValue::Expr(expr) = &attr.value { expressions(expr, f); } } }
    });
}
fn walk_nodes(nodes: &[Node], f: &mut impl FnMut(&Node)) {
    for node in nodes {
        f(node);
        match node {
            Node::Element(e) => walk_nodes(&e.children, f), Node::ComponentRef(e) => walk_nodes(&e.children, f),
            Node::If(e) => {
                let mut known = true;
                let mut taken = false;
                for branch in &e.branches {
                    match branch.test.unparen() {
                        Expr::Bool { value: true, .. } => { walk_nodes(&branch.body, f); taken = true; break; }
                        Expr::Bool { value: false, .. } => {},
                        _ => { known = false; break; },
                    }
                }
                if known && !taken && let Some(body) = &e.otherwise { walk_nodes(body, f); }
            }
            Node::Each(_) => {},
            Node::Await(e) => walk_nodes(&e.pending, f),
            Node::SlotScope(e) => walk_nodes(&e.body, f), _ => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!("brain-captcha-site-key-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
            let out = Self(root);
            out.write("server/auth/captcha.jsbrain", "<meta server>\nexport default {verify_url: 'https://challenges.example.com/verify', secret_env: 'BRAIN_TEST_CAPTCHA_SITE_SECRET', protect: ['signup']}\n");
            out.write(".brain_env", "BRAIN_TEST_CAPTCHA_SITE_SECRET=synthetic-provider-issued-secret\n");
            out
        }
        fn write(&self, file: &str, source: &str) {
            let path = self.0.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    const DUMMY: &str = "1x00000000000000000000AA";
    #[test]
    fn production_refuses_direct_widget_site_keys_with_a_real_secret() {
        let fixture = Fixture::new();
        for key in [DUMMY, "2x00000000000000000000AB", "1x00000000000000000000BB", "2x00000000000000000000BB", "3x00000000000000000000FF", "10000000-ffff-ffff-ffff-000000000001", "20000000-ffff-ffff-ffff-000000000002", "30000000-ffff-ffff-ffff-000000000003", "6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI"] {
            fixture.write("views/index.brain", &format!("<meta view>\n<div data-sitekey=\"{key}\"></div>\n"));
            let (file, message, disables) = super::super::captcha_public_test_secret_problem(&fixture.0, "production").expect("known test site key");
            assert_eq!(file, "views/index.brain");
            assert!(message.contains("test site key") && message.contains(":2:") && !disables);
            assert!(!message.contains(key) && !message.contains("synthetic-provider-issued-secret"));
            assert!(super::super::load(&fixture.0, "production", &[], &Default::default()).err().unwrap().contains("test site key"));
            assert!(super::super::load(&fixture.0, "development", &[], &Default::default()).is_ok());
        }
    }
    #[test]
    fn direct_public_environment_bindings_use_the_selected_environment() {
        let fixture = Fixture::new();
        fixture.write("views/index.brain", "<meta view>\n<div data-sitekey={process.env.PUBLIC_BRAIN_CAPTCHA_SITE_KEY}></div>\n");
        fixture.write(".brain_env", &format!("BRAIN_TEST_CAPTCHA_SITE_SECRET=synthetic-provider-issued-secret\n[development]\nPUBLIC_BRAIN_CAPTCHA_SITE_KEY={DUMMY}\n[production]\nPUBLIC_BRAIN_CAPTCHA_SITE_KEY=synthetic-provider-issued-site-key\n"));
        assert!(problem(&fixture.0, "production").is_none());
        fixture.write(".brain_env", &format!("BRAIN_TEST_CAPTCHA_SITE_SECRET=synthetic-provider-issued-secret\nPUBLIC_BRAIN_CAPTCHA_SITE_KEY={DUMMY}\n"));
        let (_, message, _) = problem(&fixture.0, "production").unwrap();
        assert!(message.contains("PUBLIC_BRAIN_CAPTCHA_SITE_KEY") && !message.contains(DUMMY));
        assert!(problem(&fixture.0, "development").is_none());
        fixture.write("views/index.brain", "<meta view>\n<div data-sitekey={import.meta.env.PUBLIC_BRAIN_CAPTCHA_SITE_KEY}></div>\n");
        assert!(problem(&fixture.0, "production").is_some());
        fixture.write("views/index.brain", "<meta view>\n<script>const key = process.env.PUBLIC_BRAIN_CAPTCHA_SITE_KEY</script>\n<div data-sitekey={key}></div>\n");
        assert!(problem(&fixture.0, "production").is_some(), "an immutable unshadowed module alias preserves evidence");
        fixture.write("views/index.brain", "<meta view>\n<script>const key = process.env.PUBLIC_BRAIN_CAPTCHA_SITE_KEY\nfunction sample(key) { return key }</script>\n<div data-sitekey={key}></div>\n");
        assert!(problem(&fixture.0, "production").is_none(), "shadowed aliases are left unknown");
        fixture.write(".brain_env", &format!("BRAIN_TEST_CAPTCHA_SITE_SECRET=synthetic-provider-issued-secret\nPUBLIC_TURNSTILE_SITE_KEY={DUMMY}\n"));
        fixture.write("views/index.brain", "<meta view>\n<div data-sitekey={props.dynamicKey}></div>\n");
        let (_, advisory) = super::super::captcha_public_test_site_key_warning(&fixture.0, "production").unwrap();
        assert!(advisory.contains("PUBLIC_TURNSTILE_SITE_KEY") && advisory.contains("advisory only") && !advisory.contains(DUMMY));
        assert!(super::super::load(&fixture.0, "production", &[], &Default::default()).is_ok(), "unproven binding cannot refuse startup");
    }
    #[test]
    fn a_process_override_is_resolved_without_mutating_the_parent_environment() {
        const CHILD: &str = "BRAIN_TEST_CAPTCHA_SITE_CHILD";
        if let Ok(root) = std::env::var(CHILD) {
            assert!(problem(Path::new(&root), "production").is_none());
            return;
        }
        let fixture = Fixture::new();
        fixture.write("views/index.brain", "<meta view>\n<div data-sitekey={process.env.PUBLIC_BRAIN_CAPTCHA_SITE_KEY}></div>\n");
        fixture.write(".brain_env", &format!("BRAIN_TEST_CAPTCHA_SITE_SECRET=synthetic-provider-issued-secret\nPUBLIC_BRAIN_CAPTCHA_SITE_KEY={DUMMY}\n"));
        assert!(problem(&fixture.0, "production").is_some());
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "platform_config::captcha_site_keys::tests::a_process_override_is_resolved_without_mutating_the_parent_environment", "--test-threads=1"])
            .env(CHILD, &fixture.0).env("PUBLIC_BRAIN_CAPTCHA_SITE_KEY", "synthetic-provider-issued-site-key")
            .output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    }
    #[test]
    fn only_reachable_widget_bindings_are_evidence() {
        let fixture = Fixture::new();
        fixture.write("views/index.brain", &format!("<meta view>\n<script>\n// turnstile.render('#widget', {{sitekey: '{DUMMY}'}})\nconst text = 'sitekey: {DUMMY}'\n</script>\n<div>{{text}}</div>\n"));
        fixture.write("comp/unused.brain", &format!("<meta>\n<div data-sitekey=\"{DUMMY}\"></div>"));
        fixture.write("views/index.test.brain", &format!("<div data-sitekey=\"{DUMMY}\"></div>"));
        fixture.write("docs/sample.brain", &format!("<div data-sitekey=\"{DUMMY}\"></div>"));
        fixture.write(".brain/cache/sample.brain", &format!("<div data-sitekey=\"{DUMMY}\"></div>"));
        fixture.write(".brain_env", &format!("BRAIN_TEST_CAPTCHA_SITE_SECRET=synthetic-provider-issued-secret\nPUBLIC_UNUSED={DUMMY}\n"));
        assert!(problem(&fixture.0, "production").is_none());
        fixture.write("views/index.brain", &format!("<meta view>\n<script>function unused() {{ turnstile.render('#widget', {{sitekey: '{DUMMY}'}}) }}\nif (process.env.PUBLIC_MODE === 'development') {{ turnstile.render('#widget', {{sitekey: '{DUMMY}'}}) }}</script>\n{{#if false}}<div data-sitekey=\"{DUMMY}\"></div>{{/if}}\n"));
        assert!(problem(&fixture.0, "production").is_none(), "uncalled functions and unproven branches stay unknown");
        fixture.write("views/index.brain", &format!("<meta view>\n<script>const process = {{env: {{PUBLIC_BRAIN_CAPTCHA_SITE_KEY: '{DUMMY}'}}}}</script>\n<div data-sitekey={{process.env.PUBLIC_BRAIN_CAPTCHA_SITE_KEY}}></div>\n"));
        assert!(problem(&fixture.0, "production").is_none(), "a shadowed global is not an environment binding");
        fixture.write("views/index.brain", "<meta view>\n<script>import Widget from '../comp/widget.brain'</script>\n<Widget />\n");
        fixture.write("comp/widget.brain", &format!("<meta>\n<div data-sitekey=\"{DUMMY}\"></div>"));
        assert_eq!(problem(&fixture.0, "production").unwrap().0, "comp/widget.brain");
    }
    #[test]
    fn render_options_are_checked_but_dynamic_and_server_values_are_unknown() {
        let fixture = Fixture::new();
        for provider in ["turnstile", "hcaptcha", "grecaptcha"] {
            fixture.write("views/index.brain", &format!("<meta view>\n<script>onMount(() => {{ {provider}.render('#widget', {{sitekey: '{DUMMY}'}}) }})</script>\n<div id=\"widget\"></div>\n"));
            assert!(problem(&fixture.0, "production").is_some(), "{provider}");
        }
        fixture.write("views/index.brain", &format!("<meta view>\n<script>\nexport async function load(ctx) {{ return {{key: '{DUMMY}'}} }}\nconst sample = {{sitekey: '{DUMMY}'}}\n</script>\n<div data-sitekey={{props.key}}></div>\n"));
        assert!(problem(&fixture.0, "production").is_none());
        fixture.write("views/index.brain", "<meta view>\n<div data-sitekey=\"synthetic-provider-issued-site-key\"></div>\n");
        assert!(problem(&fixture.0, "production").is_none());
        fs::remove_file(fixture.0.join(brain_backend::captcha::CONTRACT)).unwrap();
        fixture.write("views/index.brain", &format!("<meta view>\n<div data-sitekey=\"{DUMMY}\"></div>\n"));
        assert!(super::super::captcha_public_test_secret_problem(&fixture.0, "production").is_none(), "no configured backend CAPTCHA");
    }
    fn write_widget(fixture: &Fixture, lifecycle: &str, options: &str) {
        let source = format!("<meta view>\n<script>{lifecycle}(() => {{ turnstile.render('#widget', {{{options}}}) }})</script>\n<div id=\"widget\"></div>\n");
        brain_parse::parse(&source, Path::new("views/index.brain")).expect("the widget regression must parse");
        fixture.write("views/index.brain", &source);
    }
    #[test]
    fn render_options_respect_last_keys_and_leave_runtime_overwrites_unknown() {
        let fixture = Fixture::new();
        for options in [
            format!("sitekey: '{DUMMY}', ...unknown"),
            format!("sitekey: '{DUMMY}', sitekey: 'synthetic-real-site-key'"),
            format!("sitekey: '{DUMMY}', 'sitekey': 'synthetic-real-site-key'"),
            format!("sitekey: '{DUMMY}', [runtimeKey]: 'synthetic-real-site-key'"),
            format!("sitekey: '{DUMMY}', ['sitekey']: 'synthetic-real-site-key'"),
            format!("sitekey: '{DUMMY}', sitekey"),
            format!("sitekey: '{DUMMY}', sitekey() {{ return 'synthetic-real-site-key' }}"),
            format!("sitekey: '{DUMMY}', sitekey: props.dynamicKey"),
        ] {
            write_widget(&fixture, "onMount", &options);
            assert!(problem(&fixture.0, "production").is_none(), "an earlier test literal cannot prove the final key: {options}");
            assert!(super::super::load(&fixture.0, "production", &[], &Default::default()).is_ok());
        }
        for options in [
            format!("sitekey: 'synthetic-real-site-key', sitekey: '{DUMMY}'"),
            format!("theme: 'light', sitekey: '{DUMMY}'"),
        ] {
            write_widget(&fixture, "onMount", &options);
            assert!(problem(&fixture.0, "production").is_some(), "the final known literal is still checked");
        }
    }
    #[test]
    fn the_documented_mounted_callback_checks_literal_site_keys() {
        let fixture = Fixture::new();
        write_widget(&fixture, "mounted", &format!("sitekey: '{DUMMY}'"));
        assert!(problem(&fixture.0, "production").is_some(), "mounted is the documented lifecycle primitive");
        write_widget(&fixture, "mounted", "sitekey: 'synthetic-real-site-key'");
        assert!(problem(&fixture.0, "production").is_none());
    }
    #[test]
    fn view_discovery_bounds_entries_directories_and_depth() {
        let fixture = Fixture::new();
        fixture.write("views/a.brain", "<div></div>");
        fixture.write("views/b.brain", "<div></div>");
        let root = fixture.0.canonicalize().unwrap();
        let views_dir = root.join("views");
        let mut out = Vec::new();
        let mut budget = ViewBudget { entries: MAX_VIEW_ENTRIES - 1, ..Default::default() };
        assert!(!views(&root, &views_dir, 0, &mut budget, &mut out));
        assert_eq!(budget.entries, MAX_VIEW_ENTRIES, "the buffer never grows beyond the global entry budget");
        let mut budget = ViewBudget { dirs: MAX_VIEW_DIRS, ..Default::default() };
        assert!(!views(&root, &views_dir, 0, &mut budget, &mut out));
        assert_eq!(budget.entries, 0, "an exhausted directory budget stops before enumeration");
        assert!(!views(&root, &views_dir, MAX_VIEW_DEPTH + 1, &mut ViewBudget::default(), &mut out));
        fixture.write("app.brain", &format!("<div data-sitekey=\"{DUMMY}\"></div>"));
        for index in 0..MAX_VIEW_DIRS { fs::create_dir_all(views_dir.join(format!("branch-{index:03}"))).unwrap(); }
        assert!(problem(&root, "production").is_none(), "partial discovery cannot become a fatal finding");
    }
    #[cfg(unix)]
    #[test]
    fn view_discovery_refuses_symlinks_and_outside_directories_before_enumeration() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = Fixture::new();
        outside.write("views/index.brain", &format!("<div data-sitekey=\"{DUMMY}\"></div>"));
        let root = fixture.0.canonicalize().unwrap();
        let outside_dir = outside.0.join("views").canonicalize().unwrap();
        let mut budget = ViewBudget::default();
        assert!(!views(&root, &outside_dir, 0, &mut budget, &mut Vec::new()));
        assert_eq!(budget.entries, 0);
        symlink(&outside_dir, root.join("views")).unwrap();
        assert!(!views(&root, &root.join("views"), 0, &mut budget, &mut Vec::new()));
        assert_eq!(budget.entries, 0, "a symlink target is never enumerated");
        assert!(problem(&root, "production").is_none());
    }

}
