//! Impact report on every save (INTERNALS §43.1): after each save in `brain run`, one compact JSON record — the
//! diagnostics the save introduced or fixed (with their edits), the Δ gz bytes and the SEO score change of every view it
//! changed, measured
//! with the production pipeline (`brain_pack::impact`, cached by content, so only touched chunks are redone). The
//! agent reads it instead of rerunning doctor. Plain `brain run` prints it as an `impact {…}` line; MCP `brain_impact`
//! returns the last 50. Mount time and updates per interaction join when the runtime reports them (RUNTIME #198).

use brain_mcp::Json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const KEEP: usize = 50;
const MAX_PENDING: usize = 128;
const MAX_TRACKED: usize = 512;
#[derive(Default)]
struct Pending {
    files: BTreeSet<String>,
    running: bool,
    overflow: bool,
}
impl Pending {
    fn enqueue(&mut self, file: String) -> bool {
        if self.files.len() < MAX_PENDING || self.files.contains(&file) {
            self.files.insert(file);
        } else {
            self.overflow = true;
        }
        if self.running {
            false
        } else {
            self.running = true;
            true
        }
    }
}
fn source_path(root: &Path, file: &str) -> Option<String> {
    if file.len() > 1024 || file.contains(['\\', '?', '#']) {
        return None;
    }
    let path = Path::new(file)
        .strip_prefix(root)
        .unwrap_or_else(|_| Path::new(file.trim_start_matches('/')));
    if path
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_string_lossy().into_owned())
}

#[derive(Default)]
struct State {
    /// last measurement (the first save compares against the one taken at start)
    bytes: Option<brain_pack::impact::Measure>,
    /// view → its SEO score in that measurement (brain-seo over the HTML the build would write)
    seo: BTreeMap<String, Option<u32>>,
    /// file → its last diagnostics (code|line|col → JSON)
    diags: BTreeMap<String, BTreeMap<String, Json>>,
    records: VecDeque<String>,
    seq: u64,
    backend: Option<serde_json::Value>,
}

/// One per dev server; cheap to clone.
#[derive(Clone)]
pub struct Tracker {
    root: PathBuf,
    st: Arc<Mutex<State>>,
    /// serializes the measurements (a burst of saves runs one after another, never in parallel)
    busy: Arc<Mutex<()>>,
    print: bool,
    pending: Arc<Mutex<Pending>>,
}

impl Tracker {
    /// Takes the baseline in the background. `print`: write each record to stdout as `impact {…}`.
    pub fn new(root: &Path, print: bool) -> Tracker {
        let t = Tracker {
            root: root.to_path_buf(),
            st: Arc::default(),
            busy: Arc::default(),
            print,
            pending: Arc::default(),
        };
        t.st.lock().unwrap().backend = crate::backend_diff::snapshot(root).ok();
        let t2 = t.clone();
        std::thread::spawn(move || {
            let _g = t2.busy.lock().unwrap();
            let t0 = std::time::Instant::now();
            let m = measure(&t2.root).ok();
            let views = m.as_ref().map_or(0, |m| m.len());
            let seo = m
                .as_ref()
                .map(|m| seo_scores(&t2.root, m))
                .unwrap_or_default();
            let mut st = t2.st.lock().unwrap();
            st.bytes = m;
            st.seo = seo;
            drop(st);
            // saves from now on report Δ bytes against this baseline
            if t2.print {
                println!(
                    "impact {{\"baseline\":true,\"views\":{views},\"ms\":{}}}",
                    t0.elapsed().as_millis()
                );
            }
        });
        t
    }

    /// A dev server event (`update` with `path`, or `error` with `diags`): measures in the background.
    pub fn event(&self, ev: &str) {
        if ev.len() > 65536 {
            return;
        }
        let Ok(j) = Json::parse(ev) else { return };
        let file = match j.get("type").and_then(Json::as_str) {
            Some("update" | "reload") => j
                .get("path")
                .and_then(Json::as_str)
                .map(|p| p.trim_start_matches('/').to_string()),
            Some("error") => match j.get("diags") {
                Some(Json::Arr(d)) => d
                    .first()
                    .and_then(|d| d.get("file"))
                    .and_then(Json::as_str)
                    .map(str::to_string),
                _ => None,
            },
            _ => None,
        };
        let Some(file) = file.filter(|f| {
            f.ends_with(".brain") || f.ends_with(".jsbrain") || f.ends_with(".cssbrain")
        }) else {
            return;
        };
        let Some(file) = source_path(&self.root, &file) else {
            return;
        };
        if !self.pending.lock().unwrap().enqueue(file) {
            return;
        }
        let t = self.clone();
        std::thread::spawn(move || {
            loop {
                let next = {
                    let mut pending = t.pending.lock().unwrap();
                    match pending.files.pop_first() {
                        Some(file) => Some(file),
                        None => {
                            pending.running = false;
                            None
                        }
                    }
                };
                match next {
                    Some(file) => t.record(&file),
                    None => break,
                }
            }
        });
    }

    fn record(&self, file: &str) {
        let _g = self.busy.lock().unwrap();
        let t0 = std::time::Instant::now();
        // diagnostics of the saved file: what this save introduced and what it fixed
        let now: BTreeMap<String, Json> = crate::run_check_base(Some(self.root.join(file)))
            .map(|(_, ds, _)| {
                ds.iter()
                    .map(|d| {
                        (
                            format!("{}|{}|{}", d.code, d.line, d.col),
                            Json::parse(&d.to_json()).unwrap_or(Json::Null),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let after = measure(&self.root).ok();
        let mut st = self.st.lock().unwrap();
        let backend = if file.starts_with("server/resources/")
            || file.starts_with("server/jobs/")
            || file.starts_with("server/auth/")
        {
            match crate::backend_diff::snapshot(&self.root) {
                Ok(after) => {
                    let change = crate::backend_diff::diff(st.backend.as_ref(), &after);
                    st.backend = Some(after);
                    change
                }
                Err(_) => {
                    serde_json::json!({"schemaVersion":1,"valid":false,"runtimeState":"unchanged; invalid source contract","fix":"Run brain backend schema --json and correct the declared schema, policy, hooks or roles before restarting the backend"})
                }
            }
        } else {
            serde_json::Value::Null
        };
        let backend = Json::parse(&backend.to_string()).unwrap_or(Json::Null);
        let evicted = st.diags.len() >= MAX_TRACKED && !st.diags.contains_key(file);
        if evicted {
            st.diags.pop_first();
        }
        let incomplete = {
            let mut pending = self.pending.lock().unwrap();
            std::mem::take(&mut pending.overflow) || evicted
        };
        let before = st
            .diags
            .insert(file.to_string(), now.clone())
            .unwrap_or_default();
        let new: Vec<Json> = now
            .iter()
            .filter(|(k, _)| !before.contains_key(*k))
            .map(|(_, v)| v.clone())
            .collect();
        let fixed: Vec<Json> = before
            .iter()
            .filter(|(k, _)| !now.contains_key(*k))
            .map(|(k, _)| Json::str(k.split('|').next().unwrap_or("")))
            .collect();
        let bytes: Vec<Json> = match (&st.bytes, &after) {
            (Some(a), Some(b)) => brain_pack::impact::diff(a, b)
                .into_iter()
                .map(|d| {
                    Json::obj([
                        ("view", Json::str(&d.view)),
                        ("route", Json::str(&d.route)),
                        ("before", Json::Num(d.before as f64)),
                        ("after", Json::Num(d.after as f64)),
                        ("delta", Json::Num(d.delta() as f64)),
                    ])
                })
                .collect(),
            _ => vec![],
        };
        // SEO score per view that changed with this save (a11y regressions arrive as `new` diagnostics)
        let mut seo: Vec<Json> = Vec::new();
        if let Some(m) = &after {
            let now_seo = seo_scores(&self.root, m);
            for (view, sc) in &now_seo {
                let before = st.seo.get(view).copied().flatten();
                if st.seo.contains_key(view) && before != *sc {
                    let num = |x: Option<u32>| x.map_or(Json::Null, |v| Json::Num(v as f64));
                    seo.push(Json::obj([
                        ("view", Json::str(view)),
                        ("before", num(before)),
                        ("after", num(*sc)),
                    ]));
                }
            }
            st.seo = now_seo;
        }
        if after.is_some() {
            st.bytes = after;
        }
        st.seq += 1;
        let rec = Json::obj([
            ("seq", Json::Num(st.seq as f64)),
            ("file", Json::str(file)),
            ("incomplete", Json::Bool(incomplete)),
            ("pendingLimit", Json::Num(MAX_PENDING as f64)),
            ("backendContract", backend),
            (
                "errors",
                Json::Num(
                    now.values()
                        .filter(|d| {
                            d.get("level").and_then(Json::as_str).unwrap_or("error") == "error"
                        })
                        .count() as f64,
                ),
            ),
            ("new", Json::Arr(new)),
            ("fixed", Json::Arr(fixed)),
            ("bytes", Json::Arr(bytes)),
            ("seo", Json::Arr(seo)),
            ("ms", Json::Num(t0.elapsed().as_millis() as f64)),
        ])
        .stringify();
        if st.records.len() >= KEEP {
            st.records.pop_front();
        }
        st.records.push_back(rec.clone());
        if self.print {
            println!("impact {rec}");
        }
    }

    /// The last `n` records, newest last (`brain_impact`).
    pub fn last(&self, n: usize) -> Vec<String> {
        let st = self.st.lock().unwrap();
        st.records
            .iter()
            .skip(st.records.len().saturating_sub(n))
            .cloned()
            .collect()
    }
}

/// The SEO score brain-seo gives each measured view (one audit over all pages, so duplicate titles count).
fn seo_scores(root: &Path, m: &brain_pack::impact::Measure) -> BTreeMap<String, Option<u32>> {
    let Ok(cfg) = brain_seo::Config::load(root) else {
        return BTreeMap::new();
    };
    let pages: Vec<brain_seo::Page> = m
        .values()
        .filter(|v| !v.html.is_empty())
        .map(|v| brain_seo::Page {
            route: v.route.clone(),
            file: if v.route == "/" {
                "index.html".into()
            } else {
                format!("{}/index.html", v.route.trim_matches('/'))
            },
            html: v.html.clone(),
            lastmod: None,
            keyword: None,
            article: None,
            view: Some(v.view.clone()),
            ssr: false,
        })
        .collect();
    let routes: Vec<String> = m.values().map(|v| v.route.clone()).collect();
    let site = brain_seo::Site {
        cfg: &cfg,
        pages: &pages,
        routes: &routes,
        assets: &[],
        redirects: &[],
        images: &[],
        i18n: None,
    };
    let report = brain_seo::audit(&site);
    m.values()
        .map(|v| {
            (
                v.view.clone(),
                report
                    .pages
                    .iter()
                    .find(|p| p.route == v.route)
                    .and_then(|p| p.score),
            )
        })
        .collect()
}

/// gz bytes per view, as `brain build` would ship them (no SEO files, no signature, no server output).
pub fn measure(root: &Path) -> Result<brain_pack::impact::Measure, String> {
    let rt = crate::runtime_files();
    let shim = rt
        .iter()
        .find(|(n, _)| n == "shim/dom.js")
        .map(|(_, b)| b.clone());
    let core_src = rt
        .iter()
        .find(|(n, _)| n == "brain.js")
        .map(|(_, b)| b.clone())
        .ok_or("runtime brain.js is not embedded")?;
    let core = brain_min::minify_runtime_core(&core_src)?;
    let siblings: Vec<(String, String)> = rt
        .into_iter()
        .filter(|(n, _)| {
            (n.ends_with(".js")
                && !n.contains('/')
                && n != "brain-dev.js"
                && n != "brain-inspect.js"
                && n != "brain-why.js"
                && n != "brain-mock.js"
                && n != "brain-overlay.js")
                || (n.starts_with("ui/") && n.ends_with(".brain"))
        })
        .collect();
    let r2 = root.to_path_buf();
    let compile = move |src: &str, rel: &Path| -> Result<brain_pack::Unit, String> {
        let o = crate::compiler::compile_hash(src, rel, &r2, false, 4)
            .map_err(|ds| format!("{} does not compile ({} error(s))", rel.display(), ds.len()))?;
        Ok(brain_pack::Unit {
            js: o.js,
            css: o.css,
            server_js: o.server_js,
            server_fns: o.server_fns,
        })
    };
    let input = brain_pack::Input {
        server_module: None,
        cache: crate::cache_cfg(root)?,
        server_mjs: None,
        progress: None,
        post: None,
        root: root.to_path_buf(),
        compile: &compile,
        runtime_core: core,
        runtime_siblings: siblings,
        layer_order: crate::head_prepend(),
        minify: true,
        site: None,
        shim,
        generator: String::new(),
    };
    // the same views as `brain build` (every routable file under views/)
    let views: Vec<(String, String)> = brain_pack::routes::discover(root)
        .into_iter()
        .filter_map(|r| crate::map::route_of(&r.view).map(|p| (p, r.view)))
        .collect();
    // the static `<meta seo>` values, as the build passes them (evaluated seo(props) comes from the prerender)
    let seo_of = |v: &str| -> Vec<(String, String)> {
        let Some(c) = std::fs::read_to_string(root.join(v))
            .ok()
            .and_then(|src| brain_parse::parse(&src, Path::new(v)).ok())
        else {
            return Vec::new();
        };
        brain_check::seo::view_info(&c)
            .attrs
            .into_iter()
            .filter_map(|(k, val)| match val {
                brain_check::seo::SeoValue::Text(t) => Some((k, t)),
                brain_check::seo::SeoValue::Expr(_) => None,
            })
            .collect()
    };
    brain_pack::impact::measure(&input, &views, &|v| crate::layouts_of(root, v), &seo_of)
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    #[test]
    fn save_bursts_coalesce_and_bound_pending_work_with_visible_overflow() {
        let mut pending = Pending::default();
        assert!(pending.enqueue("views/home.brain".into()));
        for _ in 0..10000 {
            assert!(!pending.enqueue("views/home.brain".into()));
        }
        assert_eq!(pending.files.len(), 1);
        for i in 0..10000 {
            assert!(!pending.enqueue(format!("comp/item-{i}.brain")));
        }
        assert_eq!(pending.files.len(), MAX_PENDING);
        assert!(pending.overflow);
    }
    #[test]
    fn hmr_urls_and_source_paths_resolve_inside_the_project() {
        let root = Path::new("/project");
        assert_eq!(
            source_path(root, "/views/home.brain").as_deref(),
            Some("views/home.brain")
        );
        assert_eq!(
            source_path(root, "/project/comp/card.brain").as_deref(),
            Some("comp/card.brain")
        );
        for file in [
            "/../outside.brain",
            "views/../outside.brain",
            "views/home.brain?token=private",
            "C:\\private.brain",
        ] {
            assert!(source_path(root, file).is_none());
        }
    }
}
