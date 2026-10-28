//! Videos (INTERNALS §33): every `<video>` in a page's main content becomes a `VideoObject` (JSON-LD) and a
//! `<video:video>` sitemap entry when it has what search engines require: a poster (thumbnail), a name (`title`,
//! `aria-label`, else the page's H1) and an upload date (`data-upload-date`, else the article's or page's date).
//! The duration comes from `data-duration` (`PT1M30S`) or, for a local MP4, from its `mvhd` box.

use crate::config::Config;
use brain_lex::Json;
use std::io::{Read, Seek, SeekFrom};

/// One video as the search engines see it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Video {
    pub name: String,
    pub description: String,
    /// Absolute URLs.
    pub thumbnail: String,
    pub content: Option<String>,
    /// `YYYY-MM-DD` or ISO 8601.
    pub upload: String,
    pub seconds: Option<u64>,
}

/// `PT1H2M3S` → 3723 seconds (`PT` durations with H/M/S parts only).
pub fn parse_iso(d: &str) -> Option<u64> {
    let t = d.strip_prefix("PT")?;
    let (mut total, mut num) = (0u64, String::new());
    for c in t.chars() {
        match c {
            '0'..='9' => num.push(c),
            'H' | 'M' | 'S' => {
                let n: u64 = num.parse().ok()?;
                total += n * match c {
                    'H' => 3600,
                    'M' => 60,
                    _ => 1,
                };
                num.clear();
            }
            '.' => return None,
            _ => return None,
        }
    }
    num.is_empty().then_some(total)
}

/// 3723 → `PT1H2M3S`.
pub fn iso(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let mut o = String::from("PT");
    if h > 0 {
        o.push_str(&format!("{h}H"));
    }
    if m > 0 {
        o.push_str(&format!("{m}M"));
    }
    if s > 0 || (h == 0 && m == 0) {
        o.push_str(&format!("{s}S"));
    }
    o
}

/// Duration in seconds of an MP4/MOV file from its `moov/mvhd` box, reading only box headers and `moov`.
pub fn mp4_seconds(path: &std::path::Path) -> Option<u64> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut pos = 0u64;
    while pos + 8 <= len {
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut h = [0u8; 8];
        f.read_exact(&mut h).ok()?;
        let mut size = u64::from(u32::from_be_bytes([h[0], h[1], h[2], h[3]]));
        let mut header = 8;
        if size == 1 {
            let mut big = [0u8; 8];
            f.read_exact(&mut big).ok()?;
            size = u64::from_be_bytes(big);
            header = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < header {
            return None;
        }
        if &h[4..8] == b"moov" {
            let body_len = (size - header).min(64 * 1024 * 1024) as usize;
            let mut body = vec![0u8; body_len];
            f.read_exact(&mut body).ok()?;
            return mvhd_seconds(&body);
        }
        pos += size;
    }
    None
}

fn mvhd_seconds(moov: &[u8]) -> Option<u64> {
    let i = moov.windows(4).position(|w| w == b"mvhd")? + 4;
    let v = *moov.get(i)?;
    let be32 = |o: usize| {
        moov.get(o..o + 4)
            .map(|b| u64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])))
    };
    let (scale, dur) = if v == 1 {
        let d = moov
            .get(i + 24..i + 32)
            .map(|b| u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]));
        (be32(i + 20)?, d?)
    } else {
        (be32(i + 12)?, be32(i + 16)?)
    };
    (scale > 0).then(|| (dur + scale / 2) / scale)
}

pub fn video_object(v: &Video) -> Json {
    let mut kv = vec![
        ("@context".to_string(), Json::str("https://schema.org")),
        ("@type".into(), Json::str("VideoObject")),
        ("name".into(), Json::str(&v.name)),
        (
            "description".into(),
            Json::str(if v.description.is_empty() {
                &v.name
            } else {
                &v.description
            }),
        ),
        ("thumbnailUrl".into(), Json::str(&v.thumbnail)),
        ("uploadDate".into(), Json::str(&v.upload)),
    ];
    if let Some(c) = &v.content {
        kv.push(("contentUrl".into(), Json::str(c)));
    }
    if let Some(s) = v.seconds {
        kv.push(("duration".into(), Json::str(iso(s))));
    }
    Json::Obj(kv)
}

/// `<video:video>` sitemap element.
pub fn sitemap_entry(v: &Video) -> String {
    let x = crate::files::xml;
    let mut o = format!(
        "<video:video><video:thumbnail_loc>{}</video:thumbnail_loc><video:title>{}</video:title><video:description>{}</video:description>",
        x(&v.thumbnail),
        x(&v.name),
        x(if v.description.is_empty() {
            &v.name
        } else {
            &v.description
        })
    );
    if let Some(c) = &v.content {
        o.push_str(&format!("<video:content_loc>{}</video:content_loc>", x(c)));
    }
    if let Some(s) = v.seconds {
        o.push_str(&format!("<video:duration>{s}</video:duration>"));
    }
    o.push_str(&format!(
        "<video:publication_date>{}</video:publication_date></video:video>",
        x(&v.upload)
    ));
    o
}

/// Videos of a page that search engines can index, plus an info message for each one that cannot.
pub fn collect(
    cfg: &Config,
    root: &std::path::Path,
    route: &str,
    doc: &crate::html::Doc,
    page_date: Option<&str>,
) -> (Vec<Video>, Vec<String>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    let h1 = doc
        .headings
        .iter()
        .find(|(l, _)| *l == 1)
        .map(|(_, t)| t.clone())
        .or_else(|| doc.title.clone())
        .unwrap_or_default();
    for v in doc.videos.iter().filter(|v| v.in_content) {
        let label = v.src.clone().unwrap_or_else(|| "a video".into());
        let Some(poster) = v.poster.as_deref().filter(|p| !p.is_empty()) else {
            skipped.push(format!("`{label}` has no poster, so it is not indexed as a video; fix: add poster=\"/thumbs/….jpg\""));
            continue;
        };
        let Some(upload) = v.upload.clone().or_else(|| page_date.map(str::to_string)) else {
            skipped.push(format!(
                "`{label}` has no upload date; fix: add data-upload-date=\"YYYY-MM-DD\""
            ));
            continue;
        };
        let local = |src: &str| crate::audit::internal(cfg, route, src).map(|(p, _)| p);
        let seconds = v.duration.as_deref().and_then(parse_iso).or_else(|| {
            let p = local(v.src.as_deref()?)?;
            let l = p.to_ascii_lowercase();
            (l.ends_with(".mp4") || l.ends_with(".m4v") || l.ends_with(".mov"))
                .then(|| mp4_seconds(&root.join("public").join(p.trim_start_matches('/'))))
                .flatten()
        });
        let abs = |s: &str| {
            if s.starts_with("http") {
                s.to_string()
            } else {
                cfg.abs(&crate::files::abs_path(route, s))
            }
        };
        out.push(Video {
            name: v.title.clone().unwrap_or_else(|| h1.clone()),
            description: doc.meta("description").unwrap_or("").to_string(),
            thumbnail: abs(poster),
            content: v.src.as_deref().map(abs),
            upload,
            seconds,
        });
    }
    (out, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_iso("PT1H2M3S"), Some(3723));
        assert_eq!(parse_iso("PT90S"), Some(90));
        assert_eq!(parse_iso("1:30"), None);
        assert_eq!(
            (iso(3723), iso(90), iso(0), iso(3600)),
            (
                "PT1H2M3S".into(),
                "PT1M30S".into(),
                "PT0S".into(),
                "PT1H".into()
            )
        );
    }

    #[test]
    fn mp4_mvhd() {
        let dir = std::env::temp_dir().join(format!("brain-seo-mp4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // ftyp (16 bytes) + free (8) + moov{ mvhd v0: timescale 1000, duration 90500 }
        let mut f = Vec::new();
        f.extend(16u32.to_be_bytes());
        f.extend(b"ftypisom\0\0\0\0");
        f.extend(8u32.to_be_bytes());
        f.extend(b"free");
        let mut mvhd = Vec::new();
        mvhd.extend(b"mvhd");
        mvhd.extend([0u8; 4]); // version 0 + flags
        mvhd.extend([0u8; 8]); // creation + modification
        mvhd.extend(1000u32.to_be_bytes());
        mvhd.extend(90_500u32.to_be_bytes());
        let mut boxed = ((mvhd.len() + 4) as u32).to_be_bytes().to_vec();
        boxed.extend(mvhd);
        f.extend(((boxed.len() + 8) as u32).to_be_bytes());
        f.extend(b"moov");
        f.extend(boxed);
        let p = dir.join("a.mp4");
        std::fs::write(&p, f).unwrap();
        assert_eq!(mp4_seconds(&p), Some(91));
        std::fs::write(&p, b"not a video").unwrap();
        assert_eq!(mp4_seconds(&p), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_ld_and_sitemap() {
        let v = Video {
            name: "Demo".into(),
            description: "How it works".into(),
            thumbnail: "https://acme.dev/t.jpg".into(),
            content: Some("https://acme.dev/d.mp4".into()),
            upload: "2026-10-01".into(),
            seconds: Some(90),
        };
        let j = video_object(&v).stringify();
        assert!(
            j.contains("\"duration\":\"PT1M30S\"") && crate::jsonld::validate(&j).is_empty(),
            "{j}"
        );
        assert_eq!(
            sitemap_entry(&v),
            "<video:video><video:thumbnail_loc>https://acme.dev/t.jpg</video:thumbnail_loc><video:title>Demo</video:title><video:description>How it works</video:description><video:content_loc>https://acme.dev/d.mp4</video:content_loc><video:duration>90</video:duration><video:publication_date>2026-10-01</video:publication_date></video:video>"
        );
    }
}
