//! What the build wrote, read back: every warning here means a client gets slower or wrong bytes than the build intended.
//! - B1895 a text file large enough to matter with no `.br` or `.gz` next to it.
//! - B1896 a format that is already compressed with a `.br` or `.gz` next to it.
//! - B1897 `_headers` caching a hashed name as anything but immutable, or a stable name as immutable.
//! - B1898 `_headers` giving a text file a wrong `Content-Type` or none of the charset it needs.
//! - B1899 one page asking for more than `MAX_REQUESTS` files at load.
use std::{collections::BTreeMap, path::Path};

/// One TCP segment holds about this much: a smaller file gains almost nothing from compression.
const WORTH_COMPRESSING: u64 = 1400;
const MAX_REQUESTS: usize = 40;
const TEXT: &[&str] = &["html", "js", "mjs", "css", "svg", "json", "xml", "txt", "webmanifest", "brain", "jsbrain", "cssbrain", "md"];
const COMPRESSED: &[&str] = &["png", "jpg", "jpeg", "webp", "avif", "gif", "woff", "woff2", "mp4", "webm", "mp3", "ogg", "zip", "pdf"];
const COMPRESSED_FORMS: &[&str] = &["br", "gz", "zst"];

fn extension(path: &str) -> &str {
    path.rsplit('/').next().and_then(|name| name.rsplit_once('.')).map_or("", |(_, ext)| ext)
}
/// Read by the host or the server itself, never downloaded by a browser.
fn host_file(path: &str) -> bool {
    brain_pack::private::is_private(path)
}
fn finding(code: &str, file: &str, msg: String, fix: &str) -> String {
    format!("{code} {file}:1:1 {msg}; fix: {fix}")
}

pub fn files(sizes: &BTreeMap<String, u64>) -> Vec<String> {
    let mut out = Vec::new();
    let has = |path: &str, form: &str| sizes.contains_key(&format!("{path}.{form}"));
    for (path, size) in sizes.iter().filter(|(path, _)| !host_file(path)) {
        let ext = extension(path);
        if TEXT.contains(&ext) && *size >= WORTH_COMPRESSING && !has(path, "br") && !has(path, "gz") {
            out.push(finding("B1895", path, format!("this {size}-byte text file has no .br or .gz next to it, so it travels uncompressed"), "build it with `brain build`, or add the compressed copies when another step writes it"));
        }
        if COMPRESSED.contains(&ext) && COMPRESSED_FORMS.iter().any(|form| has(path, form)) {
            out.push(finding("B1896", path, "this format is already compressed, so a compressed copy next to it only spends disk and CPU".into(), "remove the copy; compress text only"));
        }
    }
    out
}

/// Each `_headers` rule with its headers, in file order.
fn rules(headers: &str) -> Vec<(&str, Vec<&str>)> {
    let mut out: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in headers.lines() {
        if line.starts_with('/') {
            out.push((line.trim(), Vec::new()));
        } else if let Some(rule) = out.last_mut() {
            rule.1.push(line.trim());
        }
    }
    out
}

pub fn headers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (path, lines) in rules(text) {
        let value = |name: &str| lines.iter().find_map(|line| line.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_ascii_lowercase()));
        if let Some(cache) = value("cache-control") {
            let immutable = cache.contains("immutable");
            if path.starts_with("/_b/") && !immutable {
                out.push(finding("B1897", "_headers", format!("`{path}` holds names with a content hash and is cached as `{cache}`, so every visit asks again"), "cache `/_b/*` as `public, max-age=31536000, immutable`"));
            } else if !path.starts_with("/_b/") && immutable {
                out.push(finding("B1897", "_headers", format!("`{path}` keeps its name between builds and is cached as immutable, so a change never reaches a returning visitor"), "use `no-cache`, or serve it from a hashed name"));
            }
        }
        if let Some(content_type) = value("content-type") {
            let want = match extension(path) {
                "js" | "mjs" | "jsbrain" => Some("javascript"),
                "css" | "cssbrain" => Some("text/css"),
                "html" | "brain" => Some("text/html"),
                _ => None,
            };
            if let Some(want) = want.filter(|want| !content_type.contains(want) || !content_type.contains("charset")) {
                out.push(finding("B1898", "_headers", format!("`{path}` is served as `{content_type}`, and a browser needs {want} with a charset for it"), "set the type this extension needs, with `charset=utf-8`"));
            }
        }
    }
    out
}

/// Files a page asks for as it loads: scripts, styles, preloads and images.
fn requests(html: &str) -> usize {
    let lower = html.to_ascii_lowercase();
    ["<script src=", "<script type=\"module\" src=", "rel=\"stylesheet\"", "rel=\"modulepreload\"", "rel=\"preload\"", "<img "].iter().map(|tag| lower.matches(tag).count()).sum()
}

pub fn pages(html: &BTreeMap<String, String>) -> Vec<String> {
    html.iter()
        .filter_map(|(path, text)| {
            let count = requests(text);
            (count > MAX_REQUESTS).then(|| finding("B1899", path, format!("this page asks for {count} files as it loads, above {MAX_REQUESTS}"), "lazy-load images below the fold and split components a first view does not need"))
        })
        .collect()
}

/// All findings of a finished `dist`; empty on a clean build.
pub fn check(dist: &Path) -> Vec<String> {
    let mut sizes = BTreeMap::new();
    let mut stack = vec![dist.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind) if kind.is_file() => {
                    if let (Ok(rel), Ok(meta)) = (path.strip_prefix(dist), entry.metadata()) {
                        sizes.insert(rel.to_string_lossy().replace('\\', "/"), meta.len());
                    }
                }
                _ => {}
            }
        }
    }
    let html: BTreeMap<String, String> = sizes.keys().filter(|path| path.ends_with(".html") && !host_file(path)).filter_map(|path| std::fs::read_to_string(dist.join(path)).ok().map(|text| (path.clone(), text))).collect();
    let mut out = files(&sizes);
    out.extend(headers(&std::fs::read_to_string(dist.join("_headers")).unwrap_or_default()));
    out.extend(pages(&html));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sizes(list: &[(&str, u64)]) -> BTreeMap<String, u64> {
        list.iter().map(|(path, size)| (path.to_string(), *size)).collect()
    }
    fn code(lines: Vec<String>) -> Vec<String> {
        lines.into_iter().map(|line| line.split(' ').next().unwrap().to_string()).collect()
    }
    #[test]
    fn text_needs_a_compressed_copy_and_compressed_formats_must_not_have_one() {
        assert_eq!(code(files(&sizes(&[("_b/a.js", 5000)]))), ["B1895"]);
        assert!(files(&sizes(&[("_b/a.js", 5000), ("_b/a.js.br", 900)])).is_empty());
        assert!(files(&sizes(&[("_b/a.js", 5000), ("_b/a.js.gz", 900)])).is_empty());
        assert!(files(&sizes(&[("_b/a.js", 54)])).is_empty(), "a small file gains nothing");
        assert!(files(&sizes(&[("_server/a.js", 5000), ("_brain/nginx.conf", 5000), ("server.mjs", 5000), ("_headers", 5000)])).is_empty(), "host files are not downloaded");
        assert_eq!(code(files(&sizes(&[("_b/a.woff2", 5000), ("_b/a.woff2.gz", 4900)]))), ["B1896"]);
        assert!(files(&sizes(&[("_b/a.png", 5000)])).is_empty());
    }
    #[test]
    fn cache_policy_follows_the_name_and_content_types_carry_their_charset() {
        let good = "/*\n  X-Content-Type-Options: nosniff\n/_b/*\n  Cache-Control: public, max-age=31536000, immutable\n/\n  Cache-Control: no-cache\n/_b/a.jsbrain\n  Content-Type: text/javascript; charset=utf-8\n";
        assert!(headers(good).is_empty(), "{:?}", headers(good));
        assert!(headers("").is_empty(), "no _headers: nothing to read");
        assert_eq!(code(headers("/_b/*\n  Cache-Control: no-cache\n")), ["B1897"]);
        assert_eq!(code(headers("/about\n  Cache-Control: public, max-age=31536000, immutable\n")), ["B1897"]);
        assert_eq!(code(headers("/_b/a.jsbrain\n  Content-Type: text/plain\n")), ["B1898"]);
        assert_eq!(code(headers("/about/index.brain\n  Content-Type: text/html\n")), ["B1898"], "no charset");
    }
    #[test]
    fn a_finished_dist_is_read_from_disk() {
        let dist = std::env::temp_dir().join(format!("brain-asset-check-{}", std::process::id()));
        std::fs::create_dir_all(dist.join("_b")).unwrap();
        std::fs::write(dist.join("_b/a.js"), "x".repeat(5000)).unwrap();
        std::fs::write(dist.join("_b/f.woff2"), "x").unwrap();
        std::fs::write(dist.join("_b/f.woff2.gz"), "x").unwrap();
        std::fs::write(dist.join("_headers"), "/_b/*\n  Cache-Control: no-cache\n").unwrap();
        std::fs::write(dist.join("index.html"), "<img src=x>".repeat(41)).unwrap();
        let mut found = code(check(&dist));
        found.sort();
        std::fs::remove_dir_all(&dist).ok();
        assert_eq!(found, ["B1895", "B1896", "B1897", "B1899"]);
    }
    #[test]
    fn a_page_is_measured_by_the_files_it_asks_for() {
        let many = BTreeMap::from([("index.html".to_string(), "<img src=x>".repeat(41))]);
        assert_eq!(code(pages(&many)), ["B1899"]);
        assert!(pages(&BTreeMap::from([("index.html".to_string(), "<img src=x>".repeat(40))])).is_empty());
    }
}
