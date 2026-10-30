//! #53: gaps found by the LLM test (bench/llm): imports that SPEC says are not needed, binds to fields of a
//! sig, and fixes that name what exists (B0302, B0119).

use brain_check::Ctx;
use std::path::Path;

fn diags(path: &str, src: &str, ctx: &Ctx) -> Vec<brain_parse::Diag> {
    let c = brain_parse::parse(src, Path::new(path)).unwrap_or_else(|d| panic!("{:?}", d));
    brain_check::check(&c, ctx)
}

fn codes(src: &str) -> Vec<&'static str> {
    diags("comp/x.brain", src, &Ctx::default()).into_iter().map(|d| d.code).collect()
}

#[test]
fn primitives_and_ui_kit_need_no_import() {
    assert_eq!(codes("<script>let f = form({ email: 'email!' }, { submit: async (v) => v })\nlet r = resource(() => fetch('/x'))\nconst k = key('user')</script><form on:submit={f.submit}><Field label=\"Email\"><Input value={f.v.email} /></Field><Button>Enviar</Button></form>{r.value}{k}"), ["B1107"]);
    // This is a recognized browser global, not an author-facing framework shim.
    // The runtime's internal yielding fallback does not wrap direct application calls.
    let scheduler=diags("comp/x.brain","<script>const t = async () => { await scheduler.yield() }</script><button on:click={t}>x</button>",&Ctx::default());
    assert_eq!(scheduler.iter().map(|diag|(diag.code,diag.level)).collect::<Vec<_>>(),[("B2101",brain_parse::Severity::Warning)],"{scheduler:?}");
    // Not in the kit → still B0203.
    assert_eq!(codes("<Carousel />"), ["B0203"]);
}

#[test]
fn bind_to_a_field_of_a_sig() {
    assert!(codes("<script>let prefs = sig({ email: true, name: '' })</script><input type=\"checkbox\" aria-label=\"e\" bind:checked={prefs.email}><input aria-label=\"n\" bind:value={prefs.name}>").is_empty());
    assert!(codes("<script>let f = sig({ user: { name: '' } })</script><input aria-label=\"n\" bind:value={f.user.name}>").is_empty());
    // A field of something that is not a sig is still an error.
    assert_eq!(codes("<script>const o = { a: '' }</script><input aria-label=\"n\" bind:value={o.a}>"), ["B0204"]);
}

#[test]
fn b0302_suggests_the_closest_token() {
    let tokens = ["--space-1", "--space-2", "--line", "--surface", "--fg", "--bg", "--accent"];
    let pick = |n: &str| brain_check::closest_token(n, &tokens);
    assert_eq!(pick("--sp-2"), Some("--space-2"));
    assert_eq!(pick("--spacing-1"), Some("--space-1"));
    assert_eq!(pick("--gap-2"), Some("--space-2"));
    assert_eq!(pick("--border"), Some("--line"));
    assert_eq!(pick("--card"), Some("--surface"));
    assert_eq!(pick("--primary"), Some("--accent"));
    assert_eq!(pick("--text"), Some("--fg"));
    assert_eq!(pick("--zzz"), None);
    let mut ctx = Ctx::default();
    ctx.tokens = Some(tokens.iter().map(|t| (t.to_string(), "1px".to_string())).collect());
    let d = diags("comp/x.brain", "<p>x</p><style>p { padding: var(--sp-2) }</style>", &ctx);
    assert_eq!(d[0].code, "B0302");
    assert!(d[0].fix.contains("--space-2"), "{}", d[0].fix);
}

#[test]
fn b0119_lists_existing_files() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("b0119");
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::write(root.join("lib/format.jsbrain"), "export const f = 1\n").unwrap();
    std::fs::write(root.join("lib/cart-store.jsbrain"), "export const c = 1\n").unwrap();
    let run = |src: &str| {
        let file = root.join("comp/x.brain");
        let c = brain_parse::parse(src, Path::new("comp/x.brain")).unwrap();
        let (ctx, _) = brain_check::load_ctx(&file, &c);
        brain_check::check(&c, &ctx)
    };
    let d = run("<script>import f from '../lib/formats.jsbrain'</script>{f}");
    assert_eq!(d[0].code, "B0119");
    assert!(d[0].fix.contains("`../lib/format.jsbrain`"), "{}", d[0].fix);
    let d = run("<script>import f from '../lib/dashboard.jsbrain'</script>{f}");
    assert!(d[0].fix.contains("`../lib/cart-store.jsbrain`") && d[0].fix.contains("`../lib/format.jsbrain`"), "{}", d[0].fix);
    let d = run("<script>import f from '../channels/cart.jsbrain'</script>{f}");
    assert!(d[0].fix.contains("does not exist") && d[0].fix.contains("`../lib/`"), "{}", d[0].fix);
}

#[test]
fn form_fields_layouts_and_routes() {
    assert!(codes("<script>let f = form({ email: 'email!' }, { submit: async (v) => v })</script><form on:submit={f.submit}><input aria-label=\"e\" bind:value={f.v.email}><button>Ir</button></form>").is_empty());
    let layout = diags("views/users/_layout.brain", "<section><nav aria-label=\"Usuarios\">Usuarios</nav><slot /></section>", &Ctx::default());
    assert!(layout.is_empty(), "{layout:?}");
    let nf = diags("views/404.brain", "<main><h1>Not found</h1></main>", &Ctx::default());
    assert_eq!(nf.iter().map(|d| d.code).collect::<Vec<_>>(), ["B0501"]);
}

#[test]
fn missing_import_hints_are_project_bounded_regular_and_complete() {
    struct Owned(std::path::PathBuf);
    impl Drop for Owned {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
    let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let owned=Owned(std::env::temp_dir().join(format!("brain-safe-hints-{}-{stamp}",std::process::id())));
    let project=owned.0.join("project");let external=owned.0.join("external");
    std::fs::create_dir_all(project.join("comp")).unwrap();std::fs::create_dir_all(project.join("lib")).unwrap();std::fs::create_dir_all(&external).unwrap();
    std::fs::write(project.join("lib/format.jsbrain"),"export default {}\n").unwrap();
    std::fs::write(external.join("outside-only.jsbrain"),"export default {}\n").unwrap();
    let check=|source:&str| {
        let file=project.join("comp/x.brain");let component=brain_parse::parse(source,Path::new("comp/x.brain")).unwrap();
        let (ctx,_)=brain_check::load_ctx(&file,&component);brain_check::check(&component,&ctx)
    };
    let outside=check("<script>import value from '../../external/missing.jsbrain'</script>{value}");
    assert_eq!(outside[0].code,"B0119");assert!(outside[0].fix.contains("project"));assert!(!outside[0].fix.contains("outside-only"));assert!(outside[0].edits.is_empty());
    #[cfg(unix)] {
        std::os::unix::fs::symlink(external.join("outside-only.jsbrain"),project.join("lib/leaked-symlink.jsbrain")).unwrap();
        let hints=check("<script>import value from '../lib/unrelated.jsbrain'</script>{value}");
        assert_eq!(hints[0].code,"B0119");assert!(hints[0].fix.contains("format.jsbrain"));assert!(!hints[0].fix.contains("leaked-symlink"));
    }
    for i in 0..513 {std::fs::write(project.join(format!("lib/candidate{i}.jsbrain")),"export default {}\n").unwrap();}
    let capped=check("<script>import value from '../lib/formats.jsbrain'</script>{value}");
    assert_eq!(capped[0].code,"B0119");assert!(capped[0].fix.contains("512"));assert!(capped[0].edits.is_empty());
}
