//! §33 crawler-safe status codes from `load`: `gone()` compiles to the 410 marker server.mjs and the dev server read.
use std::path::Path;

const SRC: &str = "<meta seo title=\"Job\" description=\"Job\">\n<script>\nexport async function load(ctx) {\n  if (ctx.params.slug === 'closed') throw gone()\n  if (ctx.params.slug === 'missing') throw notFound()\n  return { slug: ctx.params.slug }\n}\n</script>\n<main><h1>{props.slug}</h1></main>";

#[test]
fn gone_in_load_compiles_to_the_410_marker() {
    let o = brain_codegen::compile(SRC, Path::new("views/jobs/[slug].brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    // `notFound: true` keeps every older reader on the not-found path; `gone` turns it into 410
    assert!(o.server_js.contains("throw ({ notFound: true, gone: true, status: 410 })"), "{}", o.server_js);
    assert!(!o.server_js.contains("gone("), "{}", o.server_js);
}

#[test]
fn a_local_gone_function_is_left_alone_and_client_code_has_no_gone() {
    let local = SRC.replace("<script>\n", "<script>\nconst gone = () => ({ redirect: '/jobs' })\n");
    let o = brain_codegen::compile(&local, Path::new("views/jobs/[slug].brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.server_js.contains("throw gone()"), "{}", o.server_js);
    let d = brain_codegen::compile("<script>\nconst go = () => gone()\n</script>\n<button on:click={go}>x</button>", Path::new("comp/x.brain")).expect_err("B0111");
    assert!(d.iter().any(|d| d.code == "B0111"), "{d:#?}");
}
