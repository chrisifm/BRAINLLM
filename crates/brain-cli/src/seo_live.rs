//! `brain seo audit --url <origin>`: the B18xx rules over what a running deployment really answers, for the pages a
//! build cannot see (`[slug]` views rendered per request). It reads the sitemap, samples it and adds the checks only a
//! response can answer: error pages served with 200, repeated security headers, a site-wide noindex. It never rewrites
//! anything: a long or repeated title is reported, and changing it stays a decision of the app.
use brain_lex::Json;
use brain_seo::{Finding, Level, Page, Site};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

/// A sample cannot judge the links, anchors or orphans of the pages it left out.
const SAMPLE_BLIND: [&str; 7] = ["B1812", "B1813", "B1819", "B1820", "B1863", "B1864", "B1865"];
/// Who says whether a response came from the cache, by CDN.
const CDN_STATUS: [&str; 5] = ["cf-cache-status", "x-vercel-cache", "cdn-cache-status", "x-cache-status", "x-cache"];
/// Under this a compressed response gains nothing, and servers rightly skip it.
const COMPRESS_MIN: usize = 1024;
/// One value only: a second copy is ignored, or makes the browser discard the policy.
const SINGLE_VALUE: [&str; 5] = ["x-frame-options", "x-content-type-options", "permissions-policy", "referrer-policy", "strict-transport-security"];
const USAGE: &str = "usage: brain seo audit --url <origin> [--sample N] [--path /route]... [--delay-ms N] [--expect-noindex] [--json]";

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// What the audit asked for beyond the pages. Separate from the network so that tests can feed it responses.
#[derive(Default)]
pub struct Probes {
    /// The pages came from the sitemap, so each one should be the address it declares as canonical.
    pub listed: bool,
    pub robots: Option<String>,
    /// `(clean url, variant url, response)`.
    pub variants: Vec<(String, String, Response)>,
    /// `(url, status the CDN gave on the first request, second response)`.
    pub second: Option<(String, String, Response)>,
    /// Text assets the first page loads, requested accepting compression.
    pub assets: Vec<(String, Response)>,
}

struct Live {
    code: &'static str,
    level: Level,
    url: String,
    msg: String,
    fix: String,
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let inline = format!("{name}=");
    args.iter().enumerate().find_map(|(i, a)| if a == name { args.get(i + 1).map(String::as_str) } else { a.strip_prefix(&inline) })
}

/// `<loc>` values of a sitemap or a sitemap index.
pub fn locs(xml: &str) -> Vec<String> {
    xml.split("<loc>").skip(1).filter_map(|part| part.split("</loc>").next()).map(|loc| loc.trim().replace("&amp;", "&")).collect()
}

/// `n` paths spread over the whole list, so that every kind of page in a sorted sitemap is represented.
pub fn spread(paths: &[String], n: usize) -> Vec<String> {
    if n == 0 || paths.len() <= n {
        return paths.to_vec();
    }
    (0..n).map(|i| paths[i * paths.len() / n].clone()).collect()
}

fn values<'a>(response: &'a Response, name: &str) -> Vec<&'a str> {
    response.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).collect()
}
fn noindex(value: &str) -> bool {
    value.split([',', ' ']).any(|part| part.trim().eq_ignore_ascii_case("noindex"))
}
fn meta_noindex(html: &str) -> bool {
    brain_seo::html::parse(html).meta("robots").is_some_and(noindex)
}
fn header_noindex(response: &Response) -> bool {
    values(response, "x-robots-tag").iter().any(|v| noindex(v))
}

fn absolute(origin: &str, url: &str) -> String {
    if url.starts_with('/') && !url.starts_with("//") { format!("{origin}{url}") } else { url.to_string() }
}

/// The canonical a response declares, in a `Link` header or in the document, as an absolute URL.
fn canonical_of(origin: &str, response: &Response) -> Option<String> {
    let header = values(response, "link").into_iter().flat_map(|v| v.split(',')).find(|part| part.to_ascii_lowercase().contains("rel=\"canonical\"") || part.to_ascii_lowercase().contains("rel=canonical"));
    let target = header.and_then(|part| Some(part.split_once('<')?.1.split_once('>')?.0.to_string()));
    let target = target.or_else(|| brain_seo::html::parse(&response.body).link("canonical").map(str::to_string))?;
    Some(absolute(origin, target.trim()))
}

fn same_url(a: &str, b: &str) -> bool {
    brain_seo::audit::percent_decode(a.split('#').next().unwrap_or(a)) == brain_seo::audit::percent_decode(b.split('#').next().unwrap_or(b))
}

/// B1867 and B1868: a sitemap names final addresses, and a page agrees with it about which one it is.
fn check_addresses(origin: &str, pages: &[(String, Response)], listed: bool) -> Vec<Live> {
    let mut out = Vec::new();
    for (url, response) in pages {
        if (300..400).contains(&response.status) {
            let to = values(response, "location").first().copied().unwrap_or("nowhere").to_string();
            let msg = if listed { format!("the sitemap lists this URL and it answers {} to {to}", response.status) } else { format!("this URL answers {} to {to}", response.status) };
            out.push(Live { code: "B1867", level: Level::Warning, url: url.clone(), msg, fix: "list the final address in the sitemap, and link it directly".into() });
        } else if listed && response.status == 200 && !meta_noindex(&response.body) && let Some(canonical) = canonical_of(origin, response).filter(|c| !same_url(c, url)) {
            out.push(Live { code: "B1868", level: Level::Warning, url: url.clone(), msg: format!("the sitemap lists this URL and its canonical says {canonical}"), fix: "make the sitemap and the canonical name the same address".into() });
        }
    }
    out
}

/// Other spellings of `path`; a redirect or a 404 from any of them is the right answer.
pub fn variant_urls(origin: &str, path: &str) -> Vec<String> {
    let clean = path.split(['?', '#']).next().unwrap_or(path).trim_end_matches('/');
    let mut out = Vec::new();
    if !clean.is_empty() {
        out.push(format!("{origin}/{clean}"));
        let upper = clean.to_uppercase();
        if upper != clean {
            out.push(format!("{origin}{upper}"));
        }
    }
    out.push(format!("{origin}{clean}/index.html"));
    let joined = if clean.is_empty() { "/" } else { clean };
    out.push(format!("{origin}{joined}?utm_source=x"));
    out
}

fn check_variants(origin: &str, variants: &[(String, String, Response)]) -> Vec<Live> {
    variants.iter().filter(|(_, _, r)| r.status == 200).filter_map(|(clean, url, r)| {
        let canonical = canonical_of(origin, r);
        let agrees = canonical.as_deref().is_some_and(|c| same_url(c.trim_end_matches('/'), clean.trim_end_matches('/')));
        (!agrees).then(|| Live {
            code: "B1869",
            level: Level::Warning,
            url: url.clone(),
            msg: format!("this variant answers 200 and its canonical is {}", canonical.as_deref().map_or("missing".to_string(), |c| format!("`{c}`"))),
            fix: format!("redirect it to {clean}, or give it a canonical to that address"),
        })
    }).collect()
}

/// `Cache-Control` that lets a shared cache keep the response.
fn cacheable(response: &Response) -> bool {
    let value = values(response, "cache-control").join(",").to_ascii_lowercase();
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    if parts.iter().any(|p| matches!(*p, "private" | "no-store" | "no-cache")) {
        return false;
    }
    parts.iter().any(|p| *p == "public" || p.strip_prefix("s-maxage=").or_else(|| p.strip_prefix("max-age=")).and_then(|n| n.parse::<u64>().ok()).is_some_and(|n| n > 0))
}

fn cdn_status(response: &Response) -> Option<String> {
    CDN_STATUS.iter().find_map(|name| values(response, name).first().map(|v| v.to_ascii_uppercase()))
}

fn cdn_skipped(status: &str) -> bool {
    status.contains("DYNAMIC") || status.contains("MISS")
}

/// A content hash in the path: a segment of 6 or more letters and digits, with a digit in it.
fn hashed_name(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let path = path.split_once("://").map_or(path, |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
    path.split(['/', '.', '-', '_']).any(|part| part.len() >= 6 && part.bytes().all(|b| b.is_ascii_alphanumeric()) && part.bytes().any(|b| b.is_ascii_digit()))
}

/// B1870: what a response declares against what happens.
fn check_declared(pages: &[(String, Response)], probes: &Probes) -> Vec<Live> {
    let mut out = Vec::new();
    let mut push = |url: &str, msg: String, fix: &str| out.push(Live { code: "B1870", level: Level::Warning, url: url.to_string(), msg, fix: fix.into() });
    let seen = pages.iter().chain(probes.assets.iter()).filter(|(_, r)| r.status == 200);
    let (mut cookie, mut immutable, mut stored, mut speculation) = (false, false, false, false);
    for (url, response) in seen {
        let declared = values(response, "cache-control").join(",").to_ascii_lowercase();
        if !stored && (declared.contains("private") || declared.contains("no-store")) && cdn_status(response).is_some_and(|s| s.contains("HIT")) {
            stored = true;
            push(url, "this response says `private` or `no-store` and the CDN served it from its cache: one visitor can receive what was meant for another".into(), "remove the cache rule that overrides the origin on this path, then purge it");
        }
        if !speculation && values(response, "speculation-rules").iter().any(|v| v.contains("/cdn-cgi/")) {
            speculation = true;
            push(url, "the CDN adds its own `Speculation-Rules`, so browsers prefetch pages the origin never offered, and a prefetch the CDN refuses shows as a 503 in the console".into(), "turn the feature off at the CDN (Cloudflare: Speed Brain), or accept those 503: visitors still get the page on a real navigation");
        }
        if !cookie && cacheable(response) && !values(response, "set-cookie").is_empty() {
            cookie = true;
            push(url, "this response is cacheable and sets a cookie: a shared cache can hand one visitor's cookie to another".into(), "send `private` or `no-store` where a cookie is set, or drop the cookie from cached responses");
        }
        if !immutable && values(response, "cache-control").join(",").to_ascii_lowercase().contains("immutable") && !hashed_name(url) {
            immutable = true;
            push(url, "this name has no content hash and is marked `immutable`: a new version never reaches a browser that kept it".into(), "keep `immutable` for files whose name changes with their content");
        }
    }
    if let Some((url, first, second)) = &probes.second
        && let Some(status) = cdn_status(second).filter(|s| second.status == 200 && cdn_skipped(s))
    {
        push(url, format!("this page declares itself cacheable and the CDN said {first}, then {status} on a second request"), "tell the CDN to cache HTML (`s-maxage` and a cache rule), or stop declaring it cacheable");
    }
    for (url, response) in probes.assets.iter().filter(|(_, r)| r.status == 200) {
        if values(response, "content-encoding").is_empty() && response.body.len() >= COMPRESS_MIN {
            push(url, format!("this text asset of {} KB comes uncompressed to a request that accepts gzip and br", response.body.len() / 1024), "serve the precompressed copy (`Vary: Accept-Encoding`)");
        }
    }
    out
}

/// B1871 over what the running site answers.
fn check_robots(origin: &str, pages: &[(String, Response)], robots: &str) -> Vec<Live> {
    let list: Vec<Page> = pages.iter().filter(|(_, r)| r.status == 200).filter_map(|(url, r)| Some(Page { route: path_of(origin, url)?.to_string(), html: r.body.clone(), ..Default::default() })).collect();
    brain_seo::checks::robots_conflicts(&list, robots).into_iter().map(|(route, f)| Live { code: f.code, level: f.level, url: format!("{origin}{route}"), msg: f.msg, fix: f.fix }).collect()
}

/// Same-origin scripts and styles of a page: one of each is enough to see how the server treats them.
pub fn text_assets(origin: &str, html: &str) -> Vec<String> {
    let doc = brain_seo::html::parse(html);
    let style = doc.links.iter().find(|l| brain_seo::html::attr(l, "rel").is_some_and(|r| r.eq_ignore_ascii_case("stylesheet"))).and_then(|l| brain_seo::html::attr(l, "href")).map(str::to_string);
    let mut script = None;
    let mut rest = html;
    while let Some(at) = rest.find("<script") {
        let Some(end) = rest[at..].find('>') else { break };
        let attrs = brain_seo::html::parse_attrs(&rest[at..at + end + 1]);
        if let Some(src) = brain_seo::html::attr(&attrs, "src") {
            script = Some(src.to_string());
            break;
        }
        rest = &rest[at + end + 1..];
    }
    [style, script].into_iter().flatten().map(|u| absolute(origin, &u)).filter(|u| path_of(origin, u).is_some()).collect()
}

/// The checks that need the response itself, for the pages the sitemap promises as indexable.
fn check_pages(pages: &[(String, Response)], expect_noindex: bool) -> Vec<Live> {
    let mut out = Vec::new();
    let answered: Vec<&(String, Response)> = pages.iter().filter(|(_, r)| r.status == 200).collect();
    let all_meta = !answered.is_empty() && answered.iter().all(|(_, r)| meta_noindex(&r.body));
    let all_header = !answered.is_empty() && answered.iter().all(|(_, r)| header_noindex(r));
    if (all_meta || all_header) && !expect_noindex {
        out.push(Live {
            code: "B1858",
            level: Level::Error,
            url: answered[0].0.clone(),
            msg: format!("all {} sampled pages answer noindex ({}): search engines drop the whole site", answered.len(), if all_header { "X-Robots-Tag" } else { "<meta robots>" }),
            fix: "remove it from production; on a staging deployment set `seo.staging` in brain.json or pass --expect-noindex".into(),
        });
    }
    let mut repeated: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (url, response) in pages {
        if (300..400).contains(&response.status) {
            continue; // B1867 says what a redirect means here
        }
        if response.status != 200 {
            let retry = if matches!(response.status, 429 | 503) { " (rate limit or restart: a crawler sees it too)" } else { "" };
            out.push(Live {
                code: "B1856",
                level: Level::Error,
                url: url.clone(),
                msg: format!("this indexable page answers {}{retry}", response.status),
                fix: "list only pages that answer 200, and check the server logs for this route".into(),
            });
        } else if !all_meta && meta_noindex(&response.body) {
            let title = brain_seo::html::parse(&response.body).title.unwrap_or_default();
            out.push(Live {
                code: "B1856",
                level: Level::Error,
                url: url.clone(),
                msg: format!("this listed page answers 200 with noindex and the title \"{}\": probably an error page", title.trim()),
                fix: "answer a failed load with its real status (`throw` in `load`, 503 for a temporary failure), and keep the visitor's address in server calls so one client cannot spend everyone's rate limit".into(),
            });
        }
        for name in SINGLE_VALUE {
            let found = values(response, name);
            if found.len() > 1 {
                repeated.entry(name.to_string()).or_default().insert(found.join(" | "));
            }
        }
    }
    let first = pages.first().map(|(url, _)| url.clone()).unwrap_or_default();
    for (name, seen) in repeated {
        out.push(Live {
            code: "B1857",
            level: Level::Warning,
            url: first.clone(),
            msg: format!("`{name}` is sent more than once: {}", seen.into_iter().next().unwrap_or_default()),
            fix: "set this header in one place only: the proxy or the app, not both".into(),
        });
    }
    out
}

/// A page that does not exist must not answer 200: search engines treat that as a soft 404.
fn check_missing(url: &str, response: &Response) -> Option<Live> {
    (response.status == 200).then(|| Live {
        code: "B1859",
        level: Level::Warning,
        url: url.to_string(),
        msg: "an address that does not exist answers 200".into(),
        fix: "`throw notFound()` from `load` when the record does not exist".into(),
    })
}

/// Same words pointing at different places. Reported, never scored: it depends on the data.
fn check_links(pages: &[Page]) -> Vec<Live> {
    let mut out = Vec::new();
    for page in pages {
        let mut targets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for anchor in brain_seo::html::parse(&page.html).anchors {
            let text = anchor.text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() && anchor.in_content {
                targets.entry(text).or_default().insert(anchor.href);
            }
        }
        for (text, hrefs) in targets.into_iter().filter(|(_, hrefs)| hrefs.len() > 1).take(3) {
            out.push(Live {
                code: "B1860",
                level: Level::Info,
                url: page.route.clone(),
                msg: format!("\"{text}\" links to {} different pages", hrefs.len()),
                fix: "add what tells them apart to the link text; if they are the same thing, keep one".into(),
            });
        }
    }
    out
}

fn without_meta_noindex(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("<meta") {
        let Some(end) = rest[at..].find('>') else { break };
        let tag = &rest[at..at + end + 1];
        out.push_str(&rest[..at]);
        if !(tag.contains("name=\"robots\"") && noindex(brain_seo::html::parse_attrs(tag).iter().find(|(k, _)| k == "content").map_or("", |(_, v)| v.as_str()))) {
            out.push_str(tag);
        }
        rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    out
}

fn path_of<'a>(origin: &str, url: &'a str) -> Option<&'a str> {
    let rest = url.strip_prefix(origin)?;
    if rest.is_empty() { Some("/") } else { rest.starts_with('/').then_some(rest) }
}

fn level_json(level: Level) -> Json {
    Json::str(level.as_str())
}

/// Audits an already fetched sample. Separate from the network so that tests can feed it responses.
#[allow(clippy::too_many_arguments)]
pub fn report(origin: &str, cfg: &mut brain_seo::Config, sampled: bool, expect_noindex: bool, pages: Vec<(String, Response)>, missing: Option<(String, Response)>, probes: Probes) -> (i32, Json, String) {
    cfg.site = Some(origin.to_string());
    let mut live = check_pages(&pages, expect_noindex);
    if let Some((url, response)) = &missing {
        live.extend(check_missing(url, response));
    }
    live.extend(check_addresses(origin, &pages, probes.listed));
    live.extend(check_variants(origin, &probes.variants));
    live.extend(check_declared(&pages, &probes));
    if let Some(robots) = &probes.robots {
        live.extend(check_robots(origin, &pages, robots));
    }
    let keep_noindex = !expect_noindex && !cfg.staging;
    let blocked: BTreeSet<&str> = live.iter().filter(|l| l.code == "B1856").map(|l| l.url.as_str()).collect();
    let audited: Vec<Page> = pages
        .iter()
        .filter(|(url, r)| r.status == 200 && !blocked.contains(url.as_str()))
        .filter_map(|(url, r)| {
            let route = path_of(origin, url)?.to_string();
            let html = if keep_noindex { r.body.clone() } else { without_meta_noindex(&r.body) };
            let file = format!("{}/index.html", route.trim_matches('/'));
            Some(Page { route, file: file.trim_start_matches('/').to_string(), html, ..Default::default() })
        })
        .collect();
    live.extend(check_links(&audited));
    let was_staging = std::mem::replace(&mut cfg.staging, false);
    let routes: Vec<String> = audited.iter().map(|p| p.route.clone()).collect();
    let site = Site { cfg, pages: &audited, routes: &routes, assets: &[], redirects: &[], images: &[], i18n: None };
    let mut audit = brain_seo::audit::audit(&site);
    cfg.staging = was_staging;
    if sampled {
        let seen = |f: &Finding| !SAMPLE_BLIND.contains(&f.code);
        audit.site.retain(seen);
        audit.pages.iter_mut().for_each(|p| p.findings.retain(seen));
        audit.focus.clear(); // its counts need every page, and a sample has a few
    }
    let live_failed = live.iter().any(|l| l.level == Level::Error);
    let ok = !live_failed && audit.ok();
    let entries = live.iter().map(|l| {
        Json::obj([("code", Json::str(l.code)), ("level", level_json(l.level)), ("url", Json::str(l.url.as_str())), ("msg", Json::str(l.msg.as_str())), ("fix", Json::str(l.fix.as_str()))])
    });
    let json = Json::obj([
        ("schemaVersion", Json::Num(1.0)),
        ("ok", Json::Bool(ok)),
        ("origin", Json::str(origin)),
        ("requested", Json::Num(pages.len() as f64)),
        ("audited", Json::Num(audited.len() as f64)),
        ("live", Json::Arr(entries.collect())),
        ("audit", audit.to_json()),
    ]);
    let mut text = format!("{origin}: {} pages requested, {} audited\n", pages.len(), audited.len());
    for l in &live {
        text.push_str(&format!("{} {} {}: {}; fix: {}\n", l.level.as_str(), l.code, l.url, l.msg, l.fix));
    }
    text.push_str(&audit.text());
    (if ok { 0 } else { 1 }, json, text)
}

fn fetch(runtime: &tokio::runtime::Runtime, client: &reqwest::Client, url: &str) -> Result<Response, String> {
    fetch_with(runtime, client, url, false)
}

/// The client follows no redirect, so a 301 stays visible. `compressed` asks for an encoding; the client has no
/// decoder, so that is only for responses judged by their headers.
fn fetch_with(runtime: &tokio::runtime::Runtime, client: &reqwest::Client, url: &str, compressed: bool) -> Result<Response, String> {
    runtime.block_on(async {
        let request = client.get(url);
        let request = if compressed { request.header("accept-encoding", "gzip, br") } else { request };
        let response = request.send().await.map_err(|e| format!("{url}: {e}"))?;
        let status = response.status().as_u16();
        let headers = response.headers().iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
        let body = response.text().await.map_err(|e| format!("{url}: {e}"))?;
        Ok(Response { status, headers, body })
    })
}

/// Follows redirects by hand, for files that are only a means here: the sitemap itself.
fn fetch_following(runtime: &tokio::runtime::Runtime, client: &reqwest::Client, url: &str) -> Result<Response, String> {
    let mut current = url.to_string();
    for _ in 0..5 {
        let response = fetch(runtime, client, &current)?;
        let Some(next) = values(&response, "location").first().map(|l| l.to_string()).filter(|_| (300..400).contains(&response.status)) else { return Ok(response) };
        let base = reqwest::Url::parse(&current).map_err(|e| format!("{current}: {e}"))?;
        current = base.join(&next).map_err(|e| format!("{current}: {e}"))?.to_string();
    }
    Err(format!("{url}: too many redirects"))
}

fn sitemap_paths(runtime: &tokio::runtime::Runtime, client: &reqwest::Client, origin: &str) -> Result<Vec<String>, String> {
    let index = fetch_following(runtime, client, &format!("{origin}/sitemap.xml"))?;
    if index.status != 200 {
        return Err(format!("{origin}/sitemap.xml answers {}; name the pages with --path", index.status));
    }
    let mut urls = Vec::new();
    for loc in locs(&index.body) {
        if loc.ends_with(".xml") {
            let nested = fetch_following(runtime, client, &loc)?;
            urls.extend(locs(&nested.body));
        } else {
            urls.push(loc);
        }
    }
    Ok(urls.iter().filter_map(|u| path_of(origin, u)).map(str::to_string).collect())
}

pub fn run(root: Option<&Path>, args: &[String]) -> (i32, String) {
    let json = args.iter().any(|a| a == "--json");
    let fail = |code: i32, message: String| (code, if json { Json::obj([("ok", Json::Bool(false)), ("error", Json::str(message.as_str()))]).stringify() } else { message });
    let Some(origin) = flag(args, "--url").map(|u| u.trim_end_matches('/').to_string()) else { return fail(2, USAGE.into()) };
    if !(origin.starts_with("http://") || origin.starts_with("https://")) {
        return fail(2, format!("--url must start with http:// or https://\n{USAGE}"));
    }
    let number = |name: &str, default: u64| flag(args, name).map_or(Ok(default), |v| v.parse::<u64>().map_err(|_| format!("{name} must be a number")));
    let (sample, delay) = match (number("--sample", 30), number("--delay-ms", 2000)) {
        (Ok(sample), Ok(delay)) => (sample as usize, delay),
        (Err(e), _) | (_, Err(e)) => return fail(2, e),
    };
    let mut cfg = root.and_then(|r| brain_seo::Config::load(r).ok()).unwrap_or_default();
    let expect_noindex = args.iter().any(|a| a == "--expect-noindex") || cfg.staging;
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return fail(2, "could not start the HTTP client".into()) };
    let Ok(client) = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(30)).user_agent(concat!("brain-seo-audit/", env!("CARGO_PKG_VERSION"))).build() else {
        return fail(2, "could not start the HTTP client".into());
    };
    let named: Vec<String> = args.iter().enumerate().filter(|(i, _)| *i > 0 && args[i - 1] == "--path").map(|(_, a)| a.clone()).chain(args.iter().filter_map(|a| a.strip_prefix("--path=").map(str::to_string))).collect();
    let listed = named.is_empty();
    let paths = if named.is_empty() {
        match sitemap_paths(&runtime, &client, &origin) {
            Ok(all) => spread(&all, sample),
            Err(e) => return fail(2, e),
        }
    } else {
        named
    };
    let mut pages = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        let url = format!("{origin}{path}");
        match fetch(&runtime, &client, &url) {
            Ok(response) => pages.push((url, response)),
            Err(e) => return fail(2, e),
        }
    }
    std::thread::sleep(Duration::from_millis(delay));
    let probe = paths.iter().rfind(|p| p.trim_matches('/').contains('/')).or(paths.first()).map(|p| format!("{origin}{}-brain-missing-page", p.trim_end_matches('/')));
    let missing = probe.and_then(|url| fetch(&runtime, &client, &url).ok().map(|r| (url, r)));
    let mut probes = Probes { listed, ..Probes::default() };
    let pause = || std::thread::sleep(Duration::from_millis(delay));
    pause();
    probes.robots = fetch(&runtime, &client, &format!("{origin}/robots.txt")).ok().filter(|r| r.status == 200).map(|r| r.body);
    // one page stands for the rest: a handful of extra requests, each after the same pause
    let answered = || pages.iter().filter(|(_, r)| r.status == 200);
    let sample_page = answered().find(|(u, _)| path_of(&origin, u).is_some_and(|p| p != "/")).or_else(|| answered().next());
    if let Some((clean, _)) = sample_page {
        let path = path_of(&origin, clean).unwrap_or("/");
        for url in variant_urls(&origin, path) {
            pause();
            if let Ok(response) = fetch(&runtime, &client, &url) {
                probes.variants.push((clean.clone(), url, response));
            }
        }
    }
    if let Some((url, first)) = answered().find_map(|(u, r)| cdn_status(r).filter(|s| cacheable(r) && cdn_skipped(s)).map(|s| (u.clone(), s))) {
        pause();
        if let Ok(response) = fetch(&runtime, &client, &url) {
            probes.second = Some((url, first, response));
        }
    }
    if let Some((_, first)) = answered().next() {
        for url in text_assets(&origin, &first.body) {
            pause();
            if let Ok(response) = fetch_with(&runtime, &client, &url, true) {
                probes.assets.push((url, response));
            }
        }
    }
    let (code, value, text) = report(&origin, &mut cfg, true, expect_noindex, pages, missing, probes);
    (code, if json { value.stringify() } else { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    const ORIGIN: &str = "https://example.test";
    fn page(path: &str, status: u16, headers: &[(&str, &str)], title: &str, robots: Option<&str>) -> (String, Response) {
        let meta = robots.map_or(String::new(), |r| format!("<meta name=\"robots\" content=\"{r}\">"));
        let body = format!("<!doctype html><html lang=\"en\"><head><title>{title}</title>{meta}</head><body><main><h1>{title}</h1><p>Text.</p></main></body></html>");
        (format!("{ORIGIN}{path}"), Response { status, headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body })
    }
    fn codes(pages: Vec<(String, Response)>, expect: bool, missing: Option<(String, Response)>) -> (i32, String) {
        let mut cfg = brain_seo::Config::default();
        let (code, json, _) = report(ORIGIN, &mut cfg, true, expect, pages, missing, Probes::default());
        (code, json.stringify())
    }
    fn with(pages: Vec<(String, Response)>, probes: Probes) -> String {
        let mut cfg = brain_seo::Config::default();
        report(ORIGIN, &mut cfg, true, false, pages, None, probes).1.stringify()
    }
    fn answer(status: u16, headers: &[(&str, &str)], body: &str) -> Response {
        Response { status, headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body: body.to_string() }
    }
    const CANON: &str = "<html><head><link rel=\"canonical\" href=\"https://example.test/a\"></head></html>";

    #[test]
    fn a_listed_url_that_redirects_is_reported_and_not_called_an_error_page() {
        let redirect = (format!("{ORIGIN}/old"), answer(301, &[("location", "/new")], ""));
        let out = with(vec![redirect, page("/a", 200, &[], "Page a title", None)], Probes { listed: true, ..Probes::default() });
        assert!(out.contains("B1867") && out.contains("301 to /new") && out.contains("the sitemap lists"), "{out}");
        assert!(!out.contains("B1856"), "{out}");
    }
    #[test]
    fn a_canonical_other_than_the_sitemap_url_is_reported_only_for_listed_pages() {
        let other = || vec![(format!("{ORIGIN}/b"), answer(200, &[], CANON))];
        assert!(with(other(), Probes { listed: true, ..Probes::default() }).contains("B1868"));
        assert!(!with(other(), Probes::default()).contains("B1868"), "a named path may be a variant on purpose");
        let same = vec![(format!("{ORIGIN}/a"), answer(200, &[], CANON))];
        assert!(!with(same, Probes { listed: true, ..Probes::default() }).contains("B1868"));
        let header = vec![(format!("{ORIGIN}/a"), answer(200, &[("link", "<https://example.test/z>; rel=\"canonical\"")], ""))];
        assert!(with(header, Probes { listed: true, ..Probes::default() }).contains("https://example.test/z"), "the Link header counts");
    }
    #[test]
    fn url_variants_are_fine_when_they_redirect_or_point_at_the_clean_url() {
        assert_eq!(variant_urls(ORIGIN, "/a"), [format!("{ORIGIN}//a"), format!("{ORIGIN}/A"), format!("{ORIGIN}/a/index.html"), format!("{ORIGIN}/a?utm_source=x")]);
        assert_eq!(variant_urls(ORIGIN, "/"), [format!("{ORIGIN}/index.html"), format!("{ORIGIN}/?utm_source=x")]);
        let clean = format!("{ORIGIN}/a");
        let probe = |r: Response| Probes { variants: vec![(clean.clone(), format!("{ORIGIN}//a"), r)], ..Probes::default() };
        assert!(with(vec![], probe(answer(200, &[], "<html></html>"))).contains("B1869"));
        assert!(!with(vec![], probe(answer(200, &[], CANON))).contains("B1869"));
        assert!(!with(vec![], probe(answer(301, &[("location", "/a")], ""))).contains("B1869"));
        assert!(!with(vec![], probe(answer(404, &[], ""))).contains("B1869"));
    }
    #[test]
    fn declared_caching_is_compared_with_what_was_observed() {
        let cached = [("cache-control", "public, max-age=600"), ("cf-cache-status", "DYNAMIC")];
        let url = format!("{ORIGIN}/a");
        let second = |status: &str| Probes { second: Some((url.clone(), "DYNAMIC".into(), answer(200, &[("cache-control", "public, max-age=600"), ("cf-cache-status", status)], ""))), ..Probes::default() };
        assert!(with(vec![page("/a", 200, &cached, "Page a title", None)], second("MISS")).contains("B1870"));
        assert!(!with(vec![page("/a", 200, &cached, "Page a title", None)], second("HIT")).contains("B1870"));
        let cookie = [("cache-control", "public, max-age=600"), ("set-cookie", "s=1")];
        assert!(with(vec![page("/a", 200, &cookie, "Page a title", None)], Probes::default()).contains("sets a cookie"));
        let private = [("cache-control", "private, max-age=600"), ("set-cookie", "s=1")];
        assert!(!with(vec![page("/a", 200, &private, "Page a title", None)], Probes::default()).contains("B1870"));
    }
    #[test]
    fn a_private_page_from_the_cache_and_the_rules_a_cdn_injects_are_reported() {
        let served = |cache: &'static str, status: &'static str| vec![page("/a", 200, &[("cache-control", cache), ("cf-cache-status", status)], "Page a title", None)];
        assert!(with(served("private, no-store", "HIT"), Probes::default()).contains("served it from its cache"));
        assert!(!with(served("private, no-store", "DYNAMIC"), Probes::default()).contains("served it from its cache"));
        assert!(!with(served("public, max-age=600", "HIT"), Probes::default()).contains("served it from its cache"));
        let rules = |value: &'static str| vec![page("/a", 200, &[("speculation-rules", value)], "Page a title", None)];
        assert!(with(rules("\"/cdn-cgi/speculation\""), Probes::default()).contains("Speed Brain"));
        assert!(!with(rules("\"/_b/rules.json\""), Probes::default()).contains("Speed Brain"));
    }
    #[test]
    fn immutable_needs_a_hashed_name_and_text_assets_need_an_encoding() {
        let immutable = [("cache-control", "public, max-age=31536000, immutable"), ("content-encoding", "br")];
        let asset = |name: &str, h: &[(&str, &str)], len: usize| Probes { assets: vec![(format!("{ORIGIN}{name}"), answer(200, h, &"x".repeat(len)))], ..Probes::default() };
        assert!(with(vec![], asset("/app.js", &immutable, 10)).contains("B1870"));
        assert!(!with(vec![], asset("/assets/app.3f2a1b9c.js", &immutable, 10)).contains("B1870"));
        assert!(with(vec![], asset("/app.3f2a1b9c.css", &[], 5000)).contains("uncompressed"));
        assert!(!with(vec![], asset("/app.3f2a1b9c.css", &[], 100)).contains("B1870"), "too small to matter");
        assert!(!with(vec![], asset("/app.3f2a1b9c.css", &[("content-encoding", "gzip")], 5000)).contains("B1870"));
        assert_eq!(text_assets(ORIGIN, "<link rel=\"stylesheet\" href=\"/a.css\"><script type=\"module\" src=\"/b.js\"></script><script src=\"https://cdn.test/c.js\"></script>"), [format!("{ORIGIN}/a.css"), format!("{ORIGIN}/b.js")]);
    }
    #[test]
    fn a_blocked_noindex_is_reported_from_the_published_robots() {
        let pages = vec![page("/private/a", 200, &[], "Secret page", Some("noindex")), page("/a", 200, &[], "Page a title", None)];
        let out = with(pages, Probes { robots: Some("User-agent: *\nDisallow: /private\n".into()), ..Probes::default() });
        assert!(out.contains("B1871"), "{out}");
    }
    /// A real socket: the client must see the 301 instead of following it.
    #[test]
    fn the_real_client_keeps_a_redirect_visible() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let mut stream = stream.unwrap();
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let answer = if head.starts_with("GET /sitemap.xml") {
                    format!("HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-length: 0\r\n\r\n")
                } else {
                    "HTTP/1.1 301 Moved\r\nlocation: /sitemap.xml\r\nconnection: close\r\ncontent-length: 0\r\n\r\n".to_string()
                };
                let _ = stream.write_all(answer.as_bytes());
            }
        });
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let base = format!("http://127.0.0.1:{port}");
        assert_eq!(fetch(&runtime, &client, &format!("{base}/old")).unwrap().status, 301);
        assert_eq!(fetch_following(&runtime, &client, &format!("{base}/old")).unwrap().status, 200);
    }

    #[test]
    fn an_error_page_served_with_200_is_an_error_and_is_not_scored() {
        let (code, out) = codes(vec![page("/items/a", 200, &[], "An item with a real title", None), page("/items/b", 200, &[], "Could not load", Some("noindex"))], false, None);
        assert_eq!(code, 1);
        assert!(out.contains("B1856") && out.contains("Could not load"), "{out}");
        assert!(!out.contains("\"/items/b\""), "the error page is not audited as content: {out}");
    }
    #[test]
    fn a_failed_status_names_the_rate_limit() {
        let (code, out) = codes(vec![page("/a", 429, &[], "x", None)], false, None);
        assert_eq!(code, 1);
        assert!(out.contains("answers 429") && out.contains("rate limit"), "{out}");
    }
    #[test]
    fn a_site_wide_noindex_fails_unless_it_is_expected() {
        let both = || vec![page("/a", 200, &[("x-robots-tag", "noindex, nofollow")], "Page a title", None), page("/b", 200, &[("x-robots-tag", "noindex, nofollow")], "Page b title", None)];
        assert!(codes(both(), false, None).1.contains("B1858"));
        assert!(!codes(both(), true, None).1.contains("B1858"), "staging says so on purpose");
    }
    #[test]
    fn a_repeated_single_value_header_is_reported_and_a_repeated_csp_is_not() {
        let headers = [("x-frame-options", "DENY"), ("x-frame-options", "SAMEORIGIN"), ("content-security-policy", "a"), ("content-security-policy", "b")];
        let out = codes(vec![page("/a", 200, &headers, "Page a title", None)], false, None).1;
        assert!(out.contains("B1857") && out.contains("x-frame-options") && out.contains("DENY | SAMEORIGIN"), "{out}");
        assert!(!out.contains("`content-security-policy`"), "{out}");
    }
    #[test]
    fn a_missing_address_that_answers_200_is_a_soft_404() {
        let (_, out) = codes(vec![page("/a", 200, &[], "Page a title", None)], false, Some(page("/a-missing", 200, &[], "Page a title", None)));
        assert!(out.contains("B1859"), "{out}");
        let (_, out) = codes(vec![page("/a", 200, &[], "Page a title", None)], false, Some(page("/a-missing", 404, &[], "Not found", None)));
        assert!(!out.contains("B1859"), "{out}");
    }
    #[test]
    fn duplicate_titles_across_rendered_pages_use_the_build_rule_and_a_sample_ignores_its_blind_spots() {
        let out = codes(vec![page("/a", 200, &[], "The same title here", None), page("/b", 200, &[], "The same title here", None)], false, None).1;
        assert!(out.contains("B1814"), "{out}");
        assert!(!out.contains("B1813"), "a sample cannot call a page an orphan: {out}");
    }
    #[test]
    fn spread_covers_the_whole_list_and_locs_reads_a_sitemap() {
        let all: Vec<String> = (0..10).map(|i| format!("/{i}")).collect();
        assert_eq!(spread(&all, 3), ["/0", "/3", "/6"]);
        assert_eq!(spread(&all, 50).len(), 10);
        assert_eq!(locs("<urlset><url><loc>https://a.test/x?a=1&amp;b=2</loc></url></urlset>"), ["https://a.test/x?a=1&b=2"]);
    }
}
