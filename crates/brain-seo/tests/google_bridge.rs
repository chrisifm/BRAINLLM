//! The generated static and request-time heads use the same local opt-in module and CSP permissions.
use brain_lex::Json;
use brain_seo::{Config, Page, finish};
const HTML: &str = "<!doctype html><html lang=\"en\"><head><title>Example</title><meta name=\"description\" content=\"A generic example page.\"></head><body><main><h1>Example</h1><button data-track=\"save\">Save</button></main></body></html>";
fn configured() -> Config {
    Config::from_json(&Json::parse(r#"{"site":"https://example.com","google":{"mode":"basic","environments":{"production":{"analytics":"G-PROD"},"development":{"tagManager":"GTM-DEV"}},"events":{"save":"save_item"},"banner":true}}"#).unwrap()).unwrap()
}
#[test]
fn static_and_request_time_templates_use_one_local_module_with_exact_google_csp() {
    let root = std::env::temp_dir();
    let cfg = configured().with_env(&root, "development");
    let mut pages = vec![Page {
        route: "/".into(),
        file: "index.html".into(),
        html: HTML.into(),
        ..Page::default()
    }];
    let built = finish(&cfg, &root, &mut pages, &["/".into()], &[]).unwrap();
    let script = built
        .files
        .iter()
        .find(|(path, _)| path.starts_with("_b/g."))
        .unwrap();
    assert!(script.1.len() < 65536);
    assert!(script.1.contains("GTM-DEV"));
    assert!(!script.1.contains("G-PROD"));
    for html in [
        &pages[0].html,
        &built.server_head.apply(&cfg, "/request", HTML),
    ] {
        assert!(html.contains(&format!("type=\"module\" src=\"/{}\"", script.0)));
        assert!(!html.contains("src=\"https://www.googletagmanager.com"));
    }
    assert_eq!(built.csp, brain_seo::google::CSP);
    assert!(
        built
            .csp
            .contains(&("script-src", "https://www.googletagmanager.com"))
    );
}
#[test]
fn unconfigured_environments_and_default_sites_have_no_bridge_or_google_csp() {
    for cfg in [
        configured().with_env(&std::env::temp_dir(), "preview"),
        Config::default(),
    ] {
        let tags = brain_seo::files::google_tags(&cfg);
        assert!(!tags.head.contains("<script"));
        assert!(tags.file.is_none());
        assert!(tags.csp.is_empty());
    }
}
#[test]
fn legacy_ids_keep_the_original_loader_and_consent_bootstrap() {
    let cfg =
        Config::from_json(&Json::parse(r#"{"google":{"analytics":"G-LEGACY"}}"#).unwrap()).unwrap();
    let tags = brain_seo::files::google_tags(&cfg);
    assert!(tags.head.contains("gtag/js?id=G-LEGACY"));
    assert!(!tags.head.contains("type=\"module\""));
    assert!(tags.file.unwrap().1.contains("analytics_storage:'denied'"));
}
