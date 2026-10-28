//! #62 (INTERNALS §21): deterministic normalizations compile on their own and come back as `info`.

use brain_parse::Severity;
use std::path::Path;

fn infos(o: &brain_codegen::Output) -> Vec<&'static str> {
    o.warnings.iter().filter(|d| d.level == Severity::Info).map(|d| d.code).collect()
}

#[test]
fn normalizations_compile_and_report_info() {
    let src = r#"<script>
import { sig } from 'brain'
import { Button } from 'brain/ui'
var n = sig(0)
let items = sig([])
let active = sig(false)
const inc = () => { n++; console.warn('n', n) }
</script>
<div class={active ? 'on' : ''}>
  {#each items as it}<p class="row {it.done ? 'done' : 'todo'}">{it.name}</p>{/each}
  <Button on:click={inc}>{n}</Button>
</div>"#;
    let o = brain_codegen::compile(src, Path::new("comp/norm.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    let mut got = infos(&o);
    got.sort();
    assert_eq!(got, ["B0103", "B0121", "B0121", "B0202", "B0212", "B0212", "B1308"], "{:#?}", o.warnings);
    // Positions are those of the file as written.
    let var = o.warnings.iter().find(|d| d.code == "B0103").unwrap();
    assert_eq!((var.line, var.col), (4, 1));
    // The output is the normalized program.
    assert!(!o.js.contains("console.warn") && o.js.contains("console.log(\x27n\x27, n.v)"), "{}", o.js);
    assert!(o.js.contains("from \"/@brain/ui/button.brain\""), "{}", o.js);
    assert!(!o.js.contains("from \"brain\""), "{}", o.js);
    assert!(o.js.contains("cls("), "{}", o.js);
}

#[test]
fn token_synonyms_normalize_typos_do_not() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("norm-tokens");
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::write(root.join("tokens.cssbrain"), ":root { --space-2: 8px; --line: #ddd; --fg: #111; --bg: #fff; }\n").unwrap();
    let opts = brain_codegen::Options { root: Some(root), ..Default::default() };
    let ok = "<p>x</p><style>p { padding: var(--spacing-2) var(--space2) var(--gap-2); border: 1px solid var(--border) }</style>";
    let o = brain_codegen::compile_with(ok, Path::new("comp/t.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.css.contains("var(--space-2) var(--space-2) var(--space-2)") && o.css.contains("var(--line)"), "{}", o.css);
    assert!(infos(&o).iter().all(|c| *c == "B0302"));
    // A typo is still an error (with its edit).
    let e = brain_codegen::compile_with("<p>x</p><style>p { padding: var(--spce-2) }</style>", Path::new("comp/t.brain"), &opts).unwrap_err();
    assert_eq!(e[0].code, "B0302");
    assert_eq!(e[0].level, Severity::Error);
    assert!(!e[0].edits.is_empty());
}

#[test]
fn judgment_stays_an_error_and_nothing_is_rewritten() {
    // An error elsewhere: no normalization, original positions, infos still listed.
    let e = brain_codegen::compile("<script>var n = 1</script><img src=\"/a.png\"><p>{n}</p>", Path::new("comp/j.brain")).unwrap_err();
    let codes: Vec<(&str, Severity)> = e.iter().map(|d| (d.code, d.level)).collect();
    assert!(codes.contains(&("B0103", Severity::Info)) && codes.contains(&("B0401", Severity::Error)), "{codes:?}");
}

#[test]
fn double_h1_and_space_zero_are_normalized() {
    // #133 (QA #64): the last two failure classes of the LLM test become normalizations.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("norm133-{}", std::process::id()));
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::write(root.join("tokens.cssbrain"), ":root { --space-1: 4px; --space-2: 8px; --fg: #1a1a1a; --bg: #f5f5f7; }\n").unwrap();
    let src = "<meta seo title=\"T\" description=\"D\">\n<main class=\"m\"><h1>Title</h1><h1>Second</h1><h1 class=\"x\">Third</h1></main>\n<style>.m { padding: var(--space-0) var(--space-2); margin: var( --space-0 ); }</style>";
    let opts = brain_codegen::Options { root: Some(root.clone()), ..Default::default() };
    let o = brain_codegen::compile_with(src, std::path::Path::new("views/x.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("<h1>Title</h1><h2>Second</h2><h2 class=\\\"x\\\">Third</h2>"), "{}", o.js);
    assert!(o.css.contains("padding: 0 var(--space-2)") && o.css.contains("margin: 0;"), "{}", o.css);
    let infos: Vec<&str> = o.warnings.iter().filter(|d| d.is_info()).map(|d| d.code).collect();
    assert_eq!(infos.iter().filter(|c| **c == "B0405").count(), 2, "{infos:?}");
    assert_eq!(infos.iter().filter(|c| **c == "B0302").count(), 2, "{infos:?}");
}

#[test]
fn gate_regressions_163_are_normalized() {
    // #163: block order and undeclared emits are mechanical → the compiler fixes them; a translucent veil → --scrim.
    let p = std::path::Path::new("comp/x.brain");
    // Meta after script, style before markup: reordered and compiled.
    let src = "<script>\nlet n = sig(0)\n</script>\n<meta props=\"label:str='Hi'\">\n<style>p { color: var(--fg); }</style>\n<p>{label} {n}</p>\n";
    let o = brain_codegen::compile(src, p).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.warnings.iter().any(|d| d.code == "B0125" && d.is_info()), "{:?}", o.warnings);
    assert!(o.js.contains("$p.label") && o.css.contains("color: var(--fg)"), "{}", o.js);
    // emit without <meta emits>: declared automatically (also when a declaration exists).
    for src in [
        "<script>\nconst pick = () => emit('select')\n</script>\n<button on:click={pick}>Pick</button>",
        "<meta emits=\"close\">\n<script>\nconst pick = () => emit('select')\nconst shut = () => emit('close')\n</script>\n<button on:click={pick}>Pick</button><button on:click={shut}>Close</button>",
    ] {
        let o = brain_codegen::compile(src, p).unwrap_or_else(|d| panic!("{d:#?}"));
        assert!(o.warnings.iter().any(|d| d.code == "B0802" && d.is_info()), "{:?}", o.warnings);
    }
    // Translucent background: the edit is the scrim token.
    let src = "<div class=\"veil\">x</div>\n<style>.veil { background: rgb(0 0 0 / .5); }</style>";
    let d = brain_codegen::compile(src, p).expect_err("B0304 stays an error");
    let b = d.iter().find(|d| d.code == "B0304").expect("B0304");
    assert_eq!(b.edits[0].text, "var(--scrim)");
    let fixed = brain_parse::Diag::apply(src, &b.edits).unwrap();
    assert!(fixed.contains("background: var(--scrim)"), "{fixed}");
}

#[test]
fn static_t_resolves_at_build_and_runtime_t_is_imported() {
    // #149 (INTERNALS §37): the page carries its text. #184: a `t()` left for the runtime is B0128 until the runtime
    // ships brain-i18n.js (never an import the build cannot resolve).
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("t-static-{}", std::process::id()));
    std::fs::create_dir_all(root.join("i18n")).unwrap();
    std::fs::write(root.join("brain.json"), "{ \"i18n\": { \"default\": \"en\", \"locales\": [\"en\", \"es\"] } }").unwrap();
    std::fs::write(root.join("i18n/en.json"), "{ \"nav.home\": \"Home\", \"hello\": \"Hi {name}\", \"posts.count\": { \"one\": \"{n} post\", \"other\": \"{n} posts\" } }").unwrap();
    std::fs::write(root.join("i18n/es.json"), "{ \"nav.home\": \"Home [es]\", \"hello\": \"Hi [es] {name}\", \"posts.count\": { \"one\": \"{n} post [es]\", \"other\": \"{n} posts [es]\" } }").unwrap();
    let texts: std::collections::BTreeMap<String, String> = [("nav.home", "Home [es]"), ("hello", "Hi [es] {name}")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let opts = brain_codegen::Options { root: Some(root.clone()), i18n: Some(("es".into(), texts)), ..Default::default() };
    let ok = "<script>\nlet name = sig('Ana')\n</script>\n<h2>{t('nav.home')}</h2><p>{t('hello', { name })}</p>";
    let o = brain_codegen::compile_with(ok, std::path::Path::new("comp/t.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("insert(null, \"Home [es]\""), "{}", o.js);
    assert!(o.js.contains("insert(null, `Hi [es] ${name}`"), "{}", o.js);
    assert!(o.i18n_keys.is_empty() && !o.js.contains("brain-i18n"), "{}", o.js);
    // Plurals and dynamic keys stay for the runtime (brain-i18n.js, RUNTIME #150) and are listed for the chunk.
    let plural = "<script>\nlet n = sig(2)\n</script>\n<p>{t('posts.count', { n })}</p><button on:click={n++}>+</button>";
    let o = brain_codegen::compile_with(plural, std::path::Path::new("comp/p.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("import { t } from \"/@brain/brain-i18n.js\";") && o.js.contains("() => t('posts.count', { n: n.v })"), "{}", o.js);
    assert_eq!(o.i18n_keys, ["posts.count"]);
}
