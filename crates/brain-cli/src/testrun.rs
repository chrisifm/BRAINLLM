//! `brain test`: headless Chrome by default. A connected Viewer (Claude Viewer or Codex Viewer) runs the declared
//! scenarios only when the user opts in (`--viewer` or `BRAIN_VIEWER=1`); an opted-in Viewer failure never becomes a
//! headless success, and an unavailable opted-in Viewer falls back explicitly.
//! Headless CDP retains full AX-tree/forced-GC support; Viewer reports its DOM-label-only audit separately.

use crate::cdp::Chrome;
use brain_mcp::Json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Validate the selected source import graph before reusing a dev server or opening a browser.
/// Watcher dependency lists include discovery inventories and unrelated provider readers, so do not crawl them.
pub fn preflight(root: &Path, files: &[PathBuf]) -> Result<(), String> {
    use brain_parse::ast::Stmt;
    use brain_check::discover::Resolution;
    use std::collections::{BTreeSet, VecDeque};
    use std::io::Read;
    let root = root.canonicalize().map_err(|error| format!("test preflight: {error}"))?;
    let mut queue: VecDeque<PathBuf> = files.iter().cloned().collect();
    let mut seen = BTreeSet::new();
    let mut bytes = 0usize;
    while let Some(path) = queue.pop_front() {
        let path = path.canonicalize().map_err(|error| format!("test preflight could not read {}: {error}", path.display()))?;
        if !path.starts_with(&root) { return Err("test preflight: an imported source is outside the app; fix: keep test imports inside the project".into()); }
        if !seen.insert(path.clone()) { continue; }
        if seen.len() > 4096 { return Err("test preflight: source graph exceeds 4096 files; fix: select a smaller test folder".into()); }
        let rel = path.strip_prefix(&root).unwrap();
        let mut source = String::new();
        if !std::fs::metadata(&path).is_ok_and(|meta| meta.is_file()) { return Err(format!("test preflight: {} is not a regular source file", rel.display())); }
        let file = std::fs::File::open(&path).map_err(|error| format!("test preflight: {}: {error}", rel.display()))?;
        if !file.metadata().is_ok_and(|meta| meta.is_file()) { return Err(format!("test preflight: {} is not a regular source file", rel.display())); }
        file.take(16 * 1024 * 1024 + 1).read_to_string(&mut source).map_err(|error| format!("test preflight: {}: {error}", rel.display()))?;
        bytes = bytes.saturating_add(source.len());
        if source.len() > 16 * 1024 * 1024 || bytes > 128 * 1024 * 1024 { return Err("test preflight: source size limit exceeded; fix: select a smaller test graph".into()); }
        if rel.extension().is_some_and(|ext| ext == "cssbrain") {
            let (diags, _) = crate::compiler::check(&source, rel, &path);
            let errors: Vec<_> = diags.into_iter().filter(|diag| !diag.is_warning()).collect();
            if !errors.is_empty() { return Err(format!("test preflight failed: {}", crate::compiler::Diag::list_to_json(&errors))); }
            continue;
        }
        // Test scripts are served directly with browser-provided test globals; application modules use dev codegen.
        if !rel.to_string_lossy().ends_with(".test.brain") {
            crate::compiler::compile(&source, rel, &root, true).map_err(|diags| format!("test preflight failed on current source; fix these diagnostics before running browser assertions: {}", crate::compiler::Diag::list_to_json(&diags)))?;
        }
        let component = brain_parse::parse(&source, rel).map_err(|diags| crate::compiler::Diag::list_to_json(&diags))?;
        let dir = path.parent().unwrap();
        for statement in &component.script {
            if let Stmt::Import(import) = statement
                && (import.source.starts_with("./") || import.source.starts_with("../"))
                && matches!(Path::new(&import.source).extension().and_then(|ext| ext.to_str()), Some("brain" | "jsbrain" | "cssbrain"))
            { queue.push_back(dir.join(&import.source)); }
        }
        let (context, _) = brain_check::load_ctx(&path, &component);
        for resolution in context.auto_components.resolutions.values() {
            if let Resolution::Unique { source } = resolution { queue.push_back(dir.join(source)); }
        }
    }
    Ok(())
}

pub struct TestResult {
    pub name: String,
    pub ok: bool,
    pub ms: f64,
    pub error: Option<String>,
}

pub struct AxIssue {
    pub code: &'static str,
    pub role: String,
    pub node: String,
    pub test: String,
    pub msg: String,
    pub fix: &'static str,
    pub engine: &'static str,
}

pub struct FileReport {
    pub file: String,
    pub load_error: Option<String>,
    pub tests: Vec<TestResult>,
    pub a11y: Vec<AxIssue>,
    /// Runtime errors captured while its tests ran (brain-err.js: effects, handlers, unhandled rejections), deduplicated:
    /// `{type, msg, line, comp, n}`. Listed by `brain doctor` (SPEC: captured errors), they do not fail it by themselves.
    pub errors: Vec<Json>,
    /// `--heap` (and doctor): leaks the inspector found after the tests unmounted and the GC ran — B0601 (an instance
    /// alive after unmount) and B0602 (a global listener or interval in an fx without cleanup). They fail the file.
    pub leaks: Vec<Json>,
    pub engine: &'static str,
    pub fallback: Option<String>,
    pub a11y_mode: &'static str,
    pub heap_mode: &'static str,
    pub lanes: Vec<Json>,
}

impl FileReport {
    pub fn failed(&self) -> bool {
        self.load_error.is_some()
            || self.tests.iter().any(|t| !t.ok)
            || !self.a11y.is_empty()
            || !self.leaks.is_empty()
    }
    /// `failed()` without the leak watch: what `brain doctor` counts as failed tests, judging leaks on its own.
    pub fn failed_tests(&self) -> bool {
        self.load_error.is_some() || self.tests.iter().any(|t| !t.ok) || !self.a11y.is_empty()
    }
    pub fn to_json(&self) -> Json {
        let tests = self
            .tests
            .iter()
            .map(|t| {
                let mut kv = vec![
                    ("name".into(), Json::str(&t.name)),
                    ("ok".into(), Json::Bool(t.ok)),
                    ("ms".into(), Json::Num((t.ms * 10.0).round() / 10.0)),
                ];
                if let Some(e) = &t.error {
                    kv.push(("error".into(), Json::str(e)));
                }
                Json::Obj(kv)
            })
            .collect();
        let a11y = self
            .a11y
            .iter()
            .map(|a| {
                Json::obj([
                    ("code", Json::str(a.code)),
                    ("role", Json::str(&a.role)),
                    ("node", Json::str(&a.node)),
                    ("test", Json::str(&a.test)),
                    ("msg", Json::str(&a.msg)),
                    ("fix", Json::str(a.fix)),
                    ("engine", Json::str(a.engine)),
                ])
            })
            .collect();
        let mut kv = vec![("file".into(), Json::str(&self.file))];
        kv.push(("engine".into(), Json::str(self.engine)));
        kv.push(("lanes".into(), Json::Arr(self.lanes.clone())));
        kv.push((
            "fallback".into(),
            self.fallback
                .as_ref()
                .map(|value| Json::str(value))
                .unwrap_or(Json::Null),
        ));
        kv.push((
            "coverage".into(),
            Json::obj([
                ("accessibility", Json::str(self.a11y_mode)),
                ("heap", Json::str(self.heap_mode)),
            ]),
        ));
        if let Some(e) = &self.load_error {
            kv.push(("error".into(), Json::str(e)));
        }
        kv.push(("tests".into(), Json::Arr(tests)));
        kv.push(("a11y".into(), Json::Arr(a11y)));
        kv.push(("errors".into(), Json::Arr(self.errors.clone())));
        kv.push(("leaks".into(), Json::Arr(self.leaks.clone())));
        Json::Obj(kv)
    }
}

pub fn test_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || name == "dist" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            test_files(&p, out);
        } else if name.ends_with(".test.brain") {
            out.push(p);
        }
    }
}

/// Classifies an AX node with no accessible name according to the SPEC §8 codes.
fn ax_rule(role: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match role {
        "button" | "link" | "menuitem" | "tab" => Some((
            "B0406",
            "control without accessible text",
            "add visible text or `aria-label` to the element",
        )),
        "textbox" | "searchbox" | "checkbox" | "radio" | "combobox" | "listbox" | "slider"
        | "spinbutton" | "switch" => Some((
            "B0403",
            "form control without an accessible name",
            "wrap it in `<label>` or add `aria-label`",
        )),
        "image" | "img" => Some((
            "B0401",
            "image without alternative text",
            "add `alt` (or `alt=\"\"` if it is decorative)",
        )),
        _ => None,
    }
}

fn describe(chrome: &mut Chrome, session: &str, backend: &Json) -> String {
    let Ok(r) = chrome.call(
        Some(session),
        "DOM.describeNode",
        Json::obj([("backendNodeId", backend.clone())]),
    ) else {
        return "?".into();
    };
    let Some(node) = r.get("node") else {
        return "?".into();
    };
    let mut s = node
        .get("localName")
        .and_then(Json::as_str)
        .unwrap_or("?")
        .to_string();
    if let Some(Json::Arr(attrs)) = node.get("attributes") {
        for pair in attrs.chunks(2) {
            if let [Json::Str(k), Json::Str(v)] = pair {
                match k.as_str() {
                    "id" => s.push_str(&format!("#{v}")),
                    "class" => v
                        .split_whitespace()
                        .for_each(|c| s.push_str(&format!(".{c}"))),
                    "type" | "name" | "role" => s.push_str(&format!("[{k}=\"{v}\"]")),
                    _ => {}
                }
            }
        }
    }
    s
}

pub(crate) fn audit(
    chrome: &mut Chrome,
    session: &str,
    test: &str,
    out: &mut Vec<AxIssue>,
) -> Result<(), String> {
    let tree = chrome.call(
        Some(session),
        "Accessibility.getFullAXTree",
        Json::Obj(vec![]),
    )?;
    let Some(Json::Arr(nodes)) = tree.get("nodes") else {
        return Ok(());
    };
    for n in nodes {
        if n.get("ignored") == Some(&Json::Bool(true)) {
            continue;
        }
        let role = n
            .get("role")
            .and_then(|r| r.get("value"))
            .and_then(Json::as_str)
            .unwrap_or("");
        let name = n
            .get("name")
            .and_then(|r| r.get("value"))
            .and_then(Json::as_str)
            .unwrap_or("");
        let Some((code, msg, fix)) = ax_rule(role) else {
            continue;
        };
        if !name.trim().is_empty() {
            continue;
        }
        let node = n
            .get("backendDOMNodeId")
            .map_or_else(|| "?".into(), |b| describe(chrome, session, b));
        // The same failure on the same node is reported once per file.
        if out.iter().any(|a| a.code == code && a.node == node) {
            continue;
        }
        out.push(AxIssue {
            code,
            role: role.into(),
            node,
            test: test.into(),
            msg: msg.into(),
            fix,
            engine: "headless-cdp",
        });
    }
    Ok(())
}

/// Runs the files against the server at `base` (e.g. `http://127.0.0.1:53211`).
/// How long unmounted tests wait for their in-flight requests before aborting them and counting leaks.
#[cfg(unix)]
const SETTLE_MS: u64 = 3000;
/// How long an unmounted instance may stay alive before it is suspected: work that begins after the unmount (a promise
/// chained on a settled call, a script load) is invisible to the network watch, and it ends on its own. Spent only when
/// an instance is still alive, so a clean file does not pay for it.
const RELEASE_GRACE_MS: u64 = 10_000;

/// `brain test --heap` / `brain doctor`: watch for leaks while the tests run (§9, §18).
static HEAP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_heap(on: bool) {
    HEAP.store(on, std::sync::atomic::Ordering::SeqCst);
}

/// The Viewer lane is opt-in: never probed or opened unless the user asked for it.
static VIEWER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_viewer(on: bool) {
    VIEWER.store(on, std::sync::atomic::Ordering::SeqCst);
}
/// The adapter's own code and fix (fixed English strings, never browser data) so a Viewer failure is actionable.
fn viewer_hint(v: &serde_json::Value) -> String {
    match (v["code"].as_str(), v["fix"].as_str()) {
        (Some(code), Some(fix)) => format!(" ({code}: {fix})"),
        (Some(code), None) => format!(" ({code})"),
        _ => String::new(),
    }
}
/// `--viewer` on the command line, or `BRAIN_VIEWER=1|true|yes` in the process environment.
pub fn viewer_requested(args: &[String], env: Option<&str>) -> bool {
    args.iter().any(|a| a == "--viewer")
        || env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

pub fn run(
    base: &str,
    root: &Path,
    files: &[PathBuf],
    timeout: Duration,
) -> Result<Vec<FileReport>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    if !VIEWER.load(std::sync::atomic::Ordering::SeqCst)
        && !viewer_requested(&[], std::env::var("BRAIN_VIEWER").ok().as_deref())
    {
        let mut reports = run_headless(base, root, files, timeout)?;
        for report in &mut reports {
            report.lanes = vec![Json::obj([
                ("engine", Json::str("headless-cdp")),
                ("role", Json::str("default; a Viewer lane runs only with --viewer or BRAIN_VIEWER=1")),
            ])];
        }
        return Ok(reports);
    }
    let available = crate::viewer::probe()?;
    if available["ok"] != true {
        return Err(format!(
            "Connected Viewer capability probe failed{}; no silent headless fallback was attempted",
            viewer_hint(&available)
        ));
    }
    if available["available"] == true {
        let response = crate::viewer::run(base, root, files, timeout)?;
        if response["ok"] != true {
            return Err(format!(
                "Connected Viewer E2E failed at {}{}; owned-tab cleanup status: {}; no silent headless fallback was attempted",
                response["stage"].as_str().unwrap_or("bridge"),
                viewer_hint(&response),
                response["cleanup"].as_str().unwrap_or("unknown")
            ));
        }
        let mut primary = viewer_reports(&response)?;
        let audit=run_headless(base,root,files,timeout).map_err(|_|"Viewer scenarios completed, but the required complementary headless AX/heap replay failed; the suite did not pass".to_string())?;
        if primary.len() != audit.len() {
            return Err("Viewer and headless audit returned different file counts".into());
        }
        for (report, mut replay) in primary.iter_mut().zip(audit) {
            if report.file != replay.file {
                return Err("Viewer and headless audit file identities differ".into());
            }
            report.lanes = vec![
                Json::obj([
                    ("engine", Json::str("viewer")),
                    ("role", Json::str("primary declared interactions")),
                    ("tests", Json::Num(report.tests.len() as f64)),
                    (
                        "accessibility",
                        Json::str("DOM-label-heuristic; not AX tree"),
                    ),
                ]),
                Json::obj([
                    ("engine", Json::str("headless-cdp")),
                    (
                        "role",
                        Json::str(
                            "separate declared-fixture replay for full AX and optional heap gates",
                        ),
                    ),
                    ("tests", Json::Num(replay.tests.len() as f64)),
                    (
                        "accessibility",
                        Json::str("Chrome Accessibility.getFullAXTree"),
                    ),
                    ("heap", Json::str(replay.heap_mode)),
                ]),
            ];
            if report.load_error.is_none() {
                report.load_error = replay.load_error.take();
            }
            for mut test in replay.tests {
                if !test.ok {
                    test.name = format!("Headless audit replay: {}", test.name);
                    report.tests.push(test);
                }
            }
            // Heuristics remain observable in their lane; only the actual AX replay drives the existing AX gate.
            if let Some(Json::Obj(fields)) = report.lanes.first_mut() {
                fields.push((
                    "domLabelObservations".into(),
                    Json::Arr(
                        report
                            .a11y
                            .iter()
                            .map(|issue| {
                                Json::obj([
                                    ("code", Json::str(issue.code)),
                                    ("role", Json::str(&issue.role)),
                                    ("test", Json::str(&issue.test)),
                                ])
                            })
                            .collect(),
                    ),
                ));
            }
            report.a11y = replay.a11y;
            report.errors = report
                .errors
                .drain(..)
                .map(|value| tag_lane(value, "viewer"))
                .chain(
                    replay
                        .errors
                        .into_iter()
                        .map(|value| tag_lane(value, "headless-cdp")),
                )
                .collect();
            report.leaks = replay
                .leaks
                .into_iter()
                .map(|value| tag_lane(value, "headless-cdp"))
                .collect();
            report.engine = "viewer+headless-cdp";
            report.a11y_mode = "Viewer DOM labels plus full Chrome AX tree in separate replay";
            report.heap_mode = replay.heap_mode;
        }
        return Ok(primary);
    }
    let reason = available["reason"].as_str().unwrap_or("Viewer unavailable");
    let mut reports = run_headless(base, root, files, timeout)?;
    for report in &mut reports {
        report.fallback = Some(reason.into());
        report.lanes = vec![Json::obj([
            ("engine", Json::str("headless-cdp")),
            ("role", Json::str("explicit fallback: Viewer requested but unavailable")),
            ("reason", Json::str(reason)),
        ])];
    }
    Ok(reports)
}

fn tag_lane(value: Json, lane: &str) -> Json {
    match value {
        Json::Obj(mut fields) => {
            fields.push(("engine".into(), Json::str(lane)));
            Json::Obj(fields)
        }
        other => Json::obj([("engine", Json::str(lane)), ("detail", other)]),
    }
}

fn viewer_reports(response: &serde_json::Value) -> Result<Vec<FileReport>, String> {
    let rows = response["reports"]
        .as_array()
        .filter(|r| r.len() <= 32)
        .ok_or("Viewer did not return bounded native file reports")?;
    rows.iter()
        .map(|row| {
            let tests = row["tests"]
                .as_array()
                .ok_or("Viewer tests are missing")?
                .iter()
                .map(|test| TestResult {
                    name: test["name"].as_str().unwrap_or("Declared scenario").into(),
                    ok: test["ok"] == true,
                    ms: test["ms"].as_f64().unwrap_or(0.0),
                    error: test["error"].as_str().map(str::to_string),
                })
                .collect();
            let a11y = row["a11y"]
                .as_array()
                .ok_or("Viewer DOM audit is missing")?
                .iter()
                .filter_map(|issue| {
                    let code = match issue["code"].as_str()? {
                        "B0406" => "B0406",
                        "B0403" => "B0403",
                        "B0401" => "B0401",
                        _ => return None,
                    };
                    let (_, msg, fix) = ax_rule(issue["role"].as_str()?)?;
                    Some(AxIssue {
                        code,
                        role: issue["role"].as_str()?.into(),
                        node: issue["node"].as_str()?.into(),
                        test: issue["test"].as_str().unwrap_or("Declared scenario").into(),
                        msg: msg.into(),
                        fix,
                        engine: "viewer-dom",
                    })
                })
                .collect();
            let errors = row["errors"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| Json::parse(&e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(FileReport {
                file: row["file"].as_str().ok_or("Viewer file is missing")?.into(),
                load_error: row["error"].as_str().map(str::to_string),
                tests,
                a11y,
                errors,
                leaks: Vec::new(),
                engine: "viewer",
                fallback: None,
                a11y_mode: if row["auditTruncated"] == true {
                    "DOM-label-heuristic; truncated; not AX tree"
                } else {
                    "DOM-label-heuristic; not AX tree"
                },
                heap_mode: "not-measured",
                lanes: Vec::new(),
            })
        })
        .collect()
}

fn run_headless(
    base: &str,
    root: &Path,
    files: &[PathBuf],
    timeout: Duration,
) -> Result<Vec<FileReport>, String> {
    transport_retry(|| run_headless_once(base, root, files, timeout), Duration::from_secs(2), load_average)
}

/// A browser that dies on its pipe ("Broken pipe", "Chrome did not respond") is usually starved by machine load, not a
/// failed test: run the whole pass once more after `wait`, then name the probable cause with the 1-minute load average.
/// Any other error is returned as it is, without a retry.
fn transport_retry<T>(mut attempt: impl FnMut() -> Result<T, String>, wait: Duration, load: impl Fn() -> Option<f64>) -> Result<T, String> {
    let transport = |e: &str| e.contains("CDP transport write failed") || e.contains("Chrome did not respond") || e.starts_with("pipe CDP:");
    match attempt() {
        Err(first) if transport(&first) => {
            std::thread::sleep(wait);
            attempt().map_err(|again| {
                if !transport(&again) {
                    return again;
                }
                let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
                let busy = match load() {
                    Some(avg) => format!("the 1-minute load average is {avg:.1} on {cores} cores"),
                    None => "the load average could not be read".to_string(),
                };
                format!("{again}. The browser connection failed twice; the probable cause is machine load ({busy}); fix: rerun when the machine is less busy, or raise --timeout")
            })
        }
        other => other,
    }
}

fn load_average() -> Option<f64> {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn getloadavg(loadavg: *mut f64, nelem: i32) -> i32;
        }
        let mut avg = [0f64; 1];
        if unsafe { getloadavg(avg.as_mut_ptr(), 1) } == 1 {
            return Some(avg[0]);
        }
    }
    None
}

fn test_navigation(error: &str) -> bool {
    error.contains("context destroyed by navigation") || error.contains("context was destroyed") || error.contains("navigated or closed") || error.contains("Cannot find context with specified id") || error.contains("Session with given id not found")
}

/// Close only a tab attached by this owned headless Chrome, including failed recovery tabs.
fn close_test_page(chrome: &mut Chrome, session: &str) {
    let _=chrome.close_page(session,Duration::from_secs(2));
}

fn open_test_page(chrome: &mut Chrome, base: &str, file: &str) -> Result<(String, Json), String> {
    let session=chrome.new_page()?;
    let loaded=(|| {
        for domain in ["Runtime.enable","Page.enable","Accessibility.enable"] { chrome.call(Some(&session),domain,Json::Obj(vec![]))?; }
        chrome.call(Some(&session),"Page.navigate",Json::obj([("url",Json::str(format!("{base}/@brain/test.html")))]))?;
        let mut ready=false;
        for _ in 0..200 {
            match chrome.eval(&session,"typeof window.__brainLoad === 'function'") {
                Ok(Json::Bool(true)) => { ready=true; break; }
                Err(error) if !test_navigation(&error) => return Err(error),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !ready { return Ok(Json::obj([("error",Json::str("the test page did not load /@brain/test-api.js"))])); }
        chrome.eval(&session,&format!("window.__brainLoad({})",Json::str(format!("/{file}")).stringify()))
    })();
    match loaded { Ok(loaded)=>Ok((session,loaded)),Err(error)=>{close_test_page(chrome,&session);Err(error)} }
}

fn plan_id(plan:&Json)->Result<&str,String> {
    plan.get("id").and_then(Json::as_str).filter(|id|!id.is_empty() && id.len()<=128 && id.bytes().all(|b|b.is_ascii_alphanumeric() || matches!(b,b'-'|b'.')))
        .ok_or_else(||"the local test API has no compatible document identity; fix: restart brain run with the current compiler before testing".into())
}
fn plan_matches(chrome:&mut Chrome,session:&str,plan:&Json)->Result<bool,String> {
    let id=plan_id(plan)?;
    match chrome.eval(session,&format!("typeof window.__brainMatches==='function' && window.__brainMatches({})",Json::str(id).stringify())) {
        Ok(Json::Bool(value))=>Ok(value),Ok(_)=>Ok(false),Err(error) if test_navigation(&error)=>Ok(false),Err(error)=>Err(error),
    }
}
fn navigation_message(file:&str,name:&str)->String {
    format!("{file}: test `{name}` lost its registered document during navigation; fix: do not reload or leave /@brain/test.html from mounted hooks or component actions; stub navigation in a component test and verify real navigation on an actual app page. Interrupted post-assertion audits and feedback are not measurements.")
}
fn fail_last_navigation(report:&mut FileReport) {
    if let Some(last)=report.tests.last_mut() {last.ok=false;last.error=Some(navigation_message(&report.file,&last.name));}
    else {report.load_error=Some(format!("{}: the test document navigated before its first registered scenario; fix: remove navigation from module initialization",report.file));}
}
#[derive(Debug)]
enum BoundStepError { Navigation { started:bool }, Other(String) }
fn bound_step<F>(chrome:&mut Chrome,session:&str,plan:&Json,index:usize,name:&str,audit_step:F)->Result<(Json,Vec<AxIssue>),BoundStepError>
where F:FnOnce(&mut Chrome,&str,&str,&mut Vec<AxIssue>)->Result<(),String> {
    let valid=|chrome:&mut Chrome|plan_matches(chrome,session,plan).map_err(BoundStepError::Other);
    if !valid(chrome)? {return Err(BoundStepError::Navigation{started:false})}
    let id=plan_id(plan).map_err(BoundStepError::Other)?;
    let expression=format!("typeof window.__brainStep==='function' ? window.__brainStep({index},{}) : ({{documentChanged:true,started:false}})",Json::str(id).stringify());
    let result=match chrome.eval(session,&expression) {Ok(value)=>value,Err(error) if test_navigation(&error)=>return Err(BoundStepError::Navigation{started:true}),Err(error)=>return Err(BoundStepError::Other(error))};
    if result.get("documentChanged")==Some(&Json::Bool(true)) {return Err(BoundStepError::Navigation{started:result.get("started")==Some(&Json::Bool(true))})}
    if !valid(chrome)? {return Err(BoundStepError::Navigation{started:true})}
    let mut issues=Vec::new();let audited=audit_step(chrome,session,name,&mut issues);
    if !valid(chrome)? {return Err(BoundStepError::Navigation{started:true})}
    audited.map_err(BoundStepError::Other)?;
    Ok((result,issues))
}
fn restore_test_page(chrome:&mut Chrome,base:&str,file:&str,names:&Json,heap:bool)->Result<Option<(String,Json)>,String> {
    let (session,plan)=open_test_page(chrome,base,file)?;
    if plan.get("error").is_some() || plan.get("names")!=Some(names) || plan_id(&plan).is_err() {
        close_test_page(chrome,&session);return Ok(None)
    }
    instrument_test_page(chrome,&session,heap);Ok(Some((session,plan)))
}

fn instrument_test_page(chrome: &mut Chrome, session: &str, heap: bool) {
    if heap { let _=chrome.eval(session,"import('/@brain/brain-inspect.js').then(m => (m.inspect({ noPanel: true }).holdVerdict(true), true), () => false)"); }
    let _=chrome.eval(session,"import('/@brain/brain-err.js').then(m => (m.capture({ log: false }), true), () => false)");
}

fn run_headless_once(
    base: &str,
    root: &Path,
    files: &[PathBuf],
    timeout: Duration,
) -> Result<Vec<FileReport>, String> {
    let mut chrome = Chrome::launch(timeout).map_err(|e| e.to_string())?;
    let mut reports = Vec::new();
    'files: for f in files {
        let rel = f
            .strip_prefix(root)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        let mut rep = FileReport {
            file: rel.clone(),
            load_error: None,
            tests: vec![],
            a11y: vec![],
            errors: vec![],
            leaks: vec![],
            engine: "headless-cdp",
            fallback: None,
            a11y_mode: "Chrome Accessibility.getFullAXTree",
            heap_mode: if HEAP.load(std::sync::atomic::Ordering::SeqCst) {
                "forced-GC inspector"
            } else {
                "not-requested"
            },
            lanes: Vec::new(),
        };
        let (mut session,mut loaded)=match open_test_page(&mut chrome,base,&rel) {
            Ok(page)=>page,
            Err(error) if test_navigation(&error)=>{rep.load_error=Some(format!("{rel}: the test module navigated while loading; fix: keep test-document navigation out of module initialization"));reports.push(rep);continue;},
            Err(error)=>return Err(error),
        };
        if let Some(e) = loaded.get("error").and_then(Json::as_str) {
            rep.load_error = Some(e.to_string());
            close_test_page(&mut chrome,&session);
            reports.push(rep);
            continue;
        }
        let heap = HEAP.load(std::sync::atomic::Ordering::SeqCst);
        instrument_test_page(&mut chrome,&session,heap);
        let names=loaded.get("names").cloned().unwrap_or_else(||Json::Arr(vec![]));
        let count=match &names {Json::Arr(names)=>names.len(),_=>0};
        plan_id(&loaded)?;
        for i in 0..count {
            let name=match &names {Json::Arr(names)=>names.get(i).and_then(Json::as_str).unwrap_or("?"),_=>"?"}.to_string();
            let began=Instant::now();let mut unstarted_recovery=false;
            loop {
                match bound_step(&mut chrome,&session,&loaded,i,&name,audit) {
                    Ok((result,issues))=>{
                        rep.tests.push(TestResult {name:name.clone(),ok:result.get("ok")==Some(&Json::Bool(true)),ms:result.get("ms").and_then(|v|if let Json::Num(n)=v{Some(*n)}else{None}).unwrap_or(0.0),error:result.get("error").and_then(Json::as_str).map(str::to_string)});
                        for issue in issues {if !rep.a11y.iter().any(|old|old.code==issue.code && old.role==issue.role && old.node==issue.node){rep.a11y.push(issue)}}
                        break;
                    }
                    Err(BoundStepError::Other(error))=>return Err(error),
                    Err(BoundStepError::Navigation{started})=>{
                        if started {rep.tests.push(TestResult {name:name.clone(),ok:false,ms:began.elapsed().as_secs_f64()*1000.0,error:Some(navigation_message(&rel,&name))});}
                        else {fail_last_navigation(&mut rep);}
                        close_test_page(&mut chrome,&session);
                        if (started && i+1==count) || (!started && (unstarted_recovery || i==0)) {
                            if !started {rep.load_error=Some(format!("{rel}: navigation prevented the next registered scenario from starting; remaining tests and feedback were not measured"));}
                            rep.heap_mode="not-measured";reports.push(rep);continue 'files;
                        }
                        match restore_test_page(&mut chrome,base,&rel,&names,heap) {
                            Ok(Some((next,plan)))=>{session=next;loaded=plan;}
                            Ok(None)=>{rep.load_error=Some(format!("{rel}: could not restore the same registered test list after navigation; remaining tests were not run"));reports.push(rep);continue 'files;}
                            Err(error) if test_navigation(&error)=>{rep.load_error=Some(format!("{rel}: the test module navigated again during recovery; remaining tests were not run; fix: remove navigation from module initialization"));reports.push(rep);continue 'files;}
                            Err(error)=>return Err(format!("{error}; while restoring {rel} after a test-document navigation")),
                        }
                        if started {break;}
                        // The current assertion never began. Restore it once; the failed previous scenario is not replayed.
                        unstarted_recovery=true;
                    }
                }
            }
        }
        if !plan_matches(&mut chrome,&session,&loaded)? {
            fail_last_navigation(&mut rep);rep.load_error=Some(format!("{rel}: post-test feedback was not measured because the registered document navigated"));rep.heap_mode="not-measured";
            close_test_page(&mut chrome,&session);reports.push(rep);continue 'files;
        }
        if heap {
            // unmount, then give requests still on the way a bounded time to settle: a pending `.then` retains its
            // component until it does, which says nothing about a leak
            let _ = chrome.eval(
                &session,
                "globalThis.__brainDispose && globalThis.__brainDispose(), true",
            );
            let settle = Instant::now() + Duration::from_millis(SETTLE_MS);
            while Instant::now() < settle {
                match chrome.eval(&session, "import('/@brain/brain-inspect.js').then(m => m.inspect().inflight().length, () => 0)") {
                    Ok(Json::Num(n)) if n > 0.0 => std::thread::sleep(Duration::from_millis(50)),
                    _ => break,
                }
            }
        }
        // before the abort below: the rejected calls it causes are the runner's own, not the app's errors
        if let Ok(Json::Arr(errs)) = chrome.eval(
            &session,
            "import('/@brain/brain-err.js').then(m => m.errors.map(r => ({ type: r.type, msg: r.msg, line: r.line, location: r.location, generatedLocation: r.generatedLocation, frames: r.frames, causes: r.causes, truncated: r.truncated, mapping: r.mapping, stack: r.stack, src: r.src, brainStack: r.brainStack, crumbs: r.crumbs, comp: r.comp ? (r.comp.name || r.comp.file || null) : null, n: r.n })), () => [])",
        ) {
            rep.errors = errs;
        }
        if heap {
            let _ = chrome.eval(&session, "import('/@brain/brain-inspect.js').then(m => (m.inspect().abortInflight(), new Promise(r => setTimeout(r, 50))).then(() => true), () => false)");
            // the accessibility audit keeps DOM nodes bound in DevTools' agents: release them, or every unmounted
            // component would look alive
            for d in ["Accessibility.disable", "DOM.disable"] {
                let _ = chrome.call(Some(&session), d, Json::Obj(vec![]));
            }
            let _ = chrome.call(Some(&session), "HeapProfiler.enable", Json::Obj(vec![]));
            let collect = |chrome: &mut Chrome| {
                let _ = chrome.call(
                    Some(&session),
                    "HeapProfiler.collectGarbage",
                    Json::Obj(vec![]),
                );
                std::thread::sleep(Duration::from_millis(60));
                chrome.eval(
                    &session,
                    "import('/@brain/brain-inspect.js').then(m => (m.tick(), m.inspect().alive()), () => 0)",
                )
            };
            // give what began after the unmount its bounded time, and stop as soon as nothing is left alive
            let grace = Instant::now() + Duration::from_millis(RELEASE_GRACE_MS);
            for round in 0.. {
                let alive = matches!(collect(&mut chrome), Ok(Json::Num(n)) if n > 0.0);
                if round >= 3 && (!alive || Instant::now() >= grace) {
                    break;
                }
                if round >= 3 {
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            let _ = chrome.eval(
                &session,
                "import('/@brain/brain-inspect.js').then(m => (m.inspect().holdVerdict(false), true), () => false)",
            );
            // a few forced GC cycles more: an instance still alive after two of them is a suspected leak
            for _ in 0..4 {
                let _ = collect(&mut chrome);
            }
            if let Ok(Json::Arr(ls)) = chrome.eval(&session, "import('/@brain/brain-inspect.js').then(m => m.inspect().leaks().map(l => ({ code: l.code, name: l.name, file: l.file, line: l.line || null, msg: l.msg, fix: l.fix })), () => [])") {
                rep.leaks = ls;
            }
        }
        if !plan_matches(&mut chrome,&session,&loaded)? {
            fail_last_navigation(&mut rep);rep.load_error=Some(format!("{rel}: runtime/heap feedback was interrupted by navigation and was not measured"));rep.errors.clear();rep.leaks.clear();rep.heap_mode="not-measured";
        }
        close_test_page(&mut chrome,&session);
        reports.push(rep);
    }
    Ok(reports)
}

#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn closing_a_test_session_preserves_other_owned_browser_tabs() {
        fn targets(chrome: &mut Chrome) -> std::collections::BTreeSet<String> {
            let result=chrome.call(None,"Target.getTargets",Json::Obj(vec![])).unwrap();
            let Some(Json::Arr(rows))=result.get("targetInfos") else { panic!("Target.getTargets returned no inventory") };
            rows.iter().filter(|row|row.get("type").and_then(Json::as_str)==Some("page"))
                .filter_map(|row|row.get("targetId").and_then(Json::as_str).map(str::to_string)).collect()
        }
        let mut chrome=Chrome::launch(Duration::from_secs(20)).unwrap();
        let other=chrome.new_page().unwrap();
        let before=targets(&mut chrome);
        chrome.deadline=Some(Duration::from_secs(5));
        for _ in 0..8 {
            let test=chrome.new_page().unwrap();
            assert_eq!(targets(&mut chrome).len(),before.len()+1);
            close_test_page(&mut chrome,&test);
            assert_eq!(targets(&mut chrome),before,"closing a finished test cannot close another tab");
            assert_eq!(chrome.deadline,Some(Duration::from_secs(5)),"cleanup must restore the caller's deadline");
            assert_eq!(chrome.eval(&other,"1 + 1").unwrap(),Json::Num(2.0));
        }
        assert!(chrome.close_target("missing-owned-target",Duration::from_secs(2)).is_err());
        assert_eq!(chrome.deadline,Some(Duration::from_secs(5)));
        assert_eq!(targets(&mut chrome),before);
        assert_eq!(chrome.eval(&other,"1 + 1").unwrap(),Json::Num(2.0));
    }

    // Exercise real context replacement at deterministic protocol boundaries, without an HTTP race.
    #[test]
    fn registered_document_guards_steps_and_discards_interrupted_audits() {
        fn open(chrome:&mut Chrome)->(String,Json) {
            let api=include_str!("../../brain-serve/src/test-api.js")
                .replace("import { mount as rtMount } from '/@brain/brain.js';","const rtMount=()=>()=>{};");
            let html=format!("<div id='root'></div><script type='module'>{api}</script>");
            let session=chrome.new_page().unwrap();
            chrome.call(Some(&session),"Page.navigate",Json::obj([("url",Json::str(format!("data:text/html;base64,{}",crate::shot::base64(html.as_bytes()))))])).unwrap();
            let end=std::time::Instant::now()+Duration::from_secs(3);
            loop {
                if chrome.eval(&session,"typeof window.__brainLoad==='function'")==Ok(Json::Bool(true)){break}
                assert!(std::time::Instant::now()<end,"test API did not become ready");
                std::thread::sleep(Duration::from_millis(10));
            }
            let module="globalThis.__called=[];test('first',()=>__called.push('first'));test('second',()=>__called.push('second'));test('during',async()=>{__called.push('during');location.replace('about:blank');await new Promise(()=>{})});";
            let url=format!("data:text/javascript;base64,{}",crate::shot::base64(module.as_bytes()));
            let plan=chrome.eval(&session,&format!("window.__brainLoad({})",Json::str(url).stringify())).unwrap();
            assert!(plan_id(&plan).is_ok(),"{}",plan.stringify());(session,plan)
        }
        fn reload(chrome:&mut Chrome,session:&str,plan:&Json) {
            chrome.call(Some(session),"Page.reload",Json::Obj(vec![])).unwrap();
            let end=std::time::Instant::now()+Duration::from_secs(3);
            while plan_matches(chrome,session,plan).unwrap() {
                assert!(std::time::Instant::now()<end,"document identity survived reload");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        fn no_audit(_: &mut Chrome,_:&str,_:&str,_:&mut Vec<AxIssue>)->Result<(),String>{Ok(())}
        let mut chrome=Chrome::launch(Duration::from_secs(25)).unwrap();
        let (session,plan)=open(&mut chrome);
        assert_eq!(bound_step(&mut chrome,&session,&plan,0,"first",no_audit).unwrap().0.get("ok"),Some(&Json::Bool(true)));
        reload(&mut chrome,&session,&plan);
        assert!(matches!(bound_step(&mut chrome,&session,&plan,1,"second",no_audit),Err(BoundStepError::Navigation{started:false})));
        close_test_page(&mut chrome,&session);
        let (session,plan)=open(&mut chrome);
        assert_eq!(bound_step(&mut chrome,&session,&plan,1,"second",no_audit).unwrap().0.get("ok"),Some(&Json::Bool(true)));
        assert_eq!(chrome.eval(&session,"__called").unwrap(),Json::Arr(vec![Json::str("second")]));
        let result=bound_step(&mut chrome,&session,&plan,0,"first",|chrome,session,_,issues|{
            issues.push(AxIssue{code:"B0406",role:"button".into(),node:"button".into(),test:"first".into(),msg:"partial observation".into(),fix:"add text",engine:"test"});
            reload(chrome,session,&plan);Ok(())
        });
        assert!(matches!(result,Err(BoundStepError::Navigation{started:true})),"partial audit cannot become a measured result");
        close_test_page(&mut chrome,&session);
        let (session,plan)=open(&mut chrome);
        let result=bound_step(&mut chrome,&session,&plan,2,"during",no_audit);
        if let Err(BoundStepError::Other(ref error))=result {panic!("unexpected navigation error: {error}")}
        assert!(matches!(result,Err(BoundStepError::Navigation{started:true})),"pending assertion must retain its navigation failure");
        close_test_page(&mut chrome,&session);
    }

    #[test]
    fn a_broken_browser_pipe_is_retried_once_then_blamed_on_load() {
        let broken = || Err::<(), _>("CDP transport write failed: Broken pipe (os error 32); the owned browser session was stopped".to_string());
        let mut calls = 0;
        let again = transport_retry(|| { calls += 1; if calls == 1 { broken() } else { Ok(()) } }, Duration::ZERO, || Some(75.0));
        assert_eq!((again, calls), (Ok(()), 2), "one retry recovers");
        let mut calls = 0;
        let failed = transport_retry(|| { calls += 1; broken() }, Duration::ZERO, || Some(75.4)).unwrap_err();
        assert_eq!(calls, 2, "never more than one retry");
        assert!(failed.contains("Broken pipe") && failed.contains("machine load") && failed.contains("75.4"), "{failed}");
        let silent = transport_retry(|| Err::<(), _>("timed out: Chrome did not respond; fix: raise --timeout".to_string()), Duration::ZERO, || None).unwrap_err();
        assert!(silent.contains("could not be read"), "{silent}");
        let mut calls = 0;
        let other = transport_retry(|| { calls += 1; Err::<(), _>("the test page did not load".to_string()) }, Duration::ZERO, || Some(1.0));
        assert_eq!((other, calls), (Err("the test page did not load".to_string()), 1), "a test failure is not retried");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn preflight_rechecks_current_transitive_sources_without_unrelated_components() {
        let root = std::env::temp_dir().join(format!("brain-test-preflight-{}", std::process::id()));
        std::fs::create_dir_all(root.join("comp")).unwrap();
        std::fs::write(root.join("tokens.cssbrain"), ":root{--space-2:8px}").unwrap();
        std::fs::write(root.join("brain.json"), "{}").unwrap();
        let test = root.join("comp/parent.test.brain");
        std::fs::write(&test, "<script>import Parent from './parent.brain'\ntest('renders', async () => { await mount(Parent) })</script>").unwrap();
        std::fs::write(root.join("comp/parent.brain"), "<script>import Child from './child.brain'</script><Child />").unwrap();
        let child = root.join("comp/child.brain");
        std::fs::write(&child, "<button>Continue</button>").unwrap();
        std::fs::write(root.join("comp/unrelated.brain"), "<button></button>").unwrap();
        let selected = [test];
        super::preflight(&root, &selected).unwrap();
        // A successful previous compile must not make the now-invalid child look testable.
        std::fs::write(&child, "<button></button>").unwrap();
        let error = super::preflight(&root, &selected).unwrap_err();
        assert!(error.contains("B0406") && error.contains("comp/child.brain"), "{error}");
        std::fs::write(&child, "<button>Continue</button>").unwrap();
        super::preflight(&root, &selected).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preflight_checks_automatically_discovered_components() {
        let root = std::env::temp_dir().join(format!("brain-test-preflight-auto-{}", std::process::id()));
        std::fs::create_dir_all(root.join("comp")).unwrap();
        std::fs::write(root.join("tokens.cssbrain"), ":root{--space-2:8px}").unwrap();
        std::fs::write(root.join("brain.json"), "{}").unwrap();
        let test = root.join("comp/parent.test.brain");
        std::fs::write(&test, "<script>import Parent from './parent.brain'\ntest('renders', async () => { await mount(Parent) })</script>").unwrap();
        std::fs::write(root.join("comp/parent.brain"), "<Child />").unwrap();
        std::fs::write(root.join("comp/child.brain"), "<button></button>").unwrap();
        let error = super::preflight(&root, &[test]).unwrap_err();
        assert!(error.contains("B0406") && error.contains("comp/child.brain"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn viewer_lane_is_opt_in_only() {
        let none: Vec<String> = vec![];
        assert!(!super::viewer_requested(&none, None));
        assert!(!super::viewer_requested(&["--heap".to_string()], Some("0")));
        assert!(!super::viewer_requested(&none, Some("")));
        assert!(super::viewer_requested(&["--viewer".to_string()], None));
        assert!(super::viewer_requested(&none, Some("1")));
        assert!(super::viewer_requested(&none, Some(" TRUE ")));
        // The scaffold's agent guide must not make an extension mandatory.
        let guide = crate::scaffold::AGENTS_MD;
        assert!(guide.contains("no browser extension is required"));
        assert!(!guide.contains("MUST probe"));
    }

    #[test]
    fn viewer_failures_carry_the_adapter_code_and_fix() {
        let refused = serde_json::json!({"ok":false,"code":"VIEWER_OPEN_REFUSED","fix":"Set BRAIN_VIEWER_PROFILE"});
        assert_eq!(
            super::viewer_hint(&refused),
            " (VIEWER_OPEN_REFUSED: Set BRAIN_VIEWER_PROFILE)"
        );
        assert_eq!(
            super::viewer_hint(&serde_json::json!({"code":"VIEWER_EXECUTION_FAILED"})),
            " (VIEWER_EXECUTION_FAILED)"
        );
        assert_eq!(super::viewer_hint(&serde_json::json!({"ok":false})), "");
    }

    use super::*;

    #[test]
    fn ax_rules_map_to_spec_codes() {
        assert_eq!(ax_rule("button").unwrap().0, "B0406");
        assert_eq!(ax_rule("textbox").unwrap().0, "B0403");
        assert_eq!(ax_rule("image").unwrap().0, "B0401");
        assert!(ax_rule("generic").is_none());
    }
}
