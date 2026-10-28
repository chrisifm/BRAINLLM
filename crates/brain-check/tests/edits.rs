//! #57: every mechanical fix carries exact edits; applying them makes that diagnostic disappear.

use brain_check::Ctx;
use brain_parse::Diag;
use std::path::Path;

fn check(path: &str, src: &str, ctx: &Ctx) -> Vec<Diag> {
    match brain_parse::parse(src, Path::new(path)) {
        Ok(c) => brain_check::check(&c, ctx),
        Err(d) => d,
    }
}

/// Applies the edits of the first diagnostic with `code` and returns the new source.
fn fixed(path: &str, src: &str, ctx: &Ctx, code: &str) -> String {
    let d = check(path, src, ctx);
    let d = d.iter().find(|d| d.code == code).unwrap_or_else(|| panic!("missing {code}: {d:?}"));
    assert!(!d.edits.is_empty(), "{code} without edits: {d:?}");
    for e in &d.edits {
        assert!(e.line > 0 && e.col > 0 && e.end_line >= e.line, "{e:?}");
    }
    let out = Diag::apply(src, &d.edits).expect("applicable edits");
    assert!(!check(path, &out, ctx).iter().any(|x| x.code == code), "{code} remains after applying:\n{out}");
    out
}

fn tokens() -> Ctx {
    let mut ctx = Ctx::default();
    ctx.tokens = Some(["--space-2", "--line", "--fg", "--bg"].iter().map(|t| (t.to_string(), "1px".to_string())).collect());
    ctx
}

#[test]
fn mechanical_fixes_apply() {
    let c = Ctx::default();
    assert_eq!(fixed("comp/x.brain", "<script>var n = 1</script><p>{n}</p>", &c, "B0103"), "<script>let n = 1</script><p>{n}</p>");
    assert_eq!(
        fixed("comp/x.brain", "<script>let xs = sig([])</script>{#each xs as it}<p>{it.name}</p>{/each}", &c, "B0202"),
        "<script>let xs = sig([])</script>{#each xs as it (it.id)}<p>{it.name}</p>{/each}"
    );
    assert_eq!(
        fixed("comp/x.brain", "<script>let tags = sig([])</script>{#each tags as t, i}<p>{t}</p>{/each}", &c, "B0202"),
        "<script>let tags = sig([])</script>{#each tags as t, i (t)}<p>{t}</p>{/each}"
    );
    assert_eq!(
        fixed("comp/x.brain", "<script>let xs = sig([])\nconst add = (x) => xs.push(x, 2)</script>", &c, "B0113"),
        "<script>let xs = sig([])\nconst add = (x) => xs = [...xs, x, 2]</script>"
    );
    assert_eq!(fixed("comp/x.brain", "<script>let count = sig(0)</script><p>{cuont}</p>", &c, "B0111"), "<script>let count = sig(0)</script><p>{count}</p>");
    assert_eq!(
        fixed("comp/x.brain", "<script>server: function f(ctx) { return 1 }</script><button on:click={f()}>x</button>", &c, "B0704"),
        "<script>server: async function f(ctx) { return 1 }</script><button on:click={f()}>x</button>"
    );
    assert_eq!(
        fixed("comp/x.brain", "<p>x</p><style>p { border: var( --border) solid var(--border) }</style>", &tokens(), "B0302"),
        "<p>x</p><style>p { border: var( --line) solid var(--line) }</style>"
    );
    assert_eq!(fixed("comp/x.brain", "<p>x</p><style>p { padding: var(--sp-2) }</style>", &tokens(), "B0302"), "<p>x</p><style>p { padding: var(--space-2) }</style>");
}

#[test]
fn judgment_fixes_have_no_edits() {
    let d = check("comp/x.brain", "<img src=\"/a.png\">", &Ctx::default());
    assert_eq!(d[0].code, "B0401");
    assert!(d[0].edits.is_empty());
    assert!(d[0].to_json().ends_with(",\"level\":\"error\",\"edits\":[]}"), "{}", d[0].to_json());
}

#[test]
fn b0119_edit_points_to_the_existing_file() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("edits-b0119");
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::write(root.join("lib/format.jsbrain"), "export const f = 1\n").unwrap();
    let src = "<script>import f from '../lib/formats.jsbrain'</script>{f}";
    let c = brain_parse::parse(src, Path::new("comp/x.brain")).unwrap();
    let (ctx, _) = brain_check::load_ctx(&root.join("comp/x.brain"), &c);
    assert_eq!(fixed("comp/x.brain", src, &ctx, "B0119"), "<script>import f from '../lib/format.jsbrain'</script>{f}");
}
