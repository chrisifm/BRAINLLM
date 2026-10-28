//! Multilingual SEO (INTERNALS §33 and §37). Locales come from `brain.json` `"i18n": { "default", "locales" }`, else
//! from the `i18n/<locale>.json` files. The default locale has no prefix (`/about`); the others do (`/es/about`). The
//! variants of a page are grouped by view (so translated slugs like `<meta route es="/es/acerca">` still pair up) or,
//! without a view, by the route without its locale prefix. Each group gets a symmetric set of
//! `<link rel="alternate" hreflang>` plus `x-default`, sitemap `xhtml:link` alternates and `og:locale:alternate`.

use crate::audit::norm;
use crate::config::Config;
use std::collections::BTreeMap;
use std::path::Path;

/// `es`, `pt-BR`, `zh-Hans`: a 2–3 letter language plus optional script/region subtags.
pub fn valid_tag(t: &str) -> bool {
    let mut parts = t.split('-');
    let lang = parts.next().unwrap_or("");
    (2..=3).contains(&lang.len())
        && lang.bytes().all(|c| c.is_ascii_lowercase())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_alphanumeric()))
}

/// Language tags from `<root>/i18n/*.json` (sorted) and warnings for file names that are not tags.
pub fn languages(root: &Path) -> (Vec<String>, Vec<String>) {
    let mut langs = Vec::new();
    let mut warn = Vec::new();
    let Ok(rd) = std::fs::read_dir(root.join("i18n")) else {
        return (langs, warn);
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if valid_tag(stem) {
            langs.push(stem.to_string());
        } else {
            warn.push(format!("i18n/{name}: `{stem}` is not a language tag, so it gets no hreflang; fix: rename it like `es.json` or `pt-BR.json`"));
        }
    }
    langs.sort();
    (langs, warn)
}

pub fn default_lang(cfg: &Config, langs: &[String]) -> Option<String> {
    cfg.default_locale
        .clone()
        .filter(|l| langs.contains(l))
        .or_else(|| cfg.lang.clone().filter(|l| langs.contains(l)))
        .or_else(|| langs.iter().find(|l| *l == "en").cloned())
        .or_else(|| langs.first().cloned())
}

/// The site's locales (only for multilingual sites: two or more).
#[derive(Debug, Clone, PartialEq)]
pub struct I18n {
    pub default: String,
    pub locales: Vec<String>,
}

impl I18n {
    /// From brain.json `i18n`, else from `i18n/*.json`. `None` for single-language sites.
    pub fn load(cfg: &Config, root: &Path) -> (Option<I18n>, Vec<String>) {
        let (langs, warn) = if cfg.locales.is_empty() {
            languages(root)
        } else {
            (cfg.locales.clone(), vec![])
        };
        let i = default_lang(cfg, &langs)
            .filter(|_| langs.len() > 1)
            .map(|default| I18n {
                default,
                locales: langs,
            });
        (i, warn)
    }

    /// Locale of a route by its first segment.
    pub fn locale_of(&self, route: &str) -> &str {
        let r = norm(route);
        let first = r.trim_start_matches('/').split('/').next().unwrap_or("");
        self.locales
            .iter()
            .find(|l| l.as_str() == first && *l != &self.default)
            .map_or(self.default.as_str(), String::as_str)
    }

    /// Route without its locale prefix (`/es/blog/a` → `/blog/a`).
    pub fn strip(&self, route: &str) -> String {
        let r = norm(route);
        let l = self.locale_of(&r);
        if l == self.default {
            return r;
        }
        let rest = r
            .trim_start_matches('/')
            .split_once('/')
            .map_or("", |(_, b)| b);
        format!("/{rest}")
    }

    /// `/blog/a` in locale `es` → `/es/blog/a`.
    pub fn prefixed(&self, locale: &str, path: &str) -> String {
        if locale == self.default {
            return norm(path);
        }
        let p = norm(path);
        if p == "/" {
            format!("/{locale}")
        } else {
            format!("/{locale}{p}")
        }
    }
}

/// `(language, route without the language prefix)` of a route.
pub fn split(route: &str, langs: &[String], default: &str) -> (String, String) {
    let i = I18n {
        default: default.to_string(),
        locales: langs.to_vec(),
    };
    (i.locale_of(route).to_string(), i.strip(route))
}

/// Variants per route: `route → [(locale, route)]`, sorted, for every page that exists in two or more locales.
/// `pages` is `(route, view file)`; a static view groups by file, everything else by the unprefixed route.
pub fn alternates(
    i: &I18n,
    pages: &[(&str, Option<&str>)],
) -> BTreeMap<String, Vec<(String, String)>> {
    let mut groups: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (route, view) in pages {
        let key = match view {
            Some(v) if !v.contains('[') => format!("view:{v}"),
            _ => format!("path:{}", i.strip(route)),
        };
        let entry = (i.locale_of(route).to_string(), route.to_string());
        let g = groups.entry(key).or_default();
        if !g.iter().any(|(l, _)| *l == entry.0) {
            g.push(entry);
        }
    }
    let mut out = BTreeMap::new();
    for mut variants in groups.into_values().filter(|v| v.len() > 1) {
        variants.sort();
        for (_, r) in &variants {
            out.insert(r.clone(), variants.clone());
        }
    }
    out
}

/// Head tags for one group: one `hreflang` per locale plus `x-default` (the default-locale page).
pub fn head_tags(cfg: &Config, i: &I18n, variants: &[(String, String)]) -> String {
    let mut html = String::new();
    for (lang, r) in variants {
        html.push_str(&format!(
            "<link rel=\"alternate\" hreflang=\"{lang}\" href=\"{}\">",
            cfg.abs(&crate::files::encode_path(r))
        ));
    }
    if let Some((_, r)) = variants.iter().find(|(l, _)| *l == i.default) {
        html.push_str(&format!(
            "<link rel=\"alternate\" hreflang=\"x-default\" href=\"{}\">",
            cfg.abs(&crate::files::encode_path(r))
        ));
    }
    html
}

/// Head tags per route for every route with variants (kept for callers that only need the map).
pub fn tags(cfg: &Config, routes: &[String], langs: &[String]) -> BTreeMap<String, String> {
    let Some(default) = default_lang(cfg, langs).filter(|_| langs.len() > 1) else {
        return BTreeMap::new();
    };
    let i = I18n {
        default,
        locales: langs.to_vec(),
    };
    let pages: Vec<(&str, Option<&str>)> = routes.iter().map(|r| (r.as_str(), None)).collect();
    alternates(&i, &pages)
        .into_iter()
        .map(|(r, v)| (r, head_tags(cfg, &i, &v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_language_prefix() {
        let cfg = Config {
            site: Some("https://acme.dev".into()),
            lang: Some("en".into()),
            ..Config::default()
        };
        let langs = vec!["en".to_string(), "es".into(), "pt-BR".into()];
        let routes: Vec<String> = [
            "/",
            "/es",
            "/pricing",
            "/es/pricing",
            "/pt-BR/pricing",
            "/about",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let t = tags(&cfg, &routes, &langs);
        assert_eq!(
            t["/es/pricing"],
            "<link rel=\"alternate\" hreflang=\"en\" href=\"https://acme.dev/pricing\"><link rel=\"alternate\" hreflang=\"es\" href=\"https://acme.dev/es/pricing\"><link rel=\"alternate\" hreflang=\"pt-BR\" href=\"https://acme.dev/pt-BR/pricing\"><link rel=\"alternate\" hreflang=\"x-default\" href=\"https://acme.dev/pricing\">"
        );
        assert_eq!(
            t["/pricing"], t["/pt-BR/pricing"],
            "every variant carries the same set"
        );
        assert!(t["/"].contains("hreflang=\"es\" href=\"https://acme.dev/es\""));
        assert!(
            !t.contains_key("/about"),
            "a page in one language gets nothing"
        );
        assert!(tags(&cfg, &routes, &["en".to_string()]).is_empty());
    }

    #[test]
    fn translated_slugs_pair_by_view() {
        let i = I18n {
            default: "en".into(),
            locales: vec!["en".into(), "es".into()],
        };
        let pages = [
            ("/about", Some("views/about.brain")),
            ("/es/acerca", Some("views/about.brain")),
            ("/blog/a", Some("views/blog/[slug].brain")),
            ("/es/blog/a", Some("views/blog/[slug].brain")),
            ("/blog/b", Some("views/blog/[slug].brain")),
        ];
        let a = alternates(&i, &pages);
        assert_eq!(
            a["/es/acerca"],
            vec![
                ("en".to_string(), "/about".to_string()),
                ("es".into(), "/es/acerca".into())
            ]
        );
        assert_eq!(a["/blog/a"].len(), 2);
        assert!(!a.contains_key("/blog/b"), "dynamic views pair by slug");
        assert_eq!(
            (
                i.locale_of("/es/acerca"),
                i.strip("/es/acerca"),
                i.prefixed("es", "/")
            ),
            ("es", "/acerca".to_string(), "/es".to_string())
        );
    }

    #[test]
    fn tags_and_defaults() {
        assert!(
            valid_tag("es")
                && valid_tag("pt-BR")
                && valid_tag("zh-Hans")
                && !valid_tag("spanish")
                && !valid_tag("EN")
        );
        let langs = vec!["de".to_string(), "es".into()];
        assert_eq!(
            default_lang(&Config::default(), &langs).as_deref(),
            Some("de")
        );
        assert_eq!(split("/es/a/b", &langs, "de"), ("es".into(), "/a/b".into()));
        assert_eq!(
            split("/essay", &langs, "de"),
            ("de".into(), "/essay".into())
        );
        let cfg = Config {
            default_locale: Some("es".into()),
            locales: vec!["en".into(), "es".into()],
            ..Config::default()
        };
        let (i, _) = I18n::load(&cfg, Path::new("/nonexistent"));
        assert_eq!(i.unwrap().default, "es");
    }
}
