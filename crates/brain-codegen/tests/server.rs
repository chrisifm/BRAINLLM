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
    assert!(o.server.load && !o.server.paths && !o.server.sitemap && !o.server.forms);
    assert_eq!(o.server.rpc, [format!("{hash}/rename")]);
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
    assert_eq!(p.server, brain_lex::ServerMetadata::default());
}

#[test]
fn server_metadata_uses_exported_ast_declarations_and_the_actual_rpc_namespace() {
    let source = r#"<meta seo title="Metadata" description="Owned server metadata" robots="noindex">
<script>
// function paths() and function sitemap() are examples, not declarations.
server: async function inspect(ctx) {
  function load() { return 'nested' }
  return load() + 'function load( function paths( function sitemap( const $forms'
}
</script>
<main><h1>Metadata</h1></main>"#;
    for hash_len in [4, 5, 6] {
        let options = brain_codegen::Options { hash_len, ..Default::default() };
        let output = brain_codegen::compile_with(source, Path::new("views/metadata.brain"), &options).unwrap();
        assert!(!output.server.load && !output.server.paths && !output.server.sitemap && !output.server.forms);
        let id = format!("{}/inspect", brain_codegen::component_hash_len("views/metadata.brain", hash_len));
        assert_eq!(output.server.rpc, [id.clone()]);
        assert!(output.js.contains(&format!("rpc(\"{id}\"")));
    }
    let source = source.replace("server: async function inspect(ctx)", "export function paths()");
    let output = brain_codegen::compile(&source, Path::new("views/metadata/[id].brain")).unwrap();
    assert!(output.server.paths && !output.server.load && !output.server.sitemap && output.server.rpc.is_empty());
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

/// A job or server module may name a local anything: a local is no use of the runtime helper of the same name, so it
/// must not add an import of a client module the server build does not emit.
#[test]
fn locals_named_like_runtime_helpers_add_no_import() {
    let opts = brain_codegen::Options::default();
    let compile = |src: &str| {
        let o = brain_codegen::compile_with(src, Path::new("server/jobs/sample/handler.jsbrain"), &opts).unwrap_or_else(|d| panic!("{src}: {d:?}"));
        if o.server_js.trim().is_empty() { o.js } else { o.server_js }
    };
    let names = brain_check::scope::EXT_PRIMITIVES.iter().map(|(name, _)| *name).chain(["t", "locale", "fmt", "entries", "entry", "tags", "pages", "page"]);
    for name in names.filter(|n| !n.starts_with('$')) {
        for body in [
            format!("const {name} = 'x' + ctx.payload.n; return {name}"),
            format!("const {{ {name} }} = ctx.payload; return {name}.length"),
            format!("for (const {name} of ctx.payload.list) log2({name}.x)"),
            format!("try {{ run() }} catch ({name}) {{ return {name}.message }}"),
            format!("const run = ({name}) => {name}(1); return run(String)"),
        ] {
            let src = format!("<meta server>\nconst log2 = () => 1, run = () => 1\nexport default async function handle(ctx) {{ {body} }}\n");
            let code = compile(&src);
            let added = code.lines().filter(|l| l.starts_with("import ") && !l.contains("brain-budget.js")).count();
            assert_eq!(added, 0, "`{name}` as a local added an import:\n{src}\n{code}");
        }
    }
    // Negative: a real free use keeps its import, also next to a local of the same name in another scope.
    let free = compile("<meta server>\nexport default async function handle(ctx) { return hashText(ctx.payload.n) }\n");
    assert!(free.contains("brain-hash.js"), "{free}");
    let both = compile("<meta server>\nconst one = (hashText) => hashText\nexport default async function handle(ctx) { return one(hashText(ctx.payload.n)) }\n");
    assert!(both.contains("brain-hash.js"), "{both}");
    // A block binding ends with its block: the read after it stays free.
    let outer = compile("<meta server>\nexport default async function handle(ctx) { if (ctx.a) { const hashText = 1; return hashText } return hashText(ctx.b) }\n");
    assert!(outer.contains("brain-hash.js"), "{outer}");
}
