//! Parser-level diagnostics: code, position and a non-empty fix.

use std::path::Path;

fn err(src: &str) -> brain_parse::Diag {
    let ds = brain_parse::parse(src, Path::new("comp/x.brain")).expect_err("should have failed");
    let d = ds[0].clone();
    assert!(!d.fix.is_empty(), "empty fix in {d}");
    d
}

#[test]
fn prohibited_constructs() {
    assert_eq!(err("<script>class A {}</script>").code, "B0101");
    assert_eq!(err("<script>with (a) {}</script>").code, "B0104");
    assert_eq!(err("<script>const f = new Function('a')</script>").code, "B0105");
    assert_eq!(err("<script>function* g() {}</script>").code, "B0106");
    assert_eq!(err("<script>const o = { get x() { return 1 } }</script>").code, "B0107");
    assert_eq!(err("<script>outer: for (const a of b) {}</script>").code, "B0108");
    assert_eq!(err("<script>for (const k in o) {}</script>").code, "B0109");
    assert_eq!(err("<script>for (let i = 0; i < 3; i++) {}</script>").code, "B0110");
    assert_eq!(err("<script>while (x) {}</script>").code, "B0110");
}

#[test]
fn positions_are_exact() {
    let d = err("<script>\n  let a = 1\n  class B {}\n</script>");
    assert_eq!((d.code, d.line, d.col), ("B0101", 3, 3));
    assert_eq!(d.file, "comp/x.brain");
}

#[test]
fn markup_structure() {
    assert_eq!(err("<div><span></div>").code, "B0003");
    assert_eq!(err("{#if a}<p>x</p>").code, "B0003");
    assert_eq!(err("<p>{a b}</p>").code, "B0002");
    assert_eq!(err("<meta props=\"n:number=0\">").code, "B0004");
    assert_eq!(err("<meta props=\"n=0\">").code, "B0004");
}

#[test]
fn expressions_and_blocks() {
    let src = r#"<meta props="items:list=[] sel:num?">
<script>
  let n = sig(0)
  const f = async ({ a, b: [c, ...d] = [] }, ...rest) => { for (const x of rest) { if (!x) continue } return a ?? c }
  const t = `a${n + 1}b${`${f}`}`
  let x = a ? b : c ? d : e ** 2 ** 3
</script>
{#each items as it, i (it.id)}<li>{i}</li>{:empty}<p/>{/each}
{#await f()}…{:then v}{v}{:catch e}{e.message}{/await}
{#if a}A{:else if b}B{:else}C{/if}
<Child x={1} on:pick={f}><slot /></Child>
<input bind:value={n} class:on={n > 1} style:color={c} title="n={n}!">"#;
    let c = brain_parse::parse(src, Path::new("comp/x.brain")).unwrap();
    let d = brain_parse::dump::dump(&c);
    assert!(d.contains("let x = (a ? b : (c ? d : (e ** (2 ** 3))))"), "{d}");
    assert!(d.contains("each@8:1 items as it, i (it.id)"), "{d}");
    assert!(d.contains("component Child@11:1 x={1} on:pick={f}"), "{d}");
    assert!(d.contains("title=\"n={n}!\""), "{d}");
    assert!(d.contains("prop@1:14 items:list = []"), "{d}");
}
