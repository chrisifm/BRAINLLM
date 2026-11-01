//! SEO audit corpus: each case in `corpus/audit.txt` edits a clean three-page site and states the exact findings, as
//! `<code> <route>`. Audit findings belong to a built page, not to a source line, so they carry no line or column.
//! `@unreachable` names the B18xx codes this audit cannot produce from one site, with the reason.
use brain_seo::{Config, Page, Site, audit, redirects};
use std::collections::BTreeSet;

const WORDS: &str = "Teams plan their week with this offline task planner. ";

fn page(route: &str, title: &str, desc: &str, body: &str) -> Page {
    let canonical = format!("https://acme.dev{route}");
    Page {
        route: route.into(),
        file: if route == "/" { "index.html".into() } else { format!("{}/index.html", route.trim_matches('/')) },
        html: format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title><meta name=\"description\" content=\"{desc}\"><link rel=\"canonical\" href=\"{canonical}\"><meta property=\"og:title\" content=\"{title}\"><meta property=\"og:image\" content=\"https://acme.dev/og.png\"><meta name=\"viewport\" content=\"width=device-width\"></head><body><nav><a href=\"/\">Home</a> <a href=\"/pricing\">Pricing</a> <a href=\"/guide\">Guide</a></nav><main>{body}</main></body></html>"
        ),
        ..Page::default()
    }
}

fn clean() -> Vec<Page> {
    let words = WORDS.repeat(40);
    let cite = "<p>Read the <a href=\"https://www.w3.org/standards/\">standards</a>.</p>";
    vec![
        page("/", "Acme offline task planner for small teams", "The Acme offline planner lets you plan the week, assign tasks and track progress. Free for small teams.", &format!("<h1>Acme offline planner</h1><p>A short opening.</p><p>{words}</p>{cite}<p>See the <a href=\"/pricing\">prices</a>.</p>")),
        page("/pricing", "Pricing for the Acme offline task planner", "Simple pricing for the offline task planner: free for small teams, one flat price for larger ones.", &format!("<h1>Pricing of the planner</h1><p>A short opening.</p><p>{words}</p>{cite}<p>See the <a href=\"/guide\">guide</a>.</p>")),
        page("/guide", "Guide to planning with an offline task planner", "A guide to offline planning: why it beats a web-only tool when the network drops, with a setup checklist.", &format!("<h1>Guide to offline planning</h1><p>A short opening.</p><p>{words}</p>{cite}<p>See the <a href=\"/pricing\">prices</a>.</p>")),
    ]
}

#[derive(Default)]
struct Case {
    name: String,
    edits: Vec<(String, String, String)>,
    keywords: Vec<(String, String)>,
    redirects: Vec<String>,
    staging: bool,
    focus: Vec<String>,
    robots: Option<String>,
    pad: Vec<(String, usize)>,
    add: Vec<String>,
    expect: Vec<String>,
}

fn parse(src: &str) -> (Vec<Case>, Vec<String>) {
    let (mut cases, mut unreachable) = (Vec::<Case>::new(), vec![]);
    for line in src.lines() {
        if let Some(name) = line.strip_prefix("### ") {
            cases.push(Case { name: name.into(), ..Case::default() });
        } else if let Some(rest) = line.strip_prefix("@unreachable ") {
            unreachable.push(rest.split(' ').next().unwrap().to_string());
        } else if let Some(case) = cases.last_mut() {
            if let Some(want) = line.strip_prefix("@expect ") {
                case.expect.push(want.into());
            } else if let Some(rest) = line.strip_prefix("@replace ") {
                // `@replace <route|*> <old> => <new>`
                let (route, rest) = rest.split_once(' ').expect("@replace <route> <old> => <new>");
                let (old, new) = rest.split_once(" =>").expect("@replace needs =>");
                case.edits.push((route.into(), old.into(), new.trim_start().into()));
            } else if let Some(rest) = line.strip_prefix("@keyword ") {
                let (route, kw) = rest.split_once(' ').unwrap();
                case.keywords.push((route.into(), kw.into()));
            } else if let Some(rest) = line.strip_prefix("@redirect ") {
                case.redirects.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("@focus ") {
                case.focus.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("@robots ") {
                case.robots = Some(rest.replace("\\n", "\n"));
            } else if let Some(rest) = line.strip_prefix("@pad ") {
                let (route, bytes) = rest.split_once(' ').unwrap();
                case.pad.push((route.into(), bytes.parse().unwrap()));
            } else if let Some(rest) = line.strip_prefix("@page ") {
                case.add.push(rest.to_string());
            } else if line == "@staging" {
                case.staging = true;
            }
        }
    }
    (cases, unreachable)
}

fn run(case: &Case) -> Vec<String> {
    let mut pages = clean();
    // `@page <route> | <title> | <h1>`: one more page, linked from the navigation of no other
    for add in &case.add {
        let parts: Vec<&str> = add.split(" | ").collect();
        let desc = format!("{}: what this page offers, in a sentence long enough to read as a real result.", parts[2]);
        pages.push(page(parts[0], parts[1], &desc, &format!("<h1>{}</h1><p>A short opening.</p>", parts[2])));
    }
    for (route, old, new) in &case.edits {
        let mut hit = false;
        for p in pages.iter_mut().filter(|p| route == "*" || p.route == *route) {
            hit |= p.html.contains(old.as_str());
            p.html = p.html.replace(old.as_str(), new);
        }
        assert!(hit, "{}: `{old}` matches nothing", case.name);
    }
    for (route, bytes) in &case.pad {
        let p = pages.iter_mut().find(|p| p.route == *route).unwrap();
        p.html = p.html.replace("</main>", &format!("<!--{}--></main>", "x".repeat(*bytes)));
    }
    for (route, kw) in &case.keywords {
        pages.iter_mut().find(|p| p.route == *route).unwrap().keyword = Some(kw.clone());
    }
    let cfg = Config { site: Some("https://acme.dev".into()), name: Some("Acme".into()), staging: case.staging, focus: case.focus.clone(), ..Config::default() };
    let rules = if case.redirects.is_empty() { vec![] } else { redirects::parse(&case.redirects.join("\n")).unwrap() };
    let report = audit(&Site { cfg: &cfg, pages: &pages, routes: &[], assets: &[], redirects: &rules, images: &[], i18n: None });
    let mut got: Vec<String> = report.pages.iter().flat_map(|p| p.findings.iter().map(move |f| format!("{} {}", f.code, p.route))).collect();
    got.extend(report.site.iter().map(|f| format!("{} *", f.code)));
    if let Some(robots) = &case.robots {
        got.extend(brain_seo::checks::robots_conflicts(&pages, robots).into_iter().map(|(r, f)| format!("{} {r}", f.code)));
    }
    for p in &report.pages {
        for f in &p.findings {
            assert!(!f.msg.is_empty() && !f.fix.is_empty(), "{}: {} has no msg or fix", case.name, f.code);
        }
    }
    got.sort();
    got.dedup();
    got
}

#[test]
fn every_case_reports_exactly_its_codes_and_routes() {
    let (cases, _) = parse(include_str!("corpus/audit.txt"));
    let mut bad = vec![];
    for case in &cases {
        let mut want = case.expect.clone();
        want.sort();
        let got = run(case);
        if got != want {
            bad.push(format!("{}\n  want {want:?}\n  got  {got:?}", case.name));
        }
    }
    assert!(bad.is_empty(), "{} of {} cases differ:\n{}", bad.len(), cases.len(), bad.join("\n"));
}

#[test]
fn every_code_of_the_family_has_a_case_or_a_reason() {
    let (cases, unreachable) = parse(include_str!("corpus/audit.txt"));
    let covered: BTreeSet<&str> = cases.iter().flat_map(|c| c.expect.iter().map(|e| e.split(' ').next().unwrap())).collect();
    let missing: Vec<&str> = brain_seo::audit::CATALOG.iter().map(|c| c.0).filter(|c| c.starts_with("B18") && !covered.contains(c) && !unreachable.iter().any(|u| u == c)).collect();
    assert!(missing.is_empty(), "no case and no @unreachable reason for {missing:?}");
}
