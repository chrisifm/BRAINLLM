//! The production core includes attribution and must remain inside the public 4 KiB gzip invariant.
#[test]
fn branded_minified_runtime_core_stays_within_four_kib_gzip() {
    let source = include_str!("../../../runtime/brain.js");
    let code = brain_lex::branding::code(&brain_min::minify_runtime_core(source).unwrap());
    let compressed = brain_pack::gzip::gzip(code.as_bytes(), 15);
    assert!(compressed.len() <= 4096, "runtime core is {} B gzip; keep optional behavior in sibling modules", compressed.len());
    assert!(code.contains(brain_lex::branding::CODE));
}
