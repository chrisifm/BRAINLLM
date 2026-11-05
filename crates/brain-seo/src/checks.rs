//! Page checks that report and never score (INTERNALS §33): each one states what Google documents, not a folk rule.

use crate::audit::{Finding, Level, anchor_internal, internal, norm, url_path};
use crate::focus::{fold, tokens};
use crate::html::Doc;
use crate::jsonld::{ARTICLES, typed_nodes, types};
use crate::{Config, Page};
use brain_lex::Json;
use std::collections::{BTreeSet, HashSet};

/// Googlebot reads the first 2 MB of an uncompressed HTML file; the warning starts before anything is cut.
pub const HTML_LIMIT: usize = 2 * 1024 * 1024;
pub const HTML_NEAR: usize = 1_700_000;

/// Link text that says nothing about the target, folded, in the languages the framework ships examples for.
const GENERIC: &[&str] = &[
    "click here", "here", "read more", "learn more", "more", "this link", "link", // English
    "haz clic aqui", "click aqui", "clic aqui", "aqui", "leer mas", "ver mas", "saber mas", // Spanish
    "clique aqui", "leia mais", "saiba mais", "ver mais", // Portuguese
    "cliquez ici", "ici", "lire la suite", "en savoir plus", // French
    "hier klicken", "hier", "weiterlesen", "mehr erfahren", // German
];
const SEARCH_SEGMENTS: &[&str] = &["search", "buscar", "busqueda", "pesquisa", "recherche", "suche"];
const LISTING_SEGMENTS: &[&str] = &["tag", "tags", "category", "categories", "topic", "topics"];
const SESSION_PARAMS: &[&str] = &["phpsessid", "jsessionid", "sessionid", "session_id", "sid"];

fn note(code: &'static str, level: Level, msg: String, fix: &str) -> Finding {
    Finding { code, level, msg, fix: fix.into() }
}

fn number_from_two(s: &str) -> bool {
    s.parse::<u32>().is_ok_and(|n| n >= 2)
}

/// The first page of the listing `route` belongs to, when `route` is a later page.
fn first_page(route: &str, doc: &Doc) -> Option<String> {
    let (path, query) = route.split_once('?').unwrap_or((route, ""));
    let by_query = query.split('&').any(|kv| kv.split_once('=').is_some_and(|(k, v)| matches!(k, "page" | "p") && number_from_two(v)));
    if by_query {
        return Some(norm(path));
    }
    let path = path.trim_end_matches('/');
    let (parent, last) = path.rsplit_once('/')?;
    if !number_from_two(last) {
        return None;
    }
    if let Some(base) = parent.strip_suffix("/page") {
        return Some(norm(base));
    }
    if parent == "/page" {
        return Some("/".into());
    }
    doc.link("prev").map(|_| norm(parent))
}

/// B1872: Google asks every page of a listing to keep its own canonical; pointing them at page 1 hides what they list.
fn canonical_to_first(cfg: &Config, route: &str, doc: &Doc) -> Option<Finding> {
    let base = first_page(route, doc)?;
    let c = doc.link("canonical")?;
    let target = match internal(cfg, route, c) {
        Some((p, _)) => p,
        None if cfg.site.is_none() => norm(url_path(c)?),
        None => return None,
    };
    let keeps_page = c.split_once('?').is_some_and(|(_, q)| q.split('&').any(|kv| kv.split_once('=').is_some_and(|(k, v)| matches!(k, "page" | "p") && number_from_two(v))));
    (target == base && !keeps_page).then(|| {
        note("B1872", Level::Error, format!("this later page of a listing has its canonical at the first page, `{c}`"), "give each page of a listing a canonical to itself; Google asks not to use the first page as the canonical")
    })
}

fn generic_anchors(cfg: &Config, route: &str, doc: &Doc) -> Option<Finding> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for a in doc.anchors.iter().filter(|a| a.in_content) {
        let text = a.text.trim();
        if anchor_internal(cfg, route, a).is_some() && GENERIC.contains(&tokens(text).join(" ").as_str()) {
            seen.insert(fold(text));
        }
    }
    (!seen.is_empty()).then(|| {
        note("B1873", Level::Info, format!("link text says nothing about its target: {}", seen.into_iter().collect::<Vec<_>>().join(", ")), "name what the reader gets at the other end; Google reads the link text to understand the target")
    })
}

fn date_mismatch(doc: &Doc) -> Option<Finding> {
    if doc.times.is_empty() {
        return None;
    }
    let day = |s: &str| s.chars().take(10).collect::<String>();
    let visible: Vec<String> = doc.times.iter().map(|t| day(t)).collect();
    let nodes = typed_nodes(doc);
    let node = nodes.iter().find(|n| types(n).iter().any(|t| ARTICLES.contains(&t.as_str())))?;
    let declared: Vec<String> = ["datePublished", "dateModified"].iter().filter_map(|k| node.get(k).and_then(Json::as_str)).map(day).collect();
    (!declared.is_empty() && !declared.iter().any(|d| visible.contains(d))).then(|| {
        note("B1874", Level::Warning, format!("structured data dates the page {}, and the visible dates say {}", declared.join(" / "), visible.join(", ")), "show the same date the JSON-LD declares; Google checks one against the other")
    })
}

/// Words the markup claims are on the page, by type.
fn claimed(node: &Json) -> Vec<String> {
    let text = |key: &str, n: &Json| n.get(key).and_then(Json::as_str).map(str::to_string);
    let entities = |n: &Json| -> Vec<Json> {
        match n.get("mainEntity") {
            Some(Json::Arr(a)) => a.clone(),
            Some(o @ Json::Obj(_)) => vec![o.clone()],
            _ => vec![],
        }
    };
    let mut out = Vec::new();
    for t in types(node) {
        match t.as_str() {
            t if ARTICLES.contains(&t) => out.extend(text("headline", node)),
            "DiscussionForumPosting" => out.extend(text("headline", node).or_else(|| text("text", node))),
            "Product" | "Event" | "Recipe" | "Course" => out.extend(text("name", node)),
            "JobPosting" => out.extend(text("title", node)),
            "FAQPage" | "QAPage" => out.extend(entities(node).iter().filter_map(|e| text("name", e))),
            _ => {}
        }
    }
    out
}

/// B1875: Google's structured data guidelines ask markup to describe what a reader can see on the page.
fn structured_not_on_page(doc: &Doc) -> Option<Finding> {
    let visible: HashSet<String> = tokens(&doc.text).into_iter().chain(doc.headings.iter().flat_map(|(_, t)| tokens(t))).collect();
    let absent: Vec<String> = typed_nodes(doc).iter().flat_map(claimed).filter(|c| !tokens(c).iter().all(|w| visible.contains(w))).collect();
    let first = absent.first()?;
    Some(note("B1875", Level::Warning, format!("structured data declares \"{first}\", which the page does not show"), "show that text on the page, or remove it from the JSON-LD: markup describes what the reader can see"))
}

fn url_form(cfg: &Config, route: &str, doc: &Doc) -> Vec<Finding> {
    let mut out = Vec::new();
    let path = route.split('?').next().unwrap_or(route);
    let plain = path.split('/').filter(|s| !s.starts_with(':'));
    if plain.clone().any(|s| s.contains('_')) {
        out.push(note("B1877", Level::Info, "the URL separates words with `_`".into(), "Google suggests hyphens: they read as word separators"));
    }
    if plain.clone().any(|s| s.chars().any(char::is_uppercase)) {
        out.push(note("B1877", Level::Info, "the URL has uppercase letters".into(), "use lowercase: URLs are case-sensitive, and each spelling is a separate address"));
    }
    let session = |u: &str| {
        let lower = u.to_ascii_lowercase();
        lower.contains(";jsessionid=") || lower.split_once('?').is_some_and(|(_, q)| q.split('&').any(|kv| SESSION_PARAMS.contains(&kv.split('=').next().unwrap_or(""))))
    };
    let linked = doc.anchors.iter().find(|a| anchor_internal(cfg, route, a).is_some() && session(&a.href));
    if session(route) || linked.is_some() {
        out.push(note("B1877", Level::Info, format!("`{}` carries a session parameter", linked.map_or(route, |a| a.href.as_str())), "keep the session in a cookie: each session otherwise makes a new address to crawl"));
    }
    out
}

fn listing_value(cfg: &Config, route: &str, doc: &Doc, body: bool) -> Option<Finding> {
    let segments: Vec<&str> = route.split('?').next().unwrap_or(route).split('/').filter(|s| !s.is_empty()).collect();
    let last = fold(segments.last()?);
    if SEARCH_SEGMENTS.contains(&last.as_str()) {
        return Some(note("B1878", Level::Warning, "this looks like an internal search view and it is indexable".into(), "mark it robots=\"noindex\": a results page offers a search engine nothing of its own"));
    }
    if !body || segments.len() < 2 || !LISTING_SEGMENTS.contains(&fold(segments[segments.len() - 2]).as_str()) {
        return None;
    }
    let me = norm(route);
    let entries: BTreeSet<String> = doc.anchors.iter().filter(|a| a.in_content).filter_map(|a| anchor_internal(cfg, route, a)).map(|(p, _)| p).filter(|p| *p != me).collect();
    (entries.len() <= 1).then(|| note("B1878", Level::Info, format!("this listing links to {} page(s)", entries.len()), "wait until the listing has several entries, or mark it robots=\"noindex\" until then"))
}

/// Findings of one page. `body` is false for a server template, whose content is filled in per request.
pub fn page(cfg: &Config, p: &Page, doc: &Doc, indexable: bool, body: bool) -> Vec<Finding> {
    let mut out = Vec::new();
    out.extend(canonical_to_first(cfg, &p.route, doc));
    if !indexable {
        return out;
    }
    if p.html.len() >= HTML_NEAR {
        let past = p.html.len() > HTML_LIMIT;
        out.push(note("B1876", Level::Warning, format!("the HTML weighs {} KB, {} the 2 MB Googlebot reads", p.html.len() / 1024, if past { "past" } else { "close to" }), "move inline data and styles out of the document so the content stays inside the part that is read"));
    }
    out.extend(url_form(cfg, &p.route, doc));
    out.extend(listing_value(cfg, &p.route, doc, body));
    if body {
        out.extend(generic_anchors(cfg, &p.route, doc));
        out.extend(date_mismatch(doc));
        out.extend(structured_not_on_page(doc));
    }
    out
}

fn glob(pattern: &[u8], path: &[u8]) -> bool {
    match pattern.split_first() {
        None => true,
        Some((b'$', [])) => path.is_empty(),
        Some((b'*', rest)) => (0..=path.len()).any(|i| glob(rest, &path[i..])),
        Some((c, rest)) => path.split_first().is_some_and(|(d, tail)| c == d && glob(rest, tail)),
    }
}

/// What robots.txt says to Googlebot about `path`: its own group if there is one, else `*`; the longest rule wins and
/// `Allow` wins a tie, as RFC 9309 states.
pub fn robots_blocks(robots: &str, path: &str) -> bool {
    let mut groups: Vec<(Vec<String>, Vec<(bool, String)>)> = Vec::new();
    let mut in_agents = false;
    for line in robots.lines() {
        let line = line.split('#').next().unwrap_or("");
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        match key.as_str() {
            "user-agent" => {
                if !in_agents {
                    groups.push((Vec::new(), Vec::new()));
                }
                in_agents = true;
                if let Some(g) = groups.last_mut() {
                    g.0.push(value.to_ascii_lowercase());
                }
            }
            "allow" | "disallow" => {
                in_agents = false;
                if let (Some(g), false) = (groups.last_mut(), value.is_empty()) {
                    g.1.push((key == "allow", value.to_string()));
                }
            }
            _ => {}
        }
    }
    let of = |name: &str| groups.iter().filter(|g| g.0.iter().any(|a| a == name)).flat_map(|g| g.1.iter()).collect::<Vec<_>>();
    let has_own = groups.iter().any(|g| g.0.iter().any(|a| a == "googlebot"));
    let rules = if has_own { of("googlebot") } else { of("*") };
    rules.iter().filter(|(_, p)| glob(p.as_bytes(), path.as_bytes())).max_by_key(|(allow, p)| (p.len(), *allow)).is_some_and(|(allow, _)| !allow)
}

/// B1871: `(route, finding)` for each `noindex` page robots.txt blocks: a crawler that may not read a page never sees its `noindex`.
pub fn robots_conflicts(pages: &[Page], robots: &str) -> Vec<(String, Finding)> {
    pages
        .iter()
        .filter(|p| !p.route.starts_with(['*', '!']))
        .filter(|p| crate::html::parse(&p.html).meta("robots").is_some_and(|r| r.to_ascii_lowercase().contains("noindex")) && robots_blocks(robots, &p.route))
        .map(|p| (p.route.clone(), note("B1871", Level::Warning, "this page is `noindex` and robots.txt blocks it, so crawlers never read the `noindex` and may keep listing the URL".into(), "remove the `Disallow` for this path and keep the `noindex`")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::robots_blocks;

    #[test]
    fn robots_rules_follow_the_longest_match_and_the_googlebot_group() {
        let r = "User-agent: *\nDisallow: /private\nAllow: /private/open\nDisallow: /*.pdf$\n\nUser-agent: GPTBot\nDisallow: /\n";
        assert!(robots_blocks(r, "/private/x"));
        assert!(!robots_blocks(r, "/private/open/x"));
        assert!(robots_blocks(r, "/a/b.pdf"));
        assert!(!robots_blocks(r, "/a/b.pdf.html"));
        assert!(!robots_blocks(r, "/public"), "a rule for another crawler does not count");
        assert!(!robots_blocks("User-agent: *\nDisallow: /\n\nUser-agent: Googlebot\nDisallow: /x\n", "/y"), "Googlebot obeys its own group");
        assert!(!robots_blocks("User-agent: *\nDisallow:\n", "/y"));
    }
}
