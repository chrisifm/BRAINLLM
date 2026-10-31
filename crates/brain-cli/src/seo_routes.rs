//! B1837: public dynamic routes rendered per request that no sitemap lists. Crawlers find such pages only through
//! links or sitemap.xml, so the build names each one with the fix (`sitemap({ env })`, `paths()` or `noindex`).
use std::path::Path;

/// `(route, view)` of every dynamic view that is public (no `_guard.jsbrain` above it, not `_`-prefixed, not
/// `noindex`, not an error view) and exports neither `paths()` nor `sitemap()`.
pub fn unlisted(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for r in brain_pack::routes::discover(root) {
        if r.path == "*" || !r.path.contains([':', '*']) {
            continue;
        }
        let name = r.view.rsplit('/').next().unwrap_or("");
        if name.starts_with('_') || ["404.brain", "410.brain", "500.brain"].contains(&name) || guarded(root, &r.view) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(root.join(&r.view)) else { continue };
        if src.contains("noindex") || src.contains("function paths(") || src.contains("function sitemap(") {
            continue;
        }
        out.push((r.path, r.view));
    }
    out
}

fn guarded(root: &Path, view: &str) -> bool {
    let mut dir = root.to_path_buf();
    let parts: Vec<&str> = view.split('/').collect();
    parts[..parts.len().saturating_sub(1)].iter().any(|p| {
        dir.push(p);
        dir.join("_guard.jsbrain").is_file()
    })
}

/// The B1837 warning line for one route, in the build's `CODE file:line:col msg; fix: …` shape.
pub fn warning(route: &str, view: &str) -> String {
    format!(
        "B1837 {view}:1:1 dynamic route `{route}` renders per request but no sitemap lists its pages, so crawlers only reach them through links; fix: export `async function sitemap({{ env }})` returning the params of the pages to index (or `paths()` to prerender them), or add `<meta seo robots=\"noindex\">` if they should stay out of search"
    )
}
