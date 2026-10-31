//! Mistakes that used to compile with no diagnostic: stray delimiters in markup, `await` where no async function
//! encloses it, credentials in readable places, and SEO rules on files that are not routed.

use brain_check::Ctx;
use std::path::Path;

fn diags(path: &str, src: &str) -> Vec<brain_parse::Diag> {
    let c = brain_parse::parse(src, Path::new(path)).unwrap_or_else(|d| panic!("{d:?}"));
    let d = brain_check::check(&c, &Ctx::default());
    for x in &d {
        assert!(!x.fix.is_empty() && !x.msg.is_empty(), "{x}");
    }
    d
}

fn count(d: &[brain_parse::Diag], code: &str) -> usize {
    d.iter().filter(|x| x.code == code).count()
}

fn markup(body: &str) -> Vec<brain_parse::Diag> {
    diags("comp/x.brain", &format!("<script>\n  let x = sig(1)\n</script>\n{body}\n"))
}

#[test]
fn b0780_stray_closing_brace_in_markup() {
    for body in ["<p>{x}}</p>", "<p>{{x}}</p>", "<p>price } today {x}</p>", "<a title=\"{x}}\">go</a>", "{#if x}}<p>y</p>{/if}", "<ul>{#each [x] as y}<li>{y}}</li>{/each}</ul>"] {
        let d = markup(body);
        let b = d.iter().find(|d| d.code == "B0780").unwrap_or_else(|| panic!("{body}: {d:?}"));
        assert!(b.is_warning(), "{b:?}");
    }
    // the location is the brace itself
    let d = markup("<div>\n  <p>one\n   two {x}} three</p>\n</div>");
    let b = d.iter().find(|d| d.code == "B0780").unwrap();
    assert_eq!((b.line, b.col), (6, 11), "{b:?}");
    for body in [
        "<p>{x}</p>",
        "<p>{x} and {'}'}</p>",
        "<p>a ) b ] c > d</p>",
        "<p>&#125;</p>",
        "<pre>if (a) {'{'} b }</pre>",
        "<code>{'{'} a }</code>",
        "<a title=\"a}\">go</a>",
    ] {
        assert_eq!(count(&markup(body), "B0780"), 0, "{body}");
    }
    // the mustache habit has one exact correction
    let d = markup("<p>{{x}}</p>");
    let b = d.iter().find(|d| d.code == "B0780").unwrap();
    assert_eq!(b.edits.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["{x}"], "{b:?}");
    // directly after a value it is the same typo, even inside code
    assert_eq!(count(&markup("<code>{x}}</code>"), "B0780"), 1);
}

#[test]
fn an_unclosed_brace_is_named_as_such() {
    for src in ["<p>{x</p>", "<p>{x + 1</p>\n<p>next</p>"] {
        let c = brain_parse::parse(&format!("<script>let x = sig(1)</script>\n{src}\n"), Path::new("comp/x.brain"));
        let d = c.expect_err("must not parse").remove(0);
        assert_eq!(d.code, "B0002", "{d:?}");
        assert!(d.msg.contains("not closed") && d.msg.contains("</p>"), "{d:?}");
        assert_eq!(d.line, 2, "{d:?}");
    }
}

#[test]
fn b0781_await_without_an_enclosing_async_function() {
    let flagged = [
        "<script>\n  const g = async (u) => u\n  function f() { const r = await g(1); return r }\n  f()\n</script>\n<p>x</p>",
        "<script>\n  const g = async (u) => u\n  const f = () => { await g(1) }\n  f()\n</script>\n<p>x</p>",
        "<script>\n  const g = async (u) => u\n  const f = () => await g(1)\n  f()\n</script>\n<p>x</p>",
        "<script>\n  const g = async (u) => u\n  async function f() { [1].map((i) => await g(i)) }\n  f()\n</script>\n<p>x</p>",
        "<script>\n  const g = async (u) => u\n  const o = { m() { return await g(1) } }\n  o.m()\n</script>\n<p>x</p>",
        "<script>\n  const g = async (u) => u\n  await g(1)\n</script>\n<p>x</p>",
        "<script>\n  const g = async (u) => u\n  const c = () => g(1)\n</script>\n<button on:click={() => await g(1)}>x</button><p>{c}</p>",
        "<script>\n  const g = async (u) => u\n</script>\n<p>{await g(1)}</p>",
        "<script>\n  server: function s(ctx) { return await ctx.x }\n  s()\n</script>\n<p>x</p>",
    ];
    for src in flagged {
        let d = diags("comp/x.brain", src);
        let b = d.iter().find(|d| d.code == "B0781").unwrap_or_else(|| panic!("{src}: {d:?}"));
        assert!(!b.is_warning(), "the emitted JavaScript does not parse: {b:?}");
    }
    let named = diags("comp/x.brain", "<script>\n  const g = async (u) => u\n  function f() { return await g(1) }\n  f()\n</script>\n<p>x</p>");
    assert!(named.iter().any(|d| d.code == "B0781" && d.fix.contains("async function f")), "{named:?}");
    let clean = [
        "<script>\n  const g = async (u) => u\n  async function f() { return await g(1) }\n  const h = async () => { await g(2) }\n  const i = async () => await g(3)\n  const o = { async m() { await g(1) } }\n  f(); h(); i(); o.m()\n</script>\n<button on:click={async () => await g(1)}>x</button>",
        "<script>\n  const g = async (u) => u\n  async function f() { const k = async () => await g(1); return k }\n  f()\n</script>\n<p>x</p>",
        "<script>\n  server: async function s(ctx) { return await ctx.x }\n  s()\n</script>\n<p>x</p>",
    ];
    for src in clean {
        let d = diags("comp/x.brain", src);
        assert_eq!(count(&d, "B0781"), 0, "{src}: {d:?}");
    }
    // a module is ESM: its top level may await, its plain functions may not
    assert_eq!(count(&diags("lib/x.jsbrain", "const v = await Promise.resolve(1)\nexport default v\n"), "B0781"), 0);
    assert_eq!(count(&diags("lib/x.jsbrain", "export function f() { return await Promise.resolve(1) }\n"), "B0781"), 1);
    assert_eq!(count(&diags("comp/x.test.brain", "<script>\n  await Promise.resolve(1)\n  test('a', async () => { await Promise.resolve(1) })\n</script>\n"), "B0781"), 0);
}

#[test]
fn b0737_names_every_credential_and_reads_members() {
    let d = diags("server/auth.jsbrain", "export async function login(ctx, d) { ctx.session.set({ id: 1, token: d.token, password: d.password }) }\n");
    let b = d.iter().find(|d| d.code == "B0737").unwrap();
    assert!(b.msg.contains("`token`") && b.msg.contains("`password`"), "{b:?}");
    let d = diags("server/auth.jsbrain", "export async function login(ctx, d) { ctx.session.set(d.refreshToken) }\n");
    assert_eq!(count(&d, "B0737"), 1, "{d:?}");
    let d = diags("server/auth.jsbrain", "export async function login(ctx, d) { ctx.session.set({ id: d.id, tokens_used: d.tokens_used, seen: d.token_count }) }\n");
    assert_eq!(count(&d, "B0737"), 0, "{d:?}");
}

#[test]
fn b0783_credentials_in_other_readable_places() {
    let in_module = [
        "export function f(password) { console.log(password) }",
        "export function f(d) { console.error('sign in', d.password) }",
        "export function f(token) { console.info(`got ${token}`) }",
        "export function f(token) { console.warn('got ' + token) }",
        "export function f(accessToken) { console.debug({ accessToken }) }",
        "export function f(t) { localStorage.setItem('authToken', t) }",
        "export function f(accessToken) { sessionStorage.setItem('k', accessToken) }",
    ];
    for src in in_module {
        let d = diags("lib/x.jsbrain", &format!("{src}\n"));
        let b = d.iter().find(|d| d.code == "B0783").unwrap_or_else(|| panic!("{src}: {d:?}"));
        assert!(b.is_warning(), "{b:?}");
    }
    let returned = [
        ("views/a.brain", "<meta seo title=\"A\" description=\"A page\">\n<script>\n  export async function load(ctx) { const u = await ctx.backend.resource('accounts').get(1); return { user: { id: u.id, password: u.password } } }\n</script>\n<main><h1>{user.id}</h1></main>\n"),
        ("comp/a.brain", "<script>\n  server: async function who(ctx, u) { return { name: u.name, passwordHash: u.hash } }\n  who()\n</script>\n<p>x</p>\n"),
        ("server/routes/a/handler.jsbrain", "<meta server>\nexport default async function handle(ctx) { const u = ctx.user; if (u) { return { status: 200, body: { password_hash: u.h } } } return {} }\n"),
    ];
    for (path, src) in returned {
        let d = diags(path, src);
        assert_eq!(count(&d, "B0783"), 1, "{path}: {d:?}");
    }
    let clean = [
        ("lib/x.jsbrain", "export function f(tokens_used, d) { console.log(tokens_used, d.count, 'password reset sent') }\n"),
        ("lib/x.jsbrain", "export function f(v) { localStorage.setItem('theme', v) }\n"),
        ("comp/a.brain", "<script>\n  server: async function who(ctx, t) { return { token: t, passwordless: true, password_changed_at: 1 } }\n  who()\n</script>\n<p>x</p>\n"),
        ("comp/a.brain", "<script>\n  server: async function who(ctx, u) { const inner = () => { return { password: u } }; return inner.name }\n  who()\n</script>\n<p>x</p>\n"),
    ];
    for (path, src) in clean {
        let d = diags(path, src);
        assert_eq!(count(&d, "B0783"), 0, "{src}: {d:?}");
    }
}

#[test]
fn a_test_file_under_views_is_not_a_routed_view() {
    let script = "<script>\n  import Home from './home.brain'\n  test('a', async () => { await mount(Home) })\n</script>\n";
    let with_markup = "<main><h1>one</h1><h1>two</h1></main>\n";
    for src in [script, with_markup] {
        let d = diags("views/home.test.brain", src);
        assert!(!d.iter().any(|d| d.code.starts_with("B05")), "{d:?}");
    }
    // a real view still needs it, and at 1:1: the missing declaration has no line of its own
    let d = diags("views/home.brain", with_markup);
    let b = d.iter().find(|d| d.code == "B0501").unwrap();
    assert_eq!((b.line, b.col), (1, 1));
}
