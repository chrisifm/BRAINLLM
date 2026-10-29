//! `brain seo audit` (INTERNALS §33): score 0–100 per indexable page from weighted checks, B18xx findings with an
//! actionable `fix`, and site-wide checks (orphans, duplicates, cannibalization, broken links and anchors).

use crate::files::abs_path;
use crate::html::{Doc, attr};
use crate::{Config, Site};
use brain_lex::Json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warning,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub code: &'static str,
    pub level: Level,
    pub msg: String,
    pub fix: String,
}

#[derive(Debug, Clone)]
pub struct PageReport {
    pub route: String,
    pub url: String,
    pub indexable: bool,
    /// `None` for pages that are not meant to rank (noindex, canonicalized elsewhere, staging).
    pub score: Option<u32>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub pages: Vec<PageReport>,
    pub site: Vec<Finding>,
    pub min_score: u32,
}

/// `(code, title, why, example_ok)` — same shape as `brain_check::CATALOG`, for `brain explain` and MCP.
pub const CATALOG: &[(&str, &str, &str, &str)] = &[
    (
        "B1801",
        "Page without title",
        "The title is the clickable line of the search result and the tab name.",
        "<meta seo title=\"Pricing — Acme\" description=\"…\">",
    ),
    (
        "B1802",
        "Title length",
        "Google cuts titles wider than about 600 px on desktop; under 10 characters a title cannot name both the topic and the site.",
        "<meta seo title=\"Team task planner with offline sync — Acme\">",
    ),
    (
        "B1803",
        "Page without description",
        "Without a description the search engine picks a random snippet of the page.",
        "<meta seo title=\"…\" description=\"Plan the week, assign tasks and track progress offline.\">",
    ),
    (
        "B1804",
        "Description length",
        "Snippets are cut around 920 px (about 155 characters); under 70 characters the snippet looks empty.",
        "<meta seo description=\"Plan the week, assign tasks and track progress, even offline. Free for small teams.\">",
    ),
    (
        "B1805",
        "H1 count",
        "One H1 per page names its topic for readers, screen readers and crawlers.",
        "<h1>Pricing</h1>",
    ),
    (
        "B1806",
        "Heading level skipped",
        "Headings form an outline; jumping from h2 to h4 breaks it.",
        "<h2>Plans</h2>\n<h3>Team</h3>",
    ),
    (
        "B1807",
        "Image without alt",
        "Image search and screen readers rely on alt; decorative images use alt=\"\".",
        "<img src=\"/chart.png\" alt=\"Sales by month, 2026\">",
    ),
    (
        "B1808",
        "Thin content",
        "Pages with very little text rarely rank; app screens that should not rank use robots=\"noindex\".",
        "<meta seo title=\"…\" robots=\"noindex\">",
    ),
    (
        "B1809",
        "Canonical",
        "The canonical URL tells search engines which address to index when a page is reachable at several.",
        "<meta seo canonical=\"/pricing\">",
    ),
    (
        "B1810",
        "Document without lang",
        "`lang` picks the search locale, hyphenation and the screen reader voice.",
        "<html lang=\"en\">",
    ),
    (
        "B1811",
        "Social card incomplete",
        "Shared links show og:title and og:image; without an image the card is a bare link.",
        "<meta seo og:image=\"/og/pricing.png\">",
    ),
    (
        "B1812",
        "No internal links out",
        "Links from the content pass relevance to other pages and keep readers on the site.",
        "<p>See <a href=\"/pricing\">pricing</a>.</p>",
    ),
    (
        "B1813",
        "Orphan page",
        "No other page links here, so crawlers and readers hardly reach it.",
        "<a href=\"/guides/setup\">Setup guide</a>",
    ),
    (
        "B1814",
        "Duplicate title",
        "Two pages with the same title compete for the same result.",
        "<meta seo title=\"Pricing — Acme\">",
    ),
    (
        "B1815",
        "Duplicate description",
        "Each page needs its own snippet.",
        "<meta seo description=\"…unique per page…\">",
    ),
    (
        "B1816",
        "Focus keyword placement",
        "The focus keyword should appear in the title, the H1, the first paragraph, the description and the URL, at a natural density.",
        "<meta seo keyword=\"task planner\" title=\"Task planner for teams\">",
    ),
    (
        "B1817",
        "Keyword cannibalization",
        "Two pages targeting the same keyword split clicks and links; give each page its own keyword or merge them.",
        "<meta seo keyword=\"offline task planner\">",
    ),
    (
        "B1818",
        "Invalid JSON-LD",
        "Structured data that does not parse or lacks @type is ignored by search engines.",
        "<meta seo schema=\"Article\">",
    ),
    (
        "B1819",
        "Broken internal link",
        "The link points to a page or file the build does not produce.",
        "<a href=\"/pricing\">Pricing</a>",
    ),
    (
        "B1820",
        "Broken anchor",
        "The `#fragment` does not match any id on the target page.",
        "<h2 id=\"faq\">FAQ</h2> … <a href=\"#faq\">FAQ</a>",
    ),
    (
        "B1821",
        "Heavy or unsized main image",
        "The first content image is usually the LCP element: keep it under 200 KB and give it width and height.",
        "<img src=\"/hero.avif\" width=\"1200\" height=\"630\" alt=\"…\">",
    ),
    (
        "B1822",
        "Staging build",
        "Non-production builds are noindex everywhere and robots.txt blocks crawling.",
        "brain build --env production",
    ),
    (
        "B1823",
        "Invalid content entry",
        "A content file needs closed frontmatter with `key: value` lines, a title, YYYY-MM-DD dates and a kebab-case slug.",
        "---\ntitle: Offline first\ndescription: Why the planner works without network.\ndate: 2026-10-01\n---",
    ),
    (
        "B1824",
        "Duplicate content slug",
        "Two entries of a collection would publish at the same URL.",
        "slug: offline-first-2",
    ),
    (
        "B1825",
        "Invalid redirect",
        "redirects.json rules need a `/path` source that is not a live page, an existing target, a 301/302/307/308 status, and no loops.",
        "[{ \"from\": \"/old-pricing\", \"to\": \"/pricing\", \"status\": 301 }]",
    ),
    (
        "B1827",
        "Social image unusable",
        "Facebook, LinkedIn, X, WhatsApp and Slack need an absolute og:image of about 1200x630 (1.91:1) under 1 MB for a large preview.",
        "<meta seo og:image=\"/og/pricing.png\"> (1200x630 PNG in public/og/)",
    ),
    (
        "B1828",
        "Locale mismatch",
        "Each language version is canonical to itself; a canonical pointing at another language hides that version, and `<html lang>` must match the URL's locale.",
        "/es/precios → <link rel=canonical href=\"https://acme.dev/es/precios\"> and <html lang=\"es\">",
    ),
    (
        "B1829",
        "Untranslated content entry",
        "An entry missing in a locale is shown in the default language (noindex) so the URL never 404s.",
        "content/blog/es/offline-first.md",
    ),
    (
        "B1830",
        "Answer first",
        "AI answers and featured snippets quote a short, direct answer; a page that opens with a long or missing paragraph is cited less.",
        "<h1>What is offline sync?</h1>\n<p>Offline sync lets you edit without network; changes merge when you reconnect.</p>",
    ),
    (
        "B1831",
        "Questions without FAQ",
        "Question headings answered right below them become FAQPage structured data when grouped, which answer engines and Bing read directly (Google no longer shows FAQ rich results since 2026-05-07).",
        "<section faq><details><summary>Is it free?</summary>Yes, for up to ten people.</details></section>",
    ),
    (
        "B1832",
        "Freshness",
        "Engines and LLMs prefer content with a visible date and a dateModified they can trust.",
        "<p><time datetime=\"2026-10-01\">October 1, 2026</time></p> (and `updated:` in the frontmatter)",
    ),
    (
        "B1833",
        "Author",
        "Articles with a named author and profile links (Person + sameAs) carry more trust (E-E-A-T) for search and AI answers.",
        "\"authors\": { \"Ada Lovelace\": { \"url\": \"https://ada.dev\", \"sameAs\": [\"https://www.linkedin.com/in/ada\"] } }",
    ),
    (
        "B1834",
        "No sources",
        "Long content that cites sources is cited more often by answer engines.",
        "<p>According to the <a href=\"https://www.w3.org/TR/…\">W3C spec</a>, …</p>",
    ),
    (
        "B1836",
        "Title or description off the page topic",
        "Without a focus keyword the H1 says what the page is about; a title or description that does not name it (\"Home\") gives search results and answer engines a snippet that matches no query.",
        "<meta seo title=\"Offline task planner — Acme\" description=\"An offline task planner that syncs when you reconnect.\">",
    ),
    (
        "B1835",
        "Video not indexable",
        "Search engines index a video (rich result, video sitemap) only with a poster image and an upload date.",
        "<video src=\"/demo.mp4\" poster=\"/thumbs/demo.jpg\" data-upload-date=\"2026-10-01\" title=\"Product demo\" controls></video>",
    ),
    (
        "B1826",
        "Removed URL without redirect",
        "A URL published by the previous build is gone; visitors and search engines that kept it get a 404.",
        "[{ \"from\": \"/old\", \"to\": \"/new\" }]",
    ),
];

/// Arial advance widths in 1/1000 em for ASCII 32..=126 (what search results use to cut titles and snippets).
const ARIAL: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Rendered width in px of `s` in Arial at `size` px (CJK and wide scripts count as 1 em).
pub fn px(s: &str, size: f32) -> f32 {
    let units: u32 = s
        .chars()
        .map(|c| match c as u32 {
            32..=126 => u32::from(ARIAL[c as usize - 32]),
            0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFF60 => 1000,
            _ => 556,
        })
        .sum();
    units as f32 * size / 1000.0
}

/// Desktop result limits: title 20 px Arial up to 600 px, description 14 px Arial up to 920 px.
pub const TITLE_PX: f32 = 600.0;
pub const DESC_PX: f32 = 920.0;
pub const THIN_WORDS: usize = 300;
/// Shorter titles cannot carry topic and site ("About").
pub const TITLE_MIN: usize = 10;
pub const LCP_BYTES: u64 = 200 * 1024;

/// Internal target of `href` on page `from`: `(path, fragment)`; `None` for external or non-HTTP links.
pub fn internal(cfg: &Config, from: &str, href: &str) -> Option<(String, String)> {
    let h = href.trim();
    let lower = h.to_ascii_lowercase();
    if h.is_empty()
        || ["mailto:", "tel:", "javascript:", "data:", "sms:", "blob:"]
            .iter()
            .any(|p| lower.starts_with(p))
    {
        return None;
    }
    let local =
        if lower.starts_with("http://") || lower.starts_with("https://") || h.starts_with("//") {
            let site = cfg.site.as_deref()?;
            let host = site.split("://").nth(1).unwrap_or(site);
            let rest = h.split("//").nth(1).unwrap_or("");
            let rest = rest.strip_prefix(host)?;
            if !(rest.is_empty() || rest.starts_with(['/', '?', '#'])) {
                return None;
            }
            if rest.is_empty() {
                "/".to_string()
            } else {
                rest.to_string()
            }
        } else {
            h.to_string()
        };
    let (path, frag) = local
        .split_once('#')
        .map_or((local.as_str(), ""), |(p, f)| (p, f));
    let path = path.split('?').next().unwrap_or("");
    let path = if path.is_empty() {
        from.to_string()
    } else {
        abs_path(from, path)
    };
    Some((norm(cfg.strip_base(&path)), frag.to_string()))
}

/// `/a/` → `/a`, `/a/index.html` → `/a`, `/b%20c` → `/b c` (percent-decoded so encoded and raw forms compare equal).
pub fn norm(path: &str) -> String {
    let p = path.strip_suffix("index.html").unwrap_or(path);
    let p = p.trim_end_matches('/');
    if p.is_empty() {
        "/".into()
    } else {
        percent_decode(p)
    }
}

/// Decodes `%XX` sequences (invalid UTF-8 or malformed escapes are kept as written).
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

pub fn matches_pattern(pattern: &str, path: &str) -> bool {
    if pattern == "*" {
        return false; // the 404 page catches everything; it does not make a link valid
    }
    let (a, b): (Vec<&str>, Vec<&str>) = (pattern.split('/').collect(), path.split('/').collect());
    a.len() == b.len()
        && a.iter()
            .zip(&b)
            .all(|(x, y)| x.starts_with(':') && !y.is_empty() || x == y)
}

/// Whether a page is meant to rank: not the not-found page (`*`) or another special page (`!…`), not noindex, not
/// canonicalized to another URL, not a staging build.
pub fn indexable(cfg: &Config, route: &str, doc: &Doc) -> bool {
    if route == "*" || route.starts_with('!') || cfg.staging
        || doc
            .meta("robots")
            .is_some_and(|r| r.to_ascii_lowercase().contains("noindex"))
    {
        return false;
    }
    let Some(c) = doc.link("canonical") else {
        return true;
    };
    match internal(cfg, route, c) {
        Some((p, _)) => p == norm(route),
        // absolute canonical and no known site: the host cannot be compared, so compare the path
        None if cfg.site.is_none() => url_path(c).is_some_and(|p| norm(p) == norm(route)),
        None => false,
    }
}

/// Path of an absolute URL (`https://a.dev/x?y` → `/x`).
pub fn url_path(u: &str) -> Option<&str> {
    let rest = u.split_once("://")?.1;
    let p = rest.find('/').map_or("/", |i| &rest[i..]);
    Some(p.split(['?', '#']).next().unwrap_or("/"))
}

/// `scheme://host` of the first absolute canonical or og:url among the pages.
pub fn infer_site(docs: &[Doc]) -> Option<String> {
    docs.iter()
        .filter_map(|d| d.link("canonical").or_else(|| d.meta("og:url")))
        .find_map(|u| {
            let (scheme, rest) = u.split_once("://")?;
            let host = rest.split(['/', '?', '#']).next()?;
            (matches!(scheme, "http" | "https") && !host.is_empty())
                .then(|| format!("{scheme}://{host}"))
        })
}

struct Scorer {
    earned: u32,
    total: u32,
    findings: Vec<Finding>,
}

impl Scorer {
    fn check(
        &mut self,
        weight: u32,
        pass: bool,
        code: &'static str,
        level: Level,
        msg: impl FnOnce() -> (String, String),
    ) {
        self.total += weight;
        if pass {
            self.earned += weight;
        } else {
            let (m, f) = msg();
            self.findings.push(Finding {
                code,
                level,
                msg: m,
                fix: f,
            });
        }
    }
    fn note(&mut self, code: &'static str, level: Level, msg: String, fix: String) {
        self.findings.push(Finding {
            code,
            level,
            msg,
            fix,
        });
    }
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

/// Occurrences of `kw` in `text` as whole words (case-insensitive).
fn count_phrase(text: &str, kw: &str) -> usize {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let k: Vec<String> = kw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if k.is_empty() || words.len() < k.len() {
        return 0;
    }
    words
        .windows(k.len())
        .filter(|w| *w == k.as_slice())
        .count()
}

/// Common English words that never name a topic.
const STOP: &[&str] = &[
    "about", "after", "also", "been", "best", "each", "every", "from", "have", "here", "home",
    "into", "just", "like", "made", "make", "more", "most", "only", "other", "over", "page",
    "some", "such", "than", "that", "their", "them", "then", "there", "these", "they", "this",
    "those", "what", "when", "where", "which", "while", "will", "with", "would", "your",
];

/// What a page is about: its focus keyword, else the significant words of its H1 (or title).
fn topic(keyword: Option<&str>, doc: &Doc) -> Vec<String> {
    if let Some(k) = keyword.map(str::trim).filter(|k| !k.is_empty()) {
        return vec![k.to_lowercase()];
    }
    let src = doc
        .headings
        .iter()
        .find(|(l, _)| *l == 1)
        .map(|(_, t)| t.as_str())
        .or(doc.title.as_deref())
        .unwrap_or("");
    let mut v: Vec<String> = src
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 4 && !STOP.contains(&w.as_str()))
        .collect();
    v.dedup();
    v
}

/// The topic term `text` mentions: the whole keyword, or two of the H1 words (one if the H1 has a single word).
fn mentions(text: &str, terms: &[String]) -> Option<String> {
    let hits: Vec<&String> = terms.iter().filter(|t| count_phrase(text, t) > 0).collect();
    let need = if terms.len() == 1 { 1 } else { 2 };
    (!terms.is_empty() && hits.len() >= need).then(|| {
        hits.iter()
            .map(|h| h.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// `body` is false for server-rendered pages read from their template: only what the `<head>` carries is checked,
/// since the view's content is filled in per request.
#[allow(clippy::too_many_arguments)]
fn page_checks(
    cfg: &Config,
    site: &Site,
    route: &str,
    doc: &Doc,
    keyword: Option<&str>,
    suggest: &[(String, String)],
    body: bool,
    s: &mut Scorer,
) {
    // title
    let title = doc.title.as_deref().unwrap_or("").trim();
    s.check(10, !title.is_empty(), "B1801", Level::Error, || {
        (
            "page has no <title>".into(),
            "add `title=\"…\"` to the view's <meta seo>".into(),
        )
    });
    if !title.is_empty() {
        let w = px(title, 20.0);
        let chars = title.chars().count();
        s.check(5, w <= TITLE_PX, "B1802", Level::Warning, || {
            (
                format!(
                    "title is {w:.0} px wide ({chars} chars); results cut it at {TITLE_PX:.0} px"
                ),
                "shorten the title to about 55 characters, main words first".into(),
            )
        });
        s.check(3, chars >= TITLE_MIN, "B1802", Level::Warning, || {
            (format!("title has {chars} chars; it does not say what the page is or whose it is"), "name the topic and the site, e.g. \"Pricing — Acme\" (a titleTemplate in app.brain adds the site to every view)".into())
        });
    }
    // description
    let desc = doc.meta("description").unwrap_or("").trim();
    s.check(10, !desc.is_empty(), "B1803", Level::Warning, || {
        (
            "page has no meta description".into(),
            "add `description=\"…\"` (70–155 characters) to the view's <meta seo>".into(),
        )
    });
    if !desc.is_empty() {
        let w = px(desc, 14.0);
        let chars = desc.chars().count();
        s.check(5, w <= DESC_PX && chars >= 70, "B1804", Level::Warning, || {
            if w > DESC_PX {
                (format!("description is {w:.0} px wide ({chars} chars); snippets cut at {DESC_PX:.0} px"), "keep it under about 155 characters".into())
            } else {
                (format!("description has {chars} chars"), "write 70–155 characters that say what the page offers".into())
            }
        });
    }
    let h1: Vec<&String> = doc
        .headings
        .iter()
        .filter(|(l, _)| *l == 1)
        .map(|(_, t)| t)
        .collect();
    let words = doc.words();
    if body {
        // headings
        s.check(10, h1.len() == 1, "B1805", Level::Warning, || {
            if h1.is_empty() {
                (
                    "page has no <h1>".into(),
                    "add one <h1> naming the page topic".into(),
                )
            } else {
                (
                    format!("page has {} <h1> elements", h1.len()),
                    "keep one <h1>; turn the others into <h2>".into(),
                )
            }
        });
        let skip = doc.headings.windows(2).find(|w| w[1].0 > w[0].0 + 1);
        s.check(3, skip.is_none(), "B1806", Level::Info, || {
            let w = skip.unwrap();
            (
                format!(
                    "heading jumps from h{} to h{} (\"{}\")",
                    w[0].0, w[1].0, w[1].1
                ),
                format!("use <h{}> for \"{}\"", w[0].0 + 1, w[1].1),
            )
        });
        // images
        let no_alt: Vec<&str> = doc
            .images
            .iter()
            .filter(|(a, _)| attr(a, "alt").is_none())
            .map(|(a, _)| attr(a, "src").unwrap_or("?"))
            .collect();
        s.check(7, no_alt.is_empty(), "B1807", Level::Warning, || {
            (
                format!(
                    "{} image(s) without alt: {}",
                    no_alt.len(),
                    no_alt.join(", ")
                ),
                "describe each image in `alt`; use alt=\"\" only for decorative ones".into(),
            )
        });
        // content
        s.check(5, words >= THIN_WORDS, "B1808", Level::Info, || {
            (
                format!("main content has {words} words"),
                format!(
                    "write at least {THIN_WORDS} words, or mark app screens robots=\"noindex\""
                ),
            )
        });
    }
    // canonical
    let canonical = doc.link("canonical");
    s.check(
        8,
        canonical.is_some_and(|c| c.contains("://")),
        "B1809",
        Level::Warning,
        || match canonical {
            None => (
                "page has no canonical URL".into(),
                "build with `--site https://your-domain` (canonical is filled in from the route)"
                    .into(),
            ),
            Some(c) => (
                format!("canonical `{c}` is relative"),
                "build with `--site https://your-domain` so the canonical is absolute".into(),
            ),
        },
    );
    s.check(
        5,
        doc.lang.as_deref().is_some_and(|l| !l.is_empty()),
        "B1810",
        Level::Warning,
        || {
            (
                "<html> has no lang".into(),
                "set `<html lang=\"en\">` (or the page language) in app.brain".into(),
            )
        },
    );
    let og = doc.meta("og:title").is_some() && doc.meta("og:image").is_some();
    s.check(4, og, "B1811", Level::Info, || {
        (
            "og:title or og:image is missing".into(),
            "add `og:image=\"/og/<page>.png\"` to <meta seo> (or a site default in app.brain)"
                .into(),
        )
    });
    if let Some(img) = doc.meta("og:image") {
        let local = internal(cfg, route, img).map(|(p, _)| p);
        let dims = local.as_deref().and_then(|p| site.image_dims(p));
        let bytes = local.as_deref().and_then(|p| site.asset_size(p));
        let problem: Option<(String, String)> = if !img.starts_with("http") {
            Some((
                format!(
                    "og:image `{img}` is relative; Facebook, LinkedIn and X ignore relative images"
                ),
                "build with `--site https://your-domain` so it becomes absolute".into(),
            ))
        } else if let Some((w, h)) = dims.filter(|(w, h)| *w < 1200 || *h < 630) {
            Some((
                format!(
                    "og:image is {w}x{h}; Facebook and LinkedIn show the large card from 1200x630"
                ),
                "export the social image at 1200x630 (1.91:1)".into(),
            ))
        } else if let Some((w, h)) = dims.filter(|(w, h)| {
            let r = *w as f32 / (*h).max(1) as f32;
            !(1.6..=2.2).contains(&r)
        }) {
            Some((
                format!("og:image is {w}x{h}; feeds crop it to 1.91:1"),
                "use a 1200x630 image so nothing important is cut".into(),
            ))
        } else {
            bytes.filter(|n| *n > 1024 * 1024).map(|n| {
                (
                    format!(
                        "og:image weighs {} KB; messengers like WhatsApp skip heavy previews",
                        n / 1024
                    ),
                    "compress it under 1 MB (JPEG/PNG at 1200x630)".to_string(),
                )
            })
        };
        s.check(4, problem.is_none(), "B1827", Level::Warning, || {
            problem.clone().unwrap_or_default()
        });
    }
    // structured data
    if !doc.jsonld.is_empty() {
        let issues: Vec<crate::jsonld::Issue> = doc
            .jsonld
            .iter()
            .flat_map(|b| crate::jsonld::validate(b))
            .collect();
        let worst = issues.iter().map(|i| i.level).max().unwrap_or(Level::Info);
        let mut rest = issues.iter().filter(|i| i.level > Level::Info);
        let first = rest.next();
        s.check(5, first.is_none(), "B1818", worst, || {
            first
                .map(|i| (i.msg.clone(), i.fix.clone()))
                .unwrap_or_default()
        });
        for i in rest.chain(issues.iter().filter(|i| i.level == Level::Info)) {
            s.note("B1818", i.level, i.msg.clone(), i.fix.clone());
        }
    }
    if !body {
        return;
    }
    // internal links out of the content
    let out = doc
        .anchors
        .iter()
        .filter(|a| a.in_content)
        .filter(|a| internal(cfg, route, &a.href).is_some_and(|(p, _)| p != norm(route)))
        .count();
    s.check(4, out > 0, "B1812", Level::Info, || {
        let fix = if suggest.is_empty() {
            "link related pages from the text with descriptive anchor text".to_string()
        } else {
            format!(
                "link from the text to {}",
                suggest
                    .iter()
                    .map(|(r, t)| format!("{r} (\"{t}\")"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        ("the content links to no other page of the site".into(), fix)
    });
    // focus keyword
    if let Some(kw) = keyword.map(str::trim).filter(|k| !k.is_empty()) {
        let slug = kw
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        // the description is the snippet search results and LLM answers show: it names the query too
        let places = [
            ("title", contains_ci(title, kw), 3, "`title=` of <meta seo>"),
            ("h1", h1.iter().any(|h| contains_ci(h, kw)), 2, "<h1>"),
            ("first paragraph", contains_ci(&doc.first_paragraph, kw), 2, "first sentence of the first <p> in <main>"),
            ("description", contains_ci(desc, kw), 2, "`description=` of <meta seo>"),
            // the home page carries the brand, not the query, in its URL
            ("URL", route == "/" || route.to_lowercase().contains(&slug), 1, "route (file name under views/)"),
        ];
        for (place, ok, w, where_) in places {
            s.check(w, ok, "B1816", Level::Warning, || {
                (
                    format!("focus keyword \"{kw}\" is not in the {place}"),
                    format!("write the exact words \"{kw}\" into the {where_}, worded naturally"),
                )
            });
        }
        let hits = count_phrase(&doc.text, kw);
        let density = if words == 0 {
            0.0
        } else {
            hits as f32 * kw.split_whitespace().count() as f32 * 100.0 / words as f32
        };
        if words >= 100 && !(0.5..=3.0).contains(&density) {
            s.note(
                "B1816",
                Level::Info,
                format!("focus keyword \"{kw}\" density is {density:.1} % ({hits} uses in {words} words)"),
                if density > 3.0 { "use synonyms instead of repeating the keyword".into() } else { format!("mention \"{kw}\" naturally a few more times") },
            );
        }
    } else if body && let Some((_, h1)) = doc.headings.iter().find(|(l, _)| *l == 1) {
        // no keyword: the H1 is the page's topic, and the title and description (the result snippet) should name it
        let terms = topic(None, doc);
        for (place, text, attr_) in [("title", title, "title"), ("description", desc, "description")] {
            if !text.is_empty() && mentions(text, &terms).is_none() {
                s.note(
                    "B1836",
                    Level::Info,
                    format!("the {place} does not name the page topic \"{}\" (its H1)", h1.trim()),
                    format!(
                        "reword `{attr_}=` of <meta seo> to contain the main words of the H1 (\"{}\"), or add `keyword=\"<the query>\"` and use it in the title, H1 and description",
                        h1.trim()
                    ),
                );
            }
        }
    }
    // main image (LCP candidate)
    if let Some((a, _)) = doc.images.iter().find(|(_, c)| *c) {
        let src = attr(a, "src").unwrap_or("");
        let size = internal(cfg, route, src).and_then(|(p, _)| site.asset_size(&p));
        let sized = attr(a, "width").is_some() && attr(a, "height").is_some();
        s.check(
            5,
            size.is_none_or(|n| n <= LCP_BYTES) && sized,
            "B1821",
            Level::Warning,
            || match size {
                Some(n) if n > LCP_BYTES => (
                    format!("main image `{src}` weighs {} KB", n / 1024),
                    "compress it (AVIF/WebP) under 200 KB".into(),
                ),
                _ => (
                    format!("main image `{src}` has no width/height"),
                    "add width and height so the layout does not shift".into(),
                ),
            },
        );
    }
}

/// GEO (generative engine optimization) hints: info level, they never change the score.
fn geo_checks(cfg: &Config, route: &str, doc: &Doc, s: &mut Scorer) {
    let words = doc.words();
    if words >= 100 {
        let n = doc.first_paragraph.chars().count();
        if n == 0 {
            s.note("B1830", Level::Info, "the content does not open with a paragraph".into(), "start with 1-2 sentences (300 characters at most) that answer the page's main question".into());
        } else if n > 300 {
            s.note(
                "B1830",
                Level::Info,
                format!("the first paragraph has {n} characters"),
                "open with a direct answer in 300 characters at most, then expand".into(),
            );
        }
    }
    let questions: Vec<&str> = doc
        .headings
        .iter()
        .filter(|(l, t)| *l >= 2 && t.trim_end().ends_with('?'))
        .map(|(_, t)| t.as_str())
        .collect();
    let types = crate::jsonld::declared_types(doc);
    if questions.len() >= 2 && doc.faq.is_empty() && !types.iter().any(|t| t == "FAQPage") {
        s.note(
            "B1831",
            Level::Info,
            format!("{} question headings (\"{}\", …) are not marked as an FAQ", questions.len(), questions[0]),
            "group them as <section faq><details><summary>Question?</summary>Answer</details></section> (FAQPage JSON-LD is automatic)".into(),
        );
    }
    let article_types = ["Article", "BlogPosting", "NewsArticle"];
    let article = types.iter().any(|t| article_types.contains(&t.as_str()))
        || doc.meta("og:type") == Some("article");
    if article {
        let nodes: Vec<brain_lex::Json> = crate::jsonld::typed_nodes(doc).into_iter().filter(|n| matches!(n.get("@type"), Some(brain_lex::Json::Str(t)) if article_types.contains(&t.as_str()))).collect();
        if doc.times.is_empty() {
            s.note(
                "B1832",
                Level::Info,
                "the article shows no date".into(),
                "show the date in the text: <time datetime=\"2026-10-01\">October 1, 2026</time>"
                    .into(),
            );
        }
        if !nodes.is_empty() && !nodes.iter().any(|n| n.get("dateModified").is_some()) {
            s.note(
                "B1832",
                Level::Info,
                "the article JSON-LD has no dateModified".into(),
                "set `date:` (and `updated:` when you revise it) in the frontmatter".into(),
            );
        }
        let author = nodes.iter().find_map(|n| n.get("author"));
        match author {
            None => s.note("B1833", Level::Info, "the article has no author".into(), "add `author: <name>` to the frontmatter and the profile under \"authors\" in brain.json".into()),
            Some(a) if a.get("sameAs").is_none() && a.get("url").is_none() => {
                let name = a.get("name").and_then(brain_lex::Json::as_str).unwrap_or("the author");
                s.note("B1833", Level::Info, format!("{name} has no profile links"), format!("add \"authors\": {{ \"{name}\": {{ \"url\": \"https://…\", \"sameAs\": [\"https://www.linkedin.com/in/…\"] }} }} to brain.json"));
            }
            _ => {}
        }
    }
    if words >= THIN_WORDS
        && !doc.anchors.iter().any(|a| {
            a.in_content && internal(cfg, route, &a.href).is_none() && a.href.starts_with("http")
        })
    {
        s.note(
            "B1834",
            Level::Info,
            format!("{words} words and no link to an outside source"),
            "cite 1-3 authoritative sources with links where you state facts or numbers".into(),
        );
    }
}

/// B1835: videos in the content that cannot be indexed (info).
fn video_checks(doc: &Doc, page_date: Option<&str>, s: &mut Scorer) {
    for v in doc.videos.iter().filter(|v| v.in_content) {
        let label = v.src.clone().unwrap_or_else(|| "a video".into());
        if v.poster.is_none() {
            s.note(
                "B1835",
                Level::Info,
                format!("`{label}` has no poster, so it is not indexed as a video"),
                "add poster=\"/thumbs/….jpg\" (a frame of the video)".into(),
            );
        } else if v.upload.is_none() && page_date.is_none() {
            s.note(
                "B1835",
                Level::Info,
                format!("`{label}` has no upload date"),
                "add data-upload-date=\"YYYY-MM-DD\"".into(),
            );
        }
    }
}

/// Audits every page. Pure: reads nothing from disk (`Site::assets` carries the file list and sizes).
pub fn audit(site: &Site) -> Report {
    let cfg = site.cfg;
    let docs: Vec<Doc> = site
        .pages
        .iter()
        .map(|p| crate::html::parse(&p.html))
        .collect();
    let idx: Vec<bool> = site
        .pages
        .iter()
        .zip(&docs)
        .map(|(p, d)| indexable(cfg, &p.route, d))
        .collect();
    // pass 1: site-wide facts
    let mut inbound: BTreeMap<String, usize> = BTreeMap::new();
    for (p, doc) in site.pages.iter().zip(&docs) {
        let me = norm(&p.route);
        let targets: BTreeSet<String> = doc
            .anchors
            .iter()
            .filter_map(|a| internal(cfg, &p.route, &a.href))
            .map(|(t, _)| t)
            .filter(|t| *t != me)
            .collect();
        for t in targets {
            *inbound.entry(t).or_default() += 1;
        }
    }
    let mut groups: [BTreeMap<String, Vec<usize>>; 3] = Default::default(); // title, description, keyword
    for (i, (p, doc)) in site
        .pages
        .iter()
        .zip(&docs)
        .enumerate()
        .filter(|(i, _)| idx[*i])
    {
        let keys = [
            doc.title.as_deref(),
            doc.meta("description"),
            p.keyword.as_deref(),
        ];
        for (g, k) in groups.iter_mut().zip(keys) {
            if let Some(k) = k.map(str::trim).filter(|k| !k.is_empty()) {
                g.entry(k.to_lowercase()).or_default().push(i);
            }
        }
    }
    // internal-link suggestions by shared topic: (from, to, term) where `from`'s content mentions `to`'s topic but does not link it
    let topics: Vec<Vec<String>> = site
        .pages
        .iter()
        .zip(&docs)
        .map(|(p, d)| topic(p.keyword.as_deref(), d))
        .collect();
    let links: Vec<BTreeSet<String>> = site
        .pages
        .iter()
        .zip(&docs)
        .map(|(p, d)| {
            d.anchors
                .iter()
                .filter(|a| a.in_content)
                .filter_map(|a| internal(cfg, &p.route, &a.href))
                .map(|(t, _)| t)
                .collect()
        })
        .collect();
    let mut suggest: Vec<(usize, usize, String)> = Vec::new();
    for (from, d) in docs.iter().enumerate().filter(|(i, _)| idx[*i]) {
        for (to, t) in topics
            .iter()
            .enumerate()
            .filter(|(j, _)| idx[*j] && *j != from)
        {
            if !links[from].contains(&norm(&site.pages[to].route))
                && let Some(term) = mentions(&d.text, t)
            {
                suggest.push((from, to, term));
            }
        }
    }
    // language versions of one page (paired by hreflang) are the same document: an untranslated copy shares its title
    // and description without competing, and a link to any version reaches the group
    let alts: BTreeMap<String, Vec<(String, String)>> = site.i18n.map_or_else(BTreeMap::new, |i| {
        let list: Vec<(&str, Option<&str>)> = site
            .pages
            .iter()
            .map(|p| (p.route.as_str(), p.view.as_deref()))
            .collect();
        crate::hreflang::alternates(i, &list)
    });
    let paired = |a: &str, b: &str| {
        alts.get(a)
            .is_some_and(|v| v.iter().any(|(_, r)| norm(r) == norm(b)))
    };
    let others = |g: &BTreeMap<String, Vec<usize>>, key: Option<&str>, me: usize| -> Vec<String> {
        key.and_then(|k| g.get(&k.trim().to_lowercase()))
            .map_or(vec![], |l| {
                l.iter()
                    .filter(|&&j| j != me && !paired(&site.pages[me].route, &site.pages[j].route))
                    .map(|&j| site.pages[j].route.clone())
                    .collect()
            })
    };
    // pass 2: one score per page
    let mut pages = Vec::new();
    for (i, (p, doc)) in site.pages.iter().zip(&docs).enumerate() {
        let mut s = Scorer {
            earned: 0,
            total: 0,
            findings: Vec::new(),
        };
        if idx[i] {
            let out: Vec<(String, String)> = suggest
                .iter()
                .filter(|(f, _, _)| *f == i)
                .take(3)
                .map(|(_, t, term)| (site.pages[*t].route.clone(), term.clone()))
                .collect();
            page_checks(
                cfg,
                site,
                &p.route,
                doc,
                p.keyword.as_deref(),
                &out,
                !p.ssr,
                &mut s,
            );
            if !p.ssr {
                geo_checks(cfg, &p.route, doc, &mut s);
            }
            let page_date = p
                .article
                .as_ref()
                .and_then(|a| a.published.as_deref())
                .or(p.lastmod.as_deref());
            video_checks(doc, page_date, &mut s);
            let linked = |r: &str| inbound.get(&norm(r)).copied().unwrap_or(0) > 0;
            let orphan = norm(&p.route) != "/"
                && !linked(&p.route)
                && !alts
                    .get(&p.route)
                    .is_some_and(|v| v.iter().any(|(_, r)| linked(r)));
            s.check(6, !orphan, "B1813", Level::Warning, || {
                let from: Vec<String> = suggest
                    .iter()
                    .filter(|(_, t, _)| *t == i)
                    .take(3)
                    .map(|(f, _, term)| format!("{} (mentions \"{term}\")", site.pages[*f].route))
                    .collect();
                let fix = if from.is_empty() {
                    "link it from a related page or the navigation".to_string()
                } else {
                    format!("link it from {}", from.join(", "))
                };
                ("no other page links here".into(), fix)
            });
            let dt = others(&groups[0], doc.title.as_deref(), i);
            s.check(6, dt.is_empty(), "B1814", Level::Warning, || {
                (
                    format!("same title as {}", dt.join(", ")),
                    "write a title unique to this page".into(),
                )
            });
            let dd = others(&groups[1], doc.meta("description"), i);
            s.check(4, dd.is_empty(), "B1815", Level::Warning, || {
                (
                    format!("same description as {}", dd.join(", ")),
                    "write a description unique to this page".into(),
                )
            });
            let dk = others(&groups[2], p.keyword.as_deref(), i);
            if !dk.is_empty() {
                s.note(
                    "B1817",
                    Level::Warning,
                    format!("same focus keyword as {}", dk.join(", ")),
                    "give each page its own focus keyword, or merge the pages and redirect one"
                        .into(),
                );
            }
        }
        if let Some(c) = doc
            .link("canonical")
            .filter(|c| !c.contains("://") && !c.starts_with('/'))
        {
            s.note(
                "B1809",
                Level::Warning,
                format!("canonical `{c}` is not a URL"),
                "use the page's absolute URL, or no canonical at all on a not-found page".into(),
            );
        } else if !idx[i]
            && let Some(c) = doc
                .link("canonical")
                .filter(|c| c.contains("://") && internal(cfg, &p.route, c).is_none())
        {
            s.note(
                "B1809",
                Level::Warning,
                format!("canonical `{c}` is outside {}, so the page counts as a copy and is not scored", cfg.site.as_deref().unwrap_or("the site")),
                "make PUBLIC_SITE_URL (.brain_env, per environment) the site's real origin, or remove the canonical override".into(),
            );
        }
        if let Some(i) = site.i18n {
            let mine = i.locale_of(&p.route).to_string();
            if let Some((target, _)) = doc
                .link("canonical")
                .and_then(|c| internal(cfg, &p.route, c))
                && i.locale_of(&target) != mine
            {
                s.note("B1828", Level::Error, format!("canonical points at `{target}`, a {} page, from a {mine} page", i.locale_of(&target)), "keep each language canonical to itself; pair versions with hreflang (automatic)".into());
            }
            let primary = |t: &str| {
                t.split(['-', '_'])
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
            };
            if let Some(l) = doc.lang.as_deref().filter(|l| primary(l) != primary(&mine)) {
                s.note(
                    "B1828",
                    Level::Warning,
                    format!("`<html lang=\"{l}\">` on a {mine} URL"),
                    format!("render this page with lang=\"{mine}\""),
                );
            }
        }
        // links: checked on every page, indexable or not
        for a in &doc.anchors {
            let Some((path, frag)) = internal(cfg, &p.route, &a.href) else {
                continue;
            };
            let target = if path == norm(&p.route) {
                Some(doc)
            } else {
                site.pages
                    .iter()
                    .position(|q| norm(&q.route) == path)
                    .map(|j| &docs[j])
            };
            let exists = target.is_some()
                || site.routes.iter().any(|r| matches_pattern(r, &path))
                || site.asset_size(&path).is_some();
            let redirected = site
                .redirects
                .iter()
                .find_map(|r| crate::redirects::apply(r, &path));
            if let (false, Some(to)) = (exists, &redirected) {
                s.note(
                    "B1825",
                    Level::Info,
                    format!("link `{}` goes through a redirect to `{to}`", a.href),
                    format!("link `{to}` directly"),
                );
            } else if !exists {
                s.note(
                    "B1819",
                    Level::Error,
                    format!(
                        "link `{}` (\"{}\") points to `{path}`, which the build does not produce",
                        a.href, a.text
                    ),
                    "fix the href, create the view, or add a redirect in redirects.json".into(),
                );
            } else if let Some(t) = target
                && !frag.is_empty()
                && frag != "top"
                && !t.ids.contains(&frag)
            {
                s.note(
                    "B1820",
                    Level::Error,
                    format!(
                        "anchor `{}` has no element with id=\"{frag}\" on `{path}`",
                        a.href
                    ),
                    anchor_fix(&frag, t.ids.iter().map(|x| x.as_str())),
                );
            }
            if a.rel
                .split_ascii_whitespace()
                .any(|r| r.eq_ignore_ascii_case("nofollow"))
            {
                s.note(
                    "B1812",
                    Level::Info,
                    format!("internal link `{}` is nofollow", a.href),
                    "remove rel=\"nofollow\" from links to your own pages".into(),
                );
            }
        }
        let score = idx[i].then(|| {
            (s.earned * 100 + s.total / 2)
                .checked_div(s.total)
                .unwrap_or(100)
        });
        pages.push(PageReport {
            route: p.route.clone(),
            url: cfg.abs(&p.route),
            indexable: idx[i],
            score,
            findings: s.findings,
        });
    }
    let mut site_f = Vec::new();
    if cfg.staging {
        site_f.push(Finding {
            code: "B1822",
            level: Level::Info,
            msg: "staging build: every page is noindex and robots.txt disallows everything".into(),
            fix: "build with `--env production` to publish".into(),
        });
    }
    Report {
        pages,
        site: site_f,
        min_score: cfg.min_score,
    }
}

impl Report {
    /// Pages scoring under `min_score` (0 disables the threshold).
    pub fn below(&self) -> Vec<&PageReport> {
        self.pages
            .iter()
            .filter(|p| self.min_score > 0 && p.score.is_some_and(|s| s < self.min_score))
            .collect()
    }
    pub fn errors(&self) -> impl Iterator<Item = (&str, &Finding)> {
        self.pages
            .iter()
            .flat_map(|p| p.findings.iter().map(move |f| (p.route.as_str(), f)))
            .filter(|(_, f)| f.level == Level::Error)
            .chain(
                self.site
                    .iter()
                    .filter(|f| f.level == Level::Error)
                    .map(|f| ("", f)),
            )
    }
    /// `brain doctor` passes: no error finding and no page under the threshold.
    pub fn ok(&self) -> bool {
        self.errors().next().is_none() && self.below().is_empty()
    }

    pub fn to_json(&self) -> Json {
        let f = |f: &Finding| {
            Json::obj([
                ("code", Json::str(f.code)),
                ("level", Json::str(f.level.as_str())),
                ("msg", Json::str(&f.msg)),
                ("fix", Json::str(&f.fix)),
            ])
        };
        Json::obj([
            ("ok", Json::Bool(self.ok())),
            ("minScore", Json::Num(f64::from(self.min_score))),
            (
                "pages",
                Json::Arr(
                    self.pages
                        .iter()
                        .map(|p| {
                            Json::obj([
                                ("route", Json::str(&p.route)),
                                ("url", Json::str(&p.url)),
                                ("indexable", Json::Bool(p.indexable)),
                                (
                                    "score",
                                    p.score.map_or(Json::Null, |s| Json::Num(f64::from(s))),
                                ),
                                ("findings", Json::Arr(p.findings.iter().map(f).collect())),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("site", Json::Arr(self.site.iter().map(f).collect())),
        ])
    }

    /// Human/LLM-readable report: one block per page, worst first.
    pub fn text(&self) -> String {
        let mut order: Vec<&PageReport> = self.pages.iter().collect();
        order.sort_by_key(|p| (p.score.unwrap_or(101), p.route.clone()));
        let mut o = format!(
            "SEO audit: {} page(s), min score {}\n",
            self.pages.len(),
            self.min_score
        );
        for f in &self.site {
            o.push_str(&format!(
                "  {} {}: {} · fix: {}\n",
                f.level.as_str(),
                f.code,
                f.msg,
                f.fix
            ));
        }
        for p in order {
            let score = p
                .score
                .map_or("noindex".to_string(), |s| format!("{s}/100"));
            o.push_str(&format!("\n{} — {score}\n", p.route));
            let mut fs: Vec<&Finding> = p.findings.iter().collect();
            fs.sort_by_key(|f| (std::cmp::Reverse(f.level), f.code));
            for f in fs {
                o.push_str(&format!(
                    "  {} {}: {} · fix: {}\n",
                    f.level.as_str(),
                    f.code,
                    f.msg,
                    f.fix
                ));
            }
        }
        o.push_str(if self.ok() {
            "\nok\n"
        } else {
            "\nfailed: fix the errors and the pages under the minimum score\n"
        });
        o
    }
}

/// Fix for a broken `#frag`. `faq~3` is an instance id that codegen assigns (#268): adding it by hand is impossible,
/// so the fix lists the ids of that family the target page does have (#310).
fn anchor_fix<'a>(frag: &str, ids: impl Iterator<Item = &'a str>) -> String {
    let family = frag.rsplit_once('~').filter(|(b, n)| !b.is_empty() && !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()));
    let Some((base, _)) = family else {
        return format!("add id=\"{frag}\" to the target heading or fix the link");
    };
    let mut have: Vec<&str> = ids
        .filter(|x| *x == base || x.strip_prefix(base).and_then(|r| r.strip_prefix('~')).is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit())))
        .collect();
    have.sort_by_key(|x| x.rsplit_once('~').and_then(|(_, n)| n.parse::<u32>().ok()).unwrap_or(1));
    if have.is_empty() {
        return format!("`{frag}` is the id of instance {} of a component, assigned by the compiler; that component is not on the page with id=\"{base}\": fix the link", frag.rsplit_once('~').map(|(_, n)| n).unwrap_or("?"));
    }
    let list: Vec<String> = have.iter().map(|x| format!("`#{x}`")).collect();
    format!("`~N` ids are assigned per component instance and this page has {} of them: link {}", have.len(), list.join(" or "))
}

#[cfg(test)]
mod anchor_fix_tests {
    use super::anchor_fix;

    #[test]
    fn instance_anchor_lists_the_family() {
        let f = anchor_fix("faq~3", ["faq", "faq~2", "intro", "faq-x"].into_iter());
        assert!(f.contains("`#faq` or `#faq~2`") && !f.contains("add id"), "{f}");
        let none = anchor_fix("faq~3", ["intro"].into_iter());
        assert!(none.contains("assigned by the compiler") && !none.contains("add id"), "{none}");
        assert!(anchor_fix("pricing", ["intro"].into_iter()).contains("add id=\"pricing\""));
        assert!(anchor_fix("a~b", ["a"].into_iter()).contains("add id=\"a~b\""));
    }
}
