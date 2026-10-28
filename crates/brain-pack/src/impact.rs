//! Δ bytes per view for the impact report on every save (INTERNALS §43, BUILD #201; TOOLCHAIN #200 shows it).
//! `measure` runs the production pipeline in measuring mode: the same module graph, minifier, tree shaking, CSS,
//! prerender and own gzip as `brain build`, without SEO files, the caching layer (no signature: it would change every
//! page's bytes on every edit) or server output, and it never fails because a view has `load` or `server:` functions. Its work is cached by content in `.brain/cache/` like the build,
//! so after a save only the touched chunks are minified and compressed again. `diff` compares two measurements; the
//! dev server keeps the previous one in memory (the first save compares against the first measurement).

use std::collections::BTreeMap;

/// Bytes one view downloads in production (its first route; a dynamic route counts once by its pattern).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewBytes {
    pub view: String,
    pub route: String,
    pub js_gz: usize,
    pub css_gz: usize,
    pub html_gz: usize,
    /// The page as the build writes it (prerendered for static views): TOOLCHAIN audits it for the SEO and a11y
    /// deltas of the impact report (#200). Not part of the byte comparison.
    pub html: String,
}

impl ViewBytes {
    pub fn total(&self) -> usize {
        self.js_gz + self.css_gz + self.html_gz
    }
}

/// A measurement: view → bytes, in view order.
pub type Measure = BTreeMap<String, ViewBytes>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewDelta {
    pub view: String,
    pub route: String,
    /// gz bytes before (0 for a new view) and after (0 for a removed one).
    pub before: usize,
    pub after: usize,
    pub js: i64,
    pub css: i64,
    pub html: i64,
}

impl ViewDelta {
    pub fn delta(&self) -> i64 {
        self.after as i64 - self.before as i64
    }
}

/// Measures every view (gz bytes of its JS, CSS and HTML). `post`, `server_*` and `site` are ignored; with
/// `input.shim` static views are measured with their prerendered HTML.
pub fn measure(
    input: &crate::Input,
    views: &[(String, String)],
    layouts_of: &dyn Fn(&str) -> Vec<String>,
    seo_of: &dyn Fn(&str) -> Vec<(String, String)>,
) -> Result<Measure, String> {
    let m = crate::Input {
        root: input.root.clone(),
        compile: input.compile,
        runtime_core: input.runtime_core.clone(),
        runtime_siblings: input.runtime_siblings.clone(),
        layer_order: input.layer_order.clone(),
        minify: true,
        site: None,
        shim: input.shim.clone(),
        generator: input.generator.clone(),
        server_module: None,
        cache: crate::cache::CacheCfg { sw_template: None, ..input.cache.clone() },
        server_mjs: None,
        progress: None,
        post: None,
    };
    let out = crate::build_inner(&m, views, layouts_of, seo_of, true)?;
    let mut res = Measure::new();
    for row in &out.report {
        let Some(page) = out.pages.iter().find(|p| p.route == row.route) else { continue };
        res.entry(page.view.clone()).or_insert_with(|| ViewBytes {
            view: page.view.clone(),
            route: row.route.clone(),
            js_gz: row.js_gz,
            css_gz: row.css_gz,
            html_gz: row.html_gz,
            html: page.html.clone(),
        });
    }
    Ok(res)
}

/// Views whose bytes changed (or that appeared / disappeared), largest change first.
pub fn diff(before: &Measure, after: &Measure) -> Vec<ViewDelta> {
    let mut out = Vec::new();
    let zero = |v: &ViewBytes| ViewBytes { js_gz: 0, css_gz: 0, html_gz: 0, html: String::new(), ..v.clone() };
    for v in before.keys().chain(after.keys().filter(|k| !before.contains_key(*k))) {
        let (a, b) = match (before.get(v), after.get(v)) {
            (Some(a), Some(b)) => (a.clone(), b.clone()),
            (Some(a), None) => (a.clone(), zero(a)),
            (None, Some(b)) => (zero(b), b.clone()),
            (None, None) => continue,
        };
        let same = a.js_gz == b.js_gz && a.css_gz == b.css_gz && a.html_gz == b.html_gz;
        if same && before.contains_key(v) && after.contains_key(v) {
            continue;
        }
        out.push(ViewDelta {
            view: v.clone(),
            route: b.route.clone(),
            before: if before.contains_key(v) { a.total() } else { 0 },
            after: if after.contains_key(v) { b.total() } else { 0 },
            js: b.js_gz as i64 - a.js_gz as i64,
            css: b.css_gz as i64 - a.css_gz as i64,
            html: b.html_gz as i64 - a.html_gz as i64,
        });
    }
    out.sort_by(|x, y| y.delta().abs().cmp(&x.delta().abs()).then_with(|| x.view.cmp(&y.view)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vb(view: &str, js: usize, css: usize) -> ViewBytes {
        ViewBytes { view: view.into(), route: format!("/{view}"), js_gz: js, css_gz: css, html_gz: 100, html: String::new() }
    }

    #[test]
    fn diff_reports_changed_new_and_removed_views() {
        let a: Measure = [vb("a", 1000, 200), vb("b", 500, 100), vb("gone", 10, 10)].into_iter().map(|v| (v.view.clone(), v)).collect();
        let b: Measure = [vb("a", 1300, 200), vb("b", 500, 100), vb("new", 40, 0)].into_iter().map(|v| (v.view.clone(), v)).collect();
        let d = diff(&a, &b);
        let views: Vec<&str> = d.iter().map(|x| x.view.as_str()).collect();
        assert_eq!(views, ["a", "new", "gone"], "{d:?}");
        assert_eq!((d[0].delta(), d[0].js, d[0].css), (300, 300, 0));
        assert_eq!((d[1].before, d[1].after), (0, 140));
        assert_eq!((d[2].before, d[2].after, d[2].delta()), (120, 0, -120));
        assert!(diff(&a, &a).is_empty());
    }
}
