//! Social images (INTERNALS §33): `views/_og.brain` (or the built-in [`DEFAULT_TEMPLATE`]) rendered per page to a
//! 1200×630 PNG with the page's title and description, cached by content hash in `.brain/cache/og/`. Rendering
//! (compile the template with the props, prerender, screenshot over CDP) is the caller's [`Shoot`] closure; this
//! module decides which pages get an image, names and caches them, and writes the `og:image` tags.
//! `seo.og`: `"missing"` (default: pages without an og:image), `"always"` (every indexable page) or `"off"`.

use crate::config::Config;
use crate::html::Doc;

pub const DEFAULT_TEMPLATE: &str = include_str!("og.brain");
pub const WIDTH: u32 = 1200;
pub const HEIGHT: u32 = 630;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Missing,
    Always,
    Off,
}

impl Mode {
    /// The `seo.og` value in brain.json.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Missing => "missing",
            Mode::Always => "always",
            Mode::Off => "off",
        }
    }
}

/// One image to render.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub route: String,
    /// `title`, `description`, `site`, `url` (the template's props).
    pub props: Vec<(String, String)>,
    /// Content hash of template + props + version: the cache key.
    pub hash: String,
    /// `dist/` path of the PNG (`og/pricing.png`).
    pub path: String,
}

/// Renders a job with the template source to PNG bytes (`WIDTH`×`HEIGHT`).
pub type Shoot<'a> = &'a dyn Fn(&Job, &str) -> Result<Vec<u8>, String>;

/// The app's `views/_og.brain`, else the built-in template.
pub fn template(root: &std::path::Path) -> String {
    std::fs::read_to_string(root.join("views/_og.brain"))
        .unwrap_or_else(|_| DEFAULT_TEMPLATE.to_string())
}

/// `/` → `og/index.png`, `/blog/a` → `og/blog/a.png`.
pub fn path_for(route: &str) -> String {
    let r = route.trim_matches('/');
    if r.is_empty() {
        "og/index.png".into()
    } else {
        format!("og/{r}.png")
    }
}

/// Pages that get an image under `mode`: `(route, doc, indexable)` in, jobs out.
pub fn plan(
    cfg: &Config,
    mode: Mode,
    pages: &[(&str, &Doc, bool)],
    site_name: &str,
    template: &str,
) -> Vec<Job> {
    // without `site` the card's og:image can only be relative, which no social network reads (B1827): rendering it
    // would cost a browser launch on every cold build for nothing. "always" is an explicit request and still renders.
    if mode == Mode::Off || mode == Mode::Missing && cfg.site.is_none() {
        return vec![];
    }
    pages
        .iter()
        .filter(|(_, d, idx)| *idx && (mode == Mode::Always || d.meta("og:image").is_none()))
        .map(|(route, d, _)| {
            // the card shows the site name on its own line, so the title drops the " · Site" a title template adds
            let full = d
                .meta("og:title")
                .map(str::to_string)
                .or_else(|| d.title.clone())
                .unwrap_or_else(|| site_name.to_string());
            let title = crate::files::strip_site(&full, site_name);
            let desc = d
                .meta("og:description")
                .or_else(|| d.meta("description"))
                .unwrap_or("")
                .to_string();
            let url = cfg.abs(route);
            let shown = url
                .split_once("://")
                .map_or(url.as_str(), |(_, r)| r)
                .trim_end_matches('/')
                .to_string();
            let props = vec![
                ("title".to_string(), title),
                ("description".into(), desc),
                ("site".into(), site_name.to_string()),
                ("url".into(), shown),
            ];
            let key = format!("{template}\0{props:?}\0{}", env!("CARGO_PKG_VERSION"));
            Job {
                route: route.to_string(),
                props,
                hash: crate::files::hash(&key),
                path: path_for(route),
            }
        })
        .collect()
}

/// Renders every job, reusing `.brain/cache/og/<hash>.png`. Returns `(dist path, PNG bytes)`.
pub fn render(
    root: &std::path::Path,
    jobs: &[Job],
    template: &str,
    shoot: Shoot,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let cache = root.join(".brain/cache/og");
    let mut out = Vec::new();
    for j in jobs {
        let cached = cache.join(format!("{}.png", j.hash));
        let bytes = match std::fs::read(&cached) {
            Ok(b) => b,
            Err(_) => {
                let b = shoot(j, template).map_err(|e| format!("og image for {}: {e}", j.route))?;
                match crate::files::png_size(&b) {
                    Some((WIDTH, HEIGHT)) => {}
                    Some((w, h)) => {
                        return Err(format!(
                            "og image for {} is {w}x{h}; the renderer must produce {WIDTH}x{HEIGHT}",
                            j.route
                        ));
                    }
                    None => return Err(format!("og image for {} is not a PNG", j.route)),
                }
                if std::fs::create_dir_all(&cache).is_ok() {
                    let _ = std::fs::write(&cached, &b);
                }
                b
            }
        };
        out.push((j.path.clone(), bytes));
    }
    Ok(out)
}

/// Tags that point a page at its generated image (they go last in `<head>`, so they replace a site default).
pub fn head_tags(cfg: &Config, job: &Job) -> String {
    format!(
        "<meta property=\"og:image\" content=\"{}\"><meta property=\"og:image:width\" content=\"{WIDTH}\"><meta property=\"og:image:height\" content=\"{HEIGHT}\"><meta property=\"og:image:type\" content=\"image/png\">",
        cfg.abs(&format!("/{}", crate::files::encode_path(&job.path)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::parse;

    #[test]
    fn plans_and_caches() {
        let cfg = Config {
            site: Some("https://acme.dev".into()),
            ..Config::default()
        };
        let a = parse(
            "<head><title>Pricing — Acme</title><meta name=description content='Plans'></head>",
        );
        let b = parse(
            "<head><title>About</title><meta property=og:image content=https://acme.dev/x.png></head>",
        );
        let pages = [
            ("/pricing", &a, true),
            ("/about", &b, true),
            ("/admin", &a, false),
        ];
        let jobs = plan(&cfg, Mode::Missing, &pages, "Acme", DEFAULT_TEMPLATE);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].path, "og/pricing.png");
        assert_eq!(jobs[0].props[3], ("url".into(), "acme.dev/pricing".into()));
        assert_eq!(
            jobs[0].props[0],
            ("title".into(), "Pricing".into()),
            "site suffix dropped"
        );
        assert_eq!(
            plan(&cfg, Mode::Always, &pages, "Acme", DEFAULT_TEMPLATE).len(),
            2
        );
        assert!(plan(&cfg, Mode::Off, &pages, "Acme", DEFAULT_TEMPLATE).is_empty());
        let local = Config::default();
        assert!(
            plan(&local, Mode::Missing, &pages, "Acme", DEFAULT_TEMPLATE).is_empty(),
            "no site: a relative og:image is useless, so nothing is rendered"
        );
        assert_eq!(
            plan(&local, Mode::Always, &pages, "Acme", DEFAULT_TEMPLATE).len(),
            2
        );
        assert_ne!(
            jobs[0].hash,
            plan(&cfg, Mode::Missing, &pages, "Acme", "<p>other</p>")[0].hash,
            "template changes the key"
        );

        let dir = std::env::temp_dir().join(format!("brain-seo-og-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(WIDTH.to_be_bytes());
        png.extend(HEIGHT.to_be_bytes());
        let calls = std::cell::Cell::new(0);
        let shoot = |_: &Job, _: &str| {
            calls.set(calls.get() + 1);
            Ok(png.clone())
        };
        let out = render(&dir, &jobs, DEFAULT_TEMPLATE, &shoot).unwrap();
        assert_eq!(out[0].0, "og/pricing.png");
        render(&dir, &jobs, DEFAULT_TEMPLATE, &shoot).unwrap();
        assert_eq!(calls.get(), 1, "second build reads the cache");
        let wrong = |_: &Job, _: &str| Ok(b"not a png".to_vec());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            render(&dir, &jobs, DEFAULT_TEMPLATE, &wrong)
                .unwrap_err()
                .contains("not a PNG")
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert!(head_tags(&cfg, &jobs[0]).starts_with(
            "<meta property=\"og:image\" content=\"https://acme.dev/og/pricing.png\">"
        ));
    }
}
