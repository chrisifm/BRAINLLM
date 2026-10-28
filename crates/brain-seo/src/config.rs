//! SEO settings from `brain.json` (INTERNALS §23: flag > `BRAIN_*` variable > `brain.json` > defaults; the caller
//! applies flags and variables on top of [`Config::from_json`]).
//!
//! ```json
//! { "name": "Acme", "description": "…", "site": "https://acme.dev", "lang": "en", "env": "production",
//!   "seo": { "minScore": 80, "llms": true, "llmsFull": true, "sitemap": true, "manifest": true },
//!   "manifest": { "shortName": "Acme", "themeColor": "#0071e3", "backgroundColor": "#f5f5f7", "display": "standalone" },
//!   "google": { "analytics": "G-…", "tagManager": "GTM-…", "siteVerification": "…" },
//!   "organization": { "name": "Acme Inc.", "logo": "/logo.png", "sameAs": ["https://github.com/acme"] },
//!   "social": { "x": "@acme", "facebook": "https://www.facebook.com/acme", "instagram": "https://www.instagram.com/acme",
//!               "linkedin": "https://www.linkedin.com/company/acme", "youtube": "…", "tiktok": "…", "github": "…",
//!               "threads": "…", "mastodon": "https://mastodon.social/@acme", "facebookAppId": "1234567890" } }
//! ```
//! `seo.ai` sets the AI crawler policy: `"allow"` (default: training, AI answers and search), `"search-only"` (AI
//! search and answers yes, training no) or `"block"` (no AI crawler; classic search engines still allowed).
//! `seo.markdown` (default true) publishes a Markdown copy of every indexable page for LLMs (`/pricing.md`).
//! `seo.search` (`"/search?q={query}"`) adds a `SearchAction` to the home page's `WebSite` JSON-LD.

use brain_lex::Json;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Google {
    pub analytics: Option<String>,
    pub tag_manager: Option<String>,
    pub site_verification: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manifest {
    pub short_name: Option<String>,
    pub theme_color: Option<String>,
    pub background_color: Option<String>,
    pub display: Option<String>,
}

/// AI crawler policy (`seo.ai`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Ai {
    #[default]
    Allow,
    SearchOnly,
    Block,
}

/// Social profiles (`social` in brain.json): Open Graph/X tags, `rel="me"` links and `Organization.sameAs`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Social {
    /// X/Twitter handle with `@` (`twitter:site`).
    pub x: Option<String>,
    /// `(network, profile URL)` for every other network, in brain.json order.
    pub profiles: Vec<(String, String)>,
    pub facebook_app_id: Option<String>,
    /// `@user@server` from the Mastodon profile URL (`fediverse:creator`).
    pub fediverse: Option<String>,
}

/// An author profile (`authors` in brain.json): turns `author: Ada` in content into a `Person` with links.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Author {
    pub name: String,
    pub url: Option<String>,
    pub same_as: Vec<String>,
}

/// Google News sitemap (`seo.news`): the publication name and the collections that hold news articles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct News {
    pub name: String,
    pub collections: Vec<String>,
}

/// `Organization` JSON-LD on the home page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Organization {
    pub name: String,
    pub logo: Option<String>,
    pub same_as: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Absolute origin without trailing slash (`--site`).
    pub site: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub lang: Option<String>,
    /// `--env staging` (or any non-production env): every page `noindex`, robots `Disallow: /`, no sitemap.
    pub staging: bool,
    /// `brain doctor` fails when a page scores below this (0 = never).
    pub min_score: u32,
    pub sitemap: bool,
    pub llms: bool,
    pub llms_full: bool,
    pub manifest: bool,
    pub manifest_cfg: Manifest,
    pub google: Google,
    pub organization: Option<Organization>,
    /// Site search URL template with `{query}` (`/search?q={query}`).
    pub search: Option<String>,
    /// Entries per listing page of a content collection (`content.perPage`).
    pub per_page: usize,
    /// `base` in brain.json (`/app/`, default `/`): the path the site is served under. Absolute URLs are
    /// site + base + route; routes and internal links stay base-free (the build prefixes them after `finish`).
    pub base: String,
    /// `seo.indexnow`: publish the key file and allow `brain seo indexnow`.
    pub indexnow: bool,
    /// `INDEXNOW_KEY` resolved from `.brain_env` for the build environment ([`Config::with_env`]; never brain.json).
    pub indexnow_key: Option<String>,
    pub ai: Ai,
    /// Generated social images (`seo.og`: "missing" | "always" | "off").
    pub og: crate::og::Mode,
    /// Publish `<route>.md` for every indexable page and link it (`seo.markdown`).
    pub markdown: bool,
    pub social: Social,
    /// `i18n.default` (INTERNALS §37); `None` = single-language site, or take the languages from `i18n/*.json`.
    pub default_locale: Option<String>,
    /// `i18n.locales`, the default included.
    pub locales: Vec<String>,
    /// `authors`: `{ "Ada Lovelace": { "url": "https://…", "sameAs": ["https://…"] } }`.
    pub authors: Vec<Author>,
    /// `seo.news`: `{ "name": "Acme News", "collections": ["news"] }` → `sitemap-news.xml`.
    pub news: Option<News>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            site: None,
            name: None,
            description: None,
            lang: None,
            staging: false,
            min_score: 80,
            sitemap: true,
            llms: true,
            llms_full: true,
            manifest: true,
            manifest_cfg: Manifest::default(),
            google: Google::default(),
            organization: None,
            search: None,
            per_page: 10,
            base: "/".into(),
            indexnow: false,
            indexnow_key: None,
            ai: Ai::Allow,
            og: crate::og::Mode::Missing,
            markdown: true,
            social: Social::default(),
            default_locale: None,
            locales: vec![],
            authors: vec![],
            news: None,
        }
    }
}

fn s(j: &Json, k: &str) -> Option<String> {
    j.get(k)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn b(j: &Json, k: &str, def: bool) -> Result<bool, String> {
    match j.get(k) {
        None | Some(Json::Null) => Ok(def),
        Some(Json::Bool(v)) => Ok(*v),
        Some(_) => Err(format!(
            "brain.json: `seo.{k}` must be true or false; fix: \"{k}\": {def}"
        )),
    }
}

/// `G-XXXX` / `GTM-XXXX`: the ids are written into HTML and JS, so only `[A-Z0-9]` is accepted after the prefix.
fn tag_id(v: Option<String>, prefix: &str, key: &str) -> Result<Option<String>, String> {
    match v {
        Some(id)
            if id.strip_prefix(prefix).is_some_and(|r| {
                !r.is_empty()
                    && r.bytes()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            }) =>
        {
            Ok(Some(id))
        }
        Some(id) => Err(format!(
            "brain.json: `google.{key}` is `{id}`, expected `{prefix}` followed by letters and digits; fix: copy the id from the Google console, e.g. \"{prefix}ABC123\""
        )),
        None => Ok(None),
    }
}

/// The value of `key` for environment `env` from `<root>/.brain_env` (process environment wins), empty = unset.
pub fn env_value(root: &std::path::Path, env: &str, key: &str) -> Option<String> {
    let process = |k: &str| std::env::var(k).ok();
    let v = match std::fs::read_to_string(root.join(brain_check::env::FILE)) {
        Ok(src) => brain_check::env::parse(&src)
            .resolve(env, &process)
            .remove(key),
        Err(_) => process(key),
    };
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Absolute `http(s)://host` origin, without trailing slash.
pub fn origin(site: &str) -> Result<String, String> {
    let t = site.trim().trim_end_matches('/');
    let rest = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"));
    match rest {
        Some(h) if !h.is_empty() && !h.contains(['"', '<', '>', ' ', '\'']) => Ok(t.to_string()),
        _ => Err(format!(
            "site `{site}` must be an absolute origin; fix: `--site https://{}`",
            site.trim_start_matches("//")
        )),
    }
}

impl Config {
    /// Reads the SEO-relevant keys of a parsed `brain.json`. Unknown keys are ignored (other modules own them).
    pub fn from_json(j: &Json) -> Result<Config, String> {
        let mut c = Config {
            site: s(j, "site").map(|v| origin(&v)).transpose()?,
            name: s(j, "name"),
            description: s(j, "description"),
            lang: s(j, "lang"),
            ..Config::default()
        };
        if let Some(env) = s(j, "env") {
            c.staging = env != "production";
        }
        if let Some(seo) = j.get("seo") {
            if let Some(v) = seo.get("minScore") {
                match v {
                    Json::Num(n) if (0.0..=100.0).contains(n) => c.min_score = *n as u32,
                    _ => return Err("brain.json: `seo.minScore` must be a number from 0 to 100; fix: \"minScore\": 80".into()),
                }
            }
            c.sitemap = b(seo, "sitemap", true)?;
            c.llms = b(seo, "llms", true)?;
            c.llms_full = b(seo, "llmsFull", true)?;
            c.manifest = b(seo, "manifest", true)?;
            c.indexnow = b(seo, "indexnow", false)?;
            c.markdown = b(seo, "markdown", true)?;
            if let Some(n) = seo.get("news") {
                let Some(name) = s(n, "name") else {
                    return Err("brain.json: `seo.news` needs a `name` (the publication name Google News shows); fix: \"news\": { \"name\": \"Acme News\", \"collections\": [\"news\"] }".into());
                };
                let collections: Vec<String> = match n.get("collections") {
                    Some(Json::Arr(a)) => a
                        .iter()
                        .filter_map(Json::as_str)
                        .map(str::to_string)
                        .collect(),
                    _ => vec![],
                };
                if collections.is_empty() {
                    return Err("brain.json: `seo.news.collections` lists no collection; fix: \"collections\": [\"news\"]".into());
                }
                c.news = Some(News { name, collections });
            }
            if let Some(v) = s(seo, "og") {
                c.og = match v.as_str() {
                    "missing" => crate::og::Mode::Missing,
                    "always" => crate::og::Mode::Always,
                    "off" => crate::og::Mode::Off,
                    _ => {
                        return Err(format!(
                            "brain.json: `seo.og` is `{v}`; fix: use \"missing\", \"always\" or \"off\""
                        ));
                    }
                };
            }
            if let Some(v) = s(seo, "ai") {
                c.ai = match v.as_str() {
                    "allow" => Ai::Allow,
                    "search-only" => Ai::SearchOnly,
                    "block" => Ai::Block,
                    _ => {
                        return Err(format!(
                            "brain.json: `seo.ai` is `{v}`; fix: use \"allow\", \"search-only\" or \"block\""
                        ));
                    }
                };
            }
            if let Some(q) = s(seo, "search") {
                if !q.contains("{query}") {
                    return Err(format!(
                        "brain.json: `seo.search` is `{q}` and has no `{{query}}`; fix: \"search\": \"/search?q={{query}}\""
                    ));
                }
                c.search = Some(q);
            }
        }
        if let Some(b) = s(j, "base") {
            if b.contains(['"', '<', '>', ' ', '?', '#', '\\']) {
                return Err(format!(
                    "brain.json: `base` is `{b}`; fix: a path like \"/app/\""
                ));
            }
            c.base = format!("/{}/", b.trim_matches('/')).replace("//", "/");
        }
        if let Some(n) = j.get("content").and_then(|c| c.get("perPage")) {
            match n {
                Json::Num(v) if (1.0..=100.0).contains(v) && v.fract() == 0.0 => c.per_page = *v as usize,
                _ => return Err("brain.json: `content.perPage` must be a whole number from 1 to 100; fix: \"perPage\": 10".into()),
            }
        }
        if let Some(so) = j.get("social") {
            let Json::Obj(kv) = so else {
                return Err(
                    "brain.json: `social` must be an object; fix: \"social\": { \"x\": \"@acme\" }"
                        .into(),
                );
            };
            for (k, v) in kv {
                let Some(v) = v.as_str().map(str::trim).filter(|v| !v.is_empty()) else {
                    continue;
                };
                match k.as_str() {
                    "x" | "twitter" => {
                        let handle = v.rsplit('/').next().unwrap_or(v).trim_start_matches('@');
                        if handle.is_empty()
                            || !handle
                                .bytes()
                                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                            || handle.len() > 15
                        {
                            return Err(format!(
                                "brain.json: `social.{k}` is `{v}`; fix: the handle, e.g. \"@acme\""
                            ));
                        }
                        c.social.x = Some(format!("@{handle}"));
                        c.social
                            .profiles
                            .push(("x".into(), format!("https://x.com/{handle}")));
                    }
                    "facebookAppId" => {
                        if !v.bytes().all(|c| c.is_ascii_digit()) {
                            return Err(format!(
                                "brain.json: `social.facebookAppId` is `{v}`; fix: the numeric App ID from developers.facebook.com"
                            ));
                        }
                        c.social.facebook_app_id = Some(v.to_string());
                    }
                    net => {
                        if !v.starts_with("https://") || v.contains(['"', '<', '>', ' ']) {
                            return Err(format!(
                                "brain.json: `social.{net}` is `{v}`; fix: the full profile URL, e.g. \"https://www.instagram.com/acme\""
                            ));
                        }
                        if net == "mastodon"
                            && let Some((host, user)) =
                                v.strip_prefix("https://").and_then(|r| r.split_once("/@"))
                        {
                            c.social.fediverse =
                                Some(format!("@{}@{host}", user.trim_end_matches('/')));
                        }
                        c.social.profiles.push((net.to_string(), v.to_string()));
                    }
                }
            }
        }
        if let Some(i) = j.get("i18n") {
            let locales: Vec<String> = match i.get("locales") {
                Some(Json::Arr(a)) => a
                    .iter()
                    .filter_map(Json::as_str)
                    .map(str::to_string)
                    .collect(),
                _ => vec![],
            };
            if let Some(bad) = locales.iter().find(|l| !crate::hreflang::valid_tag(l)) {
                return Err(format!(
                    "brain.json: `i18n.locales` has `{bad}`, which is not a language tag; fix: use tags like \"es\" or \"pt-BR\""
                ));
            }
            let default = s(i, "default").or_else(|| locales.first().cloned());
            if let Some(d) = &default
                && !locales.is_empty()
                && !locales.contains(d)
            {
                return Err(format!(
                    "brain.json: `i18n.default` `{d}` is not in `i18n.locales`; fix: add it to the list"
                ));
            }
            c.locales = if locales.is_empty() {
                default.iter().cloned().collect()
            } else {
                locales
            };
            c.default_locale = default;
        }
        if let Some(a) = j.get("authors") {
            let Json::Obj(kv) = a else {
                return Err("brain.json: `authors` must be an object; fix: \"authors\": { \"Ada Lovelace\": { \"url\": \"https://…\" } }".into());
            };
            for (name, v) in kv {
                let url = s(v, "url");
                let same_as: Vec<String> = match v.get("sameAs") {
                    Some(Json::Arr(l)) => l
                        .iter()
                        .filter_map(Json::as_str)
                        .map(str::to_string)
                        .collect(),
                    _ => vec![],
                };
                if let Some(bad) = url
                    .iter()
                    .chain(&same_as)
                    .find(|u| !u.starts_with("https://") || u.contains(['"', '<', '>', ' ']))
                {
                    return Err(format!(
                        "brain.json: author `{name}` has `{bad}`; fix: full https:// profile URLs"
                    ));
                }
                c.authors.push(Author {
                    name: name.clone(),
                    url,
                    same_as,
                });
            }
        }
        if let Some(o) = j.get("organization") {
            let Some(name) = s(o, "name") else {
                return Err("brain.json: `organization` needs a `name`; fix: \"organization\": { \"name\": \"Acme Inc.\" }".into());
            };
            let same_as = match o.get("sameAs") {
                Some(Json::Arr(a)) => a
                    .iter()
                    .filter_map(Json::as_str)
                    .map(str::to_string)
                    .collect(),
                _ => vec![],
            };
            c.organization = Some(Organization {
                name,
                logo: s(o, "logo"),
                same_as,
            });
        }
        if let Some(m) = j.get("manifest") {
            c.manifest_cfg = Manifest {
                short_name: s(m, "shortName"),
                theme_color: s(m, "themeColor"),
                background_color: s(m, "backgroundColor"),
                display: s(m, "display"),
            };
        }
        if let Some(g) = j.get("google") {
            c.google = Google {
                analytics: tag_id(s(g, "analytics"), "G-", "analytics")?,
                tag_manager: tag_id(s(g, "tagManager"), "GTM-", "tagManager")?,
                site_verification: s(g, "siteVerification"),
            };
            if let Some(v) = &c.google.site_verification
                && !v
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            {
                return Err(format!(
                    "brain.json: `google.siteVerification` `{v}` has characters a verification token never has; fix: paste only the `content` value of the meta tag Google gives you"
                ));
            }
        }
        Ok(c)
    }

    /// Reads `<root>/brain.json` (defaults when the file does not exist).
    pub fn load(root: &std::path::Path) -> Result<Config, String> {
        let c = match std::fs::read_to_string(root.join("brain.json")) {
            Ok(src) => Config::from_json(
                &Json::parse(&src)
                    .map_err(|e| format!("brain.json: {e}; fix: make it valid JSON"))?,
            )?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(format!("brain.json: {e}")),
        };
        Ok(c.with_env(root, "production"))
    }

    /// Applies the build environment (`--env`, INTERNALS §36): `staging` for any non-production environment, and the
    /// environment-dependent keys resolved from `.brain_env` (process environment > `[env]` section > shared lines).
    pub fn with_env(mut self, root: &std::path::Path, env: &str) -> Config {
        if env != "production" {
            self.staging = true;
        }
        self.indexnow_key = if self.indexnow {
            env_value(root, env, "INDEXNOW_KEY")
        } else {
            None
        };
        self
    }

    /// `path` → absolute URL when `site` is known, else unchanged.
    pub fn abs(&self, path: &str) -> String {
        match &self.site {
            Some(s) if path.starts_with('/') => {
                format!("{s}{}{path}", self.base.trim_end_matches('/'))
            }
            _ => path.to_string(),
        }
    }

    /// Site origin plus base, without trailing slash (`https://acme.dev/app`), for absolutizing root-relative links.
    pub fn root(&self) -> Option<String> {
        self.site
            .as_ref()
            .map(|s| format!("{s}{}", self.base.trim_end_matches('/')))
    }

    /// `/app/about` → `/about` (paths that carry the base); anything else unchanged.
    pub fn strip_base<'a>(&self, path: &'a str) -> &'a str {
        let b = self.base.trim_end_matches('/');
        if b.is_empty() {
            return path;
        }
        match path.strip_prefix(b) {
            Some("") => "/",
            Some(rest) if rest.starts_with('/') => rest,
            _ => path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_brain_json() {
        let j = Json::parse(r#"{"name":"Acme","site":"https://acme.dev/","env":"staging","seo":{"minScore":85,"llmsFull":false},
            "google":{"analytics":"G-AB12","tagManager":"GTM-XY9","siteVerification":"abc_DEF-1"},"budget":{"viewKB":20}}"#)
        .unwrap();
        let c = Config::from_json(&j).unwrap();
        assert_eq!(c.site.as_deref(), Some("https://acme.dev"));
        assert!(c.staging && !c.llms_full && c.llms);
        assert_eq!(c.min_score, 85);
        assert_eq!(c.google.analytics.as_deref(), Some("G-AB12"));
        assert_eq!(c.abs("/x"), "https://acme.dev/x");
    }

    #[test]
    fn rejects_unsafe_values() {
        let bad = |src: &str| Config::from_json(&Json::parse(src).unwrap()).unwrap_err();
        assert!(bad(r#"{"google":{"analytics":"G-1\"><script>"}}"#).contains("fix:"));
        assert!(bad(r#"{"google":{"tagManager":"G-123"}}"#).contains("GTM-"));
        assert!(bad(r#"{"site":"acme.dev"}"#).contains("https://acme.dev"));
        assert!(bad(r#"{"seo":{"minScore":"high"}}"#).contains("0 to 100"));
        assert!(Config::from_json(&Json::parse("{}").unwrap()).unwrap() == Config::default());
        assert_eq!(
            Config::default().min_score,
            80,
            "default fixed by Chris (2026-10-13)"
        );
        let social = Config::from_json(&Json::parse(r#"{"seo":{"ai":"search-only"},"social":{"x":"https://x.com/acme_dev","instagram":"https://www.instagram.com/acme","mastodon":"https://mastodon.social/@acme","facebookAppId":"123"}}"#).unwrap()).unwrap();
        assert_eq!(social.ai, Ai::SearchOnly);
        let i = Config::from_json(
            &Json::parse(r#"{"i18n":{"default":"en","locales":["en","es","pt-BR"]}}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (i.default_locale.as_deref(), i.locales.len()),
            (Some("en"), 3)
        );
        assert!(
            bad(r#"{"i18n":{"default":"fr","locales":["en","es"]}}"#)
                .contains("not in `i18n.locales`")
        );
        assert_eq!(social.social.x.as_deref(), Some("@acme_dev"));
        assert_eq!(
            social.social.fediverse.as_deref(),
            Some("@acme@mastodon.social")
        );
        assert_eq!(social.social.profiles.len(), 3);
        assert!(
            bad(r#"{"social":{"instagram":"instagram.com/acme"}}"#)
                .contains("https://www.instagram.com/acme")
        );
        assert!(bad(r#"{"seo":{"ai":"maybe"}}"#).contains("search-only"));
        assert_eq!(
            Config::from_json(&Json::parse(r#"{"seo":{"minScore":0}}"#).unwrap())
                .unwrap()
                .min_score,
            0,
            "0 turns the threshold off"
        );
    }
}
