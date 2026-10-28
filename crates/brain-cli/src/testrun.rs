//! `brain test`: runs every `*.test.brain` in real headless Chrome (our own CDP over a pipe) and audits the
//! accessibility tree after each test with `Accessibility.getFullAXTree` (SPEC §8).

use crate::cdp::Chrome;
use brain_mcp::Json;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
}

impl FileReport {
    pub fn failed(&self) -> bool {
        self.load_error.is_some() || self.tests.iter().any(|t| !t.ok) || !self.a11y.is_empty() || !self.leaks.is_empty()
    }
    pub fn to_json(&self) -> Json {
        let tests = self
            .tests
            .iter()
            .map(|t| {
                let mut kv = vec![("name".into(), Json::str(&t.name)), ("ok".into(), Json::Bool(t.ok)), ("ms".into(), Json::Num((t.ms * 10.0).round() / 10.0))];
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
                ])
            })
            .collect();
        let mut kv = vec![("file".into(), Json::str(&self.file))];
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
    let Ok(rd) = std::fs::read_dir(dir) else { return };
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
        "button" | "link" | "menuitem" | "tab" => Some(("B0406", "control without accessible text", "add visible text or `aria-label` to the element")),
        "textbox" | "searchbox" | "checkbox" | "radio" | "combobox" | "listbox" | "slider" | "spinbutton" | "switch" => {
            Some(("B0403", "form control without an accessible name", "wrap it in `<label>` or add `aria-label`"))
        }
        "image" | "img" => Some(("B0401", "image without alternative text", "add `alt` (or `alt=\"\"` if it is decorative)")),
        _ => None,
    }
}

fn describe(chrome: &mut Chrome, session: &str, backend: &Json) -> String {
    let Ok(r) = chrome.call(Some(session), "DOM.describeNode", Json::obj([("backendNodeId", backend.clone())])) else {
        return "?".into();
    };
    let Some(node) = r.get("node") else { return "?".into() };
    let mut s = node.get("localName").and_then(Json::as_str).unwrap_or("?").to_string();
    if let Some(Json::Arr(attrs)) = node.get("attributes") {
        for pair in attrs.chunks(2) {
            if let [Json::Str(k), Json::Str(v)] = pair {
                match k.as_str() {
                    "id" => s.push_str(&format!("#{v}")),
                    "class" => v.split_whitespace().for_each(|c| s.push_str(&format!(".{c}"))),
                    "type" | "name" | "role" => s.push_str(&format!("[{k}=\"{v}\"]")),
                    _ => {}
                }
            }
        }
    }
    s
}

pub(crate) fn audit(chrome: &mut Chrome, session: &str, test: &str, out: &mut Vec<AxIssue>) -> Result<(), String> {
    let tree = chrome.call(Some(session), "Accessibility.getFullAXTree", Json::Obj(vec![]))?;
    let Some(Json::Arr(nodes)) = tree.get("nodes") else { return Ok(()) };
    for n in nodes {
        if n.get("ignored") == Some(&Json::Bool(true)) {
            continue;
        }
        let role = n.get("role").and_then(|r| r.get("value")).and_then(Json::as_str).unwrap_or("");
        let name = n.get("name").and_then(|r| r.get("value")).and_then(Json::as_str).unwrap_or("");
        let Some((code, msg, fix)) = ax_rule(role) else { continue };
        if !name.trim().is_empty() {
            continue;
        }
        let node = n.get("backendDOMNodeId").map_or_else(|| "?".into(), |b| describe(chrome, session, b));
        // The same failure on the same node is reported once per file.
        if out.iter().any(|a| a.code == code && a.node == node) {
            continue;
        }
        out.push(AxIssue { code, role: role.into(), node, test: test.into(), msg: msg.into(), fix });
    }
    Ok(())
}

/// Runs the files against the server at `base` (e.g. `http://127.0.0.1:53211`).
/// `brain test --heap` / `brain doctor`: watch for leaks while the tests run (§9, §18).
static HEAP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_heap(on: bool) {
    HEAP.store(on, std::sync::atomic::Ordering::SeqCst);
}

pub fn run(base: &str, root: &Path, files: &[PathBuf], timeout: Duration) -> Result<Vec<FileReport>, String> {
    let mut chrome = Chrome::launch(timeout).map_err(|e| e.to_string())?;
    let mut reports = Vec::new();
    for f in files {
        let rel = f.strip_prefix(root).unwrap_or(f).to_string_lossy().replace('\\', "/");
        let session = chrome.new_page()?;
        for d in ["Runtime.enable", "Page.enable", "Accessibility.enable"] {
            chrome.call(Some(&session), d, Json::Obj(vec![]))?;
        }
        chrome.call(Some(&session), "Page.navigate", Json::obj([("url", Json::str(format!("{base}/@brain/test.html")))]))?;
        // Waits until the API module has defined __brainLoad.
        let mut ready = false;
        for _ in 0..200 {
            if chrome.eval(&session, "typeof window.__brainLoad === 'function'")? == Json::Bool(true) {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut rep = FileReport { file: rel.clone(), load_error: None, tests: vec![], a11y: vec![], errors: vec![], leaks: vec![] };
        if !ready {
            rep.load_error = Some("the test page did not load /@brain/test-api.js".into());
            reports.push(rep);
            continue;
        }
        let loaded = chrome.eval(&session, &format!("window.__brainLoad({})", Json::str(format!("/{rel}")).stringify()))?;
        if let Some(e) = loaded.get("error").and_then(Json::as_str) {
            rep.load_error = Some(e.to_string());
            reports.push(rep);
            continue;
        }
        let heap = HEAP.load(std::sync::atomic::Ordering::SeqCst);
        if heap {
            // the inspector watches every unmounted instance (FinalizationRegistry) and fx timers/listeners
            let _ = chrome.eval(&session, "import('/@brain/brain-inspect.js').then(m => (m.inspect({ noPanel: true }), true), () => false)");
        }
        // capture runtime errors (without console noise) while this file's tests run
        let _ = chrome.eval(&session, "import('/@brain/brain-err.js').then(m => (m.capture({ log: false }), true), () => false)");
        let count = match loaded.get("names") {
            Some(Json::Arr(a)) => a.len(),
            _ => 0,
        };
        for i in 0..count {
            let r = chrome.eval(&session, &format!("window.__brainStep({i})"))?;
            let name = r.get("name").and_then(Json::as_str).unwrap_or("?").to_string();
            rep.tests.push(TestResult {
                ok: r.get("ok") == Some(&Json::Bool(true)),
                ms: match r.get("ms") {
                    Some(Json::Num(n)) => *n,
                    _ => 0.0,
                },
                error: r.get("error").and_then(Json::as_str).map(str::to_string),
                name: name.clone(),
            });
            audit(&mut chrome, &session, &name, &mut rep.a11y)?;
        }
        if heap {
            // unmount, then a few forced GC cycles: an instance still alive after two of them is a leak
            let _ = chrome.eval(&session, "globalThis.__brainDispose && globalThis.__brainDispose(), true");
            // the accessibility audit keeps DOM nodes bound in DevTools' agents: release them, or every unmounted
            // component would look alive
            for d in ["Accessibility.disable", "DOM.disable"] {
                let _ = chrome.call(Some(&session), d, Json::Obj(vec![]));
            }
            let _ = chrome.call(Some(&session), "HeapProfiler.enable", Json::Obj(vec![]));
            for _ in 0..4 {
                let _ = chrome.call(Some(&session), "HeapProfiler.collectGarbage", Json::Obj(vec![]));
                std::thread::sleep(Duration::from_millis(60));
                let _ = chrome.eval(&session, "import('/@brain/brain-inspect.js').then(m => (m.tick(), true), () => false)");
            }
            if let Ok(Json::Arr(ls)) = chrome.eval(&session, "import('/@brain/brain-inspect.js').then(m => m.inspect().leaks().map(l => ({ code: l.code, name: l.name, file: l.file, line: l.line || null, msg: l.msg, fix: l.fix })), () => [])") {
                rep.leaks = ls;
            }
        }
        if let Ok(Json::Arr(errs)) = chrome.eval(
            &session,
            "import('/@brain/brain-err.js').then(m => m.errors.map(r => ({ type: r.type, msg: r.msg, line: r.line, comp: r.comp ? (r.comp.name || r.comp.file || null) : null, n: r.n })), () => [])",
        ) {
            rep.errors = errs;
        }
        reports.push(rep);
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ax_rules_map_to_spec_codes() {
        assert_eq!(ax_rule("button").unwrap().0, "B0406");
        assert_eq!(ax_rule("textbox").unwrap().0, "B0403");
        assert_eq!(ax_rule("image").unwrap().0, "B0401");
        assert!(ax_rule("generic").is_none());
    }
}
