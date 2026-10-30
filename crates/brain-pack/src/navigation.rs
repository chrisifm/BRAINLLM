//! Build-time next-page graph: each page ships only its real outgoing routes and their immutable assets.
use crate::{Page, cache, prerender_json, tag_end};
use std::collections::{BTreeMap, BTreeSet};

fn tags(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = html[at..].find('<') {
        let start = at + i;
        if html[start..].starts_with("<!--") {
            at = html[start..]
                .find("-->")
                .map_or(html.len(), |n| start + n + 3);
            continue;
        }
        let Some(end) = tag_end(html, start) else {
            break;
        };
        let tag = &html[start..end];
        out.push(tag);
        at = end;
        // Text inside scripts/styles is not markup; template strings must not become routes.
        if tag.starts_with("<script") || tag.starts_with("<style") {
            let close = if tag.starts_with("<script") {
                "</script"
            } else {
                "</style"
            };
            at = html[at..].find(close).map_or(html.len(), |n| at + n);
        }
    }
    out
}
fn key(path: &str) -> String {
    let p = path.split(['?', '#']).next().unwrap_or(path);
    if p == "/" {
        p.into()
    } else {
        p.trim_end_matches('/').into()
    }
}
fn assets(html: &str, pre: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for tag in tags(html) {
        let url = if tag.starts_with("<script") {
            cache::attr(tag, "src")
        } else if tag.starts_with("<link")
            && cache::attr(tag, "rel")
                .is_some_and(|s| matches!(s.as_str(), "stylesheet" | "modulepreload"))
        {
            cache::attr(tag, "href")
        } else {
            None
        };
        if let Some(url) = url.filter(|u| u.starts_with(&format!("{pre}/_b/"))) {
            out.insert(url);
        }
    }
    out.into_iter().take(16).collect()
}
/// Group only adjacent equal views: moving a later catch-all ahead of a specific
/// route would change loading precedence. Patterns themselves remain unchanged.
pub(crate) fn loading(entries: &[(String, String)]) -> String {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (route, view) in entries {
        let rx = crate::routes::regex_source(route);
        if let Some((last, patterns)) = groups.last_mut() {
            if last == view {
                patterns.push(rx);
                continue;
            }
        }
        groups.push((view.clone(), vec![rx]));
    }
    groups
        .iter()
        .map(|(view, patterns)| {
            let rx = if patterns.len() == 1 {
                prerender_json(&patterns[0])
            } else {
                format!(
                    "[{}]",
                    patterns
                        .iter()
                        .map(|p| prerender_json(p))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            format!("{{rx:{rx},view:{}}}", prerender_json(view))
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Must run after base-path rewriting and before the signature/CSP/SRI pass. Does not run server loads.
pub fn apply(pages: &mut [Page], files: &mut BTreeMap<String, String>, pre: &str) {
    let routes: BTreeMap<String, (&str, Vec<String>)> = pages
        .iter()
        .map(|p| {
            (
                key(&format!("{pre}{}", p.route)),
                (p.view.as_str(), assets(&p.html, pre)),
            )
        })
        .collect();
    let mut plans = Vec::new();
    for page in pages.iter() {
        // These exact stylesheet/modulepreload URLs are already requested by this page.
        // Do not repeat or rewarm their immutable bytes in each destination plan.
        let loaded: BTreeSet<_> = assets(&page.html, pre).into_iter().collect();
        let mut next = BTreeMap::new();
        for tag in tags(&page.html) {
            if !tag.starts_with("<a ") && !tag.starts_with("<a\n") {
                continue;
            }
            if cache::attr(tag, "data-prefetch").as_deref() == Some("false") {
                continue;
            }
            let Some(href) = cache::attr(tag, "href") else {
                continue;
            };
            if !href.starts_with('/') || href.starts_with("//") || href.contains('?') {
                continue;
            }
            let target = key(&href);
            if target == key(&format!("{pre}{}", page.route)) {
                continue;
            }
            if let Some((_, assets)) = routes.get(&target) {
                next.insert(
                    href.split('#').next().unwrap_or(&href).to_string(),
                    assets
                        .iter()
                        .filter(|asset| !loaded.contains(*asset))
                        .cloned()
                        .collect::<Vec<_>>(),
                );
            }
            if next.len() >= 24 {
                break;
            }
        }
        let items: Vec<_> = next
            .iter()
            .map(|(url, assets)| {
                format!(
                    "{{\"url\":{},\"assets\":[{}]}}",
                    prerender_json(url),
                    assets
                        .iter()
                        .map(|a| prerender_json(a))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect();
        plans.push(format!("{{\"v\":1,\"next\":[{}]}}", items.join(",")));
    }
    for (page, plan) in pages.iter_mut().zip(plans) {
        let safe = plan.replace('<', "\\u003c");
        let script = format!("<script type=\"application/json\" id=\"brain-next\">{safe}</script>");
        page.html = match page.html.rfind("</body>") {
            Some(i) => format!("{}{script}{}", &page.html[..i], &page.html[i..]),
            None => format!("{}{script}", page.html),
        };
        files.insert(page.file.clone(), page.html.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loading_groups_only_contiguous_views_and_retains_individual_fallbacks() {
        let entries = vec![
            ("/a".into(), "/_b/a.js".into()),
            ("/b".into(), "/_b/a.js".into()),
            ("/fixed".into(), "/_b/b.js".into()),
            ("/[id]".into(), "/_b/a.js".into()),
        ];
        let output = loading(&entries);
        assert_eq!(
            output,
            format!(
                "{{rx:[{},{}],view:\"/_b/a.js\"}},{{rx:{},view:\"/_b/b.js\"}},{{rx:{},view:\"/_b/a.js\"}}",
                prerender_json(&crate::routes::regex_source("/a")),
                prerender_json(&crate::routes::regex_source("/b")),
                prerender_json(&crate::routes::regex_source("/fixed")),
                prerender_json(&crate::routes::regex_source("/[id]"))
            )
        );
        assert!(loading(&[]).is_empty());
    }
    #[test]
    fn plans_keep_destination_assets_but_omit_already_preloaded_immutable_urls() {
        let mut pages=vec![Page{route:"/".into(),file:"index.html".into(),view:"views/home.brain".into(),html:"<link rel='modulepreload' href='/_b/shared.js'><link rel='stylesheet' href='/_b/shared.css'><body><a href='/next'>Next</a></body>".into()},Page{route:"/next".into(),file:"next/index.html".into(),view:"views/next.brain".into(),html:"<link rel='modulepreload' href='/_b/shared.js'><link rel='stylesheet' href='/_b/shared.css'><link rel='modulepreload' href='/_b/next.js'><body>Next</body>".into()}];
        apply(&mut pages, &mut BTreeMap::new(), "");
        let marker = "<script type=\"application/json\" id=\"brain-next\">";
        let plan = pages[0]
            .html
            .split(marker)
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        assert!(plan.contains("\"url\":\"/next\""));
        assert!(plan.contains("/_b/next.js"));
        assert!(!plan.contains("/_b/shared.js") && !plan.contains("/_b/shared.css"));
    }
    #[test]
    fn graph_follows_real_links_and_base_and_ignores_script_text() {
        let mut pages=vec![Page{route:"/".into(),file:"index.html".into(),view:"views/home.brain".into(),html:"<body><a href='/app/posts/'>Posts</a><a href='https://other.test/'>External</a><script>const x=\"<a href='/app/secret/'>\"</script></body>".into()},Page{route:"/posts/".into(),file:"posts/index.html".into(),view:"views/posts.brain".into(),html:"<link rel='modulepreload' href='/app/_b/p.js'><body>Posts</body>".into()}];
        let mut files = BTreeMap::new();
        apply(&mut pages, &mut files, "/app");
        assert!(pages[0].html.contains("\"url\":\"/app/posts/\""));
        assert!(pages[0].html.contains("\"assets\":[\"/app/_b/p.js\"]"));
        assert!(!pages[0].html.contains("\"url\":\"/app/secret"));
        let first = pages[0].html.clone();
        let mut other=vec![Page{route:"/".into(),file:"index.html".into(),view:"views/home.brain".into(),html:"<body><a href='/app/posts/'>Posts</a><a href='https://other.test/'>External</a><script>const x=\"<a href='/app/secret/'>\"</script></body>".into()},Page{route:"/posts/".into(),file:"posts/index.html".into(),view:"views/posts.brain".into(),html:"<link rel='modulepreload' href='/app/_b/p.js'><body>Posts</body>".into()}];
        apply(&mut other, &mut BTreeMap::new(), "/app");
        assert_eq!(first, other[0].html);
    }
}

/// Planning metadata names future assets; size reports must not count them as current-page dependencies.
pub fn without_plan(html: &str) -> std::borrow::Cow<'_, str> {
    let marker = "<script type=\"application/json\" id=\"brain-next\">";
    let Some(start) = html.find(marker) else {
        return std::borrow::Cow::Borrowed(html);
    };
    let Some(end) = html[start..].find("</script>").map(|i| start + i + 9) else {
        return std::borrow::Cow::Borrowed(html);
    };
    std::borrow::Cow::Owned(format!("{}{}", &html[..start], &html[end..]))
}
