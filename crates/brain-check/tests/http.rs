//! INTERNALS §40-§41 (#170): `<meta http>` (B1104), `{:catch}` without error states (B1105), `load` waterfalls
//! (B1106), `fetch()` in app code (B1107), `store()` inside `provide()` and `in: 'file'`.

use brain_check::Ctx;
use std::path::Path;

fn diags(path: &str, src: &str) -> Vec<brain_parse::Diag> {
    let c = brain_parse::parse(src, Path::new(path)).unwrap_or_else(|d| panic!("{d:?}"));
    brain_check::check(&c, &Ctx::default())
}

fn codes(path: &str, src: &str, code: &str) -> Vec<brain_parse::Diag> {
    diags(path, src).into_iter().filter(|d| d.code == code).collect()
}

const SHELL: &str = "<!doctype html>\n<html lang=\"en\">\n<head><title>x</title></head>\n<body><slot /></body>\n</html>\n";

#[test]
fn meta_http_vocabulary() {
    let ok = format!("<meta http base=\"/api\" timeout=\"10s\" retry=\"2\" login=\"/login\">\n{SHELL}");
    assert!(codes("app.brain", &ok, "B1104").is_empty());
    let c = brain_parse::parse(&ok, Path::new("app.brain")).unwrap();
    assert_eq!(brain_check::http::policy_js(&c).unwrap(), "http({ base: \"/api\", timeout: \"10s\", retry: 2, login: \"/login\" });\n");

    let bad = format!("<meta http base=\"api\" timeout=\"ten\" retry=\"9\" login=\"//evil.dev\" retries=\"2\">\n{SHELL}");
    let d = codes("app.brain", &bad, "B1104");
    assert_eq!(d.len(), 5, "{d:#?}");
    let unknown = d.iter().find(|d| d.msg.contains("retries")).unwrap();
    assert!(unknown.fix.contains("did you mean `retry`"), "{}", unknown.fix);
    assert_eq!(unknown.edits[0].text, "");

    let d = codes("views/x.brain", "<meta seo title=\"X\" description=\"Y\">\n<meta http base=\"/api\">\n<p>x</p>\n", "B1104");
    assert_eq!(d.len(), 1);
    assert!(d[0].msg.contains("only goes in `app.brain`"));
}

#[test]
fn catch_without_error_states() {
    let src = "<script>\n  import posts from '../api/posts.jsbrain'\n  let list = resource(posts.get)\n</script>\n{#await list}<p>…</p>{:then xs}{#each xs as p (p.id)}<p>{p.title}</p>{:empty}<p>None</p>{/each}{:catch e}<p role=\"alert\">Could not load.</p>{/await}\n";
    let d = codes("comp/list.brain", src, "B1105");
    assert_eq!(d.len(), 1, "{d:#?}");
    assert_eq!(d[0].edits[0].text, "<ErrorState error={e} retry={list.refetch} />");
    // Handled: reads the kind/status, or the file renders <ErrorState>.
    assert!(codes("comp/list.brain", &src.replace("<p role=\"alert\">Could not load.</p>", "<p role=\"alert\">{e.status === 403 ? 'No access' : 'Could not load'}</p>"), "B1105").is_empty());
    assert!(codes("comp/list.brain", &src.replace("<p role=\"alert\">Could not load.</p>", "<ErrorState error={e} />"), "B1105").is_empty());
}

#[test]
fn load_waterfall() {
    let src = "<meta seo title=\"X\" description=\"Y\">\n<script>\n  import users from '../api/users.jsbrain'\n  export async function load(ctx) {\n    const user = await users.get(ctx.params.id)\n    const all = await users.list()\n    const mine = await users.list({ owner: user.id })\n    return { user, all, mine }\n  }\n</script>\n<p>{props.user.name}</p>\n";
    let d = codes("views/u.brain", src, "B1106");
    assert_eq!(d.len(), 1, "{d:#?}");
    // `mine` reads `user`, so only the first two run together.
    assert_eq!(d[0].edits[0].text, "const [user, all] = await Promise.all([users.get(ctx.params.id), users.list()])");
}

#[test]
fn fetch_in_app_code() {
    assert_eq!(codes("comp/x.brain", "<script>\n  const go = async () => { await fetch('/x') }\n</script>\n<button on:click={go}>Go</button>\n", "B1107").len(), 1);
    // Server code may call third parties.
    assert!(codes("views/x.brain", "<meta seo title=\"X\" description=\"Y\">\n<script>\n  export async function load() {\n    const r = await fetch('https://api.example.com/x')\n    return { r: await r.json() }\n  }\n</script>\n<p>{props.r}</p>\n", "B1107").is_empty());
    assert!(codes("api/_http.jsbrain", "export default { get: (u) => fetch(u) }\n", "B1107").is_empty());
}

#[test]
fn store_inside_provide_and_file_endpoints() {
    let src = "<script>\n  const Cart = key('cart')\n  provide(Cart, store({ items: [] }))\n</script>\n<slot />\n";
    assert!(codes("comp/shop.brain", src, "B0806").is_empty());
    let d = codes("comp/shop.brain", "<script>\n  const s = store({ items: [] })\n</script>\n<p>{s.items}</p>\n", "B0806");
    assert!(d[0].fix.contains("provide(Key, store("), "{}", d[0].fix);
    let up = "export default api('/files', { post: { in: 'file', out: 'str' }, put: { in: '[file]' } })\n";
    assert!(diags("api/files.jsbrain", up).iter().all(|d| d.code != "B1103"));
}

#[test]
fn portal_component_and_transitions() {
    let d = diags("comp/x.brain", "<script>\n  let open = sig(false)\n</script>\n<button on:click={open = !open}>T</button>{#if open}<div transition:fdae>x</div>{/if}<component label=\"x\" /><Portal to=\"#layer\"><p>t</p></Portal>\n");
    let codes: Vec<&str> = d.iter().map(|d| d.code).collect();
    assert!(codes.contains(&"B0216") && codes.contains(&"B0217") && !codes.contains(&"B0203"), "{d:#?}");
    let t = d.iter().find(|d| d.code == "B0216").unwrap();
    assert_eq!(t.edits[0].text, "fade");
}

#[test]
fn b1101_covers_any_await_expression() {
    let src = "<script>\n  import posts from '../api/posts.jsbrain'\n  let list = resource(posts.get)\n  let on = sig(true)\n</script>\n{#await on ? list : null}<p>…</p>{:then xs}<p>{xs}</p>{/await}\n{#await list ?? null}<p>…</p>{:then xs}<p>{xs}</p>{/await}\n";
    let d = codes("comp/l.brain", src, "B1101");
    assert_eq!(d.len(), 2, "{d:#?}");
}

#[test]
fn b1101_names_the_missing_piece() {
    let head = "<script>\n  import posts from '../api/posts.jsbrain'\n  let list = resource(posts.get)\n</script>\n";
    let only_empty = format!("{head}{{#await list}}<p>…</p>{{:then xs}}<ul>{{#each xs as x (x.id)}}<li>{{x.t}}</li>{{/each}}</ul>{{:catch e}}<ErrorState error={{e}} retry={{list.refetch}} />{{/await}}\n");
    let d = codes("comp/l.brain", &only_empty, "B1101");
    assert_eq!(d.len(), 1, "{d:#?}");
    assert!(d[0].msg.contains("{:empty}") && !d[0].msg.contains("{:catch}"), "{}", d[0].msg);
    let only_catch = format!("{head}{{#await list}}<p>…</p>{{:then xs}}<ul>{{#each xs as x (x.id)}}<li>{{x.t}}</li>{{:empty}}<li>None</li>{{/each}}</ul>{{/await}}\n");
    let d = codes("comp/l.brain", &only_catch, "B1101");
    assert_eq!(d.len(), 1, "{d:#?}");
    assert!(d[0].msg.contains("{:catch}") && !d[0].msg.contains("{:empty}") && d[0].fix.contains("ErrorState"), "{}", d[0].msg);
}

#[test]
fn aria_reference_to_conditional_content() {
    let bad = "<script>\n  let open = sig(false)\n</script>\n<button aria-controls=\"a1\" aria-expanded={open} on:click={open = !open}>Q</button>{#if open}<div id=\"a1\">A</div>{/if}\n";
    assert_eq!(codes("comp/faq.brain", bad, "B0410").len(), 1);
    let ok = "<script>\n  let open = sig(false)\n</script>\n<button aria-controls=\"a1\" aria-expanded={open} on:click={open = !open}>Q</button><div id=\"a1\" hidden={!open}>A</div>\n";
    assert!(codes("comp/faq.brain", ok, "B0410").is_empty());
    let both = "<script>\n  let open = sig(false)\n</script>\n{#if open}<button aria-controls=\"a1\">Q</button><div id=\"a1\">A</div>{/if}\n";
    assert!(codes("comp/faq.brain", both, "B0410").is_empty());
}

#[test]
fn aria_reference_computed_and_nested_conditional() {
    // #308 N2: the FAQ pattern — the reference renders in every iteration, the panel only when open.
    let head = "<meta props=\"items:list\">\n<script>\n  let open = sig(0)\n</script>\n";
    let bad = format!("{head}{{#each items as it (it.id)}}<button aria-controls={{'faq-' + it.id}} on:click={{open = it.id}}>Q</button>{{#if open == it.id}}<div id={{'faq-' + it.id}}>A</div>{{/if}}{{/each}}\n");
    let d = codes("comp/faq.brain", &bad, "B0410");
    assert_eq!(d.len(), 1, "{d:#?}");
    assert!(d[0].msg.contains("aria-controls={") && d[0].msg.contains("'faq-' + it.id"), "{}", d[0].msg);
    // Same iteration, target always rendered: fine.
    let ok = format!("{head}{{#each items as it (it.id)}}<button aria-controls={{'faq-' + it.id}} on:click={{open = it.id}}>Q</button><div id={{'faq-' + it.id}} hidden={{open != it.id}}>A</div>{{/each}}\n");
    assert!(codes("comp/faq.brain", &ok, "B0410").is_empty());
    // Static reference into an `{#if}` nested in the same `{#each}`.
    let nested = format!("{head}{{#each items as it (it.id)}}<button aria-controls=\"p\">Q</button>{{#if open == it.id}}<div id=\"p\">A</div>{{/if}}{{/each}}\n");
    assert_eq!(codes("comp/faq.brain", &nested, "B0410").len(), 1);
}

#[test]
fn label_for_and_id_with_the_same_expression() {
    let src = "<script>\n  let rows = sig([{ id: 1, name: 'a' }])\n</script>\n{#each rows as r (r.id)}\n  <label for={'name-' + r.id}>Name</label>\n  <input id={'name-' + r.id} value={r.name} />\n  <label for={`n2-${r.id}`}>Other</label>\n  <input id={`n2-${r.id}`} value={r.name} />\n  <label for={'s-' + r.id}>Pick</label>\n  <select id={'s-' + r.id}><option>a</option></select>\n{/each}\n<label for=\"static\">Static</label><input id=\"static\" />\n";
    assert!(codes("comp/rows.brain", src, "B0403").is_empty(), "{:#?}", codes("comp/rows.brain", src, "B0403"));
    // A different expression is still unlabelled.
    let bad = src.replace("<input id={'name-' + r.id}", "<input id={'other-' + r.id}");
    assert_eq!(codes("comp/rows.brain", &bad, "B0403").len(), 1);
}

#[test]
fn labels_inside_await_branches() {
    let src = "<script>\n  const api = { get: () => Promise.resolve([]) }\n  let users = resource(api.get)\n</script>\n{#await users}\n  <p role=\"status\">Loading…</p>\n{:then list}\n  {#each list as u (u.id)}\n    <label for=\"name-{u.id}\">Name</label>\n    <input id=\"name-{u.id}\" value={u.name} />\n  {:empty}<p>None</p>\n  {/each}\n  <label for=\"plain\">Plain</label>\n  <input id=\"plain\" />\n{:catch e}<ErrorState error={e} />{/await}\n";
    assert!(codes("comp/users.brain", src, "B0403").is_empty(), "{:#?}", codes("comp/users.brain", src, "B0403"));
}
