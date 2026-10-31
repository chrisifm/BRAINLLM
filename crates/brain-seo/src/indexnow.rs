//! IndexNow (INTERNALS §33), opt-in: `"seo": { "indexnow": true }` in brain.json and the key as `INDEXNOW_KEY` in
//! `.brain_env` (per environment, INTERNALS §36; the process environment may override it on the host). The build publishes `/<key>.txt` (how engines verify the key); `brain seo indexnow` sends
//! the sitemap URLs (optionally only those with `lastmod` ≥ `--since`) to api.indexnow.org. Sending is a deploy step:
//! it never runs during a build, and `--dry-run` prints the request instead.

use brain_lex::Json;

pub const ENDPOINT: &str = "https://api.indexnow.org/indexnow";
/// URLs per request (protocol limit).
pub const MAX_URLS: usize = 10_000;

/// IndexNow keys: 8–128 characters of `a-z A-Z 0-9 -`.
pub fn valid_key(k: &str) -> bool {
    (8..=128).contains(&k.len()) && k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

/// Why there is no usable key, for messages (`key` is the resolved `INDEXNOW_KEY`).
pub fn key_problem(key: Option<&str>, env: &str) -> Option<String> {
    match key {
        Some(k) if valid_key(k) => None,
        Some(_) => Some("INDEXNOW_KEY must be 8-128 letters, digits or `-`; fix: generate one, e.g. 32 hex characters".into()),
        None => Some(format!("seo.indexnow is on but INDEXNOW_KEY is empty for `{env}`; fix: add INDEXNOW_KEY=<32 hex characters> to the [{env}] section of .brain_env")),
    }
}

/// `(dist path, content)` of the key file.
pub fn key_file(key: &str) -> (String, String) {
    (format!("{key}.txt"), key.to_string())
}

/// `<loc>` values of a sitemap whose `<lastmod>` is at least `since` (all when `since` is `None`; URLs without
/// `lastmod` are kept, since nothing says they are old).
pub fn urls(sitemap: &str, since: Option<&str>) -> Vec<String> {
    sitemap
        .split("<url>")
        .skip(1)
        .filter_map(|u| {
            let loc = u
                .split("<loc>")
                .nth(1)?
                .split("</loc>")
                .next()?
                .replace("&amp;", "&");
            let lastmod = u
                .split("<lastmod>")
                .nth(1)
                .and_then(|x| x.split("</lastmod>").next());
            match (since, lastmod) {
                (Some(s), Some(d)) if d < s => None,
                _ => Some(loc),
            }
        })
        .collect()
}

/// The page URLs of the built sitemap (S02): a urlset as is, or the local children of a sitemap index (`sitemap-N.xml`
/// under `site`, the origin plus base path), deduplicated in order. Children are read from `dist` only: one under
/// another origin, with `..`, missing, or itself an index (no nesting, so no cycle) is an error with a fix.
pub fn page_urls(dist: &std::path::Path, sitemap: &str, site: &str, since: Option<&str>) -> Result<Vec<String>, String> {
    if !sitemap.contains("<sitemapindex") {
        return Ok(urls(sitemap, since));
    }
    let root = site.trim_end_matches('/');
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for loc in sitemap.split("<sitemap>").skip(1).filter_map(|c| c.split("<loc>").nth(1)?.split("</loc>").next()) {
        let loc = loc.replace("&amp;", "&");
        let rel = loc
            .strip_prefix(root)
            .filter(|r| r.starts_with('/') && r.ends_with(".xml") && !r.contains("..") && !r.contains('\\') && !r.contains("//"))
            .ok_or_else(|| format!("sitemap index child {loc} is not a sitemap file of {root}; fix: rebuild with `brain build` so every child lives under the site URL"))?;
        let child = std::fs::read_to_string(dist.join(&rel[1..]))
            .map_err(|_| format!("sitemap index child {loc} is missing from dist/{}; fix: run `brain build` again and deploy every sitemap-N.xml", &rel[1..]))?;
        if child.contains("<sitemapindex") {
            return Err(format!("sitemap index child {loc} is itself an index; fix: list only urlset files in dist/sitemap.xml"));
        }
        out.extend(urls(&child, since).into_iter().filter(|u| seen.insert(u.clone())));
    }
    Ok(out)
}

/// Request body for [`ENDPOINT`]. `site` is the origin plus the base path (`https://acme.dev/app`): the key file
/// lives under it, the host is its authority.
pub fn payload(site: &str, key: &str, urls: &[String]) -> Json {
    let host = site
        .split_once("://")
        .map_or(site, |(_, h)| h)
        .split('/')
        .next()
        .unwrap_or("");
    Json::obj([
        ("host", Json::str(host)),
        ("key", Json::str(key)),
        (
            "keyLocation",
            Json::str(format!("{}/{key}.txt", site.trim_end_matches('/'))),
        ),
        ("urlList", Json::Arr(urls.iter().map(Json::str).collect())),
    ])
}

/// POSTs the payload with the system `curl` (no TLS code of our own). Returns the HTTP status.
pub fn send(body: &Json) -> Result<u16, String> {
    let out = std::process::Command::new("curl")
        .args(["-sS", "-o", "/dev/null", "-w", "%{http_code}", "-X", "POST", "-H", "Content-Type: application/json; charset=utf-8", "--data-binary", &body.stringify(), ENDPOINT])
        .output()
        .map_err(|e| format!("IndexNow needs curl ({e}); fix: install curl or run with --dry-run and send the body yourself"))?;
    let code: u16 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    match code {
        200 | 202 => Ok(code),
        0 => Err(format!("IndexNow request failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
        403 => Err("IndexNow answered 403: the key file is not reachable yet; fix: deploy dist/ (with /<key>.txt) first, then ping".into()),
        422 => Err("IndexNow answered 422: the URLs do not belong to the host or the key does not match".into()),
        c => Err(format!("IndexNow answered HTTP {c}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_payload() {
        let sm = "<urlset><url><loc>https://acme.dev/</loc></url><url><loc>https://acme.dev/a?x=1&amp;y=2</loc><lastmod>2026-10-01</lastmod></url><url><loc>https://acme.dev/old</loc><lastmod>2025-01-01</lastmod></url></urlset>";
        assert_eq!(urls(sm, None).len(), 3);
        assert_eq!(
            urls(sm, Some("2026-01-01")),
            vec![
                "https://acme.dev/".to_string(),
                "https://acme.dev/a?x=1&y=2".into()
            ]
        );
        let p = payload(
            "https://acme.dev/",
            "abcdef0123456789",
            &urls(sm, Some("2026-01-01")),
        );
        assert_eq!(
            p.stringify(),
            r#"{"host":"acme.dev","key":"abcdef0123456789","keyLocation":"https://acme.dev/abcdef0123456789.txt","urlList":["https://acme.dev/","https://acme.dev/a?x=1&y=2"]}"# // gitleaks:allow -- deterministic hexadecimal example fixture
        );
        assert!(
            valid_key("abcdef0123456789") && !valid_key("short") && !valid_key("has space in it")
        );
        assert_eq!(
            key_file("abcdef0123456789"),
            ("abcdef0123456789.txt".into(), "abcdef0123456789".into())
        );
    }
}
