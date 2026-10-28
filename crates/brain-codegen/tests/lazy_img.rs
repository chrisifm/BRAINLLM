//! #66 (§22): `<Chart lazy />` loads its chunk on a trigger; built-in `<Img>` gets decoding/loading/fetchpriority.
use std::path::Path;

fn app(files: &[(&str, &str)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("brain-lazy-{}-{}", std::process::id(), files.len()));
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
