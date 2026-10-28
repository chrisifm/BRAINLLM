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
