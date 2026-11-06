//! Read-only migration coverage: compare path + query in a baseline sitemap with built dist/sitemap.xml.
//! Origins must be consistent within each inventory, but may differ between inventories (host migrations).
//! This observes sitemap membership only; it does not verify content, redirects or indexing.
use brain_lex::Json;
use reqwest::Url;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_FILE: usize = 8 * 1024 * 1024;
const MAX_TOTAL: usize = 32 * 1024 * 1024;
const MAX_CHILDREN: usize = 128;
const MAX_URLS: usize = 500_000;
const USAGE: &str = "usage: brain seo diff --from <sitemap.xml|http(s) sitemap URL> [--json]";

// Timed-out system DNS work can still occupy a Tokio blocking task. Dropping the observer must not wait for it
// past the command deadline; the CLI process owns the runtime and exits after returning the report.
struct ObserverRuntime(Option<tokio::runtime::Runtime>);
impl std::ops::Deref for ObserverRuntime {
    type Target = tokio::runtime::Runtime;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}
impl Drop for ObserverRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

struct Budget {
    bytes: usize,
    urls: usize,
    deadline: Instant,
}
impl Budget {
    fn new() -> Self {
        Self {
            bytes: 0,
            urls: 0,
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }
    fn time(&self) -> Result<(), String> {
        if Instant::now() >= self.deadline {
            Err("sitemap comparison exceeded its 30 second deadline".into())
        } else {
            Ok(())
        }
    }
    fn bytes(&mut self, n: usize) -> Result<(), String> {
        self.time()?;
        self.bytes = self.bytes.saturating_add(n);
        if n > MAX_FILE || self.bytes > MAX_TOTAL {
            Err("sitemap input exceeds its byte limit (8 MiB per file, 32 MiB total)".into())
        } else {
            Ok(())
        }
    }
}

fn web_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "sitemap locations must be absolute HTTP(S) URLs")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || value.chars().any(char::is_control)
    {
        return Err(
            "sitemap URLs must use HTTP(S), without credentials, fragments or control characters"
                .into(),
        );
    }
    Ok(url)
}
fn origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}
fn key(url: &Url) -> String {
    format!(
        "{}{}",
        url.path(),
        url.query().map(|q| format!("?{q}")).unwrap_or_default()
    )
}

/// Read only ordinary files, with bounded reads even if metadata changes. Child paths never traverse a symlink.
fn local_read(path: &Path, budget: &mut Budget) -> Result<String, String> {
    budget.time()?;
    let mut checked = PathBuf::new();
    for component in path.components() {
        checked.push(component);
        if std::fs::symlink_metadata(&checked).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("sitemap inputs cannot traverse symlinks".into());
        }
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| "cannot read sitemap file; provide an existing regular XML file")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("sitemap inputs must be regular files, not symlinks".into());
    }
    if metadata.len() > MAX_FILE as u64 {
        return Err("sitemap file exceeds its 8 MiB byte limit".into());
    }
    let file = std::fs::File::open(path).map_err(|_| "cannot open sitemap file")?;
    let mut bytes = Vec::new();
    file.take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read sitemap file")?;
    budget.bytes(bytes.len())?;
    String::from_utf8(bytes).map_err(|_| "sitemap must be UTF-8 XML".into())
}

fn remote_read(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    url: &Url,
    budget: &mut Budget,
) -> Result<String, String> {
    budget.time()?;
    let timeout = budget
        .deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_secs(3));
    let text = runtime
        .block_on(async {
            let mut response = client
                .get(url.clone())
                .timeout(timeout)
                .send()
                .await
                .map_err(|_| "cannot fetch sitemap within its request deadline")?;
            if response.status() != reqwest::StatusCode::OK {
                return Err("sitemap must answer HTTP 200; redirects are not followed");
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_FILE as u64)
            {
                return Err("sitemap response exceeds its 8 MiB byte limit");
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "cannot read sitemap response")?
            {
                if bytes.len().saturating_add(chunk.len()) > MAX_FILE {
                    return Err("sitemap response exceeds its 8 MiB byte limit");
                }
                bytes.extend_from_slice(&chunk);
            }
            String::from_utf8(bytes).map_err(|_| "sitemap must be UTF-8 XML")
        })
        .map_err(str::to_string)?;
    budget.bytes(text.len())?;
    Ok(text)
}

// Validate attribute boundaries rather than accepting HTML-style unquoted attributes in XML.
fn attributes(mut text: &str) -> bool {
    while !text.trim().is_empty() {
        if !text.starts_with(char::is_whitespace) {
            return false;
        }
        text = text.trim_start();
        let n = text
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.')))
            .unwrap_or(text.len());
        if n == 0 {
            return false;
        }
        text = text[n..].trim_start();
        let Some(rest) = text.strip_prefix('=') else {
            return false;
        };
        text = rest.trim_start();
        let Some(quote @ ('\'' | '"')) = text.chars().next() else {
            return false;
        };
        text = &text[1..];
        let Some(end) = text.find(quote) else {
            return false;
        };
        if text[..end].contains('<') {
            return false;
        }
        text = &text[end + 1..];
    }
    true
}

/// A bounded, owned XML walk: accept complete urlsets or one-level indexes and extract only entry locs,
/// never image/video extension locs. Reject DTDs, custom entities, nesting errors and incomplete entries.
fn locations(xml: &str) -> Result<(bool, Vec<String>), String> {
    let bad = || {
        "sitemap must be complete XML with one urlset or sitemapindex root and one loc per entry"
            .to_string()
    };
    let mut stack: Vec<String> = Vec::new();
    let mut root = None;
    let mut closed = false;
    let mut out = Vec::new();
    let mut entry_locs = 0;
    let mut loc = String::new();
    let mut pos = 0;
    while pos < xml.len() {
        let rest = &xml[pos..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->").ok_or_else(bad)?;
            pos += end + 3;
            continue;
        }
        if rest.starts_with("<?") {
            if !stack.is_empty() || closed {
                return Err(bad());
            }
            let end = rest.find("?>").ok_or_else(bad)?;
            pos += end + 2;
            continue;
        }
        if rest.starts_with("<!") {
            return Err(
                "sitemap DTDs, declarations and CDATA are unsupported; use escaped XML text".into(),
            );
        }
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            let value = &rest[..end];
            if stack.len() == 3 && stack[2] == "loc" {
                loc.push_str(value);
            } else if stack.len() < 2 && !value.trim_start_matches('\u{feff}').trim().is_empty() {
                return Err(bad());
            }
            pos += end;
            continue;
        }
        let mut quoted = None;
        let end = rest
            .char_indices()
            .skip(1)
            .find_map(|(i, c)| {
                if let Some(q) = quoted {
                    if c == q {
                        quoted = None;
                    }
                } else if matches!(c, '\'' | '"') {
                    quoted = Some(c);
                } else if c == '>' {
                    return Some(i);
                }
                None
            })
            .ok_or_else(bad)?;
        let raw = rest[1..end].trim();
        if let Some(name) = raw.strip_prefix('/') {
            if stack.pop().as_deref() != Some(name.trim()) {
                return Err(bad());
            }
            if stack.len() == 2 && name.trim() == "loc" {
                entry_locs += 1;
                if entry_locs != 1 || loc.trim().is_empty() || loc.contains('<') {
                    return Err(bad());
                }
                // Reuse the framework's owned entity decoder, after rejecting undeclared entities.
                for entity in loc.split('&').skip(1) {
                    let token = entity.split_once(';').map(|(a, _)| a).ok_or_else(bad)?;
                    if !matches!(token, "amp" | "lt" | "gt" | "quot" | "apos")
                        && !token.strip_prefix('#').is_some_and(|n| {
                            n.parse::<u32>().ok().and_then(char::from_u32).is_some()
                                || n.strip_prefix('x').is_some_and(|h| {
                                    u32::from_str_radix(h, 16)
                                        .ok()
                                        .and_then(char::from_u32)
                                        .is_some()
                                })
                        })
                    {
                        return Err(bad());
                    }
                }
                out.push(brain_seo::html::decode(loc.trim()));
                if out.len() > MAX_URLS {
                    return Err("sitemap exceeds its 500000 URL limit".into());
                }
            }
            if stack.len() == 1 && entry_locs != 1 {
                return Err(bad());
            }
            if stack.is_empty() {
                closed = true;
            }
        } else {
            if closed {
                return Err(bad());
            }
            let self_close = raw.ends_with('/');
            let start = raw.strip_suffix('/').unwrap_or(raw);
            let name = start.split_whitespace().next().ok_or_else(bad)?;
            if !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b':' | b'_' | b'-' | b'.'))
                || !attributes(&start[name.len()..])
            {
                return Err(bad());
            }
            if stack.is_empty() {
                if root.is_some() || !matches!(name, "urlset" | "sitemapindex") {
                    return Err(bad());
                }
                root = Some(name == "sitemapindex");
            } else if stack.len() == 1 {
                if name != if root == Some(true) { "sitemap" } else { "url" } {
                    return Err(bad());
                }
                entry_locs = 0;
            } else if stack.len() == 2 && name == "loc" {
                loc.clear();
            } else if stack.len() == 3 && stack[2] == "loc" {
                return Err(bad());
            }
            if stack.len() >= 32 {
                return Err("sitemap XML exceeds its nesting limit".into());
            }
            if self_close {
                if stack.is_empty() {
                    closed = true;
                } else if stack.len() == 1 || (stack.len() == 2 && name == "loc") {
                    return Err(bad());
                }
            } else {
                stack.push(name.to_string());
            }
        }
        pos += end + 1;
    }
    if !closed || !stack.is_empty() {
        return Err(bad());
    }
    Ok((root.ok_or_else(bad)?, out))
}

fn child_path(base: &Path, index_url: &Url, value: &str) -> Result<PathBuf, String> {
    let child = web_url(value)?;
    if origin(&child) != origin(index_url) || child.query().is_some() {
        return Err(
            "local sitemap children must share their index origin and have no query".into(),
        );
    }
    // Check the original spelling too: URL parsing normalizes dot segments away.
    let raw_path = value
        .split_once("://")
        .and_then(|(_, s)| s.find('/').map(|i| &s[i..]))
        .ok_or("local sitemap child has no path")?;
    let decoded = brain_seo::audit::percent_decode(raw_path);
    if decoded.contains('\\')
        || decoded.split('/').any(|s| matches!(s, "." | ".."))
        || decoded != raw_path
    {
        return Err(
            "local sitemap child path must use plain paths without traversal or encoded separators"
                .into(),
        );
    }
    let prefix = index_url.path().rsplit_once('/').map_or("/", |(p, _)| p);
    let relative = child
        .path()
        .strip_prefix(&format!("{}/", prefix.trim_end_matches('/')))
        .ok_or("local sitemap child leaves the index directory")?;
    let mut path = base.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(part) = component else {
            return Err("local sitemap child leaves the index directory".into());
        };
        path.push(part);
        if std::fs::symlink_metadata(&path)
            .map_err(|_| "local sitemap child is missing")?
            .file_type()
            .is_symlink()
        {
            return Err("local sitemap children cannot traverse symlinks".into());
        }
    }
    Ok(path)
}

fn inventory(
    source: &str,
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    budget: &mut Budget,
) -> Result<BTreeSet<String>, String> {
    let remote = source.contains("://");
    let remote_url = if remote { Some(web_url(source)?) } else { None };
    let path = Path::new(source);
    let xml = match &remote_url {
        Some(url) => remote_read(runtime, client, url, budget)?,
        None => local_read(path, budget)?,
    };
    let (index, entries) = locations(&xml)?;
    let mut pages = entries.clone();
    let mut inventory_origin = remote_url.as_ref().map(origin);
    if index {
        let unique: BTreeSet<String> = entries.into_iter().collect();
        if unique.is_empty() || unique.len() > MAX_CHILDREN {
            return Err("sitemap index must contain 1 to 128 distinct child files".into());
        }
        let first = web_url(unique.first().unwrap())?;
        let index_url = remote_url.clone().unwrap_or_else(|| {
            // A local index does not carry its public URL. Infer its directory from the first child.
            first.join("sitemap.xml").unwrap()
        });
        inventory_origin = Some(origin(&index_url));
        pages.clear();
        for entry in unique {
            budget.time()?;
            let url = web_url(&entry)?;
            if origin(&url) != origin(&index_url) {
                return Err("sitemap child must share its index origin".into());
            }
            let child = if remote {
                remote_read(runtime, client, &url, budget)?
            } else {
                local_read(
                    &child_path(path.parent().unwrap_or(Path::new(".")), &index_url, &entry)?,
                    budget,
                )?
            };
            let (nested, urls) = locations(&child)?;
            if nested {
                return Err("nested sitemap indexes are unsupported; list urlsets directly".into());
            }
            pages.extend(urls);
            if pages.len() > MAX_URLS {
                return Err("sitemap inventory exceeds its 500000 URL limit".into());
            }
        }
    }
    budget.urls = budget.urls.saturating_add(pages.len());
    if budget.urls > MAX_URLS {
        return Err("sitemap comparison exceeds its 500000 combined URL limit".into());
    }
    let mut expected = inventory_origin;
    let mut result = BTreeSet::new();
    for page in pages {
        budget.time()?;
        let url = web_url(&page)?;
        let observed = origin(&url);
        if expected.as_ref().is_some_and(|e| e != &observed) {
            return Err(
                "page URLs in each sitemap inventory must use one consistent origin".into(),
            );
        }
        expected = Some(observed);
        result.insert(key(&url));
    }
    Ok(result)
}

pub fn run(root: Option<&Path>, args: &[String]) -> (i32, String) {
    let json = args.iter().any(|a| a == "--json");
    let fail = |message: String| {
        (
            2,
            if json {
                Json::obj([
                    ("schemaVersion", Json::Num(1.0)),
                    ("ok", Json::Bool(false)),
                    ("error", Json::str(message)),
                ])
                .stringify()
            } else {
                format!("error: {message}")
            },
        )
    };
    if args.first().map(String::as_str) != Some("diff") {
        return fail(USAGE.into());
    }
    let mut from = None;
    let mut seen_json = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--json" if !seen_json => {
                seen_json = true;
            }
            "--from" if from.is_none() => {
                i += 1;
                from = args
                    .get(i)
                    .filter(|a| !a.starts_with("--"))
                    .map(String::as_str);
                if from.is_none() {
                    return fail(USAGE.into());
                }
            }
            arg if arg.starts_with("--from=") && from.is_none() => {
                from = arg.strip_prefix("--from=").filter(|a| !a.is_empty());
                if from.is_none() {
                    return fail(USAGE.into());
                }
            }
            _ => return fail(USAGE.into()),
        }
        i += 1;
    }
    let Some(from) = from else {
        return fail(USAGE.into());
    };
    let Some(root) = root else {
        return fail("run sitemap comparison in a built app".into());
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => ObserverRuntime(Some(r)),
        Err(_) => return fail("cannot start sitemap HTTP observer".into()),
    };
    let client = match crate::http_observer::client(reqwest::Client::builder())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .retry(reqwest::retry::never())
        .user_agent(concat!("brain-seo-diff/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(c) => c,
        Err(_) => return fail("cannot start sitemap HTTP client".into()),
    };
    let mut budget = Budget::new();
    let previous = match inventory(from, &runtime, &client, &mut budget) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let current_path = root.join("dist/sitemap.xml");
    let Some(current_path) = current_path.to_str() else {
        return fail("built sitemap path is not UTF-8".into());
    };
    let current = match inventory(current_path, &runtime, &client, &mut budget) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let missing: Vec<&String> = previous.difference(&current).collect();
    let retained = previous.len() - missing.len();
    let ok = missing.is_empty();
    let text = if json {
        Json::obj([
            ("schemaVersion", Json::Num(1.0)),
            ("ok", Json::Bool(ok)),
            ("comparison", Json::str("path+query")),
            ("previous", Json::Num(previous.len() as f64)),
            ("current", Json::Num(current.len() as f64)),
            ("retained", Json::Num(retained as f64)),
            ("missingCount", Json::Num(missing.len() as f64)),
            (
                "missing",
                Json::Arr(missing.iter().map(|s| Json::str(s.as_str())).collect()),
            ),
        ])
        .stringify()
    } else {
        format!(
            "Sitemap coverage (path + query): {} previous, {} current, {retained} retained, {} missing{}",
            previous.len(),
            current.len(),
            missing.len(),
            if ok {
                String::new()
            } else {
                format!(
                    "\n{}\nReview each missing path and preserve it or add an intentional redirect; sitemap membership alone does not verify redirects.",
                    missing
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        )
    };
    (if ok { 0 } else { 1 }, text)
}

#[cfg(test)]
mod bounds_tests {
    use super::*;
    #[test]
    fn aggregate_bytes_and_expired_deadlines_fail_closed() {
        let mut budget = Budget::new();
        for _ in 0..4 {
            assert!(budget.bytes(MAX_FILE).is_ok());
        }
        assert!(budget.bytes(1).is_err());
        let mut budget = Budget::new();
        budget.deadline = Instant::now() - Duration::from_millis(1);
        assert!(budget.time().is_err());
        assert!(budget.bytes(0).is_err());
    }
    #[test]
    fn xml_limits_and_extension_locs_are_scoped_to_entries() {
        assert_eq!(locations("<urlset/>").unwrap(), (false, vec![]));
        assert!(
            locations("<urlset><url><loc>https://example.com/a</loc></url></urlset>trailing")
                .is_err()
        );
        let deeply_nested = format!(
            "<urlset><url>{}<loc>https://example.com/a</loc>{}</url></urlset>",
            "<extension>".repeat(32),
            "</extension>".repeat(32)
        );
        assert!(locations(&deeply_nested).is_err());
    }
}
