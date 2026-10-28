//! INTERNALS §38 (#154): forms that submit to a `server:` function are real `<form method="post">` (RUNTIME #156
//! contract: hidden `_brain=<hash>/<fn>`, `name` from `bind:value`, RPC stub with `.id`, server `$forms`), and
//! `<meta client>` renders a sized placeholder on the server (B0730 info, B0731 for a bad size).
use std::path::Path;

const LOGIN: &str = "<meta seo title=\"Sign in\" description=\"Sign in to your account.\">
<script>
  server: async function signIn(ctx, v) {
    return { ok: v.email.length > 0 }
  }
  const f = form({ email: 'email!', password: 'str!', address: { city: 'str' } }, { submit: signIn })
</script>
<form on:submit={f.submit}>
  <label>Email <input type=\"email\" bind:value={f.v.email}></label>
  <label>Password <input type=\"password\" bind:value={f.v.password}></label>
  <label>City <input bind:value={f.v.address.city}></label>
  <label>Nick <input name=\"nick\" bind:value={f.v.email}></label>
  <button>Sign in</button>
</form>
";

#[test]
fn server_forms_post_without_js() {
    let o = brain_codegen::compile(LOGIN, Path::new("views/login.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    let h = o.js.split("data-b=\\\"").nth(1).and_then(|r| r.split('\\').next()).unwrap().to_string();
    assert!(o.js.contains("<form data-b=") || o.js.contains("<form method=\\\"post\\\""), "{}", o.js);
    assert!(o.js.contains("method=\\\"post\\\""), "{}", o.js);
    assert!(o.js.contains(&format!("<input type=\\\"hidden\\\" name=\\\"_brain\\\" value=\\\"{h}/signIn\\\"></form>")), "hidden field last: {}", o.js);
    assert!(o.js.contains("name=\\\"email\\\"") && o.js.contains("name=\\\"password\\\"") && o.js.contains("name=\\\"address.city\\\""), "{}", o.js);
    // An explicit name wins.
    assert_eq!(o.js.matches("name=\\\"nick\\\"").count(), 1, "{}", o.js);
    assert!(o.js.contains(&format!("signIn.id = \"{h}/signIn\";")), "{}", o.js);
    assert!(o.server_js.contains("export const $forms = { signIn: {"), "{}", o.server_js);
    assert!(o.server_js.contains("email: 'email!'") || o.server_js.contains("\"email\": 'email!'") || o.server_js.contains("email: \"email!\""), "{}", o.server_js);
}

#[test]
fn client_forms_stay_as_they_are() {
    let src = "<script>\n  const f = form({ q: 'str' }, { submit: async (v) => v })\n</script>\n<form on:submit={f.submit}><input bind:value={f.v.q} aria-label=\"Query\"><button>Go</button></form>\n";
    let o = brain_codegen::compile(src, Path::new("comp/search.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(!o.js.contains("method=\\\"post\\\"") && !o.js.contains("_brain"), "{}", o.js);
    // Field names help autofill and a JS-less GET anyway.
    assert!(o.js.contains("name=\\\"q\\\""), "{}", o.js);
}

#[test]
fn meta_client_is_a_sized_placeholder_on_the_server() {
    let src = "<meta client height=\"320\" aspect=\"16/9\">\n<script>\n  let box = ref()\n</script>\n<canvas bind:ref={box} aria-label=\"Chart\"></canvas>\n";
    let o = brain_codegen::compile(src, Path::new("comp/chart.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("if (globalThis.__brainSSR || globalThis.__brainRoute) return $t"), "{}", o.js);
    assert!(o.js.contains("data-client aria-busy=\\\"true\\\""), "{}", o.js);
    assert!(o.css.contains("[data-client]{display:block;block-size:320px;aspect-ratio:16/9}"), "{}", o.css);

    let c = brain_parse::parse(src, Path::new("comp/chart.brain")).unwrap();
    let d = brain_check::check(&c, &brain_check::Ctx::default());
    let info: Vec<_> = d.iter().filter(|d| d.code == "B0730").collect();
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].level, brain_parse::Severity::Info);

    let bad = "<meta client height=\"tall\" hieght=\"2\">\n<canvas aria-label=\"Chart\"></canvas>\n";
    let c = brain_parse::parse(bad, Path::new("comp/chart.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0731").collect();
    assert_eq!(d.len(), 2, "{d:#?}");
    assert!(d.iter().any(|d| d.fix.contains("did you mean `height`")), "{d:#?}");
}

#[test]
fn meta_budget_raises_the_guard() {
    let src = "<meta budget=\"rows:200000 ms:400\">\n<script>\n  let rows = sig([])\n</script>\n<ul>{#each rows as r (r.id)}<li>{r.name}</li>{/each}</ul>\n";
    let o = brain_codegen::compile(src, Path::new("comp/big.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("guard(Big, undefined, { rows: 200000, ms: 400 })"), "{}", o.js);
    let with_fb = "<meta fallback=\"Could not load\" budget=\"nodes:90000\">\n<p>x</p>\n";
    let o = brain_codegen::compile(with_fb, Path::new("comp/fb.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("guard(Fb, \"Could not load\", { nodes: 90000 })"), "{}", o.js);
    let bad = "<meta budget=\"rows:lots items:3\">\n<p>x</p>\n";
    let c = brain_parse::parse(bad, Path::new("comp/b.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0909").collect();
    assert_eq!(d.len(), 1, "{d:#?}");
    assert!(d[0].msg.contains("rows:lots") && d[0].msg.contains("items:3"), "{}", d[0].msg);
}

#[test]
fn arrow_submit_carries_the_server_id_to_form() {
    let src = "<meta seo title=\"Sign in\" description=\"Sign in to your account.\">\n<script>\n  server: async function login(ctx, v) {\n    return { ok: true }\n  }\n  let f = form({ email: 'email!' }, {\n    submit: async (v) => {\n      await login(v)\n      navigate('/account')\n    },\n  })\n</script>\n<form on:submit={f.submit}><label>Email <input type=\"email\" bind:value={f.v.email}></label><button>Go</button></form>\n";
    let o = brain_codegen::compile(src, Path::new("views/login.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("((o) => (o.submit.id = login.id, o))("), "{}", o.js);
    assert!(o.js.contains("name=\\\"_brain\\\""), "{}", o.js);
    assert!(o.server_js.contains("export const $forms = { login:"), "{}", o.server_js);
}
