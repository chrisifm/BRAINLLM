//! #93 (INTERNALS §31): proactive protections in the checker.

use brain_check::Ctx;
use std::path::Path;

fn diags_with(path: &str, src: &str, ctx: &Ctx) -> Vec<brain_parse::Diag> {
    let c = brain_parse::parse(src, Path::new(path)).unwrap_or_else(|d| panic!("{d:?}"));
    let d = brain_check::check(&c, ctx);
    for x in &d {
        assert!(!x.fix.is_empty() && !x.msg.is_empty(), "{x}");
    }
    d
}

/// Only the §31 codes (snippets touch `document` etc. loosely).
fn codes_in(path: &str, src: &str, ctx: &Ctx) -> Vec<&'static str> {
    diags_with(path, src, ctx).into_iter().map(|d| d.code).filter(|c| matches!(*c, "B0122" | "B0123" | "B0213" | "B0215" | "B0710" | "B0711" | "B0713" | "B1310" | "B1311")).collect()
}

fn codes(src: &str) -> Vec<&'static str> {
    codes_in("comp/x.brain", src, &Ctx::default())
}

#[test]
fn b0213_dangerous_static_urls() {
    assert_eq!(codes("<a href=\"javascript:alert(1)\">x</a>"), ["B0213"]);
    assert_eq!(codes("<a href=\" JavaScript:void(0)\">x</a>"), ["B0213"]);
    assert_eq!(codes("<iframe title=\"t\" src=\"data:text/html,<b>x</b>\"></iframe>"), ["B0213"]);
    assert_eq!(codes("<form action={'javascript:go()'}><button>x</button></form>"), ["B0213"]);
    assert!(codes("<img alt=\"x\" src=\"data:image/png;base64,AAAA\"><a href=\"/home\">x</a><a href=\"https://example.com\">y</a>").is_empty());
}

#[test]
fn b0215_noopener_is_added() {
    let d = diags_with("comp/x.brain", "<a href=\"https://example.com\" target=\"_blank\">x</a>", &Ctx::default());
    let b = d.iter().find(|d| d.code == "B0215").expect("B0215");
    assert!(b.is_info() && b.edits[0].text == " rel=\"noopener noreferrer\"", "{b:?}");
    let d = diags_with("comp/x.brain", "<a href=\"https://example.com\" target=\"_blank\" rel=\"external\">x</a>", &Ctx::default());
    assert!(d.iter().find(|d| d.code == "B0215").is_some_and(|b| b.edits[0].text == "rel=\"external noopener noreferrer\""), "{d:?}");
    assert!(codes("<a href=\"https://example.com\" target=\"_blank\" rel=\"noopener\">x</a>").is_empty());
    // The compiler applies it.
    let o = brain_codegen_free_check("<a href=\"https://example.com\" target=\"_blank\">x</a>");
    assert!(o, "normalized source must be clean");
}

fn brain_codegen_free_check(src: &str) -> bool {
    let c = brain_parse::parse(src, Path::new("comp/x.brain")).unwrap();
    let d = brain_check::check(&c, &Ctx::default());
    let edits: Vec<_> = d.iter().filter(|d| d.is_info()).flat_map(|d| d.edits.clone()).collect();
    let fixed = brain_parse::Diag::apply(src, &edits).unwrap();
    codes(&fixed).is_empty()
}

#[test]
fn b0122_b0123_supply_chain() {
    assert_eq!(codes("<script>import x from 'https://cdn.example.com/x.js'</script><p>{x}</p>"), ["B0122"]);
    let app = |s: &str| format!("<html lang=\"en\"><head>{s}</head><body><slot /></body></html>");
    assert_eq!(codes_in("app.brain", &app("<script src=\"https://cdn.example.com/a.js\"></script>"), &Ctx::default()), ["B0123"]);
    assert!(codes_in("app.brain", &app("<script src=\"https://cdn.example.com/a.js\" integrity=\"sha384-abc\" crossorigin=\"anonymous\"></script>"), &Ctx::default()).is_empty());
    assert!(codes_in("app.brain", &app("<script src=\"/local.js\"></script>"), &Ctx::default()).is_empty());
}

#[test]
fn b0710_open_redirect() {
    let guard = |arg: &str| format!("export default (ctx) => ctx.user ? true : redirect({arg})");
    assert!(codes_in("views/_guard.jsbrain", &guard("'/login'"), &Ctx::default()).is_empty());
    assert_eq!(codes_in("views/_guard.jsbrain", &guard("'https://evil.example'"), &Ctx::default()), ["B0710"]);
    assert_eq!(codes_in("views/_guard.jsbrain", &guard("ctx.query.next"), &Ctx::default()), ["B0710"]);
    assert_eq!(codes_in("views/_guard.jsbrain", &guard("'//evil.example/x'"), &Ctx::default()), ["B0710"]);
    let ctx = Ctx { redirect_hosts: vec!["login.example.com".into()], ..Ctx::default() };
    assert!(codes_in("views/_guard.jsbrain", &guard("'https://login.example.com/sso'"), &ctx).is_empty());
}

#[test]
fn b0711_ssrf() {
    let rpc = |url: &str| format!("<script>server: async function get(ctx, id) {{ return fetch({url}) }}</script><button on:click={{get(1)}}>x</button>");
    assert_eq!(codes(&rpc("id")), ["B0711"]);
    assert_eq!(codes(&rpc("`${id}/x`")), ["B0711"]);
    assert!(codes(&rpc("`https://api.example.com/users/${id}`")).is_empty());
    assert!(codes(&rpc("'https://api.example.com/x'")).is_empty());
    let view = "<meta seo title=\"T\" description=\"D\" canonical=\"/x\"><script>export async function load(ctx) { return { r: await fetch(ctx.query.url) } }</script><main><h1>x</h1></main>";
    assert_eq!(codes_in("views/x.brain", view, &Ctx::default()), ["B0711"]);
    // Client fetches are the browser's business.
    assert!(codes("<script>const f = (u) => fetch(u)</script><button on:click={f('/a')}>x</button>").is_empty());
}

#[test]
fn b0713_session_cookie_from_the_browser() {
    assert_eq!(codes("<script>const f = (t) => { document.cookie = `session=${t}` }</script><button on:click={f(1)}>x</button>"), ["B0713"]);
    assert_eq!(codes("<script>const f = () => { document.cookie = '__Host-id=1; Path=/' }</script><button on:click={f()}>x</button>"), ["B0713"]);
    assert!(codes("<script>const f = () => { document.cookie = 'theme=dark' }</script><button on:click={f()}>x</button>").is_empty());
}

#[test]
fn b1310_b1311_warnings() {
    assert_eq!(codes("<script>let rows = sig(Array.from({ length: 50000 }, (_, i) => ({ id: i })))</script><ul>{#each rows as r (r.id)}<li>{r.id}</li>{/each}</ul>"), ["B1310"]);
    assert!(codes("<script>let rows = sig(Array.from({ length: 50 }, (_, i) => ({ id: i })))</script><ul>{#each rows as r (r.id)}<li>{r.id}</li>{/each}</ul>").is_empty());
    assert_eq!(codes("<script>let q = sig('')\nlet re = drv(() => new RegExp(q))</script><input aria-label=\"q\" bind:value={q} /><p>{re}</p>"), ["B1311"]);
    assert!(codes("<script>let q = sig('')\nconst escape = (s) => s\nlet re = drv(() => new RegExp(escape(q), 'i'))\nconst digits = new RegExp('[0-9]+')</script><input aria-label=\"q\" bind:value={q} /><p>{re}{digits}</p>").is_empty());
    let d = diags_with("comp/x.brain", "<script>let q = sig('')\nlet re = drv(() => new RegExp(q))</script><input aria-label=\"q\" bind:value={q} /><p>{re}</p>", &Ctx::default());
    assert!(d.iter().all(|d| d.code != "B1311" || d.is_warning()));
}
