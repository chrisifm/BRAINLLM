//! Structured data (INTERNALS §33): automatic JSON-LD per page (`BreadcrumbList` from the route tree, `WebSite` +
//! `SearchAction` and `Organization` on the home page, `FAQPage` from `<details>` groups marked `faq`, `Article` for
//! content collections) and validation of every block against the properties search engines need per type.

use crate::audit::Level;
use crate::config::Config;
use crate::html::Doc;
use brain_lex::Json;
use std::collections::BTreeMap;

/// Content-collection facts for an `Article` (filled by the collections step).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Article {
    pub headline: String,
    pub description: Option<String>,
    /// `YYYY-MM-DD` (or full ISO 8601).
    pub published: Option<String>,
    pub modified: Option<String>,
    pub image: Option<String>,
    pub author: Option<String>,
    pub tags: Vec<String>,
}

const CONTEXT: &str = "https://schema.org";

fn s(v: &str) -> Json {
    Json::str(v)
}

/// `@type` values declared anywhere in a page's JSON-LD (top level, arrays and `@graph`).
pub fn declared_types(doc: &Doc) -> Vec<String> {
    let mut out = Vec::new();
    for block in &doc.jsonld {
        if let Ok(j) = Json::parse(block.trim()) {
            for n in nodes(&j) {
                out.extend(types(n));
            }
        }
    }
    out
}

fn nodes(j: &Json) -> Vec<&Json> {
    match j {
        Json::Arr(a) => a.iter().flat_map(nodes).collect(),
        Json::Obj(_) => match j.get("@graph") {
            Some(g) => nodes(g),
            None => vec![j],
        },
        _ => vec![],
    }
}

fn types(n: &Json) -> Vec<String> {
    match n.get("@type") {
        Some(Json::Str(t)) => vec![t.clone()],
        Some(Json::Arr(a)) => a
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_string)
            .collect(),
        _ => vec![],
    }
}

/// Breadcrumb trail of `route`: home, every ancestor that is a page, the page itself. `names` maps route → name.
pub fn breadcrumb(cfg: &Config, route: &str, names: &BTreeMap<String, String>) -> Option<Json> {
    if route == "/" || cfg.site.is_none() {
        return None;
    }
    let mut trail = vec![(
        "/".to_string(),
        names.get("/").cloned().unwrap_or_else(|| "Home".into()),
    )];
    let mut prefix = String::new();
    for seg in route.split('/').filter(|x| !x.is_empty()) {
        prefix.push('/');
        prefix.push_str(seg);
        if let Some(n) = names.get(&prefix) {
            trail.push((prefix.clone(), n.clone()));
        }
    }
    if trail.len() < 2 || trail.last().is_none_or(|(r, _)| r != route) {
        return None;
    }
    let items = trail
        .iter()
        .enumerate()
        .map(|(i, (r, n))| {
            Json::obj([
                ("@type", s("ListItem")),
                ("position", Json::Num((i + 1) as f64)),
                ("name", s(n)),
                ("item", s(&cfg.abs(r))),
            ])
        })
        .collect();
    Some(Json::obj([
        ("@context", s(CONTEXT)),
        ("@type", s("BreadcrumbList")),
        ("itemListElement", Json::Arr(items)),
    ]))
}

pub fn website(cfg: &Config, name: &str) -> Option<Json> {
    let url = cfg.abs("/");
    if !url.contains("://") {
        return None;
    }
    let mut kv = vec![
        ("@context".to_string(), s(CONTEXT)),
        ("@type".into(), s("WebSite")),
        ("name".into(), s(name)),
        ("url".into(), s(&url)),
    ];
    if let Some(q) = &cfg.search {
        let target = Json::obj([
            ("@type", s("EntryPoint")),
            (
                "urlTemplate",
                s(&cfg.abs(&q.replace("{query}", "{search_term_string}"))),
            ),
        ]);
        kv.push((
            "potentialAction".into(),
            Json::obj([
                ("@type", s("SearchAction")),
                ("target", target),
                ("query-input", s("required name=search_term_string")),
            ]),
        ));
    }
    Some(Json::Obj(kv))
}

/// `Organization` from brain.json `organization`, or from the site name when only `social` profiles are set (the
/// `sameAs` links tie the site to its Facebook, Instagram, LinkedIn… accounts for search engines and LLMs).
pub fn organization(cfg: &Config, site_name: &str) -> Option<Json> {
    let fallback = crate::config::Organization {
        name: site_name.to_string(),
        logo: None,
        same_as: vec![],
    };
    let o = match &cfg.organization {
        Some(o) => o,
        None if !cfg.social.profiles.is_empty() => &fallback,
        None => return None,
    };
    let mut same: Vec<String> = o.same_as.clone();
    for (_, u) in &cfg.social.profiles {
        if !same.contains(u) {
            same.push(u.clone());
        }
    }
    let mut kv = vec![
        ("@context".to_string(), s(CONTEXT)),
        ("@type".into(), s("Organization")),
        ("name".into(), s(&o.name)),
    ];
    if cfg.site.is_some() {
        kv.push(("url".into(), s(&cfg.abs("/"))));
    }
    if let Some(l) = &o.logo {
        kv.push(("logo".into(), s(&cfg.abs(l))));
    }
    if !same.is_empty() {
        kv.push((
            "sameAs".into(),
            Json::Arr(same.iter().map(|x| s(x)).collect()),
        ));
    }
    Some(Json::Obj(kv))
}

pub fn faq_page(doc: &Doc) -> Option<Json> {
    if doc.faq.is_empty() {
        return None;
    }
    let qs = doc
        .faq
        .iter()
        .map(|(q, a)| {
            Json::obj([
                ("@type", s("Question")),
                ("name", s(q)),
                (
                    "acceptedAnswer",
                    Json::obj([("@type", s("Answer")), ("text", s(a))]),
                ),
            ])
        })
        .collect();
    Some(Json::obj([
        ("@context", s(CONTEXT)),
        ("@type", s("FAQPage")),
        ("mainEntity", Json::Arr(qs)),
    ]))
}

pub fn article(cfg: &Config, route: &str, a: &Article) -> Json {
    let mut kv = vec![
        ("@context".to_string(), s(CONTEXT)),
        ("@type".into(), s("BlogPosting")),
        (
            "headline".into(),
            s(&a.headline.chars().take(110).collect::<String>()),
        ),
    ];
    if let Some(d) = &a.description {
        kv.push(("description".into(), s(d)));
    }
    if let Some(i) = &a.image {
        kv.push(("image".into(), s(&cfg.abs(i))));
    }
    if let Some(d) = &a.published {
        kv.push(("datePublished".into(), s(d)));
    }
    if let Some(d) = a.modified.as_ref().or(a.published.as_ref()) {
        kv.push(("dateModified".into(), s(d)));
    }
    if let Some(au) = &a.author {
        let mut p = vec![("@type".to_string(), s("Person")), ("name".into(), s(au))];
        if let Some(prof) = cfg.authors.iter().find(|x| x.name.eq_ignore_ascii_case(au)) {
            if let Some(u) = &prof.url {
                p.push(("url".into(), s(u)));
            }
            if !prof.same_as.is_empty() {
                p.push((
                    "sameAs".into(),
                    Json::Arr(prof.same_as.iter().map(|x| s(x)).collect()),
                ));
            }
        }
        kv.push(("author".into(), Json::Obj(p)));
    }
    if !a.tags.is_empty() {
        kv.push(("keywords".into(), s(&a.tags.join(", "))));
    }
    kv.push(("mainEntityOfPage".into(), s(&cfg.abs(route))));
    Json::Obj(kv)
}

/// Automatic blocks for one page, skipping any type the author already declared.
pub fn auto(
    cfg: &Config,
    route: &str,
    doc: &Doc,
    names: &BTreeMap<String, String>,
    site_name: &str,
    art: Option<&Article>,
    videos: &[crate::video::Video],
) -> Vec<Json> {
    let have = declared_types(doc);
    let mut out = Vec::new();
    let mut add = |t: &str, j: Option<Json>| {
        if let Some(j) = j
            && !have.iter().any(|h| h == t)
        {
            out.push(j);
        }
    };
    if route == "/" {
        add("WebSite", website(cfg, site_name));
        add("Organization", organization(cfg, site_name));
    }
    add("BreadcrumbList", breadcrumb(cfg, route, names));
    add("FAQPage", faq_page(doc));
    if let Some(a) = art
        && !have
            .iter()
            .any(|h| matches!(h.as_str(), "Article" | "BlogPosting" | "NewsArticle"))
    {
        out.push(article(cfg, route, a));
    }
    if !have.iter().any(|h| h == "VideoObject") {
        out.extend(videos.iter().map(crate::video::video_object));
    }
    out
}

/// Every node (top level, arrays, `@graph`) of a page's JSON-LD.
pub fn typed_nodes(doc: &Doc) -> Vec<Json> {
    doc.jsonld
        .iter()
        .filter_map(|b| Json::parse(b.trim()).ok())
        .flat_map(|j| nodes(&j).into_iter().cloned().collect::<Vec<_>>())
        .collect()
}

/// `<script type="application/ld+json">` with `</` escaped so the content cannot close the element.
pub fn script(j: &Json) -> String {
    format!(
        "<script type=\"application/ld+json\">{}</script>",
        j.stringify().replace("</", "<\\/")
    )
}

/// Properties a type needs (`a|b` = either). Rich-result requirements; missing ones are warnings.
const REQUIRED: &[(&str, &[&str])] = &[
    (
        "AggregateRating",
        &["ratingValue", "ratingCount|reviewCount"],
    ),
    ("Answer", &["text"]),
    ("Article", &["headline"]),
    ("BlogPosting", &["headline"]),
    ("BreadcrumbList", &["itemListElement"]),
    ("Course", &["name", "description"]),
    ("Dataset", &["name", "description"]),
    ("Event", &["name", "startDate", "location"]),
    ("FAQPage", &["mainEntity"]),
    (
        "JobPosting",
        &[
            "title",
            "description",
            "datePosted",
            "hiringOrganization",
            "jobLocation|applicantLocationRequirements",
        ],
    ),
    ("ListItem", &["position", "name"]),
    ("LocalBusiness", &["name", "address"]),
    ("NewsArticle", &["headline"]),
    ("Organization", &["name"]),
    ("Person", &["name"]),
    ("Product", &["name", "offers|review|aggregateRating"]),
    ("Question", &["name", "acceptedAnswer"]),
    ("Recipe", &["name", "image"]),
    ("VideoObject", &["name", "thumbnailUrl", "uploadDate"]),
    ("WebSite", &["name", "url"]),
];

/// Recommended (info only, no score penalty).
const RECOMMENDED: &[(&str, &[&str])] = &[
    ("Article", &["image", "datePublished", "author"]),
    ("BlogPosting", &["image", "datePublished", "author"]),
    ("NewsArticle", &["image", "datePublished", "author"]),
    ("Organization", &["url", "logo"]),
    ("SoftwareApplication", &["offers", "aggregateRating|review"]),
    ("WebApplication", &["offers", "aggregateRating|review"]),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub level: Level,
    pub msg: String,
    pub fix: String,
}

fn has(n: &Json, alts: &str) -> bool {
    alts.split('|').any(|k| match n.get(k) {
        None | Some(Json::Null) => false,
        Some(Json::Str(v)) => !v.trim().is_empty(),
        Some(Json::Arr(a)) => !a.is_empty(),
        Some(_) => true,
    })
}

fn check_node(n: &Json, top: bool, out: &mut Vec<Issue>) {
    let ts = types(n);
    if ts.is_empty() && top {
        out.push(Issue {
            level: Level::Error,
            msg: "JSON-LD node without @type".into(),
            fix: "add \"@type\" (e.g. \"Article\"), or use `schema=\"Article\"` in <meta seo>"
                .into(),
        });
    }
    for t in &ts {
        for (table, level) in [(REQUIRED, Level::Warning), (RECOMMENDED, Level::Info)] {
            if let Some((_, props)) = table.iter().find(|(k, _)| k == t) {
                for p in props.iter().filter(|p| !has(n, p)) {
                    let shown = p.replace('|', "` or `");
                    out.push(Issue {
                        level,
                        msg: format!("{t} has no `{shown}`"),
                        fix: format!("add `{shown}` to the {t} JSON-LD"),
                    });
                }
            }
        }
    }
    // nested typed objects (ListItem, Question, Answer, Offer…)
    if let Json::Obj(kv) = n {
        for (k, v) in kv {
            if k == "@context" {
                continue;
            }
            let children: Vec<&Json> = match v {
                Json::Arr(a) => a.iter().collect(),
                Json::Obj(_) => vec![v],
                _ => vec![],
            };
            for c in children.into_iter().filter(|c| c.get("@type").is_some()) {
                check_node(c, false, out);
            }
        }
    }
}

/// Validates one `<script type="application/ld+json">` body.
pub fn validate(block: &str) -> Vec<Issue> {
    let j = match Json::parse(block.trim()) {
        Ok(j) => j,
        Err(e) => {
            return vec![Issue {
                level: Level::Error,
                msg: format!("JSON-LD does not parse ({e})"),
                fix: "emit it with `jsonld={obj}` in <meta seo> so the compiler serializes it"
                    .into(),
            }];
        }
    };
    let mut out = Vec::new();
    let top: Vec<&Json> = match &j {
        Json::Arr(a) => a.iter().collect(),
        Json::Obj(_) => vec![&j],
        _ => vec![],
    };
    if top.is_empty() {
        out.push(Issue {
            level: Level::Error,
            msg: "JSON-LD is not an object".into(),
            fix: "use an object with \"@context\" and \"@type\"".into(),
        });
    }
    for t in top {
        if t.get("@context").is_none() {
            out.push(Issue {
                level: Level::Warning,
                msg: "JSON-LD without @context".into(),
                fix: "add \"@context\": \"https://schema.org\"".into(),
            });
        }
        for n in nodes(t) {
            check_node(n, true, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Organization;
    use crate::html::parse;

    fn cfg() -> Config {
        Config {
            site: Some("https://acme.dev".into()),
            search: Some("/search?q={query}".into()),
            organization: Some(Organization {
                name: "Acme Inc.".into(),
                logo: Some("/logo.png".into()),
                same_as: vec!["https://github.com/acme".into()],
            }),
            ..Config::default()
        }
    }

    fn names() -> BTreeMap<String, String> {
        [
            ("/", "Acme"),
            ("/docs", "Docs"),
            ("/docs/setup/install", "Install"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    #[test]
    fn breadcrumb_skips_missing_ancestors() {
        let b = breadcrumb(&cfg(), "/docs/setup/install", &names())
            .unwrap()
            .stringify();
        assert_eq!(
            b,
            r#"{"@context":"https://schema.org","@type":"BreadcrumbList","itemListElement":[{"@type":"ListItem","position":1,"name":"Acme","item":"https://acme.dev/"},{"@type":"ListItem","position":2,"name":"Docs","item":"https://acme.dev/docs"},{"@type":"ListItem","position":3,"name":"Install","item":"https://acme.dev/docs/setup/install"}]}"#
        );
        assert!(breadcrumb(&cfg(), "/", &names()).is_none());
        assert!(
            breadcrumb(&Config::default(), "/docs", &names()).is_none(),
            "needs absolute URLs"
        );
    }

    #[test]
    fn home_gets_website_and_organization() {
        let doc = parse(
            "<html><head><title>Acme</title></head><body><main><h1>Acme</h1></main></body></html>",
        );
        let blocks = auto(&cfg(), "/", &doc, &names(), "Acme", None, &[]);
        let all: Vec<String> = blocks.iter().map(Json::stringify).collect();
        assert_eq!(all.len(), 2);
        assert!(
            all[0].contains(r#""urlTemplate":"https://acme.dev/search?q={search_term_string}""#)
        );
        assert!(
            all[1].contains(r#""logo":"https://acme.dev/logo.png""#)
                && all[1].contains(r#""sameAs":["https://github.com/acme"]"#)
        );
        for b in &blocks {
            assert!(
                validate(&b.stringify())
                    .iter()
                    .all(|i| i.level == Level::Info),
                "{}",
                b.stringify()
            );
        }
    }

    #[test]
    fn faq_and_author_types_are_respected() {
        let doc = parse(
            "<head><script type=\"application/ld+json\">{\"@context\":\"https://schema.org\",\"@type\":\"FAQPage\",\"mainEntity\":[]}</script></head>\
<main><section faq><details><summary>Free?</summary>Yes.</details></section></main>",
        );
        assert!(
            auto(&cfg(), "/pricing", &doc, &names(), "Acme", None, &[]).is_empty(),
            "author FAQPage wins; /pricing has no breadcrumb parent page"
        );
        let doc = parse(
            "<main><section faq><details><summary>Free?</summary>Yes.</details></section></main>",
        );
        let f = faq_page(&doc).unwrap();
        assert!(validate(&f.stringify()).is_empty());
        assert!(script(&f).starts_with("<script type=\"application/ld+json\">{\"@context\""));
    }

    #[test]
    fn article_block() {
        let a = Article {
            headline: "Offline first".into(),
            published: Some("2026-10-01".into()),
            author: Some("Ada".into()),
            image: Some("/c.png".into()),
            ..Article::default()
        };
        let j = article(&cfg(), "/blog/offline-first", &a).stringify();
        assert!(
            j.contains(r#""dateModified":"2026-10-01""#)
                && j.contains(r#""image":"https://acme.dev/c.png""#)
        );
        assert!(validate(&j).is_empty(), "{:?}", validate(&j));
    }

    #[test]
    fn validation() {
        assert_eq!(validate("{not json").len(), 1);
        assert_eq!(
            validate(r#"{"@context":"https://schema.org","name":"x"}"#)[0].level,
            Level::Error
        );
        let v = validate(r#"{"@context":"https://schema.org","@type":"Product","name":"Desk"}"#);
        assert_eq!(v.len(), 1);
        assert!(
            v[0].msg
                .contains("`offers` or `review` or `aggregateRating`")
                && v[0].level == Level::Warning
        );
        let v = validate(
            r#"{"@context":"https://schema.org","@type":"FAQPage","mainEntity":[{"@type":"Question","name":"Q"}]}"#,
        );
        assert!(
            v.iter()
                .any(|i| i.msg == "Question has no `acceptedAnswer`")
        );
        let g = validate(
            r#"{"@context":"https://schema.org","@graph":[{"@type":"WebSite","name":"A","url":"https://a.dev"},{"@type":"Person"}]}"#,
        );
        assert_eq!(
            g.iter().map(|i| i.msg.as_str()).collect::<Vec<_>>(),
            vec!["Person has no `name`"]
        );
        let web_app = validate(
            r#"{"@context":"https://schema.org","@type":"WebApplication","name":"Tasks"}"#,
        );
        assert!(
            web_app.iter().all(|i| i.level == Level::Info),
            "rich-result extras for apps are only recommended"
        );
    }
}
