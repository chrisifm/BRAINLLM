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
const SAMPLE_BLIND: [&str; 4] = ["B1812", "B1813", "B1819", "B1820"];
/// One value only: a second copy is ignored, or makes the browser discard the policy.
const SINGLE_VALUE: [&str; 5] = ["x-frame-options", "x-content-type-options", "permissions-policy", "referrer-policy", "strict-transport-security"];
const USAGE: &str = "usage: brain seo audit --url <origin> [--sample N] [--path /route]... [--delay-ms N] [--expect-noindex] [--json]";

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
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
pub fn report(origin: &str, cfg: &mut brain_seo::Config, sampled: bool, expect_noindex: bool, pages: Vec<(String, Response)>, missing: Option<(String, Response)>) -> (i32, Json, String) {
    cfg.site = Some(origin.to_string());
    let mut live = check_pages(&pages, expect_noindex);
    if let Some((url, response)) = &missing {
        live.extend(check_missing(url, response));
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
    runtime.block_on(async {
        let response = client.get(url).send().await.map_err(|e| format!("{url}: {e}"))?;
        let status = response.status().as_u16();
        let headers = response.headers().iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
        let body = response.text().await.map_err(|e| format!("{url}: {e}"))?;
        Ok(Response { status, headers, body })
    })
}

fn sitemap_paths(runtime: &tokio::runtime::Runtime, client: &reqwest::Client, origin: &str) -> Result<Vec<String>, String> {
    let index = fetch(runtime, client, &format!("{origin}/sitemap.xml"))?;
    if index.status != 200 {
        return Err(format!("{origin}/sitemap.xml answers {}; name the pages with --path", index.status));
    }
    let mut urls = Vec::new();
    for loc in locs(&index.body) {
        if loc.ends_with(".xml") {
            let nested = fetch(runtime, client, &loc)?;
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
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(30)).user_agent(concat!("brain-seo-audit/", env!("CARGO_PKG_VERSION"))).build() else {
        return fail(2, "could not start the HTTP client".into());
    };
    let named: Vec<String> = args.iter().enumerate().filter(|(i, _)| *i > 0 && args[i - 1] == "--path").map(|(_, a)| a.clone()).chain(args.iter().filter_map(|a| a.strip_prefix("--path=").map(str::to_string))).collect();
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
    let (code, value, text) = report(&origin, &mut cfg, true, expect_noindex, pages, missing);
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
        let (code, json, _) = report(ORIGIN, &mut cfg, true, expect, pages, missing);
        (code, json.stringify())
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
