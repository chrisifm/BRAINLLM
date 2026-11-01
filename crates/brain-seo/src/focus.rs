//! `seo.focus` (INTERNALS §33): the few terms a site wants to be known for, most relevant first. The audit reports where
//! each term lives and how pages link to it. Every finding is advice: crawlable links and descriptive anchors are what
//! Google documents, this scheme is not a documented ranking factor, and a page about another subject owes the term nothing.

use crate::audit::{Finding, Level, internal, norm, percent_decode};
use crate::html::Doc;
use crate::{Page, Site};

/// Terms a site can give real attention to. More is a warning; every term is still audited.
pub const LIMIT: usize = 3;
/// Same line as the focus keyword (B1816): a heuristic for repetition, since no search engine publishes a density.
const DENSITY: f32 = 3.0;
const DENSITY_MIN_WORDS: usize = 100;
/// "Nearly all pages": the share of titles, from this many indexable pages on.
const TITLE_SHARE: f32 = 0.9;
const TITLE_MIN_PAGES: usize = 5;

/// Lowercase without diacritics, so that a term matches however the page writes its accents (composed or combining).
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        let plain = match c as u32 {
            0x300..=0x36F => continue,
            0xE0..=0xE5 | 0x100..=0x105 => 'a',
            0xE7 | 0x106..=0x10D => 'c',
            0xF0 | 0x10E..=0x111 => 'd',
            0xE8..=0xEB | 0x112..=0x11B => 'e',
            0x11C..=0x123 => 'g',
            0x124..=0x127 => 'h',
            0xEC..=0xEF | 0x128..=0x131 => 'i',
            0x134..=0x135 => 'j',
            0x136..=0x137 => 'k',
            0x139..=0x142 => 'l',
            0xF1 | 0x143..=0x148 => 'n',
            0xF2..=0xF6 | 0xF8 | 0x14C..=0x151 => 'o',
            0x154..=0x159 => 'r',
            0x15A..=0x161 => 's',
            0x162..=0x167 => 't',
            0xF9..=0xFC | 0x168..=0x173 => 'u',
            0x174..=0x175 => 'w',
            0xFD | 0xFF | 0x176..=0x178 => 'y',
            0x179..=0x17E => 'z',
            0xDF => {
                out.push_str("ss");
                continue;
            }
            0xE6 => {
                out.push_str("ae");
                continue;
            }
            0x153 => {
                out.push_str("oe");
                continue;
            }
            _ => c,
        };
        out.push(plain);
    }
    out
}

/// Folded words of `text`.
pub fn tokens(text: &str) -> Vec<String> {
    fold(text).split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

fn has(words: &[String], term: &[String]) -> bool {
    !term.is_empty() && words.windows(term.len()).any(|w| w == term)
}

/// What the audit knows about one term.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Term {
    pub term: String,
    /// Indexable pages whose content mentions it.
    pub mentions: usize,
    /// Pages carrying it in the URL, the title and the h1.
    pub hubs: Vec<String>,
    /// Pages linking to the only hub, and the links among them whose text names the term.
    pub inbound: usize,
    pub inbound_anchors: usize,
    pub anchors_with_term: usize,
    /// Pages that mention the term and do not link to the only hub.
    pub unlinked: Vec<String>,
}

struct Words {
    text: Vec<String>,
    title: Vec<String>,
    h1: Vec<String>,
    route: Vec<String>,
}

fn finding(code: &'static str, level: Level, msg: String, fix: &str) -> Finding {
    Finding { code, level, msg, fix: fix.into() }
}

fn sample(list: &[String]) -> String {
    let shown: Vec<&str> = list.iter().take(5).map(String::as_str).collect();
    let more = list.len().saturating_sub(shown.len());
    if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") }
}

/// `(report per term, site findings, findings of each page)`. `paired` tells language versions of one page apart from
/// pages that really compete.
pub fn analyze(site: &Site, docs: &[Doc], idx: &[bool], paired: &dyn Fn(&str, &str) -> bool) -> (Vec<Term>, Vec<Finding>, Vec<Vec<Finding>>) {
    let terms = &site.cfg.focus;
    let mut per_page: Vec<Vec<Finding>> = vec![Vec::new(); docs.len()];
    let mut out = Vec::new();
    let mut found = Vec::new();
    if terms.is_empty() {
        return (out, found, per_page);
    }
    if terms.len() > LIMIT {
        found.push(finding(
            "B1861",
            Level::Warning,
            format!("seo.focus lists {} terms; attention spread over more than {LIMIT} rarely shows on any of them", terms.len()),
            "keep the most relevant terms first and move the rest to the `keyword` of their own pages",
        ));
    }
    let words: Vec<Words> = site.pages.iter().zip(docs).map(|(p, d): (&Page, &Doc)| Words {
        text: tokens(&d.text),
        title: tokens(d.title.as_deref().unwrap_or("")),
        h1: d.headings.iter().filter(|(l, _)| *l == 1).flat_map(|(_, t)| tokens(t).into_iter().chain([String::new()])).collect(),
        route: tokens(&percent_decode(&p.route)),
    }).collect();
    let indexable = idx.iter().filter(|i| **i).count();
    let home = site.pages.iter().position(|p| norm(&p.route) == "/");
    let mut seen: Vec<Vec<String>> = Vec::new();
    for term in terms {
        let t = tokens(term);
        if t.is_empty() || seen.contains(&t) {
            continue;
        }
        seen.push(t.clone());
        let mut row = Term { term: term.clone(), ..Term::default() };
        let live = |i: usize| idx[i] && !site.pages[i].ssr;
        if let Some(h) = home
            && !has(&words[h].title, &t)
            && (site.pages[h].ssr || !has(&words[h].h1, &t))
        {
            found.push(finding("B1862", Level::Warning, format!("focus term \"{term}\" is in neither the title nor the h1 of the home page"), "say what the site is about in the home title or h1, in words a reader would use"));
        }
        let mut hubs: Vec<usize> = Vec::new();
        for i in (0..docs.len()).filter(|i| idx[*i]) {
            let w = &words[i];
            // a template read from a server page has no body, so its h1 is unknown
            if has(&w.route, &t) && has(&w.title, &t) && (site.pages[i].ssr || has(&w.h1, &t)) && !hubs.iter().any(|h| paired(&site.pages[*h].route, &site.pages[i].route)) {
                hubs.push(i);
            }
        }
        row.hubs = hubs.iter().map(|i| site.pages[*i].route.clone()).collect();
        match hubs.len() {
            0 => found.push(finding("B1863", Level::Warning, format!("focus term \"{term}\" has no hub: no indexable page carries it in its URL, title and h1"), "give the term one page of its own, with the term in the route, the title and the h1")),
            1 => {}
            _ => found.push(finding("B1864", Level::Warning, format!("focus term \"{term}\" has {} hubs: {}", hubs.len(), sample(&row.hubs)), "give each page its own focus keyword, or merge the pages and redirect one")),
        }
        let mentioned: Vec<usize> = (0..docs.len()).filter(|i| live(*i) && has(&words[*i].text, &t)).collect();
        row.mentions = mentioned.len();
        if let [hub] = hubs[..] {
            let target = norm(&site.pages[hub].route);
            for (i, (p, d)) in site.pages.iter().zip(docs).enumerate().filter(|(i, _)| *i != hub) {
                let links: Vec<_> = d.anchors.iter().filter(|a| internal(site.cfg, &p.route, &a.href).is_some_and(|(to, _)| to == target)).collect();
                if links.is_empty() {
                    if mentioned.contains(&i) {
                        row.unlinked.push(p.route.clone());
                    }
                    continue;
                }
                row.inbound += 1;
                row.inbound_anchors += links.len();
                row.anchors_with_term += links.iter().filter(|a| has(&tokens(&a.text), &t)).count();
            }
            if row.inbound == 0 && docs.len() > 1 {
                found.push(finding("B1865", Level::Warning, format!("no page links to {target}, the hub of focus term \"{term}\""), "link the hub with a crawlable <a href> from the pages that talk about the term, with descriptive link text"));
            } else if !row.unlinked.is_empty() || row.anchors_with_term < row.inbound_anchors {
                let linking = row.mentions.saturating_sub(row.unlinked.len() + usize::from(mentioned.contains(&hub)));
                found.push(finding(
                    "B1865",
                    Level::Info,
                    format!("focus term \"{term}\": {linking} of {} pages that mention it link to {target}; {} of {} links name it{}", linking + row.unlinked.len(), row.anchors_with_term, row.inbound_anchors, if row.unlinked.is_empty() { String::new() } else { format!("; without a link: {}", sample(&row.unlinked)) }),
                    "where a page really talks about the term, link the hub with text that says what it is; no page has to mention it",
                ));
            }
        }
        let titled = (0..docs.len()).filter(|i| idx[*i] && has(&words[*i].title, &t)).count();
        if indexable >= TITLE_MIN_PAGES && titled as f32 >= indexable as f32 * TITLE_SHARE {
            found.push(finding("B1866", Level::Warning, format!("focus term \"{term}\" is in the title of {titled} of {indexable} pages"), "title each page after its own subject; Google's spam policies name keyword stuffing, and repeated titles hide what sets a page apart"));
        }
        for &i in &mentioned {
            let total = words[i].text.len();
            let hits = words[i].text.windows(t.len()).filter(|w| *w == t).count();
            let density = if total == 0 { 0.0 } else { (hits * t.len()) as f32 * 100.0 / total as f32 };
            if total >= DENSITY_MIN_WORDS && density > DENSITY {
                per_page[i].push(finding("B1866", Level::Warning, format!("focus term \"{term}\" is {density:.1} % of this page ({hits} uses in {total} words)"), "write for the reader and use other words for the same idea; Google's spam policies name keyword stuffing"));
            }
        }
        out.push(row);
    }
    (out, found, per_page)
}

#[cfg(test)]
mod tests {
    use super::{fold, tokens};

    #[test]
    fn folding_ignores_case_and_accents_composed_or_combining() {
        assert_eq!(fold("A\u{f1}O M\u{c1}S"), "ano mas");
        assert_eq!(fold("Mu\u{308}nchen"), fold("M\u{fc}nchen"));
        assert_eq!(fold("Stra\u{df}e \u{152}uvre"), "strasse oeuvre");
        assert_eq!(tokens("/caf\u{e9}-de-a\u{f1}o"), ["cafe", "de", "ano"]);
        assert_eq!(fold("\u{65e5}\u{672c}"), "\u{65e5}\u{672c}", "other scripts stay as they are");
    }
}
