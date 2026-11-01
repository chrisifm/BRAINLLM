//! `.brain_env` truth: optional keys, a doctor that does not write versioned files, and leak verdicts that wait for
//! in-flight calls. BRAIN_RED_BIN runs the same assertions against an older binary to show they fail there.
#![cfg(unix)]
use std::{fs, path::{Path, PathBuf}, process::{Command, Stdio}, sync::atomic::{AtomicU64, Ordering}};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct App(PathBuf);
impl Drop for App { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn bin() -> String { std::env::var("BRAIN_RED_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_brain").to_string()) }
fn brain(root: &Path, args: &[&str]) -> (bool, String) {
    let o = Command::new(bin()).args(args).current_dir(root).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap();
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}
fn app(label: &str) -> App {
    let tmp = std::env::temp_dir().join(format!("brain-env-truth-{label}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&tmp).unwrap();
    let (ok, out) = brain(&tmp, &["new", "app"]);
    assert!(ok, "{out}");
    let root = tmp.join("app");
    fs::remove_dir_all(root.join("node_modules")).ok();
    App(root)
}
fn git(root: &Path, args: &[&str]) -> String {
    let o = Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@example.test"]).args(args).current_dir(root).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).to_string()
}
fn chrome(out: &str) -> bool { !(out.contains("Chrome") && out.contains("not found")) }
const READS: &str = "<meta server>\nexport default {\n  limit: () => Number(process.env.CALL_LIMIT) || 800,\n  gateway: (ctx) => ctx.env.GATEWAY_KEY ? 1 : 0,\n  must: () => process.env.REQUIRED_TOKEN,\n}\n";

#[test]
fn optional_keys_pass_the_build_and_required_empty_keys_do_not() {
    let a = app("optional");
    let env = fs::read_to_string(a.0.join(".brain_env")).unwrap();
    fs::write(a.0.join(".brain_env"), format!("CALL_LIMIT?=\n{env}")).unwrap();
    fs::write(a.0.join("views/uses.brain"), "<meta seo title=\"Uses\" description=\"Uses a limit from the environment, or a default.\">\n<script>\n  load: async function () { return { n: process.env.CALL_LIMIT ?? 800 } }\n</script>\n<main><h1>{n}</h1></main>\n").ok();
    let (_, out) = brain(&a.0, &["build", "--progress=off"]);
    assert!(!out.contains("CALL_LIMIT") && !out.contains("B0724"), "an optional key does not stop the build: {out}");
    let env2 = fs::read_to_string(a.0.join(".brain_env")).unwrap().replacen("CALL_LIMIT?=", "CALL_LIMIT=", 1);
    fs::write(a.0.join(".brain_env"), env2).unwrap();
    let (ok, out) = brain(&a.0, &["build", "--progress=off"]);
    assert!(!ok && out.contains("B0720") && out.contains("CALL_LIMIT") && out.contains("CALL_LIMIT?="), "a required empty key still stops it: {out}");
}

#[test]
fn b0721_proposes_optional_only_for_tolerant_reads() {
    let a = app("b0721");
    fs::create_dir_all(a.0.join("lib")).unwrap();
    fs::write(a.0.join("lib/settings.jsbrain"), READS).unwrap();
    let (_, out) = brain(&a.0, &["check", "--json"]);
    let v: serde_json::Value = serde_json::from_str(out.lines().find(|l| l.starts_with('{') || l.starts_with('[')).unwrap()).unwrap();
    let diags = v.as_array().cloned().or_else(|| v["diags"].as_array().cloned()).unwrap();
    let fix = |key: &str| diags.iter().find(|d| d["code"] == "B0721" && d["msg"].as_str().unwrap().contains(key)).map(|d| d["fix"].as_str().unwrap().to_string()).unwrap_or_else(|| panic!("{key}: {out}"));
    for key in ["CALL_LIMIT", "GATEWAY_KEY"] {
        let f = fix(key);
        assert!(f.contains(&format!("`{key}?=`")) && !f.contains("value per environment"), "{f}");
    }
    let f = fix("REQUIRED_TOKEN");
    assert!(f.contains("`REQUIRED_TOKEN=`") && f.contains("value per environment"), "{f}");
    // the mechanical fix writes those forms, and then nothing is reported for them
    let (_, _) = brain(&a.0, &["fix"]);
    let env = fs::read_to_string(a.0.join(".brain_env")).unwrap();
    assert!(env.contains("CALL_LIMIT?=\n") && env.contains("GATEWAY_KEY?=\n") && env.contains("REQUIRED_TOKEN=\n"), "{env}");
    let (_, out) = brain(&a.0, &["check"]);
    assert!(!out.contains("B0721"), "{out}");
    let example = fs::read_to_string(a.0.join(".brain_env.example")).unwrap();
    assert!(example.contains("CALL_LIMIT?=\n") && example.contains("REQUIRED_TOKEN=\n"), "{example}");
}

#[test]
fn doctor_writes_no_versioned_file_and_fix_keeps_every_key_the_code_reads() {
    let a = app("nowrite");
    fs::create_dir_all(a.0.join("lib")).unwrap();
    fs::write(a.0.join("lib/settings.jsbrain"), READS).unwrap();
    fs::write(a.0.join(".brain_env.example"), "# by hand\nOLD_KEY=\n").unwrap();
    git(&a.0, &["init", "-q"]);
    git(&a.0, &["add", "-A"]);
    git(&a.0, &["commit", "-qm", "base"]);
    let (_, out) = brain(&a.0, &["doctor"]);
    assert_eq!(git(&a.0, &["status", "--porcelain"]), "", "doctor left the worktree dirty: {out}");
    assert!(out.contains("B0727") && out.contains("REQUIRED_TOKEN") && out.contains("brain doctor --fix"), "{out}");
    assert!(fs::read_to_string(a.0.join(".brain_env.example")).unwrap().contains("OLD_KEY"), "untouched");
    let (_, check) = brain(&a.0, &["check"]);
    assert_eq!(git(&a.0, &["status", "--porcelain"]), "", "check left the worktree dirty: {check}");
    // negative: the explicit command writes it, with what the code reads and without values
    let (_, _) = brain(&a.0, &["doctor", "--fix"]);
    let example = fs::read_to_string(a.0.join(".brain_env.example")).unwrap();
    assert!(example.contains("REQUIRED_TOKEN=\n") && !example.contains("OLD_KEY"), "{example}");
    let (_, out) = brain(&a.0, &["doctor"]);
    assert!(!out.contains("B0727"), "in sync after --fix: {out}");
}

#[test]
fn startup_settings_are_reported_before_start_without_their_values() {
    let a = app("startup");
    let env = fs::read_to_string(a.0.join(".brain_env")).unwrap();
    fs::write(a.0.join(".brain_env"), format!("BRAIN_MAIL_RESEND_KEY=re_0123456789abcdef\nBRAIN_MAIL_FROM=bad-sender-value\n{env}")).unwrap();
    let (_, out) = brain(&a.0, &["check"]);
    assert!(out.contains("B0760") && out.contains("BRAIN_MAIL_FROM") && !out.contains("bad-sender-value") && !out.contains("re_0123456789abcdef"), "{out}");
    let serve = |env_text: &str| {
        fs::write(a.0.join(".brain_env"), env_text).unwrap();
        brain(&a.0, &["backend", "serve", "--port", "0"])
    };
    let (ok, out) = serve("DATABASE_URL=postgres://u:p@127.0.0.1:1/none\nBRAIN_MAIL_RESEND_KEY=re_0123456789abcdef\nBRAIN_MAIL_FROM=bad-sender-value\n");
    assert!(!ok && out.contains("B0760") && out.contains("fix:") && !out.contains("bad-sender-value"), "{out}");
    // negative: a friendly sender gets past the settings, to the (absent) database
    let (ok, out) = serve("DATABASE_URL=postgres://u:p@127.0.0.1:1/none\nBRAIN_MAIL_RESEND_KEY=re_0123456789abcdef\nBRAIN_MAIL_FROM=Shop Team <hello@shop.example>\n");
    assert!(!ok && !out.contains("B0760") && !out.contains("Resend requires") && out.contains("B0762"), "{out}");
}

fn component(a: &App, name: &str, body: &str) {
    fs::write(a.0.join(format!("comp/{name}.brain")), body).unwrap();
    let cap: String = name.split('-').map(|w| w[..1].to_uppercase() + &w[1..]).collect();
    fs::write(a.0.join(format!("comp/{name}.test.brain")), format!("<script>\n  import {cap} from './{name}.brain'\n  test('mounts', async () => {{ await mount({cap}, {{}}); expect(text('p')).toBeTruthy() }})\n</script>\n")).unwrap();
}

#[test]
fn doctor_waits_for_in_flight_calls_and_a_suspicion_only_fails_when_opted_in() {
    let a = app("leaks");
    fs::create_dir_all(a.0.join("comp")).unwrap();
    component(&a, "slow-call", "<script>\n  server: async function slow(ctx) { await new Promise((r) => setTimeout(r, 20000)); return 1 }\n  let n = sig(0)\n  mounted(() => { slow().then((v) => { n = v }) })\n</script>\n<p>{n}</p>\n");
    let (ok, out) = brain(&a.0, &["doctor", "--json"]);
    if !chrome(&out) { eprintln!("skipped: no Chrome"); return; }
    let report: serde_json::Value = serde_json::from_str(out.lines().rev().find(|l| l.starts_with('{')).unwrap()).unwrap();
    assert_eq!(report["heap"]["leaks"].as_array().map(|l| l.iter().filter(|x| x["code"] == "B0601").count()), Some(0), "a pending call is no leak: {}", report["heap"]);
    assert!(report["test"]["errors"].as_array().is_some_and(|e| e.iter().all(|x| !x["msg"].as_str().unwrap_or("").contains("abort"))), "{}", report["test"]["errors"]);
    let _ = ok;
    // a real retention: suspected, reported with the fix, and not a failure by default
    fs::create_dir_all(a.0.join("lib")).unwrap();
    fs::write(a.0.join("lib/cache.jsbrain"), "const all = []\nexport default { keep: (x) => all.push(x) }\n").unwrap();
    component(&a, "kept", "<script>\n  import cache from '../lib/cache.jsbrain'\n  let el = ref()\n  mounted(() => { cache.keep(el) })\n</script>\n<p bind:ref={el}>Kept</p>\n");
    let (_, out) = brain(&a.0, &["doctor", "--json"]);
    let report: serde_json::Value = serde_json::from_str(out.lines().rev().find(|l| l.starts_with('{')).unwrap()).unwrap();
    let leak = report["heap"]["leaks"].as_array().unwrap().iter().find(|x| x["code"] == "B0601").cloned().unwrap_or_else(|| panic!("{}", report["heap"]));
    assert!(leak["fix"].as_str().unwrap().contains("in-flight"), "{leak}");
    assert!(!report["failed"].as_array().unwrap().iter().any(|g| g == "leaks" || g == "tests"), "{}", report["failed"]);
    let (_, text) = brain(&a.0, &["doctor"]);
    assert!(text.contains("suspected (a warning") && !text.contains("there are errors"), "{text}");
    // opted in, the same suspicion fails and the summary says which gate
    let mut config: serde_json::Value = serde_json::from_str(&fs::read_to_string(a.0.join("brain.json")).unwrap()).unwrap();
    config["doctor"] = serde_json::json!({ "failOnSuspectedLeaks": true });
    fs::write(a.0.join("brain.json"), config.to_string()).unwrap();
    let (ok, text) = brain(&a.0, &["doctor"]);
    assert!(!ok && text.contains("✗ failed:") && text.contains("leaks"), "{text}");
}

#[test]
fn a_session_set_the_backend_would_refuse_warns_until_the_app_owns_the_session() {
    let app = app("own-session");
    fs::write(app.0.join("server/own.jsbrain"), "export function enter(ctx) { ctx.session.set({ name: 'a' }) }\n").unwrap();
    let declared = fs::read_to_string(app.0.join(".brain_env")).unwrap();
    fs::write(app.0.join(".brain_env"), format!("{declared}\nBRAIN_BACKEND_URL=http://127.0.0.1:4100\n")).unwrap();
    let (_, out) = brain(&app.0, &["check", "--json"]);
    assert!(out.contains("B0786") && out.contains("server/own.jsbrain"), "{out}");
    assert!(!out.contains("server/auth.jsbrain\",\"code\":\"B0786"), "the generated login branches on the backend: {out}");
    fs::write(app.0.join(".brain_env"), format!("{declared}\nBRAIN_BACKEND_URL=http://127.0.0.1:4100\nBRAIN_BACKEND_SESSION=app\n")).unwrap();
    let (_, out) = brain(&app.0, &["check", "--json"]);
    assert!(!out.contains("B0786"), "{out}");
}

fn with_node_env(root: &Path, args: &[&str], node_env: Option<&str>) -> (bool, String) {
    let mut command = Command::new(bin());
    command.args(args).current_dir(root).env("NO_COLOR", "1").env_remove("DATABASE_URL").env_remove("NODE_ENV").stdin(Stdio::null());
    if let Some(value) = node_env { command.env("NODE_ENV", value); }
    let o = command.output().unwrap();
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}
const PRODUCTION_ONLY_DB: &str = "[production]\nDATABASE_URL=postgres://u:synthetic-password@127.0.0.1:1/app\n";

// A deployed shell that forgot --env must learn that, not that the database is missing from a section it never asked for.
#[test]
fn a_production_shell_without_env_is_told_about_env_on_every_database_command() {
    let a = app("node-env");
    fs::write(a.0.join(".brain_env"), PRODUCTION_ONLY_DB).unwrap();
    for args in [&["jobs", "work", "mail"][..], &["jobs", "stats"], &["backend", "migrate"], &["backend", "serve"], &["analytics", "report"], &["media", "list", "--owner", "x"]] {
        let (ok, text) = with_node_env(&a.0, args, Some("production"));
        assert!(!ok && text.contains("B0785") && text.contains("--env production") && !text.contains("B0762"), "{args:?}: {text}");
        assert!(!text.contains("synthetic-password"), "{text}");
    }
}

#[test]
fn a_database_url_in_another_section_is_named_and_an_equals_env_counts_as_stated() {
    let a = app("other-section");
    fs::write(a.0.join(".brain_env"), PRODUCTION_ONLY_DB).unwrap();
    let (_, text) = with_node_env(&a.0, &["jobs", "stats"], None);
    assert!(text.contains("B0762") && text.contains("[production]") && text.contains("--env production") && !text.contains("synthetic-password"), "{text}");
    let (_, text) = with_node_env(&a.0, &["jobs", "stats", "--env=production"], Some("production"));
    assert!(!text.contains("B0785"), "--env=production states the environment: {text}");
    // negative: nothing to point at when no section has it
    fs::write(a.0.join(".brain_env"), "PUBLIC_SITE_NAME=app\n").unwrap();
    let (_, text) = with_node_env(&a.0, &["jobs", "stats"], None);
    assert!(text.contains("B0762") && !text.contains("[production]"), "{text}");
}

fn with_stripe_route(a: &App, env: &str) {
    let dir = a.0.join("server/routes/payments");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("route.jsbrain"), "<meta server>\nexport default { method: 'POST', path: '/hooks/payments', verify: { scheme: 'timestamped', header: 'X-Signature', secret: 'PAYMENTS_WEBHOOK_SECRET' } }\n").unwrap();
    fs::write(dir.join("handler.jsbrain"), "<meta server>\nexport default async function handle(ctx) { return { ok: true } }\n").unwrap();
    fs::write(a.0.join(".brain_env"), env).unwrap();
}

// A supervisor restarts a process that exits with a failure: a missing secret would loop forever, each time looking like a
// crash. It stops before the first connection, with its code, and with the exit status supervisors are told not to restart.
#[test]
fn a_missing_route_secret_stops_serve_once_with_its_code_and_a_no_restart_status() {
    let a = app("config-exit");
    for secret in ["", "PAYMENTS_WEBHOOK_SECRET=\n", "PAYMENTS_WEBHOOK_SECRET?=\n", "PAYMENTS_WEBHOOK_SECRET=short-value\n"] {
        // nothing listens on this port: reaching the database would take a minute and report it instead
        with_stripe_route(&a, &format!("DATABASE_URL=postgres://u@127.0.0.1:1/none\n{secret}"));
        let started = std::time::Instant::now();
        let o = Command::new(bin()).args(["backend", "serve", "--env", "development", "--port", "0"]).current_dir(&a.0).env("NO_COLOR", "1").env_remove("DATABASE_URL").env_remove("NODE_ENV").env_remove("PAYMENTS_WEBHOOK_SECRET").stdin(Stdio::null()).output().unwrap();
        let text = String::from_utf8_lossy(&o.stderr).to_string();
        assert_eq!(o.status.code(), Some(78), "{secret:?}: {text}");
        assert!(text.contains("B0790") && text.contains("PAYMENTS_WEBHOOK_SECRET") && text.contains("fix:") && !text.contains("short-value"), "{text}");
        assert!(started.elapsed().as_secs() < 20, "it must not wait for the database: {text}");
    }
    // a one-shot command keeps the ordinary failure status
    with_stripe_route(&a, "DATABASE_URL=not-a-url\n");
    let (ok, out) = brain(&a.0, &["backend", "migrate", "--env", "development"]);
    assert!(!ok && out.contains("B0762"), "{out}");
    let o = Command::new(bin()).args(["backend", "migrate", "--env", "development"]).current_dir(&a.0).stdin(Stdio::null()).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}

// An intentionally empty key has one way out, and it has to be named everywhere the author may look, with its limit.
#[test]
fn the_empty_key_guidance_names_the_optional_form_and_what_it_does_not_cover() {
    let a = app("optional-guidance");
    let (_, explained) = brain(&a.0, &["explain", "B0720"]);
    assert!(explained.contains("?="), "{explained}");
    fs::write(a.0.join(".brain_env"), "EMPTY_KEY=\n").unwrap();
    let (ok, out) = brain(&a.0, &["build", "--progress=off"]);
    assert!(!ok && out.contains("EMPTY_KEY?=") && out.contains("B0790"), "{out}");
    with_stripe_route(&a, "DATABASE_URL=postgres://u@127.0.0.1:1/none\nPAYMENTS_WEBHOOK_SECRET?=\n");
    let (ok, out) = brain(&a.0, &["backend", "serve", "--env", "development", "--port", "0"]);
    assert!(!ok && out.contains("B0790") && out.contains("?="), "`KEY?=` does not disable the route: {out}");
}

// The worker a deploy starts by hand: the message has to say which file it read and how to pass the environment.
#[test]
fn a_missing_database_url_names_the_file_it_read_and_how_to_pass_the_environment() {
    let a = app("names-the-file");
    fs::write(a.0.join(".brain_env"), "PUBLIC_SITE_NAME=app\n").unwrap();
    let (_, text) = with_node_env(&a.0, &["jobs", "work", "mail"], None);
    assert!(text.contains("B0762") && text.contains(".brain_env") && text.contains("--env"), "{text}");
    fs::write(a.0.join(".brain_env"), "[staging]\nDATABASE_URL=postgres://synthetic-password@127.0.0.1:1/x\n").unwrap();
    let (_, text) = with_node_env(&a.0, &["jobs", "stats"], None);
    assert!(text.contains("[staging]") && text.contains("--env staging") && !text.contains("synthetic-password"), "any section is named: {text}");
    fs::remove_file(a.0.join(".brain_env")).unwrap();
    let (_, text) = with_node_env(&a.0, &["jobs", "stats"], None);
    assert!(text.contains("does not exist") && text.contains(".brain_env"), "the wrong folder shows: {text}");
}
