//! Structured data (INTERNALS §33): one automatic JSON-LD `@graph` per page (`WebSite` + `SearchAction` and
//! `Organization` on every page, `WebPage`, `BreadcrumbList` from the route tree, `FAQPage` from `<details>` groups
//! marked `faq`, `BlogPosting` for content collections, linked by `@id`) and validation of every block against the properties search engines need per type.

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

/// Article types whose name property is `headline` (the `schema="T"` shortcut fills it from the title).
pub(crate) const ARTICLES: &[&str] = &["Article", "BlogPosting", "NewsArticle", "TechArticle", "Report"];

/// `<meta seo schema="T">` left as `<meta name="schema" content="T">` (a view's own `<head>` block, an older build):
/// the node it stands for, from the page's title, description, canonical, og:image and `lastmod`. None for anything
/// that is not a plain type name.
pub fn schema_shortcut(cfg: &Config, route: &str, doc: &Doc, lastmod: Option<&str>) -> Option<Json> {
    let t = doc.metas.iter().find_map(|m| {
        crate::html::attr(m, "name")
            .filter(|n| n.eq_ignore_ascii_case("schema"))
            .and_then(|_| crate::html::attr(m, "content"))
    })?;
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut kv = vec![("@context".to_string(), s(CONTEXT)), ("@type".into(), s(t))];
    if let Some(title) = doc.title.as_deref().filter(|x| !x.trim().is_empty()) {
        let key = if ARTICLES.contains(&t) { "headline" } else { "name" };
        kv.push((key.into(), s(&title.chars().take(110).collect::<String>())));
    }
    if let Some(d) = doc.meta("description") {
        kv.push(("description".into(), s(d)));
    }
    if cfg.site.is_some() {
        kv.push(("url".into(), s(&cfg.abs(route))));
    }
    if let Some(i) = doc.meta("og:image") {
        kv.push(("image".into(), s(&cfg.abs(i))));
    }
    if let Some(d) = lastmod {
        kv.push(("dateModified".into(), s(d)));
    }
    Some(Json::Obj(kv))
}

/// Removes every `<meta name="schema" …>` (the shortcut is never a meta tag in the output; `schema_shortcut` turns
/// it into JSON-LD).
pub fn drop_schema_meta(html: &str) -> String {
    let lower = html.to_ascii_lowercase(); // same byte offsets as `html`
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    while let Some(i) = lower[at..].find("<meta").map(|i| at + i) {
        let Some(end) = lower[i..].find('>').map(|e| i + e + 1) else { break };
        let tag = &html[i..end];
        let doc = crate::html::parse(&format!("<head>{tag}</head>"));
        let is_schema = doc
            .metas
            .iter()
            .any(|m| crate::html::attr(m, "name").is_some_and(|n| n.eq_ignore_ascii_case("schema")));
        out.push_str(&html[at..i]);
        if !is_schema {
            out.push_str(tag);
        }
        at = end;
    }
    out.push_str(&html[at..]);
    out
}

/// Page types an author may declare for the page itself; any of them replaces the automatic `WebPage`.
const PAGE_TYPES: &[&str] = &[
    "WebPage", "AboutPage", "CheckoutPage", "CollectionPage", "ContactPage", "FAQPage", "ItemPage", "MedicalWebPage",
    "ProfilePage", "QAPage", "RealEstateListing", "SearchResultsPage",
];

fn id_ref(id: &str) -> Json {
    Json::obj([("@id", s(id))])
}

/// `node` as a member of a `@graph`: its `@id` first, without its own `@context`.
fn graph_node(node: Json, id: Option<&str>) -> Json {
    match node {
        Json::Obj(kv) => {
            let mut out: Vec<(String, Json)> = Vec::with_capacity(kv.len() + 1);
            let mut kv: Vec<(String, Json)> = kv.into_iter().filter(|(k, _)| k != "@context").collect();
            if let Some(id) = id {
                out.push(("@id".into(), s(id)));
            }
            // keep `@type` right after `@id`
            if let Some(i) = kv.iter().position(|(k, _)| k == "@type") {
                out.push(kv.remove(i));
            }
            out.extend(kv);
            Json::Obj(out)
        }
        other => other,
    }
}

fn set(node: &mut Json, key: &str, v: Json) {
    if let Json::Obj(kv) = node {
        match kv.iter_mut().find(|(k, _)| k == key) {
            Some((_, old)) => *old = v,
            None => kv.push((key.into(), v)),
        }
    }
}

/// Automatic structured data for one page, skipping any type the author already declared.
///
/// With an absolute site URL the page gets one `@graph` that ties its nodes together by `@id`: `WebSite`
/// (`<site>/#website`), `Organization` (`<site>/#organization`, when brain.json describes one), `WebPage`
/// (`<url>#webpage`, typed `["WebPage","FAQPage"]` when the page has a FAQ) with its `BreadcrumbList`
/// (`<url>#breadcrumb`), and the collection's `BlogPosting` (`<url>#article`) pointing at that page. Every reference
/// names a node of the same graph. Videos stay separate blocks.
#[allow(clippy::too_many_arguments)]
pub fn auto(
    cfg: &Config,
    route: &str,
    doc: &Doc,
    names: &BTreeMap<String, String>,
    site_name: &str,
    art: Option<&Article>,
    videos: &[crate::video::Video],
    lastmod: Option<&str>,
) -> Vec<Json> {
    let have = declared_types(doc);
    let declared = |t: &str| have.iter().any(|h| h == t);
    let mut out = Vec::new();
    let home = cfg.abs("/");
    if cfg.site.is_some() && home.contains("://") {
        let url = cfg.abs(route);
        let (site_id, org_id) = (format!("{home}#website"), format!("{home}#organization"));
        let (page_id, crumb_id) = (format!("{url}#webpage"), format!("{url}#breadcrumb"));
        let mut graph = Vec::new();
        let org = organization(cfg, site_name).filter(|_| !declared("Organization"));
        let site = website(cfg, site_name).filter(|_| !declared("WebSite"));
        if let Some(mut w) = site.clone() {
            if org.is_some() {
                set(&mut w, "publisher", id_ref(&org_id));
            }
            if let Some(l) = doc.lang.as_deref().filter(|l| !l.is_empty()) {
                set(&mut w, "inLanguage", s(l));
            }
            graph.push(graph_node(w, Some(&site_id)));
        }
        if let Some(o) = org.clone() {
            graph.push(graph_node(o, Some(&org_id)));
        }
        let crumb = breadcrumb(cfg, route, names).filter(|_| !declared("BreadcrumbList"));
        let faq = faq_page(doc).filter(|_| !declared("FAQPage"));
        let own_page = PAGE_TYPES.iter().any(|t| declared(t));
        let mut faq_left = faq.clone();
        if !own_page {
            let mut kv: Vec<(String, Json)> = vec![
                ("@id".into(), s(&page_id)),
                (
                    "@type".into(),
                    if faq.is_some() { Json::Arr(vec![s("WebPage"), s("FAQPage")]) } else { s("WebPage") },
                ),
                ("url".into(), s(&url)),
            ];
            if let Some(t) = doc.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                kv.push(("name".into(), s(t)));
            }
            if let Some(d) = doc.meta("description").map(str::trim).filter(|d| !d.is_empty()) {
                kv.push(("description".into(), s(d)));
            }
            if site.is_some() {
                kv.push(("isPartOf".into(), id_ref(&site_id)));
            }
            if org.is_some() {
                kv.push(("about".into(), id_ref(&org_id)));
                kv.push(("publisher".into(), id_ref(&org_id)));
            }
            if let Some(l) = doc.lang.as_deref().filter(|l| !l.is_empty()) {
                kv.push(("inLanguage".into(), s(l)));
            }
            let modified = art
                .and_then(|a| a.modified.as_deref().or(a.published.as_deref()))
                .or(lastmod);
            if let Some(d) = modified {
                kv.push(("dateModified".into(), s(d)));
            }
            if let Some(i) = doc.meta("og:image").filter(|i| !i.is_empty()) {
                kv.push((
                    "primaryImageOfPage".into(),
                    Json::obj([("@type", s("ImageObject")), ("url", s(&cfg.abs(i)))]),
                ));
            }
            if crumb.is_some() {
                kv.push(("breadcrumb".into(), id_ref(&crumb_id)));
            }
            if let Some(f) = faq.as_ref().and_then(|f| f.get("mainEntity")) {
                kv.push(("mainEntity".into(), f.clone()));
            }
            faq_left = None;
            graph.push(Json::Obj(kv));
        }
        if let Some(c) = crumb {
            graph.push(graph_node(c, Some(&crumb_id)));
        }
        if let Some(f) = faq_left {
            graph.push(graph_node(f, None));
        }
        if let Some(a) = art
            && !have.iter().any(|h| matches!(h.as_str(), "Article" | "BlogPosting" | "NewsArticle"))
        {
            let mut node = graph_node(article(cfg, route, a), Some(&format!("{url}#article")));
            if !own_page {
                set(&mut node, "isPartOf", id_ref(&page_id));
                set(&mut node, "mainEntityOfPage", id_ref(&page_id));
            }
            if org.is_some() {
                set(&mut node, "publisher", id_ref(&org_id));
            }
            graph.push(node);
        }
        if !graph.is_empty() {
            out.push(Json::obj([("@context", s(CONTEXT)), ("@graph", Json::Arr(graph))]));
        }
    } else {
        // no absolute site URL: no ids to tie together, only the self-contained blocks
        if let Some(f) = faq_page(doc).filter(|_| !declared("FAQPage")) {
            out.push(f);
        }
        if let Some(a) = art
            && !have.iter().any(|h| matches!(h.as_str(), "Article" | "BlogPosting" | "NewsArticle"))
        {
            out.push(article(cfg, route, a));
        }
    }
    if !declared("VideoObject") {
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

    /// The `@graph` nodes of the single block `auto` returns.
    fn graph(blocks: &[Json]) -> Vec<Json> {
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        match blocks[0].get("@graph") {
            Some(Json::Arr(g)) => g.clone(),
            other => panic!("no @graph: {other:?}"),
        }
    }

    fn node<'a>(g: &'a [Json], t: &str) -> &'a Json {
        g.iter().find(|n| types(n).iter().any(|x| x == t)).unwrap_or_else(|| panic!("no {t} in {g:?}"))
    }

    /// Every `{"@id": …}` reference in `j` names a node of `g`.
    fn refs_resolve(j: &Json, g: &[Json]) {
        match j {
            Json::Obj(kv) if kv.len() == 1 && kv[0].0 == "@id" => {
                let id = kv[0].1.as_str().unwrap();
                assert!(g.iter().any(|n| n.get("@id").and_then(Json::as_str) == Some(id)), "dangling {id}");
            }
            Json::Obj(kv) => kv.iter().for_each(|(_, v)| refs_resolve(v, g)),
            Json::Arr(a) => a.iter().for_each(|v| refs_resolve(v, g)),
            _ => {}
        }
    }

    #[test]
    fn home_gets_website_and_organization() {
        let doc = parse(
            "<html lang=\"en\"><head><title>Acme</title><meta name=\"description\" content=\"Plan offline.\"><meta property=\"og:image\" content=\"/og.png\"></head><body><main><h1>Acme</h1></main></body></html>",
        );
        let blocks = auto(&cfg(), "/", &doc, &names(), "Acme", None, &[], Some("2026-10-01"));
        let g = graph(&blocks);
        let site = node(&g, "WebSite").stringify();
        assert!(site.starts_with(r#"{"@id":"https://acme.dev/#website","@type":"WebSite""#), "{site}");
        assert!(site.contains(r#""urlTemplate":"https://acme.dev/search?q={search_term_string}""#));
        let org = node(&g, "Organization").stringify();
        assert!(
            org.contains(r#""@id":"https://acme.dev/#organization""#)
                && org.contains(r#""logo":"https://acme.dev/logo.png""#)
                && org.contains(r#""sameAs":["https://github.com/acme"]"#)
        );
        let page = node(&g, "WebPage").stringify();
        for part in [
            r#""@id":"https://acme.dev/#webpage""#,
            r#""url":"https://acme.dev/""#,
            r#""name":"Acme""#,
            r#""description":"Plan offline.""#,
            r#""isPartOf":{"@id":"https://acme.dev/#website"}"#,
            r#""about":{"@id":"https://acme.dev/#organization"}"#,
            r#""inLanguage":"en""#,
            r#""dateModified":"2026-10-01""#,
            r#""primaryImageOfPage":{"@type":"ImageObject","url":"https://acme.dev/og.png"}"#,
        ] {
            assert!(page.contains(part), "{part} in {page}");
        }
        assert!(!g.iter().any(|n| n.get("@context").is_some()), "one @context, on the block");
        refs_resolve(&blocks[0], &g);
        let issues = validate(&blocks[0].stringify());
        assert!(issues.iter().all(|i| i.level == Level::Info), "{issues:?}");
    }

    #[test]
    fn every_page_links_its_nodes_by_id() {
        let doc = parse(
            "<html lang=\"en\"><head><title>Install</title></head><body><main><h1>Install</h1><section faq><details><summary>Free?</summary>Yes.</details></section></main></body></html>",
        );
        let a = Article { headline: "Install".into(), published: Some("2026-10-01".into()), author: Some("Ada".into()), image: Some("/c.png".into()), ..Article::default() };
        let blocks = auto(&cfg(), "/docs/setup/install", &doc, &names(), "Acme", Some(&a), &[], None);
        let g = graph(&blocks);
        // WebSite and Organization are on every page, not only the home page
        node(&g, "WebSite");
        node(&g, "Organization");
        let page = node(&g, "WebPage");
        assert_eq!(types(page), ["WebPage", "FAQPage"], "the FAQ is the page itself");
        assert!(page.get("mainEntity").is_some());
        assert_eq!(page.get("breadcrumb").and_then(|b| b.get("@id")).and_then(Json::as_str), Some("https://acme.dev/docs/setup/install#breadcrumb"));
        assert_eq!(page.get("dateModified").and_then(Json::as_str), Some("2026-10-01"), "the entry date");
        let post = node(&g, "BlogPosting");
        for k in ["isPartOf", "mainEntityOfPage"] {
            assert_eq!(post.get(k).and_then(|b| b.get("@id")).and_then(Json::as_str), Some("https://acme.dev/docs/setup/install#webpage"), "{k}");
        }
        assert_eq!(post.get("publisher").and_then(|b| b.get("@id")).and_then(Json::as_str), Some("https://acme.dev/#organization"));
        node(&g, "BreadcrumbList");
        refs_resolve(&blocks[0], &g);
        let issues = validate(&blocks[0].stringify());
        assert!(issues.iter().all(|i| i.level == Level::Info), "{issues:?}");
        // without an absolute site URL there are no ids: only the self-contained blocks, as before
        let plain = auto(&Config::default(), "/docs/setup/install", &doc, &names(), "Acme", Some(&a), &[], None);
        assert_eq!(plain.iter().flat_map(types).collect::<Vec<_>>(), ["FAQPage", "BlogPosting"]);
    }

    #[test]
    fn faq_and_author_types_are_respected() {
        let doc = parse(
            "<head><script type=\"application/ld+json\">{\"@context\":\"https://schema.org\",\"@type\":\"FAQPage\",\"mainEntity\":[]}</script></head>\
<main><section faq><details><summary>Free?</summary>Yes.</details></section></main>",
        );
        let g = graph(&auto(&cfg(), "/pricing", &doc, &names(), "Acme", None, &[], None));
        assert_eq!(
            g.iter().flat_map(types).collect::<Vec<_>>(),
            ["WebSite", "Organization"],
            "the author's FAQPage is the page: no second FAQPage and no automatic WebPage; /pricing has no breadcrumb parent page"
        );
        // a page that declares its own WebPage keeps it, and nothing points at a #webpage that is not there
        let own = parse(
            "<head><title>About</title><script type=\"application/ld+json\">{\"@context\":\"https://schema.org\",\"@type\":\"AboutPage\",\"name\":\"About\"}</script></head><main><h1>About</h1></main>",
        );
        let a = Article { headline: "About".into(), ..Article::default() };
        let blocks = auto(&cfg(), "/docs", &own, &names(), "Acme", Some(&a), &[], None);
        let g = graph(&blocks);
        assert!(!g.iter().any(|n| types(n).iter().any(|t| t == "WebPage")), "{g:?}");
        refs_resolve(&blocks[0], &g);
        assert_eq!(node(&g, "BlogPosting").get("mainEntityOfPage").and_then(Json::as_str), Some("https://acme.dev/docs"));
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
