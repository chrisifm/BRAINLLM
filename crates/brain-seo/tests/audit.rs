//! Audit and build step over small in-memory sites.

use brain_seo::{Config, Level, Page, Site, audit, finish};

/// A per-test, per-process scratch directory, emptied first so a crashed earlier run cannot leak files into this one.
fn fresh_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("brain-seo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const LONG: &str = "Teams plan their week with this offline task planner. ";

fn page(route: &str, title: &str, desc: &str, body: &str) -> Page {
    let canonical = format!("https://acme.dev{}", if route == "/" { "/" } else { route });
    Page {
        route: route.into(),
        file: if route == "/" {
            "index.html".into()
        } else {
            format!("{}/index.html", route.trim_matches('/'))
        },
        html: format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title><meta name=\"description\" content=\"{desc}\">\
<link rel=\"canonical\" href=\"{canonical}\"><meta property=\"og:title\" content=\"{title}\"><meta property=\"og:image\" content=\"https://acme.dev/og.png\"></head>\
<body><nav><a href=\"/\">Home</a> <a href=\"/pricing\">Pricing</a> <a href=\"/blog/offline-task-planner\">Blog</a></nav><main>{body}</main></body></html>"
        ),
        ..Page::default()
    }
}

fn cfg() -> Config {
    Config {
        site: Some("https://acme.dev".into()),
        name: Some("Acme".into()),
        ..Config::default()
    }
}

fn good_site() -> Vec<Page> {
    let words = LONG.repeat(40);
    vec![
        page(
            "/",
            "Acme — offline task planner for small teams",
            "Plan the week, assign tasks and track progress offline. Free for small teams of up to ten people.",
            &format!("<h1>Acme</h1><p>{words} See <a href=\"/pricing\">pricing</a>.</p>"),
        ),
        page(
            "/pricing",
            "Pricing — Acme offline task planner for teams",
            "Simple pricing for the offline task planner: free for small teams, one flat price for larger ones.",
            &format!(
                "<h1>Pricing</h1><p>{words} Read the <a href=\"/blog/offline-task-planner\">guide</a>.</p><h2 id=\"faq\">FAQ</h2>"
            ),
        ),
        Page {
            keyword: Some("offline task planner".into()),
            ..page(
                "/blog/offline-task-planner",
                "How an offline task planner keeps teams moving",
                "Why an offline task planner beats a web-only tool when the network drops, with a setup checklist.",
                &format!(
                    "<h1>Why an offline task planner</h1><p>An offline task planner keeps working without network. {words} <a href=\"/pricing#faq\">Questions</a></p>"
                ),
            )
        },
    ]
}

#[test]
fn a_complete_page_scores_100() {
    let c = cfg();
    let pages = good_site();
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    for p in &r.pages {
        assert_eq!(p.score, Some(100), "{}: {:?}", p.route, p.findings);
    }
    assert!(r.ok());
}

#[test]
fn missing_basics_lower_the_score() {
    let c = cfg();
    let mut pages = good_site();
    pages[1].html = pages[1]
        .html
        .replace(
            "<title>Pricing — Acme offline task planner for teams</title>",
            "",
        )
        .replace("<h1>Pricing</h1>", "<h1>A</h1><h1>B</h1><h4>Deep</h4>");
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let p = &r.pages[1];
    let codes: Vec<&str> = p.findings.iter().map(|f| f.code).collect();
    assert!(
        codes.contains(&"B1801") && codes.contains(&"B1805") && codes.contains(&"B1806"),
        "{codes:?}"
    );
    assert!(p.score.unwrap() < 80);
    assert!(!r.ok(), "B1801 is an error");
    assert!(r.text().contains("B1801"));
}

#[test]
fn broken_links_and_anchors_are_errors() {
    let c = cfg();
    let mut pages = good_site();
    pages[0].html = pages[0].html.replace("</main>", "<a href=\"/nope\">x</a><a href=\"/pricing#missing\">y</a><a href=\"#top\">up</a><a href=\"/users/7\">u</a><a href=\"https://other.dev/\">ext</a><a href=\"https://acme.dev/pricing\">abs</a></main>");
    let routes = vec!["/users/:id".to_string()];
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &routes,
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let errs: Vec<String> = r
        .errors()
        .map(|(route, f)| format!("{route} {} {}", f.code, f.msg))
        .collect();
    assert_eq!(errs.len(), 2, "{errs:?}");
    assert!(errs[0].contains("B1819") && errs[0].contains("/nope"));
    assert!(errs[1].contains("B1820") && errs[1].contains("missing"));
}

#[test]
fn orphans_duplicates_and_cannibalization() {
    let c = cfg();
    let mut pages = good_site();
    let mut lonely = page(
        "/lonely",
        "Pricing — Acme offline task planner for teams",
        "Unique enough description for the lonely page that nobody links to from anywhere else.",
        "<h1>Lonely</h1>",
    );
    lonely.keyword = Some("Offline Task Planner".into());
    pages.push(lonely);
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let codes = |i: usize| {
        r.pages[i]
            .findings
            .iter()
            .map(|f| f.code)
            .collect::<Vec<_>>()
    };
    assert!(codes(3).contains(&"B1813"), "orphan: {:?}", codes(3));
    assert!(
        codes(3).contains(&"B1814") && codes(1).contains(&"B1814"),
        "duplicate title"
    );
    assert!(
        codes(3).contains(&"B1817") && codes(2).contains(&"B1817"),
        "same keyword"
    );
    assert!(!codes(0).contains(&"B1813"), "home is never an orphan");
}

#[test]
fn keyword_placement() {
    let c = cfg();
    let mut pages = good_site();
    pages[2].keyword = Some("kanban board".into());
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let kw: Vec<&str> = r.pages[2]
        .findings
        .iter()
        .filter(|f| f.code == "B1816")
        .map(|f| f.msg.as_str())
        .collect();
    assert_eq!(
        kw.len(),
        5,
        "title, h1, first paragraph, URL and density: {kw:?}"
    );
}

#[test]
fn noindex_and_canonicalized_pages_are_not_scored() {
    let c = cfg();
    let mut pages = good_site();
    pages[1].html = pages[1]
        .html
        .replace("<head>", "<head><meta name=\"robots\" content=\"noindex\">");
    pages[2].html = pages[2].html.replace(
        "https://acme.dev/blog/offline-task-planner\"",
        "https://acme.dev/pricing\"",
    );
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    assert_eq!((r.pages[1].score, r.pages[1].indexable), (None, false));
    assert_eq!((r.pages[2].score, r.pages[2].indexable), (None, false));
    assert_eq!(r.pages[0].score, Some(100));
}

#[test]
fn heavy_main_image() {
    let c = cfg();
    let mut pages = good_site();
    pages[0].html = pages[0].html.replace(
        "<h1>Acme</h1>",
        "<h1>Acme</h1><img src=\"/hero.png\" alt=\"Team board\">",
    );
    let assets = vec![("hero.png".to_string(), 512 * 1024)];
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &assets,
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let f = r.pages[0]
        .findings
        .iter()
        .find(|f| f.code == "B1821")
        .expect("B1821");
    assert_eq!(f.level, Level::Warning);
    assert!(f.msg.contains("512 KB"));
}

#[test]
fn finish_rewrites_heads_and_generates_files() {
    let dir = fresh_dir("finish");
    std::fs::create_dir_all(dir.join("public")).unwrap();
    std::fs::write(
        dir.join("tokens.cssbrain"),
        ":root { --bg: #f5f5f7; --accent: #0071e3; }",
    )
    .unwrap();
    std::fs::write(dir.join("public/favicon.svg"), "<svg/>").unwrap();
    let mut c = cfg();
    c.google.analytics = Some("G-AB12".into());
    let mut pages = good_site();
    pages[0].html = pages[0]
        .html
        .replace("</main>", "<a href=\"/favicon.svg\">icon</a></main>");
    let built = finish(&c, &dir, &mut pages, &[], &[]).unwrap();
    let names: Vec<&str> = built.files.iter().map(|(p, _)| p.as_str()).collect();
    for f in [
        "robots.txt",
        "sitemap.xml",
        "llms.txt",
        "llms-full.txt",
        "index.md",
        "pricing.md",
        "blog/offline-task-planner.md",
        "manifest.webmanifest",
    ] {
        assert!(names.contains(&f), "{f} missing: {names:?}");
    }
    let g = names
        .iter()
        .find(|n| n.starts_with("_b/g."))
        .expect("google bootstrap");
    assert!(g.ends_with(".js"));
    assert!(
        pages[1]
            .html
            .contains("<link rel=\"alternate\" type=\"text/markdown\" href=\"/pricing.md\">")
    );
    assert!(built.files.iter().any(|(p, c)| p == "pricing.md"
        && c.starts_with("# Pricing\nSource: https://acme.dev/pricing\n")));
    assert!(pages[0].html.contains("<link rel=\"manifest\" href=\"/manifest.webmanifest\"><meta name=\"theme-color\" content=\"#0071e3\">"));
    assert!(pages[0].html.contains("gtag/js?id=G-AB12"));
    assert!(built.csp.iter().any(|(d, _)| *d == "connect-src"));
    assert!(built.report.ok());
    // same input → same bytes (invariant 8)
    let again = finish(&c, &dir, &mut good_site(), &[], &[]).unwrap();
    assert_eq!(built.files[2], again.files[2]);

    // a broken link fails the build
    let mut broken = good_site();
    broken[1].html = broken[1]
        .html
        .replace("</main>", "<a href=\"/gone\">old</a></main>");
    let err = finish(&c, &dir, &mut broken, &[], &[]).err().unwrap();
    assert!(
        err.contains("B1819 /pricing") && err.contains("fix:"),
        "{err}"
    );

    // staging: noindex everywhere, robots blocks all, no sitemap
    let staging = Config {
        staging: true,
        ..cfg()
    };
    let mut pages = good_site();
    let built = finish(&staging, &dir, &mut pages, &[], &[]).unwrap();
    assert!(pages.iter().all(|p| {
        p.html
            .contains("<meta name=\"robots\" content=\"noindex, nofollow\">")
            && p.html.matches("name=\"robots\"").count() == 1
    }));
    assert_eq!(built.files[0].1, "User-agent: *\nDisallow: /\n");
    assert!(!built.files.iter().any(|(p, _)| p == "sitemap.xml"));
    assert!(built.report.pages.iter().all(|p| p.score.is_none()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn absolute_canonical_without_known_site() {
    let c = Config {
        site: None,
        ..cfg()
    };
    let pages = good_site();
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    assert!(
        r.pages.iter().all(|p| p.indexable),
        "self canonical on an unknown host is still self"
    );
    let docs: Vec<_> = pages
        .iter()
        .map(|p| brain_seo::html::parse(&p.html))
        .collect();
    assert_eq!(
        brain_seo::audit::infer_site(&docs).as_deref(),
        Some("https://acme.dev")
    );
    assert_eq!(
        brain_seo::audit::url_path("https://a.dev/x/?q=1#f"),
        Some("/x/")
    );
}

#[test]
fn finish_adds_structured_data() {
    let dir = fresh_dir("ld");
    std::fs::create_dir_all(&dir).unwrap();
    let mut c = cfg();
    c.organization = Some(brain_seo::config::Organization {
        name: "Acme Inc.".into(),
        logo: None,
        same_as: vec![],
    });
    let mut pages = good_site();
    pages.push(page(
        "/blog",
        "Blog — Acme offline task planner for small teams",
        "Guides, release notes and stories from teams that plan their work offline with Acme.",
        "<h1>Blog</h1><p><a href=\"/blog/offline-task-planner\">Guide</a></p>",
    ));
    pages[1].html = pages[1].html.replace("</main>", "<section faq><details><summary>Is it free?</summary>Yes, up to ten people.</details></section></main>");
    pages[2].article = Some(brain_seo::jsonld::Article {
        headline: "Offline task planner".into(),
        published: Some("2026-10-01".into()),
        author: Some("Ada".into()),
        image: Some("/c.png".into()),
        ..Default::default()
    });
    let built = finish(&c, &dir, &mut pages, &[], &[]).unwrap();
    let ld = |i: usize| brain_seo::html::parse(&pages[i].html).jsonld;
    assert_eq!(
        brain_seo::jsonld::declared_types(&brain_seo::html::parse(&pages[0].html)),
        vec!["WebSite", "Organization"]
    );
    assert!(
        ld(1)
            .iter()
            .any(|b| b.contains("\"FAQPage\"") && b.contains("Is it free?"))
    );
    let post = ld(2).join("\n");
    assert!(
        post.contains("\"BreadcrumbList\"")
            && post.contains("\"name\":\"Blog\"")
            && post.contains("\"BlogPosting\""),
        "{post}"
    );
    // every generated block validates: no B1818 anywhere
    assert!(
        built.report.pages.iter().all(|p| p
            .findings
            .iter()
            .all(|f| f.code != "B1818" || f.level == Level::Info)),
        "{:?}",
        built
            .report
            .pages
            .iter()
            .map(|p| &p.findings)
            .collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn schema_meta_becomes_json_ld() {
    let dir = fresh_dir("schema-meta");
    let mut pages = good_site();
    pages[1].html = pages[1].html.replace(
        "</head>",
        "<meta name=\"schema\" content=\"Article\"></head>",
    );
    pages[1].lastmod = Some("2026-10-01".into());
    finish(&cfg(), &dir, &mut pages, &[], &[]).unwrap();
    let html = &pages[1].html;
    assert!(!html.contains("name=\"schema\""), "{html}");
    let ld = brain_seo::html::parse(html).jsonld.join("\n");
    assert!(
        ld.contains("\"@type\":\"Article\",\"headline\":\"Pricing — Acme offline task planner for teams\"")
            && ld.contains("\"url\":\"https://acme.dev/pricing\"")
            && ld.contains("\"dateModified\":\"2026-10-01\""),
        "{ld}"
    );
    assert_eq!(
        brain_seo::jsonld::declared_types(&brain_seo::html::parse(html))
            .iter()
            .filter(|t| *t == "Article")
            .count(),
        1
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_author_json_ld_is_reported() {
    let c = cfg();
    let mut pages = good_site();
    pages[0].html = pages[0].html.replace("</head>", "<script type=\"application/ld+json\">{\"@context\":\"https://schema.org\",\"name\":\"x\"}</script></head>");
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let f = r.pages[0]
        .findings
        .iter()
        .find(|f| f.code == "B1818")
        .expect("B1818");
    assert_eq!(f.level, Level::Error);
    assert!(!r.ok());
}

#[test]
fn internal_link_suggestions() {
    let c = cfg();
    let mut pages = good_site();
    let mut sync = page(
        "/sync",
        "Conflict-free sync for distributed teams — Acme",
        "How Acme merges edits made offline by several people without conflicts or lost changes.",
        "<h1>Conflict-free sync</h1><p>Edits merge, even inside the offline task planner.</p>",
    );
    sync.keyword = Some("conflict-free sync".into());
    pages.push(sync);
    pages[1].html = pages[1].html.replace(
        "</main>",
        "<p>Every plan includes conflict-free sync across devices.</p></main>",
    );
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let orphan = r.pages[3]
        .findings
        .iter()
        .find(|f| f.code == "B1813")
        .expect("orphan");
    assert_eq!(
        orphan.fix,
        "link it from /pricing (mentions \"conflict-free sync\")"
    );
    let out = r.pages[3]
        .findings
        .iter()
        .find(|f| f.code == "B1812")
        .expect("no links out");
    assert!(
        out.fix
            .starts_with("link from the text to /blog/offline-task-planner"),
        "{}",
        out.fix
    );
}

#[test]
fn mcp_tool_over_a_dist() {
    let dir = fresh_dir("mcp");
    for p in good_site() {
        let f = dir.join("dist").join(&p.file);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, p.html).unwrap();
    }
    let args = brain_lex::Json::obj([("route", brain_lex::Json::str("/pricing"))]);
    let j = brain_seo::tool("brain_seo_audit", &args, &dir).unwrap();
    let pages = match j.get("pages") {
        Some(brain_lex::Json::Arr(a)) => a.clone(),
        _ => panic!(),
    };
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].get("score"), Some(&brain_lex::Json::Num(100.0)));
    assert_eq!(j.get("ok"), Some(&brain_lex::Json::Bool(true)));
    assert!(
        brain_seo::tool(
            "brain_seo_audit",
            &brain_lex::Json::obj([]),
            &dir.join("nope")
        )
        .unwrap_err()
        .contains("brain build")
    );
    assert!(
        brain_seo::TOOLS
            .iter()
            .all(|(n, d, _)| n.starts_with("brain_seo_") && !d.is_empty())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn percent_encoded_routes() {
    let c = cfg();
    let mut pages = good_site();
    let post = page(
        "/post/b%20c",
        "A post whose slug has a space in the address bar",
        "Routes from paths() arrive percent-encoded; links may use either the raw or the encoded form.",
        "<h1>Post</h1>",
    );
    pages.push(post);
    pages[0].html = pages[0].html.replace(
        "</main>",
        "<a href=\"/post/b c\">raw</a><a href=\"/post/b%20c\">encoded</a></main>",
    );
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    assert!(
        r.errors().next().is_none(),
        "{:?}",
        r.errors().map(|(_, f)| f.msg.clone()).collect::<Vec<_>>()
    );
    assert!(
        r.pages[3].indexable,
        "self canonical with %20 matches the route"
    );
    assert_eq!(
        brain_seo::files::encode_path("/post/b c/\u{e9}"),
        "/post/b%20c/%C3%A9"
    );
    assert_eq!(brain_seo::files::encode_path("/post/b%20c"), "/post/b%20c");
}

#[test]
fn short_titles() {
    let c = cfg();
    let mut pages = good_site();
    pages[1].html = pages[1].html.replace(
        "<title>Pricing — Acme offline task planner for teams</title>",
        "<title>Pricing</title>",
    );
    pages[2].html = pages[2].html.replace(
        "<title>How an offline task planner keeps teams moving</title>",
        "<title>About · Acme</title>",
    );
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    assert!(
        r.pages[1].findings.iter().any(|f| f.code == "B1802"),
        "a bare topic is too short"
    );
    assert!(
        !r.pages[2].findings.iter().any(|f| f.code == "B1802"),
        "topic + site is enough"
    );
}

#[test]
fn finish_with_content_collections() {
    let dir = fresh_dir("content");
    std::fs::create_dir_all(dir.join("content/blog")).unwrap();
    std::fs::write(dir.join("content/blog/offline-first.md"), "---\ntitle: Offline first\ndescription: Why the planner works without network, and what it syncs later.\ndate: 2026-10-01\ntags: [sync]\nauthor: Ada\n---\n# Offline first\n\nText.\n").unwrap();
    std::fs::write(dir.join("content/blog/older.md"), "---\ntitle: Older post\ndescription: An older post about the planner and the first sync engine we shipped.\ndate: 2026-01-05\n---\nOld text.\n").unwrap();
    std::fs::write(
        dir.join("content/blog/draft.md"),
        "---\ntitle: Draft\ndraft: true\n---\nNot yet.\n",
    )
    .unwrap();
    let mut c = cfg();
    c.per_page = 1;
    let listing = |r: &str| {
        page(
            r,
            "Blog — Acme offline task planner for small teams",
            "Guides, release notes and stories from teams that plan their work offline with Acme.",
            "<h1>Blog</h1><p><a href=\"/blog/offline-first\">a</a> <a href=\"/blog/older\">b</a> <a href=\"/blog/page/2\">next</a></p>",
        )
    };
    let mut pages = good_site();
    pages.extend([
        listing("/blog"),
        listing("/blog/page/2"),
        page(
            "/blog/offline-first",
            "Offline first — Acme offline task planner",
            "Why the planner works without network, and what it syncs later.",
            "<h1>Offline first</h1><p><a href=\"/blog\">Blog</a></p>",
        ),
        page(
            "/blog/older",
            "Older post — Acme offline task planner blog",
            "An older post about the planner and the first sync engine we shipped.",
            "<h1>Older post</h1><p><a href=\"/blog\">Blog</a></p>",
        ),
    ]);
    let built = finish(&c, &dir, &mut pages, &[], &[]).unwrap();
    let file = |n: &str| {
        built
            .files
            .iter()
            .find(|(p, _)| p == n)
            .map(|(_, c)| c.clone())
            .unwrap_or_else(|| panic!("{n} missing"))
    };
    let rss = file("blog/rss.xml");
    assert!(
        rss.contains("<title>Offline first</title>")
            && rss.contains("<title>Older post</title>")
            && !rss.contains("Draft")
    );
    assert!(file("blog/atom.xml").contains("<published>2026-10-01T00:00:00Z</published>"));
    assert!(
        file("sitemap.xml").contains(
            "<loc>https://acme.dev/blog/offline-first</loc><lastmod>2026-10-01</lastmod>"
        )
    );
    let blog = pages.iter().find(|p| p.route == "/blog").unwrap();
    assert!(
        blog.html
            .contains("<link rel=\"next\" href=\"https://acme.dev/blog/page/2\">")
            && !blog.html.contains("rel=\"prev\"")
    );
    assert!(blog.html.contains("<link rel=\"alternate\" type=\"application/rss+xml\" title=\"blog\" href=\"/blog/rss.xml\">"));
    let p2 = pages.iter().find(|p| p.route == "/blog/page/2").unwrap();
    assert!(
        p2.html
            .contains("<link rel=\"prev\" href=\"https://acme.dev/blog\">")
            && !p2.html.contains("rel=\"next\"")
    );
    let post = pages
        .iter()
        .find(|p| p.route == "/blog/offline-first")
        .unwrap();
    assert!(
        post.html.contains("\"@type\":\"BlogPosting\"")
            && post.html.contains("\"datePublished\":\"2026-10-01\"")
    );
    assert_eq!(post.lastmod.as_deref(), Some("2026-10-01"));
    assert!(built.report.pages.iter().all(|p| {
        p.findings
            .iter()
            .all(|f| f.code != "B1818" || f.level == Level::Info)
    }));
    // a broken frontmatter fails the build
    std::fs::write(
        dir.join("content/blog/bad.md"),
        "---\ntitle: Bad\ndate: soon\n---\nx\n",
    )
    .unwrap();
    let err = finish(&c, &dir, &mut good_site(), &[], &[]).err().unwrap();
    assert!(
        err.contains("B1823 content/blog/bad.md:3: `date: soon` is not a date"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn content_module_runs_in_node() {
    let Ok(v) = std::process::Command::new("node").arg("--version").output() else {
        return;
    };
    if !v.status.success() {
        return;
    }
    let mut cols = Vec::new();
    for (i, slug) in ["a", "b", "c"].iter().enumerate() {
        let src = format!(
            "---\ntitle: Post {slug}\ndescription: Description of post {slug} long enough to be a summary.\ndate: 2026-10-0{}\ntags: [Sync Engine]\n---\nBody {slug}.\n",
            i + 1
        );
        let (e, p) = brain_seo::content::parse_entry("blog", &format!("{slug}.md"), &src);
        assert!(p.is_empty(), "{p:?}");
        cols.push(e.unwrap());
    }
    cols.sort_by(|a, b| b.date.cmp(&a.date));
    let module = brain_seo::content::module_js(
        &[brain_seo::content::Collection {
            name: "blog".into(),
            entries: cols,
            ..Default::default()
        }],
        2,
    );
    let script = format!(
        "{module}\nconst out = [entries('blog').map((e) => e.slug).join(), entry('blog', 'b').title, entry('blog', 'zz'), JSON.stringify(pages('blog')), JSON.stringify(page('blog', 2)), tagged('blog', 'sync-engine').length, tags('blog')[0].slug];\ntry {{ entries('news') }} catch (e) {{ out.push(e.message) }}\nconsole.log(JSON.stringify(out))"
    );
    let out = std::process::Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = String::from_utf8_lossy(&out.stdout);
    assert!(
        got.starts_with(r#"["c,b,a","Post b",null,"[{\"n\":\"2\"}]""#),
        "{got}"
    );
    assert!(
        got.contains(r#"\"n\":2,\"pages\":2,\"prev\":\"/blog\",\"next\":null"#),
        "{got}"
    );
    assert!(
        got.contains(r#",3,"sync-engine","content/news/ does not exist; collections: blog"]"#),
        "{got}"
    );
}

#[test]
fn finish_with_redirects() {
    let dir = fresh_dir("redir");
    std::fs::create_dir_all(dir.join("dist")).unwrap();
    std::fs::write(dir.join("redirects.json"), r#"[{ "from": "/old-pricing", "to": "/pricing" }, { "from": "/blog/:slug", "to": "/posts/:slug", "status": 308 }]"#).unwrap();
    std::fs::write(dir.join("dist/sitemap.xml"), "<urlset><url><loc>https://acme.dev/</loc></url><url><loc>https://acme.dev/old-pricing</loc></url><url><loc>https://acme.dev/guides/offline-task-planner</loc></url></urlset>").unwrap();
    let c = cfg();
    let mut pages = good_site();
    pages[0].html = pages[0]
        .html
        .replace("</main>", "<a href=\"/old-pricing\">old</a></main>");
    let routes = vec!["/posts/:slug".to_string()];
    let built = finish(&c, &dir, &mut pages, &routes, &[]).unwrap();
    assert_eq!(
        built
            .files
            .iter()
            .find(|(p, _)| p == "_redirects")
            .unwrap()
            .1,
        "/old-pricing /pricing 301\n/blog/:slug /posts/:slug 308\n"
    );
    assert_eq!(built.redirects.len(), 2);
    let w = built
        .warnings
        .iter()
        .find(|w| w.starts_with("B1826"))
        .expect("removed URL warning");
    assert!(
        w.contains("`/guides/offline-task-planner`")
            && w.contains("\"to\": \"/blog/offline-task-planner\""),
        "{w}"
    );
    assert!(
        !built.warnings.iter().any(|w| w.contains("`/old-pricing`")),
        "redirected URLs are not reported"
    );
    let home = &built.report.pages[0];
    assert!(home.findings.iter().any(|f| f.code == "B1825"
        && f.level == Level::Info
        && f.fix == "link `/pricing` directly"));
    assert!(built.report.ok());
    // a rule from a live page fails the build
    std::fs::write(
        dir.join("redirects.json"),
        r#"[{ "from": "/pricing", "to": "/" }]"#,
    )
    .unwrap();
    let err = finish(&c, &dir, &mut good_site(), &[], &[]).err().unwrap();
    assert!(
        err.contains("B1825 redirects.json: `/pricing` is still a page"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finish_adds_hreflang() {
    let dir = std::env::temp_dir().join(format!("brain-seo-i18n-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("i18n")).unwrap();
    for l in ["en", "es"] {
        std::fs::write(dir.join(format!("i18n/{l}.json")), "{}").unwrap();
    }
    std::fs::write(dir.join("i18n/Spanish.json"), "{}").unwrap();
    let c = Config {
        lang: Some("en".into()),
        ..cfg()
    };
    let mut pages = good_site();
    pages.push(page("/es/pricing", "Pricing in the second language — Acme task planner", "Translated pricing page used to check that hreflang links pair it with the default-language page.", "<h1>Pricing (es)</h1><p><a href=\"/pricing\">English</a></p>"));
    let built = finish(&c, &dir, &mut pages, &[], &[]).unwrap();
    let en = &pages[1].html;
    assert!(
        en.contains(
            "<link rel=\"alternate\" hreflang=\"es\" href=\"https://acme.dev/es/pricing\">"
        ) && en.contains("hreflang=\"x-default\" href=\"https://acme.dev/pricing\"")
    );
    assert!(
        pages[3]
            .html
            .contains("hreflang=\"en\" href=\"https://acme.dev/pricing\"")
    );
    assert!(
        !pages[0].html.contains("hreflang"),
        "home exists in one language only"
    );
    assert!(
        built
            .warnings
            .iter()
            .any(|w| w.contains("i18n/Spanish.json"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_over_a_dist() {
    let dir = fresh_dir("preview");
    for p in good_site() {
        let f = dir.join("dist").join(&p.file);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, p.html).unwrap();
    }
    let (code, out) = brain_seo::cli(&dir, &["preview".into(), "/pricing".into()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("Search result (desktop)\n  acme.dev \u{203a} pricing\n  Pricing — Acme offline task planner for teams\n"), "{out}");
    assert!(out.contains("Audit: 100/100"));
    let j = brain_seo::tool(
        "brain_seo_preview",
        &brain_lex::Json::obj([("route", brain_lex::Json::str("/nope"))]),
        &dir,
    )
    .unwrap_err();
    assert!(
        j.contains("no built page at `/nope`") && j.contains("/pricing"),
        "{j}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn indexnow_key_file_and_dry_run() {
    let dir = fresh_dir("indexnow");
    std::fs::create_dir_all(&dir).unwrap();
    let c = Config {
        indexnow: true,
        indexnow_key: Some("abcdef0123456789".into()),
        ..cfg()
    };
    let built = finish(&c, &dir, &mut good_site(), &[], &[]).unwrap();
    assert!(
        built
            .files
            .iter()
            .any(|(p, k)| p == "abcdef0123456789.txt" && k == "abcdef0123456789")
    );
    let sm = built
        .files
        .iter()
        .find(|(p, _)| p == "sitemap.xml")
        .unwrap()
        .1
        .clone();
    std::fs::create_dir_all(dir.join("dist")).unwrap();
    std::fs::write(dir.join("dist/sitemap.xml"), sm).unwrap();
    std::fs::write(
        dir.join("brain.json"),
        r#"{"site":"https://acme.dev","seo":{"indexnow":true}}"#,
    )
    .unwrap();
    // the key comes from .brain_env for the chosen environment (process environment would override it)
    if std::env::var("INDEXNOW_KEY").is_err() {
        let (code, out) = brain_seo::cli(&dir, &["indexnow".into(), "--dry-run".into()]);
        assert_eq!(code, 1);
        assert!(
            out.contains("INDEXNOW_KEY is empty for `production`") && out.contains(".brain_env"),
            "{out}"
        );
        std::fs::write(dir.join(".brain_env"), "[staging]\nINDEXNOW_KEY=stagingkey0123\n\n[production]\nINDEXNOW_KEY=prodkey0123456789\n").unwrap(); // gitleaks:allow -- synthetic environment-selection fixture
        let (code, out) = brain_seo::cli(&dir, &["indexnow".into(), "--dry-run".into()]);
        assert_eq!(code, 0, "{out}");
        assert!(
            out.contains("\"key\":\"prodkey0123456789\"")
                && out.contains("\"keyLocation\":\"https://acme.dev/prodkey0123456789.txt\""),
            "{out}"
        );
        let (_, out) = brain_seo::cli(
            &dir,
            &[
                "indexnow".into(),
                "--dry-run".into(),
                "--env".into(),
                "staging".into(),
            ],
        );
        assert!(out.contains("stagingkey0123"), "{out}");
        assert_eq!(
            Config::load(&dir).unwrap().indexnow_key.as_deref(),
            Some("prodkey0123456789")
        );
        std::fs::remove_file(dir.join(".brain_env")).unwrap();
    }
    let off = finish(
        &Config {
            indexnow: true,
            ..cfg()
        },
        &dir,
        &mut good_site(),
        &[],
        &[],
    )
    .unwrap();
    assert!(
        off.warnings
            .iter()
            .any(|w| w.contains("INDEXNOW_KEY is empty")),
        "{:?}",
        off.warnings
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finish_social_and_ai_layer() {
    let dir = fresh_dir("social");
    std::fs::create_dir_all(dir.join("public")).unwrap();
    let mut small = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    small.extend(800u32.to_be_bytes());
    small.extend(400u32.to_be_bytes());
    std::fs::write(dir.join("public/small.png"), &small).unwrap();
    let j = brain_lex::Json::parse(r#"{"site":"https://acme.dev","name":"Acme","lang":"en","seo":{"ai":"search-only"},
        "social":{"x":"@acme","instagram":"https://www.instagram.com/acme","linkedin":"https://www.linkedin.com/company/acme","facebook":"https://www.facebook.com/acme"}}"#).unwrap();
    let c = Config::from_json(&j).unwrap();
    let mut pages = good_site();
    pages[1].html = pages[1]
        .html
        .replace("https://acme.dev/og.png", "https://acme.dev/small.png");
    pages[2].article = Some(brain_seo::jsonld::Article {
        headline: "Offline task planner".into(),
        published: Some("2026-10-01".into()),
        ..Default::default()
    });
    let built = finish(&c, &dir, &mut pages, &[], &[]).unwrap();
    let home = &pages[0].html;
    for t in [
        "<link rel=\"me\" href=\"https://www.instagram.com/acme\">",
        "<meta name=\"twitter:site\" content=\"@acme\">",
        "<meta property=\"og:site_name\" content=\"Acme\">",
        "<meta property=\"og:locale\" content=\"en_US\">",
        "<meta name=\"twitter:card\" content=\"summary_large_image\">",
        "max-image-preview:large",
    ] {
        assert!(home.contains(t), "{t} missing");
    }
    assert!(home.contains("\"sameAs\":[\"https://x.com/acme\",\"https://www.instagram.com/acme\",\"https://www.linkedin.com/company/acme\",\"https://www.facebook.com/acme\"]"), "Organization from social profiles");
    assert!(
        pages[2]
            .html
            .contains("<meta property=\"og:type\" content=\"article\">")
            && pages[2].html.matches("og:type").count() == 1
    );
    assert!(
        pages[1]
            .html
            .contains("<meta property=\"og:image:width\" content=\"800\">")
    );
    let robots = &built
        .files
        .iter()
        .find(|(p, _)| p == "robots.txt")
        .unwrap()
        .1;
    assert!(
        robots.contains("Content-Signal: search=yes, ai-input=yes, ai-train=no")
            && robots.contains("User-agent: GPTBot")
            && !robots.contains("User-agent: OAI-SearchBot")
    );
    let f = built.report.pages[1]
        .findings
        .iter()
        .find(|f| f.code == "B1827")
        .expect("B1827");
    assert!(f.msg.contains("800x400"));
    let llms = &built.files.iter().find(|(p, _)| p == "llms.txt").unwrap().1;
    assert!(llms.contains("(https://acme.dev/pricing.md)"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finish_renders_social_images() {
    let dir = fresh_dir("og");
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend(1200u32.to_be_bytes());
    png.extend(630u32.to_be_bytes());
    let seen = std::cell::RefCell::new(Vec::new());
    let shoot = |j: &brain_seo::og::Job, tpl: &str| {
        assert!(tpl.contains("<meta props=\"title:str"));
        seen.borrow_mut().push(j.props[0].1.clone());
        Ok(png.clone())
    };
    let mut pages = good_site();
    pages[1].html = pages[1].html.replace(
        "<meta property=\"og:image\" content=\"https://acme.dev/og.png\">",
        "",
    );
    let built =
        brain_seo::finish_with_og(&cfg(), &dir, &mut pages, &[], &[], Some(&shoot)).unwrap();
    assert_eq!(
        *seen.borrow(),
        vec!["Pricing — Acme offline task planner for teams".to_string()],
        "only the page without og:image"
    );
    assert_eq!(
        built
            .binary
            .iter()
            .map(|(p, _)| p.as_str())
            .collect::<Vec<_>>(),
        vec!["og/pricing.png"]
    );
    let html = &pages[1].html;
    assert!(html.contains("<meta property=\"og:image\" content=\"https://acme.dev/og/pricing.png\"><meta property=\"og:image:width\" content=\"1200\">"));
    assert_eq!(html.matches("og:image:width").count(), 1);
    assert!(html.contains("<meta name=\"twitter:card\" content=\"summary_large_image\">"));
    let p = built
        .report
        .pages
        .iter()
        .find(|p| p.route == "/pricing")
        .unwrap();
    assert!(
        !p.findings
            .iter()
            .any(|f| f.code == "B1811" || f.code == "B1827"),
        "{:?}",
        p.findings
    );
    assert!(dir.join(".brain/cache/og").read_dir().unwrap().count() == 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finish_multilingual_site() {
    let dir = fresh_dir("multilang");
    for (l, slug, title) in [
        ("en", "a", "Offline first"),
        ("es", "a", "Offline first in the second language"),
        ("en", "b", "Second post"),
    ] {
        std::fs::create_dir_all(dir.join(format!("content/blog/{l}"))).unwrap();
        std::fs::write(dir.join(format!("content/blog/{l}/{slug}.md")), format!("---\ntitle: {title}\ndescription: A description for {title} that is long enough for a search snippet.\ndate: 2026-10-0{}\n---\nBody.\n", if slug == "a" { 1 } else { 2 })).unwrap();
    }
    let j = brain_lex::Json::parse(r#"{"site":"https://acme.dev","name":"Acme","i18n":{"default":"en","locales":["en","es"]}}"#).unwrap();
    let c = Config::from_json(&j).unwrap();
    let mut pages = good_site();
    pages[1].view = Some("views/pricing.brain".into());
    let mut es = page(
        "/es/precios",
        "Pricing page in the second language — Acme",
        "Translated pricing with a translated slug, paired with the default page by its view file.",
        "<h1>Pricing (es)</h1><p><a href=\"/pricing\">en</a></p>",
    );
    es.view = Some("views/pricing.brain".into());
    es.html = es.html.replace("<html lang=\"en\">", "<html lang=\"es\">");
    pages.push(es);
    for r in ["/blog/a", "/es/blog/a", "/blog/b", "/es/blog/b"] {
        let mut p = page(
            r,
            &format!("Post {r} — Acme offline task planner blog"),
            "A description that is long enough for the search result snippet of this post.",
            "<h1>Post</h1><p><a href=\"/\">home</a></p>",
        );
        p.view = Some("views/blog/[slug].brain".into());
        if r.starts_with("/es/") {
            p.html = p.html.replace("<html lang=\"en\">", "<html lang=\"es\">");
        }
        pages.push(p);
    }
    // a view's literal canonical="/about" rendered for /es/about points at the default version: it becomes its own
    let mut about = page(
        "/es/about",
        "About the team in the second language — Acme",
        "Who builds the offline task planner, rendered from the same view as the default page.",
        "<h1>About (es)</h1><p><a href=\"/\">home</a></p>",
    );
    about.html = about
        .html
        .replace("https://acme.dev/es/about\"", "https://acme.dev/about\"")
        .replace("<html lang=\"en\">", "<html lang=\"es\">");
    assert!(about.html.contains("href=\"https://acme.dev/about\""));
    pages.push(about);
    let built = finish(&c, &dir, &mut pages, &[], &[]).unwrap();
    let by = |r: &str| pages.iter().find(|p| p.route == r).unwrap().html.clone();
    let about = by("/es/about");
    assert!(
        about.contains("<link rel=\"canonical\" href=\"https://acme.dev/es/about\">")
            && about.matches("rel=\"canonical\"").count() == 1,
        "{about}"
    );
    assert!(by("/pricing").contains("<link rel=\"alternate\" hreflang=\"es\" href=\"https://acme.dev/es/precios\"><link rel=\"alternate\" hreflang=\"x-default\" href=\"https://acme.dev/pricing\">"));
    assert!(
        by("/es/precios").contains("<meta property=\"og:locale\" content=\"es_ES\">")
            && by("/es/precios")
                .contains("<meta property=\"og:locale:alternate\" content=\"en_US\">")
    );
    assert!(by("/es/blog/a").contains("hreflang=\"en\" href=\"https://acme.dev/blog/a\""));
    let fb = by("/es/blog/b");
    assert!(
        fb.contains("<meta name=\"robots\" content=\"noindex, follow\">")
            && !fb.contains("hreflang"),
        "fallback is noindex and unpaired"
    );
    assert!(
        !by("/blog/b").contains("hreflang"),
        "the English page has no Spanish twin to point at"
    );
    let file = |n: &str| {
        built
            .files
            .iter()
            .find(|(p, _)| p == n)
            .map(|(_, c)| c.clone())
            .unwrap_or_else(|| panic!("{n} missing"))
    };
    let sm = file("sitemap.xml");
    assert!(sm.contains("xmlns:xhtml=\"http://www.w3.org/1999/xhtml\""));
    assert!(sm.contains("<loc>https://acme.dev/es/precios</loc><xhtml:link rel=\"alternate\" hreflang=\"en\" href=\"https://acme.dev/pricing\"/><xhtml:link rel=\"alternate\" hreflang=\"es\" href=\"https://acme.dev/es/precios\"/><xhtml:link rel=\"alternate\" hreflang=\"x-default\" href=\"https://acme.dev/pricing\"/>"));
    assert!(!sm.contains("/es/blog/b"));
    let es_llms = file("es/llms.txt");
    assert!(
        es_llms.contains("es/precios")
            && !es_llms.contains("(https://acme.dev/pricing")
            && !es_llms.contains("es/blog/b"),
        "{es_llms}"
    );
    assert!(!file("llms.txt").contains("/es/"));
    assert!(
        file("es/blog/rss.xml").contains("Offline first in the second language")
            && !file("es/blog/rss.xml").contains("Second post")
    );
    assert!(
        built
            .warnings
            .iter()
            .any(|w| w.starts_with("B1829") && w.contains("`b` has no es version")),
        "{:?}",
        built.warnings
    );
    // a cross-locale canonical is an error
    let mut bad = good_site();
    let mut es2 = page(
        "/es/precios",
        "Pricing page in the second language — Acme",
        "Translated pricing with a canonical that wrongly points at the default page.",
        "<h1>Pricing (es)</h1>",
    );
    es2.html = es2
        .html
        .replace(
            "https://acme.dev/es/precios\"",
            "https://acme.dev/pricing\"",
        )
        .replace("<html lang=\"en\">", "<html lang=\"fr\">");
    bad.push(es2);
    let i = brain_seo::hreflang::I18n {
        default: "en".into(),
        locales: vec!["en".into(), "es".into()],
    };
    let r = audit(&Site {
        cfg: &c,
        pages: &bad,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: Some(&i),
    });
    let f: Vec<(&str, Level)> = r.pages[3]
        .findings
        .iter()
        .filter(|f| f.code == "B1828")
        .map(|f| (f.msg.as_str(), f.level))
        .collect();
    assert_eq!(f.len(), 2, "{f:?}");
    assert!(f[0].0.contains("canonical points at `/pricing`") && f[0].1 == Level::Error);
    assert!(f[1].0.contains("lang=\"fr\"") && f[1].1 == Level::Warning);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn untranslated_language_versions_are_not_duplicates() {
    let c = cfg();
    let mut pages = good_site();
    let en = pages[1].clone();
    assert_eq!(en.route, "/pricing");
    let mut es = en.clone();
    es.route = "/es/pricing".into();
    es.file = "es/pricing/index.html".into();
    es.html = es
        .html
        .replace(
            "https://acme.dev/pricing\"",
            "https://acme.dev/es/pricing\"",
        )
        .replace("<html lang=\"en\">", "<html lang=\"es\">");
    pages.push(es);
    let i = brain_seo::hreflang::I18n {
        default: "en".into(),
        locales: vec!["en".into(), "es".into()],
    };
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: Some(&i),
    });
    for route in ["/pricing", "/es/pricing"] {
        let p = r.pages.iter().find(|p| p.route == route).unwrap();
        let codes: Vec<&str> = p.findings.iter().map(|f| f.code).collect();
        assert!(
            !codes
                .iter()
                .any(|c| ["B1813", "B1814", "B1815"].contains(c)),
            "{route}: {codes:?}"
        );
    }
    // without i18n the same two pages are plain duplicates
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let p = r.pages.iter().find(|p| p.route == "/es/pricing").unwrap();
    assert!(p.findings.iter().any(|f| f.code == "B1814"));
}

#[test]
fn content_module_switches_locale() {
    let Ok(v) = std::process::Command::new("node").arg("--version").output() else {
        return;
    };
    if !v.status.success() {
        return;
    }
    let dir = fresh_dir("module-locale");
    for (l, slug) in [("en", "a"), ("en", "b"), ("es", "a")] {
        std::fs::create_dir_all(dir.join(format!("content/blog/{l}"))).unwrap();
        std::fs::write(dir.join(format!("content/blog/{l}/{slug}.md")), format!("---\ntitle: {slug} {l}\ndescription: Description long enough to be a summary of this entry.\ndate: 2026-10-01\n---\nText.\n")).unwrap();
    }
    let i = brain_seo::hreflang::I18n {
        default: "en".into(),
        locales: vec!["en".into(), "es".into()],
    };
    let (cols, _) = brain_seo::content::load_i18n(&dir, false, Some(&i));
    let module = brain_seo::content::module_js(&cols, 10);
    let script = format!(
        "{module}\nconst a = entries('blog').map((e) => e.title).join();\n__setLocale('es');\nconst b = entries('blog').map((e) => `${{e.title}}:${{e.fallback}}:${{e.url}}`).join();\nconsole.log(a + '|' + b)"
    );
    let out = std::process::Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "a en,b en|a es:false:/es/blog/a,b en:true:/es/blog/b"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn geo_hints_are_info_and_keep_the_score() {
    let c = cfg();
    let pages = good_site();
    let base = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    // the blog post: long opening paragraph, question headings, article without date or author links, no sources
    let r2 = &base.pages[2];
    let codes: Vec<&str> = r2
        .findings
        .iter()
        .filter(|f| f.code.starts_with("B183") && f.code != "B1813")
        .map(|f| f.code)
        .collect();
    assert!(
        codes.contains(&"B1830") && codes.contains(&"B1834"),
        "{codes:?}"
    );
    assert!(
        r2.findings
            .iter()
            .filter(|f| ["B1830", "B1831", "B1832", "B1833", "B1834"].contains(&f.code))
            .all(|f| f.level == Level::Info)
    );
    assert_eq!(r2.score, Some(100), "GEO hints never lower the score");
    let mut pages = good_site();
    pages[2].html = pages[2].html.replace("</head>", "<script type=\"application/ld+json\">{\"@context\":\"https://schema.org\",\"@type\":\"BlogPosting\",\"headline\":\"H\",\"author\":{\"@type\":\"Person\",\"name\":\"Ada\"}}</script></head>")
        .replace("</main>", "<h2>Is it free?</h2><p>Yes.</p><h2>Does it sync?</h2><p>Yes.</p></main>");
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let msgs: Vec<String> = r.pages[2]
        .findings
        .iter()
        .filter(|f| ["B1831", "B1832", "B1833"].contains(&f.code))
        .map(|f| format!("{} {}", f.code, f.msg))
        .collect();
    assert!(
        msgs.iter()
            .any(|m| m.starts_with("B1831 2 question headings")),
        "{msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m == "B1832 the article shows no date")
            && msgs
                .iter()
                .any(|m| m == "B1832 the article JSON-LD has no dateModified"),
        "{msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m == "B1833 Ada has no profile links"),
        "{msgs:?}"
    );
    // with a date, a profile and a source the hints go away
    let j = brain_lex::Json::parse(r#"{"authors":{"Ada":{"url":"https://ada.dev","sameAs":["https://www.linkedin.com/in/ada"]}}}"#).unwrap();
    let with_authors = Config {
        authors: Config::from_json(&j).unwrap().authors,
        ..cfg()
    };
    let a = brain_seo::jsonld::Article {
        headline: "H".into(),
        published: Some("2026-10-01".into()),
        author: Some("Ada".into()),
        ..Default::default()
    };
    let ld = brain_seo::jsonld::article(&with_authors, "/blog/x", &a).stringify();
    assert!(ld.contains("\"author\":{\"@type\":\"Person\",\"name\":\"Ada\",\"url\":\"https://ada.dev\",\"sameAs\":[\"https://www.linkedin.com/in/ada\"]}"), "{ld}");
}

#[test]
fn finish_videos_and_news() {
    let dir = fresh_dir("video");
    std::fs::create_dir_all(dir.join("content/news")).unwrap();
    std::fs::write(dir.join("content/news/launch.md"), "---\ntitle: Acme launches offline sync\ndescription: Today Acme ships offline sync for every team plan, with conflict-free merges.\ndate: 2026-10-12\n---\nText.\n").unwrap();
    let j = brain_lex::Json::parse(r#"{"site":"https://acme.dev","name":"Acme","lang":"en","seo":{"news":{"name":"Acme News","collections":["news"]}}}"#).unwrap();
    let c = Config::from_json(&j).unwrap();
    let mut pages = good_site();
    pages[1].html = pages[1].html.replace("</main>", "<video poster=\"/thumbs/demo.jpg\" title=\"Product demo\" data-duration=\"PT1M30S\" data-upload-date=\"2026-10-01\" controls><source src=\"/demo.mp4\" type=\"video/mp4\"></video><video src=\"/no-poster.mp4\"></video></main>");
    pages.push(page(
        "/news/launch",
        "Acme launches offline sync — Acme News",
        "Today Acme ships offline sync for every team plan, with conflict-free merges.",
        "<h1>Acme launches offline sync</h1><p><a href=\"/\">home</a></p>",
    ));
    let built = finish(&c, &dir, &mut pages, &[], &[]).unwrap();
    let html = &pages[1].html;
    assert!(
        html.contains("\"@type\":\"VideoObject\",\"name\":\"Product demo\"")
            && html.contains("\"contentUrl\":\"https://acme.dev/demo.mp4\"")
            && html.contains("\"duration\":\"PT1M30S\"")
    );
    let file = |n: &str| {
        built
            .files
            .iter()
            .find(|(p, _)| p == n)
            .map(|(_, c)| c.clone())
            .unwrap_or_else(|| panic!("{n} missing"))
    };
    let sm = file("sitemap.xml");
    assert!(sm.contains("xmlns:video=") && sm.contains("<video:thumbnail_loc>https://acme.dev/thumbs/demo.jpg</video:thumbnail_loc><video:title>Product demo</video:title>") && sm.contains("<video:duration>90</video:duration>"));
    assert!(!sm.contains("no-poster"), "videos without poster stay out");
    let news = file("sitemap-news.xml");
    assert!(news.contains("<loc>https://acme.dev/news/launch</loc><news:news><news:publication><news:name>Acme News</news:name><news:language>en</news:language></news:publication><news:publication_date>2026-10-12</news:publication_date><news:title>Acme launches offline sync</news:title>"), "{news}");
    assert!(file("robots.txt").contains("Sitemap: https://acme.dev/sitemap-news.xml"));
    let p = built
        .report
        .pages
        .iter()
        .find(|p| p.route == "/pricing")
        .unwrap();
    assert!(
        p.findings.iter().any(|f| f.code == "B1835"
            && f.msg.contains("/no-poster.mp4")
            && f.level == Level::Info)
    );
    assert!(
        Config::from_json(
            &brain_lex::Json::parse(r#"{"seo":{"news":{"collections":["news"]}}}"#).unwrap()
        )
        .unwrap_err()
        .contains("needs a `name`")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_uses_the_origin_of_the_build() {
    let dir = fresh_dir("origin");
    for p in good_site() {
        let f = dir.join("dist").join(&p.file);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, p.html).unwrap();
    }
    // brain.json names another origin (legacy "site"); the build used https://acme.dev
    std::fs::write(dir.join("brain.json"), r#"{"site":"https://old.example"}"#).unwrap();
    let (code, out) = brain_seo::cli(&dir, &["audit".into()]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("/pricing — 100/100") && !out.contains("noindex"),
        "{out}"
    );
    // a canonical on another host is reported, not silently skipped
    let c = cfg();
    let mut pages = good_site();
    pages[1].html = pages[1].html.replace(
        "https://acme.dev/pricing\"",
        "https://example.com/pricing\"",
    );
    let r = audit(&Site {
        cfg: &c,
        pages: &pages,
        routes: &[],
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    });
    let f = r.pages[1]
        .findings
        .iter()
        .find(|f| f.code == "B1809")
        .expect("B1809");
    assert!(f.msg.contains("outside https://acme.dev") && f.level == Level::Warning);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn content_without_pages_publishes_no_feed() {
    let dir = fresh_dir("nopages");
    std::fs::create_dir_all(dir.join("content/guides")).unwrap();
    std::fs::write(dir.join("content/guides/first.md"), "---\ntitle: First guide\ndescription: A guide whose template view does not exist yet, so no page is built for it.\ndate: 2026-10-13\n---\nText.\n").unwrap();
    let c = Config {
        news: Some(brain_seo::config::News {
            name: "Acme".into(),
            collections: vec!["guides".into()],
        }),
        ..cfg()
    };
    let built = finish(&c, &dir, &mut good_site(), &[], &[]).unwrap();
    assert!(
        !built.files.iter().any(|(p, _)| p.starts_with("guides/")),
        "no feed for entries without pages"
    );
    let news = &built
        .files
        .iter()
        .find(|(p, _)| p == "sitemap-news.xml")
        .unwrap()
        .1;
    assert!(!news.contains("/guides/first"), "{news}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn not_found_page_canonical_must_be_a_url() {
    let dir = fresh_dir("notfound");
    for p in good_site() {
        let f = dir.join("dist").join(&p.file);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, p.html).unwrap();
    }
    std::fs::write(dir.join("dist/404.html"), "<!doctype html><html lang=\"en\"><head><title>Page not found · Acme</title><meta name=\"robots\" content=\"noindex\"><link rel=\"canonical\" href=\"*\"></head><body><main><h1>Not found</h1><p><a href=\"/\">Home</a></p></main></body></html>").unwrap();
    let (_, out) = brain_seo::cli(&dir, &["audit".into()]);
    assert!(
        out.contains("* — noindex") && out.contains("warning B1809: canonical `*` is not a URL"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn og_and_redirects_commands() {
    let dir = fresh_dir("cmds");
    let mut pages = good_site();
    pages[1].html = pages[1].html.replace(
        "<meta property=\"og:image\" content=\"https://acme.dev/og.png\">",
        "",
    );
    for p in &pages {
        let f = dir.join("dist").join(&p.file);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, &p.html).unwrap();
    }
    let (code, out) = brain_seo::cli(&dir, &["og".into()]);
    assert!(
        out.contains("template: built-in")
            && out.contains("/pricing — generated at the next `brain build`: /og/pricing.png"),
        "{out}"
    );
    assert!(
        out.contains("/ — https://acme.dev/og.png (not found in dist/ or public/)"),
        "{out}"
    );
    assert_eq!(code, 0, "generated images count as covered");
    std::fs::write(
        dir.join("redirects.json"),
        r#"[{"from":"/old-pricing","to":"/pricing"},{"from":"/gone","to":"/nowhere"}]"#,
    )
    .unwrap();
    let (code, out) = brain_seo::cli(&dir, &["redirects".into()]);
    assert_eq!(code, 1);
    assert!(
        out.contains("error B1825: `/gone` redirects to `/nowhere`"),
        "{out}"
    );
    std::fs::write(
        dir.join("redirects.json"),
        r#"[{"from":"/old-pricing","to":"/pricing"}]"#,
    )
    .unwrap();
    let j = brain_seo::tool("brain_seo_redirects", &brain_lex::Json::obj([]), &dir).unwrap();
    assert_eq!(
        j.get("nginx").and_then(brain_lex::Json::as_str),
        Some("location = /old-pricing { return 301 /pricing; }\n")
    );
    assert!(
        brain_seo::tool("brain_seo_og", &brain_lex::Json::obj([]), &dir)
            .unwrap()
            .get("pages")
            .is_some()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dist_audit_accepts_server_routes() {
    let dir = fresh_dir("ssr");
    let mut pages = good_site();
    pages[0].html = pages[0].html.replace(
        "</main>",
        "<a href=\"/account\">Account</a><a href=\"/users/7\">User</a></main>",
    );
    for p in &pages {
        let f = dir.join("dist").join(&p.file);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, &p.html).unwrap();
    }
    let (code, out) = brain_seo::cli(&dir, &["audit".into()]);
    assert_eq!(
        code, 1,
        "without the server manifest the SSR links look broken: {out}"
    );
    std::fs::create_dir_all(dir.join("dist/_server")).unwrap();
    std::fs::write(dir.join("dist/_server/manifest.json"), r#"{"v":1,"routes":[{"path":"/account","rx":"^/account/?$"},{"path":"/users/:id","rx":"^/users/([^/]+)/?$"}],"notFound":"_server/pages/x.html"}"#).unwrap();
    let (code, out) = brain_seo::cli(&dir, &["audit".into()]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("B1819"), "{out}");
    assert_eq!(
        brain_seo::server_routes(&dir.join("dist")),
        vec!["/account".to_string(), "/users/:id".into()]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ssr_templates_are_audited_by_their_head_only() {
    let dir = fresh_dir("ssrhead");
    for p in good_site() {
        let f = dir.join("dist").join(&p.file);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, &p.html).unwrap();
    }
    std::fs::create_dir_all(dir.join("dist/_server/pages")).unwrap();
    std::fs::write(dir.join("dist/_server/pages/login.html"), "<!doctype html><html lang=\"en\"><head><title>Sign in to the offline task planner · Acme</title><meta name=\"description\" content=\"Sign in to your Acme account to plan the week and sync your team's tasks offline.\"><link rel=\"canonical\" href=\"https://acme.dev/login\"><meta property=\"og:title\" content=\"Sign in\"><meta property=\"og:image\" content=\"https://acme.dev/og.png\"></head><body><div id=\"brain-view\"></div></body></html>").unwrap();
    std::fs::write(dir.join("dist/_server/manifest.json"), r#"{"routes":[{"path":"/login","page":"_server/pages/login.html"},{"path":"/users/:id","page":"_server/pages/u.html"}]}"#).unwrap();
    let (_, out) = brain_seo::cli(&dir, &["audit".into()]);
    let login = out
        .split("\n\n")
        .find(|b| b.starts_with("/login"))
        .unwrap_or_else(|| panic!("{out}"));
    assert!(
        !login.contains("B1805") && !login.contains("B1808"),
        "body checks skipped for SSR templates: {login}"
    );
    assert!(
        login.contains("B1813"),
        "orphan check still applies: {login}"
    );
    assert!(
        !out.contains("/users/:id —"),
        "parameterized SSR routes are not audited as pages"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn base_path_prefixes_absolute_urls_only() {
    let dir = fresh_dir("base");
    std::fs::create_dir_all(dir.join("content/blog")).unwrap();
    std::fs::write(dir.join("content/blog/a.md"), "---\ntitle: A post\ndescription: A post that lives under the base path of the site, used to check feed URLs.\ndate: 2026-10-01\n---\nSee [pricing](/pricing).\n").unwrap();
    let j = brain_lex::Json::parse(r#"{"base":"app","name":"Acme"}"#).unwrap();
    let mut c = Config::from_json(&j).unwrap();
    assert_eq!(c.base, "/app/");
    c.site = Some("https://acme.dev".into());
    assert_eq!(
        (
            c.abs("/pricing"),
            c.strip_base("/app/pricing"),
            c.strip_base("/app"),
            c.strip_base("/application")
        ),
        (
            "https://acme.dev/app/pricing".into(),
            "/pricing",
            "/",
            "/application"
        )
    );
    let mut pages: Vec<Page> = good_site()
        .into_iter()
        .map(|mut p| {
            p.html = p
                .html
                .replace("https://acme.dev/", "https://acme.dev/app/")
                .replace("https://acme.dev/app/og.png", "https://acme.dev/og.png");
            p
        })
        .collect();
    pages.push(page(
        "/blog/a",
        "A post under the base path — Acme blog",
        "A post that lives under the base path of the site, used to check feed URLs.",
        "<h1>A post</h1><p><a href=\"/app/pricing\">pricing</a></p>",
    ));
    pages.last_mut().unwrap().html = pages
        .last()
        .unwrap()
        .html
        .replace("https://acme.dev/blog/a\"", "https://acme.dev/app/blog/a\"");
    let built = finish(&c, &dir, &mut pages, &[], &[]).unwrap();
    let file = |n: &str| {
        built
            .files
            .iter()
            .find(|(p, _)| p == n)
            .map(|(_, c)| c.clone())
            .unwrap_or_else(|| panic!("{n} missing"))
    };
    assert!(file("sitemap.xml").contains("<loc>https://acme.dev/app/pricing</loc>"));
    assert!(file("robots.txt").contains("Sitemap: https://acme.dev/app/sitemap.xml"));
    assert!(file("llms.txt").contains("(https://acme.dev/app/pricing.md)"));
    assert!(file("blog/rss.xml").contains("<link>https://acme.dev/app/blog/a</link>"));
    let m = file("manifest.webmanifest");
    assert!(
        m.contains("\"start_url\":\"/app/\"") && m.contains("\"scope\":\"/app/\""),
        "{m}"
    );
    assert!(
        pages[0].html.contains("\"url\":\"https://acme.dev/app/\""),
        "WebSite JSON-LD under the base"
    );
    assert!(
        built
            .report
            .pages
            .iter()
            .all(|p| p.indexable || p.route == "/blog/a"),
        "canonicals with the base still match their routes"
    );
    assert!(
        built.report.errors().next().is_none(),
        "links written with the base resolve: {:?}",
        built
            .report
            .errors()
            .map(|(_, f)| f.msg.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        built.warnings.iter().any(|w| w.contains("robots.txt")),
        "{:?}",
        built.warnings
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn blog_posting_image_falls_back_to_the_social_card() {
    let dir = fresh_dir("ld-og");
    let mut pages = good_site();
    pages[2].article = Some(brain_seo::jsonld::Article {
        headline: "Offline task planner".into(),
        published: Some("2026-10-01".into()),
        author: Some("Ada".into()),
        ..Default::default()
    });
    let built = finish(&cfg(), &dir, &mut pages, &[], &[]).unwrap();
    let post = brain_seo::html::parse(&pages[2].html).jsonld.join("\n");
    assert!(
        post.contains("\"image\":\"https://acme.dev/og.png\""),
        "{post}"
    );
    assert!(
        built.report.pages[2]
            .findings
            .iter()
            .all(|f| !f.msg.contains("has no `image`")),
        "{:?}",
        built.report.pages[2].findings
    );
    let _ = std::fs::remove_dir_all(&dir);
}
