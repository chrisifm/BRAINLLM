//! #85 (INTERNALS §28): per-block hashes, change classification and the style-only fast path.

use brain_codegen::blocks::{blocks, cache_key, change, compile_style, Change};
use std::path::Path;

const BASE: &str = r#"<meta props="label:str count:num=0" emits="select">
<script>
let n = sig(count)
</script>
<button on:click={n++}>{label}: {n}</button>
<slot />
<style>
  button { padding: var(--space-1); }
</style>"#;

fn b(src: &str) -> brain_codegen::blocks::Blocks {
    blocks(src, Path::new("comp/x.brain"))
}

#[test]
fn identical_source_is_no_change() {
    assert_eq!(change(&b(BASE), &b(BASE)), Change::None);
}

#[test]
fn each_edit_maps_to_the_cheapest_change() {
    let cases = [
        (BASE.replace("var(--space-1)", "var(--space-2)"), Change::Style),
        (BASE.replace("{label}: {n}", "{label} = {n}"), Change::Component),
        (BASE.replace("let n = sig(count)", "let n = sig(count + 1)"), Change::Component),
        (BASE.replace("count:num=0", "count:num=1"), Change::Contract),
        (BASE.replace("label:str", "label:str?"), Change::Contract),
        (BASE.replace(r#"emits="select""#, r#"emits="select close""#), Change::Contract),
        (BASE.replace("<slot />", r#"<slot name="footer" />"#), Change::Contract),
        // Whitespace inside the meta tag changes the meta block, not the contract.
        (BASE.replace(r#"<meta props="#, r#"<meta  props="#), Change::Component),
    ];
    for (src, want) in cases {
        assert_eq!(change(&b(BASE), &b(&src)), want, "{src}");
    }
}

#[test]
fn compile_output_carries_the_blocks_and_style_fast_path_matches() {
    let o = brain_codegen::compile(BASE, Path::new("comp/x.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert_eq!(o.blocks, b(BASE));
    let css = compile_style(BASE, Path::new("comp/x.brain"), &Default::default()).unwrap_or_else(|d| panic!("{d:#?}"));
    assert_eq!(css, o.css);
    assert!(!css.is_empty());
}

#[test]
fn cache_key_depends_on_options_and_content() {
    let p = Path::new("comp/x.brain");
    let prod = brain_codegen::Options::default();
    let dev = brain_codegen::Options { dev: true, ..Default::default() };
    assert_ne!(cache_key(&b(BASE), p, &prod), cache_key(&b(BASE), p, &dev));
    assert_eq!(cache_key(&b(BASE), p, &prod), cache_key(&b(BASE), p, &prod));
    assert_ne!(cache_key(&b(BASE), p, &prod), cache_key(&b(&BASE.replace("{n}", "{n}!")), p, &prod));
}

#[test]
fn modules_and_stylesheets_are_single_blocks() {
    let js = "export const x = 1\n";
    let a = blocks(js, Path::new("lib/a.jsbrain"));
    let b2 = blocks("export const x = 2\n", Path::new("lib/a.jsbrain"));
    assert_ne!(a.script, b2.script);
    assert_eq!(a.markup, b2.markup);
    let rename = blocks("export const y = 1\n", Path::new("lib/a.jsbrain"));
    assert_eq!(change(&a, &b2), Change::Component);
    assert_eq!(change(&a, &rename), Change::Contract);
    let c1 = blocks(":root { --x: 1px; }", Path::new("tokens.cssbrain"));
    let c2 = blocks(":root { --x: 2px; }", Path::new("tokens.cssbrain"));
    assert_eq!(change(&c1, &c2), Change::Style);
}

#[test]
fn cache_key_changes_with_public_env_values() {
    // TOOLCHAIN #138: a staging build must never reuse a production entry (and the key holds no value).
    let p = Path::new("comp/x.brain");
    let env = |v: &str| brain_codegen::Options { public_env: vec![("PUBLIC_SITE_URL".into(), v.into())], ..Default::default() };
    let a = cache_key(&b(BASE), p, &env("https://staging.example.com"));
    let c = cache_key(&b(BASE), p, &env("https://example.com"));
    assert_ne!(a, c);
    assert!(!a.contains("example"));
}

#[test]
fn cache_key_changes_with_locale_and_texts() {
    let p = Path::new("comp/x.brain");
    let loc = |l: &str, v: &str| brain_codegen::Options { i18n: Some((l.into(), [("k".to_string(), v.to_string())].into_iter().collect())), ..Default::default() };
    let a = cache_key(&b(BASE), p, &loc("en", "Home"));
    assert_ne!(a, cache_key(&b(BASE), p, &loc("es", "Home")));
    assert_ne!(a, cache_key(&b(BASE), p, &loc("en", "Start")));
}

#[test]
fn persistent_keys_cover_adoption_lint_and_field_boundaries() {
    use brain_codegen::blocks::source_cache_key;
    let p = Path::new("comp/x.brain");
    let base = brain_codegen::Options::default();
    let key = source_cache_key(BASE, p, &base);
    let variants = [
        brain_codegen::Options { adopt: true, ..Default::default() },
        brain_codegen::Options { lint: vec![("B1001".into(), None)], ..Default::default() },
        brain_codegen::Options { root: Some("other".into()), ..Default::default() },
    ];
    for opts in variants {
        assert_ne!(key, source_cache_key(BASE, p, &opts));
        assert_ne!(cache_key(&b(BASE), p, &base), cache_key(&b(BASE), p, &opts));
    }
    let env = |k: &str, v: &str| brain_codegen::Options { public_env: vec![(k.into(), v.into())], ..Default::default() };
    assert_ne!(source_cache_key(BASE, p, &env("A", "B=C")), source_cache_key(BASE, p, &env("A=B", "C")));
    assert_ne!(key, source_cache_key(&format!("{BASE}\n"), p, &base));
    assert_eq!(key, source_cache_key(BASE, p, &base));
}
