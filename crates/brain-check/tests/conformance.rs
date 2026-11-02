//! #97: rules the SPEC serves that QA measured as not enforced (bench/conformance.md rows 22, 23, 24, 37, 93, 99).

use brain_check::Ctx;
use brain_parse::Severity;
use std::path::Path;

fn diags(path: &str, src: &str, ctx: &Ctx) -> Vec<brain_parse::Diag> {
    let c = brain_parse::parse(src, Path::new(path)).unwrap_or_else(|d| panic!("{d:?}"));
    brain_check::check(&c, ctx)
}

fn codes(path: &str, src: &str) -> Vec<&'static str> {
    diags(path, src, &Ctx::default()).into_iter().map(|d| d.code).collect()
}

fn tokens() -> Ctx {
    let t = [("--space-1", "4px"), ("--space-2", "8px"), ("--fg", "#1a1a1a"), ("--bg", "#f5f5f7"), ("--muted", "#6b7280"), ("--bad", "#d1242f")];
    Ctx { tokens: Some(t.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()), ..Ctx::default() }
}

#[test]
fn row22_one_export_per_module() {
    assert_eq!(codes("lib/two.jsbrain", "export const a = 1\nexport const b = 2\n"), ["B0124"]);
    assert!(codes("lib/one.jsbrain", "const a = 1\nconst b = 2\nexport default { a, b }\n").is_empty());
    // The type catalog is the exception: one export per type.
    let types = "export const User = { id: 'num!' }\nexport const Post = { id: 'num!' }\nexport const Tag = { name: 'str!' }\n";
    assert!(codes("types.jsbrain", types).is_empty());
    assert_eq!(codes("lib/types-copy.jsbrain", types)[0], "B0124");
}

#[test]
fn row23_block_order() {
    assert_eq!(codes("comp/x.brain", "<p>{n}</p>\n<script>let n = sig(0)</script>"), ["B0125"]);
    assert_eq!(codes("comp/x.brain", "<p>{a}</p>\n<style>p { color: var(--fg) }</style>\n<meta props=\"a:str?\">"), ["B0125"]);
    assert!(codes("comp/x.brain", "<meta props=\"a:str?\">\n<script>let n = sig(0)</script>\n<p>{n}{a}</p>\n<style>p { color: var(--fg) }</style>").is_empty());
    // HTML <meta>/<script src> inside app.brain's <head> are markup.
    let app = "<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"></head><body><slot /></body></html>";
    assert!(!codes("app.brain", app).contains(&"B0125"), "{:?}", codes("app.brain", app));
}

#[test]
fn row24_markup_is_required() {
    assert_eq!(codes("comp/x.brain", "<script>let n = sig(0)</script>"), ["B0126"]);
    assert!(!codes("comp/x.test.brain", "<script>test('a', async () => {})</script>").contains(&"B0126"), "{:?}", codes("comp/x.test.brain", "<script>test('a', async () => {})</script>"));
}

#[test]
fn row37_export_only_in_modules() {
    let d = diags("comp/x.brain", "<script>export const a = 1</script><p>{a}</p>", &Ctx::default());
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["B0120"]);
    assert_eq!(d[0].edits.len(), 1, "the edit removes `export `");
    // `load` outside a view is B0117's case.
    assert_eq!(codes("comp/x.brain", "<script>export async function load(ctx) { return {} }</script><p>x</p>"), ["B0117"]);
    // a layout's `load` never runs (only the route's view loads): B0117 instead of a silently empty prop.
    assert_eq!(codes("views/_layout.brain", "<script>export async function load(ctx) { return {} }</script><slot />"), ["B0117"]);
    assert_eq!(codes("views/admin/_layout.brain", "<script>export async function load(ctx) { return {} }</script><slot />"), ["B0117"]);
    // `load` in a view is the one allowed export.
    assert!(!codes("views/x.brain", "<meta seo title=\"T\" description=\"D\" canonical=\"/x\"><script>export async function load(ctx) { return {} }</script><main><h1>x</h1></main>").contains(&"B0120"));
}

#[test]
fn row93_colors_and_spacing_from_tokens() {
    let ctx = tokens();
    let d = diags("comp/x.brain", "<p class=\"a\">x</p><style>.a { color: #d1242f; padding: 4px 8px; margin: 3px; gap: 0; width: 10px; box-shadow: 0 1px 2px #000; }</style>", &ctx);
    let got: Vec<(&str, Severity)> = d.iter().map(|d| (d.code, d.level)).collect();
    assert_eq!(got, [("B0304", Severity::Error), ("B0305", Severity::Info), ("B0305", Severity::Warning)], "{d:#?}");
    assert_eq!(d[0].edits[0].text, "var(--bad)");
    assert_eq!(d[1].edits.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["var(--space-1)", "var(--space-2)"]);
    // Shorthands and named colors: the nearest token is named in the fix, never written (it changes the color).
    let d = diags("comp/x.brain", "<p class=\"a\">x</p><style>.a { border: 1px solid red; }</style>", &ctx);
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["B0304"]);
    assert!(d[0].edits.is_empty() && d[0].fix.contains("var(--bad)"), "{:#?}", d[0]);
    // Allowed: tokens, keywords, 0/auto/%.
    assert!(diags("comp/x.brain", "<p class=\"a\">x</p><style>.a { color: var(--fg); background: transparent; margin: 0 auto; padding: 5%; border-color: currentColor; }</style>", &ctx).is_empty());
}

#[test]
fn row99_no_opacity_on_text_color() {
    let d = diags("comp/x.brain", "<p class=\"a\">x</p><style>.a { color: var(--fg); background: var(--bg); opacity: .5 }</style>", &tokens());
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["B0408"]);
    assert!(diags("comp/x.brain", "<p class=\"a\">x</p><style>.a { opacity: .5 }</style>", &tokens()).is_empty());
}

// `brain fix` turned `padding: 2px` into `var(--focus-w)`: equal by value, not by meaning. B0305 substitutes only a
// token named for spacing; any other match stays a warning without an edit.
#[test]
fn b0305_substitutes_only_spacing_tokens() {
    let toks = |extra: &[(&str, &str)]| {
        let mut t = vec![("--space-1".to_string(), "4px".to_string())];
        t.extend(extra.iter().map(|(n, v)| (n.to_string(), v.to_string())));
        brain_check::Ctx { tokens: Some(t), ..Default::default() }
    };
    let css = "<p class=\"a\">x</p><style>.a { padding: 2px; }</style>";
    let d = diags("comp/x.brain", css, &toks(&[("--focus-w", "2px"), ("--radius-xs", "2px")]));
    let b = d.iter().find(|d| d.code == "B0305").unwrap_or_else(|| panic!("{d:#?}"));
    assert!(b.edits.is_empty() && b.level == Severity::Warning, "{b:#?}");
    for name in ["--gap-xs", "--gutter", "--pad-tight", "--spacing-hair", "--space-0"] {
        let d = diags("comp/x.brain", css, &toks(&[("--focus-w", "2px"), (name, "2px")]));
        let b = d.iter().find(|d| d.code == "B0305").unwrap_or_else(|| panic!("{d:#?}"));
        assert_eq!((b.level, b.edits.iter().map(|e| e.text.clone()).collect::<Vec<_>>()), (Severity::Info, vec![format!("var({name})")]), "{name}");
    }
}

// B0304 wrote the *nearest* color token by RGB distance: `#1b1b1b` silently became `var(--fg)` (`#1a1a1a`), and
// `red` became whatever token was closest. Only an exact value is substituted; the nearest one is a suggestion.
#[test]
fn b0304_substitutes_only_an_exact_color_token() {
    let toks = brain_check::Ctx { tokens: Some(vec![("--fg".into(), "#1a1a1a".into()), ("--accent".into(), "#0071e3".into())]), ..Default::default() };
    let b0304 = |css: &str| {
        let d = diags("comp/x.brain", &format!("<p class=\"a\">x</p><style>.a {{ {css} }}</style>"), &toks);
        d.into_iter().find(|d| d.code == "B0304").unwrap_or_else(|| panic!("no B0304 for {css}"))
    };
    let exact = b0304("color: #1A1A1A;");
    assert_eq!(exact.edits.iter().map(|e| e.text.clone()).collect::<Vec<_>>(), ["var(--fg)"], "{exact:#?}");
    let exact = b0304("color: rgb(0 113 227);");
    assert_eq!(exact.edits.iter().map(|e| e.text.clone()).collect::<Vec<_>>(), ["var(--accent)"], "{exact:#?}");
    for css in ["color: #1b1b1b;", "color: red;", "border-color: #1a1a1a #0070e3;", "color: rgb(26 26 26 / .5);"] {
        let near = b0304(css);
        assert!(near.edits.is_empty(), "{css}: {near:#?}");
        assert!(near.fix.contains("nearest") && near.fix.contains("var(--"), "{css}: {}", near.fix);
    }
}
