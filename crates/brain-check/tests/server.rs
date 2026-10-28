//! SPEC §11.1: server code, env vars and secrets (B0701–B0704) + scan_secrets.

use brain_check::{Ctx, Imported};
use std::path::Path;

fn codes_with(path: &str, src: &str, ctx: &Ctx) -> Vec<&'static str> {
    let c = brain_parse::parse(src, Path::new(path)).unwrap_or_else(|d| panic!("{:?}", d));
    let d = brain_check::check(&c, ctx);
    for x in &d {
        assert!(!x.fix.is_empty(), "empty fix: {x}");
    }
    d.into_iter().map(|d| d.code).filter(|c| !c.starts_with("B10")).collect()
}

fn codes(src: &str) -> Vec<&'static str> {
    codes_with("comp/x.brain", src, &Ctx::default())
}

fn server_ctx() -> Ctx {
    let mut ctx = Ctx::default();
    ctx.imports.insert("../lib/db.jsbrain".into(), Imported::Server);
    ctx
}

#[test]
fn server_imports_b0701() {
    let ctx = server_ctx();
    let imp = "import db from '../lib/db.jsbrain'\n";
    // Inside load() and server: functions → fine.
    assert!(codes_with("views/u.brain", &format!("<script>{imp}export async function load(ctx) {{ return {{ u: await db.all() }} }}</script><main><h1>x</h1></main>"), &ctx)
        .iter()
        .all(|c| c.starts_with("B05")));
    assert!(codes_with("comp/x.brain", &format!("<script>{imp}server: async function save(ctx, d) {{ return db.put(d) }}</script><button on:click={{save(1)}}>ok</button>"), &ctx).is_empty());
    // In client code → B0701.
    assert_eq!(codes_with("comp/x.brain", &format!("<script>{imp}const f = () => db.all()</script><p>x</p>"), &ctx), ["B0701"]);
    assert_eq!(codes_with("comp/x.brain", &format!("<script>{imp}</script><p>{{db.name}}</p>"), &ctx), ["B0701"]);
    // A shadowing local is not the import.
    assert!(codes_with("comp/x.brain", &format!("<script>{imp}const f = (db) => db.x</script><p>x</p>"), &ctx).is_empty());
    // A client .jsbrain cannot use it either; a <meta server> one can.
    assert_eq!(codes_with("comp/m.jsbrain", &format!("{imp}export const f = () => db.all()"), &ctx), ["B0701"]);
    assert!(codes_with("comp/m.jsbrain", &format!("<meta server>\n{imp}export const f = () => db.all()"), &ctx).is_empty());
}

#[test]
fn env_vars_b0702() {
    assert_eq!(codes("<script>const k = process.env.STRIPE_KEY</script>{k}"), ["B0702"]);
    assert_eq!(codes("<script>const k = import.meta.env.DB_URL</script>{k}"), ["B0702"]);
    assert_eq!(codes("<script>const n = 'X'\nconst k = import.meta.env[n]</script>{k}"), ["B0702"]);
    assert_eq!(codes("<script>const { A } = process.env</script>{A}"), ["B0702"]);
    assert!(codes("<script>const k = import.meta.env.PUBLIC_API</script>{k}").is_empty());
    assert!(codes("<script>const k = process.env['PUBLIC_API']</script>{k}").is_empty());
    // Server side: anything goes.
    assert!(codes("<script>server: async function f(ctx) { return process.env.SECRET }</script><button on:click={f()}>x</button>").is_empty());
    assert!(codes_with("lib/m.jsbrain", "<meta server>\nexport const url = process.env.DB_URL", &Ctx::default()).is_empty());
}

#[test]
fn secrets_b0703() {
    // Key-shaped values are assembled at runtime so the repository never holds one (GitHub push protection).
    let ant = format!("{}{}", "sk-ant-", "api03-abcdefghijklmnopqrstuv");
    assert_eq!(codes(&format!("<script>const k = '{ant}'</script>{{k}}")), ["B0703"]);
    assert_eq!(codes("<script>const k = `AKIAIOSFODNN7EXAMPLE`</script>{k}"), ["B0703"]);
    assert_eq!(codes("<p>-----BEGIN PRIVATE KEY-----</p>"), ["B0703"]);
    assert_eq!(codes("<img alt=\"x\" src=\"/a.png?t=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abc\">"), ["B0703"]);
    // Look-alikes are fine.
    assert!(codes("<script>const k = 'task-1234 sk-short'</script>{k}").is_empty());
    assert!(codes(&format!("<script>server: async function f(ctx) {{ return '{ant}' }}</script><button on:click={{f()}}>x</button>")).is_empty());
}

#[test]
fn server_functions_b0704() {
    assert_eq!(codes("<script>server: function f(ctx) { return 1 }</script><button on:click={f()}>x</button>"), ["B0704"]);
    assert_eq!(codes("<script>let n = sig(0)\nserver: async function f(ctx) { return n }</script><button on:click={f()}>{n}</button>"), ["B0704"]);
    assert_eq!(codes("<script>const f = () => { server: async function g(ctx) {} }</script><p>x</p>"), ["B0704"]);
    assert_eq!(codes_with("lib/m.jsbrain", "server: async function g(ctx) {}", &Ctx::default()), ["B0704"]);
    // Server functions see imports and each other.
    assert!(codes("<script>server: async function a(ctx) { return 1 }\nserver: async function b(ctx) { return a(ctx) }</script><button on:click={b()}>x</button>").is_empty());
    // Plain labels are still B0108.
    let e = brain_parse::parse("<script>loop: for (x of y) {}</script><p>x</p>", Path::new("comp/x.brain")).unwrap_err();
    assert_eq!(e[0].code, "B0108");
}

#[test]
fn scan_secrets_output() {
    let out = format!("const a=\"ok\";const k=\"{}{}\";/*☃*/const g=\"{}{}\"", "sk-", "proj-ABCDEFGHIJKLMNOPQRSTUVWX", "ghp_", "0123456789abcdef0123456789abcdef0123");
    let kinds: Vec<&str> = brain_check::scan_secrets(&out).into_iter().map(|(_, k)| k).collect();
    assert_eq!(kinds, ["sk-…", "GitHub token"]);
    assert!(brain_check::scan_secrets("mask-aaaaaaaaaaaaaaaaaaaaaaaaaa naïve ☃").is_empty());
}
