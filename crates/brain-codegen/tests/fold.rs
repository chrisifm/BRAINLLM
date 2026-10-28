//! #34 (INTERNALS §13): production folding of signals never written, constant derived values and effects
//! without reactive reads; dev keeps everything reactive (HMR).

use std::path::Path;

const SRC: &str = r#"<meta props="start:num=0">
<script>
let title = sig('Orders')
let tax = sig(0.16)
let rate = drv(() => tax * 100)
let n = sig(start)
let double = drv(() => n * 2)
let label = drv(() => title + ': ' + rate + '%')
fx(() => { log('ready') })
fx(() => { log(n) })
fx(() => { const id = setInterval(() => {}, 1000); return () => clearInterval(id) })
</script>
<h2>{label}</h2>
<button on:click={n++}>{n} {double}</button>"#;

#[test]
fn constants_fold_in_production() {
    let o = brain_codegen::compile(SRC, Path::new("comp/fold.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    let js = &o.js;
    assert!(js.contains("const title = 'Orders';"), "{js}");
    assert!(js.contains("const tax = 0.16;"), "{js}");
    assert!(js.contains("const rate = tax * 100;"), "{js}");
    assert!(js.contains("const label = title + ': ' + rate + '%';"), "{js}");
    // Written or derived from a written sig: stays reactive.
    assert!(js.contains("const n = sig("), "{js}");
    assert!(js.contains("const double = drv("), "{js}");
    // The pure effect runs inline; the reactive one and the one with cleanup stay effects.
    assert_eq!(js.matches("fx(").count(), 2, "{js}");
    // A folded value is read directly, without `.v`.
    assert!(!js.contains("label.v") && !js.contains("title.v"), "{js}");
}

#[test]
fn dev_keeps_signals_for_hmr() {
    let opts = brain_codegen::Options { dev: true, ..Default::default() };
    let o = brain_codegen::compile_with(SRC, Path::new("comp/fold.brain"), &opts).unwrap();
    assert!(o.js.contains("const title = sig('Orders', \"title\")"), "{}", o.js);
}

#[test]
fn provided_and_bound_signals_are_not_folded() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fold-provide");
    std::fs::create_dir_all(root.join("channels")).unwrap();
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::write(root.join("channels/theme.jsbrain"), "export const Theme = key('theme')\n").unwrap();
    let src = "<script>import { Theme } from '../channels/theme.jsbrain'\nlet theme = sig('light')\nlet q = sig('')\nprovide(Theme, theme)</script><input aria-label=\"q\" bind:value={q}>";
    let opts = brain_codegen::Options { root: Some(root), ..Default::default() };
    let o = brain_codegen::compile_with(src, Path::new("comp/p.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("const theme = sig(") && o.js.contains("const q = sig("), "{}", o.js);
}

#[test]
fn source_map_points_at_brain_lines() {
    // #36 (INTERNALS §18): generated lines map back to the `.brain` line of the code they come from.
    let src = "<script>\nlet n = sig(0)\nconst inc = () => { n++ }\n</script>\n<button on:click={inc}>{n}</button>";
    let o = brain_codegen::compile(src, std::path::Path::new("comp/c.brain")).unwrap();
    assert!(!o.js.contains("/*@L"), "markers must be stripped:\n{}", o.js);
    assert!(o.map.starts_with("{\"version\":3,\"file\":\"c.brain.js\",\"sources\":[\"c.brain\"]"), "{}", o.map);
    // Decode the mappings: generated line → source line (0-based).
    let mappings = o.map.split("\"mappings\":\"").nth(1).unwrap().split('"').next().unwrap();
    let mut src_line = 0i64;
    let mut by_gen: Vec<Option<i64>> = Vec::new();
    for seg in mappings.split(';') {
        if seg.is_empty() {
            by_gen.push(None);
            continue;
        }
        let mut vals = Vec::new();
        let (mut shift, mut value) = (0, 0i64);
        for ch in seg.chars() {
            let d = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".find(ch).unwrap() as i64;
            value += (d & 31) << shift;
            if d & 32 != 0 {
                shift += 5;
            } else {
                vals.push(if value & 1 == 1 { -(value >> 1) } else { value >> 1 });
                shift = 0;
                value = 0;
            }
        }
        src_line += vals[2];
        by_gen.push(Some(src_line));
    }
    let lines: Vec<&str> = o.js.lines().collect();
    let line_of = |needle: &str| lines.iter().position(|l| l.contains(needle)).and_then(|i| by_gen[i]).map(|l| l + 1);
    assert_eq!(line_of("const n = sig("), Some(2));
    assert_eq!(line_of("const inc ="), Some(3));
    assert_eq!(line_of("on($e"), Some(5));
    // Dev inlines it for the browser.
    let dev = brain_codegen::compile_with(src, std::path::Path::new("comp/c.brain"), &brain_codegen::Options { dev: true, ..Default::default() }).unwrap();
    assert!(dev.js.contains("//# sourceMappingURL=data:application/json;base64,"), "{}", dev.js);
}

#[test]
fn public_env_values_compile_to_constants() {
    // #137 (INTERNALS §36): the build's PUBLIC_* values replace their reads in client code.
    let src = "<script>\nconst site = import.meta.env.PUBLIC_SITE_NAME\nconst other = process.env.PUBLIC_MISSING\n</script>\n<p>{site} {other}</p>";
    let opts = brain_codegen::Options { public_env: vec![("PUBLIC_SITE_NAME".into(), "Nimbus".into())], ..Default::default() };
    let o = brain_codegen::compile_with(src, std::path::Path::new("comp/s.brain"), &opts).unwrap();
    assert!(o.js.contains("const site = \"Nimbus\";"), "{}", o.js);
    // Unknown keys stay as written (the host may provide them at runtime).
    assert!(o.js.contains("process.env.PUBLIC_MISSING"), "{}", o.js);
}

#[test]
fn source_map_covers_statements_inside_nested_functions() {
    // RUNTIME #158: statements of an arrow inside an expression map to their own lines.
    let src = "<script>\nlet saved = sig('')\nconst save = async (v) => {\n  await fetch('/x')\n  saved = 'ok'\n}\n</script>\n<button on:click={save(1)}>{saved}</button>";
    let o = brain_codegen::compile(src, std::path::Path::new("comp/n.brain")).unwrap();
    let mappings = o.map.split("\"mappings\":\"").nth(1).unwrap().split('"').next().unwrap().to_string();
    let lines: Vec<&str> = o.js.lines().collect();
    let mut src_line = 0i64;
    let mut by_gen: Vec<Option<i64>> = Vec::new();
    for seg in mappings.split(';') {
        if seg.is_empty() {
            by_gen.push(None);
            continue;
        }
        let mut vals = Vec::new();
        let (mut shift, mut value) = (0, 0i64);
        for ch in seg.chars() {
            let d = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".find(ch).unwrap() as i64;
            value += (d & 31) << shift;
            if d & 32 != 0 {
                shift += 5;
            } else {
                vals.push(if value & 1 == 1 { -(value >> 1) } else { value >> 1 });
                shift = 0;
                value = 0;
            }
        }
        src_line += vals[2];
        by_gen.push(Some(src_line));
    }
    let line_of = |needle: &str| lines.iter().position(|l| l.contains(needle)).and_then(|i| by_gen[i]).map(|l| l + 1);
    assert_eq!(line_of("await fetch"), Some(4), "{}", o.js);
    assert_eq!(line_of("saved.v = 'ok'"), Some(5), "{}", o.js);
}
