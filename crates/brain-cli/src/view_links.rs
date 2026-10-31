//! What `brain check` can see in the sources before a build: B0776 a static link to a path nothing answers (the build
//! stops on it as B1819), one finding per source line; B0777 a view title that repeats what `titleTemplate` adds.
use crate::compiler::Diag;
use brain_check::seo::{SeoValue, view_info};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn sources(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for path in entries.flatten().map(|e| e.path()) {
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|x| x == "brain") {
            out.push(path);
        }
    }
}

/// The path a static `href` asks this app for, or None when it is not this checker's to judge: another site, a
/// fragment, a generated file (`/feed.xml`) or an endpoint of the server.
fn internal(href: &str) -> Option<&str> {
    let path = href.split(['?', '#']).next()?;
    let own = path.starts_with('/') && !path.starts_with("//");
    let last = path.rsplit('/').next()?;
    let endpoint = ["/api/", "/_brain/", "/auth/"].iter().any(|p| path.starts_with(p));
    (own && !last.contains('.') && !endpoint).then_some(path)
}

/// `Pricing · Acme` or `Acme` under `%s · Acme`; `About Acme` reads as a sentence and is left alone.
fn repeats(title: &str, suffix: &str) -> bool {
    title == suffix || title.strip_suffix(suffix).is_some_and(|rest| rest.trim_end().ends_with(['·', '-', '|', '–', '—', ':']))
}

fn warning(code: &'static str, file: &str, line: u32, message: String, fix: &str) -> Diag {
    let mut d = Diag::new(code, file, brain_parse::Span::default(), message, fix.to_string());
    d.line = line.max(1) as _;
    d.col = 1;
    d.level = brain_parse::Severity::Warning;
    d
}

pub fn check(app: &Path) -> Vec<Diag> {
    let mut out = Vec::new();
    let routes: Vec<String> = brain_pack::routes::discover(app).into_iter().map(|r| r.path).filter(|p| p != "*").collect();
    let locales: Vec<String> = brain_pack::read_i18n(app).map_or(Vec::new(), |(_, all)| all);
    let redirects = std::fs::read_to_string(app.join("redirects.json")).unwrap_or_default();
    let answers = |path: &str| {
        let path = path.trim_end_matches('/');
        let path = if path.is_empty() { "/" } else { path };
        let bare = path.strip_prefix('/').and_then(|p| p.split_once('/').map_or(Some((p, "")), Some)).filter(|(first, _)| locales.iter().any(|l| l == first)).map(|(_, rest)| format!("/{rest}"));
        let known = |p: &str| {
            routes.iter().any(|r| r == p || brain_seo::audit::matches_pattern(r, p))
                || app.join("public").join(p.trim_start_matches('/')).exists()
                || redirects.contains(&format!("\"{p}\""))
        };
        known(path) || bare.as_deref().is_some_and(known)
    };
    let template = std::fs::read_to_string(app.join("app.brain")).ok().and_then(|src| brain_parse::parse(&src, Path::new("app.brain")).ok()).and_then(|c| c.meta.iter().find(|m| m.name == "titleTemplate").and_then(|m| m.value.clone()));
    let suffix = template.map(|t| t.replace("%s", "").trim_matches(|c: char| !c.is_alphanumeric()).to_string()).filter(|s| !s.is_empty());
    let mut files = vec![app.join("app.brain")];
    sources(&app.join("views"), &mut files);
    sources(&app.join("comp"), &mut files);
    for file in files {
        let Ok(src) = std::fs::read_to_string(&file) else { continue };
        let rel = file.strip_prefix(app).unwrap_or(&file).to_string_lossy().replace('\\', "/");
        let Ok(c) = brain_parse::parse(&src, Path::new(&rel)) else { continue }; // the compiler already reported it
        let info = view_info(&c);
        let mut by_line: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for (href, _, external, span) in &info.links {
            if !external && internal(href).is_some_and(|p| !answers(p)) {
                let hrefs = by_line.entry(span.line).or_default();
                if !hrefs.contains(href) {
                    hrefs.push(href.clone());
                }
            }
        }
        for (line, hrefs) in by_line {
            let list = hrefs.iter().map(|h| format!("`{h}`")).collect::<Vec<_>>().join(", ");
            out.push(warning("B0776", &rel, line, format!("no view answers {list}"), "fix the href, create the view, or add a redirect in redirects.json"));
        }
        if rel.starts_with("views/")
            && let Some(suffix) = &suffix
            && let Some((_, SeoValue::Text(title))) = info.attrs.iter().find(|(k, _)| k == "title")
            && repeats(title, suffix)
        {
            let line = src.lines().position(|l| l.contains("<meta seo")).map_or(1, |n| n as u32 + 1);
            out.push(warning("B0777", &rel, line, format!("the title `{title}` already includes `{suffix}`, which `titleTemplate` adds again"), "leave only the page's own name in the title"));
        }
    }
    out
}
