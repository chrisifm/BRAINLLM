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

/// Applies every B1403 edit to the view and to `i18n/en.json` (identical edits once, as `brain fix` does).
fn extract(root: &Path, rel: &str, src: &str) -> (String, String) {
    let d = check(root, rel, src);
    let mut out = Vec::new();
    for file in [rel, "i18n/en.json"] {
        let cur = std::fs::read_to_string(root.join(file)).unwrap();
        let mut edits: Vec<brain_parse::Edit> = Vec::new();
        for x in d.iter().filter(|x| x.code == "B1403" && x.file == file) {
            for e in &x.edits {
                if !edits.iter().any(|y| y.start == e.start && y.end == e.end && y.text == e.text) {
                    edits.push(e.clone());
                }
            }
        }
        out.push(brain_parse::Diag::apply(&cur, &edits).unwrap_or_else(|| panic!("overlapping edits in {file}: {d:#?}")));
    }
    (out.remove(0), out.remove(0))
}

const META: &str = "<meta seo title={t('nav.home')} description={t('nav.home')}>\n";

#[test]
fn b1403_keys_are_scoped_by_view_path_ascii_and_never_lose_text() {
    let root = app("keys", &["en", "es"], &[("i18n/en.json", EN), ("i18n/es.json", ES)]);
    for d in ["views/posts", "views/account", "views/blog", "comp"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let key = |rel: &str, text: &str| -> Vec<String> {
        let d = check(&root, rel, &format!("{META}<main><p>{text}</p></main>"));
        d.iter().filter(|d| d.code == "B1403" && d.file == rel).map(|d| d.edits[0].text.clone()).collect()
    };
    // Two index views no longer share `index.*`; the home page, a dynamic route, the layout and a component.
    assert_eq!(key("views/posts/index.brain", "Your posts"), ["{t('posts.your_posts')}"]);
    assert_eq!(key("views/account/index.brain", "Your posts"), ["{t('account.your_posts')}"]);
    assert_eq!(key("views/index.brain", "Latest news"), ["{t('home.latest_news')}"]);
    assert_eq!(key("views/blog/[slug].brain", "Share this story"), ["{t('blog-slug.share_this_story')}"]);
    assert_eq!(key("views/_layout.brain", "Sign out"), ["{t('layout.sign_out')}"]);
    assert_eq!(key("comp/card.brain", "Read more"), ["{t('card.read_more')}"]);
    // Keys are ASCII: diacritics fold to their base letter.
    assert_eq!(key("views/about.brain", "Our na\u{ef}ve fa\u{e7}ade"), ["{t('about.our_naive_facade')}"]);
    // Same text twice: one key, one entry. Same first words, another text: a suffixed key, the text is kept.
    let (view, en) = extract(&root, "views/about.brain", &format!("{META}<main><p>Read the full story</p><p>Read the full story</p><p>Read the full story here</p></main>"));
    assert!(view.contains("<p>{t('about.read_the_full_story')}</p><p>{t('about.read_the_full_story')}</p><p>{t('about.read_the_full_story_2')}</p>"), "{view}");
    let parsed = parse_locale(&en, "en", "x");
    assert!(parsed.error.is_none(), "{en}");
    assert_eq!(en.matches("\"about.read_the_full_story\"").count(), 1, "{en}");
    assert_eq!(parsed.entries["about.read_the_full_story_2"].0.as_plain(), Some("Read the full story here"));
    // A key already in the default file with the same text is reused (no new entry); with another text, suffixed.
    let en_has = EN.replace("\"nav.home\": \"Home\",", "\"nav.home\": \"Home\",\n  \"about.contact_us\": \"Contact us\",\n  \"about.get_started\": \"Get started now\",");
    std::fs::write(root.join("i18n/en.json"), &en_has).unwrap();
    let (view, en) = extract(&root, "views/about.brain", &format!("{META}<main><p>Contact us</p><p>Get started</p></main>"));
    assert!(view.contains("{t('about.contact_us')}") && view.contains("{t('about.get_started_2')}"), "{view}");
    assert!(!en.contains("\"about.contact_us\": \"Contact us\",\n  \"about.contact_us\"") && en.matches("about.contact_us").count() == 1, "{en}");
}

#[test]
fn b1403_merges_text_and_interpolations_into_one_message() {
    let root = app("merge", &["en", "es"], &[("i18n/en.json", EN), ("i18n/es.json", ES)]);
    let src = format!("{META}<script>\nlet user = sig({{ email: 'a@b.c' }})\nlet count = sig(2)\n</script>\n<main><p>Signed in as {{user.email}} with {{count}} drafts.</p><p>{{count}}</p></main>");
    let (view, en) = extract(&root, "views/account.brain", &src);
    assert!(view.contains("<p>{t('account.signed_in_as_with', { email: user.email, count })}</p><p>{count}</p>"), "{view}");
    let parsed = parse_locale(&en, "en", "x");
    assert_eq!(parsed.entries["account.signed_in_as_with"].0.as_plain(), Some("Signed in as {email} with {count} drafts."), "{en}");
}

#[test]
fn b1403_covers_attributes_meta_seo_and_literals_in_expressions() {
    let root = app("attrs", &["en", "es"], &[("i18n/en.json", EN), ("i18n/es.json", ES)]);
    let src = "<meta seo title=\"Latest posts\" description=\"Everything we wrote this year\" keyword=\"posts\">\n<script>\nlet copied = sig(false)\nlet open = sig(true)\nlet city = sig({ name: 'Lima' })\nlet name = sig('Ann')\n</script>\n<main class=\"page wide\">\n<input type=\"search\" name=\"q\" placeholder=\"Search posts\" aria-label=\"Search\">\n<img src=\"/a.png\" alt=\"Team photo\">\n<input type=\"submit\" value=\"Send message\"><input type=\"email\" value=\"hello there\">\n<a href=\"/about\" title=\"About {name}\">x</a>\n<button>{copied ? 'Copied' : 'Copy link'}</button>\n<p>{open ? 'Open now' : 'Closed today'}</p>\n<h2>{'Jobs in ' + city.name}</h2>\n<p>{name || 'Nobody yet'}</p>\n</main>";
    let (view, en) = extract(&root, "views/posts.brain", src);
    for want in [
        "title={t('posts.meta_title')}",
        "description={t('posts.meta_description')}",
        "keyword=\"posts\"",
        "class=\"page wide\"",
        "placeholder={t('posts.search_posts')}",
        "aria-label={t('posts.search')}",
        "alt={t('posts.team_photo')}",
        "value={t('posts.send_message')}",
        "type=\"email\" value=\"hello there\"",
        "href=\"/about\"",
        "title={t('posts.about', { name })}",
        "{copied ? t('posts.copied') : t('posts.copy_link')}",
        "{open ? t('posts.open_now') : t('posts.closed_today')}",
        "{t('posts.jobs_in', { name: city.name })}",
        "{name || t('posts.nobody_yet')}",
    ] {
        assert!(view.contains(want), "missing {want} in:\n{view}");
    }
    let parsed = parse_locale(&en, "en", "x");
    assert!(parsed.error.is_none(), "{en}");
    for (k, v) in [("posts.meta_title", "Latest posts"), ("posts.about", "About {name}"), ("posts.jobs_in", "Jobs in {name}"), ("posts.copy_link", "Copy link")] {
        assert_eq!(parsed.entries.get(k).and_then(|e| e.0.as_plain()), Some(v), "{k} in {en}");
    }
    // After the fix, nothing is left to extract.
    std::fs::write(root.join("i18n/en.json"), &en).unwrap();
    let d = check(&root, "views/posts.brain", &view);
    assert!(!d.iter().any(|d| d.code == "B1403"), "{d:#?}");
}

#[test]
fn framework_english_covers_every_runtime_message_key() {
    // Every `msg('form.x', …)` of the runtime and every `say('x', …)` of <ErrorState> has its English text in
    // FRAMEWORK_EN, so `brain add locale en` on a site in another language translates them all.
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let mut keys = std::collections::BTreeSet::new();
    for (file, call, prefix) in [("brain-schema.js", "msg('", ""), ("brain-form.js", "msg('", ""), ("ui/error-state.brain", "say('", "error_state.")] {
        let src = std::fs::read_to_string(rt.join(file)).unwrap();
        for part in src.split(call).skip(1) {
            keys.insert(format!("{prefix}{}", part.split('\'').next().unwrap()));
        }
    }
    keys.retain(|k| k != "error_state.' + key + '" && !k.contains(' '));
    let have: Vec<&str> = brain_check::i18n::FRAMEWORK_EN.iter().map(|(k, _)| *k).collect();
    let missing: Vec<&String> = keys.iter().filter(|k| !have.contains(&k.as_str())).collect();
    assert!(keys.len() >= 25 && missing.is_empty(), "missing English for {missing:?}");
}
