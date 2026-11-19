//! User-authored text and malformed selectors must never panic the checker.
use brain_check::{Ctx, parse_hex, split};
use brain_parse::{Diag, ast::Component};
use std::path::Path;

fn parse(src: &str) -> Component {
    brain_parse::parse(src, Path::new("comp/sample.brain")).unwrap_or_else(|d| panic!("{d:?}"))
}

#[test]
fn structure_accepts_multibyte_text_and_whitespace_after_blocks() {
    for text in ["\u{b7}", "\u{20ac}", "\u{1f680}", "\u{a0}", "\u{2003}"] {
        let src = format!("<script>let n = 1</script>{text}<p>{{n}}</p>");
        let diags = brain_check::check(&parse(&src), &Ctx::default());
        assert!(diags.is_empty(), "{text:?}: {diags:?}");
    }
    let src = "<script>let a = 1\nlet b = 2\nlet c = 3</script><p>{a < b ? ' \u{b7} x' : ''} \u{b7} {c}</p>";
    assert!(brain_check::check(&parse(src), &Ctx::default()).is_empty());
}

#[test]
fn reordered_blocks_preserve_multibyte_markup_and_valid_edit_boundaries() {
    let src = "\u{1f680}<p>{n}</p><script>let n = 1</script>";
    let diags = brain_check::check(&parse(src), &Ctx::default());
    let order = diags.iter().find(|d| d.code == "B0125").expect("block-order edit");
    let fixed = Diag::apply(src, &order.edits).expect("UTF-8-safe edit");
    assert!(fixed.contains("\u{1f680}<p>{n}</p>"), "{fixed}");
    assert!(brain_check::check(&parse(&fixed), &Ctx::default()).is_empty());
}

#[test]
fn invalid_multibyte_hex_colors_return_none() {
    for value in ["#\u{b7}a", "#a\u{b7}", "#\u{20ac}", "#a\u{20ac}", "#a\u{1f680}b", "#\u{1f680}abcd"] {
        assert_eq!(parse_hex(value), None, "{value}");
    }
    assert_eq!(parse_hex("#fff"), Some((1.0, 1.0, 1.0)));
    assert_eq!(parse_hex("#000000"), Some((0.0, 0.0, 0.0)));
}

#[test]
fn malformed_attribute_selectors_return_an_error_without_panicking() {
    let component = parse("<p role=\"note\">Sample</p>");
    for selector in ["[", "[role", "[role=\u{b7}", "[role]\u{1f680}"] {
        let error = split::split(&component, selector, "Note", "comp/note.brain").unwrap_err();
        assert!(error.contains("no element"), "{selector}: {error}");
    }
}

#[test]
fn valid_multibyte_class_and_attribute_selectors_still_extract() {
    let component = parse("<p class=\"\u{1f680}\" title=\"\u{20ac}\">Sample</p>");
    for selector in [".\u{1f680}", "[title=\"\u{20ac}\"]"] {
        let result = split::split(&component, selector, "Note", "comp/note.brain").unwrap();
        assert!(result.extracted.contains("Sample"));
        parse(&result.original);
        parse(&result.extracted);
    }
}
