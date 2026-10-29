//! Sections manifest and site feeds (INTERNALS §32). `sections.json` is the one place that lists every section of the
//! site and documents it (title, description, home page, pages, Markdown copies, feeds), so a crawler or an agent can
//! discover the whole site from one file. `rss.xml` is the site-wide feed (like WordPress `/feed`) and each section
//! without a content collection gets `<section>/rss.xml`; collections keep their richer feed. On by default:
//! `seo.sections: false` drops the manifest, `seo.feed: false` drops these feeds.

use crate::config::Config;
use crate::content::{FEED_MAX, rfc822};
use crate::files::{self, Entry, xml};
use brain_lex::Json;

/// One section of the site: the first path segment (`/blog/a` → `blog`), or `""` for top-level pages.
pub struct Section<'a> {
    pub id: String,
    pub title: String,
    /// The section's own page (`/blog` for `/blog/a`), if it exists.
    pub home: Option<&'a Entry<'a>>,
    pub pages: Vec<&'a Entry<'a>>,
}

/// Indexable pages grouped by section, top-level pages first, then sections by id.
pub fn sections<'a>(entries: &'a [Entry<'a>], site: &str) -> Vec<Section<'a>> {
    // first segments that have nested pages, computed once (linear in the number of pages)
    let nested_ids: std::collections::HashSet<&str> = entries.iter().filter(|e| e.indexable && seg(e.route).1).map(|e| seg(e.route).0).collect();
    let deep = |id: &str| nested_ids.contains(id);
    let mut out: Vec<Section> = Vec::new();
    for e in entries.iter().filter(|e| e.indexable) {
        let (first, nested) = seg(e.route);
        let id = if nested || (!first.is_empty() && deep(first)) { first.to_string() } else { String::new() };
        let i = match out.iter().position(|s| s.id == id) {
            Some(i) => i,
            None => {
                let title = if id.is_empty() { "Pages".into() } else { files::section_title(&format!("/{id}/x"), entries, site) };
                out.push(Section { id: id.clone(), title, home: None, pages: vec![] });
                out.len() - 1
            }
        };
        if !id.is_empty() && !nested {
            out[i].home = Some(e);
        }
        out[i].pages.push(e);
    }
    out.sort_by(|a, b| (!a.id.is_empty(), &a.id).cmp(&(!b.id.is_empty(), &b.id)));
    out
}

/// `(first segment, has more segments)` of a route.
fn seg(route: &str) -> (&str, bool) {
    let mut it = route.split('/').filter(|s| !s.is_empty());
    (it.next().unwrap_or(""), it.next().is_some())
}

/// RSS 2.0 of `pages`, newest `lastmod` first (undated pages last), at most [`FEED_MAX`] items. `None` without `site`
/// (feed readers need absolute URLs) or without pages.
/// Item titles drop the ` · site` suffix, as in `sections.json`.
pub fn rss(cfg: &Config, title: &str, description: &str, home: &str, path: &str, pages: &[&Entry], site: &str) -> Option<String> {
    cfg.site.as_ref()?;
    let mut items: Vec<&&Entry> = pages.iter().collect();
    if items.is_empty() {
        return None;
    }
    items.sort_by(|a, b| b.lastmod.cmp(&a.lastmod).then(a.route.cmp(b.route)));
    let mut o = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n<channel>\n<title>{}</title>\n<link>{}</link>\n<description>{}</description>\n<atom:link href=\"{}\" rel=\"self\" type=\"application/rss+xml\"/>\n",
        xml(title),
        xml(&cfg.abs(home)),
        xml(description),
        xml(&cfg.abs(path))
    );
    if let Some(l) = &cfg.lang {
        o.push_str(&format!("<language>{}</language>\n", xml(l)));
    }
    if let Some((g, ..)) = &cfg.built_with {
        o.push_str(&format!("<generator>{}</generator>\n", xml(g)));
    }
    if let Some(d) = items.iter().filter_map(|e| e.lastmod).max().and_then(rfc822) {
        o.push_str(&format!("<lastBuildDate>{d}</lastBuildDate>\n"));
    }
    for e in items.into_iter().take(FEED_MAX) {
        let t = files::strip_site(&e.doc.title.clone().unwrap_or_else(|| e.route.to_string()), site);
        o.push_str(&format!("<item>\n<title>{}</title>\n<link>{}</link>\n<guid isPermaLink=\"true\">{}</guid>\n", xml(&t), xml(&e.url), xml(&e.url)));
        if let Some(d) = e.lastmod.and_then(rfc822) {
            o.push_str(&format!("<pubDate>{d}</pubDate>\n"));
        }
        if let Some(d) = e.doc.meta("description") {
            o.push_str(&format!("<description>{}</description>\n", xml(d)));
        }
        o.push_str("</item>\n");
    }
    o.push_str("</channel>\n</rss>\n");
    Some(o)
}

/// Feed path of a section (`blog/rss.xml`), the site feed for top-level pages.
pub fn feed_path(id: &str) -> String {
    if id.is_empty() { "rss.xml".into() } else { format!("{id}/rss.xml") }
}

/// `sections.json`: the site, what built it, its discovery files and every section with its pages. `feeds` holds the
/// feed paths actually written (a section is only given a feed that exists).
pub fn manifest(cfg: &Config, entries: &[Entry], secs: &[Section], feeds: &[String]) -> String {
    let site = files::site_name(cfg, entries);
    let url = |route: &str| Json::str(cfg.abs(route));
    let feed = |p: String| if feeds.contains(&p) { url(&format!("/{p}")) } else { Json::Null };
    let page = |e: &Entry| {
        let mut kv = vec![
            ("title".to_string(), Json::str(files::strip_site(&e.doc.title.clone().unwrap_or_else(|| e.route.into()), &site))),
            ("url".into(), Json::str(e.url.clone())),
        ];
        if let Some(d) = e.doc.meta("description") {
            kv.push(("description".into(), Json::str(d)));
        }
        if cfg.markdown {
            kv.push(("markdown".into(), url(&format!("/{}", files::md_path(e.route)))));
        }
        if let Some(l) = e.lastmod {
            kv.push(("lastmod".into(), Json::str(l)));
        }
        Json::Obj(kv)
    };
    let list = secs
        .iter()
        .map(|s| {
            let description = s.home.and_then(|h| h.doc.meta("description")).map(Json::str).unwrap_or(Json::Null);
            Json::Obj(vec![
                ("id".into(), Json::str(s.id.clone())),
                ("title".into(), Json::str(s.title.clone())),
                ("description".into(), description),
                ("url".into(), s.home.map(|h| Json::str(h.url.clone())).unwrap_or(Json::Null)),
                ("feed".into(), feed(feed_path(&s.id))),
                ("pages".into(), Json::Arr(s.pages.iter().map(|e| page(e)).collect())),
            ])
        })
        .collect();
    let mut kv = vec![("name".to_string(), Json::str(site))];
    if let Some(d) = files::site_description(cfg, entries) {
        kv.push(("description".into(), Json::str(d)));
    }
    kv.push(("url".into(), url("/")));
    if let Some((g, home, source)) = &cfg.built_with {
        kv.push(("generator".into(), Json::obj([("name", Json::str(g)), ("home", Json::str(home)), ("source", Json::str(source))])));
    }
    let mut discover = vec![("feed".to_string(), feed("rss.xml".into()))];
    if cfg.sitemap {
        discover.push(("sitemap".into(), url("/sitemap.xml")));
    }
    if cfg.llms {
        discover.push(("llms".into(), url("/llms.txt")));
    }
    if cfg.llms_full {
        discover.push(("llmsFull".into(), url("/llms-full.txt")));
    }
    kv.push(("discover".into(), Json::Obj(discover)));
    kv.push(("sections".into(), Json::Arr(list)));
    Json::Obj(kv).stringify() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::{Doc, parse};

    fn docs() -> Vec<(String, Doc)> {
        [
            ("/", "<html><head><title>Acme</title><meta name=description content='Tools for teams'></head><body><main><h1>Acme</h1></main></body></html>"),
            ("/about", "<html><head><title>About</title></head><body><main><h1>About</h1></main></body></html>"),
            ("/blog", "<html><head><title>Blog</title><meta name=description content='Notes from the team'></head><body><main><h1>Blog</h1></main></body></html>"),
            ("/blog/a", "<html><head><title>First</title><meta name=description content='The first one'></head><body><main><h1>First</h1></main></body></html>"),
            ("/blog/b", "<html><head><title>Second</title></head><body><main><h1>Second</h1></main></body></html>"),
            ("/admin", "<html><head><title>Admin</title><meta name=robots content=noindex></head><body><main><h1>Admin</h1></main></body></html>"),
        ]
        .iter()
        .map(|(r, h)| (r.to_string(), parse(h)))
        .collect()
    }

    fn entries<'a>(cfg: &Config, d: &'a [(String, Doc)]) -> Vec<Entry<'a>> {
        d.iter()
            .map(|(r, doc)| Entry {
                route: r,
                url: cfg.abs(r),
                doc,
                lastmod: match r.as_str() {
                    "/blog/a" => Some("2026-10-01"),
                    "/blog/b" => Some("2026-10-20"),
                    _ => None,
                },
                indexable: r != "/admin",
                alternates: vec![],
                videos: vec![],
            })
            .collect()
    }

    fn cfg() -> Config {
        Config {
            site: Some("https://acme.dev".into()),
            built_with: Some(("BRAINLLM 0.3.0".into(), "https://brain.example/".into(), "https://git.example/brain".into())),
            ..Config::default()
        }
    }

    #[test]
    fn pages_group_by_first_segment_with_their_section_home() {
        let (c, d) = (cfg(), docs());
        let e = entries(&c, &d);
        let s = sections(&e, "Acme");
        let ids: Vec<&str> = s.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["", "blog"]); // `/about` has no nested pages: it stays a top-level page; noindex is out
        let routes = |i: usize| s[i].pages.iter().map(|e| e.route).collect::<Vec<_>>();
        assert_eq!(routes(0), ["/", "/about"]);
        assert_eq!(routes(1), ["/blog", "/blog/a", "/blog/b"]);
        assert_eq!(s[1].home.map(|h| h.route), Some("/blog"));
        assert_eq!((feed_path(""), feed_path("blog")), ("rss.xml".to_string(), "blog/rss.xml".to_string()));
    }

    #[test]
    fn rss_lists_newest_first_and_names_the_generator() {
        let (c, d) = (cfg(), docs());
        let e = entries(&c, &d);
        let s = sections(&e, "Acme");
        let x = rss(&c, "Acme — Blog", "Notes", "/blog", "/blog/rss.xml", &s[1].pages, "Acme").unwrap();
        assert!(x.contains("<atom:link href=\"https://acme.dev/blog/rss.xml\" rel=\"self\""), "{x}");
        assert!(x.contains("<generator>BRAINLLM 0.3.0</generator>"));
        let (b, a) = (x.find("<title>Second</title>").unwrap(), x.find("<title>First</title>").unwrap());
        assert!(b < a, "newest lastmod first: {x}");
        assert!(x.contains("<description>The first one</description>"));
        // feed readers need absolute URLs: no site, no feed
        assert!(rss(&Config::default(), "t", "d", "/", "/rss.xml", &s[1].pages, "Acme").is_none());
    }

    #[test]
    fn manifest_lists_every_section_and_only_feeds_that_exist() {
        let (c, d) = (cfg(), docs());
        let e = entries(&c, &d);
        let s = sections(&e, "Acme");
        let j = Json::parse(&manifest(&c, &e, &s, &["blog/rss.xml".into()])).unwrap();
        assert_eq!(j.get("generator").and_then(|g| g.get("source")).and_then(Json::as_str), Some("https://git.example/brain"));
        let Some(Json::Arr(secs)) = j.get("sections") else { panic!("no sections") };
        assert_eq!(secs.len(), 2);
        assert!(matches!(secs[0].get("feed"), Some(Json::Null)), "the root feed was not written");
        assert_eq!(secs[1].get("feed").and_then(Json::as_str), Some("https://acme.dev/blog/rss.xml"));
        assert_eq!(secs[1].get("description").and_then(Json::as_str), Some("Notes from the team"));
        assert!(matches!(secs[1].get("pages"), Some(Json::Arr(p)) if p.len() == 3));
    }
}
