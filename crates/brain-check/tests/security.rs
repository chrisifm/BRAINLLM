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
    diags_with(path, src, ctx).into_iter().map(|d| d.code).filter(|c| matches!(*c, "B0122" | "B0123" | "B0210" | "B0213" | "B0215" | "B0710" | "B0711" | "B0713" | "B0737" | "B1310" | "B1311")).collect()
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
fn b0213_covers_every_url_attribute_and_each_srcset_candidate() {
    assert_eq!(codes("<svg><a xlink:href=\"javascript:alert(1)\"><text>x</text></a></svg>"), ["B0213"]);
    assert_eq!(codes("<blockquote cite=\"javascript:x\">q</blockquote>"), ["B0213"]);
    assert_eq!(codes("<object data=\"data:text/html,x\" title=\"o\"></object>"), ["B0213"]);
    assert_eq!(codes("<a href=\"/a\" ping=\"/ok javascript:x\">x</a>"), ["B0213"]);
    assert_eq!(codes("<img alt=\"x\" width=\"1\" height=\"1\" srcset=\"a.jpg 1x, javascript:x 2x\">"), ["B0213"]);
    assert!(codes("<img alt=\"x\" width=\"1\" height=\"1\" srcset=\"a.jpg 1x, data:image/png;base64,AA 2x\">").is_empty());
    assert!(codes("<div data=\"javascript-free text\">x</div>").is_empty(), "`data` is a URL on <object> only");
}

#[test]
fn b0210_bound_srcdoc_needs_an_opaque_sandbox() {
    let comp = |attrs: &str| format!("<script>\nlet body = sig('')\n</script>\n<iframe title=\"t\" {attrs}></iframe>");
    assert_eq!(codes(&comp("srcdoc={body}")), ["B0210"]);
    assert_eq!(codes(&comp("srcdoc=\"<p>{body}</p>\"")), ["B0210"]);
    assert_eq!(codes(&comp("srcdoc={`<p>${body}</p>`}")), ["B0210"]);
    assert_eq!(codes(&comp("sandbox=\"allow-scripts allow-same-origin\" srcdoc={body}")), ["B0210"]);
    assert!(codes(&comp("sandbox srcdoc={body}")).is_empty());
    assert!(codes(&comp("sandbox=\"allow-scripts\" srcdoc={body}")).is_empty());
    assert!(codes(&comp("srcdoc=\"<p>static</p>\"")).is_empty());
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

#[test]
fn b0737_credentials_in_the_signed_session_cookie() {
    let flagged = [
        "ctx.session.set({ id: user.id, access_token: t })",
        "ctx.session.set({ accessToken })",
        "ctx.session.set({ user: { password: p } })",
        "ctx.session.set(refreshToken)",
        "ctx.session.set({ apiKey: k })",
    ];
    for call in flagged {
        let d = diags_with("server/auth.jsbrain", &format!("export async function login(ctx, t, p, k, accessToken, refreshToken, user) {{ {call} }}"), &Ctx::default());
        let b = d.iter().find(|d| d.code == "B0737").unwrap_or_else(|| panic!("{call}: {d:?}"));
        assert!(b.is_warning() && b.fix.contains("setSecret"), "{b:?}");
    }
    let clean = [
        "ctx.session.set({ id: user.id, email: user.email, role: user.role })",
        "ctx.session.set(user)",
        "ctx.session.set({ tokens_used: 3, passwordless: true, monkey: 1 })",
        "ctx.session.setSecret({ token: t })",
    ];
    for call in clean {
        let d = diags_with("server/auth.jsbrain", &format!("export async function login(ctx, t, user) {{ {call} }}"), &Ctx::default());
        assert!(!d.iter().any(|d| d.code == "B0737"), "{call}: {d:?}");
    }
}

// B1311 warned on the very escape its fix proposes, alone or concatenated with literal parts.
#[test]
fn b1311_accepts_the_escape_its_fix_proposes() {
    let wrap = |expr: &str| format!("<script>let q = sig('')\nlet re = drv(() => {expr})</script><input aria-label=\"q\" bind:value={{q}} /><p>{{re}}</p>");
    for ok in [
        r"new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))",
        r"new RegExp('\\b' + q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '\\b', 'i')",
        r"new RegExp(`^${q.replaceAll(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`)",
        r"RegExp(q.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&'))",
    ] {
        assert!(codes(&wrap(ok)).is_empty(), "{ok}: {:?}", codes(&wrap(ok)));
    }
    for bad in [
        r"new RegExp('\\b' + q + '\\b')",
        r"new RegExp(q.replace(/[.*]/g, '\\$&'))",
        r"new RegExp(q.replace(/[.*+?^${}()|[\]\\]/, '\\$&'))",
        r"new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, ''))",
        r"new RegExp(`^${q}$`)",
    ] {
        assert_eq!(codes(&wrap(bad)), ["B1311"], "{bad}");
    }
}

// B0710 rejected `redirect(helper(x), 301)` even when the helper only returns same-origin paths.
#[test]
fn b0710_accepts_helpers_that_only_return_same_origin_paths() {
    let module = |helper: &str, arg: &str| format!("{helper}\nexport default (ctx) => ctx.user ? true : redirect({arg}, 301)");
    let ok = [
        ("const pathOf = (r) => '/items/' + r.slug", "pathOf(ctx.params)"),
        ("function pathOf(r) { if (r.old) return `/archive/${r.slug}`\n  return '/items/' + r.slug }", "pathOf(ctx.params)"),
        ("const base = (r) => '/items/' + r.slug\nconst pathOf = (r) => base(r) + '/edit'", "pathOf(ctx.params)"),
        ("const pathOf = (r) => r.old ? '/archive' : '/items/' + r.slug", "pathOf(ctx.params) + '?from=old'"),
    ];
    for (helper, arg) in ok {
        assert!(codes_in("views/_guard.jsbrain", &module(helper, arg), &Ctx::default()).is_empty(), "{helper}");
    }
    let bad = [
        ("const pathOf = (r) => r.next", "pathOf(ctx.query)"),
        ("const pathOf = (r) => r.old ? '//evil.example' : '/items'", "pathOf(ctx.params)"),
        ("const pathOf = (r) => `${r.host}/items`", "pathOf(ctx.query)"),
        ("const pathOf = (r) => '/items'", "other(ctx.query)"),
    ];
    for (helper, arg) in bad {
        assert_eq!(codes_in("views/_guard.jsbrain", &module(helper, arg), &Ctx::default()), ["B0710"], "{helper}");
    }
    // Helpers imported from a `.jsbrain` module, default or named.
    let dir = std::env::temp_dir().join(format!("brain-b0710-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::create_dir_all(dir.join("views")).unwrap();
    std::fs::write(dir.join("app.brain"), "<slot />").unwrap();
    std::fs::write(dir.join("lib/paths.jsbrain"), "const item = (r) => '/items/' + r.slug\nconst raw = (r) => r.next\nexport default { item, raw, edit: (r) => item(r) + '/edit' }").unwrap();
    let check = |src: &str| {
        let file = dir.join("views/_guard.jsbrain");
        std::fs::write(&file, src).unwrap();
        let c = brain_parse::parse(src, Path::new("views/_guard.jsbrain")).unwrap();
        let (ctx, _) = brain_check::load_ctx(&file, &c);
        brain_check::check(&c, &ctx).into_iter().map(|d| d.code).filter(|c| *c == "B0710").collect::<Vec<_>>()
    };
    assert!(check("import paths from '../lib/paths.jsbrain'\nexport default (ctx) => ctx.user ? true : redirect(paths.item(ctx.params), 301)").is_empty());
    assert!(check("import paths from '../lib/paths.jsbrain'\nexport default (ctx) => ctx.user ? true : redirect(paths.edit(ctx.params))").is_empty());
    assert_eq!(check("import paths from '../lib/paths.jsbrain'\nexport default (ctx) => ctx.user ? true : redirect(paths.raw(ctx.query))"), ["B0710"]);
    std::fs::remove_dir_all(&dir).ok();
}

// B0710 accepted `redirect('/' + runtime)`: the runtime part can start with `/` or `\`, which turns the path into a
// protocol-relative URL (`//evil.example`, `/\evil.example`) that leaves the site.
#[test]
fn b0710_rejects_a_bare_slash_followed_by_a_runtime_value() {
    let guard = |arg: &str| format!("export default (ctx) => ctx.user ? true : redirect({arg})");
    for arg in [
        "'/' + ctx.query.next",
        "`/${ctx.query.next}`",
        "'/' + '/evil.example'",
        "'/\\\\evil.example'",
        "'/\\t/evil.example'",
        "'https://allowed.example' + ctx.query.rest",
    ] {
        let ctx = Ctx { redirect_hosts: vec!["allowed.example".into()], ..Ctx::default() };
        assert_eq!(codes_in("views/_guard.jsbrain", &guard(arg), &ctx), ["B0710"], "{arg}");
    }
    let module = |helper: &str, arg: &str| format!("{helper}\nexport default (ctx) => ctx.user ? true : redirect({arg})");
    for (helper, arg) in [
        ("const pathOf = (r) => '/' + r.slug", "pathOf(ctx.query)"),
        ("const pathOf = (r) => r.home ? '/' : '/items/' + r.slug", "pathOf(ctx.query) + ctx.query.rest"),
    ] {
        assert_eq!(codes_in("views/_guard.jsbrain", &module(helper, arg), &Ctx::default()), ["B0710"], "{helper}");
    }
    for arg in ["'/'", "'/login'", "'/items/' + ctx.params.slug", "`/items/${ctx.params.slug}`", "'https://allowed.example/' + ctx.query.rest", "ctx.query.home ? '/' : '/login'"] {
        let ctx = Ctx { redirect_hosts: vec!["allowed.example".into()], ..Ctx::default() };
        assert!(codes_in("views/_guard.jsbrain", &guard(arg), &ctx).is_empty(), "{arg}");
    }
}
