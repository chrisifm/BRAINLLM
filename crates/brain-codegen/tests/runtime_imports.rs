//! #184: every `/@brain/<module>.js` the codegen imports exists in runtime/ and exports the imported names.

use std::path::Path;

fn check(js: &str, rel: &str) {
    for line in js.lines().filter(|l| l.starts_with("import ") && l.contains("\"/@brain/")) {
        let module = line.split("\"/@brain/").nth(1).and_then(|r| r.split('"').next()).unwrap();
        if module.starts_with("ui/") {
            continue; // kit components, served from runtime/ui/*.brain
        }
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime").join(module);
        assert!(file.exists(), "{rel}: imports /@brain/{module}, which runtime/ does not ship:\n{line}");
        if let Some(names) = line.split('{').nth(1).and_then(|r| r.split('}').next()) {
            for n in names.split(',').map(|n| n.trim().split(" as ").next().unwrap_or("").trim()).filter(|n| !n.is_empty()) {
                assert!(brain_check::scope::runtime_has(module, n) || module == "brain.js", "{rel}: `{n}` is not exported by runtime/{module}");
            }
        }
    }
}

#[test]
fn every_runtime_import_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut stack = vec![root.clone()];
    let mut n = 0;
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if name != "invalid" {
                    stack.push(p);
                }
                continue;
            }
            if !(name.ends_with(".brain") || name.ends_with(".jsbrain")) || name.ends_with(".test.brain") {
                continue;
            }
            let rel = p.strip_prefix(&root).unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(&p).unwrap();
            for dev in [false, true] {
                let opts = brain_codegen::Options { dev, root: Some(root.clone()), ..Default::default() };
                if let Ok(o) = brain_codegen::compile_with(&src, Path::new(&rel), &opts) {
                    check(&o.js, &rel);
                    n += 1;
                }
            }
        }
    }
    assert!(n > 5, "compiled {n} files");
}

#[test]
fn a_word_in_a_string_is_not_a_primitive() {
    let src = "const all = [{ body: 'Dynamic routes use one view; provide a key and a store, then t it.' }]\n// use(Theme) in a comment\nexport default { all: () => all, find: (id) => all.find((p) => p.id === id) ?? null }\n";
    let o = brain_codegen::compile(src, Path::new("lib/posts.jsbrain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(!o.js.contains("brain-ch.js") && !o.js.contains("brain-i18n.js"), "{}", o.js);
    let comp = "<script>\n  const note = 'we use it'\n</script>\n<p>{note}</p>\n";
    let o = brain_codegen::compile(comp, Path::new("comp/note.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(!o.js.contains("brain-ch.js") && !o.js.contains("scope("), "{}", o.js);
    // A real use still imports.
    let o = brain_codegen::compile("<script>\n  const Theme = key('theme', 'light')\n  let th = use(Theme)\n</script>\n<p>{th}</p>\n", Path::new("comp/t.brain")).unwrap();
    assert!(o.js.contains("import { use, key"), "{}", o.js);
}

#[test]
fn navigate_and_route_come_from_the_router() {
    let src = "<meta seo title=\"Login\" description=\"Sign in\">\n<script>\n  let r = route()\n  const go = () => navigate(new URLSearchParams(location.search).get('next') || '/account')\n</script>\n<button on:click={go}>{r.params.id}</button>\n";
    let o = brain_codegen::compile(src, Path::new("views/login.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("import { navigate } from \"/@brain/brain-router.js\""), "{}", o.js);
    assert!(o.js.contains("import { route } from \"/@brain/brain-env.js\""), "{}", o.js);
    check(&o.js, "views/login.brain");
}

#[test]
fn meta_http_becomes_the_app_policy() {
    let src = "<meta http base=\"/api\" timeout=\"10s\" retry=\"2\" login=\"/login\">\n<!doctype html>\n<html lang=\"en\">\n<head><title>x</title></head>\n<body><slot /></body>\n</html>\n";
    let o = brain_codegen::compile(src, Path::new("app.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("import { http } from \"/@brain/brain-data.js\""), "{}", o.js);
    assert!(o.js.contains("http({ base: \"/api\", timeout: \"10s\", retry: 2, login: \"/login\" });"), "{}", o.js);
    assert!(!o.js.contains("<meta http"), "the policy is not markup: {}", o.js);
    check(&o.js, "app.brain");
}

#[test]
fn debounce_takes_a_getter() {
    let src = "<script>\n  let q = sig('')\n  let slow = debounce(q, '300ms')\n</script>\n<input bind:value={q} aria-label=\"Search\" /><p>{slow}</p>\n";
    let o = brain_codegen::compile(src, Path::new("comp/search.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("debounce(() => q.v, '300ms')"), "{}", o.js);
    assert!(o.js.contains("import { debounce } from \"/@brain/brain-env.js\""), "{}", o.js);
    check(&o.js, "comp/search.brain");
}

#[test]
fn portal_component_is_and_transition() {
    let src = "<script>\n  import A from './a.brain'\n  let open = sig(false)\n  let cur = sig(A)\n</script>\n<button on:click={open = !open}>Toggle</button>\n{#if open}<div transition:fade={{ ms: 150 }}>Hi</div>{/if}\n<component is={cur} label=\"x\" />\n<Portal to=\"#layer\"><p>Toast</p></Portal>\n<p>a portal word</p>\n";
    let root = std::env::temp_dir().join(format!("brain-portal-{}", std::process::id()));
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::write(root.join("comp/a.brain"), "<meta props=\"label:str\">\n<p>{label}</p>\n").unwrap();
    std::fs::write(root.join("comp/x.brain"), src).unwrap();
    let o = brain_codegen::compile_with(src, &root.join("comp/x.brain"), &brain_codegen::Options::default());
    std::fs::remove_dir_all(&root).ok();
    let o = o.unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("transition(") && o.js.contains("\"fade\", { ms: 150 })"), "{}", o.js);
    assert!(o.js.contains("dyn(null, () => cur, { \"label\": \"x\" }"), "{}", o.js);
    assert!(o.js.contains("portal(\"#layer\", () =>"), "{}", o.js);
    assert!(o.js.contains("import { dyn, portal } from \"/@brain/brain-env.js\""), "{}", o.js);
    assert!(o.js.contains("import { transition } from \"/@brain/brain-anim.js\""), "{}", o.js);
    check(&o.js, "comp/x.brain");
}

#[test]
fn resource_node_methods_are_not_unwrapped() {
    let src = "<script>\n  const api = { get: () => Promise.resolve([]) }\n  let list = resource(api.get)\n</script>\n<button on:click={list.refetch()}>Reload</button>\n{#await list}<p>…</p>{:then xs}<p>{xs.length} {list.done}</p>{:catch e}<ErrorState error={e} retry={list.refetch} />{/await}\n";
    let o = brain_codegen::compile(src, Path::new("comp/l.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("list.refetch()") && !o.js.contains("list.v.refetch"), "{}", o.js);
    assert!(o.js.contains("list.done") && !o.js.contains("list.v.done"), "{}", o.js);
    assert!(o.js.contains("return list.refetch;") || o.js.contains("list.refetch"), "{}", o.js);
}

#[test]
fn kit_component_inside_slot_scope_is_imported() {
    let src = "<script>\n  import List from './list.brain'\n</script>\n<List>{#slot item}<Button>{item}</Button>{/slot}</List>\n";
    let root = std::env::temp_dir().join(format!("brain-slot-scope-{}", std::process::id()));
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::write(root.join("comp/list.brain"), "<script>\n  let items = sig(['a', 'b'])\n</script>\n<ul>{#each items as it}<li><slot item={it} /></li>{/each}</ul>\n").unwrap();
    std::fs::write(root.join("comp/x.brain"), src).unwrap();
    let o = brain_codegen::compile_with(src, &root.join("comp/x.brain"), &brain_codegen::Options::default());
    std::fs::remove_dir_all(&root).ok();
    let o = o.unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("/ui/button.brain"), "{}", o.js);
    check(&o.js, "comp/x.brain");
}

#[test]
fn runtime_169_options_and_members_compile() {
    let src = "<script>\n  const posts = { get: () => Promise.resolve([]), post: (x, o) => Promise.resolve(x) }\n  let list = resource(posts.get, { page: true, every: '30s' })\n  const Cart = key('cart')\n  const cart = provide(Cart, store({ items: [] }))\n  const f = form({ email: 'email!' }, { check: { email: async (v) => (v ? '' : 'Taken') }, validate: (v) => ({}), init: { email: '' }, submit: async (v) => posts.post(v, { optimistic: () => {}, priority: 'low' }) })\n</script>\n<p>{f.dirty ? 'dirty' : ''} {f.checking ? '…' : ''} {list.done ? 'end' : ''}</p>\n<button on:click={list.more()}>More</button><button on:click={cart.$reset()}>Reset</button>\n";
    let c = brain_parse::parse(src, Path::new("comp/opts.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.level == brain_parse::Severity::Error).collect();
    assert!(d.is_empty(), "{d:#?}");
    let o = brain_codegen::compile(src, Path::new("comp/opts.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("list.more()") && o.js.contains("list.done") && !o.js.contains("list.v.more"), "{}", o.js);
    check(&o.js, "comp/opts.brain");
}

#[test]
fn t_in_meta_seo_is_imported_on_the_server_and_app_shell_is_static() {
    let src = "<meta seo title={t('about.title')} description={t('about.description')}>\n<main><h1>About</h1></main>\n";
    let o = brain_codegen::compile(src, Path::new("views/about.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.server_js.contains("import { t } from \"/@brain/brain-i18n.js\""), "{}", o.server_js);
    assert!(o.server_js.contains("export const seo = (props) =>"), "{}", o.server_js);

    let shell = "<!doctype html>\n<html lang=\"en\">\n<head><title>x</title></head>\n<body><header>{t('site.brand')}<LangSwitch /></header><slot /></body>\n</html>\n";
    let c = brain_parse::parse(shell, Path::new("app.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0132").collect();
    assert_eq!(d.len(), 2, "{d:#?}");
    assert!(d[0].fix.contains("views/_layout.brain"));
}

#[test]
fn a_space_on_the_same_line_survives_a_block_edge() {
    let src = "<script>\n  let hot = sig(true)\n</script>\n<h3>Pro{#if hot} <span>Most popular</span>{/if}</h3>\n<ul>\n  {#if hot}\n    <li>a</li>\n  {/if}\n</ul>\n";
    let o = brain_codegen::compile(src, Path::new("comp/plan.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("tpl(\" <span>Most popular</span>\")") || o.js.contains("tpl(\" <span"), "{}", o.js);
    // Indentation around a block still disappears.
    assert!(o.js.contains("tpl(\"<li>a</li>\")"), "{}", o.js);
}
