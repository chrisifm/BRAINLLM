//! Social and AI discovery (requested by Chris, 2026-10-13): complete Open Graph (Facebook, LinkedIn, WhatsApp,
//! Instagram DMs, Slack, Discord, iMessage all read it), X cards, `fb:app_id`, `fediverse:creator`, `rel="me"` profile
//! links, `og:type=article` + `article:*` for content entries, image dimensions read from the file, the robots
//! `max-image-preview:large` hint, and the AI crawler policy for robots.txt (with Cloudflare's `Content-Signal`).
//! Only tags the page does not already have are added; the author's own tags always win.

use crate::config::{Ai, Config};
use crate::html::Doc;
use crate::jsonld::Article;

// AI crawler tokens change often: re-check every 30 days (last verified 2026-10-13) against the operators' pages:
//   OpenAI      https://platform.openai.com/docs/bots
//   Anthropic   https://support.anthropic.com/en/articles/8896518 (ClaudeBot, Claude-SearchBot, Claude-User)
//   Perplexity  https://docs.perplexity.ai/guides/bots
//   Google      https://developers.google.com/search/docs/crawling-indexing/google-common-crawlers (Google-Extended)
//   Apple       https://support.apple.com/en-us/119829 (Applebot-Extended)
//   Meta        https://developers.facebook.com/docs/sharing/webmasters/web-crawlers
//   Common Crawl https://commoncrawl.org/ccbot
//   Content-Signal syntax: https://contentsignals.org and https://blog.cloudflare.com/content-signals-policy/

/// AI crawlers by role (robots.txt tokens).
pub const AI_TRAINING: &[&str] = &[
    "GPTBot",
    "ClaudeBot",
    "anthropic-ai",
    "Google-Extended",
    "Applebot-Extended",
    "CCBot",
    "Bytespider",
    "Meta-ExternalAgent",
];
/// AI search indexes and answer engines (they cite and link the page).
pub const AI_SEARCH: &[&str] = &["OAI-SearchBot", "Claude-SearchBot", "PerplexityBot"];
/// Fetchers acting on a user's request in a chat (several ignore robots.txt by design).
pub const AI_USER: &[&str] = &["ChatGPT-User", "Claude-User", "Perplexity-User"];

/// robots.txt groups for the AI policy, appended after the `User-agent: *` group.
pub fn ai_robots(cfg: &Config) -> String {
    let (blocked, signal): (Vec<&str>, &str) = match cfg.ai {
        Ai::Allow => (vec![], "search=yes, ai-input=yes, ai-train=yes"),
        Ai::SearchOnly => (
            AI_TRAINING.to_vec(),
            "search=yes, ai-input=yes, ai-train=no",
        ),
        Ai::Block => (
            AI_TRAINING
                .iter()
                .chain(AI_SEARCH)
                .chain(AI_USER)
                .copied()
                .collect(),
            "search=yes, ai-input=no, ai-train=no",
        ),
    };
    let mut o = format!("Content-Signal: {signal}\n");
    if !blocked.is_empty() {
        o.push('\n');
        for b in &blocked {
            o.push_str(&format!("User-agent: {b}\n"));
        }
        o.push_str("Disallow: /\n");
    }
    o
}

/// `en` → `en_US`, `es-MX` → `es_MX` (Open Graph wants language_TERRITORY).
pub fn og_locale(lang: &str) -> String {
    let mut parts = lang.split(['-', '_']);
    let l = parts.next().unwrap_or("en").to_ascii_lowercase();
    if let Some(t) = parts.find(|p| p.len() == 2) {
        return format!("{l}_{}", t.to_ascii_uppercase());
    }
    let t = match l.as_str() {
        "en" => "US",
        "pt" => "BR",
        "zh" => "CN",
        "ja" => "JP",
        "ko" => "KR",
        "sv" => "SE",
        "da" => "DK",
        "cs" => "CZ",
        "el" => "GR",
        "he" => "IL",
        "hi" => "IN",
        "uk" => "UA",
        "vi" => "VN",
        _ => "",
    };
    if t.is_empty() {
        format!("{l}_{}", l.to_ascii_uppercase())
    } else {
        format!("{l}_{t}")
    }
}

/// Width × height of a PNG, JPEG, GIF or WebP from its header.
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    if let Some(s) = crate::files::png_size(b) {
        return Some(s);
    }
    if b.len() >= 10 && (b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        return Some((
            u32::from(u16::from_le_bytes([b[6], b[7]])),
            u32::from(u16::from_le_bytes([b[8], b[9]])),
        ));
    }
    if b.len() >= 30 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return match &b[12..16] {
            b"VP8X" => Some((
                1 + (u32::from(b[24]) | u32::from(b[25]) << 8 | u32::from(b[26]) << 16),
                1 + (u32::from(b[27]) | u32::from(b[28]) << 8 | u32::from(b[29]) << 16),
            )),
            b"VP8 " => Some((
                u32::from(u16::from_le_bytes([b[26], b[27]]) & 0x3fff),
                u32::from(u16::from_le_bytes([b[28], b[29]]) & 0x3fff),
            )),
            b"VP8L" => {
                let v = u32::from_le_bytes([b[21], b[22], b[23], b[24]]);
                Some(((v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1))
            }
            _ => None,
        };
    }
    if b.len() > 4 && b[0] == 0xFF && b[1] == 0xD8 {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = b[i + 1];
            let len = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
            // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC)
            if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
                return Some((
                    u32::from(u16::from_be_bytes([b[i + 7], b[i + 8]])),
                    u32::from(u16::from_be_bytes([b[i + 5], b[i + 6]])),
                ));
            }
            i += 2 + len;
        }
    }
    None
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Head tags for one page: `(start, end)`. `start` goes first in `<head>` (defaults the author can override);
/// `end` goes last (corrections that must win over the generic `og:type=website`).
pub struct Facts<'a> {
    pub site_name: &'a str,
    pub home: bool,
    pub indexable: bool,
    pub article: Option<&'a Article>,
    /// Collection name for `article:section`.
    pub section: Option<&'a str>,
    /// Dimensions of the page's `og:image` when it is a local file.
    pub image_dims: Option<(u32, u32)>,
    /// Locale of the page by its route (multilingual sites).
    pub locale: Option<&'a str>,
    /// Other locales the page exists in (`og:locale:alternate`).
    pub locale_alternates: &'a [String],
}

pub fn head(cfg: &Config, doc: &Doc, f: &Facts) -> (String, String) {
    let has = |k: &str| doc.meta(k).is_some();
    let mut s = String::new();
    let mut meta = |kind: &str, k: &str, v: &str| {
        s.push_str(&format!("<meta {kind}=\"{k}\" content=\"{}\">", esc(v)))
    };
    if !has("og:site_name") {
        meta("property", "og:site_name", f.site_name);
    }
    if let (false, Some(l)) = (
        has("og:locale"),
        doc.lang.as_deref().or(f.locale).or(cfg.lang.as_deref()),
    ) {
        meta("property", "og:locale", &og_locale(l));
    }
    if !has("og:locale:alternate") {
        for l in f.locale_alternates {
            meta("property", "og:locale:alternate", &og_locale(l));
        }
    }
    if doc.meta("og:image").is_some() {
        if !has("og:image:alt") {
            meta(
                "property",
                "og:image:alt",
                doc.meta("og:title")
                    .or(doc.title.as_deref())
                    .unwrap_or(f.site_name),
            );
        }
        if let (Some((w, h)), false) = (f.image_dims, has("og:image:width")) {
            meta("property", "og:image:width", &w.to_string());
            meta("property", "og:image:height", &h.to_string());
        }
    }
    if !has("twitter:card") {
        meta(
            "name",
            "twitter:card",
            if doc.meta("og:image").is_some() {
                "summary_large_image"
            } else {
                "summary"
            },
        );
    }
    if let (Some(x), false) = (&cfg.social.x, has("twitter:site")) {
        meta("name", "twitter:site", x);
    }
    if let (Some(fv), false) = (&cfg.social.fediverse, has("fediverse:creator")) {
        meta("name", "fediverse:creator", fv);
    }
    if let (Some(id), false) = (&cfg.social.facebook_app_id, has("fb:app_id")) {
        meta("property", "fb:app_id", id);
    }
    if f.indexable && !has("robots") {
        meta(
            "name",
            "robots",
            "max-image-preview:large, max-snippet:-1, max-video-preview:-1",
        );
    }
    if f.home {
        for (_, url) in &cfg.social.profiles {
            s.push_str(&format!("<link rel=\"me\" href=\"{}\">", esc(url)));
        }
    }
    let mut e = String::new();
    if let Some(a) = f.article {
        e.push_str("<meta property=\"og:type\" content=\"article\">");
        let mut am = |k: &str, v: &str| {
            e.push_str(&format!("<meta property=\"{k}\" content=\"{}\">", esc(v)))
        };
        if let Some(d) = &a.published {
            am("article:published_time", d);
        }
        if let Some(d) = a.modified.as_ref().or(a.published.as_ref()) {
            am("article:modified_time", d);
        }
        if let Some(au) = &a.author {
            am("article:author", au);
        }
        if let Some(sec) = f.section {
            am("article:section", sec);
        }
        for t in &a.tags {
            am("article:tag", t);
        }
    }
    (s, e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Social;
    use crate::html::parse;

    #[test]
    fn robots_policy() {
        let mut cfg = Config::default();
        assert_eq!(
            ai_robots(&cfg),
            "Content-Signal: search=yes, ai-input=yes, ai-train=yes\n"
        );
        cfg.ai = Ai::SearchOnly;
        let r = ai_robots(&cfg);
        assert!(
            r.starts_with(
                "Content-Signal: search=yes, ai-input=yes, ai-train=no\n\nUser-agent: GPTBot\n"
            ) && r.ends_with("User-agent: Meta-ExternalAgent\nDisallow: /\n")
        );
        assert!(!r.contains("OAI-SearchBot"), "AI search stays allowed");
        cfg.ai = Ai::Block;
        assert!(
            ai_robots(&cfg).contains("User-agent: Claude-SearchBot\n")
                && ai_robots(&cfg).contains("ai-input=no")
        );
    }

    #[test]
    fn locales_and_images() {
        assert_eq!(
            (
                og_locale("en"),
                og_locale("es-MX"),
                og_locale("es"),
                og_locale("pt")
            ),
            (
                "en_US".into(),
                "es_MX".into(),
                "es_ES".into(),
                "pt_BR".into()
            )
        );
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(1200u32.to_be_bytes());
        png.extend(630u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((1200, 630)));
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 17, 8, 0x02, 0x76, 0x04, 0xB0, 3, 0,
            0, 0, 0,
        ];
        assert_eq!(image_size(&jpeg), Some((1200, 630)));
        assert_eq!(image_size(b"GIF89a\xb0\x04\x76\x02"), Some((1200, 630)));
        assert_eq!(image_size(b"nope"), None);
    }

    #[test]
    fn head_tags_fill_only_what_is_missing() {
        let cfg = Config {
            lang: Some("en".into()),
            social: Social {
                x: Some("@acme".into()),
                profiles: vec![("instagram".into(), "https://www.instagram.com/acme".into())],
                facebook_app_id: Some("123".into()),
                fediverse: Some("@acme@mastodon.social".into()),
            },
            ..Config::default()
        };
        let doc = parse(
            "<html lang=en><head><title>Pricing — Acme</title><meta property=og:image content=https://acme.dev/og.png><meta name=twitter:card content=summary></head></html>",
        );
        let (start, end) = head(
            &cfg,
            &doc,
            &Facts {
                site_name: "Acme",
                home: true,
                indexable: true,
                article: None,
                section: None,
                image_dims: Some((1200, 630)),
                locale: None,
                locale_alternates: &[],
            },
        );
        for t in [
            "<meta property=\"og:site_name\" content=\"Acme\">",
            "<meta property=\"og:locale\" content=\"en_US\">",
            "<meta property=\"og:image:alt\" content=\"Pricing — Acme\">",
            "<meta property=\"og:image:width\" content=\"1200\"><meta property=\"og:image:height\" content=\"630\">",
            "<meta name=\"twitter:site\" content=\"@acme\">",
            "<meta name=\"fediverse:creator\" content=\"@acme@mastodon.social\">",
            "<meta property=\"fb:app_id\" content=\"123\">",
            "<meta name=\"robots\" content=\"max-image-preview:large, max-snippet:-1, max-video-preview:-1\">",
            "<link rel=\"me\" href=\"https://www.instagram.com/acme\">",
        ] {
            assert!(start.contains(t), "missing {t} in {start}");
        }
        assert!(!start.contains("twitter:card"), "the author's card wins");
        assert!(end.is_empty());
        let a = Article {
            headline: "Offline first".into(),
            published: Some("2026-10-01".into()),
            author: Some("Ada".into()),
            tags: vec!["sync".into()],
            ..Article::default()
        };
        let (_, end) = head(
            &cfg,
            &doc,
            &Facts {
                site_name: "Acme",
                home: false,
                indexable: false,
                article: Some(&a),
                section: Some("blog"),
                image_dims: None,
                locale: None,
                locale_alternates: &[],
            },
        );
        assert_eq!(
            end,
            "<meta property=\"og:type\" content=\"article\"><meta property=\"article:published_time\" content=\"2026-10-01\"><meta property=\"article:modified_time\" content=\"2026-10-01\"><meta property=\"article:author\" content=\"Ada\"><meta property=\"article:section\" content=\"blog\"><meta property=\"article:tag\" content=\"sync\">"
        );
    }
}
