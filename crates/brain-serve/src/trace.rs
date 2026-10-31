//! `brain trace` (INTERNALS §49): the development capture. `brain run` calls [`enable`]; the dev server, the `load`
//! sidecar and the browser client then hand their events to a Node writer that redacts them and stores them in
//! `.brain/trace.db`. Nothing here is reachable from a production build (test: `tests/trace_production.rs` in brain-cli).

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock, Weak};

const STORE: &str = include_str!("trace-store.mjs");
/// Served at `/@brain/brain-trace.js` when capture is off, so no page wraps `fetch` for nothing.
pub(crate) const CLIENT_OFF: &str = "export const attach=()=>{};export const ids={};export const inspected=()=>{};export const lastFailed=()=>null;\n";
/// Events waiting for the writer. A full queue drops: capture never slows the app down.
const QUEUE: usize = 4096;

fn registry() -> &'static Mutex<HashMap<PathBuf, Option<Weak<Trace>>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Option<Weak<Trace>>>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// Turns capture on for the dev server of this app. Only `brain run` and MCP `brain_run_start` call it, so `brain test`,
/// `brain doctor`, the debugger's own server and every production path never record.
pub fn enable(root: &Path) {
    if settings(root).enabled {
        registry().lock().unwrap().entry(root.to_path_buf()).or_insert(None);
    }
}

pub(crate) fn attach(root: &Path) -> Option<Arc<Trace>> {
    let mut map = registry().lock().unwrap();
    let slot = map.get_mut(root)?;
    if let Some(live) = slot.as_ref().and_then(Weak::upgrade) {
        return Some(live);
    }
    let trace = Trace::new(root);
    *slot = Some(Arc::downgrade(&trace));
    Some(trace)
}

pub(crate) fn for_root(root: &Path) -> Option<Arc<Trace>> {
    registry().lock().unwrap().get(root)?.as_ref()?.upgrade()
}

/// `brain.json` `trace`, plus `BRAIN_TRACE=0`.
pub struct Settings {
    pub enabled: bool,
    pub config: String,
}

pub fn settings(root: &Path) -> Settings {
    let value = std::fs::read_to_string(root.join("brain.json")).ok().and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let trace = value.as_ref().and_then(|json| json.get("trace"));
    let off = std::env::var("BRAIN_TRACE").is_ok_and(|v| matches!(v.as_str(), "0" | "off" | "false"));
    Settings {
        enabled: !off && trace.and_then(|t| t.get("enabled")).and_then(serde_json::Value::as_bool).unwrap_or(true),
        config: trace.map(ToString::to_string).unwrap_or_else(|| "{}".into()),
    }
}

fn node(root: &Path) -> Command {
    let mut command = Command::new(std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into()));
    command.args(["--disable-warning=ExperimentalWarning", "--input-type=module", "-e", STORE]).current_dir(root);
    command
}
const NEEDS_NODE: &str = "`brain trace` stores events with Node's built-in SQLite; fix: install Node 22.13 or newer, or set BRAIN_NODE";

/// One answer from the store (`mode`: list, issues), as the store prints it.
pub fn query(root: &Path, args: &str) -> Result<String, String> {
    let output = node(root).args(["query", &settings(root).config, args]).stdin(Stdio::null()).output().map_err(|e| format!("{NEEDS_NODE} ({e})"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let error = String::from_utf8_lossy(&output.stderr);
    if error.contains("node:sqlite") {
        return Err(NEEDS_NODE.into());
    }
    Err(error.trim().trim_start_matches("error: ").to_string())
}

/// The handle every source uses. Lines go through a bounded queue to the thread that owns the writer process.
pub struct Trace {
    queue: Mutex<Option<SyncSender<String>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    warned: Arc<AtomicBool>,
}

impl Trace {
    fn new(root: &Path) -> Arc<Trace> {
        let (queue, lines) = sync_channel(QUEUE);
        let (root, warned) = (root.to_path_buf(), Arc::new(AtomicBool::new(false)));
        let flag = warned.clone();
        let worker = std::thread::spawn(move || pump(&root, lines, &flag));
        Arc::new(Trace { queue: Mutex::new(Some(queue)), worker: Mutex::new(Some(worker)), warned })
    }
    /// One JSON object per line. The line may hold raw values: the writer redacts before anything reaches disk.
    pub(crate) fn record(&self, line: String) {
        if line.contains('\n') {
            return;
        }
        if let Some(queue) = self.queue.lock().unwrap().as_ref()
            && let Err(TrySendError::Disconnected(_)) = queue.try_send(line)
            && !self.warned.swap(true, Ordering::Relaxed)
        {
            eprintln!("brain trace: the store stopped; capture is off until the next `brain run`");
        }
    }
    /// A finished HTTP request as the dev server saw it.
    fn request(&self, kind: &str, method: &str, path: &str, query: &str, id: &str, status: Option<u16>, ms: f64, payload: &[u8]) {
        let route = if query.is_empty() { path.to_string() } else { format!("{path}?{query}") };
        let mut data = serde_json::json!({ "method": method });
        if !payload.is_empty() {
            data["payload"] = String::from_utf8_lossy(&payload[..payload.len().min(262_144)]).into_owned().into();
        }
        self.record(serde_json::json!({ "ts": epoch_ms(), "module": "server", "kind": kind, "request_id": id, "route": route, "status": status, "duration_ms": ms, "data": data }).to_string());
    }
}

/// One request through the dev server, from the moment it is routed until its handler returns. The status comes from the
/// response writer, whichever route answered.
pub(crate) struct RequestScope {
    trace: Arc<Trace>,
    method: String,
    path: String,
    query: String,
    id: String,
    native: bool,
    started: std::time::Instant,
    payload: Vec<u8>,
}

/// A same-origin page may name its own request, so the id it recorded is the one server and backend use.
pub(crate) fn begin(trace: &Arc<Trace>, req: &mut crate::http::Request, native: bool) -> RequestScope {
    if let Some((_, id)) = req.headers.iter().find(|(name, value)| name.eq_ignore_ascii_case("x-brain-trace") && valid_id(value)) {
        req.id = id.clone();
    }
    crate::http::trace_begin(&req.id, report_only_policy());
    RequestScope {
        trace: trace.clone(),
        method: req.method.clone(),
        path: req.path.clone(),
        query: req.query.clone(),
        id: req.id.clone(),
        native,
        started: std::time::Instant::now(),
        payload: if matches!(req.method.as_str(), "GET" | "HEAD") { Vec::new() } else { req.body.clone() },
    }
}

impl Drop for RequestScope {
    fn drop(&mut self) {
        let status = crate::http::trace_end();
        if let Some(kind) = classify(&self.path, status.unwrap_or(200), self.native) {
            self.trace.request(kind, &self.method, &self.path, &self.query, &self.id, status, self.started.elapsed().as_secs_f64() * 1000.0, &self.payload);
        }
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        self.queue.lock().unwrap().take();
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// The writer starts with the first event: a server nobody opens spawns nothing.
fn pump(root: &Path, lines: Receiver<String>, warned: &AtomicBool) {
    let mut child: Option<Child> = None;
    while let Ok(line) = lines.recv() {
        if child.is_none() {
            let _ = std::fs::create_dir_all(root.join(".brain"));
            child = node(root).args(["writer", &settings(root).config]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::inherit()).spawn().ok();
            if child.is_none() {
                if !warned.swap(true, Ordering::Relaxed) {
                    eprintln!("brain trace: capture is off; {NEEDS_NODE}");
                }
                continue;
            }
        }
        let wrote = child.as_mut().and_then(|c| c.stdin.as_mut()).is_some_and(|stdin| write_line(stdin, &line).is_ok());
        if !wrote {
            if let Some(mut dead) = child.take() {
                let _ = dead.kill();
                let _ = dead.wait();
            }
            if !warned.swap(true, Ordering::Relaxed) {
                eprintln!("brain trace: the store stopped; fix: {NEEDS_NODE}");
            }
        }
    }
    if let Some(mut done) = child {
        drop(done.stdin.take());
        let _ = done.wait();
    }
}

fn write_line(stdin: &mut impl Write, line: &str) -> io::Result<()> {
    stdin.write_all(line.as_bytes())?;
    stdin.write_all(b"\n")?;
    stdin.flush()
}

/// The ids a same-origin page may set on its own request, so one id follows it through server and backend.
pub(crate) fn valid_id(value: &str) -> bool {
    value.len() == 36 && value.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// Which kind of request a path is, or None for what nobody debugs: dev assets that answered fine.
pub(crate) fn classify(path: &str, status: u16, native: bool) -> Option<&'static str> {
    if path.starts_with("/@brain/") {
        return None;
    }
    if path.starts_with("/_r/") {
        return Some("rpc");
    }
    if native {
        return Some("native");
    }
    if std::path::Path::new(path).extension().is_none() {
        return Some("request");
    }
    (status >= 400).then_some("asset")
}

/// The report-only policy of a dev page: the production directives (`brain_pack::cache::csp`), with what only the dev
/// server needs (inline bootstrap scripts, local APIs). It never blocks; it shows what production would.
pub(crate) fn report_only_policy() -> String {
    // what a build adds when a page needs it (wasm, Google sign-in) is allowed here: a dev server cannot know it per page
    let mut dev: Vec<(String, String)> = vec![
        ("script-src".into(), "'unsafe-inline'".into()),
        ("script-src".into(), "'wasm-unsafe-eval'".into()),
        ("connect-src".into(), "http://localhost:*".into()),
        ("connect-src".into(), "http://127.0.0.1:*".into()),
    ];
    dev.extend(brain_pack::cache::google_csp());
    brain_pack::cache::csp(&[], &dev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_well_formed_id_is_adopted() {
        assert!(valid_id("0123abcd-0123-4abc-8def-0123456789ab"));
        for bad in ["", "client-supplied", "0123abcd_0123_4abc_8def_0123456789ab", "0123abcd-0123-4abc-8def-0123456789ax", "0123abcd-0123-4abc-8def-0123456789ab0"] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn dev_assets_that_work_are_not_recorded() {
        assert_eq!(classify("/@brain/brain.js", 500, false), None);
        assert_eq!(classify("/views/home.brain", 200, false), None);
        assert_eq!(classify("/views/gone.brain", 404, false), Some("asset"));
        assert_eq!(classify("/_r/abc/save", 200, false), Some("rpc"));
        assert_eq!(classify("/about", 200, false), Some("request"));
        assert_eq!(classify("/_brain/auth/me", 200, true), Some("native"));
    }

    /// The store through its real entry points: lines in the spool, answers from `query`.
    fn store(name: &str, brain_json: &str, lines: &[String]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("brain-trace-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".brain/trace-spool")).unwrap();
        std::fs::write(root.join("brain.json"), brain_json).unwrap();
        std::fs::write(root.join(".brain/trace-spool/test.ndjson"), lines.join("\n") + "\n").unwrap();
        root
    }
    fn answer(root: &Path, args: serde_json::Value) -> serde_json::Value {
        serde_json::from_str(&query(root, &args.to_string()).unwrap()).unwrap()
    }
    fn on_disk(root: &Path) -> String {
        let mut text = String::new();
        for name in ["trace.db", "trace.db-wal"] {
            if let Ok(bytes) = std::fs::read(root.join(".brain").join(name)) {
                text.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        text
    }

    #[test]
    fn secrets_never_reach_the_disk() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.c2lnbmF0dXJlLXZhbHVl";
        let event = serde_json::json!({
            "ts": epoch_ms(), "module": "browser", "kind": "fetch", "request_id": "r1", "route": "/login?token=LEAK-QUERY&page=2", "status": 200,
            "title": format!("sent Authorization: Bearer LEAK-BEARER and {jwt}"),
            "data": {
                "payload": "{\"password\":\"LEAK-JSON\",\"nested\":{\"apiKey\":\"LEAK-KEY\",\"cardCode\":\"LEAK-CONFIGURED\",\"city\":\"Puebla\"}}",
                "response": "session_id=LEAK-FORM&name=visible",
                "headers": { "cookie": "LEAK-COOKIE", "authorization": "LEAK-HEADER", "accept": "text/html" },
                "note": "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123 xoxb-1234567890-LEAK AKIAABCDEFGHIJKLMNOP",
            },
        });
        let rpc = serde_json::json!({ "ts": epoch_ms(), "module": "server", "kind": "rpc", "route": "/_r/a/login", "status": 200, "data": { "payload": "[\"LEAK-POSITIONAL\",42]" } });
        let root = store("redact", r#"{"trace":{"redact":["cardCode"]}}"#, &[event.to_string(), rpc.to_string()]);
        let events = answer(&root, serde_json::json!({ "json": true }));
        let printed = events.to_string();
        assert!(printed.contains("Puebla") && printed.contains("visible") && printed.contains("page=2"), "ordinary values stay: {printed}");
        assert!(printed.contains("[string, 15 chars]") && printed.contains("42"), "{printed}");
        let disk = on_disk(&root);
        assert!(disk.contains("Puebla"), "control: the store file is readable as text");
        for secret in ["LEAK", "ghp_ABC", "AKIAABC", jwt] {
            assert!(!printed.contains(secret), "{secret} in the answer: {printed}");
            assert!(!disk.contains(secret), "{secret} on disk");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(root.join(".brain/trace.db")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bodies_are_truncated_and_age_and_size_are_capped() {
        let now = epoch_ms() as i64;
        let big = "x".repeat(300_000);
        let mut lines = vec![
            serde_json::json!({ "ts": now - 3 * 3_600_000, "module": "server", "kind": "request", "route": "/old" }).to_string(),
            serde_json::json!({ "ts": now - 3 * 3_600_000, "module": "server", "kind": "exception", "level": "error", "title": "old failure" }).to_string(),
            serde_json::json!({ "ts": now, "module": "server", "kind": "request", "route": "/big", "data": { "payload": big } }).to_string(),
        ];
        let filler = "y".repeat(3000);
        for n in 0..1500 {
            lines.push(serde_json::json!({ "ts": now, "module": "server", "kind": "request", "route": format!("/fill/{n}"), "data": { "payload": filler } }).to_string());
        }
        let root = store("caps", r#"{"trace":{"maxBodyKB":1,"maxMB":1,"maxAgeHours":1}}"#, &lines);
        let page = answer(&root, serde_json::json!({ "json": true, "limit": 1000 }));
        let events = page["events"].as_array().unwrap();
        let routes: Vec<&str> = events.iter().filter_map(|e| e["route"].as_str()).collect();
        assert!(!routes.contains(&"/old"), "older than maxAgeHours");
        assert!(routes.contains(&"/fill/1499"), "the newest stay");
        assert!(!routes.contains(&"/fill/0"), "the oldest went to fit maxMB");
        assert!(events.iter().all(|e| e["data"]["payload"].as_str().is_none_or(|p| p.len() < 1200)), "bodies cut at maxBodyKB");
        assert_eq!(answer(&root, serde_json::json!({ "json": true, "mode": "issues" }))["issues"], serde_json::json!([]), "aged issues go too");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failure_groups_once_and_a_new_module_needs_no_schema() {
        let failed = |module: &str, id: &str| serde_json::json!({ "module": module, "kind": "rpc", "request_id": id, "route": "/_r/a/save", "status": 500, "data": { "method": "POST" } }).to_string();
        let custom = serde_json::json!({ "module": "payments", "kind": "declined", "level": "error", "request_id": "c1", "title": "Card declined", "code": "B0999", "fix": "Ask for another card", "file": "server/pay.jsbrain", "line": 7 });
        let root = store("issues", "{}", &[failed("browser", "r1"), failed("server", "r1"), failed("browser", "r2"), custom.to_string()]);
        let issues = answer(&root, serde_json::json!({ "json": true, "mode": "issues" }));
        let list = issues["issues"].as_array().unwrap();
        assert_eq!(list.len(), 2, "{issues}");
        let request = list.iter().find(|i| i["b_code"] == "B1840").unwrap();
        assert_eq!(request["count"], 2, "two requests, however many layers saw them: {request}");
        let module = list.iter().find(|i| i["module"] == "payments").unwrap();
        assert_eq!((&module["b_code"], &module["fix"], &module["line"]), (&serde_json::json!("B0999"), &serde_json::json!("Ask for another card"), &serde_json::json!(7)));
        let one = answer(&root, serde_json::json!({ "json": true, "requestId": "r1" }));
        assert_eq!(one["events"].as_array().unwrap().len(), 2);
        assert_eq!(answer(&root, serde_json::json!({ "json": true, "module": "payments" }))["events"][0]["kind"], "declined");
        assert_eq!(answer(&root, serde_json::json!({ "json": true, "failed": true }))["events"].as_array().unwrap().len(), 4);
        assert!(query(&root, r#"{"since":"soon"}"#).unwrap_err().contains("fix:"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_report_only_policy_keeps_production_directives() {
        let policy = report_only_policy();
        assert!(policy.contains("object-src 'none'"), "{policy}");
        assert!(policy.contains("'unsafe-inline'"), "{policy}");
        assert!(!policy.contains("'unsafe-eval'"), "{policy}");
    }
}
