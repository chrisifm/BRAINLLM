//! Generated SEO files (INTERNALS §32): robots.txt, sitemap.xml (+ index and images), llms.txt, llms-full.txt,
//! manifest.webmanifest and the Google tag bootstrap. Every function is pure and deterministic (invariant 8).

use crate::config::Config;
use crate::html::{Doc, attr};
use brain_lex::Json;

/// What the generators need to know about one page.
#[derive(Clone)]
pub struct Entry<'a> {
    pub route: &'a str,
    /// Absolute URL when `site` is known, else the route.
    pub url: String,
    pub doc: &'a Doc,
    pub lastmod: Option<&'a str>,
    /// Not `noindex`, not canonicalized to another URL, not a staging build.
    pub indexable: bool,
    /// `(hreflang, absolute URL)` of the page's language variants, `x-default` included (empty for one language).
    pub alternates: Vec<(String, String)>,
    /// Indexable videos of the page (video sitemap).
    pub videos: Vec<crate::video::Video>,
}

/// URLs per sitemap file (protocol limit: 50 000 URLs and 50 MB).
pub const SITEMAP_MAX: usize = 50_000;
/// Uncompressed bytes per sitemap file (protocol limit: 50 MB).
pub const SITEMAP_MAX_BYTES: usize = 50 * 1024 * 1024;
/// Characters per `<loc>` (sitemaps.org protocol: under 2 048).
pub const LOC_MAX: usize = 2_048;

/// Percent-encodes what a URL path cannot carry raw (spaces, quotes, non-ASCII); existing `%XX` escapes are kept.
pub fn encode_path(p: &str) -> String {
    let mut o = String::with_capacity(p.len());
    for c in p.chars() {
        if c.is_ascii_alphanumeric() || "/-._~!$&'()*+,;=:@%?#".contains(c) {
            o.push(c);
        } else {
            let mut buf = [0u8; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                o.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    o
}

pub fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// robots.txt. A `noindex` page stays crawlable on purpose: blocking it here would hide its `noindex` from crawlers
/// (Google keeps listing blocked URLs it finds through links). Staging builds block everything.
pub fn robots(cfg: &Config) -> String {
    if cfg.staging {
        return "User-agent: *\nDisallow: /\n".into();
    }
    let mut o = String::from("User-agent: *\nAllow: /\n");
    o.push_str(&crate::social::ai_robots(cfg));
    if let (Some(_), true) = (&cfg.site, cfg.sitemap) {
        o.push_str(&format!("\nSitemap: {}\n", cfg.abs("/sitemap.xml")));
        if cfg.news.is_some() {
            o.push_str(&format!("Sitemap: {}\n", cfg.abs("/sitemap-news.xml")));
        }
    }
    o
}

/// News articles per sitemap (Google News limit).
pub const NEWS_MAX: usize = 1000;

/// `sitemap-news.xml` from the collections listed in `seo.news`: the newest dated, non-fallback entries. Google News
/// only uses articles from the last two days; older ones are ignored, and leaving them keeps the build deterministic.
pub fn news_sitemap(cfg: &Config, cols: &[crate::content::Collection]) -> Option<(String, String)> {
    let (news, _) = (cfg.news.as_ref()?, cfg.site.as_ref()?);
    if cfg.staging || !cfg.sitemap {
        return None;
    }
    let mut items: Vec<&crate::content::Entry> = cols
        .iter()
        .filter(|c| news.collections.contains(&c.name))
        .flat_map(|c| &c.entries)
        .filter(|e| e.date.is_some() && !e.fallback)
        .collect();
    items.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.route().cmp(&b.route())));
    items.truncate(NEWS_MAX);
    let mut o = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\" xmlns:news=\"http://www.google.com/schemas/sitemap-news/0.9\">\n",
    );
    for e in items {
        let lang = e
            .locale
            .as_deref()
            .or(cfg.lang.as_deref())
            .unwrap_or("en")
            .split(['-', '_'])
            .next()
            .unwrap_or("en")
            .to_ascii_lowercase();
        o.push_str(&format!(
            "  <url><loc>{}</loc><news:news><news:publication><news:name>{}</news:name><news:language>{}</news:language></news:publication><news:publication_date>{}</news:publication_date><news:title>{}</news:title></news:news></url>\n",
            xml(&cfg.abs(&encode_path(&e.route()))),
            xml(&news.name),
            xml(&lang),
            xml(e.date.as_deref().unwrap_or_default()),
            xml(&e.title)
        ));
    }
    o.push_str("</urlset>\n");
    Some(("sitemap-news.xml".into(), o))
}

/// Absolute image URLs of a page's main content, for the image sitemap.
fn images(cfg: &Config, e: &Entry) -> Vec<String> {
    let mut v: Vec<String> = e
        .doc
        .images
        .iter()
        .filter(|(_, in_content)| *in_content)
        .filter_map(|(a, _)| attr(a, "src"))
        .filter(|s| !s.starts_with("data:"))
        .map(|s| {
            if s.starts_with("http") {
                s.to_string()
            } else {
                cfg.abs(&abs_path(e.route, s))
            }
        })
        .collect();
    v.dedup();
    v
}

/// `src` relative to the page → site path.
pub fn abs_path(route: &str, href: &str) -> String {
    if href.starts_with('/') || href.contains("://") {
        return href.to_string();
    }
    let base = if route.ends_with('/') {
        route.to_string()
    } else {
        format!("{}/", route.rsplit_once('/').map_or("", |(a, _)| a))
    };
    let mut segs: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for part in href.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                segs.pop();
            }
            p => segs.push(p),
        }
    }
    format!("/{}", segs.join("/"))
}

/// `sitemap.xml` files: one urlset, or a sitemap index plus `sitemap-N.xml` above [`SITEMAP_MAX`] URLs or
/// [`SITEMAP_MAX_BYTES`] serialized bytes. Empty when there is no `site` (sitemaps need absolute URLs) or the build is
/// staging.
pub fn sitemaps(cfg: &Config, entries: &[Entry]) -> Vec<(String, String)> {
    sitemaps_with(cfg, entries, SITEMAP_MAX, SITEMAP_MAX_BYTES).0
}

/// [`sitemaps`] with explicit limits, plus the URLs left out because they are invalid on their own (a `<loc>` over
/// [`LOC_MAX`] characters, or one entry larger than a whole file). Children are filled in URL order, so the split is
/// deterministic; every child respects both limits, counted on the escaped UTF-8 XML.
pub fn sitemaps_with(
    cfg: &Config,
    entries: &[Entry],
    max_urls: usize,
    max_bytes: usize,
) -> (Vec<(String, String)>, Vec<String>) {
    if cfg.site.is_none() || cfg.staging || !cfg.sitemap {
        return (vec![], vec![]);
    }
    let mut list: Vec<&Entry> = entries.iter().filter(|e| e.indexable).collect();
    list.sort_by(|a, b| a.url.cmp(&b.url));
    let line = |e: &Entry| {
        let mut o = format!("  <url><loc>{}</loc>", xml(&e.url));
        if let Some(d) = e.lastmod {
            o.push_str(&format!("<lastmod>{}</lastmod>", xml(d)));
        }
        for img in images(cfg, e) {
            o.push_str(&format!(
                "<image:image><image:loc>{}</image:loc></image:image>",
                xml(&img)
            ));
        }
        for v in &e.videos {
            o.push_str(&crate::video::sitemap_entry(v));
        }
        for (lang, href) in &e.alternates {
            o.push_str(&format!(
                "<xhtml:link rel=\"alternate\" hreflang=\"{}\" href=\"{}\"/>",
                xml(lang),
                xml(href)
            ));
        }
        o.push_str("</url>\n");
        o
    };
    let head = |chunk: &[(&Entry, String)]| {
        let mut o = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"",
        );
        if chunk.iter().any(|(e, _)| !images(cfg, e).is_empty()) {
            o.push_str(" xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"");
        }
        if chunk.iter().any(|(e, _)| !e.alternates.is_empty()) {
            o.push_str(" xmlns:xhtml=\"http://www.w3.org/1999/xhtml\"");
        }
        if chunk.iter().any(|(e, _)| !e.videos.is_empty()) {
            o.push_str(" xmlns:video=\"http://www.google.com/schemas/sitemap-video/1.1\"");
        }
        o.push_str(">\n");
        o
    };
    const FOOT: &str = "</urlset>\n";
    // the widest header (every namespace) is reserved, so a chunk never grows past the limit once its header is known
    const HEAD_MAX: usize = 360;
    let mut rejected = Vec::new();
    let mut chunks: Vec<Vec<(&Entry, String)>> = vec![vec![]];
    let mut size = 0;
    for e in list {
        let l = line(e);
        if e.url.chars().count() > LOC_MAX || l.len() + HEAD_MAX + FOOT.len() > max_bytes {
            rejected.push(e.url.clone());
            continue;
        }
        let cur = chunks.last().unwrap();
        if !cur.is_empty() && (cur.len() >= max_urls || size + l.len() + HEAD_MAX + FOOT.len() > max_bytes) {
            chunks.push(vec![]);
            size = 0;
        }
        size += l.len();
        chunks.last_mut().unwrap().push((e, l));
    }
    let urlset = |chunk: &[(&Entry, String)]| {
        let mut o = head(chunk);
        for (_, l) in chunk {
            o.push_str(l);
        }
        o.push_str(FOOT);
        o
    };
    if chunks.len() == 1 {
        return (vec![("sitemap.xml".into(), urlset(&chunks[0]))], rejected);
    }
    let mut out = Vec::new();
    let mut index = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for (i, chunk) in chunks.iter().enumerate() {
        let name = format!("sitemap-{}.xml", i + 1);
        let last = chunk.iter().filter_map(|(e, _)| e.lastmod).max();
        index.push_str(&format!(
            "  <sitemap><loc>{}</loc>",
            xml(&cfg.abs(&format!("/{name}")))
        ));
        if let Some(d) = last {
            index.push_str(&format!("<lastmod>{}</lastmod>", xml(d)));
        }
        index.push_str("</sitemap>\n");
        out.push((name, urlset(chunk)));
    }
    index.push_str("</sitemapindex>\n");
    out.insert(0, ("sitemap.xml".into(), index));
    (out, rejected)
}

/// Markdown copy of a page for LLMs (llmstxt.org convention, extension-replaced form: `/about` → `about.md`, `/` →
/// `index.md`). Derived from the route, so the page file's extension (`.html`, `.brain`) never leaks into it.
pub fn md_path(route: &str) -> String {
    let r = route.trim_matches('/');
    if r.is_empty() {
        "index.md".into()
    } else {
        format!("{r}.md")
    }
}

/// One page as Markdown: `# heading` + `Source:` + content (shared by llms-full.txt and the per-page copies).
pub fn page_markdown(cfg: &Config, e: &Entry) -> String {
    let md = match &cfg.site {
        Some(_) => e
            .doc
            .markdown
            .replace("](/", &format!("]({}/", cfg.root().unwrap_or_default())),
        None => e.doc.markdown.clone(),
    };
    let (heading, body) = match md.strip_prefix("# ") {
        Some(rest) => {
            let (h, b) = rest.split_once('\n').unwrap_or((rest, ""));
            (h.to_string(), b.trim_start().to_string())
        }
        None => (
            one_line(&e.doc.title.clone().unwrap_or_else(|| e.route.to_string())),
            md,
        ),
    };
    let mut o = format!("# {heading}\nSource: {}\n", e.url);
    if !body.is_empty() {
        o.push_str(&format!("\n{body}\n"));
    }
    o
}

/// Site name: `brain.json` `name`, else the home page title, else the host.
pub fn site_name(cfg: &Config, entries: &[Entry]) -> String {
    cfg.name
        .clone()
        .or_else(|| {
            entries
                .iter()
                .find(|e| e.route == "/")
                .and_then(|e| e.doc.title.clone())
        })
        .or_else(|| {
            cfg.site
                .as_ref()
                .map(|s| s.split("://").nth(1).unwrap_or(s).to_string())
        })
        .unwrap_or_else(|| "Site".into())
}

pub(crate) fn site_description(cfg: &Config, entries: &[Entry]) -> Option<String> {
    cfg.description.clone().or_else(|| {
        entries
            .iter()
            .find(|e| e.route == "/")
            .and_then(|e| e.doc.meta("description").map(str::to_string))
    })
}

/// Section of llms.txt a route belongs to: its first path segment (`/blog/x` → `Blog`), root pages under `Pages`.
fn section(route: &str) -> String {
    let segs: Vec<&str> = route.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        [first, _, ..] => {
            let mut c = first.replace(['-', '_'], " ");
            if let Some(f) = c.get(..1) {
                c = f.to_uppercase() + &c[1..];
            }
            c
        }
        _ => "Pages".into(),
    }
}

/// Heading of a route's llms.txt section: the title of the section's own page (`/cases` titled «Customer stories» for
/// `/cases/x`), which carries the wording, accents and casing the folder name lacks; else [`section`].
pub(crate) fn section_title(route: &str, entries: &[Entry], site: &str) -> String {
    let segs: Vec<&str> = route.split('/').filter(|s| !s.is_empty()).collect();
    if let [first, _, ..] = segs.as_slice() {
        let own = format!("/{first}");
        if let Some(t) = entries
            .iter()
            .find(|e| e.route == own.as_str() || e.route == format!("{own}/"))
            .and_then(|e| e.doc.title.clone())
        {
            let t = one_line(&strip_site(&t, site));
            if !t.is_empty() {
                return t;
            }
        }
    }
    section(route)
}

/// A page title without the site suffix a title template adds (`Pricing · Acme` → `Pricing`): llms.txt already
/// names the site once, so repeating it on every line is noise for the model reading it.
pub fn strip_site(title: &str, site_name: &str) -> String {
    for sep in [" · ", " | ", " — ", " – ", " - ", " :: "] {
        if let Some(t) = title
            .strip_suffix(&format!("{sep}{site_name}"))
            .filter(|t| !t.trim().is_empty())
        {
            return t.trim().to_string();
        }
    }
    title.to_string()
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// llms.txt (llmstxt.org): H1 name, blockquote summary, one `- [title](url): description` per indexable page,
/// grouped by top-level section.
pub fn llms_txt(cfg: &Config, entries: &[Entry]) -> String {
    let mut o = format!("# {}\n", one_line(&site_name(cfg, entries)));
    if let Some(d) = site_description(cfg, entries) {
        o.push_str(&format!("\n> {}\n", one_line(&d)));
    }
    let mut groups: Vec<(String, Vec<&Entry>)> = Vec::new();
    let mut list: Vec<&Entry> = entries.iter().filter(|e| e.indexable).collect();
    list.sort_by(|a, b| (a.route != "/", a.route).cmp(&(b.route != "/", b.route)));
    let site = site_name(cfg, entries);
    for e in list {
        let s = section_title(e.route, entries, &site);
        match groups.iter_mut().find(|(g, _)| *g == s) {
            Some((_, v)) => v.push(e),
            None => groups.push((s, vec![e])),
        }
    }
    groups.sort_by_key(|(g, _)| (g != "Pages", g.clone()));
    for (g, list) in groups {
        o.push_str(&format!("\n## {g}\n\n"));
        for e in list {
            let title = strip_site(&e.doc.title.clone().unwrap_or_else(|| e.route.to_string()), &site);
            let link = if cfg.markdown {
                cfg.abs(&format!("/{}", encode_path(&md_path(e.route))))
            } else {
                e.url.clone()
            };
            o.push_str(&format!(
                "- [{}]({link})",
                one_line(&title).replace(['[', ']'], "")
            ));
            if let Some(d) = e.doc.meta("description") {
                o.push_str(&format!(": {}", one_line(d)));
            }
            o.push('\n');
        }
    }
    o.push_str(&built_with(cfg));
    o
}

/// Closing `## Built with` section of llms.txt and llms-full.txt: the generator, its home and its source.
fn built_with(cfg: &Config) -> String {
    match &cfg.built_with {
        Some((generator, home, source)) => format!(
            "\n## Built with\n\n- [{generator}]({home}): the web compiler for LLMs, written in Rust. Source: {source}\n"
        ),
        None => String::new(),
    }
}

/// llms-full.txt: every indexable page's main content as Markdown, with root-relative links made absolute. Each page
/// is `# <heading>` + `Source: <url>` + content; the heading is the page's leading H1 (removed from the content), else
/// its title.
pub fn llms_full(cfg: &Config, entries: &[Entry]) -> String {
    let mut o = format!("# {}\n", one_line(&site_name(cfg, entries)));
    if let Some(d) = site_description(cfg, entries) {
        o.push_str(&format!("\n> {}\n", one_line(&d)));
    }
    let mut list: Vec<&Entry> = entries.iter().filter(|e| e.indexable).collect();
    list.sort_by(|a, b| (a.route != "/", a.route).cmp(&(b.route != "/", b.route)));
    for e in list {
        o.push_str("\n---\n\n");
        o.push_str(&page_markdown(cfg, e));
    }
    if cfg.built_with.is_some() {
        o.push_str("\n---\n");
        o.push_str(&built_with(cfg));
    }
    o
}

/// Width × height of a PNG from its IHDR chunk.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Some((be(16), be(20)))
}

/// One icon the author put in `public/`: path relative to `public/` and its bytes.
pub struct Icon<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
}

/// `--name: value` of a CSS custom property in the tokens file.
pub fn token(tokens: &str, name: &str) -> Option<String> {
    let key = format!("--{name}:");
    let i = tokens.find(&key)? + key.len();
    let v = tokens[i..].split([';', '}', '\n']).next()?.trim();
    (!v.is_empty() && !v.contains("var(")).then(|| v.to_string())
}

/// manifest.webmanifest: name and colors from `brain.json` (colors default to the `--accent`/`--bg` tokens), icons
/// that already exist in `public/` (PNG sizes read from the file; never generated). Returns the JSON and warnings.
pub fn manifest(
    cfg: &Config,
    entries: &[Entry],
    tokens: &str,
    icons: &[Icon],
) -> (String, Vec<String>) {
    let mut warn = Vec::new();
    let name = site_name(cfg, entries);
    let m = &cfg.manifest_cfg;
    let mut kv: Vec<(String, Json)> = vec![("name".into(), Json::str(&name))];
    kv.push((
        "short_name".into(),
        Json::str(
            m.short_name
                .clone()
                .unwrap_or_else(|| name.chars().take(12).collect::<String>().trim().to_string()),
        ),
    ));
    if let Some(d) = site_description(cfg, entries) {
        kv.push(("description".into(), Json::str(one_line(&d))));
    }
    if let Some(l) = &cfg.lang {
        kv.push(("lang".into(), Json::str(l)));
    }
    kv.push(("start_url".into(), Json::str(&cfg.base)));
    kv.push(("scope".into(), Json::str(&cfg.base)));
    kv.push((
        "display".into(),
        Json::str(m.display.clone().unwrap_or_else(|| "standalone".into())),
    ));
    if let Some(c) = m.background_color.clone().or_else(|| token(tokens, "bg")) {
        kv.push(("background_color".into(), Json::str(c)));
    }
    if let Some(c) = m.theme_color.clone().or_else(|| token(tokens, "accent")) {
        kv.push(("theme_color".into(), Json::str(c)));
    }
    let mut list = Vec::new();
    let mut sizes = Vec::new();
    let mut sorted: Vec<&Icon> = icons.iter().collect();
    sorted.sort_by_key(|i| i.path);
    for ic in sorted {
        let src = format!("/{}", ic.path.trim_start_matches('/'));
        if ic.path.ends_with(".svg") {
            list.push(Json::obj([
                ("src", Json::str(src)),
                ("sizes", Json::str("any")),
                ("type", Json::str("image/svg+xml")),
            ]));
        } else if ic.path.ends_with(".png") {
            match png_size(ic.bytes) {
                Some((w, h)) if w == h => {
                    sizes.push(w);
                    list.push(Json::obj([("src", Json::str(src)), ("sizes", Json::str(format!("{w}x{h}"))), ("type", Json::str("image/png"))]));
                }
                Some((w, h)) => warn.push(format!("public/{}: icon is {w}x{h}, manifest icons must be square; fix: export it square", ic.path)),
                None => warn.push(format!("public/{}: not a valid PNG; fix: re-export the icon as PNG", ic.path)),
            }
        }
    }
    for need in [192, 512] {
        if !sizes.contains(&need) {
            warn.push(format!("manifest has no {need}x{need} PNG icon (needed to install the app); fix: add public/icon-{need}.png"));
        }
    }
    kv.push(("icons".into(), Json::Arr(list)));
    (Json::Obj(kv).stringify(), warn)
}

/// FNV-1a 64 → 8 base36 chars: stable content hash for generated asset names.
pub fn hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let mut o = String::new();
    for _ in 0..8 {
        o.push(char::from_digit((h % 36) as u32, 36).unwrap());
        h /= 36;
    }
    o
}

/// Google tags (INTERNALS §32): a same-origin bootstrap that sets Consent Mode v2 to denied before anything loads
/// (no inline script, so no nonce or hash is needed), then the async loader. The runtime's
/// `consent('analytics')` calls the global `gtag('consent','update',…)` this file defines.
pub struct GoogleTags {
    /// `<head>` tags, in order.
    pub head: String,
    /// `(dist path, content)` of the bootstrap script.
    pub file: Option<(String, String)>,
    /// `(CSP directive, source)` pairs the server config must allow.
    pub csp: Vec<(&'static str, &'static str)>,
}

pub fn google_tags(cfg: &Config) -> GoogleTags {
    let g = &cfg.google;
    let mut head = String::new();
    if let Some(v) = &g.site_verification {
        head.push_str(&format!(
            "<meta name=\"google-site-verification\" content=\"{v}\">"
        ));
    }
    if g.analytics.is_none() && g.tag_manager.is_none() {
        return GoogleTags {
            head,
            file: None,
            csp: vec![],
        };
    }
    let mut js = String::from(
        "window.dataLayer=window.dataLayer||[];function gtag(){dataLayer.push(arguments)}\
gtag('consent','default',{ad_storage:'denied',ad_user_data:'denied',ad_personalization:'denied',analytics_storage:'denied',wait_for_update:500});",
    );
    if let Some(id) = &g.analytics {
        js.push_str(&format!("gtag('js',new Date());gtag('config','{id}');"));
    }
    if g.tag_manager.is_some() {
        js.push_str("dataLayer.push({'gtm.start':new Date().getTime(),event:'gtm.js'});");
    }
    js.push('\n');
    let path = format!("_b/g.{}.js", hash(&js));
    head.push_str(&format!("<script src=\"/{path}\"></script>"));
    if let Some(id) = &g.analytics {
        head.push_str(&format!(
            "<script async src=\"https://www.googletagmanager.com/gtag/js?id={id}\"></script>"
        ));
    }
    if let Some(id) = &g.tag_manager {
        head.push_str(&format!(
            "<script async src=\"https://www.googletagmanager.com/gtm.js?id={id}\"></script>"
        ));
    }
    let csp = vec![
        ("script-src", "https://www.googletagmanager.com"),
        ("connect-src", "https://*.google-analytics.com"),
        ("connect-src", "https://*.analytics.google.com"),
        ("connect-src", "https://*.googletagmanager.com"),
        ("img-src", "https://*.google-analytics.com"),
        ("img-src", "https://*.googletagmanager.com"),
    ];
    GoogleTags {
        head,
        file: Some((path, js)),
        csp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::parse;

    fn docs() -> Vec<(String, Doc)> {
        [
            ("/", "<html><head><title>Acme</title><meta name=description content='Tools for teams'></head><body><main><h1>Acme</h1><p>Hi <a href='/blog/a'>post</a></p><img src='hero.png' alt='Hero'></main></body></html>"),
            ("/blog/a", "<html><head><title>First post</title><meta name=description content='The first one'></head><body><main><h1>First post</h1><p>Text</p></main></body></html>"),
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
                lastmod: (r == "/blog/a").then_some("2026-10-01"),
                indexable: r != "/admin",
                alternates: vec![],
                videos: vec![],
            })
            .collect()
    }

    #[test]
    fn robots_and_sitemap() {
        let cfg = Config {
            site: Some("https://acme.dev".into()),
            ..Config::default()
        };
        let d = docs();
        let e = entries(&cfg, &d);
        assert_eq!(
            robots(&cfg),
            "User-agent: *\nAllow: /\nContent-Signal: search=yes, ai-input=yes, ai-train=yes\n\nSitemap: https://acme.dev/sitemap.xml\n"
        );
        let sm = sitemaps(&cfg, &e);
        assert_eq!(sm.len(), 1);
        let x = &sm[0].1;
        assert!(x.contains("<url><loc>https://acme.dev/</loc><image:image><image:loc>https://acme.dev/hero.png</image:loc></image:image></url>"));
        assert!(x.contains("<loc>https://acme.dev/blog/a</loc><lastmod>2026-10-01</lastmod>"));
        assert!(!x.contains("admin"));
        let staging = Config {
            staging: true,
            ..cfg.clone()
        };
        assert_eq!(robots(&staging), "User-agent: *\nDisallow: /\n");
        assert!(sitemaps(&staging, &e).is_empty());
        assert!(sitemaps(&Config::default(), &e).is_empty());
    }

    #[test]
    fn sitemaps_split_by_serialized_bytes_and_reject_oversized_urls() {
        // S01: the protocol limit is 50 000 URLs *and* 50 MB; a child must respect both, counting escaped UTF-8 bytes
        let cfg = Config {
            site: Some("https://acme.dev".into()),
            ..Config::default()
        };
        let d = parse("<html><head><title>T</title></head><body><main><h1>T</h1></main></body></html>");
        let long = "€&".repeat(60); // 3 UTF-8 bytes + 5 escaped bytes per pair
        let mut routes: Vec<String> = (0..7).map(|i| format!("/p{i}-{long}")).collect();
        routes.push(format!("/huge-{}", "x".repeat(2100))); // over the 2 048-character <loc> limit
        let entries: Vec<Entry> = routes
            .iter()
            .map(|r| Entry {
                route: r,
                url: cfg.abs(r),
                doc: &d,
                lastmod: None,
                indexable: true,
                alternates: vec![],
                videos: vec![],
            })
            .collect();
        let (sm, rejected) = sitemaps_with(&cfg, &entries, 3, 1_100);
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert!(rejected[0].starts_with("https://acme.dev/huge-"));
        assert_eq!(sm[0].0, "sitemap.xml");
        assert!(sm[0].1.contains("<sitemapindex"));
        let children = &sm[1..];
        assert!(children.len() > 3, "the byte limit must split before the URL limit: {}", children.len());
        let mut seen = 0;
        for (name, body) in children {
            assert!(body.len() <= 1_100, "{name} is {} bytes", body.len());
            let n = body.matches("<url>").count();
            assert!((1..=3).contains(&n), "{name} has {n} URLs");
            assert!(sm[0].1.contains(&format!("https://acme.dev/{name}")));
            seen += n;
        }
        assert_eq!(seen, 7);
        assert!(children.iter().all(|(_, b)| !b.contains("/huge-")));
        assert_eq!(sitemaps_with(&cfg, &entries, 3, 1_100), (sm.clone(), rejected.clone()), "deterministic");
        // the defaults keep an ordinary site in one file
        let (one, none) = sitemaps_with(&cfg, &entries[..2], SITEMAP_MAX, SITEMAP_MAX_BYTES);
        assert_eq!((one.len(), none.len()), (1, 0));
    }

    #[test]
    fn llms_sections_take_the_section_page_title() {
        let cfg = Config {
            site: Some("https://acme.dev".into()),
            ..Config::default()
        };
        let d: Vec<(String, Doc)> = [
            ("/", "<html><head><title>Acme</title></head><body><main><h1>Acme</h1></main></body></html>"),
            ("/cases", "<html><head><title>Customer stories · Acme</title></head><body><main><h1>Customer stories</h1></main></body></html>"),
            ("/cases/acme-labs", "<html><head><title>Acme Labs · Acme</title></head><body><main><h1>Acme Labs</h1></main></body></html>"),
            ("/job-board/a", "<html><head><title>A</title></head><body><main><h1>A</h1></main></body></html>"),
        ]
        .iter()
        .map(|(r, h)| (r.to_string(), parse(h)))
        .collect();
        let e = entries(&cfg, &d);
        let txt = llms_txt(&cfg, &e);
        assert!(txt.contains("\n## Customer stories\n\n- [Acme Labs](https://acme.dev/cases/acme-labs.md)"), "{txt}");
        assert!(!txt.contains("## Cases"), "{txt}");
        // without a page of its own, the folder name stays the heading
        assert!(txt.contains("\n## Job board\n"), "{txt}");
    }

    #[test]
    fn the_not_found_page_is_never_indexable() {
        let cfg = Config {
            site: Some("https://acme.dev".into()),
            ..Config::default()
        };
        let d = parse("<html><head><title>Not found</title><meta name=description content='Lost'></head><body><main><h1>Lost</h1></main></body></html>");
        assert!(!crate::audit::indexable(&cfg, "*", &d));
        assert!(!crate::audit::indexable(&cfg, "!error:views/_error.brain", &d));
        assert!(crate::audit::indexable(&cfg, "/lost", &d));
    }

    #[test]
    fn llms_files() {
        let cfg = Config {
            site: Some("https://acme.dev".into()),
            ..Config::default()
        };
        let d = docs();
        let e = entries(&cfg, &d);
        assert_eq!(
            llms_txt(&cfg, &e),
            "# Acme\n\n> Tools for teams\n\n## Pages\n\n- [Acme](https://acme.dev/index.md): Tools for teams\n\n## Blog\n\n- [First post](https://acme.dev/blog/a.md): The first one\n"
        );
        let full = llms_full(&cfg, &e);
        assert!(
            full.contains(
                "\n---\n\n# Acme\nSource: https://acme.dev/\n\nHi [post](https://acme.dev/blog/a)"
            ),
            "{full}"
        );
        assert!(full.contains("Source: https://acme.dev/blog/a") && !full.contains("Admin"));
    }

    #[test]
    fn llms_files_close_with_what_built_the_site() {
        let cfg = Config {
            site: Some("https://acme.dev".into()),
            built_with: Some(("BRAINLLM 0.3.0".into(), "https://brain.example/".into(), "https://git.example/brain".into())),
            ..Config::default()
        };
        let d = docs();
        let e = entries(&cfg, &d);
        let tail = "\n## Built with\n\n- [BRAINLLM 0.3.0](https://brain.example/): the web compiler for LLMs, written in Rust. Source: https://git.example/brain\n";
        assert!(llms_txt(&cfg, &e).ends_with(tail));
        assert!(llms_full(&cfg, &e).ends_with(&format!("\n---\n{tail}")));
    }

    #[test]
    fn manifest_from_tokens_and_icons() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(192u32.to_be_bytes());
        png.extend(192u32.to_be_bytes());
        let cfg = Config {
            name: Some("Acme Team Tools".into()),
            lang: Some("en".into()),
            ..Config::default()
        };
        let d = docs();
        let (m, warn) = manifest(
            &cfg,
            &entries(&cfg, &d),
            ":root { --bg: #f5f5f7; --accent: #0071e3; }",
            &[
                Icon {
                    path: "icon-192.png",
                    bytes: &png,
                },
                Icon {
                    path: "logo.svg",
                    bytes: b"<svg/>",
                },
            ],
        );
        let j = Json::parse(&m).unwrap();
        assert_eq!(
            j.get("short_name").and_then(Json::as_str),
            Some("Acme Team To")
        );
        assert_eq!(j.get("theme_color").and_then(Json::as_str), Some("#0071e3"));
        assert_eq!(
            j.get("background_color").and_then(Json::as_str),
            Some("#f5f5f7")
        );
        assert!(m.contains("\"sizes\":\"192x192\"") && m.contains("\"sizes\":\"any\""));
        assert_eq!(warn.len(), 1);
        assert!(warn[0].contains("512x512"));
    }

    #[test]
    fn google_consent_first() {
        let cfg = Config {
            google: crate::config::Google {
                analytics: Some("G-AB12".into()),
                tag_manager: None,
                site_verification: Some("tok".into()),
            },
            ..Config::default()
        };
        let g = google_tags(&cfg);
        let (path, js) = g.file.unwrap();
        assert!(
            js.starts_with("window.dataLayer=")
                && js.contains("analytics_storage:'denied'")
                && js.contains("gtag('config','G-AB12')")
        );
        let boot = g.head.find(&format!("src=\"/{path}\"")).unwrap();
        let loader = g.head.find("gtag/js?id=G-AB12").unwrap();
        assert!(
            g.head
                .starts_with("<meta name=\"google-site-verification\" content=\"tok\">")
                && boot < loader
        );
        assert!(
            g.csp
                .contains(&("script-src", "https://www.googletagmanager.com"))
        );
        assert!(google_tags(&Config::default()).head.is_empty());
    }

    #[test]
    fn site_suffix() {
        assert_eq!(strip_site("Pricing · Acme", "Acme"), "Pricing");
        assert_eq!(strip_site("Acme", "Acme"), "Acme");
        assert_eq!(strip_site("Docs | Acme", "Acme"), "Docs");
        assert_eq!(strip_site("Pricing · Other", "Acme"), "Pricing · Other");
    }

    #[test]
    fn paths_and_hash() {
        assert_eq!(abs_path("/blog/a", "img.png"), "/blog/img.png");
        assert_eq!(abs_path("/blog/", "../x.png"), "/x.png");
        assert_eq!(abs_path("/", "hero.png"), "/hero.png");
        assert_eq!(hash("a"), hash("a"));
        assert_ne!(hash("a"), hash("b"));
        assert_eq!(token(":root{--bg:#fff;--x:var(--bg)}", "x"), None);
    }
}
