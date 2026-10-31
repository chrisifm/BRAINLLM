//! BRAINLLM SEO module (INTERNALS §32-§33): pure functions over the prerendered pages. `brain build`, `brain doctor`,
//! `brain seo …` and the MCP tools only call into this crate.
//!
//! - [`finish`]: one call from `brain build` after the pages are assembled and before precompression. Rewrites each
//!   page's `<head>` (Google tags, manifest link, staging `noindex`, duplicate tags removed), returns the generated
//!   files (robots.txt, sitemaps, llms.txt, llms-full.txt, manifest.webmanifest) and fails on broken internal links.
//! - [`audit`]: score and B18xx findings per page ([`audit::CATALOG`]).
//! - [`cli`]: `brain seo audit|links|preview <route> [--json]` over an existing `dist/`; [`TOOLS`] + [`tool`]: the
//!   same for MCP.
//! - [`CATALOG`]: `brain explain B18xx`.

pub mod audit;
pub mod config;
pub mod content;
pub mod files;
pub mod hreflang;
pub mod html;
pub mod indexnow;
pub mod jsonld;
pub mod markdown;
pub mod og;
pub mod preview;
pub mod redirects;
pub mod sections;
pub mod social;
pub mod video;

pub use audit::{CATALOG, Finding, Level, Report};
pub use config::Config;

use brain_lex::Json;
use std::path::Path;

/// One prerendered page (what `brain_pack::Output::pages` holds, plus SEO-only facts from the view source).
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub route: String,
    /// `dist/` relative path (`index.html`, `blog/a/index.html`).
    pub file: String,
    pub html: String,
    /// `YYYY-MM-DD` for the sitemap (content date or `<meta seo lastmod>`); never a file mtime (invariant 8).
    pub lastmod: Option<String>,
    /// `<meta seo keyword>` (not rendered into the HTML).
    pub keyword: Option<String>,
    /// Set for content-collection entries: emits `BlogPosting` JSON-LD.
    pub article: Option<jsonld::Article>,
    /// View file (`views/about.brain`): pairs language variants with translated slugs.
    pub view: Option<String>,
    /// Read from a server template (`dist/_server/pages/…`): only the `<head>` is audited.
    pub ssr: bool,
}

/// `(path without leading slash, size in bytes)` of every non-page file served from `dist/`.
pub type Assets = Vec<(String, u64)>;

/// Everything the audit sees. Built by the caller; nothing here touches the disk.
pub struct Site<'a> {
    pub cfg: &'a Config,
    pub pages: &'a [Page],
    /// Every route pattern from `views/`, including SSR-only ones (`/users/:id`): links matching one are valid.
    pub routes: &'a [String],
    /// Every other file served from `dist/` (path without leading slash) and its size in bytes.
    pub assets: &'a [(String, u64)],
    /// `redirects.json` rules: a link to a redirected source is not broken, but should point at the target.
    pub redirects: &'a [redirects::Rule],
    /// `(path without leading slash, width, height)` of local images (social card checks).
    pub images: &'a [(String, u32, u32)],
    /// Locales of a multilingual site (B1828 checks).
    pub i18n: Option<&'a hreflang::I18n>,
}

impl Site<'_> {
    /// Dimensions of the local image at site path `path`.
    pub fn image_dims(&self, path: &str) -> Option<(u32, u32)> {
        let p = path.trim_start_matches('/');
        self.images
            .iter()
            .find(|(a, _, _)| a == p)
            .map(|(_, w, h)| (*w, *h))
    }
}

/// Dimensions of every PNG/JPEG/GIF/WebP under `dir` (paths relative to it).
pub fn image_dims_in(dir: &Path) -> Vec<(String, u32, u32)> {
    let mut all = Vec::new();
    walk(dir, "", &mut all);
    all.into_iter()
        .filter(|(p, _)| {
            [".png", ".jpg", ".jpeg", ".gif", ".webp"]
                .iter()
                .any(|x| p.to_ascii_lowercase().ends_with(x))
        })
        .filter_map(|(p, _)| {
            let b = std::fs::read(dir.join(&p)).ok()?;
            social::image_size(&b).map(|(w, h)| (p, w, h))
        })
        .collect()
}

impl Site<'_> {
    /// Size of the served file at site path `path` (`/favicon.svg` or `favicon.svg`).
    pub fn asset_size(&self, path: &str) -> Option<u64> {
        let p = path.trim_start_matches('/');
        self.assets.iter().find(|(a, _)| a == p).map(|(_, n)| *n)
    }
}

pub fn audit(site: &Site) -> Report {
    audit::audit(site)
}

/// Result of [`finish`].
pub struct Built {
    /// `(dist path, content)` of every generated file.
    pub files: Vec<(String, String)>,
    pub warnings: Vec<String>,
    /// `(CSP directive, source)` the server config must allow (Google tags).
    pub csp: Vec<(&'static str, &'static str)>,
    /// `redirects.json` rules, validated: feed them to [`redirects::nginx`], [`redirects::caddy`] and
    /// [`redirects::server_json`] for the server configs (`_redirects` is already in `files`).
    pub redirects: Vec<redirects::Rule>,
    /// `seo.trailingSlash` / `seo.canonicalHost`: feed with the rules to [`redirects::server_json_with`].
    pub policy: redirects::Policy,
    /// `(dist path, bytes)` of generated binary files (social images).
    pub binary: Vec<(String, Vec<u8>)>,
    pub report: Report,
}

/// Byte index of the first case-insensitive match.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.len() > h.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Inserts `tags` at the start of `<head>` (after `<meta charset>` when it comes first) or right before `</head>`.
pub fn inject_head(html: &str, tags: &str, at_end: bool) -> String {
    if tags.is_empty() {
        return html.to_string();
    }
    let at = if at_end {
        find_ci(html, "</head>")
    } else {
        find_ci(html, "<head")
            .and_then(|h| html[h..].find('>').map(|e| h + e + 1))
            .map(|after_head| {
                let rest = &html[after_head..];
                let trimmed = rest.trim_start();
                if trimmed.len() >= 13 && trimmed[..13].eq_ignore_ascii_case("<meta charset") {
                    let off = after_head + (rest.len() - trimmed.len());
                    trimmed.find('>').map_or(after_head, |e| off + e + 1)
                } else {
                    after_head
                }
            })
    };
    match at {
        Some(i) => format!("{}{tags}{}", &html[..i], &html[i..]),
        None => format!("{tags}{html}"),
    }
}

/// Meta properties that may legitimately repeat.
const MULTI: &[&str] = &[
    "article:tag",
    "og:locale:alternate",
    "video:tag",
    "book:tag",
];

/// Removes earlier duplicates of head tags so the most specific one wins (the shell's defaults come first, the
/// view's tags later): `<title>`, meta by `name`/`property`/`http-equiv`/`charset`, `<link rel=canonical|manifest>`
/// and `<link rel=alternate hreflang>`.
pub fn dedupe_head(html: &str) -> String {
    let Some(h0) = find_ci(html, "<head") else {
        return html.to_string();
    };
    let h1 = find_ci(&html[h0..], "</head>").map_or(html.len(), |e| h0 + e);
    let head = &html[h0..h1];
    let mut tags: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0;
    while let Some(lt) = head[i..].find('<').map(|x| i + x) {
        let rest = &head[lt + 1..];
        let name_len = rest.bytes().take_while(|b| b.is_ascii_alphabetic()).count();
        let name = rest[..name_len].to_ascii_lowercase();
        let Some(gt) = head[lt..].find('>').map(|x| lt + x + 1) else {
            break;
        };
        let attrs = html::parse_attrs(&head[lt + 1 + name_len..gt - 1]);
        let lower = |k: &str| html::attr(&attrs, k).map(str::to_ascii_lowercase);
        let (end, key) = match name.as_str() {
            "title" => (
                find_ci(&head[gt..], "</title>").map_or(gt, |e| gt + e + 8),
                Some("title".to_string()),
            ),
            "meta" => (
                gt,
                if attrs.iter().any(|(k, _)| k == "charset") {
                    Some("charset".into())
                } else {
                    lower("name")
                        .map(|n| format!("name:{n}"))
                        .or_else(|| {
                            lower("property")
                                .filter(|p| !MULTI.contains(&p.as_str()))
                                .map(|p| format!("property:{p}"))
                        })
                        .or_else(|| lower("http-equiv").map(|n| format!("he:{n}")))
                },
            ),
            "link" => (
                gt,
                match lower("rel").as_deref() {
                    Some("canonical") => Some("canonical".into()),
                    Some("manifest") => Some("manifest".into()),
                    Some("alternate") => lower("hreflang").map(|l| format!("alternate:{l}")),
                    _ => None,
                },
            ),
            "script" | "style" => (
                find_ci(&head[gt..], &format!("</{name}>")).map_or(gt, |e| gt + e + name.len() + 3),
                None,
            ),
            _ => (gt, None),
        };
        if let Some(k) = key {
            tags.push((lt, end, k));
        }
        i = end.max(lt + 1);
    }
    let mut drop: Vec<(usize, usize)> = Vec::new();
    for (n, (a, b, k)) in tags.iter().enumerate() {
        if tags[n + 1..].iter().any(|(_, _, k2)| k2 == k) {
            drop.push((*a, *b));
        }
    }
    if drop.is_empty() {
        return html.to_string();
    }
    let mut out = html[..h0].to_string();
    let mut last = 0;
    for (a, b) in drop {
        out.push_str(&head[last..a]);
        last = b;
    }
    out.push_str(&head[last..]);
    out.push_str(&html[h1..]);
    out
}

/// Icons for the manifest: PNG/SVG in `public/` or `public/icons/` named like an app icon.
fn public_icons(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for dir in ["", "icons/"] {
        let Ok(rd) = std::fs::read_dir(root.join("public").join(dir)) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            let iconish = !dir.is_empty()
                || ["icon", "android-chrome", "apple-touch-icon", "maskable"]
                    .iter()
                    .any(|p| lower.starts_with(p))
                || lower == "favicon.svg";
            if iconish
                && (lower.ends_with(".png") || lower.ends_with(".svg"))
                && let Ok(b) = std::fs::read(e.path())
            {
                out.push((format!("{dir}{name}"), b));
            }
        }
    }
    out.sort();
    out
}

/// Every file under `public/` with its size (served as-is from the site root).
pub fn public_assets(root: &Path) -> Assets {
    let mut out = Vec::new();
    walk(&root.join("public"), "", &mut out);
    out
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, u64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = format!("{prefix}{}", e.file_name().to_string_lossy());
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&e.path(), &format!("{name}/"), out),
            Ok(_) => out.push((name, e.metadata().map_or(0, |m| m.len()))),
            Err(_) => {}
        }
    }
}

/// The site feed's discovery tag, on every page while `rss.xml` exists (§32).
const FEED_LINK: &str = "<link rel=\"alternate\" type=\"application/rss+xml\" title=\"Site feed\" href=\"/rss.xml\">";

/// The `brain build` step (INTERNALS §32): rewrites every page's `<head>` in place and returns the generated files.
/// `assets` lists the other files of `dist/` (chunks, CSS); `public/` is added here. Fails on broken internal links
/// or anchors (B1819/B1820), listing each one with its fix.
pub fn finish(
    cfg: &Config,
    root: &Path,
    pages: &mut [Page],
    routes: &[String],
    assets: &[(String, u64)],
) -> Result<Built, String> {
    finish_with_og(cfg, root, pages, routes, assets, None)
}

/// [`finish`] plus social images rendered by `shoot` (`views/_og.brain` → 1200×630 PNG over CDP, from the caller).
pub fn finish_with_og(
    cfg: &Config,
    root: &Path,
    pages: &mut [Page],
    routes: &[String],
    assets: &[(String, u64)],
    shoot: Option<og::Shoot>,
) -> Result<Built, String> {
    let mut warnings = Vec::new();
    let mut files: Vec<(String, String)> = Vec::new();
    let google = files::google_tags(cfg);
    let tokens = std::fs::read_to_string(root.join("tokens.cssbrain")).unwrap_or_default();
    let home = pages
        .iter()
        .find(|p| p.route == "/")
        .map(|p| html::parse(&p.html));
    let has_name = cfg.name.is_some() || home.as_ref().is_some_and(|d| d.title.is_some());
    let manifest = cfg.manifest && has_name && !pages.is_empty();
    let mut start = google.head.clone();
    if manifest {
        start.push_str("<link rel=\"manifest\" href=\"/manifest.webmanifest\">");
        if let Some(c) = cfg
            .manifest_cfg
            .theme_color
            .clone()
            .or_else(|| files::token(&tokens, "accent"))
        {
            start.push_str(&format!(
                "<meta name=\"theme-color\" content=\"{}\">",
                c.replace(['"', '<', '>'], "")
            ));
        }
    }
    let end = if cfg.staging {
        "<meta name=\"robots\" content=\"noindex, nofollow\">"
    } else {
        ""
    };
    // locales, then content collections (one per locale): drafts only in staging; dates and BlogPosting for entry
    // pages, rel prev/next on listings, untranslated entries noindex
    let (i18n, lang_warn) = hreflang::I18n::load(cfg, root);
    warnings.extend(lang_warn);
    let (cols, probs) = content::load_i18n(root, cfg.staging, i18n.as_ref());
    let fatal: Vec<String> = probs
        .iter()
        .filter(|p| p.error)
        .map(|p| {
            format!(
                "{} {}:{}: {}; fix: {}",
                p.code, p.file, p.line, p.msg, p.fix
            )
        })
        .collect();
    if !fatal.is_empty() {
        return Err(format!(
            "{} content error(s):\n{}",
            fatal.len(),
            fatal.join("\n")
        ));
    }
    warnings.extend(probs.iter().map(|p| {
        format!(
            "{} {}:{}: {}; fix: {}",
            p.code, p.file, p.line, p.msg, p.fix
        )
    }));
    // feeds, news and RSS links only list entries that have a built page (content without a template yet → nothing)
    let built: std::collections::HashSet<String> =
        pages.iter().map(|p| audit::norm(&p.route)).collect();
    let published: Vec<content::Collection> = cols
        .iter()
        .map(|c| content::Collection {
            entries: c
                .entries
                .iter()
                .filter(|e| built.contains(&e.route()))
                .cloned()
                .collect(),
            ..c.clone()
        })
        .collect();
    let by_route = content::index(&cols);
    let fallback: std::collections::HashSet<String> = by_route
        .iter()
        .filter(|(_, e)| e.fallback)
        .map(|(r, _)| r.clone())
        .collect();
    let alternates: std::collections::BTreeMap<String, Vec<(String, String)>> = match &i18n {
        Some(i) => hreflang::alternates(
            i,
            &pages
                .iter()
                .filter(|p| !fallback.contains(&audit::norm(&p.route)))
                .map(|p| (p.route.as_str(), p.view.as_deref()))
                .collect::<Vec<_>>(),
        ),
        None => Default::default(),
    };
    for p in pages.iter_mut() {
        let route = audit::norm(&p.route);
        let mut tags = match (&i18n, alternates.get(&p.route)) {
            (Some(i), Some(v)) => hreflang::head_tags(cfg, i, v),
            _ => String::new(),
        };
        // WordPress-style discovery on every page: the site feed and the sections manifest (§32)
        // a site URL is needed for a feed; a build with no indexable page writes none and drops the link below
        if cfg.feed && cfg.site.is_some() {
            tags.push_str(FEED_LINK);
        }
        if cfg.sections {
            tags.push_str("<link rel=\"alternate\" type=\"application/json\" title=\"Sections\" href=\"/sections.json\">");
        }
        // a view's literal `canonical="/x"` is rendered once per locale: on `/es/x` it means the page itself, not the
        // default-locale version (which B1828 reports as an error); an untranslated content fallback keeps its own
        if let Some(i) = &i18n
            && i.locale_of(&route) != i.default
            && !fallback.contains(&route)
            && html::parse(&p.html)
                .link("canonical")
                .and_then(|c| audit::internal(cfg, &p.route, c))
                .is_some_and(|(t, _)| t == i.strip(&route))
        {
            tags.push_str(&format!(
                "<link rel=\"canonical\" href=\"{}\">",
                cfg.abs(&files::encode_path(&route))
            ));
        }
        if let Some(e) = by_route.get(&route) {
            if p.lastmod.is_none() {
                p.lastmod = e.updated.clone().or_else(|| e.date.clone());
            }
            if p.article.is_none() {
                p.article = Some(jsonld::Article {
                    headline: e.title.clone(),
                    description: Some(e.summary()),
                    published: e.date.clone(),
                    modified: e.updated.clone(),
                    image: e.cover.clone(),
                    author: e.author.clone(),
                    tags: e.tags.clone(),
                });
            }
            if e.fallback {
                tags.push_str("<meta name=\"robots\" content=\"noindex, follow\">");
            }
        }
        for c in &published {
            let base = c.base();
            let listing = route == format!("/{base}")
                || route
                    .strip_prefix(&format!("/{base}/page/"))
                    .is_some_and(|n| n.parse::<usize>().is_ok());
            if listing {
                let n = route
                    .rsplit('/')
                    .next()
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(1);
                let (prev, next) = content::page_links(&base, n, c.entries.len(), cfg.per_page);
                for (rel, r) in [("prev", prev), ("next", next)] {
                    if let Some(r) = r {
                        tags.push_str(&format!("<link rel=\"{rel}\" href=\"{}\">", cfg.abs(&r)));
                    }
                }
            }
            if (route == format!("/{base}") || route.starts_with(&format!("/{base}/")))
                && c.entries.iter().any(|e| e.date.is_some() && !e.fallback)
            {
                tags.push_str(&format!("<link rel=\"alternate\" type=\"application/rss+xml\" title=\"{}\" href=\"/{base}/rss.xml\">", files::xml(&c.name)));
            }
        }
        p.html = dedupe_head(&inject_head(
            &inject_head(&p.html, &start, false),
            &format!("{tags}{end}"),
            true,
        ));
    }
    // social images: rendered for the pages that need one, before the tags that describe them
    let mut images = image_dims_in(&root.join("public"));
    let mut binary: Vec<(String, Vec<u8>)> = Vec::new();
    if let Some(shoot) = shoot.filter(|_| cfg.og != og::Mode::Off) {
        let docs: Vec<html::Doc> = pages.iter().map(|p| html::parse(&p.html)).collect();
        let entries: Vec<files::Entry> = pages
            .iter()
            .zip(&docs)
            .map(|(p, d)| files::Entry {
                route: &p.route,
                url: String::new(),
                doc: d,
                lastmod: None,
                indexable: true,
                alternates: vec![],
                videos: vec![],
            })
            .collect();
        let site = files::site_name(cfg, &entries);
        let list: Vec<(&str, &html::Doc, bool)> = pages
            .iter()
            .zip(&docs)
            .map(|(p, d)| (p.route.as_str(), d, audit::indexable(cfg, &p.route, d)))
            .collect();
        let tpl = og::template(root);
        let jobs = og::plan(cfg, cfg.og, &list, &site, &tpl);
        binary = og::render(root, &jobs, &tpl, shoot)?;
        for j in &jobs {
            if let Some(p) = pages.iter_mut().find(|p| p.route == j.route) {
                p.html = dedupe_head(&inject_head(&p.html, &og::head_tags(cfg, j), true));
            }
            images.push((j.path.clone(), og::WIDTH, og::HEIGHT));
        }
    }
    // structured data: breadcrumbs need every page's name, so parse all pages first
    let first: Vec<html::Doc> = pages.iter().map(|p| html::parse(&p.html)).collect();
    let names: std::collections::BTreeMap<String, String> = pages
        .iter()
        .zip(&first)
        .filter_map(|(p, d)| {
            let n = d
                .headings
                .iter()
                .find(|(l, _)| *l == 1)
                .map(|(_, t)| t.clone())
                .or_else(|| d.title.clone())?;
            Some((audit::norm(&p.route), n))
        })
        .collect();
    let name = {
        let e: Vec<files::Entry> = pages
            .iter()
            .zip(&first)
            .map(|(p, d)| files::Entry {
                route: &p.route,
                url: String::new(),
                doc: d,
                lastmod: None,
                indexable: true,
                alternates: vec![],
                videos: vec![],
            })
            .collect();
        files::site_name(cfg, &e)
    };
    let mut names = names;
    names.insert("/".into(), name.clone()); // the first breadcrumb is the site, not the home page's H1
    for (p, d) in pages.iter_mut().zip(&first) {
        let route = audit::norm(&p.route);
        let indexable = audit::indexable(cfg, &p.route, d);
        let dims = d
            .meta("og:image")
            .and_then(|src| audit::internal(cfg, &p.route, src))
            .and_then(|(path, _)| {
                images
                    .iter()
                    .find(|(a, _, _)| format!("/{a}") == path)
                    .map(|(_, w, h)| (*w, *h))
            });
        let section = by_route.get(&route).map(|e| e.collection.clone());
        let locale = i18n.as_ref().map(|i| i.locale_of(&p.route).to_string());
        let others: Vec<String> = alternates.get(&p.route).map_or(vec![], |v| {
            v.iter()
                .map(|(l, _)| l.clone())
                .filter(|l| Some(l) != locale.as_ref())
                .collect()
        });
        let facts = social::Facts {
            site_name: &name,
            home: route == "/",
            indexable,
            article: p.article.as_ref(),
            section: section.as_deref(),
            image_dims: dims,
            locale: locale.as_deref(),
            locale_alternates: &others,
        };
        let (social_start, social_end) = social::head(cfg, d, &facts);
        let page_date = p
            .article
            .as_ref()
            .and_then(|a| a.published.clone())
            .or_else(|| p.lastmod.clone());
        let (vids, _) = video::collect(cfg, root, &p.route, d, page_date.as_deref());
        // an entry without a cover still has a social card (generated or the site default): it is the post's image
        let article = p.article.clone().map(|mut a| {
            if a.image.is_none() {
                a.image = d.meta("og:image").map(str::to_string);
            }
            a
        });
        let mut end_tags: String =
            jsonld::auto(cfg, &route, d, &names, &name, article.as_ref(), &vids, p.lastmod.as_deref())
                .iter()
                .map(jsonld::script)
                .collect();
        // `<meta name="schema" content="T">` left in the page: JSON-LD instead of a meta tag (SPEC «SEO»)
        if let Some(node) = jsonld::schema_shortcut(cfg, &route, d, p.lastmod.as_deref()) {
            let declared = jsonld::declared_types(d);
            if node.get("@type").and_then(Json::as_str).is_some_and(|t| !declared.iter().any(|h| h == t)) {
                end_tags.push_str(&jsonld::script(&node));
            }
        }
        if d.meta("schema").is_some() {
            p.html = jsonld::drop_schema_meta(&p.html);
        }
        end_tags.push_str(&social_end);
        if cfg.markdown && indexable {
            end_tags.push_str(&format!(
                "<link rel=\"alternate\" type=\"text/markdown\" href=\"/{}\">",
                files::encode_path(&files::md_path(&p.route))
            ));
        }
        p.html = dedupe_head(&inject_head(
            &inject_head(&p.html, &social_start, false),
            &end_tags,
            true,
        ));
    }
    let docs: Vec<html::Doc> = pages.iter().map(|p| html::parse(&p.html)).collect();
    let entries: Vec<files::Entry> = pages
        .iter()
        .zip(&docs)
        .map(|(p, d)| {
            let mut alt: Vec<(String, String)> = alternates.get(&p.route).map_or(vec![], |v| {
                v.iter()
                    .map(|(l, r)| (l.clone(), cfg.abs(&files::encode_path(r))))
                    .collect()
            });
            if let (Some(i), false) = (&i18n, alt.is_empty())
                && let Some(d) = alt
                    .iter()
                    .find(|(l, _)| *l == i.default)
                    .map(|(_, u)| u.clone())
            {
                alt.push(("x-default".into(), d));
            }
            let page_date = p
                .article
                .as_ref()
                .and_then(|a| a.published.clone())
                .or_else(|| p.lastmod.clone());
            let videos = video::collect(cfg, root, &p.route, d, page_date.as_deref()).0;
            files::Entry {
                route: &p.route,
                url: cfg.abs(&files::encode_path(&p.route)),
                doc: d,
                lastmod: p.lastmod.as_deref(),
                indexable: audit::indexable(cfg, &p.route, d),
                alternates: alt,
                videos,
            }
        })
        .collect();
    files.push(("robots.txt".into(), files::robots(cfg)));
    if cfg.base != "/" && !cfg.staging {
        warnings.push(format!(
            "robots.txt is published at {}robots.txt, but crawlers only read the host root's robots.txt; fix: add `Sitemap: {}` to the robots.txt at the root of the host",
            cfg.base,
            cfg.abs("/sitemap.xml")
        ));
    }
    let (sm, rejected) = files::sitemaps_with(cfg, &entries, files::SITEMAP_MAX, files::SITEMAP_MAX_BYTES);
    for u in rejected {
        warnings.push(format!(
            "sitemap: {u} is left out, its URL is longer than {} characters or its entry is larger than a sitemap file; fix: shorten the route or the slug",
            files::LOC_MAX
        ));
    }
    if sm.is_empty() && cfg.sitemap && !cfg.staging {
        warnings.push("no sitemap.xml: it needs absolute URLs; fix: `brain build --site https://your-domain` or \"site\" in brain.json".into());
    }
    files.extend(sm);
    files.extend(files::news_sitemap(cfg, &published));
    // llms.txt / llms-full.txt per locale: the default at the root, the others under /<locale>/
    let groups: Vec<(String, Vec<files::Entry>)> = match &i18n {
        Some(i) => i
            .locales
            .iter()
            .map(|l| {
                (
                    if *l == i.default {
                        String::new()
                    } else {
                        format!("{l}/")
                    },
                    entries
                        .iter()
                        .filter(|e| i.locale_of(e.route) == l)
                        .cloned()
                        .collect(),
                )
            })
            .collect(),
        None => vec![(String::new(), entries.clone())],
    };
    for (dir, list) in &groups {
        if cfg.llms && (dir.is_empty() || !list.is_empty()) {
            files.push((format!("{dir}llms.txt"), files::llms_txt(cfg, list)));
        }
        if cfg.llms_full && (dir.is_empty() || !list.is_empty()) {
            files.push((format!("{dir}llms-full.txt"), files::llms_full(cfg, list)));
        }
    }
    if cfg.markdown {
        for e in entries.iter().filter(|e| e.indexable) {
            files.push((files::md_path(e.route), files::page_markdown(cfg, e)));
        }
    }
    if manifest {
        let icons = public_icons(root);
        let icon_refs: Vec<files::Icon> = icons
            .iter()
            .map(|(p, b)| files::Icon { path: p, bytes: b })
            .collect();
        let (m, w) = files::manifest(cfg, &entries, &tokens, &icon_refs);
        files.push(("manifest.webmanifest".into(), m));
        warnings.extend(w);
    }
    let name_for_feeds = files::site_name(cfg, &entries);
    files.extend(content::feeds(cfg, &published, &name_for_feeds));
    if cfg.feed || cfg.sections {
        let secs = sections::sections(&entries, &name_for_feeds);
        let mut feeds = Vec::new();
        if cfg.feed {
            let desc = files::site_description(cfg, &entries).unwrap_or_else(|| name_for_feeds.clone());
            // the site feed covers every indexable page, whether or not there are top-level pages (the root section)
            let all: Vec<&files::Entry> = entries.iter().filter(|e| e.indexable).collect();
            let mut todo: Vec<(String, String, String, String, Vec<&files::Entry>)> =
                vec![(sections::feed_path(""), name_for_feeds.clone(), desc.clone(), "/".into(), all)];
            for s in secs.iter().filter(|s| !s.id.is_empty()) {
                let d = s.home.and_then(|h| h.doc.meta("description")).map(str::to_string).unwrap_or_else(|| desc.clone());
                todo.push((sections::feed_path(&s.id), format!("{name_for_feeds} — {}", s.title), d, format!("/{}", s.id), s.pages.clone()));
            }
            for (path, title, d, home, pages) in todo {
                // a content collection already wrote its own (richer) feed at that path
                if files.iter().any(|(f, _)| *f == path) {
                    feeds.push(path);
                    continue;
                }
                if let Some(x) = sections::rss(cfg, &title, &d, &home, &format!("/{path}"), &pages, &name_for_feeds) {
                    files.push((path.clone(), x));
                    feeds.push(path);
                }
            }
        }
        if cfg.sections {
            files.push(("sections.json".into(), sections::manifest(cfg, &entries, &secs, &feeds)));
        }
    }
    // never link a feed that was not written (no indexable page at all)
    if !files.iter().any(|(f, _)| f == "rss.xml") {
        for p in pages.iter_mut() {
            p.html = p.html.replace(FEED_LINK, "");
        }
    }
    if let Some(f) = google.file.clone() {
        files.push(f);
    }
    if cfg.indexnow && !cfg.staging {
        match indexnow::key_problem(cfg.indexnow_key.as_deref(), "production") {
            None => files.push(indexnow::key_file(
                cfg.indexnow_key.as_deref().unwrap_or_default(),
            )),
            Some(p) => warnings.push(p),
        }
    }
    let mut all_assets: Vec<(String, u64)> = assets.to_vec();
    all_assets.extend(public_assets(root));
    all_assets.extend(files.iter().map(|(p, c)| (p.clone(), c.len() as u64)));
    all_assets.extend(binary.iter().map(|(p, b)| (p.clone(), b.len() as u64)));
    // redirects.json: invalid rules fail the build; removed URLs since the previous build are reported
    let rules =
        redirects::load(root).map_err(|(m, f)| format!("B1825 redirects.json: {m}; fix: {f}"))?;
    let page_routes: Vec<String> = pages.iter().map(|p| p.route.clone()).collect();
    let exists = |p: &str| {
        page_routes.iter().any(|r| audit::norm(r) == p)
            || routes.iter().any(|r| audit::matches_pattern(r, p))
            || all_assets.iter().any(|(a, _)| format!("/{a}") == p)
    };
    let bad = redirects::validate(&rules, &page_routes, &exists);
    if !bad.is_empty() {
        return Err(format!(
            "{} invalid redirect(s):\n{}",
            bad.len(),
            bad.iter()
                .map(|(m, f)| format!("B1825 redirects.json: {m}; fix: {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !rules.is_empty() || !cfg.redirect_policy.is_default() {
        files.push(("_redirects".into(), redirects::netlify_with(&rules, &cfg.redirect_policy)));
    }
    if let (Some(site), Ok(prev)) = (
        &cfg.site,
        std::fs::read_to_string(root.join("dist/sitemap.xml")),
    ) {
        for (old, to) in redirects::removed(&prev, site, &page_routes, &rules) {
            let suggestion = to.map_or(
                "{ \"from\": \"".to_string() + &old + "\", \"to\": \"/…\" }",
                |t| format!("{{ \"from\": \"{old}\", \"to\": \"{t}\" }}"),
            );
            warnings.push(format!("B1826 `{old}` was published by the previous build and is gone; fix: add {suggestion} to redirects.json"));
        }
    }
    let report = audit::audit(&Site {
        cfg,
        pages,
        routes,
        assets: &all_assets,
        redirects: &rules,
        images: &images,
        i18n: i18n.as_ref(),
    });
    let broken: Vec<String> = report
        .errors()
        .filter(|(_, f)| f.code == "B1819" || f.code == "B1820")
        .map(|(r, f)| format!("{} {r}: {}; fix: {}", f.code, f.msg, f.fix))
        .collect();
    if !broken.is_empty() {
        return Err(format!(
            "{} broken internal link(s):\n{}",
            broken.len(),
            broken.join("\n")
        ));
    }
    Ok(Built {
        files,
        warnings,
        csp: google.csp,
        redirects: rules,
        policy: cfg.redirect_policy.clone(),
        binary,
        report,
    })
}

/// Route patterns served by the server output (`dist/_server/manifest.json` → `routes[].path`): SSR-only pages such
/// as `/account` or `/users/:id` that have no HTML file in `dist/` but are valid link targets.
pub fn server_routes(dist: &Path) -> Vec<String> {
    let Ok(src) = std::fs::read_to_string(dist.join("_server/manifest.json")) else {
        return vec![];
    };
    let Ok(j) = Json::parse(&src) else {
        return vec![];
    };
    match j.get("routes") {
        Some(Json::Arr(a)) => a
            .iter()
            .filter_map(|r| r.get("path").and_then(Json::as_str))
            .map(str::to_string)
            .collect(),
        _ => vec![],
    }
}

/// Pages and assets of an existing `dist/` (for `brain seo audit` without rebuilding).
pub fn read_dist(dist: &Path) -> Result<(Vec<Page>, Assets), String> {
    if !dist.is_dir() {
        return Err(format!(
            "{} does not exist; fix: run `brain build` first",
            dist.display()
        ));
    }
    let mut all = Vec::new();
    walk(dist, "", &mut all);
    let mut pages = Vec::new();
    let mut assets = Vec::new();
    for (path, size) in all {
        if path.ends_with(".gz") || path.ends_with(".br") {
            continue;
        }
        if path == "404.html" {
            let html =
                std::fs::read_to_string(dist.join(&path)).map_err(|e| format!("{path}: {e}"))?;
            pages.push(Page {
                route: "*".into(),
                file: path,
                html,
                ..Page::default()
            });
        } else if path == "index.html" || path.ends_with("/index.html") {
            let html =
                std::fs::read_to_string(dist.join(&path)).map_err(|e| format!("{path}: {e}"))?;
            let route = format!(
                "/{}",
                path.trim_end_matches("index.html").trim_end_matches('/')
            );
            pages.push(Page {
                route,
                file: path,
                html,
                ..Page::default()
            });
        } else {
            assets.push((path, size));
        }
    }
    // SSR pages with a fixed path (`/login`, `/account`): their server template is the page a crawler gets
    if let Ok(src) = std::fs::read_to_string(dist.join("_server/manifest.json"))
        && let Ok(j) = Json::parse(&src)
        && let Some(Json::Arr(routes)) = j.get("routes")
    {
        for r in routes {
            let (Some(route), Some(page)) = (
                r.get("path").and_then(Json::as_str),
                r.get("page").and_then(Json::as_str),
            ) else {
                continue;
            };
            if route.contains([':', '*'])
                || pages
                    .iter()
                    .any(|p: &Page| audit::norm(&p.route) == audit::norm(route))
            {
                continue;
            }
            if let Ok(html) = std::fs::read_to_string(dist.join(page)) {
                pages.push(Page {
                    route: route.to_string(),
                    file: page.to_string(),
                    html,
                    ssr: true,
                    ..Page::default()
                });
            }
        }
    }
    Ok((pages, assets))
}

/// `brain seo <audit|links|preview <route>> [--json]`, run in the app root. Returns `(exit code, stdout)`: 0 ok,
/// 1 findings that fail, 2 usage.
pub fn cli(root: &Path, args: &[String]) -> (i32, String) {
    cli_with(root, args, &|_| None)
}

/// [`cli`] with the `keyword` of each route (it is never rendered, so `dist/` alone cannot tell B1816/B1836 apart):
/// `brain seo` reads it from the view sources, like the build does.
pub fn cli_with(root: &Path, args: &[String], keyword: &dyn Fn(&str) -> Option<String>) -> (i32, String) {
    let json = args.iter().any(|a| a == "--json");
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    let since = flag("--since");
    let env = flag("--env").unwrap_or("production");
    let words: Vec<&str> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            !a.starts_with("--")
                && (*i == 0 || !matches!(args[i - 1].as_str(), "--since" | "--env"))
        })
        .map(|(_, a)| a.as_str())
        .collect();
    let sub = words.first().copied().unwrap_or("audit");
    let usage = "usage: brain seo <audit|links|preview <route>|og|redirects|indexnow [--dry-run] [--since YYYY-MM-DD] [--env production]> [--json]\n  audit      score 0-100 and B18xx findings per page of dist/\n  links      broken links, broken anchors and orphan pages only\n  preview    search result, social card, structured data and findings of one page\n  og         social image of every page: set, generated from views/_og.brain, or missing\n  redirects  validate redirects.json against dist/ and print the rules for nginx, Caddy and _redirects\n  indexnow   send the sitemap URLs to IndexNow (deploy step; seo.indexnow + INDEXNOW_KEY in .brain_env)";
    if !matches!(
        sub,
        "audit" | "links" | "preview" | "og" | "redirects" | "indexnow"
    ) {
        return (2, format!("unknown subcommand `{sub}`\n{usage}"));
    }
    let fail = |e: String| {
        (
            1,
            if json {
                Json::obj([("ok", Json::Bool(false)), ("error", Json::str(&e))]).stringify()
            } else {
                e
            },
        )
    };
    let load = || -> Result<(Config, Vec<Page>, Report), String> {
        let mut cfg = Config::load(root)?;
        let (mut pages, mut assets) = read_dist(&root.join("dist"))?;
        for p in &mut pages {
            p.keyword = keyword(&p.route);
        }
        if pages.is_empty() {
            return Err("dist/ has no pages; fix: run `brain build` first".into());
        }
        // the origin the build actually used (PUBLIC_SITE_URL per environment) wins over brain.json "site"
        if let Some(built) = audit::infer_site(
            &pages
                .iter()
                .map(|p| html::parse(&p.html))
                .collect::<Vec<_>>(),
        ) {
            cfg.site = Some(built);
        }
        assets.extend(public_assets(root));
        let rules = redirects::load(root).unwrap_or_default();
        let mut images = image_dims_in(&root.join("dist"));
        images.extend(image_dims_in(&root.join("public")));
        let (i18n, _) = hreflang::I18n::load(&cfg, root);
        let routes = server_routes(&root.join("dist"));
        let report = audit::audit(&Site {
            cfg: &cfg,
            pages: &pages,
            routes: &routes,
            assets: &assets,
            redirects: &rules,
            images: &images,
            i18n: i18n.as_ref(),
        });
        Ok((cfg, pages, report))
    };
    if sub == "indexnow" {
        return match indexnow_cmd(root, env, since, args.iter().any(|a| a == "--dry-run")) {
            Ok(out) => (0, out),
            Err(e) => fail(e),
        };
    }
    let (cfg, pages, mut r) = match load() {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    if sub == "preview" {
        let route = audit::norm(words.get(1).copied().unwrap_or("/"));
        let Some(page) = pages.iter().find(|p| audit::norm(&p.route) == route) else {
            let known: Vec<&str> = pages.iter().map(|p| p.route.as_str()).collect();
            return fail(format!(
                "no built page at `{route}`; fix: use one of {}",
                known.join(", ")
            ));
        };
        let rep = r.pages.iter().find(|p| audit::norm(&p.route) == route);
        let p = preview::preview(&html::parse(&page.html), &cfg.abs(&page.route), rep);
        return (0, if json { p.json.stringify() } else { p.text });
    }
    if sub == "og" {
        let docs: Vec<html::Doc> = pages.iter().map(|p| html::parse(&p.html)).collect();
        let entries: Vec<files::Entry> = pages
            .iter()
            .zip(&docs)
            .map(|(p, d)| files::Entry {
                route: &p.route,
                url: String::new(),
                doc: d,
                lastmod: None,
                indexable: true,
                alternates: vec![],
                videos: vec![],
            })
            .collect();
        let tpl = og::template(root);
        let custom = root.join("views/_og.brain").is_file();
        let list: Vec<(&str, &html::Doc, bool)> = pages
            .iter()
            .zip(&docs)
            .map(|(p, d)| (p.route.as_str(), d, audit::indexable(&cfg, &p.route, d)))
            .collect();
        let jobs = og::plan(&cfg, cfg.og, &list, &files::site_name(&cfg, &entries), &tpl);
        let mut images = image_dims_in(&root.join("dist"));
        images.extend(image_dims_in(&root.join("public")));
        let mut rows = Vec::new();
        let mut text = format!(
            "Social images (seo.og = {}, template: {})\n",
            cfg.og.as_str(),
            if custom {
                "views/_og.brain"
            } else {
                "built-in"
            }
        );
        if cfg.og == og::Mode::Missing && cfg.site.is_none() {
            text.push_str(
                "  none generated: og:image must be an absolute URL; fix: set \"site\" in brain.json\n",
            );
        }
        let mut missing = 0;
        for (p, d) in pages
            .iter()
            .zip(&docs)
            .filter(|(p, d)| audit::indexable(&cfg, &p.route, d))
        {
            let img = d.meta("og:image");
            let dims = img
                .and_then(|i| audit::internal(&cfg, &p.route, i))
                .and_then(|(path, _)| {
                    images
                        .iter()
                        .find(|(a, _, _)| format!("/{a}") == path)
                        .map(|(_, w, h)| (*w, *h))
                });
            let job = jobs.iter().find(|j| j.route == p.route);
            let planned = job.is_some();
            let state = match (img, job) {
                (_, Some(j)) => match images.iter().find(|(a, _, _)| *a == j.path) {
                    Some((_, w, h)) => format!("generated: /{} ({w}x{h})", j.path),
                    None => format!("generated at the next `brain build`: /{}", j.path),
                },
                (Some(i), _) => match dims {
                    Some((w, h)) => format!("{i} ({w}x{h})"),
                    None => format!("{i} (not found in dist/ or public/)"),
                },
                (None, None) => {
                    missing += 1;
                    "none".to_string()
                }
            };
            text.push_str(&format!("  {} — {state}\n", p.route));
            rows.push(Json::obj([
                ("route", Json::str(&p.route)),
                ("image", img.map_or(Json::Null, Json::str)),
                (
                    "width",
                    dims.map_or(Json::Null, |d| Json::Num(f64::from(d.0))),
                ),
                (
                    "height",
                    dims.map_or(Json::Null, |d| Json::Num(f64::from(d.1))),
                ),
                ("generated", Json::Bool(planned)),
            ]));
        }
        let j = Json::obj([
            ("ok", Json::Bool(missing == 0)),
            (
                "template",
                Json::str(if custom {
                    "views/_og.brain"
                } else {
                    "built-in"
                }),
            ),
            ("pages", Json::Arr(rows)),
        ]);
        return (
            if missing == 0 { 0 } else { 1 },
            if json { j.stringify() } else { text },
        );
    }
    if sub == "redirects" {
        let rules = match redirects::load(root) {
            Ok(r) => r,
            Err((m, f)) => return fail(format!("B1825 redirects.json: {m}; fix: {f}")),
        };
        let (_, mut assets) = match read_dist(&root.join("dist")) {
            Ok(x) => x,
            Err(e) => return fail(e),
        };
        assets.extend(public_assets(root));
        let routes: Vec<String> = pages.iter().map(|p| p.route.clone()).collect();
        let ssr = server_routes(&root.join("dist"));
        let exists = |p: &str| {
            routes.iter().any(|r| audit::norm(r) == p)
                || ssr.iter().any(|r| audit::matches_pattern(r, p))
                || assets.iter().any(|(a, _)| format!("/{a}") == p)
        };
        let problems = redirects::validate(&rules, &routes, &exists);
        let ok = problems.is_empty();
        if json {
            let j = Json::obj([
                ("ok", Json::Bool(ok)),
                (
                    "rules",
                    Json::Arr(
                        rules
                            .iter()
                            .map(|r| {
                                Json::obj([
                                    ("from", Json::str(&r.from)),
                                    ("to", Json::str(&r.to)),
                                    ("status", Json::Num(f64::from(r.status))),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (
                    "problems",
                    Json::Arr(
                        problems
                            .iter()
                            .map(|(m, f)| {
                                Json::obj([
                                    ("code", Json::str("B1825")),
                                    ("msg", Json::str(m)),
                                    ("fix", Json::str(f)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("nginx", Json::str(redirects::nginx(&rules))),
                ("caddy", Json::str(redirects::caddy(&rules))),
                ("netlify", Json::str(redirects::netlify(&rules))),
                ("server", redirects::server_json(&rules)),
            ]);
            return (if ok { 0 } else { 1 }, j.stringify());
        }
        let mut o = format!("redirects.json: {} rule(s)\n", rules.len());
        for (m, f) in &problems {
            o.push_str(&format!("  error B1825: {m} · fix: {f}\n"));
        }
        if ok && !rules.is_empty() {
            o.push_str(&format!("\n_redirects (Cloudflare Pages, Netlify; already in dist/):\n{}\nnginx (server block):\n{}\nCaddy (site block):\n{}", redirects::netlify(&rules), redirects::nginx(&rules), redirects::caddy(&rules)));
        }
        return (if ok { 0 } else { 1 }, o);
    }
    if sub == "links" {
        for p in &mut r.pages {
            p.findings
                .retain(|f| matches!(f.code, "B1813" | "B1819" | "B1820" | "B1825"));
            p.score = None;
        }
        r.min_score = 0;
    }
    let code = if r.ok() { 0 } else { 1 };
    (
        code,
        if json {
            r.to_json().stringify()
        } else {
            r.text()
        },
    )
}

/// `brain seo indexnow`: the sitemap URLs (≥ `since`) as an IndexNow request; printed with `dry_run`, else sent.
fn indexnow_cmd(
    root: &Path,
    env: &str,
    since: Option<&str>,
    dry_run: bool,
) -> Result<String, String> {
    let cfg = Config::load(root)?.with_env(root, env);
    if !cfg.indexnow {
        return Err("IndexNow is off; fix: set \"seo\": { \"indexnow\": true } in brain.json and INDEXNOW_KEY in .brain_env".into());
    }
    if let Some(p) = indexnow::key_problem(cfg.indexnow_key.as_deref(), env) {
        return Err(p);
    }
    let key = cfg.indexnow_key.clone().unwrap_or_default();
    let sitemap = std::fs::read_to_string(root.join("dist/sitemap.xml")).map_err(|_| {
        "dist/sitemap.xml does not exist; fix: `brain build --site https://your-domain` first"
            .to_string()
    })?;
    let site = cfg
        .root()
        .or_else(|| {
            indexnow::urls(&sitemap, None).first().and_then(|u| {
                u.split_once("://")
                    .map(|(s, r)| format!("{s}://{}", r.split('/').next().unwrap_or(r)))
            })
        })
        .ok_or("the sitemap has no URLs")?;
    let urls = indexnow::urls(&sitemap, since);
    if urls.is_empty() {
        return Ok("nothing to send: no sitemap URL matches".into());
    }
    let body = indexnow::payload(&site, &key, &urls);
    if dry_run {
        return Ok(format!("POST {}\n{}", indexnow::ENDPOINT, body.stringify()));
    }
    let code = indexnow::send(&body)?;
    Ok(format!(
        "IndexNow accepted {} URL(s) (HTTP {code})",
        urls.len()
    ))
}

/// One MCP tool: `(name, description, [(param, JSON type, description)])`. All params are optional.
pub type ToolDef = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str, &'static str)],
);

/// MCP tools (`brain_seo_*`).
pub const TOOLS: &[ToolDef] = &[
    (
        "brain_seo_audit",
        "SEO audit of the built site (dist/): score 0-100 per page and B18xx findings, each with msg and fix. Run `brain_build` first. ok=false when a page is under seo.minScore or a link is broken.",
        &[
            ("path", "string", "app root (default: the current project)"),
            ("route", "string", "only this page, e.g. /pricing"),
        ],
    ),
    (
        "brain_seo_links",
        "Broken internal links (B1819), broken #anchors (B1820) and orphan pages (B1813) of the built site, with the page to link from.",
        &[("path", "string", "app root (default: the current project)")],
    ),
    (
        "brain_seo_og",
        "Social image of every built page: the og:image it has (with size), the ones generated from views/_og.brain, and the pages without one.",
        &[("path", "string", "app root (default: the current project)")],
    ),
    (
        "brain_seo_redirects",
        "Validates redirects.json against the built site (loops, chains, live sources, broken targets) and returns the rules for nginx, Caddy, _redirects and server.mjs.",
        &[("path", "string", "app root (default: the current project)")],
    ),
    (
        "brain_seo_preview",
        "How one built page shows in a search result (title and snippet cut by pixel width) and as a social card, its JSON-LD and its audit findings.",
        &[
            ("path", "string", "app root (default: the current project)"),
            ("route", "string", "page route, e.g. /pricing (default /)"),
        ],
    ),
];

/// Runs one of [`TOOLS`] with its JSON arguments; `root` is the project root when `path` is absent.
pub fn tool(name: &str, args: &Json, root: &Path) -> Result<Json, String> {
    let root = args
        .get("path")
        .and_then(Json::as_str)
        .map_or_else(|| root.to_path_buf(), std::path::PathBuf::from);
    let sub = match name {
        "brain_seo_audit" => "audit",
        "brain_seo_links" => "links",
        "brain_seo_preview" => "preview",
        "brain_seo_og" => "og",
        "brain_seo_redirects" => "redirects",
        _ => return Err(format!("unknown tool `{name}`")),
    };
    let mut argv = vec![sub.to_string()];
    if sub == "preview" {
        argv.push(
            args.get("route")
                .and_then(Json::as_str)
                .unwrap_or("/")
                .to_string(),
        );
    }
    argv.push("--json".into());
    let (_, out) = cli(&root, &argv);
    if matches!(sub, "preview" | "og" | "redirects") {
        let j = Json::parse(&out).map_err(|e| format!("internal: {e}"))?;
        return match j.get("error").and_then(Json::as_str) {
            Some(e) => Err(e.to_string()),
            None => Ok(j),
        };
    }
    let mut j = Json::parse(&out).map_err(|e| format!("internal: {e}"))?;
    if let (Some(route), Json::Obj(kv)) = (args.get("route").and_then(Json::as_str), &mut j) {
        for (k, v) in kv.iter_mut() {
            if k == "pages"
                && let Json::Arr(list) = v
            {
                list.retain(|p| p.get("route").and_then(Json::as_str) == Some(route));
            }
        }
    }
    if let Some(e) = j.get("error").and_then(Json::as_str) {
        return Err(e.to_string());
    }
    Ok(j)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_injection_and_dedupe() {
        let html = "<!doctype html><html><head><meta charset=\"utf-8\"><title>App</title><meta name=description content=a><link rel=canonical href=/a>\
<script>let s = '<meta name=description content=x>'</script><title>View</title><meta name=\"description\" content=\"b\"><link rel=\"canonical\" href=\"/b\"></head><body></body></html>";
        let out = dedupe_head(&inject_head(html, "<meta name=x content=1>", false));
        assert!(out.starts_with(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=x content=1>"
        ));
        assert_eq!(out.matches("<title>").count(), 1);
        assert!(
            out.contains("<title>View</title>")
                && out.contains("content=\"b\"")
                && !out.contains("content=a>")
        );
        assert!(out.contains("href=\"/b\"") && !out.contains("href=/a"));
        assert!(
            out.contains("'<meta name=description content=x>'"),
            "script contents are not tags"
        );
        let end = inject_head(
            "<head><title>x</title></head>",
            "<meta name=robots content=noindex>",
            true,
        );
        assert_eq!(
            end,
            "<head><title>x</title><meta name=robots content=noindex></head>"
        );
    }
}
