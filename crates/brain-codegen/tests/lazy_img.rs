//! #66 (§22): `<Chart lazy />` loads its chunk on a trigger; built-in `<Img>` gets decoding/loading/fetchpriority.
use std::path::Path;

fn app(files: &[(&str, &str)]) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("brain-lazy-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (rel, src) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
    root
}

#[test]
fn lazy_component_imports_on_its_trigger() {
    let page = "<script>\n  import Chart from './chart.brain'\n  let rows = sig([1, 2])\n</script>\n<section><Chart lazy data={rows} /><Chart lazy=\"idle\" data={rows} /></section>\n";
    let root = app(&[("comp/chart.brain", "<meta props=\"data:list\">\n<p>{data.length}</p>\n"), ("comp/page.brain", page)]);
    let o = brain_codegen::compile_with(page, &root.join("comp/page.brain"), &brain_codegen::Options::default());
    std::fs::remove_dir_all(&root).ok();
    let o = o.unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("lazy($l") && o.js.contains("() => import(\"./chart.brain"), "{}", o.js);
    assert!(o.js.contains("\"visible\", { \"data\": rows }") && o.js.contains("\"idle\", { \"data\": rows }"), "{}", o.js);
    assert!(!o.js.contains("import Chart from"), "a lazy-only component is not imported up front: {}", o.js);
    assert!(o.js.contains("import { lazy } from \"/@brain/brain-lazy.js\""), "{}", o.js);

    let c = brain_parse::parse(&page.replace("lazy=\"idle\"", "lazy=\"idel\""), Path::new("comp/page.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0220").collect();
    assert!(d.len() == 1 && d[0].edits[0].text == "idle", "{d:#?}");
}

#[test]
fn img_gets_priority_and_lazy_loading() {
    let view = "<meta seo title=\"Team\" description=\"The people behind the product.\">\n<main><h1>Team</h1><Img src=\"/hero.jpg\" alt=\"Team\" width=\"1200\" height=\"630\" /><Img src=\"/b.jpg\" alt=\"B\" width=\"400\" height=\"300\" /></main>\n";
    let o = brain_codegen::compile(view, Path::new("views/team.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("src=\\\"/hero.jpg\\\" alt=\\\"Team\\\" width=\\\"1200\\\" height=\\\"630\\\" decoding=\\\"async\\\" fetchpriority=\\\"high\\\""), "{}", o.js);
    assert!(o.js.contains("src=\\\"/b.jpg\\\" alt=\\\"B\\\" width=\\\"400\\\" height=\\\"300\\\" decoding=\\\"async\\\" loading=\\\"lazy\\\""), "{}", o.js);
    let c = brain_parse::parse("<Img src=\"/x.jpg\" />\n", Path::new("comp/x.brain")).unwrap();
    let codes: Vec<&str> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().map(|d| d.code).collect();
    assert!(codes.contains(&"B0221") && codes.contains(&"B0401") && !codes.contains(&"B0203"), "{codes:?}");
}

#[test]
fn compiled_interaction_placeholder_is_keyboard_usable() {
    let page = "<script>\nimport Chart from './chart.brain'\n</script>\n<section><Chart lazy=\"interaction\" /></section>";
    let chart = "<p>Loaded chart</p>";
    let root = app(&[("comp/chart.brain", chart), ("comp/page.brain", page), ("tokens.cssbrain", "@tokens { --fg: #111111; --bg: #ffffff; }")]);
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime").canonicalize().unwrap();
    let opts = brain_codegen::Options { runtime: format!("file://{}/brain.js", runtime.display()), import_suffix: ".mjs".into(), root: Some(root.clone()), ..Default::default() };
    let out = brain_codegen::compile_with(page, &root.join("comp/page.brain"), &opts).unwrap_or_else(|d| panic!("{d:?}"));
    assert!(out.js.contains("createElement(\"button\")") && out.js.contains("Load component"), "{}", out.js);
    assert!(out.js.contains("setAttribute(\"type\", \"button\")"), "no accidental form submit");
    std::fs::write(root.join("comp/page.brain.mjs"), &out.js).unwrap();
    let child = brain_codegen::compile_with(chart, &root.join("comp/chart.brain"), &opts).unwrap();
    std::fs::write(root.join("comp/chart.brain.mjs"), child.js).unwrap();
    let script = format!(r#"
import assert from 'node:assert/strict';
import {{ install, Event }} from 'file://{rt}/shim/dom.js';
const doc = install();
const {{ mount }} = await import('file://{rt}/brain.js');
const {{ default: Page }} = await import('./comp/page.brain.mjs');
const host = doc.createElement('main');
const dispose = mount(Page, host);
const button = host.querySelector('button');
assert.equal(button.textContent, 'Load component');
assert.equal(button.getAttribute('type'), 'button');
button.dispatchEvent(Object.assign(new Event('keydown', {{ cancelable: true }}), {{ key: 'Enter' }}));
for (let i = 0; i < 50 && !host.textContent.includes('Loaded chart'); i++) await new Promise(r => setTimeout(r, 10));
assert.equal(host.textContent, 'Loaded chart');
dispose();
assert.equal(host.textContent, '');
"#, rt = runtime.display());
    std::fs::write(root.join("run.mjs"), script).unwrap();
    let result = std::process::Command::new("node").arg(root.join("run.mjs")).output().expect("Node is required for compiled lazy integration");
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn img_respects_explicit_attributes_and_user_components() {
    let page = "<meta seo title=\"Images\" description=\"Image examples\"><main><h1>Images</h1><Img src=\"/hero.jpg\" alt=\"Hero\" width=\"800\" height=\"400\" decoding=\"sync\" fetchpriority=\"low\" loading=\"eager\" /></main>";
    let out = brain_codegen::compile(page, Path::new("views/images.brain")).unwrap();
    assert!(out.js.contains("decoding=\\\"sync\\\"") && out.js.contains("fetchpriority=\\\"low\\\""), "{}", out.js);
    assert!(!out.js.contains("loading=\\\"lazy\\\""));
    let page = "<script>\nimport Img from './custom.brain'\n</script>\n<Img />";
    let root = app(&[("comp/custom.brain", "<p>Custom image component</p>"), ("comp/owner.brain", page), ("tokens.cssbrain", "@tokens { --fg: #111111; --bg: #ffffff; }"), ("app.brain", "<slot />")]);
    let out = brain_codegen::compile_with(page, &root.join("comp/owner.brain"), &brain_codegen::Options::default()).unwrap();
    assert!(out.js.contains("Img(") && !out.js.contains("decoding"), "{}", out.js);
    std::fs::remove_dir_all(root).unwrap();
}

fn img_tag<'a>(js: &'a str, src: &str) -> &'a str {
    let at = js.find(&format!("src=\\\"{src}\\\"")).unwrap_or_else(|| panic!("{src} not found in {js}"));
    let rest = &js[at..];
    &rest[..rest.find('>').unwrap_or(rest.len())]
}

#[test]
fn img_priority_goes_to_the_first_large_image_and_small_ones_stay_eager() {
    let view = "<meta seo title=\"Team\" description=\"The people behind the product.\">\n<main><h1>Team</h1><Img src=\"/logo.png\" alt=\"Logo\" width=\"64\" height=\"64\" /><Img src=\"/hero.jpg\" alt=\"Hero\" width=\"1200\" height=\"630\" /><Img src=\"/b.jpg\" alt=\"B\" width=\"400\" height=\"300\" /></main>\n";
    let o = brain_codegen::compile(view, Path::new("views/team.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    let logo = img_tag(&o.js, "/logo.png");
    assert!(!logo.contains("fetchpriority") && !logo.contains("lazy"), "a small first image neither takes the priority nor waits: {logo}");
    let hero = img_tag(&o.js, "/hero.jpg");
    assert!(hero.contains("fetchpriority=\\\"high\\\"") && !hero.contains("lazy"), "{hero}");
    let rest = img_tag(&o.js, "/b.jpg");
    assert!(rest.contains("loading=\\\"lazy\\\"") && !rest.contains("fetchpriority"), "{rest}");
}

#[test]
fn img_alternatives_each_get_their_own_priority() {
    let view = "<script>\nlet wide = sig(true)\n</script>\n<meta seo title=\"Team\" description=\"The people behind the product.\">\n<main><h1>Team</h1>{#if wide}<Img src=\"/a.jpg\" alt=\"A\" width=\"1200\" height=\"630\" />{:else}<Img src=\"/b.jpg\" alt=\"B\" width=\"1200\" height=\"630\" />{/if}<Img src=\"/c.jpg\" alt=\"C\" width=\"400\" height=\"300\" /></main>\n";
    let o = brain_codegen::compile(view, Path::new("views/team.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    for src in ["/a.jpg", "/b.jpg"] {
        assert!(img_tag(&o.js, src).contains("fetchpriority=\\\"high\\\""), "{}", o.js);
    }
    assert!(img_tag(&o.js, "/c.jpg").contains("loading=\\\"lazy\\\""), "{}", o.js);
}

#[test]
fn a_hero_in_a_component_is_not_lazy_and_takes_no_priority_on_its_own() {
    let comp = "<div><Img src=\"/hero.jpg\" alt=\"Hero\" width=\"1200\" height=\"630\" /><Img src=\"/b.jpg\" alt=\"B\" width=\"400\" height=\"300\" /></div>\n";
    let o = brain_codegen::compile(comp, Path::new("comp/banner.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    let hero = img_tag(&o.js, "/hero.jpg");
    assert!(!hero.contains("lazy") && !hero.contains("fetchpriority"), "{hero}");
    assert!(img_tag(&o.js, "/b.jpg").contains("loading=\\\"lazy\\\""), "{}", o.js);
    let small = "<Img src=\"/icon.png\" alt=\"Icon\" width=\"32\" height=\"32\" />\n";
    let o = brain_codegen::compile(small, Path::new("comp/icon.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(img_tag(&o.js, "/icon.png").contains("loading=\\\"lazy\\\""), "a small image of a component may repeat in a list: {}", o.js);
}

#[test]
fn only_the_first_iteration_of_an_each_takes_the_priority_and_three_are_eager() {
    let page = "<script>\nlet rows = sig(['a', 'b', 'c', 'd', 'e'])\n</script>\n<meta seo title=\"Gallery\" description=\"Every picture in one list.\">\n<main><h1>Gallery</h1>{#each rows as row (row)}<Img src={row} alt={row} width=\"1200\" height=\"630\" />{/each}{#each rows as row (row)}<Img src={row} alt=\"later\" width=\"1200\" height=\"630\" />{/each}</main>\n";
    let root = app(&[("views/gallery.brain", page), ("tokens.cssbrain", "@tokens { --fg: #111111; --bg: #ffffff; }"), ("app.brain", "<slot />")]);
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime").canonicalize().unwrap();
    let opts = brain_codegen::Options { runtime: format!("file://{}/brain.js", runtime.display()), import_suffix: ".mjs".into(), root: Some(root.clone()), ..Default::default() };
    let out = brain_codegen::compile_with(page, &root.join("views/gallery.brain"), &opts).unwrap_or_else(|d| panic!("{d:?}"));
    std::fs::write(root.join("views/gallery.brain.mjs"), &out.js).unwrap();
    let script = format!(r#"
import assert from 'node:assert/strict';
import {{ install }} from 'file://{rt}/shim/dom.js';
const doc = install();
const {{ mount }} = await import('file://{rt}/brain.js');
const {{ default: Page }} = await import('./views/gallery.brain.mjs');
const host = doc.createElement('main');
mount(Page, host);
const all = host.querySelectorAll('img');
assert.equal(all.length, 10);
const first = all.slice(0, 5), later = all.slice(5);
assert.deepEqual(first.map(i => i.getAttribute('fetchpriority')), ['high', null, null, null, null]);
assert.deepEqual(first.map(i => i.getAttribute('loading')), [null, null, null, 'lazy', 'lazy']);
assert.ok(later.every(i => i.getAttribute('loading') === 'lazy' && i.getAttribute('fetchpriority') === null));
"#, rt = runtime.display());
    std::fs::write(root.join("run.mjs"), script).unwrap();
    let result = std::process::Command::new("node").arg(root.join("run.mjs")).output().expect("Node is required");
    assert!(result.status.success(), "{}\n{}", String::from_utf8_lossy(&result.stderr), out.js);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn placeholder_text_is_the_apps_own_words() {
    let chart = "<p>Loaded chart</p>";
    let compile = |attrs: &str| {
        let page = format!("<script>\nimport Chart from './chart.brain'\nconst label = 'Ver gr\u{e1}fica'\n</script>\n<section><Chart {attrs} /></section>");
        let root = app(&[("comp/chart.brain", chart), ("comp/page.brain", &page), ("tokens.cssbrain", "@tokens { --fg: #111111; --bg: #ffffff; }")]);
        let opts = brain_codegen::Options { root: Some(root.clone()), ..Default::default() };
        brain_codegen::compile_with(&page, &root.join("comp/page.brain"), &opts).unwrap_or_else(|d| panic!("{d:?}")).js
    };
    let silent = compile("lazy");
    assert!(!silent.contains("Loading component"), "no fixed English text in the visible placeholder: {silent}");
    let literal = compile("lazy lazyLabel=\"Cargando\"");
    assert!(literal.contains("textContent = \"Cargando\""), "{literal}");
    assert!(!literal.contains("lazyLabel"), "the label is not a prop of the child: {literal}");
    let button = compile("lazy=\"interaction\" lazyLabel={label}");
    assert!(button.contains("textContent = label") || button.contains("textContent = (label)") , "{button}");
    assert!(!button.contains("Load component"), "{button}");
}
