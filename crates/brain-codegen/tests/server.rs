//! SPEC §11.1: `load` and `server:` functions leave the client bundle; the client gets RPC stubs.

use std::path::Path;

#[test]
fn server_code_is_split_from_the_client() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("server-split");
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::write(root.join("lib/db.jsbrain"), "<meta server>\nconst all = async () => [process.env.DB_URL]\nexport default { all }\n").unwrap();
    std::fs::write(root.join("lib/fmt.jsbrain"), "export const up = (s) => s.toUpperCase()\n").unwrap();
    let src = r#"<meta seo title="Users" description="List">
<script>
import db from '../lib/db.jsbrain'
import { up } from '../lib/fmt.jsbrain'
export async function load(ctx) { return { users: await db.all() } }
server: async function rename(ctx, id, name) { return db.all(id, up(name)) }
let users = sig(props.users)
const go = async () => { users = await rename(1, 'x') }
</script>
<main><h1>{up('u')}</h1><button on:click={go}>Rename</button></main>"#;
    let opts = brain_codegen::Options { root: Some(root.clone()), ..Default::default() };
    let o = brain_codegen::compile_with(src, Path::new("views/users.brain"), &opts).unwrap_or_else(|d| panic!("{d:?}"));
    // Client: no load, no server body, no server import; one stub through the runtime's rpc.
    assert!(!o.js.contains("load"), "{}", o.js);
    assert!(!o.js.contains("db.jsbrain") && !o.js.contains("db.all"), "{}", o.js);
    assert!(o.js.contains(", rpc } from \"/@brain/brain.js\""), "{}", o.js);
    let hash = brain_codegen::component_hash("views/users.brain");
    assert!(o.js.contains(&format!("const rename = (...a) => rpc(\"{hash}/rename\", a);")), "{}", o.js);
    assert!(o.js.contains("import { up } from \"../lib/fmt.jsbrain\""), "{}", o.js);
    // Server: both functions exported with their imports (only the ones they use).
    assert_eq!(o.server_fns, ["rename"]);
    assert!(o.server_js.contains("import db from \"../lib/db.jsbrain\";"), "{}", o.server_js);
    assert!(o.server_js.contains("import { up } from \"../lib/fmt.jsbrain\";"), "{}", o.server_js);
    assert!(o.server_js.contains("export async function load(ctx)"), "{}", o.server_js);
    assert!(o.server_js.contains("export async function rename(ctx, id, name)"), "{}", o.server_js);

    // A `<meta server>` module compiles to server_js only.
    let m = brain_codegen::compile_with(&std::fs::read_to_string(root.join("lib/db.jsbrain")).unwrap(), Path::new("lib/db.jsbrain"), &opts).unwrap();
    assert!(m.js.is_empty() && m.server_js.contains("process.env.DB_URL"), "{m:?}");

    // Without server code nothing changes.
    let p = brain_codegen::compile("<script>let n = sig(0)</script><button on:click={n++}>{n}</button>", Path::new("comp/c.brain")).unwrap();
    assert!(p.server_js.is_empty() && p.server_fns.is_empty() && !p.js.contains("rpc"));
}

#[test]
fn server_js_imports_only_what_server_code_reads() {
    // TOOLCHAIN #145: `ctx.session.set(…)` must not pull in the client-only import `session`.
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("server-imports-{}", std::process::id()));
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::create_dir_all(root.join("server")).unwrap();
    std::fs::create_dir_all(root.join("channels")).unwrap();
    std::fs::write(root.join("app.brain"), "<html lang=\"en\"><head></head><body><slot /></body></html>").unwrap();
    std::fs::write(root.join("server/auth.jsbrain"), "<meta server>\nexport default { configured: () => true, check: (e, p) => null }\n").unwrap();
    std::fs::write(root.join("channels/session.jsbrain"), "export default store({ user: null })\n").unwrap();
    let view = "<meta seo title=\"Login\" description=\"Sign in.\">\n<script>\nimport auth from '../server/auth.jsbrain'\nimport session from '../channels/session.jsbrain'\nexport async function load() { return { configured: auth.configured() } }\nserver: async function login(ctx, v) { const user = auth.check(v.email, v.password); ctx.session.set(user); return user }\nlet f = form({ email: 'email!', password: 'str!' }, { submit: async (v) => { session.user = await login(v) } })\n</script>\n<main><h1>Login</h1><form on:submit={f.submit}><input aria-label=\"Email\" bind:value={f.v.email} /><button>Go</button></form></main>";
    let opts = brain_codegen::Options { root: Some(root.clone()), ..Default::default() };
    let o = brain_codegen::compile_with(view, Path::new("views/login.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.server_js.contains("import auth from \"../server/auth.jsbrain\";"), "{}", o.server_js);
    assert!(!o.server_js.contains("session.jsbrain"), "client-only import leaked into server_js:\n{}", o.server_js);
    assert!(o.js.contains("session.jsbrain"), "{}", o.js);
}
