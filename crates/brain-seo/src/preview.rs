//! `brain seo preview <route>` (INTERNALS §33): what a search result and a social card will show for a page, cut the
//! way they cut (by pixel width), plus the page's structured data and its audit findings. Text for the LLM, JSON for
//! tools and devtools.

use crate::audit::{DESC_PX, PageReport, TITLE_PX, px};
use crate::html::Doc;
use brain_lex::Json;

/// `s` cut to `max` px at `size` px Arial, with an ellipsis when cut.
pub fn cut_px(s: &str, size: f32, max: f32) -> (String, bool) {
    if px(s, size) <= max {
        return (s.to_string(), false);
    }
    let ell = px("\u{2026}", size);
    let mut out = String::new();
    for c in s.chars() {
        let mut next = out.clone();
        next.push(c);
        if px(&next, size) + ell > max {
            break;
        }
        out = next;
    }
    let trimmed = match out.rfind(' ') {
        Some(i) if i > out.len() / 2 => out[..i].to_string(),
        _ => out,
    };
    (
        format!(
            "{}\u{2026}",
            trimmed.trim_end_matches([',', '.', ';', ':', ' '])
        ),
        true,
    )
}

/// `acme.dev › blog › offline-first` (how results show the URL).
pub fn breadcrumb_url(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{203a} ")
}

pub struct Preview {
    pub json: Json,
    pub text: String,
}

pub fn preview(doc: &Doc, url: &str, report: Option<&PageReport>) -> Preview {
    let title = doc.title.clone().unwrap_or_default();
    let (t_cut, t_was) = cut_px(&title, 20.0, TITLE_PX);
    let desc = doc.meta("description").map(str::to_string);
    let snippet_src = desc.clone().unwrap_or_else(|| doc.first_paragraph.clone());
    let (d_cut, d_was) = cut_px(&snippet_src, 14.0, DESC_PX);
    let og = |k: &str| doc.meta(k).map(str::to_string);
    let (og_title, og_desc, og_image, og_url) = (
        og("og:title").or(Some(title.clone())),
        og("og:description").or(desc.clone()),
        og("og:image"),
        og("og:url"),
    );
    let card = og("twitter:card").unwrap_or_else(|| {
        if og_image.is_some() {
            "summary_large_image".into()
        } else {
            "summary".into()
        }
    });
    let ld: Vec<Json> = doc
        .jsonld
        .iter()
        .filter_map(|b| Json::parse(b.trim()).ok())
        .collect();
    let types = crate::jsonld::declared_types(doc);

    let mut text = String::from("Search result (desktop)\n");
    text.push_str(&format!("  {}\n", breadcrumb_url(url)));
    text.push_str(&format!(
        "  {t_cut}{}\n",
        if t_was {
            format!(
                "   [title cut: {:.0} of {TITLE_PX:.0} px]",
                px(&title, 20.0)
            )
        } else {
            String::new()
        }
    ));
    match &desc {
        Some(_) => text.push_str(&format!(
            "  {d_cut}{}\n",
            if d_was { "   [description cut]" } else { "" }
        )),
        None => text.push_str(&format!(
            "  {d_cut}   [no description: the engine picks text from the page]\n"
        )),
    }
    text.push_str("\nSocial cards\n");
    text.push_str(
        "  Facebook, LinkedIn, WhatsApp, Slack, Discord, iMessage, Instagram DMs (Open Graph):\n",
    );
    text.push_str(&format!(
        "    title: {}\n    description: {}\n    image: {}\n    site: {}\n    url: {}\n",
        og_title.as_deref().unwrap_or("-"),
        og_desc.as_deref().unwrap_or("-"),
        og_image
            .as_deref()
            .unwrap_or("- (none: the card shows no picture)"),
        og("og:site_name").as_deref().unwrap_or("-"),
        og_url.as_deref().unwrap_or(url)
    ));
    text.push_str(&format!(
        "  X: card {card}{} (title, description and image fall back to Open Graph)\n",
        og("twitter:site").map_or(String::new(), |s| format!(", site {s}"))
    ));
    text.push_str(&format!(
        "\nStructured data: {}\n",
        if types.is_empty() {
            "none".to_string()
        } else {
            types.join(", ")
        }
    ));
    if let Some(r) = report {
        text.push_str(&format!(
            "\nAudit: {}\n",
            r.score.map_or(
                "not indexable (noindex or canonicalized)".to_string(),
                |s| format!("{s}/100")
            )
        ));
        for f in &r.findings {
            text.push_str(&format!(
                "  {} {}: {} · fix: {}\n",
                f.level.as_str(),
                f.code,
                f.msg,
                f.fix
            ));
        }
    }
    let opt = |v: &Option<String>| v.as_ref().map_or(Json::Null, Json::str);
    let json = Json::obj([
        ("url", Json::str(url)),
        (
            "serp",
            Json::obj([
                ("url", Json::str(breadcrumb_url(url))),
                ("title", Json::str(&t_cut)),
                ("titleCut", Json::Bool(t_was)),
                ("titlePx", Json::Num(f64::from(px(&title, 20.0).round()))),
                ("snippet", Json::str(&d_cut)),
                ("snippetCut", Json::Bool(d_was)),
                ("hasDescription", Json::Bool(desc.is_some())),
            ]),
        ),
        (
            "social",
            Json::obj([
                ("card", Json::str(&card)),
                ("title", opt(&og_title)),
                ("description", opt(&og_desc)),
                ("image", opt(&og_image)),
                ("url", Json::str(og_url.as_deref().unwrap_or(url))),
            ]),
        ),
        ("jsonld", Json::Arr(ld)),
        (
            "score",
            report
                .and_then(|r| r.score)
                .map_or(Json::Null, |s| Json::Num(f64::from(s))),
        ),
    ]);
    Preview { json, text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::parse;

    #[test]
    fn cuts_by_pixel_width() {
        let (t, cut) = cut_px(
            "A very long title that keeps going well past the width a desktop result can show at all",
            20.0,
            TITLE_PX,
        );
        assert!(
            cut && t.ends_with('\u{2026}') && px(&t, 20.0) <= TITLE_PX,
            "{t}"
        );
        assert_eq!(cut_px("Short", 20.0, TITLE_PX), ("Short".into(), false));
        assert_eq!(
            breadcrumb_url("https://acme.dev/blog/offline-first"),
            "acme.dev \u{203a} blog \u{203a} offline-first"
        );
    }

    #[test]
    fn page_preview() {
        let d = parse(
            "<html><head><title>Pricing — Acme</title><meta property=og:image content=https://acme.dev/og.png><script type=application/ld+json>{\"@type\":\"WebPage\"}</script></head><body><main><p>Free for small teams.</p></main></body></html>",
        );
        let p = preview(&d, "https://acme.dev/pricing", None);
        assert!(p.text.contains("acme.dev \u{203a} pricing\n  Pricing — Acme\n  Free for small teams.   [no description"), "{}", p.text);
        assert!(
            p.text.contains("X: card summary_large_image")
                && p.text.contains("Structured data: WebPage"),
            "{}",
            p.text
        );
        assert_eq!(
            p.json.get("serp").and_then(|s| s.get("hasDescription")),
            Some(&Json::Bool(false))
        );
    }
}
