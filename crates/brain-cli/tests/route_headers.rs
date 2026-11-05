//! Native route headers are an explicit allowlist, including reads through proven aliases.
use std::{fs, path::{Path, PathBuf}, process::{Command, Stdio}, sync::atomic::{AtomicU64, Ordering}};
struct App(PathBuf);
impl Drop for App { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn fixture() -> App {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!("brain-route-headers-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&root).unwrap();
    let new = Command::new(env!("CARGO_BIN_EXE_brain")).args(["new", "app", "--json"]).current_dir(&root).stdin(Stdio::null()).output().unwrap();
    assert!(new.status.success(), "{}", String::from_utf8_lossy(&new.stderr));
    fs::create_dir_all(root.join("app/server/routes/headers_example")).unwrap();
    route(&root.join("app"), &["x-allowed"]);
    App(root)
}
fn route(app: &Path, names: &[&str]) {
    fs::write(app.join("server/routes/headers_example/route.jsbrain"), format!("<meta server>\nexport default {{ method: 'GET', path: '/headers-example', auth: 'public', headers: {} }}\n", serde_json::to_string(names).unwrap())).unwrap();
}
fn check(app: &Path, source: &str) -> Vec<serde_json::Value> {
    fs::write(app.join("server/routes/headers_example/handler.jsbrain"), source).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(["check", "--json"]).current_dir(app).stdin(Stdio::null()).output().unwrap();
    let diags: Vec<serde_json::Value> = serde_json::from_slice(&o.stdout).unwrap();
    assert!(o.status.success() && !diags.iter().any(|d| d["level"] == "error"), "{} {}", String::from_utf8_lossy(&o.stderr), String::from_utf8_lossy(&o.stdout));
    diags.into_iter().filter(|d| d["code"] == "B0816").collect()
}
#[test]
fn literal_header_aliases_warn_and_contract_edits_invalidate_cached_checks() {
    let root = fixture(); let app = root.0.join("app");
    let source = "<meta server>\nexport default async function handle(request) {\n  const context = request\n  const h = context.headers\n  return { body: { missing: h['x-missing'], allowed: request.headers['x-allowed'] } }\n}\n";
    let warnings = check(&app, source);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0]["level"], "warning");
    assert_eq!(warnings[0]["file"], "server/routes/headers_example/handler.jsbrain");
    assert_eq!(warnings[0]["line"], 5);
    assert!(warnings[0]["msg"].as_str().unwrap().contains("x-missing"));
    assert!(warnings[0]["fix"].as_str().unwrap().contains("route.jsbrain"));
    assert_eq!(warnings[0]["edits"].as_array().unwrap().len(), 0, "forwarding is an explicit author decision");
    route(&app, &["x-allowed", "x-missing"]);
    assert!(check(&app, source).is_empty(), "the handler source did not change, but its contract did");
}
#[test]
fn header_proofs_respect_scope_unknown_keys_mutation_and_auth_boundaries() {
    let root = fixture(); let app = root.0.join("app");
    for body in [
        "function mutate(value) { value['x-local'] = 'local' } const alias = ctx.headers.valueOf(); mutate(alias); return { body: ctx.headers['x-local'] }",
        "ctx.headers.__defineSetter__('x-local', () => 'local'); return { body: ctx.headers['x-local'] }",
        "ctx.headers.__defineGetter__('x-local', () => 'local'); return { body: ctx.headers['x-local'] }",
        "return { body: ctx.headers.hasOwnProperty('x-allowed') }",
        "const { toString: describe } = ctx.headers; return { body: describe }",
        "const h = { 'x-local': 1 }; return { body: h['x-local'] }",
        "const h = ctx.headers; function local(h) { return h['x-local'] } return { body: local({ 'x-local': 1 }) }",
        "const h = ctx.headers; { const h = { 'x-local': 1 }; h['x-local'] } return { body: 'ok' }",
        "const h = ctx.headers; h['x-local'] = 'local'; return { body: h['x-local'] }",
        "function mutate(value) { value.headers = { 'x-local': 'local' } } mutate(ctx); return { body: ctx.headers['x-local'] }",
        "const key = 'x-' + ctx.params.name; return { body: ctx.headers[key] }",
        "function mutate(value) { value['x-local'] = 'local' } const key = ctx.params.key; const maybe = ctx[key]; mutate(maybe); return { body: ctx.headers['x-local'] }",
        "function mutate(value) { value.headers = {} } const maybe = ctx.params.flag ? ctx : {}; mutate(maybe); return { body: ctx.headers['x-local'] }",
        "function mutate(value) { value.headers['x-local'] = 'local' } const maybe = { ...ctx }; mutate(maybe); return { body: ctx.headers['x-local'] }",
        "function mutate(value) { value.headers['x-local'] = 'local' } const { ...maybe } = ctx; mutate(maybe); return { body: ctx.headers['x-local'] }",
        "function mutate(value) { value[0].headers['x-local'] = 'local' } const maybe = [ctx, {}]; mutate(maybe); return { body: ctx.headers['x-local'] }",
        "function mutate(value) { value.headers = {} } const [maybe] = [ctx]; mutate(maybe); return { body: ctx.headers['x-local'] }",
        "function mutate(value) { value.h['x-local'] = 'local' } const extra = {}; const box = { ...extra, h: ctx.headers }; mutate(box); return { body: ctx.headers['x-local'] }",
        "function mutate(value) { value[0].box.h['x-local'] = 'local' } const extra = {}; const box = [{ box: { ...extra, h: ctx.headers } }, {}]; mutate(box); return { body: ctx.headers['x-local'] }",
        "return { body: 'ok' }; ctx.headers['x-unreachable']",
        "const note = 'ctx.headers[not-a-read]'; return { body: note } // ctx.headers['x-comment']",
    ] {
        let source = format!("<meta server>\nexport default async function handle(ctx) {{\n{body}\n}}\n");
        assert!(check(&app, &source).is_empty(), "unproven header read: {body}");
    }
    for body in [
        "const { headers: h } = ctx; const { 'x-missing': missing } = h; return { body: missing }",
        "return { body: ctx.headers['X-Allowed'] }",
        "return { body: ctx.headers.authorization }",
    ] {
        let source = format!("<meta server>\nexport default async function handle(ctx) {{\n{body}\n}}\n");
        let warnings = check(&app, &source);
        assert_eq!(warnings.len(), 1, "{body}: {warnings:?}");
        assert!(warnings[0]["fix"].as_str().unwrap().contains("ctx.user"));
    }
}
