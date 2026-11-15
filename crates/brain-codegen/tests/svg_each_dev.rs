//! A block fragment inside <svg> ({#each}/{#if}) keeps the SVG namespace in dev too: fragment() rebuilds the
//! template from $hmr, so it receives the same svg flag as tpl() and re-wraps the raw HTML before parsing.

use std::path::Path;

const VIEW: &str = r#"<script>
let ls = sig([{ id: 'a', d: 'M0 0 L100 50' }])
</script>
<svg width="200" height="100">{#each ls as l (l.id)}<path d={l.d} stroke="black" />{/each}</svg>"#;

#[test]
fn svg_block_fragment_passes_the_svg_flag_to_the_dev_fragment() {
    let dev = brain_codegen::Options { dev: true, ..Default::default() };
    let o = brain_codegen::compile_with(VIEW, Path::new("comp/lines.brain"), &dev).unwrap_or_else(|d| panic!("{d:#?}"));
    let line = o.js.lines().find(|l| l.contains("<svg><path")).unwrap_or_else(|| panic!("no wrapped svg template:\n{}", o.js));
    assert!(line.ends_with(", 1);"), "dev fragment() must get the svg flag: {line}");
}

#[test]
fn svg_block_fragment_stays_wrapped_in_production() {
    let o = brain_codegen::compile(VIEW, Path::new("comp/lines.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("<svg><path"), "{}", o.js);
}
