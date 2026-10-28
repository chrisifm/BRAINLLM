//! #149 (INTERNALS §37): languages — locale files, `t()` keys, placeholders, fixed text, RTL and translated slugs.

use brain_check::i18n::{check_locales, load, parse_locale, valid_locale};
use std::path::{Path, PathBuf};

fn app(name: &str, locales: &[&str], files: &[(&str, &str)]) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("i18n-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("i18n")).unwrap();
    std::fs::create_dir_all(root.join("views")).unwrap();
    let list: Vec<String> = locales.iter().map(|l| format!("\"{l}\"")).collect();
    std::fs::write(root.join("brain.json"), format!("{{ \"i18n\": {{ \"default\": \"{}\", \"locales\": [{}] }} }}", locales[0], list.join(", "))).unwrap();
    std::fs::write(root.join("app.brain"), "<html lang=\"en\"><head></head><body><slot /></body></html>").unwrap();
    for (p, s) in files {
        std::fs::write(root.join(p), s).unwrap();
    }
    root
}

fn check(root: &Path, rel: &str, src: &str) -> Vec<brain_parse::Diag> {
    std::fs::write(root.join(rel), src).unwrap();
    let c = brain_parse::parse(src, Path::new(rel)).unwrap_or_else(|d| panic!("{d:?}"));
    let (ctx, _) = brain_check::load_ctx(&root.join(rel), &c);
    brain_check::check(&c, &ctx)
}

const EN: &str = "{\n  \"nav.home\": \"Home\",\n  \"hello\": \"Hi {name}\",\n  \"posts.count\": { \"one\": \"{n} post\", \"other\": \"{n} posts\" }\n}\n";
// Repository rule: no Spanish in files, so the second locale holds tagged English stand-ins.
const ES: &str = "{\n  \"nav.home\": \"Home [es]\",\n  \"hello\": \"Hi [es] {username}\",\n  \"posts.count\": { \"one\": \"{n} post [es]\", \"other\": \"{n} posts [es]\" }\n}\n";

#[test]
fn locale_files_parse_with_lines() {
    let f = parse_locale(EN, "en", "i18n/en.json");
    assert!(f.error.is_none());
    assert_eq!(f.entries["hello"].1, 3);
    assert_eq!(f.entries["posts.count"].0.vars().into_iter().collect::<Vec<_>>(), ["n"]);
    assert!(parse_locale("{ \"a\": \"x\"", "en", "x").error.is_some());
    assert!(valid_locale("es") && valid_locale("pt-BR") && valid_locale("zh-Hant") && valid_locale("es-419"));
    assert!(!valid_locale("english") && !valid_locale("xx") && !valid_locale("es-br"));
}

#[test]
fn b1402_b1404_b1405_across_locale_files() {
    let pt = "{\n  \"nav.home\": \"Home\",\n  \"hello\": \"Hi {name}\",\n  \"posts.count\": { \"one\": \"{n} post\", \"other\": \"{n} posts\" }\n}\n";
    let root = app("files", &["en", "es", "pt", "klingon"], &[("i18n/en.json", EN), ("i18n/es.json", ES), ("i18n/pt.json", pt)]);
    let i = load(&root).unwrap();
    let d = check_locales(&i);
    let got: Vec<(&str, &str, String)> = d.iter().map(|d| (d.code, d.file.as_str(), format!("{:?}", d.level))).collect();
    assert!(got.contains(&("B1405", "brain.json", "Error".into())), "{got:?}");
    assert!(got.contains(&("B1404", "i18n/es.json", "Error".into())), "{got:?}"); // {username} vs {name}
    assert!(got.contains(&("B1402", "i18n/pt.json", "Info".into())), "{got:?}"); // one word: Home
    assert!(got.iter().filter(|g| g.0 == "B1402" && g.2 == "Warning").count() >= 2, "{got:?}"); // Hi {name}, plural
}

#[test]
fn b1401_missing_key_and_b1404_call_vars() {
    let root = app("calls", &["en", "es"], &[("i18n/en.json", EN), ("i18n/es.json", ES)]);
    let view = "<meta seo title=\"T\" description=\"D\">\n<script>\nlet n = sig(2)\n</script>\n<main><h1>{t('nav.home')}</h1><p>{t('nav.about')}</p><p>{t('posts.count', { n })}</p><p>{t('hello')}</p></main>";
    let d = check(&root, "views/home.brain", view);
    let b1401: Vec<&brain_parse::Diag> = d.iter().filter(|d| d.code == "B1401").collect();
    assert_eq!(b1401.iter().map(|d| d.file.as_str()).collect::<Vec<_>>(), ["i18n/en.json", "i18n/es.json"], "{d:#?}");
    let fixed = brain_parse::Diag::apply(EN, &b1401[0].edits).unwrap();
    assert!(parse_locale(&fixed, "en", "x").entries.contains_key("nav.about"), "{fixed}");
    let b1404: Vec<&brain_parse::Diag> = d.iter().filter(|d| d.code == "B1404").collect();
    assert_eq!(b1404.len(), 1, "t('hello') without {{ name }}: {d:#?}");
}

#[test]
fn b1403_fixed_text_only_with_two_locales() {
    let view = "<meta seo title=\"T\" description=\"D\">\n<main><h1>{t('nav.home')}</h1><p>Welcome back</p><code>npm i</code></main>";
    let one = app("one", &["en"], &[("i18n/en.json", EN)]);
    assert!(!check(&one, "views/home.brain", view).iter().any(|d| d.code == "B1403"));
    let two = app("two", &["en", "es"], &[("i18n/en.json", EN), ("i18n/es.json", ES)]);
    let d = check(&two, "views/home.brain", view);
    let b: Vec<&brain_parse::Diag> = d.iter().filter(|d| d.code == "B1403").collect();
    assert_eq!(b.iter().map(|d| d.file.as_str()).collect::<Vec<_>>(), ["views/home.brain", "i18n/en.json", "i18n/es.json"], "{d:#?}");
    assert_eq!(b[0].edits[0].text, "{t('home.welcome_back')}");
    assert!(b[1].edits[0].text.contains("\"home.welcome_back\": \"Welcome back\""));
}

#[test]
fn b0306_physical_properties_with_rtl_and_route_slugs() {
    let css = "<meta seo title=\"T\" description=\"D\">\n<main class=\"m\"><h1>{t('nav.home')}</h1></main>\n<style>.m { margin-left: var(--space-2); text-align: right; }</style>";
    let ltr = app("ltr", &["en", "es"], &[("i18n/en.json", EN), ("i18n/es.json", ES)]);
    assert!(!check(&ltr, "views/home.brain", css).iter().any(|d| d.code == "B0306"));
    let rtl = app("rtl", &["en", "ar"], &[("i18n/en.json", EN), ("i18n/ar.json", EN)]);
    let d = check(&rtl, "views/home.brain", css);
    let b: Vec<String> = d.iter().filter(|d| d.code == "B0306").map(|d| d.edits[0].text.clone()).collect();
    assert_eq!(b, ["margin-inline-start", "end"], "{d:#?}");
    let route = "<meta seo title=\"T\" description=\"D\">\n<meta route es=\"/es/about-us\" fr=\"/fr/about-us\">\n<main><h1>{t('nav.home')}</h1></main>";
    let d = check(&ltr, "views/about.brain", route);
    assert_eq!(d.iter().filter(|d| d.code == "B1406").count(), 1, "fr is not a site locale: {d:#?}");
}

#[test]
fn declared_locale_without_file_is_b1401_with_a_creating_edit() {
    // QA #153: a locale in brain.json with no i18n/<locale>.json was silently skipped.
    let root = app("missing-file", &["en", "es", "ar"], &[("i18n/en.json", EN), ("i18n/es.json", ES)]);
    let i = load(&root).unwrap();
    assert_eq!(i.missing, ["ar"]);
    let d = check_locales(&i);
    let b = d.iter().find(|d| d.code == "B1401" && d.file == "i18n/ar.json").expect("B1401 for ar");
    let created = &b.edits[0].text;
    let parsed = parse_locale(created, "ar", "i18n/ar.json");
    assert!(parsed.error.is_none() && parsed.entries.len() == 3, "{created}");
}
