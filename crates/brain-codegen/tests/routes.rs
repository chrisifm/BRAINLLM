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
