//! #123 (INTERNALS §35): B0127 DOM in a worker, B1102 `cache` of `api()`, `store({ persist: true })`.

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

fn codes(path: &str, src: &str) -> Vec<&'static str> {
    diags(path, src).into_iter().map(|d| d.code).collect()
}

#[test]
fn b0127_no_dom_in_workers() {
    assert!(codes("lib/stats.worker.jsbrain", "const mean = (xs) => xs.length\nexport default { mean }").is_empty());
    assert_eq!(codes("lib/a.worker.jsbrain", "export default { f: () => document.title }"), ["B0127"]);
    assert_eq!(codes("lib/a.worker.jsbrain", "export default { f: () => localStorage.getItem('x') }"), ["B0127"]);
    assert!(codes("lib/a.worker.jsbrain", "export default { f: () => sig(1) }").contains(&"B0127"));
    assert_eq!(codes("lib/a.worker.jsbrain", "import Card from '../comp/card.brain'\nexport default { f: () => 1 }").iter().filter(|c| **c == "B0127").count(), 1);
    // The same code outside a worker is not B0127.
    assert!(!codes("lib/a.jsbrain", "export default { f: () => localStorage.getItem('x') }").contains(&"B0127"));
}

#[test]
fn b1102_api_cache() {
    let api = |m: &str| format!("export default api('/posts', {{ {m} }})");
    assert!(codes("api/posts.jsbrain", &api("get: { out: 'list', cache: '10m' }")).is_empty());
    let d = diags("api/posts.jsbrain", &api("get: { out: 'list', cache: '10 minutes' }"));
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["B1102"]);
    assert_eq!(d[0].edits[0].text, "'10m'");
    assert_eq!(codes("api/posts.jsbrain", &api("get: { out: 'list', cache: 'soon' }")), ["B1102"]);
    let d = diags("api/posts.jsbrain", &api("post: { in: 'obj', cache: '1h', out: 'obj' }"));
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["B1102"]);
    let fixed = brain_parse::Diag::apply(&api("post: { in: 'obj', cache: '1h', out: 'obj' }"), &d[0].edits).unwrap();
    assert!(!fixed.contains("cache") && codes("api/posts.jsbrain", &fixed).is_empty(), "{fixed}");
}

#[test]
fn store_persist_is_accepted() {
    assert!(codes("channels/cart.jsbrain", "export default store({ items: [] }, { persist: true })").is_empty());
}

#[test]
fn b1103_unknown_api_type() {
    let mut ctx = Ctx::default();
    ctx.types = Some(["User".to_string()].into_iter().collect());
    let check = |src: &str, ctx: &Ctx| -> Vec<&'static str> {
        let c = brain_parse::parse(src, Path::new("api/x.jsbrain")).unwrap();
        brain_check::check(&c, ctx).into_iter().map(|d| d.code).collect()
    };
    assert!(check("export default api('/u', { get: { out: '[User]' } })", &ctx).is_empty());
    assert_eq!(check("export default api('/u', { get: { out: '[Usr]' } })", &ctx), ["B1103"]);
    // Primitive types of the mini-language are not type names.
    assert!(check("export default api('/n', { get: { out: 'num' } })", &ctx).is_empty());
    // Without types.jsbrain every type name is unknown.
    assert_eq!(check("export default api('/u', { get: { out: 'User' } })", &Ctx::default()), ["B1103"]);
}

#[test]
fn b1102_when_data_caching_is_off() {
    // QA #124 / §35: with brain.json "cache": { "data": false } the `cache:` option is dead → warning with a removing edit.
    let mut ctx = Ctx { cache_data_off: true, ..Ctx::default() };
    ctx.types = Some(["User".to_string()].into_iter().collect());
    let src = "export default api('/x', { get: { out: '[User]', cache: '10m' } })";
    let c = brain_parse::parse(src, Path::new("api/x.jsbrain")).unwrap();
    let d = brain_check::check(&c, &ctx);
    assert_eq!(d.iter().map(|d| (d.code, d.is_warning())).collect::<Vec<_>>(), [("B1102", true)]);
    let fixed = brain_parse::Diag::apply(src, &d[0].edits).unwrap();
    assert!(!fixed.contains("cache"), "{fixed}");
    // Data caching on (the default): nothing.
    let on = Ctx { types: ctx.types.clone(), ..Ctx::default() };
    assert!(brain_check::check(&c, &on).is_empty());
}
