//! #53: codegen side — automatic imports for primitives outside the core and the `brain/ui` kit, and
//! `bind:` on a field of a sig object.

use std::path::Path;

#[test]
fn auto_imports_and_field_binds() {
    let src = r#"<script>
let prefs = sig({ email: true, user: { name: '' } })
let f = form({ email: 'email!' }, { submit: async (v) => v })
const t = { form: 1 }
</script>
<form on:submit={f.submit}>
  <input type="checkbox" aria-label="e" bind:checked={prefs.email}>
  <input aria-label="n" bind:value={prefs.user.name}>
  <Field label="Email"><Button>Send</Button></Field>
</form>
<p>{t.form}</p>"#;
    let o = brain_codegen::compile(src, Path::new("comp/prefs.brain")).unwrap_or_else(|d| panic!("{d:?}"));
    assert!(o.js.contains("import { form } from \"/@brain/brain-form.js\";"), "{}", o.js);
    assert!(o.js.contains("import Button from \"/@brain/ui/button.brain\";"), "{}", o.js);
    assert!(o.js.contains("import Field from \"/@brain/ui/field.brain\";"), "{}", o.js);
    assert!(o.js.contains("prefs.v = { ...prefs.v, email: e.target.checked }"), "{}", o.js);
    assert!(o.js.contains("prefs.v = { ...prefs.v, user: { ...prefs.v.user, name: e.target.value } }"), "{}", o.js);
    // A user name that shadows a primitive is not imported; `t.form` is a property, not the primitive.
    let o = brain_codegen::compile("<script>const t = { form: 1 }</script><p>{t.form}</p>", Path::new("comp/a.brain")).unwrap();
    assert!(!o.js.contains("brain-form"), "{}", o.js);
    let o = brain_codegen::compile("<script>const key = 'x'</script><p>{key}</p>", Path::new("comp/b.brain")).unwrap();
    assert!(!o.js.contains("brain-ch"), "{}", o.js);
}

#[test]
fn dynamic_route_names_and_load_only_imports() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("dyn-route");
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::create_dir_all(root.join("views/users")).unwrap();
    std::fs::write(root.join("lib/db.jsbrain"), "export const get = async (id) => ({ id })\n").unwrap();
    let src = "<meta seo title=\"U\" description=\"u\">\n<script>\nimport db from '../../lib/db.jsbrain'\nexport async function load(ctx) { return { user: await db.get(ctx.params.id) } }\n</script>\n<main><h1>{props.user.id}</h1></main>";
    let opts = brain_codegen::Options { root: Some(root), ..Default::default() };
    let o = brain_codegen::compile_with(src, Path::new("views/users/[id].brain"), &opts).unwrap_or_else(|d| panic!("{d:?}"));
    assert!(o.js.contains("function UsersId($p = {})") && o.js.contains("export default guard(UsersId);"), "{}", o.js);
    assert!(!o.js.contains("db.jsbrain"), "{}", o.js);
    assert!(o.server_js.contains("import db from \"../../lib/db.jsbrain\";"), "{}", o.server_js);
    let f = brain_codegen::compile("<script>let f = form({ e: 'email!' }, {})</script><input aria-label=\"e\" bind:value={f.v.e}>", Path::new("comp/f.brain")).unwrap();
    assert!(f.js.contains("f.v.e = e.target.value"), "{}", f.js);
}

#[test]
fn hash_len_is_the_same_everywhere() {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/comp/counter.brain")).unwrap();
    let opts = brain_codegen::Options { hash_len: 4, ..Default::default() };
    let o = brain_codegen::compile_with(&src, Path::new("comp/counter.brain"), &opts).unwrap_or_else(|d| panic!("{d:?}"));
    let h = brain_codegen::component_hash_len("comp/counter.brain", 4);
    assert_eq!(h.len(), 4);
    assert!(brain_codegen::component_hash("comp/counter.brain").starts_with(&h));
    assert!(o.js.contains(&format!("data-b=\\\"{h}\\\"")), "{}", o.js);
    assert!(o.js.contains(&format!("Counter.h = \"{h}\";")), "{}", o.js);
    assert!(o.css.contains(&format!("@scope ([data-b=\"{h}\"])")), "{}", o.css);
    let s = brain_codegen::compile_with("<script>server: async function f(ctx) { return 1 }</script><button on:click={f()}>x</button>", Path::new("comp/s.brain"), &opts).unwrap();
    assert!(s.js.contains(&format!("rpc(\"{}/f\", a)", brain_codegen::component_hash_len("comp/s.brain", 4))), "{}", s.js);
}

#[test]
fn meta_fallback_wraps_the_component() {
    let src = "<meta fallback=\"Could not load the list\">\n<p>x</p>";
    let o = brain_codegen::compile(src, Path::new("comp/list.brain")).unwrap();
    // #174 (§42): the boundary is brain-guard's; `<meta fallback>` only sets its text. brain-err is not imported.
    assert!(o.js.starts_with("import { guard } from \"/@brain/brain-guard.js\";"), "{}", o.js);
    assert!(o.js.contains("List.f = \"comp/list.brain\";") && o.js.ends_with("export default guard(List, \"Could not load the list\");\n"), "{}", o.js);
    assert!(!o.js.contains("brain-err"), "{}", o.js);
    let d = brain_codegen::compile_with(src, Path::new("comp/list.brain"), &brain_codegen::Options { dev: true, ..Default::default() }).unwrap();
    assert!(d.js.contains("export default dev(guard(List, \"Could not load the list\"));"), "{}", d.js);
    // ui/ components fall back to their parent's boundary.
    let u = brain_codegen::compile("<meta props=\"text:str\">\n<span>{text}</span>", Path::new("ui/chip.brain")).unwrap();
    assert!(!u.js.contains("guard") && u.js.ends_with("export default Chip;\n"), "{}", u.js);
}
