//! #34: route files compile to valid modules: dynamic segments, layouts and the 404 view.

use std::path::Path;

const VIEW: &str = r#"<meta seo title="T" description="D" canonical="/x">
<script>
export async function load(ctx) { return { id: ctx.params.id } }
</script>
<main><h1>Item {props.id}</h1></main>"#;

const LAYOUT: &str = r#"<header>Top</header>
<slot />"#;

fn compile(src: &str, path: &str) -> String {
    brain_codegen::compile(src, Path::new(path)).unwrap_or_else(|d| panic!("{path}: {d:#?}")).js
}

#[test]
fn route_files_get_valid_component_names() {
    for (src, path, name) in [
        (VIEW, "views/users/[id].brain", "UsersId"),
        (VIEW, "views/404.brain", "V404"),
        (VIEW, "views/blog/[...slug].brain", "Slug"),
        (LAYOUT, "views/users/_layout.brain", "Layout"),
    ] {
        let js = compile(src, path);
        assert!(js.contains(&format!("function {name}($p")), "{path} → expected {name}:\n{js}");
        assert!(js.contains(&format!("export default guard({name})")), "{path}:\n{js}");
    }
}

#[test]
fn guard_redirect_and_deny_compile_to_plain_objects() {
    // TOOLCHAIN #65 (§22): no import; the same module runs in the dev sidecar, server.mjs and the client router.
    let src = "export default (ctx) => ctx.user ? true : redirect('/login?next=' + ctx.url.pathname)";
    let o = brain_codegen::compile(src, Path::new("views/account/_guard.jsbrain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("({ redirect: '/login?next=' + ctx.url.pathname, status: 303 })"), "{}", o.js);
    let deny = "export default (ctx) => ctx.user?.admin ? true : deny('admins only')";
    let o = brain_codegen::compile(deny, Path::new("views/admin/_middleware.jsbrain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("({ deny: 'admins only' })"), "{}", o.js);
    // Outside guards and server code it is still an undeclared name.
    let d = brain_codegen::compile("<script>\nconst go = () => redirect('/x')\n</script>\n<button on:click={go}>x</button>", Path::new("comp/x.brain")).expect_err("B0111");
    assert!(d.iter().any(|d| d.code == "B0111"), "{d:#?}");
}

#[test]
fn not_found_in_load_compiles_to_a_plain_object() {
    // F11: `load` answers a real 404 (server.mjs and the dev sidecar read `notFound`); no import.
    let src = "<meta seo title=\"Job\" description=\"Job\">\n<script>\nexport async function load(ctx) {\n  if (ctx.params.slug === 'gone') throw notFound()\n  return { slug: ctx.params.slug }\n}\n</script>\n<main><h1>{props.slug}</h1></main>";
    let o = brain_codegen::compile(src, Path::new("views/jobs/[slug].brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.server_js.contains("throw ({ notFound: true, status: 404 })"), "{}", o.server_js);
    // with arguments it is still the 404 marker, not a call to an undefined `notFound` (ReferenceError → 500)
    let o = brain_codegen::compile(&src.replace("throw notFound()", "throw notFound('No such job')"), Path::new("views/jobs/[slug].brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.server_js.contains("throw ({ notFound: true, status: 404 })") && !o.server_js.contains("notFound("), "{}", o.server_js);
    // Client code has no notFound: it is still an undeclared name there.
    let d = brain_codegen::compile("<script>\nconst go = () => notFound()\n</script>\n<button on:click={go}>x</button>", Path::new("comp/x.brain")).expect_err("B0111");
    assert!(d.iter().any(|d| d.code == "B0111"), "{d:#?}");
}
