//! What `brain backend serve` would refuse at startup is reported by `brain check` first: contract errors as B0735,
//! a secret the environment does not hold as B0790.
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn brain(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env_remove("BRAIN_BACKEND_URL")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}
struct App(PathBuf);
impl App {
    fn new() -> Self {
        let tmp = std::env::temp_dir().join(format!("brain-contract-check-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(brain(&tmp, &["new", "app"]).status.success());
        let app = App(tmp);
        assert!(!text(&brain(app.dir().as_path(), &["check"])).contains("B07"), "a new app has nothing to report");
        app
    }
    fn dir(&self) -> PathBuf {
        self.0.join("app")
    }
    fn write(&self, rel: &str, body: &str) {
        let path = self.dir().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    fn route(&self, contract: &str) {
        self.write("server/routes/pay/route.jsbrain", &format!("<meta server>\nexport default {contract}\n"));
        self.write("server/routes/pay/handler.jsbrain", "<meta server>\nexport default async function handle(ctx) {\n  return { ok: true }\n}\n");
    }
    fn env(&self, key: &str, value: &str) {
        let path = self.dir().join(".brain_env");
        let kept: Vec<String> = std::fs::read_to_string(&path).unwrap_or_default().lines().filter(|line| !line.starts_with(&format!("{key}="))).map(str::to_string).collect();
        std::fs::write(path, format!("{key}={value}\n{}\n", kept.join("\n"))).unwrap();
    }
    fn check(&self) -> (bool, String) {
        let run = brain(&self.dir(), &["check"]);
        (run.status.success(), text(&run))
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn an_undefined_route_secret_is_reported_with_its_name_and_never_its_value() {
    let app = App::new();
    app.route("{ method: 'POST', path: '/pay/hook', auth: 'internal', internal: { secret: 'PAY_INTERNAL_SECRET' } }");
    let (ok, out) = app.check();
    assert!(out.contains("B0790") && out.contains("server/routes/pay/route.jsbrain") && out.contains("PAY_INTERNAL_SECRET"), "{out}");
    assert!(ok, "a secret may live only on the host, so this warns and never fails the check: {out}");
    assert!(!out.contains("B0735"), "the contract itself is valid: {out}");

    // Too short counts as undefined: serve refuses it the same way, and the value stays out of the message.
    app.env("PAY_INTERNAL_SECRET", "short-private-value");
    let (_, out) = app.check();
    assert!(out.contains("B0790") && out.contains("at least 32 bytes"), "{out}");
    assert!(!out.contains("short-private-value"), "{out}");

    app.env("PAY_INTERNAL_SECRET", &"k".repeat(32));
    let (ok, out) = app.check();
    assert!(ok && !out.contains("B0790"), "{out}");
}

#[test]
fn a_signature_secret_is_reported_for_every_route_that_lacks_one() {
    let app = App::new();
    app.route("{ method: 'POST', path: '/pay/hook', auth: 'public', body: 'raw', verify: { header: 'x-signature', secret: 'PAY_SIGNING_SECRET' } }");
    app.write("server/routes/refund/route.jsbrain", "<meta server>\nexport default { method: 'POST', path: '/refund/hook', auth: 'public', body: 'raw', verify: { header: 'x-signature', secret: 'REFUND_SIGNING_SECRET' } }\n");
    app.write("server/routes/refund/handler.jsbrain", "<meta server>\nexport default async function handle(ctx) {\n  return { ok: true }\n}\n");
    let (_, out) = app.check();
    assert!(out.contains("PAY_SIGNING_SECRET") && out.contains("REFUND_SIGNING_SECRET"), "one report per route: {out}");
}

#[test]
fn a_route_without_its_secret_is_still_documented() {
    let app = App::new();
    app.route("{ method: 'POST', path: '/pay/hook', auth: 'internal', internal: { secret: 'PAY_INTERNAL_SECRET' } }");
    let out = text(&brain(&app.dir(), &["backend", "schema", "--json"]));
    assert!(out.contains("/pay/hook"), "the contract documents the route whether or not this machine holds its secret: {out}");
    assert!(!out.contains("0000000000000000"), "the stand-in never leaves the validation: {out}");
}

#[test]
fn an_invalid_route_contract_is_a_contract_error_whatever_the_secrets() {
    let app = App::new();
    app.route("{ method: 'FETCH', path: '/pay/hook', auth: 'public' }");
    let (ok, out) = app.check();
    assert!(out.contains("B0735") && out.contains("server/routes/pay/route.jsbrain") && out.contains("method"), "{out}");
    assert!(!ok, "{out}");
}

#[test]
fn a_route_without_a_handler_file_is_a_contract_error() {
    let app = App::new();
    app.route("{ method: 'POST', path: '/pay/hook', auth: 'public' }");
    std::fs::remove_file(app.dir().join("server/routes/pay/handler.jsbrain")).unwrap();
    let (ok, out) = app.check();
    assert!(out.contains("B0735") && out.contains("server/routes/pay/handler.jsbrain"), "{out}");
    assert!(!ok, "{out}");
}

#[test]
fn undefined_webhook_keys_are_reported_and_a_bad_webhook_contract_is_an_error() {
    let app = App::new();
    let config = "server/webhooks/config.jsbrain";
    app.write(config, "<meta server>\nexport default { encryptionKeys: ['BRAIN_WEBHOOK_ENCRYPTION_KEY'], inbound: { orders: { queue: 'order_received', signingKeys: ['BRAIN_WEBHOOK_ORDER_KEY'] } } }\n");
    let (ok, out) = app.check();
    assert!(out.contains("B0790") && out.contains(config) && out.contains("BRAIN_WEBHOOK_ENCRYPTION_KEY"), "{out}");
    assert!(ok, "{out}");

    app.write(config, "<meta server>\nexport default { encryptionKeys: ['literal-private-value'], inbound: {} }\n");
    let (ok, out) = app.check();
    assert!(out.contains("B0735") && out.contains(config), "{out}");
    assert!(!out.contains("literal-private-value"), "a pasted key never reaches a message: {out}");
    assert!(!ok, "{out}");
}

#[test]
fn an_invalid_api_key_contract_is_a_contract_error() {
    let app = App::new();
    let config = "server/api-keys.jsbrain";
    app.write(config, "<meta server>\nexport default { enabled: true, colour: 'red' }\n");
    let (ok, out) = app.check();
    assert!(out.contains("B0735") && out.contains(config), "{out}");
    assert!(!ok, "{out}");
}

#[test]
fn an_invalid_platform_contract_is_a_contract_error() {
    let app = App::new();
    let config = "server/platform.jsbrain";
    app.write(config, "<meta server>\nexport default { colour: 'red' }\n");
    let (ok, out) = app.check();
    assert!(out.contains("B0735") && out.contains(config), "{out}");
    assert!(!ok, "{out}");
}

#[test]
fn invalid_analytics_settings_are_a_contract_error_on_brain_json() {
    let app = App::new();
    let path = app.dir().join("brain.json");
    let source = std::fs::read_to_string(&path).unwrap();
    let write = |analytics: &str| std::fs::write(&path, source.replacen('{', &format!("{{\n  \"analytics\": {analytics},"), 1)).unwrap();
    for (analytics, says) in [("{ \"consent\": \"maybe\" }", "consent"), ("{ \"retentionDays\": 0 }", "retention"), ("{ \"colour\": \"red\" }", "colour"), ("{ \"retentionDays\": \"30\" }", "whole number"), ("7", "true, false or an object")] {
        write(analytics);
        let (ok, out) = app.check();
        assert!(out.contains("B0735") && out.contains("brain.json") && out.contains(says), "{analytics}: {out}");
        assert!(!ok, "{analytics}: {out}");
    }
    write("{ \"consent\": \"required\", \"retentionDays\": 30 }");
    assert!(!app.check().1.contains("B0735"));
    write("false");
    assert!(!app.check().1.contains("B0735"));
}

#[test]
fn a_before_hook_without_its_file_is_a_contract_error() {
    let app = App::new();
    app.write("server/resources/notes/schema.jsbrain", "<meta server>\nexport default { title: 'str!' }\n");
    app.write("server/resources/notes/policy.jsbrain", "<meta server>\nexport default { read: ['admin'], create: ['admin'], update: ['admin'], delete: ['admin'] }\n");
    app.write("server/resources/notes/hooks.jsbrain", "<meta server>\nexport default { before_create: 'validate_note' }\n");
    let (ok, out) = app.check();
    assert!(out.contains("B0735") && out.contains("server/hooks/validate_note.jsbrain"), "{out}");
    assert!(!ok, "{out}");

    app.write("server/hooks/validate_note.jsbrain", "<meta server>\nexport default async function validate(ctx) {\n  return ctx.payload\n}\n");
    let (ok, out) = app.check();
    assert!(ok && !out.contains("B0735"), "{out}");
}
