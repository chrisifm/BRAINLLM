//! #234 (§21 normalizations for the LLM gate): a view's `<main>` under a layout that already has one becomes
//! `<section>`, and a raw `<input>` without a label becomes the kit `<Input label=…>` (B0403/B0405, info + edit).
use std::path::Path;

fn app(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("brain-a11y-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (rel, src) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
    root
}

const SHELL: &str = "<!doctype html>\n<html lang=\"en\">\n<head><title>x</title></head>\n<body><slot /></body>\n</html>\n";
const VIEW: &str = "<meta seo title=\"About\" description=\"About us and the team.\">\n<main class=\"page\">\n  <h1>About</h1>\n</main>\n";

#[test]
fn view_main_under_a_layout_main_becomes_section() {
    let root = app("main", &[("app.brain", SHELL), ("brain.json", "{}"), ("views/_layout.brain", "<main><slot /></main>\n"), ("views/about.brain", VIEW)]);
    let path = root.join("views/about.brain");
    let c = brain_parse::parse(VIEW, &path).unwrap();
    let (ctx, _) = brain_check::load_ctx(&path, &c);
    assert!(ctx.wrapper_main);
    let d: Vec<_> = brain_check::check(&c, &ctx).into_iter().filter(|d| d.code == "B0405").collect();
    assert_eq!(d.len(), 1, "{d:#?}");
    assert_eq!(d[0].level, brain_parse::Severity::Info);
    let o = brain_codegen::compile_with(VIEW, &path, &brain_codegen::Options::default());
    std::fs::remove_dir_all(&root).ok();
    let o = o.unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("<section") && !o.js.contains("<main"), "{}", o.js);

    // Without a wrapper `<main>`, the view keeps its own.
    let root = app("nomain", &[("app.brain", SHELL), ("brain.json", "{}"), ("views/about.brain", VIEW)]);
    let path = root.join("views/about.brain");
    let c = brain_parse::parse(VIEW, &path).unwrap();
    let (ctx, _) = brain_check::load_ctx(&path, &c);
    std::fs::remove_dir_all(&root).ok();
    assert!(!ctx.wrapper_main);
    assert!(brain_check::check(&c, &ctx).iter().all(|d| d.code != "B0405"));
}

#[test]
fn raw_input_becomes_the_kit_input() {
    let src = "<script>\n  let email = sig('')\n</script>\n<form><input type=\"email\" placeholder=\"Your email…\" bind:value={email}><button>Join</button></form>\n";
    let c = brain_parse::parse(src, Path::new("comp/join.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0403").collect();
    assert_eq!(d.len(), 1, "{d:#?}");
    assert_eq!(d[0].level, brain_parse::Severity::Info);
    assert_eq!(d[0].edits[0].text, "<Input label=\"Your email\" type=\"email\" placeholder=\"Your email…\" bind:value={email} />");
    let o = brain_codegen::compile(src, Path::new("comp/join.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("/ui/input.brain"), "{}", o.js);

    // The label can come from the name.
    let by_name = "<script>\n  let v = sig('')\n</script>\n<input name=\"first_name\" bind:value={v}>\n";
    let c = brain_parse::parse(by_name, Path::new("comp/n.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0403").collect();
    assert!(d[0].edits[0].text.starts_with("<Input label=\"First name\""), "{d:#?}");

    // Nothing to build a label from, or attributes the kit does not take: still an error with a fix.
    for raw in ["<input bind:value={v}>", "<input class=\"x\" placeholder=\"Find\" bind:value={v}>", "<select><option>a</option></select>", "<textarea placeholder=\"Notes\"></textarea>"] {
        let src = format!("<script>\n  let v = sig('')\n</script>\n{raw}\n");
        let c = brain_parse::parse(&src, Path::new("comp/r.brain")).unwrap();
        let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0403").collect();
        assert_eq!(d.len(), 1, "{raw}: {d:#?}");
        assert_eq!(d[0].level, brain_parse::Severity::Error, "{raw}");
    }
}
