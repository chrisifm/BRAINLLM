//! #143 (with RUNTIME #285): in adopt mode node references walk with `fc`/`nx` from brain-hydrate.js; the default
//! output is unchanged. `Options.lint` tunes B09–B13 before the compile stops on errors (TOOLCHAIN).
use std::path::Path;

const SRC: &str = "<script>\n  let n = sig(0)\n  let open = sig(false)\n</script>\n<section><h2>Count</h2><p>{n}</p><button on:click={n++}>+</button>{#if open}<p>x</p>{:else if n > 2}<p>y</p>{/if}</section>\n";

#[test]
fn adopt_mode_walks_with_fc_and_nx() {
    let opts = brain_codegen::Options { adopt: true, ..Default::default() };
    let o = brain_codegen::compile_with(SRC, Path::new("comp/c.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("import { fc, nx } from \"/@brain/brain-hydrate.js\""), "{}", o.js);
    assert!(o.js.contains("fc(") && o.js.contains("nx(") && !o.js.contains(".firstChild") && !o.js.contains(".nextSibling"), "{}", o.js);
    let plain = brain_codegen::compile(SRC, Path::new("comp/c.brain")).unwrap();
    assert!(!plain.js.contains("brain-hydrate") && plain.js.contains(".firstChild"), "{}", plain.js);
}

#[test]
fn lint_overrides_tunable_families() {
    let src = "<script>\n  const posts = { get: () => Promise.resolve([]) }\n  let list = resource(posts.get)\n</script>\n{#await list}<p>…</p>{:then xs}<p>{xs}</p>{/await}\n";
    assert!(brain_codegen::compile(src, Path::new("comp/l.brain")).is_err(), "B1101 blocks by default");
    let warn = brain_codegen::Options { lint: vec![("B1101".into(), Some(brain_parse::Severity::Warning))], ..Default::default() };
    let o = brain_codegen::compile_with(src, Path::new("comp/l.brain"), &warn).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.warnings.iter().any(|d| d.code == "B1101"));
    let off = brain_codegen::Options { lint: vec![("B1101".into(), None)], ..Default::default() };
    let o = brain_codegen::compile_with(src, Path::new("comp/l.brain"), &off).unwrap();
    assert!(o.warnings.iter().all(|d| d.code != "B1101"));
    // Outside B09–B13 the catalog level stays.
    let bad = brain_codegen::Options { lint: vec![("B0111".into(), None)], ..Default::default() };
    assert!(brain_codegen::compile_with("<p>{nope}</p>\n", Path::new("comp/x.brain"), &bad).is_err());
}
