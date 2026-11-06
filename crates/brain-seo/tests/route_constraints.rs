use brain_lex::Json;
use brain_seo::{Config, Page, Site, redirects};
#[test]
fn seo_dynamic_links_and_redirects_respect_raw_constraints() {
    let cfg=Config::from_json(&Json::parse(r#"{"base":"/app/","routeParams":{"/:code":{"code":"[2-9a-z]{7}"}},"i18n":{"default":"en","locales":["en","fr"]}}"#).unwrap()).unwrap();
    let pages=vec![Page{route:"/".into(),file:"index.html".into(),html:r#"<html lang="en"><head><title>Route constraints</title></head><body><main><h1>Routes</h1><a href="/app/3abcdef">valid</a><a href="/app/zzz">short</a><a href="/app/%33abcdef">encoded</a><a href="/app/old/3abcdef">redirect</a><a href="/app/old/%33abcdef">encoded redirect</a></main></body></html>"#.into(),..Default::default()}];
    let rules = redirects::parse(
        r#"[{"from":"/old/:code","params":{"code":"[2-9a-z]{7}"},"to":"/:code"}]"#,
    )
    .unwrap();
    let report = brain_seo::audit(&Site {
        cfg: &cfg,
        pages: &pages,
        routes: &["/:code".into()],
        assets: &[],
        redirects: &rules,
        images: &[],
        i18n: None,
    });
    let broken = report.pages[0]
        .findings
        .iter()
        .filter(|f| f.code == "B1819")
        .collect::<Vec<_>>();
    assert_eq!(broken.len(), 3, "{broken:?}");
    assert!(broken.iter().any(|f| f.msg.contains("/app/zzz")));
    assert!(broken.iter().any(|f| f.msg.contains("/app/%33abcdef")));
    assert!(broken.iter().any(|f| f.msg.contains("/app/old/%33abcdef")));
    assert!(
        report.pages[0]
            .findings
            .iter()
            .any(|f| f.code == "B1825" && f.msg.contains("/app/old/3abcdef"))
    );
    assert!(cfg.route_allows("/fr/:code", "/fr/3abcdef"));
    assert!(!cfg.route_allows("/fr/:code", "/fr/%33abcdef"));
    let list = vec!["/zzz".into(), "/%33abcdef".into(), "/3abcdef".into()];
    assert_eq!(
        brain_seo::register_gone_with_constraints(
            &cfg,
            &list,
            &|p| brain_seo::audit::matches_pattern("/:code", p) && cfg.route_allows("/:code", p),
            &[]
        ),
        vec!["/zzz", "/%33abcdef"]
    );
    let previous = "<urlset><url><loc>https://example.com/old/%33abcdef</loc></url></urlset>";
    assert_eq!(
        redirects::removed(previous, "https://example.com", &[], &rules).len(),
        1
    );
}
#[test]
fn opt_in_diagnostic_leaves_legacy_config_severity_unchanged() {
    let root = std::env::temp_dir().join(format!("brain-redirect-opt-in-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for text in [
        r#"[{"from":"/old","status":200,"to":"/new"}]"#,
        "broken JSON",
    ] {
        std::fs::write(root.join("redirects.json"), text).unwrap();
        assert!(redirects::check_constraints(&root).is_ok());
    }
    std::fs::write(
        root.join("redirects.json"),
        r#"[{"from":"/:id","params":{"id":"a+"},"to":"/new"}]"#,
    )
    .unwrap();
    assert!(redirects::check_constraints(&root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
