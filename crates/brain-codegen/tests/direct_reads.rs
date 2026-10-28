//! One form for every signal (team lead, #170): `media()`, `debounce()`, `stream()` and `visible()` bind like a `drv` and
//! are read directly (`{wide}`, `if (wide)`); only named extras use a property (`feed.connected`). An explicit `.v`
//! written by the author is B0130 with an edit that removes it, and compiles to the same single read.
use std::path::Path;

const SRC: &str = "<script>
  let q = sig('')
  let box = ref()
  let wide = media('(max-width: 600px)')
  let slow = debounce(q, '300ms')
  let feed = stream('/events')
  let seen = visible(box)
  const label = () => (wide ? 'narrow' : 'wide')
</script>
<input bind:value={q} aria-label=\"Search\" />
<section bind:ref={box}>
  {#if wide}<p>small</p>{/if}
  <p>{slow} {feed} {seen ? 'seen' : ''} {feed.connected ? 'live' : 'offline'} {label()}</p>
</section>
";

#[test]
fn the_four_helpers_are_read_directly() {
    let c = brain_parse::parse(SRC, Path::new("comp/live.brain")).unwrap();
    let d = brain_check::check(&c, &brain_check::Ctx::default());
    assert!(d.iter().all(|d| d.level != brain_parse::Severity::Error), "{d:#?}");
    let o = brain_codegen::compile(SRC, Path::new("comp/live.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    let js = &o.js;
    for (want, why) in [
        ("const wide = media('(max-width: 600px)');", "media() binds the node"),
        ("const slow = debounce(() => q.v, '300ms');", "debounce() takes a getter"),
        ("const feed = stream('/events');", "stream() binds the node"),
        ("const seen = visible(box);", "visible() takes the ref"),
        ("wide.v ? 'narrow' : 'wide'", "script reads unwrap"),
        ("() => wide.v", "{#if wide} tracks the node"),
        ("feed.connected", "named extras stay properties"),
    ] {
        assert!(js.contains(want), "{why}: missing `{want}` in\n{js}");
    }
    for bad in ["feed.v.connected", "slow.v.v", "wide.v.v"] {
        assert!(!js.contains(bad), "`{bad}` in\n{js}");
    }
}

#[test]
fn explicit_v_is_b0130_and_compiles_the_same() {
    let src = "<script>\n  let wide = media('(max-width: 600px)')\n  let n = sig(0)\n  const f = form({ email: 'email!' })\n</script>\n<p>{wide.v ? 'a' : 'b'} {n.v + 1} {f.v.email}</p>\n";
    let c = brain_parse::parse(src, Path::new("comp/v.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0130").collect();
    // `form()` fields are written `f.v.email` by design: no warning there.
    assert_eq!(d.len(), 2, "{d:#?}");
    assert!(d.iter().all(|d| d.edits.len() == 1 && d.edits[0].text.is_empty()));
    let fixed = brain_lex_apply(src, &d);
    assert!(fixed.contains("{wide ? 'a' : 'b'} {n + 1} {f.v.email}"), "{fixed}");
    let o = brain_codegen::compile(src, Path::new("comp/v.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("wide.v ? 'a' : 'b'") && o.js.contains("n.v + 1") && !o.js.contains(".v.v"), "{}", o.js);
}

/// Applies the edits of `d` back to front.
fn brain_lex_apply(src: &str, d: &[brain_parse::Diag]) -> String {
    let mut edits: Vec<&brain_parse::Edit> = d.iter().flat_map(|d| d.edits.iter()).collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
    let mut s = src.to_string();
    for e in edits {
        s.replace_range(e.start as usize..e.end as usize, &e.text);
    }
    s
}
