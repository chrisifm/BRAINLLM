use crate::{http, json_str, ws};
#[path = "dev_threads.rs"]
mod dev_threads;
use dev_threads::{Threads, Role, PendingRequest, FailureLog};
#[path="native_proxy.rs"]
pub(crate) mod native_proxy;
use notify::{RecursiveMode, Watcher};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

const CLIENT_JS: &str = concat!(include_str!("client.js"), include_str!("origin_help.js"));
const TEST_HTML: &str = include_str!("test.html");
const TEST_API_JS: &str = include_str!("test-api.js");
const CLIENT_TAG: &str = "<script type=\"module\" src=\"/@brain/client.js\"></script><script>navigator.serviceWorker&&navigator.serviceWorker.getRegistrations().then(r=>r.forEach(x=>x.unregister()))</script>";

#[derive(Default)]
pub struct Compiled {
    pub js: String,
    pub css: String,
    pub deps: Vec<PathBuf>,
    /// Server code (`load` + `server:` functions), empty if there is none.
    pub server_js: String,
    pub server: brain_lex::ServerMetadata,
}

/// `(src, path) -> Ok(Compiled) | Err(diags as SPEC §5 JSON objects)`.
pub type CompileFn = Arc<dyn Fn(&str, &Path) -> Result<Compiled, Vec<String>> + Send + Sync>;

pub struct Options {
    pub root: PathBuf,
    pub host: String,
    pub port: u16,
    /// HTML inserted right after `<head>` on every page (e.g. `<style>@layer …;</style>`, which must come
    /// before any stylesheet to fix the SPEC §7 layer order).
    pub head_prepend: String,
    /// Runtime files served at `/@brain/<name>` (SPEC §6): brain.js, brain-dev.js, brain-wc.js.
    pub runtime: Vec<(String, String)>,
    /// Resolves the environment for `load` and `server:` functions (`.brain_env`, §36); called again when the file
    /// changes. None = the process environment only.
    pub env: Option<EnvLoader>,
    /// brain.json `base` (#148): `/` or `/app/`. Every URL is served under it, as in production; outside it → 404.
    pub base: String,
    /// Every diagnostic of one file (errors, warnings, info; `brain check --json` objects), for the dev overlay (#267).
    pub diagnose: Option<DiagnoseFn>,
    /// `brain check --json` of the whole app, for `GET /@brain/problems` (the debugger's Problems view, #269).
    pub problems: Option<ProblemsFn>,
    /// The debugger server (#269): `/@brain/metrics` and `/@brain/problems` are proxied to this app dev server.
    pub upstream: Option<String>,
    /// `brain run --mock[=seed]` (#266): every api() answers fake, deterministic data from its schema (RUNTIME's brain-mock.js).
    pub mock: Option<String>,
}

/// `(absolute path) -> diagnostic JSON objects` of that file.
pub type DiagnoseFn = Arc<dyn Fn(&Path) -> Vec<String> + Send + Sync>;
/// `() -> JSON array` of every diagnostic of the app.
pub type ProblemsFn = Arc<dyn Fn() -> String + Send + Sync>;

/// `() -> resolved variables` (the caller also refreshes its `PUBLIC_*` constants for the compiler).
pub type EnvLoader = Arc<dyn Fn() -> Vec<(String, String)> + Send + Sync>;

/// Exact source bytes identify dev output even when an editor preserves file metadata.
type Cache = Arc<Mutex<HashMap<PathBuf, ([u8; 32], Arc<Compiled>)>>>;
type Clients = Arc<Mutex<Vec<TcpStream>>>;
/// Receives every dev server event as JSON: `{"type":"update"|"error"|"reload"|"client-error"|"log", …}`.
pub type EventFn = Arc<dyn Fn(&str) + Send + Sync>;

const MAX_CONNECTIONS: usize = 64;

/// Includes readers, requests waiting for the sidecar and live WebSockets; released even on a handler panic.
struct ConnectionPermit(Arc<AtomicUsize>);
impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
/// Queue time is independent from the running job's deadline; later requests cannot wait behind 64 slow jobs.
fn lock_for<T>(mutex: &Mutex<T>, budget: Duration) -> Option<std::sync::MutexGuard<'_, T>> {
    let deadline = Instant::now() + budget;
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => return None,
            Err(std::sync::TryLockError::WouldBlock) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return None;
                }
                std::thread::sleep(left.min(Duration::from_millis(2)));
            }
        }
    }
}

fn sidecar_slot(
    ctx: &Ctx,
) -> Result<std::sync::MutexGuard<'_, Option<crate::load::Sidecar>>, String> {
    if ctx.stop.load(Ordering::Acquire) {
        return Err("dev server stopped".into());
    }
    lock_for(&ctx.sidecar, Duration::from_secs(5)).ok_or_else(|| {
        "sidecar busy: queue wait exceeded 5 s; retry after the current request finishes".into()
    })
}

fn connection_permit(active: &Arc<AtomicUsize>) -> Option<ConnectionPermit> {
    active
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < MAX_CONNECTIONS).then_some(n + 1)
        })
        .ok()?;
    Some(ConnectionPermit(active.clone()))
}

const RING: usize = 500;
/// Dev pages are cross-origin isolated so the inspector can read the JS heap with
/// `performance.measureUserAgentSpecificMemory()` (§9); `credentialless` keeps cross-origin resources without CORP working.
const ISOLATION: &[(&str, &str)] = &[
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Embedder-Policy", "credentialless"),
];
/// Inspector batches kept for `GET /@brain/metrics` and `brain_inspect` (§9: a 5-minute circular buffer).
const METRICS_MS: u64 = 5 * 60 * 1000;
/// The same buffer is also capped by size: a chatty client cannot hold more than this many batches or bytes.
const METRICS_MAX: usize = 1000;
const METRICS_BYTES: usize = 32 * 1024 * 1024;

/// Appends one inspector batch, dropping the oldest ones past the age, count or byte cap.
fn push_metric(m: &mut VecDeque<(u64, String)>, now: u64, text: String) {
    while m.front().is_some_and(|(at, _)| now.saturating_sub(*at) > METRICS_MS) {
        m.pop_front();
    }
    let mut bytes = m.iter().map(|(_, t)| t.len()).sum::<usize>() + text.len();
    while !m.is_empty() && (m.len() >= METRICS_MAX || bytes > METRICS_BYTES) {
        bytes -= m.pop_front().map_or(0, |(_, t)| t.len());
    }
    m.push_back((now, text));
}

/// Set by `brain run` in rich mode: the terminal view draws the events itself, so the server stops printing them.
static QUIET: AtomicBool = AtomicBool::new(false);

pub fn set_quiet(q: bool) {
    QUIET.store(q, Ordering::Relaxed);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Ctx {
    fn backend_configured(&self)->bool{self.env_vars().iter().any(|(name,value)|name=="BRAIN_BACKEND_URL"&&!value.is_empty())}
    /// The contracts in server/routes, re-read at most every two seconds so a new route needs no restart.
    fn declared_routes(&self) -> Arc<native_proxy::NativeRoutes> {
        let Some(loader) = &self.native_routes else { return Default::default() };
        let mut cache = self.native_routes_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, routes)) = cache.as_ref() && at.elapsed() < Duration::from_secs(2) {
            return routes.clone();
        }
        let routes = Arc::new(native_proxy::NativeRoutes::new(loader()));
        *cache = Some((Instant::now(), routes.clone()));
        routes
    }
    fn backend_target(&self)->Result<Option<native_proxy::Target>,String>{
        self.env_vars().into_iter().find(|(name,value)|name=="BRAIN_BACKEND_URL"&&!value.is_empty()).map(|(_,value)|native_proxy::Target::parse(&value)).transpose()
    }
    /// brain.json `server` (§11.1), read per request so an edit applies without a restart; BRAIN_MAX_BODY_KB,
    /// BRAIN_TIMEOUT_MS and BRAIN_RPC_LIMIT (dev environment, then process) win as in production. An invalid
    /// brain.json value keeps the defaults: `brain check` reports it (B0707).
    fn server_limits(&self) -> brain_pack::ServerLimits {
        let mut l = brain_pack::read_server_limits(&self.root).unwrap_or_default();
        let vars = self.env_vars();
        let var = |name: &str| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).or_else(|| std::env::var(name).ok()).and_then(|v| v.trim().parse::<u64>().ok());
        if let Some(kb) = var("BRAIN_MAX_BODY_KB").filter(|kb| (1..=brain_pack::MAX_BODY_KB_CAP).contains(kb)) { l.max_body_kb = kb; }
        if let Some(ms) = var("BRAIN_TIMEOUT_MS").filter(|ms| *ms > 0) { l.timeout_ms = ms; }
        if let Some(n) = var("BRAIN_RPC_LIMIT").filter(|n| *n > 0) { l.rpc_per_minute = n; }
        l
    }
    /// BRAIN_SECRET of the dev environment (signs the session cookie).
    fn secret(&self) -> String {
        self.env_vars()
            .into_iter()
            .find(|(k, _)| k == "BRAIN_SECRET")
            .map(|(_, v)| v)
            .unwrap_or_default()
    }
    /// The environment `load` and `server:` functions see (never logged).
    fn env_vars(&self) -> Vec<(String, String)> {
        let Some(f) = &self.env else {
            return Vec::new();
        };
        let mut c = self.env_cache.lock().unwrap();
        c.get_or_insert_with(|| f()).clone()
    }
    fn log(&self, level: &'static str, msg: String) {
        if !QUIET.load(Ordering::Relaxed) {
            eprintln!("{msg}");
        }
        let n = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut l = self.logs.lock().unwrap();
        if l.len() >= RING {
            l.pop_front();
        }
        l.push_back((n, now_ms(), level, msg.clone()));
        drop(l);
        self.emit(&format!(
            "{{\"type\":\"log\",\"level\":{},\"msg\":{}}}",
            json_str(level),
            json_str(&msg)
        ));
    }
    fn emit(&self, ev: &str) {
        if let Some(f) = &self.on_event {
            f(ev);
        }
    }
    fn client_message(&self, text: String) {
        if text.starts_with("{\"t\":\"brain:trace\"") {
            if let Some(trace) = &self.trace {
                trace.record(text);
            }
            return;
        }
        if text.starts_with("{\"t\":\"m\"") {
            push_metric(&mut self.metrics.lock().unwrap(), now_ms(), text);
            return;
        }
        if text.contains("\"t\":\"brain:net\"") {
            if let Some(line) = net_line(&text) {
                self.log("net", line);
            }
            return;
        }
        if text.contains("\"brain:error\"") {
            if let Some(trace) = &self.trace {
                trace.record(text.clone());
            }
            let n = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let mut e = self.errors.lock().unwrap();
            if e.len() >= RING {
                e.pop_front();
            }
            e.push_back((n, text.clone()));
            drop(e);
            if !QUIET.load(Ordering::Relaxed) {
                eprintln!("[browser] {}", text);
            }
            self.emit(&format!("{{\"type\":\"client-error\",\"error\":{text}}}"));
        }
    }
}

/// `{"t":"brain:net","method","url","status","ms","size"}` from the browser → `[net] GET /api/users?page 200 12 ms 1.2 KB`.
/// Query values are dropped (they may carry tokens); only their names stay.
fn net_line(text: &str) -> Option<String> {
    let j = brain_lex::json::Json::parse(text).ok()?;
    let s = |k: &str| j.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let n = |k: &str| match j.get(k) {
        Some(brain_lex::json::Json::Num(x)) => *x,
        _ => 0.0,
    };
    let url = s("url");
    let url = match url.split_once('?') {
        Some((p, q)) => format!(
            "{p}?{}",
            q.split('&')
                .map(|kv| kv.split('=').next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("&")
        ),
        None => url,
    };
    let size = n("size");
    let size = if size >= 1024.0 {
        format!("{:.1} KB", size / 1024.0)
    } else {
        format!("{size} B")
    };
    Some(format!(
        "[net] {} {url} {} {} ms {size}",
        s("method"),
        n("status") as i64,
        n("ms") as i64
    ))
}

/// The dev server running inside this process (`brain mcp` uses it to manage it).
pub struct Dev {
    pub addr: SocketAddr,
    ctx: Arc<Ctx>,
    thread: Option<JoinHandle<()>>,
}

impl Dev {
    /// Blocks until the server thread ends.
    pub fn join(mut self) -> Result<(), String> {
        match self.thread.take() {
            Some(t) => t
                .join()
                .map_err(|_| "the server ended with a panic".to_string()),
            None => Ok(()),
        }
    }
    /// The app's URL, under its base (`http://127.0.0.1:5180/app` with brain.json `"base": "/app/"`).
    pub fn url(&self) -> String {
        format!("http://{}{}", self.addr, self.ctx.pre)
    }
    /// Stops the accept loop and the watcher. Open connections finish on their own.
    pub fn stop(&mut self) {
        self.ctx.stop.store(true, Ordering::SeqCst);
        if let Some(proxy) = &self.ctx.native_sockets {
            if proxy.stop().is_err() {
                self.ctx.log("error", "{\"schemaVersion\":1,\"code\":\"BNATIVE_SOCKET_STOP_TIMEOUT\",\"phase\":\"stop\",\"valuesCaptured\":false}".into());
            }
        }
        let _ = TcpStream::connect(self.addr); // wakes up the accept
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        broadcast(&self.ctx.clients, "{\"type\":\"reload\",\"path\":null}");
        let mut clients = self.ctx.clients.lock().unwrap();
        for client in clients.iter() {
            let _ = client.shutdown(Shutdown::Both);
        }
        clients.clear();
    }
    /// Log lines after `since` (seq) at the requested level, as JSON.
    pub fn logs(&self, since: u64, level: Option<&str>) -> String {
        if let Some(proxy) = &self.ctx.native_sockets {
            for item in proxy.drain_diagnostics() { self.ctx.log("error", item.to_json()); }
        }
        let l = self.ctx.logs.lock().unwrap();
        let items: Vec<String> = l
            .iter()
            .filter(|(n, _, lv, _)| *n > since && level.is_none_or(|x| x == *lv))
            .map(|(n, at, lv, m)| {
                format!(
                    "{{\"seq\":{n},\"at\":{at},\"level\":{},\"msg\":{}}}",
                    json_str(lv),
                    json_str(m)
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    }
    /// Browser errors after `since`.
    pub fn errors(&self, since: u64) -> String {
        let e = self.ctx.errors.lock().unwrap();
        let items: Vec<String> = e
            .iter()
            .filter(|(n, _)| *n > since)
            .map(|(n, t)| format!("{{\"seq\":{n},\"error\":{t}}}"))
            .collect();
        format!("[{}]", items.join(","))
    }
    /// Runs `load` for the view at `url` (path + query) and returns `{route, view, params, props}` or the error.
    pub fn load(&self, url: &str) -> Result<String, String> {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        let r = crate::route::resolve(&self.ctx.root, path)
            .ok_or_else(|| format!("no view for `{path}`; fix: create views{path}.brain"))?;
        let req = http::Request {
            upgrade: Default::default(),
            method: "GET".into(),
            path: path.into(),
            query: query.into(),
            headers: vec![],
            body: vec![],
            id:http::request_id(),
            _body_budget:None,
        };
        let props = run_load(&self.ctx, &r.view, &req, &r.params, "null", false)?;
        let params: Vec<String> = r
            .params
            .iter()
            .map(|(k, v)| format!("{}:{}", json_str(k), json_str(v)))
            .collect();
        Ok(format!(
            "{{\"route\":{},\"view\":{},\"params\":{{{}}},\"props\":{props}}}",
            json_str(path),
            json_str(&r.view),
            params.join(",")
        ))
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop();
        }
    }
}

struct Ctx {
    root: PathBuf,
    native_sockets: Option<crate::NativeSocketProxy>,
    native_routes: Option<native_proxy::RouteLoader>,
    native_routes_cache: Mutex<Option<(Instant, Arc<native_proxy::NativeRoutes>)>>,
    env: Option<EnvLoader>,
    /// resolved environment, computed once and dropped when `.brain_env` changes
    env_cache: Mutex<Option<Vec<(String, String)>>>,
    /// Server log (HMR, compilation, load) for `brain_logs`: (seq, epoch ms, level, text).
    logs: Mutex<VecDeque<(u64, u64, &'static str, String)>>,
    /// Browser errors sent by the HMR client (raw JSON) for `brain_errors`.
    errors: Mutex<VecDeque<(u64, String)>>,
    seq: AtomicU64,
    on_event: Option<EventFn>,
    stop: AtomicBool,
    sidecar: Mutex<Option<crate::load::Sidecar>>,
    head_prepend: String,
    /// `base` without its trailing slash: `/app`, or empty at the root.
    pre: String,
    diagnose: Option<DiagnoseFn>,
    problems: Option<ProblemsFn>,
    upstream: Option<String>,
    mock: Option<String>,
    runtime: Vec<(String, String)>,
    compile: CompileFn,
    feedback: crate::feedback::Spool,
    cache: Cache,
    clients: Clients,
    /// URL → the HMR hash (`X.h = "…"`) of its last module that compiled: a broken save is served with that hash so
    /// the error box replaces the live instances in place and the fixed version replaces the box (#176)
    hashes: Mutex<HashMap<String, String>>,
    /// Inspector batches (`{t:"m", seq, at, d:[…]}` from brain-inspect over the HMR socket): (received ms, raw JSON).
    metrics: Mutex<VecDeque<(u64, String)>>,
    /// `brain trace`: present only under `brain run` (INTERNALS §48).
    trace: Option<Arc<crate::trace::Trace>>,
}

/// Blocking dev server with watch and HMR.
pub fn serve(opts: Options, compile: CompileFn) -> io::Result<()> {
    let host = opts.host.clone();
    let (addr, handle) = start(opts, compile, true)?;
    eprintln!("brain run → http://{host}:{}", addr.port());
    handle
        .join()
        .map_err(|_| io::Error::other("the server panicked"))
}

/// Starts the server on a thread and returns the actual address (port 0 = ephemeral, useful for `brain test`).
pub fn start(
    opts: Options,
    compile: CompileFn,
    watch: bool,
) -> io::Result<(SocketAddr, JoinHandle<()>)> {
    let mut d = start_dev(opts, compile, watch, None)?;
    // Without the thread in the handle, dropping it does not stop the server: it lives as long as the process.
    let t = d.thread.take().expect("server thread");
    Ok((d.addr, t))
}

/// Like `start`, but returns the manageable handle (stop, logs, errors, load) and reports events to `on_event`.
pub fn start_dev(
    opts: Options,
    compile: CompileFn,
    watch: bool,
    on_event: Option<EventFn>,
) -> io::Result<Dev> {
    start_dev_with_native_proxy(opts, compile, watch, on_event, None)
}

/// An explicit native proxy fixes the HTTP, SSR and socket authority until this managed server is recreated.
/// The legacy entry point keeps its environment-based HTTP forwarding and does not activate sockets.
pub fn start_dev_with_native_proxy(
    opts: Options,
    compile: CompileFn,
    watch: bool,
    on_event: Option<EventFn>,
    native: Option<native_proxy::NativeProxy>,
) -> io::Result<Dev> {
    start_with_threads(opts, compile, watch, on_event, native, Threads::default())
}

fn start_with_threads(
    mut opts: Options,
    compile: CompileFn,
    watch: bool,
    on_event: Option<EventFn>,
    native: Option<native_proxy::NativeProxy>,
    threads: Threads,
) -> io::Result<Dev> {
    let root = opts.root.canonicalize()?;
    // If the port is taken, try the next 20 (except 0, which the OS assigns). A restart first gets a moment to release it.
    // `brain run` with a declared port never gets here with it taken: the CLI claims it first and refuses to move.
    let tries = if opts.port == 0 { 0 } else { 20 };
    let requested = crate::listen::bind_waiting(&opts.host, opts.port, std::time::Duration::from_secs(1)).ok();
    let listener = requested
        .or_else(|| (1..=tries).find_map(|i| TcpListener::bind((opts.host.as_str(), opts.port.saturating_add(i))).ok()))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("ports {}-{} are in use; fix: find the owner of the first with `lsof -nP -iTCP:{} -sTCP:LISTEN` (Linux: `ss -ltnp 'sport = :{}'`) or pick another --port", opts.port, opts.port.saturating_add(20), opts.port, opts.port),
            )
        })?;
    let addr = listener.local_addr()?;
    let feedback = crate::feedback::Spool::new(&root).map_err(io::Error::other)?;
    let native_sockets = native.as_ref().map(|proxy| proxy.start_sockets()).transpose()?.flatten();
    let native_routes = native.as_ref().and_then(|proxy| proxy.route_loader());
    if let Some(proxy) = native {
        let origin = proxy.origin();
        let previous = opts.env.take();
        opts.env = Some(Arc::new(move || {
            let mut values = previous.as_ref().map(|loader| loader()).unwrap_or_default();
            values.retain(|(name, _)| name != "BRAIN_BACKEND_URL");
            values.push(("BRAIN_BACKEND_URL".into(), origin.clone()));
            values
        }));
    }
    let trace = crate::trace::attach(&root);
    let ctx = Arc::new(Ctx {
        native_routes,
        native_routes_cache: Mutex::new(None),
        trace,
        root,
        native_sockets,
        env: opts.env,
        env_cache: Mutex::new(None),
        logs: Mutex::default(),
        errors: Mutex::default(),
        seq: AtomicU64::new(0),
        on_event,
        stop: AtomicBool::new(false),
        sidecar: Mutex::new(None),
        head_prepend: opts.head_prepend,
        pre: opts.base.trim_end_matches('/').to_string(),
        diagnose: opts.diagnose,
        problems: opts.problems,
        upstream: opts.upstream,
        mock: opts.mock,
        runtime: opts.runtime,
        compile,
        feedback,
        cache: Arc::default(),
        clients: Arc::default(),
        hashes: Mutex::default(),
        metrics: Mutex::default(),
    });
    // resolve the environment now: it also sets the PUBLIC_* constants the compiler substitutes, so the very first
    // module served (whatever the first request is) already has them (QA #140)
    let _ = ctx.env_vars();
    threads.observe(&ctx);
    let watcher = if watch {
        Some(start_watch(ctx.clone(), &threads).map_err(|error| dev_threads::startup_error(&ctx, error))?)
    } else {
        None
    };
    let lctx = ctx.clone();
    let dispatch = threads.clone();
    let handle = threads.spawn(Role::Accept, move || {
        let _watcher = watcher;
        let active = Arc::new(AtomicUsize::new(0));
        let mut failures = FailureLog::default();
        for stream in listener.incoming() {
            if lctx.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(mut stream) = stream else { continue };
            let Some(permit) = connection_permit(&active) else {
                let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                let _ = http::respond_with(
                    &mut stream,
                    503,
                    "text/plain",
                    b"dev server busy; retry after a request finishes",
                    false,
                    &[("Retry-After", "1")],
                );
                continue;
            };
            let ctx = lctx.clone();
            let pending = PendingRequest::new(stream, permit);
            if dispatch.spawn(Role::Request, move || pending.run(ctx)).is_err() {
                failures.failed(&lctx);
            }
        }
        failures.finish(&lctx);
    }).map_err(|error| dev_threads::startup_error(&ctx, error))?;
    Ok(Dev {
        addr,
        ctx,
        thread: Some(handle),
    })
}

/// One total request-read budget; trickling bytes cannot renew it indefinitely.
struct RequestReader<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}
impl Read for RequestReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "request read deadline exceeded",
            ));
        }
        self.stream.set_read_timeout(Some(left))?;
        let mut stream = self.stream;
        stream.read(bytes)
    }
}

fn handle(mut s: TcpStream, ctx: &Ctx) -> io::Result<()> {
    s.set_write_timeout(Some(Duration::from_secs(5)))?;
    let upload_path=ctx.backend_configured().then(||format!("{}/_brain/uploads",ctx.pre));
    let limits = ctx.server_limits();
    // Expect: 100-continue (§11.1): the interim response once the declared length fits brain.json server.maxBodyKB
    let mut send_continue = || -> io::Result<()> { (&s).write_all(b"HTTP/1.1 100 Continue\r\n\r\n") };
    let routes = ctx.declared_routes();
    let plan = |path: &str, method: &str| {
        if !ctx.backend_configured() { return http::BodyPlan::Default }
        match native_proxy::plan(path, method, &routes, &ctx.pre) {
            // brain.json server.maxBodyKB keeps governing a larger limit in development
            http::BodyPlan::Limit(limit) => http::BodyPlan::Limit(limit.max(limits.max_body_bytes())),
            other => other,
        }
    };
    let read = http::read_request_planned(RequestReader {
        stream: &s,
        // a large body limit gets a longer total read budget (1 s per 2 MB on top of the 5 s for headers)
        deadline: Instant::now() + Duration::from_secs(5 + limits.max_body_kb / 2048),
    },upload_path.as_deref(),limits.max_body_bytes(),Some(&mut send_continue),&plan);
    let mut req = match read {
        Ok(request)=>request,
        Err(error)=>{let status=http::request_error_status(&error);let rid=http::request_id();
            // a 413 names the setting to raise; every other read failure stays generic
            let text=error.to_string();let message=match text.strip_prefix("HTTP413 ") {Some(m) if status==413 => m.to_string(), _ => "Invalid or over-budget request".into()};
            let body=format!("{{\"error\":{},\"code\":\"BHTTP_REQUEST\",\"rid\":{}}}",json_str(&message),json_str(&rid));return http::respond_with(&mut s,status,"application/json",body.as_bytes(),false,&[("X-Request-Id",&rid)]);}
    };
    // base (#148): `/app` → `/app/`; `/app/x` is routed as `/x`; anything outside the base is a 404, as in production
    if !ctx.pre.is_empty() {
        if req.path == ctx.pre {
            let to = format!(
                "{}/{}",
                ctx.pre,
                if req.query.is_empty() {
                    String::new()
                } else {
                    format!("?{}", req.query)
                }
            );
            return http::respond_with(&mut s, 301, "text/plain", b"", false, &[("Location", &to)]);
        }
        match req
            .path
            .strip_prefix(&ctx.pre)
            .filter(|r| r.starts_with('/'))
        {
            Some(r) => req.path = r.to_string(),
            None => {
                let msg = format!(
                    "not found: this app is served under {}/ (brain.json \"base\")",
                    ctx.pre
                );
                return http::respond(
                    &mut s,
                    404,
                    "text/plain; charset=utf-8",
                    msg.as_bytes(),
                    req.method == "HEAD",
                );
            }
        }
    }
    let native = ctx.backend_configured() && (native_proxy::handles(&req.path) || routes.find(&req.path).is_some());
    let _traced = ctx.trace.as_ref().map(|trace| crate::trace::begin(trace, &mut req, native));
    if req.path == "/_brain/socket" {
        return match &ctx.native_sockets {
            Some(proxy) => proxy.takeover(&mut s, &mut req),
            None => http::respond_with(&mut s, 404, "application/json", b"{\"code\":\"BNATIVE_SOCKET_DISABLED\"}", false, &[("Cache-Control", "no-store")]),
        };
    }
    // zero-config SEO in dev: /robots.txt and /sitemap.xml answer like a production build (static routes, paths() and
    // sitemap({ env }) of live views) with no build step; a file in public/ still wins
    if (req.method == "GET" || req.method == "HEAD")
        && dev_seo_file(&req.path)
        && !ctx.root.join("public").join(&req.path[1..]).is_file()
        && let Some((ctype, body)) = dev_seo(ctx, &req)
    {
        return http::respond_with(&mut s, 200, ctype, body.as_bytes(), req.method == "HEAD", &[("Cache-Control", "no-store")]);
    }
    if let Some(probe) = native_proxy::probe_alias(&req.path) {
        if !matches!(req.method.as_str(), "GET" | "HEAD") {
            return http::respond_with(&mut s, 405, "application/json", b"{\"error\":\"Health probes require GET or HEAD\"}", false, &[("Allow", "GET, HEAD"), ("Cache-Control", "no-store")]);
        }
        req.path = probe.to_string();
        return match ctx.backend_target() {
            Ok(Some(target)) => native_proxy::serve(&mut s, &target, &req, None),
            Ok(None) => http::respond_with(&mut s, 200, "application/json", b"{\"ok\":true}", req.method == "HEAD", &[("Cache-Control", "no-store")]),
            Err(_) => http::respond_with(&mut s, 503, "application/json", b"{\"ok\":false}", req.method == "HEAD", &[("Cache-Control", "no-store")]),
        };
    }
    if native_proxy::handles(&req.path) {
        match ctx.backend_target(){
            Ok(Some(target))=>return if req.upgrade.stream_body.is_some() {native_proxy::serve_files(&mut s,&target,&req)} else {native_proxy::serve(&mut s,&target,&req,None)},
            Ok(None)=>{},
            Err(_)=>return http::respond(&mut s,503,"application/json",b"{\"error\":\"Native backend configuration is invalid; use an explicit loopback HTTP origin\"}",req.method=="HEAD"),
        }
    } else if let Some(route) = routes.find(&req.path) && let Ok(Some(target)) = ctx.backend_target() {
        return native_proxy::serve(&mut s,&target,&req,Some(route));
    }
    if let Some(id) = req.path.strip_prefix("/_r/") {
        let (status, body, headers) = rpc(ctx, &req, id);
        let extra: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        return http::respond_with(
            &mut s,
            status,
            "application/json",
            body.as_bytes(),
            false,
            &extra,
        );
    }
    // a form posted without JS to a page (§38): same handling as dist/server.mjs (callForm)
    if req.method == "POST"
        && Path::new(&req.path).extension().is_none()
        && ctx.root.join("app.brain").is_file()
    {
        return form_post(&mut s, ctx, &req);
    }
    let head = req.method == "HEAD";
    if req.method != "GET" && !head {
        return http::respond(&mut s, 405, "text/plain", b"GET only", false);
    }
    match req.path.as_str() {
        "/@brain/hmr" => {
            let Some(key) = req.header("sec-websocket-key") else {
                return http::respond(
                    &mut s,
                    400,
                    "text/plain",
                    b"missing Sec-WebSocket-Key",
                    head,
                );
            };
            s.write_all(ws::handshake_response(key).as_bytes())?;
            s.set_read_timeout(None)?; // WebSockets are intentionally long-lived, unlike request headers.
            s.set_write_timeout(Some(Duration::from_millis(100)))?;
            let peer = s.peer_addr()?;
            ctx.clients.lock().unwrap().push(s.try_clone()?);
            let drained = ws::drain_client(&mut s, &mut |t| ctx.client_message(t));
            // Unregister on close or error: otherwise the clone (and its descriptor) lives until the next broadcast.
            ctx.clients
                .lock()
                .unwrap()
                .retain(|c| c.peer_addr().is_ok_and(|a| a != peer));
            return drained;
        }
        "/@brain/client.js" => {
            return http::respond(&mut s, 200, http::mime("x.js"), CLIENT_JS.as_bytes(), head);
        }
        // the client imports it unconditionally: without capture it is an empty module, so nothing wraps `fetch`
        "/@brain/brain-trace.js" if ctx.trace.is_none() || !ctx.runtime.iter().any(|(name, _)| name == "brain-trace.js") => {
            return http::respond(&mut s, 200, http::mime("x.js"), crate::trace::CLIENT_OFF.as_bytes(), head);
        }
        "/@brain/test.html" => {
            // tests read the default language's texts, like the page at `/` (t() would otherwise return the key)
            let mut pre = ctx.head_prepend.clone();
            if let Some((def, _)) = brain_pack::read_i18n(&ctx.root)
            {
                let dict = brain_pack::read_dict(&ctx.root, &def)
                    .iter()
                    .map(|(k, v)| format!("{}:{v}", json_str(k)))
                    .collect::<Vec<_>>()
                    .join(",");
                pre.push_str(&format!("<script type=\"module\">import{{add}}from'/@brain/brain-i18n.js';add({},{{{dict}}});</script>", json_str(&def)));
            }
            let html = prepend_head(TEST_HTML, &pre);
            return http::respond(&mut s, 200, http::mime("x.html"), html.as_bytes(), head);
        }
        "/@brain/test-api.js" => {
            return http::respond(
                &mut s,
                200,
                http::mime("x.js"),
                TEST_API_JS.as_bytes(),
                head,
            );
        }
        "/@brain/ping" => return http::respond(&mut s, 200, "text/plain", b"ok", head),
        // the inspector's data (§9): batches of the last 5 minutes (`?since=<ms>` for newer ones only) and every leak
        // reported so far, so a model can diagnose memory without a screen (MCP brain_inspect reads this)
        // the debugger (#269) reads the app's data on its own origin: these two go to the app's dev server
        "/@brain/metrics" | "/@brain/problems" if ctx.upstream.is_some() => {
            let target = format!(
                "{}{}",
                req.path,
                if req.query.is_empty() {
                    String::new()
                } else {
                    format!("?{}", req.query)
                }
            );
            let (status, body) = proxy_get(ctx.upstream.as_deref().unwrap_or(""), &target);
            return http::respond_with(
                &mut s,
                status,
                "application/json",
                body.as_bytes(),
                head,
                &[("Cache-Control", "no-store")],
            );
        }
        // every diagnostic of the app (`brain check --json`) plus its family (B05 = a11y…) and component, for the
        // debugger's Problems view (#269); refreshed by it every 3 s
        "/@brain/problems" => {
            let body = match &ctx.problems {
                Some(f) => f(),
                None => "[]".into(),
            };
            return http::respond_with(
                &mut s,
                200,
                "application/json",
                body.as_bytes(),
                head,
                &[("Cache-Control", "no-store")],
            );
        }
        "/@brain/metrics" => {
            let since: u64 = req
                .query
                .split('&')
                .find_map(|kv| kv.strip_prefix("since="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let body = ctx.metrics_json(since);
            return http::respond_with(
                &mut s,
                200,
                "application/json",
                body.as_bytes(),
                head,
                &[("Cache-Control", "no-store")],
            );
        }
        // the route state shown while a client navigation waits for the next page's load (§22, #235): the nearest
        // `_loading.brain` of the route `?path=` resolves to, and its layouts; `{}` when the route has none
        "/@brain/loading" => {
            let path = req
                .query
                .split('&')
                .find_map(|kv| kv.strip_prefix("path="))
                .map(http::percent_decode)
                .unwrap_or_else(|| "/".into());
            let path = if ctx.pre.is_empty() {
                path
            } else {
                path.strip_prefix(&ctx.pre)
                    .map(str::to_string)
                    .unwrap_or(path)
            };
            let body = match crate::route::resolve(&ctx.root, &path)
                .and_then(|r| loading_view(&ctx.root, &r.view).map(|v| (v, r)))
            {
                Some((v, r)) => {
                    let l: Vec<String> = r
                        .layouts
                        .iter()
                        .map(|l| json_str(&format!("/{l}")))
                        .collect();
                    format!(
                        "{{\"v\":{},\"l\":[{}]}}",
                        json_str(&format!("/{v}")),
                        l.join(",")
                    )
                }
                None => "{}".into(),
            };
            return http::respond(&mut s, 200, "application/json", body.as_bytes(), head);
        }
        // dev never runs a service worker (INTERNALS §35): a worker left by `brain preview` on this origin updates to
        // this one, which deletes its caches and unregisters itself
        "/sw.js" => {
            return http::respond(
                &mut s,
                200,
                http::mime("x.js"),
                brain_pack::cache::kill_sw("/").as_bytes(),
                head,
            );
        }
        p if p.starts_with("/@brain/ui/") && p.ends_with(".brain") => {
            // brain/ui kit (INTERNALS §14): embedded in the binary; written to .brain/kit/ and compiled like any .brain.
            let name = &p["/@brain/".len()..];
            let Some((_, body)) = ctx.runtime.iter().find(|(k, _)| k == name) else {
                return http::respond(
                    &mut s,
                    404,
                    "text/plain; charset=utf-8",
                    format!("`{name}` is not in the brain/ui kit").as_bytes(),
                    head,
                );
            };
            let disk = ctx.root.join(".brain/kit").join(name);
            if std::fs::read_to_string(&disk).ok().as_deref() != Some(body.as_str()) {
                let _ = std::fs::create_dir_all(disk.parent().unwrap());
                std::fs::write(&disk, body)?;
            }
            return serve_brain(
                &mut s,
                ctx,
                &disk,
                &req.path,
                req.query.split('&').any(|q| q == "css"),
                head,
            );
        }
        p => {
            if let Some((_, body)) = p
                .strip_prefix("/@brain/")
                .filter(|n| !n.starts_with("shim/"))
                .and_then(|n| ctx.runtime.iter().find(|(k, _)| k == n))
            {
                return http::respond(
                    &mut s,
                    200,
                    http::mime("x.js"),
                    ctx.js_out(body).as_bytes(),
                    head,
                );
            }
        }
    }
    let Some(file) = resolve(&ctx.root, &req.path) else {
        return http::respond(&mut s, 403, "text/plain", b"path outside the root", head);
    };
    if file.to_string_lossy().ends_with(".test.brain") && file.is_file() {
        // Test file: its <script> is BrainScript ⊂ JS and is served as a module as is.
        let src = std::fs::read_to_string(&file).unwrap_or_default();
        let body = script_body(&src);
        return http::respond(&mut s, 200, http::mime("x.js"), body.as_bytes(), head);
    }
    if file
        .extension()
        .is_some_and(|e| e == "brain" || e == "jsbrain")
        && file.is_file()
    {
        return serve_brain(
            &mut s,
            ctx,
            &file,
            &req.path,
            req.query.split('&').any(|q| q == "css"),
            head,
        );
    }
    let public_file = resolve(&ctx.root.join("public"), &req.path).filter(|path| path.is_file());
    let static_file = if file.is_dir() { file.join("index.html") } else { file.clone() };
    let i18n = brain_pack::read_i18n(&ctx.root);
    let (_, route_path) = split_locale(&i18n, &req.path);
    let has_security_chain = public_file.is_some() && crate::route::resolve(&ctx.root, &route_path)
        .filter(|route| !route.not_found)
        .is_some_and(|route| !chain_files(&ctx.root, &route.view, "_guard.jsbrain").is_empty()
            || !chain_files(&ctx.root, &route.view, "_middleware.jsbrain").is_empty());
    let public_first = !static_file.exists() && public_file.is_some() && !has_security_chain;
    // Explicit public files win over page fallback, including extensionless verification files.
    if !public_first && (has_security_chain || Path::new(&req.path).extension().is_none()) && ctx.root.join("app.brain").is_file() {
        let (status, html, location) = app_page(ctx, &req);
        if let Some(loc) = location {
            return http::respond_with(
                &mut s,
                status,
                "text/plain",
                b"",
                head,
                &[("Location", &ctx.under_base(&loc))],
            );
        }
        let html = ctx.page_out(&html);
        return http::respond_with(
            &mut s,
            status,
            http::mime("x.html"),
            html.as_bytes(),
            head,
            ISOLATION,
        );
    }
    let file = static_file;
    // public/ is served at the root, as in dist/
    let file = match public_file {
        Some(p) if !file.exists() => p,
        _ => file,
    };
    // browsers still ask for /favicon.ico: the app's SVG icon answers it, as in production
    let file = if req.path == "/favicon.ico" && !file.exists() && ctx.root.join("public/favicon.svg").is_file() {
        ctx.root.join("public/favicon.svg")
    } else {
        file
    };
    // §10 transition: an app that still has tokens.brain.css also answers at /tokens.cssbrain.
    let file = if req.path == "/tokens.cssbrain" && !file.exists() {
        ctx.root.join("tokens.brain.css")
    } else {
        file
    };
    // SPA: a path without extension that does not exist falls back to index.html.
    let file = if !file.exists() && Path::new(&req.path).extension().is_none() {
        ctx.root.join("index.html")
    } else {
        file
    };
    match std::fs::read(&file) {
        Ok(body) if file.extension().is_some_and(|e| e == "html") => {
            let html = ctx.page_out(&String::from_utf8_lossy(&body));
            http::respond_with(
                &mut s,
                200,
                http::mime("x.html"),
                html.as_bytes(),
                head,
                ISOLATION,
            )
        }
        // Safari plays video only from byte ranges: one span of the file, as `brain start` answers it
        Ok(body) => {
            let ctype = http::mime(&file.to_string_lossy());
            let size = body.len() as u64;
            match req.header("range").filter(|_| req.header("if-range").is_none()).and_then(|r| crate::prod::byte_range(r, size)) {
                Some(Err(())) => http::respond_with(&mut s, 416, "text/plain", b"", head, &[("Content-Range", &format!("bytes */{size}"))]),
                Some(Ok((start, end))) => http::respond_with(
                    &mut s,
                    206,
                    ctype,
                    &body[start as usize..=end as usize],
                    head,
                    &[("Accept-Ranges", "bytes"), ("Content-Range", &format!("bytes {start}-{end}/{size}"))],
                ),
                None => http::respond_with(&mut s, 200, ctype, &body, head, &[("Accept-Ranges", "bytes")]),
            }
        }
        Err(_) => http::respond(&mut s, 404, "text/plain", b"not found", head),
    }
}

/// Joins the URL path to the root, rejecting `..` and absolute paths.
fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = Path::new(url_path.trim_start_matches('/'));
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(root.join(rel))
}

/// Writes the server code of `rel` to `.brain/server/` with absolute imports and returns its `file://…?v=` URL.
/// A `.jsbrain`/`.brain` imported from server code is written the same way (its `server_js` or, if it has none, its `js`).
fn server_module(ctx: &Ctx, rel: &str, depth: u32) -> Result<String, String> {
    if depth > 32 {
        return Err(format!("server imports nested too deep (cycle?) in {rel}"));
    }
    let disk = ctx.root.join(rel);
    let c = compile_cached(ctx, &disk)
        .map_err(|d| format!("{rel} does not compile: {}", d.join(", ")))?;
    let code = if c.server_js.trim().is_empty() {
        c.js.clone()
    } else {
        c.server_js.clone()
    };
    let dir = disk.parent().unwrap_or(&ctx.root).to_path_buf();
    let code = brain_pack::rewrite_specifiers(&code, &dir, &mut |abs| {
        let abs = abs
            .canonicalize()
            .map_err(|e| format!("{}: {e}", abs.display()))?;
        let is_brain = abs
            .extension()
            .is_some_and(|e| e == "brain" || e == "jsbrain");
        match abs.strip_prefix(&ctx.root) {
            Ok(r) if is_brain => {
                server_module(ctx, &r.to_string_lossy().replace('\\', "/"), depth + 1)
            }
            _ => Ok(crate::load::file_url(&abs)),
        }
    })?;
    // runtime imports (`/@brain/brain-ch.js`) → the copies under .brain/server/_rt/; they import each other with
    // `./brain.js`, so every top-level runtime module is written next to them
    let code = brain_pack::rewrite_imports(&code, &mut |spec| {
        if spec == "brain:content" {
            return content_module(ctx).map(Some);
        }
        let Some(name) = spec.strip_prefix("/@brain/") else {
            return Ok(None);
        };
        for (n, _) in ctx
            .runtime
            .iter()
            .filter(|(n, _)| n.ends_with(".js") && !n.contains('/'))
        {
            runtime_file(ctx, n);
        }
        runtime_file(ctx, name)
            .map(Some)
            .ok_or_else(|| format!("{rel} imports `{spec}`, which is not a runtime module"))
    })?;
    let out = crate::load::server_path(&ctx.root, rel);
    std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
    if std::fs::read_to_string(&out).ok().as_deref() != Some(code.as_str()) {
        std::fs::write(&out, &code).map_err(|e| e.to_string())?;
    }
    Ok(crate::load::file_url(&out))
}

/// Paths the dev server generates like a build: robots.txt, sitemap.xml (+ sitemap-N.xml), llms.txt (+ /<locale>/llms.txt).
fn dev_seo_file(path: &str) -> bool {
    let name = path.trim_start_matches('/');
    let name = name.split_once('/').filter(|(l, _)| !l.is_empty() && l.len() <= 8 && !l.contains('.')).map_or(name, |(_, n)| n);
    path == "/robots.txt"
        || path == "/sitemap.xml"
        || name == "llms.txt"
        || (path.starts_with("/sitemap-") && path.ends_with(".xml") && path[9..path.len() - 4].bytes().all(|b| b.is_ascii_digit()))
}

/// Pages a dev sitemap renders to read their head and static images (beyond it, URLs are listed without them).
const DEV_SEO_RENDER_MAX: usize = 300;
/// Wall time a dev sitemap spends rendering pages: each render runs the view's load(), so 300 pages at 0.4 s held the
/// request past two minutes; past the budget the remaining URLs are listed without their head, like past the cap.
const DEV_SEO_RENDER_BUDGET: Duration = Duration::from_secs(3);

/// Whether the dev sitemap still renders page `n` after `spent` rendering time.
fn dev_seo_renders(n: usize, spent: Duration) -> bool {
    n < DEV_SEO_RENDER_MAX && spent < DEV_SEO_RENDER_BUDGET
}

/// Dev `/robots.txt`, `/sitemap.xml` and `/llms.txt` (zero config): the origin is the request's Host, so the files always
/// point at the server that answers them. Pages: every static route, plus the params a dynamic view's `paths()` or
/// `sitemap({ env })` returns, in every locale; views marked `noindex` and the error views are left out. Each page is
/// rendered once so the files carry what the build writes: the page title and description (llms.txt), hreflang
/// alternates with x-default, and image entries for the view's static `<img src>` (dev renders on the client, so images
/// bound to props only appear in the build).
fn dev_seo(ctx: &Ctx, req: &http::Request) -> Option<(&'static str, String)> {
    let mut cfg = brain_seo::Config::load(&ctx.root).unwrap_or_default();
    cfg.site = Some(format!("http://{}", req.header("host").unwrap_or("localhost")));
    cfg.staging = false;
    if req.path == "/robots.txt" {
        return Some(("text/plain; charset=utf-8", brain_seo::files::robots(&cfg)));
    }
    let env = format!(
        "{{\"env\":{{{}}}}}",
        ctx.env_vars().iter().map(|(k, v)| format!("{}:{}", json_str(k), json_str(v))).collect::<Vec<_>>().join(",")
    );
    cfg.markdown = false; // dev serves no .md copies: llms.txt links the pages themselves
    let mut urls: Vec<(String, Option<String>, String)> = Vec::new();
    for r in brain_pack::routes::discover(&ctx.root) {
        let src = std::fs::read_to_string(ctx.root.join(&r.view)).unwrap_or_default();
        let noindex = brain_parse::parse(&src, std::path::Path::new(&r.view)).map_or_else(|_| src.contains("noindex"), |c| brain_check::seo::declares_noindex(&c));
        if r.path == "*" || noindex || ["404", "410", "500"].iter().any(|n| r.view.ends_with(&format!("/{n}.brain"))) {
            continue;
        }
        if !r.path.contains([':', '*']) {
            urls.push((r.path.clone(), None, r.view.clone()));
            continue;
        }
        let Ok(c) = compile_cached(ctx, &ctx.root.join(&r.view)) else { continue };
        let func = if c.server.sitemap { Some("sitemap") } else if c.server.paths { Some("paths") } else { None };
        let Some(func) = func else { continue };
        let listed = (|| -> Result<String, String> {
            let url = server_module(ctx, &r.view, 0)?;
            let mut sc = sidecar_slot(ctx)?;
            if sc.is_none() {
                *sc = Some(crate::load::Sidecar::spawn(&ctx.root, &ctx.env_vars()).map_err(|e| e.to_string())?);
            }
            sc.as_mut().unwrap().call(&url, func, &env)
        })();
        let list = match listed.as_deref().map(brain_lex::Json::parse) {
            Ok(Ok(brain_lex::Json::Arr(a))) => a,
            Err(e) => {
                ctx.log("warn", format!("[seo] {}: {func}() failed: {}", r.view, e.lines().next().unwrap_or("")));
                continue;
            }
            _ => continue,
        };
        for item in list {
            let mut ok = true;
            let path = r
                .path
                .split('/')
                .map(|seg| match seg.strip_prefix([':', '*']) {
                    Some(name) => match item.get(name) {
                        Some(brain_lex::Json::Str(v)) if !v.is_empty() => brain_seo::files::encode_path(v),
                        Some(brain_lex::Json::Num(n)) => n.to_string(),
                        _ => {
                            ok = false;
                            String::new()
                        }
                    },
                    None => seg.to_string(),
                })
                .collect::<Vec<_>>()
                .join("/");
            let lastmod = item.get("lastmod").and_then(brain_lex::Json::as_str).filter(|d| d.len() >= 10).map(|d| d[..10].to_string());
            if ok {
                urls.push((path, lastmod, r.view.clone()));
            }
        }
    }
    urls.sort();
    urls.dedup();
    let i18n = brain_pack::read_i18n(&ctx.root)
        .filter(|(_, ls)| ls.len() > 1)
        .map(|(default, locales)| brain_seo::hreflang::I18n { default, locales });
    let pages: Vec<(String, Option<String>, String)> = match &i18n {
        Some(i) => urls.iter().flat_map(|(p, d, v)| i.locales.iter().map(move |l| (i.prefixed(l, p), d.clone(), v.clone()))).collect(),
        None => urls,
    };
    let alternates = match &i18n {
        Some(i) => brain_seo::hreflang::alternates(i, &pages.iter().map(|(p, _, v)| (p.as_str(), Some(v.as_str()))).collect::<Vec<_>>()),
        None => Default::default(),
    };
    let mut docs = Vec::with_capacity(pages.len());
    let rendering = Instant::now();
    let mut cut = false;
    for (n, (path, _, view)) in pages.iter().enumerate() {
        let renders = dev_seo_renders(n, rendering.elapsed());
        if !renders && !cut && n < DEV_SEO_RENDER_MAX {
            cut = true;
            ctx.log("info", format!("[seo] {}: rendered {n} of {} pages in {} ms; the rest are listed without their head", req.path, pages.len(), rendering.elapsed().as_millis()));
        }
        let html = if renders {
            let page = http::Request {
                upgrade: Default::default(),
                method: "GET".into(),
                path: path.clone(),
                query: String::new(),
                headers: req.headers.clone(),
                body: vec![],
                id: req.id.clone(),
                _body_budget: None,
            };
            match app_page(ctx, &page) {
                (200, html, None) => html,
                _ => {
                    docs.push(None);
                    continue;
                }
            }
        } else {
            String::new()
        };
        let src = std::fs::read_to_string(ctx.root.join(view)).unwrap_or_default();
        let html = match html.rfind("</body>") {
            Some(i) => format!("{}{}{}", &html[..i], static_images(&src), &html[i..]),
            None => format!("{html}{}", static_images(&src)),
        };
        docs.push(Some(brain_seo::html::parse(&html)));
    }
    let entries: Vec<brain_seo::files::Entry> = pages
        .iter()
        .zip(&docs)
        .filter_map(|((path, lastmod, _), doc)| {
            let doc = doc.as_ref()?;
            let mut alt: Vec<(String, String)> = alternates
                .get(path)
                .map_or(vec![], |v| v.iter().map(|(l, r)| (l.clone(), cfg.abs(&brain_seo::files::encode_path(r)))).collect());
            if let Some(i) = &i18n
                && let Some(d) = alt.iter().find(|(l, _)| *l == i.default).map(|(_, u)| u.clone())
            {
                alt.push(("x-default".into(), d));
            }
            Some(brain_seo::files::Entry {
                route: path,
                url: cfg.abs(&brain_seo::files::encode_path(path)),
                doc,
                lastmod: lastmod.as_deref(),
                indexable: brain_seo::audit::indexable(&cfg, path, doc),
                alternates: alt,
                videos: vec![],
            })
        })
        .collect();
    let name = req.path.trim_start_matches('/');
    if let Some(dir) = name.strip_suffix("llms.txt") {
        if !cfg.llms {
            return None;
        }
        let locale = dir.trim_end_matches('/');
        let list: Vec<brain_seo::files::Entry> = match &i18n {
            Some(i) if locale.is_empty() || i.locales.iter().any(|l| l == locale && *l != i.default) => {
                let want = if locale.is_empty() { i.default.as_str() } else { locale };
                entries.iter().filter(|e| i.locale_of(e.route) == want).cloned().collect()
            }
            _ if locale.is_empty() => entries,
            _ => return None,
        };
        return Some(("text/plain; charset=utf-8", brain_seo::files::llms_txt(&cfg, &list)));
    }
    let files = brain_seo::files::sitemaps(&cfg, &entries);
    let empty = format!("{}<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n</urlset>\n",if cfg.compiler_branding {brain_lex::branding::XML}else{"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"});
    match files.into_iter().find(|(n, _)| n == name) {
        Some((_, body)) => Some(("application/xml; charset=utf-8", body)),
        None if name == "sitemap.xml" => Some(("application/xml; charset=utf-8", empty.into())),
        None => None,
    }
}

/// The view's `<img>` tags whose `src` is a literal (no `{expr}`), as `<img src alt>` for the dev sitemap's image entries.
fn static_images(src: &str) -> String {
    let (markup, _) = take_block(src, "script");
    let mut out = String::new();
    let mut rest = markup.as_str();
    while let Some(i) = rest.find("<img") {
        let tag = &rest[i..];
        let end = tag.find('>').unwrap_or(tag.len());
        let attrs = brain_seo::html::parse_attrs(&tag[4..end]);
        if let Some(src) = brain_seo::html::attr(&attrs, "src").filter(|v| !v.is_empty() && !v.contains('{')) {
            out.push_str(&format!("<img src=\"{}\" alt=\"\">", html_escape(src)));
        }
        rest = &tag[end..];
    }
    out
}

fn query_json(q: &str) -> String {
    let kv: Vec<String> = q
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let dec = |x: &str| http::percent_decode(&x.replace('+', " "));
            format!("{}:{}", json_str(&dec(k)), json_str(&dec(v)))
        })
        .collect();
    format!("{{{}}}", kv.join(","))
}

fn cookies_json(h: Option<&str>) -> String {
    let kv: Vec<String> = h
        .unwrap_or("")
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .map(|(k, v)| format!("{}:{}", json_str(k.trim()), json_str(v.trim())))
        .collect();
    format!("{{{}}}", kv.join(","))
}

/// `load`'s `ctx` (shape agreed with COMPILER, INTERNALS §11.1): params, query, url, req, cookies; Node adds `env`.
fn load_ctx_json(req: &http::Request, params: &[(String, String)]) -> String {
    let params: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}:{}", json_str(k), json_str(v)))
        .collect();
    let headers: Vec<String> = req
        .headers
        .iter()
        .filter(|(name,_)|!name.eq_ignore_ascii_case("forwarded")&&!name.to_ascii_lowercase().starts_with("x-forwarded-")&&!name.eq_ignore_ascii_case("x-request-id"))
        .map(|(k, v)| format!("{}:{}", json_str(&k.to_ascii_lowercase()), json_str(v)))
        .collect();
    let url = if req.query.is_empty() {
        req.path.clone()
    } else {
        format!("{}?{}", req.path, req.query)
    };
    format!(
        "{{\"params\":{{{}}},\"query\":{},\"url\":{},\"req\":{{\"method\":{},\"headers\":{{{}}}}},\"cookies\":{},\"rid\":{}}}",
        params.join(","),
        query_json(&req.query),
        json_str(&url),
        json_str(&req.method),
        headers.join(","),
        cookies_json(req.header("cookie")),json_str(&req.id)
    )
}

/// Runs the view's `load(ctx)` if it has one; returns the props JSON.
/// `user`: `ctx.user` as JSON (from the signed session, through the guard chain), `null` when anonymous.
fn run_load(
    ctx: &Ctx,
    view: &str,
    req: &http::Request,
    params: &[(String, String)],
    user: &str,
    preauthenticated: bool,
) -> Result<String, String> {
    let c = compile_cached(ctx, &ctx.root.join(view))
        .map_err(|d| format!("{view} does not compile: {}", d.join(", ")))?;
    if !c.server.load {
        return Ok("{}".into());
    }
    let url = server_module(ctx, view, 0)?;
    let mut sc = sidecar_slot(ctx)?;
    if sc.is_none() {
        *sc = Some(
            crate::load::Sidecar::spawn(&ctx.root, &ctx.env_vars()).map_err(|e| e.to_string())?,
        );
    }
    let lctx = load_ctx_json(req, params);
    // t() and locale() inside load() read the page's language and dictionaries, as in production (§37)
    let i18n = brain_pack::read_i18n(&ctx.root);
    let language = match (split_locale(&i18n, &req.path).0, &i18n) {
        (Some(loc), Some((def, _))) => format!(",\"$locale\":{},\"$dict\":{{{}:{},{}:{}}}", json_str(&loc), json_str(def), dict_json(&ctx.root, def), json_str(&loc), dict_json(&ctx.root, &loc)),
        _ => String::new(),
    };
    let lctx = format!("{},\"user\":{user}{language}}}", &lctx[..lctx.len() - 1]);
    let r = if let Some(server)=runtime_server(ctx){sc.as_mut().unwrap().call_context(&url,"load",&lctx,&server,&ctx.secret(),preauthenticated)}else if ctx.backend_configured(){Err("BNATIVE_AUTH_UNAVAILABLE: Native authentication bridge is unavailable in this runtime".into())}else{sc.as_mut().unwrap().call(&url,"load",&lctx)};
    if r.as_ref()
        .is_err_and(|e| e.contains("load() node process exited"))
    {
        *sc = None; // relaunched on the next request
    }
    r
}

/// Dev document from app.brain: its markup without `<script>`/`<style>` (the `<style>` goes into the head as the
/// layout's stylesheet), tokens.cssbrain linked and `<slot />` replaced by the route's view, wrapped in its
/// `_layout.brain` files and given the props its `load` returns.
/// `src` without its `<meta <name> …>` tags.
fn strip_meta(src: &str, name: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    let open = format!("<meta {name}");
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        // `<meta http>` or `<meta http base=…>`, not `<meta httpx…>`
        if !after.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            out.push_str(&rest[..i + open.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..i]);
        rest = after.find('>').map_or("", |j| &after[j + 1..]);
    }
    out.push_str(rest);
    out
}

fn app_page(ctx: &Ctx, req: &http::Request) -> (u16, String, Option<String>) {
    let root = &ctx.root;
    let raw = std::fs::read_to_string(root.join("app.brain")).unwrap_or_default();
    // `<meta http>` (INTERNALS §40) is the app's HTTP policy, not markup: it becomes `http({…})`, run before any view
    let policy = brain_parse::parse(&raw, Path::new("app.brain"))
        .ok()
        .and_then(|c| brain_check::http::policy_js(&c));
    let (src, _script) = take_block(&raw, "script");
    let src = strip_meta(&brain_pack::strip_meta_seo(&src), "http");
    let (src, style) = take_block(&src, "style");
    let mut head_extra = String::new();
    if root.join("tokens.cssbrain").is_file() {
        head_extra.push_str("<link rel=\"stylesheet\" href=\"/tokens.cssbrain\">");
    }
    // app.brain's own styles go in the `view` layer, as in the build: unlayered CSS would beat every view's
    // `@layer view` + `@scope` rule (SEO: `nav {display:flex}` overrode a layout's `.side {display:grid}`)
    if !style.trim().is_empty() {
        head_extra.push_str(&format!("<style>@layer view{{{}}}</style>", style.trim()));
    }
    if let Some(js) = &policy {
        head_extra.push_str(&format!(
            "<script type=\"module\">import{{http}}from'/@brain/brain-data.js';{}</script>",
            js.trim()
        ));
    }
    // api/_http.jsbrain (the app's http({ headers, … }) setup) runs next, before any view — the same order as the build;
    // its own module script, so the policy above has already run when it is evaluated
    if root.join("api/_http.jsbrain").is_file() {
        head_extra.push_str(&format!(
            "<script type=\"module\">import'{}/api/_http.jsbrain'</script>",
            ctx.pre
        ));
    }
    // the view's seo(props)/head(props) (INTERNALS §32), filled when its load ran
    let mut view_seo: Option<(Vec<(String, String)>, String)> = None;
    // languages (§37): `/es/about` is `/about` in `es`; the default locale has no prefix
    let i18n = brain_pack::read_i18n(root);
    let (locale, route_path) = split_locale(&i18n, &req.path);
    if let (Some(loc), Some((def, _))) = (&locale, &i18n) {
        // this locale's texts and the default's (the fallback of t()), before any view mounts
        let mut js = format!("add({},{});", json_str(def), dict_json(root, def));
        if loc != def {
            js.push_str(&format!("add({},{});", json_str(loc), dict_json(root, loc)));
        }
        head_extra.push_str(&format!(
            "<script type=\"module\">import{{add}}from'/@brain/brain-i18n.js';{js}</script>"
        ));
        if loc != def {
            let t = brain_pack::LocaleLinks::of(root, loc, false);
            head_extra.push_str(&format!(
                "<script type=\"module\">import l from'/@brain/brain-links.js';l({},\"\",{},{},{{}});</script>",
                json_str(loc),
                json_str(&t.prefix),
                json_str(&t.pattern()),
            ));
        }
    }
    let (status, slot) = match crate::route::resolve(root, &route_path) {
        None => (
            404,
            format!(
                "<main><h1>404</h1><p>No view for <code>{}</code>. Fix: create <code>views{}.brain</code> or <code>views/404.brain</code>.</p></main>",
                html_escape(&req.path),
                html_escape(if req.path == "/" {
                    "/home"
                } else {
                    req.path.trim_end_matches('/')
                })
            ),
        ),
        Some(r) => match guard_chain(ctx, &r.view, req, &r.params) {
            Chain::Redirect(status, loc) => return (status, String::new(), Some(loc)),
            // deny → views/403.brain with props.reason (status 403), never a blank page
            Chain::Deny(reason) => (
                403,
                fixed_view(
                    ctx,
                    "views/403.brain",
                    &format!("{{\"reason\":{}}}", json_str(&reason)),
                    "Forbidden",
                    &reason,
                ),
            ),
            Chain::ErrorStatus(status,msg) => (status,fixed_view(ctx,&error_view(&ctx.root,&r.view),&format!("{{\"message\":{},\"rid\":{}}}",json_str(&msg),json_str(&req.id)),"Native authentication unavailable",&msg)),
            Chain::Error(msg) => {
                ctx.log("error", format!("[guard] {}: {msg}", r.view));
                (
                    500,
                    fixed_view(
                        ctx,
                        &error_view(&ctx.root, &r.view),
                        &format!("{{\"message\":{},\"rid\":\"dev\"}}", json_str(&msg)),
                        "Something went wrong",
                        &msg,
                    ),
                )
            }
            Chain::Ok(user) => match run_load(ctx, &r.view, req, &r.params, &user, ctx.backend_configured())
                .inspect_err(|e| ctx.log("error", format!("[load] {}: {e}", r.view)))
            {
                // the folder's _error.brain (props message + rid) when the app has one; the dev notice with the stack else
                Err(error) if error.starts_with("BNATIVE_AUTH_UNAVAILABLE:")=>(503,fixed_view(ctx,&error_view(&ctx.root,&r.view),&format!("{{\"message\":\"Native authentication is temporarily unavailable\",\"rid\":{}}}",json_str(&req.id)),"Native authentication unavailable","Native authentication is temporarily unavailable")),
                Err(error) if error.starts_with("BNATIVE_STATUS:") => {
                    let status = error.split(':').nth(1).and_then(|n| n.parse().ok()).unwrap_or(500);
                    (status, fixed_view(ctx, &error_view(&ctx.root, &r.view), &format!("{{\"message\":\"The request could not complete\",\"rid\":{}}}", json_str(&req.id)), "The request could not complete", "The request could not complete"))
                }
                Err(e) if ctx.root.join(error_view(&ctx.root, &r.view)).is_file() => (
                    500,
                    fixed_view(
                        ctx,
                        &error_view(&ctx.root, &r.view),
                        &format!(
                            "{{\"message\":{},\"rid\":\"dev\"}}",
                            json_str(e.lines().next().unwrap_or(""))
                        ),
                        "",
                        "",
                    ),
                ),
                Err(e) => (
                    500,
                    format!(
                        "<pre role=\"alert\" style=\"white-space:pre-wrap;padding:16px\">load() of {} failed:\n{}</pre>",
                        html_escape(&r.view),
                        html_escape(&e)
                    ),
                ),
                // `throw redirect(url, status)` from load: the same Location answer server.mjs gives in production
                Ok(props) if props.starts_with("{\"redirect\":") => {
                    let v: serde_json::Value = serde_json::from_str(&props).unwrap_or_default();
                    let status = v["status"].as_u64().filter(|s| (300..400).contains(s)).unwrap_or(303) as u16;
                    return (status, String::new(), Some(v["redirect"].as_str().unwrap_or("/").to_string()));
                }
                // `throw notFound()` / `return notFound()` from load (F11): views/404.brain with status 404, as in production
                // `gone()` (§33): 410 with views/410.brain when the app has one, else the 404 view
                Ok(props) if props.starts_with("{\"notFound\":true,\"gone\":true") => (
                    410,
                    fixed_view(ctx, if ctx.root.join("views/410.brain").is_file() { "views/410.brain" } else { "views/404.brain" }, "{}", "Gone", &req.path),
                ),
                Ok(props) if props.starts_with("{\"notFound\":true") => (
                    404,
                    fixed_view(ctx, "views/404.brain", "{}", "Not found", &req.path),
                ),
                Ok(props) => {
                    view_seo = view_meta(ctx, &r.view, &props);
                    // route().params on the client (RUNTIME #222): the same meta server.mjs writes on SSR pages with params
                    if !r.params.is_empty() {
                        let kv: Vec<String> = r
                            .params
                            .iter()
                            .map(|(k, v)| format!("{}:{}", json_str(k), json_str(v)))
                            .collect();
                        head_extra.push_str(&format!(
                            "<meta name=\"brain-params\" content='{{{}}}'>",
                            kv.join(",").replace('&', "&amp;").replace('\'', "&#39;")
                        ));
                    }
                    // Route data + the dev client's `route()`: it mounts the view inside its layouts and, on same-origin
                    // link clicks, fetches the next page and mounts its route without reloading (module state survives).
                    let layouts: Vec<String> = r
                        .layouts
                        .iter()
                        .map(|l| json_str(&format!("/{l}")))
                        .collect();
                    let data = format!(
                        "{{\"v\":{},\"l\":[{}],\"p\":{}}}",
                        json_str(&format!("/{}", r.view)),
                        layouts.join(","),
                        props
                    );
                    let data = brain_lex::json::in_html(&data);
                    (
                        if r.not_found { 404 } else { brain_pack::reserved_view(&r.view).map_or(200, |(status, _)| status) },
                        format!(
                            "<div id=\"brain-view\"></div><script type=\"application/json\" id=\"brain-route\">{data}</script><script type=\"module\">import{{route}}from'/@brain/client.js';route()</script>"
                        ),
                    )
                }
            },
        },
    };
    let mut body = src.replace("<slot />", &slot).replace("<slot/>", &slot);
    // dev mirrors production: the SEO tags of seo(props) and the view's <head> block, deduplicated over app.brain's
    // the title comes from the view's evaluated seo(props) (the compiler already applied app.brain's titleTemplate)
    let mut title: Option<String> = None;
    // an error page from fixed_view has no load(): its own <meta seo>, else the title production gives it
    if view_seo.is_none() && status >= 400 {
        if let Some(view) = fixed_view_of(&slot) {
            view_seo = view_meta(ctx, &view, "{}").filter(|(seo, _)| seo.iter().any(|(k, _)| k == "title"));
            if view_seo.is_none() {
                title = brain_pack::reserved_view(&view).map(|(_, t)| titled(&ctx.root, t));
                head_extra.push_str("<meta name=\"robots\" content=\"noindex\">");
            }
        }
    }
    let compiler_branding=brain_seo::Config::load(&ctx.root).map(|cfg|cfg.compiler_branding).unwrap_or(true);
    if let Some((seo, head)) = &view_seo {
        let (t, tags) = brain_pack::prerender::seo_head_with_branding(seo, None, &req.path, "",compiler_branding);
        title = t.or(title);
        head_extra = format!("{tags}{head_extra}{head}");
    }
    if compiler_branding && !head_extra.contains(brain_lex::branding::HTML) && !body.contains(brain_lex::branding::HTML) {
        head_extra.push_str(brain_lex::branding::HTML);
    }
    if let Some(title) = title {
        let t = html_escape(&title);
        body = match (body.find("<title>"), body.find("</title>")) {
            (Some(a), Some(b)) if a < b => {
                format!("{}<title>{t}</title>{}", &body[..a], &body[b + 8..])
            }
            _ => body.replacen("</head>", &format!("<title>{t}</title></head>"), 1),
        };
    }
    let html = match body.find("</head>") {
        Some(i) => format!("{}{head_extra}{}", &body[..i], &body[i..]),
        None => format!("{head_extra}{body}"),
    };
    // <html lang dir data-default-lang>: t() and fmt read the page's language from it, as in the build
    let html = match (&locale, &i18n, html.find("<html")) {
        (Some(loc), Some((def, _)), Some(i)) => {
            let end = i + html[i..].find('>').unwrap_or(5);
            let tag: String = html[i..end]
                .split_whitespace()
                .filter(|a| {
                    !a.starts_with("lang=")
                        && !a.starts_with("dir=")
                        && !a.starts_with("data-default-lang=")
                })
                .collect::<Vec<_>>()
                .join(" ");
            let dir = if brain_check::i18n::is_rtl(loc) {
                "rtl"
            } else {
                "ltr"
            };
            format!(
                "{}{tag} lang=\"{loc}\" dir=\"{dir}\" data-default-lang=\"{def}\"{}",
                &html[..i],
                &html[end..]
            )
        }
        _ => html,
    };
    (
        status,
        brain_pack::prerender::dedupe_head(&format!("<!doctype html>\n{}", html.trim_start())),
        None,
    )
}

enum Chain {
    /// passed; `ctx.user` as JSON
    Ok(String),
    Redirect(u16, String),
    Deny(String),
    Error(String),
    ErrorStatus(u16,String),
}

/// `_guard.jsbrain` and `_middleware.jsbrain` of every folder from views/ down to the view's (outer first).
fn chain_files(root: &Path, view: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut dir = PathBuf::new();
    for seg in Path::new(view)
        .parent()
        .map(|p| p.components().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        dir.push(seg);
        let f = dir.join(name);
        if root.join(&f).is_file() {
            out.push(f.to_string_lossy().replace('\\', "/"));
        }
    }
    out
}

/// Runs the route's guards and middleware like dist/server.mjs does (runChain in the sidecar), with ctx.user from
/// the signed session cookie (BRAIN_SECRET of .brain_env [development]).
fn guard_chain(ctx: &Ctx, view: &str, req: &http::Request, params: &[(String, String)]) -> Chain {
    let (g, m) = (
        chain_files(&ctx.root, view, "_guard.jsbrain"),
        chain_files(&ctx.root, view, "_middleware.jsbrain"),
    );
    let has_cookie = req
        .header("cookie")
        .is_some_and(|c| c.contains("__Host-session"));
    if g.is_empty() && m.is_empty() && !has_cookie && !ctx.backend_configured() {
        return Chain::Ok("null".into());
    }
    let Some(server) = runtime_server(ctx) else {
        return if ctx.backend_configured(){Chain::ErrorStatus(503,"Native authentication bridge is unavailable in this runtime".into())}else{Chain::Ok("null".into())};
    };
    let urls = |v: &[String]| -> Result<Vec<String>, String> {
        v.iter().map(|f| server_module(ctx, f, 0)).collect()
    };
    let (gu, mu) = match (urls(&g), urls(&m)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Chain::Error(e),
    };
    let secret = ctx.secret();
    let mut sc = match sidecar_slot(ctx) {
        Ok(slot) => slot,
        Err(error) => return Chain::Error(error),
    };
    if sc.is_none() {
        match crate::load::Sidecar::spawn(&ctx.root, &ctx.env_vars()) {
            Ok(x) => *sc = Some(x),
            Err(e) => return Chain::Error(e.to_string()),
        }
    }
    let f =
        match sc
            .as_mut()
            .unwrap()
            .chain(&server, &gu, &mu, &load_ctx_json(req, params), &secret)
        {
            Ok(f) => f,
            Err(e) => {
                *sc = None;
                return Chain::Error(e);
            }
        };
    let get = |k: &str| {
        f.iter()
            .find(|(a, _)| a == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    match get("kind").as_str() {
        "redirect" => Chain::Redirect(get("status").parse().unwrap_or(303), get("location")),
        "deny" => Chain::Deny(get("reason")),
        "error" => Chain::ErrorStatus(get("status").parse::<u16>().ok().filter(|s|(400..=599).contains(s)).unwrap_or(500),get("reason")),
        _ => Chain::Ok(
            Some(get("user"))
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| "null".into()),
        ),
    }
}

/// The nearest `_error.brain` above the view (the folder's error boundary), else views/_error.brain.
/// GET `path` on the app dev server `base` (`http://127.0.0.1:5180`): (status, body). A dev server that is gone answers
/// 502 with a JSON error, so the debugger can say so.
fn proxy_get(base: &str, path: &str) -> (u16, String) {
    let host = base
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("");
    let pre = base
        .trim_start_matches("http://")
        .split_once('/')
        .map(|(_, p)| format!("/{}", p.trim_end_matches('/')))
        .unwrap_or_default();
    let fail = |e: String| {
        (
            502,
            format!(
                "{{\"error\":{}}}",
                json_str(&format!("the app dev server at {base} did not answer: {e}"))
            ),
        )
    };
    let Ok(mut c) = TcpStream::connect(host) else {
        return fail("connection refused".into());
    };
    let _ = c.set_read_timeout(Some(Duration::from_secs(20)));
    if let Err(e) = c.write_all(
        format!("GET {pre}{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    ) {
        return fail(e.to_string());
    }
    let mut raw = Vec::new();
    if let Err(e) = std::io::Read::read_to_end(&mut c, &mut raw) {
        return fail(e.to_string());
    }
    let raw = String::from_utf8_lossy(&raw);
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    (
        head.split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(502),
        body.to_string(),
    )
}

/// The nearest `_loading.brain` from the view's folder up to views/, if the app has one.
fn loading_view(root: &Path, view: &str) -> Option<String> {
    let mut dir = Path::new(view)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    loop {
        let f = dir.join("_loading.brain");
        if root.join(&f).is_file() {
            return Some(f.to_string_lossy().replace('\\', "/"));
        }
        if !dir.pop() || dir.as_os_str().is_empty() {
            return None;
        }
    }
}

fn error_view(root: &Path, view: &str) -> String {
    let mut dir = Path::new(view)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    loop {
        let f = dir.join("_error.brain");
        if root.join(&f).is_file() {
            return f.to_string_lossy().replace('\\', "/");
        }
        if !dir.pop() {
            return "views/_error.brain".into();
        }
    }
}

/// A specific view (403, _error) mounted with `props`; a plain notice when the app has no such view.
/// The view a `fixed_view` answer mounts, read back from its route data.
fn fixed_view_of(slot: &str) -> Option<String> {
    let (_, rest) = slot.split_once("\"v\":\"/")?;
    Some(rest.split('"').next()?.to_string())
}

/// `title` through app.brain's `titleTemplate`, as the build writes it.
fn titled(root: &Path, title: &str) -> String {
    let src = std::fs::read_to_string(root.join("app.brain")).unwrap_or_default();
    let template = brain_parse::parse(&src, Path::new("app.brain")).ok().and_then(|c| c.meta.iter().find(|m| m.name == "titleTemplate").and_then(|m| m.value.clone()));
    template.map_or(title.to_string(), |t| t.replace("%s", title))
}

fn fixed_view(ctx: &Ctx, view: &str, props: &str, title: &str, detail: &str) -> String {
    if !ctx.root.join(view).is_file() {
        return format!(
            "<main role=\"alert\"><h1>{}</h1><p>{}</p></main>",
            html_escape(title),
            html_escape(detail)
        );
    }
    let data = format!(
        "{{\"v\":{},\"l\":[],\"p\":{props}}}",
        json_str(&format!("/{view}"))
    );
    let data = brain_lex::json::in_html(&data);
    format!(
        "<div id=\"brain-view\"></div><script type=\"application/json\" id=\"brain-route\">{data}</script><script type=\"module\">import{{route}}from'/@brain/client.js';route()</script>"
    )
}

/// B0741 in development: the whole reason, in the answer and in the log, because here the reader is the author. Dev runs
/// on loopback and has no allowlist to reveal; the production server answers only the code.
fn origin_refusal(ctx: &Ctx, req: &http::Request, message: &str) -> String {
    let origin = req.header("origin");
    let host = req.header("host").unwrap_or("");
    let fix = match origin {
        Some(origin) => format!("Origin {origin} does not match the host this server sees ({host}). Call it from the page served by this dev server; behind a proxy forward the host (nginx: proxy_set_header Host $host;). There is no allowlist to widen. Run brain explain B0741"),
        None => "The request has no Origin and no Sec-Fetch-Site, so its site cannot be proven. Send it from a page of this site; a script sets Origin to the site origin. Run brain explain B0741".to_string(),
    };
    let seen = origin.map_or("null".to_string(), |o| json_str(&o.chars().take(256).collect::<String>()));
    let body = format!("{{\"error\":{},\"brainCode\":\"B0741\",\"seenOrigin\":{seen},\"host\":{},\"fix\":{}}}", json_str(message), json_str(host), json_str(&fix));
    ctx.log("warn", body.clone());
    body
}

/// `POST /_r/<hash>/<fn>` in dev, with the same contract as dist/server.mjs (INTERNALS §11.1): same-origin POST with
/// `X-Brain-RPC: 1`, a JSON array of arguments, `fn(ctx, ...args)` → 200 JSON · thrown `{errors}` → 422
/// `{error, errors}` (form() marks the fields) · anything else → 500 `{error}` (the stack goes to the log).
fn rpc(ctx: &Ctx, req: &http::Request, id: &str) -> (u16, String, Vec<(String, String)>) {
    let fail = |status: u16, msg: &str| {
        (
            status,
            format!("{{\"error\":{}}}", json_str(msg)),
            Vec::new(),
        )
    };
    if req.method != "POST" {
        return fail(405, "Use POST");
    }
    if req.header("x-brain-rpc") != Some("1") {
        return fail(403, "Missing X-Brain-RPC header");
    }
    if let (Some(origin), Some(host)) = (req.header("origin"), req.header("host")) {
        if origin.split("://").nth(1) != Some(host) {
            let body = origin_refusal(ctx, req, "Cross-origin request");
            return (403, body, Vec::new());
        }
    }
    let Some((hash, func)) = id.split_once('/') else {
        return fail(404, "Unknown function");
    };
    let ident = |x: &str| {
        !x.is_empty()
            && x.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
            && !x.starts_with(|c: char| c.is_ascii_digit())
    };
    // the hash is base36 and may start with a digit; the function name is a JS identifier
    if hash.is_empty() || !hash.chars().all(|c| c.is_ascii_alphanumeric()) || !ident(func) {
        return fail(404, "Unknown function");
    }
    let limits = ctx.server_limits();
    let raw = String::from_utf8_lossy(&req.body).to_string();
    if raw.len() > limits.max_body_bytes() {
        return fail(413, &format!("Body larger than {} KB; raise brain.json server.maxBodyKB", limits.max_body_kb));
    }
    // Only compiler-owned RPC capabilities may select the module.
    let mut files = Vec::new();
    if !rpc_candidates(&ctx.root, &mut files) {
        return fail(503, "RPC module inventory is incomplete; reduce the project tree or fix unreadable directories");
    }
    let mut owner = None;
    for file in files {
        let Ok(compiled) = compile_cached(ctx, &file) else { continue };
        if compiled.server.rpc.iter().any(|rpc| rpc == id) {
            let Ok(relative) = file.strip_prefix(&ctx.root) else { continue };
            if owner.replace(relative.to_string_lossy().replace('\\', "/")).is_some() {
                return fail(404, "Ambiguous function; resolve the component namespace collision");
            }
        }
    }
    let Some(rel) = owner else {
        return fail(404, "Unknown function");
    };
    let (url, server) = match (server_module(ctx, &rel, 0), runtime_server(ctx)) {
        (Ok(u), Some(s)) => (u, s),
        (Err(e), _) => {
            ctx.log("error", format!("[rpc] {rel}: {e}"));
            return fail(500, "Internal error");
        }
        (_, None) => {
            return fail(
                500,
                "runtime/server/server.mjs is not embedded in this brain build",
            );
        }
    };
    let mut sc = match sidecar_slot(ctx) {
        Ok(slot) => slot,
        Err(error) => return fail(503, &error),
    };
    if sc.is_none() {
        match crate::load::Sidecar::spawn(&ctx.root, &ctx.env_vars()) {
            Ok(x) => *sc = Some(x),
            Err(e) => return fail(500, &e.to_string()),
        }
    }
    let ip = "127.0.0.1"; // Dev loopback peer; forwarded client identity is never trusted.
    sc.as_mut().unwrap().limits(limits.timeout_ms, limits.rpc_per_minute, limits.max_body_bytes());
    match sc.as_mut().unwrap().rpc(
        &url,
        func,
        &load_ctx_json(req, &[]),
        &raw,
        &server,
        &format!("{ip} {hash}/{func}"),
        &ctx.secret(),
    ) {
        Ok(r) => r,
        Err(e) => {
            ctx.log("error", format!("[rpc] {hash}/{func}: {e}"));
            *sc = None;
            fail(500, "Internal error; see the brain run log")
        }
    }
}

/// Languages (§37): `/es/about` is `/about` in `es`; the default locale has no prefix. (locale, path to route).
fn split_locale(i18n: &Option<(String, Vec<String>)>, path: &str) -> (Option<String>, String) {
    let Some((def, ls)) = i18n else {
        return (None, path.to_string());
    };
    let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
    match ls.iter().find(|l| *l == first && *l != def) {
        Some(l) => (
            Some(l.clone()),
            format!("/{}", path.trim_start_matches('/')[first.len()..].trim_start_matches('/')),
        ),
        None => (Some(def.clone()), path.to_string()),
    }
}

/// A locale's dictionary as a JSON object (i18n/<locale>.json, values kept as written).
fn dict_json(root: &Path, locale: &str) -> String {
    let body = brain_pack::read_dict(root, locale)
        .iter()
        .map(|(k, v)| format!("{}:{v}", json_str(k)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

/// `POST <page>` with `application/x-www-form-urlencoded`: the view's `server:` function named by `_brain` runs
/// through server.mjs's callForm (schema coercion and rules shared with form()). 303 after success (PRG), 422
/// re-renders the page with the values and errors in `#brain-forms`, 400/429 answer JSON.
fn form_post(s: &mut TcpStream, ctx: &Ctx, req: &http::Request) -> io::Result<()> {
    let json = |s: &mut TcpStream, status: u16, msg: &str| {
        http::respond(
            s,
            status,
            "application/json",
            format!("{{\"error\":{}}}", json_str(msg)).as_bytes(),
            false,
        )
    };
    if !req
        .header("content-type")
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded")
    {
        return json(s, 415, "Forms post application/x-www-form-urlencoded");
    }
    // strict same origin: a form without Origin and without Sec-Fetch-Site is refused
    let same = match (req.header("origin"), req.header("sec-fetch-site")) {
        (Some(o), _) => o.split("://").nth(1) == req.header("host"),
        (None, Some(site)) => site == "same-origin",
        (None, None) => false,
    };
    if !same {
        let body = origin_refusal(ctx, req, "Cross-origin form");
        return http::respond(s, 403, "application/json", body.as_bytes(), false);
    }
    let limits = ctx.server_limits();
    if req.body.len() > limits.max_body_bytes() {
        return json(s, 413, &format!("Body larger than {} KB; raise brain.json server.maxBodyKB", limits.max_body_kb));
    }
    // `/es/contact` is the contact page in `es`: its schema messages come from es.json (§37), like the browser
    let i18n = brain_pack::read_i18n(&ctx.root);
    let (locale, route_path) = split_locale(&i18n, &req.path);
    let Some(r) = crate::route::resolve(&ctx.root, &route_path) else {
        return json(s, 404, "No page here");
    };
    let dict = match (&locale, &i18n) {
        (Some(loc), Some((def, _))) if loc != def => format!(
            "{{{}:{},{}:{}}}",
            json_str(def),
            dict_json(&ctx.root, def),
            json_str(loc),
            dict_json(&ctx.root, loc)
        ),
        (Some(loc), _) => format!("{{{}:{}}}", json_str(loc), dict_json(&ctx.root, loc)),
        _ => "null".to_string(),
    };
    let rules = runtime_file(ctx, "brain-schema.js");
    let (module, server, rules) = match (server_module(ctx, &r.view, 0), runtime_server(ctx), rules)
    {
        (Ok(m), Some(sv), Some(ru)) => (m, sv, ru),
        _ => {
            return json(
                s,
                500,
                "the form handler is not available (server code or runtime missing)",
            );
        }
    };
    let raw = String::from_utf8_lossy(&req.body).to_string();
    let ip = "127.0.0.1".to_string(); // Never trust client-supplied forwarded identity.
    let res = {
        let mut sc = match sidecar_slot(ctx) {
            Ok(slot) => slot,
            Err(error) => return json(s, 503, &error),
        };
        if sc.is_none() {
            match crate::load::Sidecar::spawn(&ctx.root, &ctx.env_vars()) {
                Ok(x) => *sc = Some(x),
                Err(e) => return json(s, 500, &e.to_string()),
            }
        }
        let params: Vec<(String, String)> = r.params.clone();
        sc.as_mut().unwrap().limits(limits.timeout_ms, limits.rpc_per_minute, limits.max_body_bytes());
        let out = sc.as_mut().unwrap().form(
            &module,
            &load_ctx_json(req, &params),
            &raw,
            &server,
            &rules,
            &format!("{ip} form {}", req.path),
            &ctx.secret(),
            locale.as_deref().unwrap_or(""),
            &dict,
        );
        if out.is_err() {
            *sc = None;
        }
        out
    };
    match res {
        Ok((303, location, _, _, hs)) => {
            let location = ctx.under_base(&location);
            let mut extra: Vec<(&str, &str)> = vec![("Location", location.as_str())];
            extra.extend(hs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            http::respond_with(s, 303, "text/plain", b"", false, &extra)
        }
        Ok((422, _, state, _, _)) => {
            // re-render the page as a GET, then hand form() its state (values + errors)
            let get = http::Request {
                upgrade: Default::default(),
                method: "GET".into(),
                path: req.path.clone(),
                query: req.query.clone(),
                headers: req.headers.clone(),
                body: vec![],
                id:req.id.clone(),
                _body_budget:None,
            };
            let (_, html, _) = app_page(ctx, &get);
            let tag = format!(
                "<script type=\"application/json\" id=\"brain-forms\">{}</script>",
                brain_lex::json::in_html(&state)
            );
            let html = match html.rfind("</body>") {
                Some(i) => format!("{}{tag}{}", &html[..i], &html[i..]),
                None => format!("{html}{tag}"),
            };
            let html = ctx.page_out(&html);
            http::respond(s, 422, http::mime("x.html"), html.as_bytes(), false)
        }
        Ok((status, _, _, body, _)) => {
            http::respond(s, status, "application/json", body.as_bytes(), false)
        }
        Err(e) => {
            ctx.log("error", format!("[form] {}: {e}", req.path));
            json(s, 500, "Internal error; see the brain run log")
        }
    }
}

/// `brain:content` (INTERNALS §33): the collections of content/ as a module (brain-seo's `module_js`), regenerated on
/// every load so an edited .md shows on the next request; drafts are included in dev. Its URL carries the file's
/// mtime, so the views that import it are reloaded only when the content changed.
fn content_module(ctx: &Ctx) -> Result<String, String> {
    let per_page = brain_seo::Config::load(&ctx.root)
        .map(|c| c.per_page)
        .unwrap_or(10);
    let (cols, _) = brain_seo::content::load(&ctx.root, true);
    let src = brain_seo::content::module_js(&cols, per_page);
    let out = ctx.root.join(".brain/server/_rt/content.mjs");
    if std::fs::read_to_string(&out).ok().as_deref() != Some(src.as_str()) {
        std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&out, &src).map_err(|e| e.to_string())?;
    }
    Ok(crate::load::file_url(&out))
}

/// A runtime module written under .brain/server/_rt/ for the sidecar; its `file://` URL.
fn runtime_file(ctx: &Ctx, name: &str) -> Option<String> {
    let (_, src) = ctx.runtime.iter().find(|(n, _)| n == name)?;
    let out = ctx.root.join(".brain/server/_rt").join(name);
    if std::fs::read_to_string(&out).ok().as_deref() != Some(src.as_str()) {
        std::fs::create_dir_all(out.parent()?).ok()?;
        std::fs::write(&out, src).ok()?;
    }
    Some(crate::load::file_url(&out))
}

/// runtime/server/server.mjs written once under .brain/server/_rt/ (dev imports its callRpc/callForm, so dev and
/// production share the request handling); its `file://` URL.
fn runtime_server(ctx: &Ctx) -> Option<String> {
    let (_, src) = ctx.runtime.iter().find(|(n, _)| n == "server/server.mjs")?;
    for file in ["native-sockets.mjs", "llm.mjs"] {
        if !src.contains(&format!("from './{file}'")) {
            continue;
        }
        let (_, helper) = ctx.runtime.iter().find(|(n, _)| *n == format!("server/{file}"))?;
        let helper_out = ctx.root.join(".brain/server/_rt").join(file);
        if std::fs::read_to_string(&helper_out).ok().as_deref() != Some(helper.as_str()) {
            std::fs::create_dir_all(helper_out.parent()?).ok()?;
            std::fs::write(&helper_out, helper).ok()?;
        }
    }
    let out = ctx.root.join(".brain/server/_rt/server.mjs");
    if std::fs::read_to_string(&out).ok().as_deref() != Some(src.as_str()) {
        std::fs::create_dir_all(out.parent()?).ok()?;
        std::fs::write(&out, src).ok()?;
    }
    Some(crate::load::file_url(&out))
}

/// .brain/.jsbrain files that may hold `server:` functions (the app tree minus build output and dot folders).
fn rpc_candidates(dir: &Path, out: &mut Vec<PathBuf>) -> bool {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, entries: &mut usize, depth: usize) -> Option<()> {
        if depth > 32 { return None; }
        for entry in std::fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            *entries += 1;
            if *entries > 20000 { return None; }
            let name = entry.file_name(); let name = name.to_string_lossy();
            if name.starts_with('.') || matches!(name.as_ref(), "dist" | "node_modules" | "public" | "target") { continue; }
            let kind = entry.file_type().ok()?;
            if kind.is_dir() { walk(&entry.path(), out, entries, depth + 1)?; }
            else if kind.is_file() && entry.path().extension().is_some_and(|x| x == "brain" || x == "jsbrain") && !name.ends_with(".test.brain") {
                if out.len() >= 4096 { return None; }
                out.push(entry.path());
            }
        }
        Some(())
    }
    walk(dir, out, &mut 0, 0).is_some()
}

/// `seo(props)` as pairs and `head(props)` HTML of a view, evaluated in the load sidecar; None if the view has
/// neither or they fail (the page still renders; the error goes to the log).
fn view_meta(ctx: &Ctx, view: &str, props: &str) -> Option<(Vec<(String, String)>, String)> {
    let c = compile_cached(ctx, &ctx.root.join(view)).ok()?;
    let (has_seo, has_head) = (
        c.server_js.contains("export const seo ="),
        c.server_js.contains("export const head ="),
    );
    if !has_seo && !has_head {
        return None;
    }
    let url = server_module(ctx, view, 0).ok()?;
    let mut sc = sidecar_slot(ctx).ok()?;
    if sc.is_none() {
        *sc = Some(crate::load::Sidecar::spawn(&ctx.root, &ctx.env_vars()).ok()?);
    }
    let s = sc.as_mut()?;
    let mut call = |f: &str| {
        s.call(&url, f, props)
            .inspect_err(|e| ctx.log("error", format!("[{f}] {view}: {e}")))
            .ok()
    };
    let seo = if has_seo {
        call("seo")
            .and_then(|j| crate::json_flat(&j))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let head = if has_head {
        call("head")
            .map(|j| crate::load::unquote(&j))
            .unwrap_or_default()
    } else {
        String::new()
    };
    Some((seo, head))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Removes the first `<tag>…</tag>` block that belongs to BrainScript and returns (rest, body). A `<script src=…>`
/// (third-party script with SRI in app.brain's head, B0123) is markup, not the app's script: it is skipped and kept.
fn take_block(src: &str, tag: &str) -> (String, String) {
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut from = 0;
    while let Some(a) = src[from..].find(&open).map(|a| from + a) {
        let after = src[a + open.len()..].chars().next();
        let Some(gt) = src[a..].find('>').map(|i| a + i) else {
            break;
        };
        let Some(b) = src[gt..].find(&close).map(|b| gt + b) else {
            break;
        };
        let is_tag = matches!(after, Some(c) if c == '>' || c == '/' || c.is_whitespace());
        let external = src[a..gt].contains(" src=");
        if is_tag && !external {
            return (
                format!("{}{}", &src[..a], &src[b + close.len()..]),
                src[gt + 1..b].to_string(),
            );
        }
        from = b + close.len();
    }
    (src.to_string(), String::new())
}

fn script_body(src: &str) -> &str {
    let Some(a) = src.find("<script") else {
        return src;
    };
    let Some(open_end) = src[a..].find('>').map(|i| a + i + 1) else {
        return src;
    };
    let end = src[open_end..]
        .find("</script>")
        .map_or(src.len(), |i| open_end + i);
    &src[open_end..end]
}

fn prepend_head(html: &str, extra: &str) -> String {
    if extra.is_empty() {
        return html.to_string();
    }
    match html
        .find("<head")
        .and_then(|i| html[i..].find('>').map(|j| i + j + 1))
    {
        Some(at) => format!("{}{extra}{}", &html[..at], &html[at..]),
        None => format!("{extra}{html}"),
    }
}

fn inject_client(html: &str) -> String {
    match html.find("</head>") {
        Some(i) => format!("{}{CLIENT_TAG}{}", &html[..i], &html[i..]),
        None => format!("{CLIENT_TAG}{html}"),
    }
}

impl Ctx {
    /// Every diagnostic of `path` for the overlay, each with `snippet: {line, text}` (3 lines around it); None when
    /// the CLI gave no diagnose function.
    fn file_diags(&self, path: &Path) -> Option<Vec<String>> {
        let f = self.diagnose.as_ref()?;
        let src = std::fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<&str> = src.lines().collect();
        let out: Vec<String> = f(path)
            .into_iter()
            .map(|d| {
                let Ok(j) = brain_lex::json::Json::parse(&d) else {
                    return d;
                };
                let line = match j.get("line") {
                    Some(brain_lex::json::Json::Num(n)) => *n as usize,
                    _ => 0,
                };
                if line == 0 || lines.is_empty() {
                    return d;
                }
                let from = line.saturating_sub(3).max(1);
                let to = (line + 3).min(lines.len());
                let text: Vec<String> = lines[from - 1..to].iter().map(|l| json_str(l)).collect();
                match d.rfind('}') {
                    Some(i) => format!(
                        "{},\"snippet\":{{\"line\":{from},\"text\":[{}]}}}}",
                        &d[..i],
                        text.join(",")
                    ),
                    None => d,
                }
            })
            .collect();
        Some(out)
    }

    /// `{"now", "batches":[…], "leaks":[…]}`: the inspector batches received after `since` (ms) and the leak records
    /// (`k:"leak"`, B0601/B0602) of the whole buffer, deduplicated by code + component + message.
    fn metrics_json(&self, since: u64) -> String {
        let m = self.metrics.lock().unwrap();
        let batches: Vec<&str> = m
            .iter()
            .filter(|(at, _)| *at > since)
            .map(|(_, t)| t.as_str())
            .collect();
        let mut leaks: Vec<String> = Vec::new();
        let mut keys: Vec<String> = Vec::new();
        for (_, t) in m.iter() {
            let Ok(j) = brain_lex::json::Json::parse(t) else {
                continue;
            };
            if let Some(brain_lex::json::Json::Arr(d)) = j.get("d") {
                for r in d
                    .iter()
                    .filter(|r| r.get("k").and_then(|k| k.as_str()) == Some("leak"))
                {
                    let s = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let key = format!("{}|{}|{}", s("code"), s("h"), s("msg"));
                    if !keys.contains(&key) {
                        keys.push(key);
                        leaks.push(r.stringify());
                    }
                }
            }
        }
        format!(
            "{{\"now\":{},\"batches\":[{}],\"leaks\":[{}]}}",
            now_ms(),
            batches.join(","),
            leaks.join(",")
        )
    }

    /// The document as the browser gets it: HMR client, head prelude and, under a base, `<meta name="brain-base">`
    /// plus every root-relative link and module URL moved under it.
    fn page_out(&self, html: &str) -> String {
        let html = prepend_head(&inject_client(html), &self.head_prepend);
        // --mock: the client turns every api() into fake data from its schema before the page mounts
        let html = match &self.mock {
            Some(seed) => prepend_head(
                &html,
                &format!(
                    "<meta name=\"brain-mock\" content=\"{}\">",
                    html_escape(seed)
                ),
            ),
            None => html,
        };
        // behavior analytics: with a native backend the dev page loads the backend-served collector (BRAIN_ANALYTICS=off skips it)
        let html = if self.backend_configured() && !self.env_vars().iter().any(|(k, v)| k == "BRAIN_ANALYTICS" && v == "off") {
            crate::prod::with_analytics(&html, &self.pre)
        } else {
            html
        };
        if self.pre.is_empty() {
            return html;
        }
        let meta = format!("<meta name=\"brain-base\" content=\"{}/\">", self.pre);
        let html = prepend_head(&html, &meta)
            .replace("from'/@brain/", &format!("from'{}/@brain/", self.pre));
        brain_pack::prefix_links(&html, &self.pre)
    }

    /// A served module under a base: absolute import specifiers and template links get the prefix.
    fn js_out(&self, js: &str) -> String {
        if self.pre.is_empty() {
            return js.to_string();
        }
        let pre = self.pre.clone();
        let js = brain_pack::rewrite_imports(js, &mut |spec| {
            Ok((spec.starts_with('/')
                && !spec.starts_with("//")
                && !spec.starts_with(&format!("{pre}/")))
            .then(|| format!("{pre}{spec}")))
        })
        .unwrap_or_else(|_| js.to_string());
        brain_pack::prefix_links(&js, &self.pre)
    }

    /// A redirect target from a guard or a form (`/login`) under the base.
    fn under_base(&self, loc: &str) -> String {
        if self.pre.is_empty()
            || !loc.starts_with('/')
            || loc.starts_with("//")
            || loc.starts_with(&format!("{}/", self.pre))
        {
            return loc.to_string();
        }
        format!("{}{loc}", self.pre)
    }
}

fn compile_cached(ctx: &Ctx, file: &Path) -> Result<Arc<Compiled>, Vec<String>> {
    // Serialize reads and compilation so concurrent requests cannot publish stale entries out of order.
    let mut cache = ctx.cache.lock().unwrap();
    let src = match std::fs::read_to_string(file) {
        Ok(src) => src,
        Err(error) => {
            let diagnostics = vec![io_diag(file, &error)];
            compiler_feedback(ctx, file, None, Some(&diagnostics));
            return Err(diagnostics);
        }
    };
    let digest = brain_pack::sha::sha256(src.as_bytes());
    if let Some((cached_digest, compiled)) = cache.get(file)
        && *cached_digest == digest
    {
        return Ok(compiled.clone());
    }
    let source_hash = digest.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let out = match (ctx.compile)(&src, file) {
        Ok(compiled) => {
            compiler_feedback(ctx, file, Some(&source_hash), None);
            Arc::new(compiled)
        }
        Err(diagnostics) => {
            compiler_feedback(ctx, file, Some(&source_hash), Some(&diagnostics));
            return Err(diagnostics);
        }
    };
    let size = out.js.len() + out.css.len() + out.server_js.len();
    let bytes: usize = cache
        .values()
        .map(|(_, c)| c.js.len() + c.css.len() + c.server_js.len())
        .sum();
    if cache.len() >= 1024 || bytes.saturating_add(size) > 128 * 1024 * 1024 {
        cache.clear();
    }
    if size <= 16 * 1024 * 1024 {
        cache.insert(file.to_path_buf(), (digest, out.clone()));
    }
    Ok(out)
}

/// Persist and notify the local agent before returning a browser fallback or broadcasting an overlay.
fn compiler_feedback(
    ctx: &Ctx,
    file: &Path,
    revision: Option<&str>,
    diagnostics: Option<&[String]>,
) {
    match ctx.feedback.record(file,revision,diagnostics){
        Ok(Some(event))=>ctx.emit(&format!("{{\"type\":\"compiler-feedback\",\"event\":{event}}}")),
        Ok(None)=>{},
        Err(_)=>ctx.emit("{\"type\":\"compiler-feedback-gap\",\"code\":\"BFEEDBACK_WRITE\",\"fix\":\"Poll local feedback and refresh brain check; persistent delivery could not be confirmed.\"}"),
    }
}

fn io_diag(file: &Path, e: &io::Error) -> String {
    format!(
        "{{\"code\":\"B0001\",\"file\":{},\"line\":0,\"col\":0,\"msg\":{},\"fix\":\"check that the file exists and is UTF-8\"}}",
        json_str(&file.to_string_lossy()),
        json_str(&e.to_string())
    )
}

fn serve_brain(
    s: &mut TcpStream,
    ctx: &Ctx,
    file: &Path,
    url: &str,
    css: bool,
    head: bool,
) -> io::Result<()> {
    match compile_cached(ctx, file) {
        Ok(c) if css => http::respond(s, 200, http::mime("x.css"), c.css.as_bytes(), head),
        Ok(c) if c.js.trim().is_empty() && !c.server_js.trim().is_empty() => {
            // A `<meta server>` module has no client code and its server code never reaches the browser. Only a
            // browser test imports one directly (components drop such imports): name the reason instead of the
            // engine's "does not provide an export named 'default'", and keep the default binding linkable.
            let file = json_str(url.trim_start_matches('/'));
            let js = format!(concat!(
                "const file={};\n",
                "const error=new Error(file+' is a <meta server> module: it runs only on the server, so browser code and tests cannot import it.\\n",
                "  fix: test the component or view that calls it and answer its server function or endpoint with stub(), e.g. stub(\\'save\\', {{ ok: true }})');\n",
                "error.name='BrainServerOnlyModule';error.file=file;\n",
                "export default undefined;\nthrow error;\n",
            ), file);
            http::respond(s, 200, http::mime("x.js"), js.as_bytes(), head)
        }
        Ok(c) => {
            if let Some(h) =
                c.js.rfind(".h = \"")
                    .and_then(|i| c.js[i + 6..].split('"').next())
            {
                ctx.hashes
                    .lock()
                    .unwrap()
                    .insert(url.to_string(), h.to_string());
            }
            let mut js = ctx.js_out(&c.js);
            if !c.css.is_empty() {
                let u = json_str(&format!("{}{url}", ctx.pre));
                js.push_str(&format!(
                    "\nif(!document.querySelector(`link[data-brain=\"${{{u}}}\"]`)){{const l=document.createElement('link');l.rel='stylesheet';l.dataset.brain={u};l.href={u}+'?css';document.head.append(l)}}\n"
                ));
            }
            http::respond(s, 200, http::mime("x.js"), js.as_bytes(), head)
        }
        Err(_) if css => http::respond(s, 200, http::mime("x.css"), b"", head),
        Err(diags) if brain_parse::file_kind(file) == brain_parse::ast::FileKind::Module => {
            // A value module cannot render a component fallback. Keep its default binding linkable,
            // then reject evaluation with the original diagnostics instead of returning a fake API.
            // Named imports can still fail at linking; durable compiler feedback remains authoritative.
            let js = format!(concat!(
                "const diagnostics=[{}],file={};\n",
                "const first=diagnostics.find(d=>!d.level||d.level==='error')||diagnostics[0]||{{}};\n",
                "const error=new Error(file+' does not compile\\n'+diagnostics.map(d=>d.code+' '+(d.file||file)+':'+d.line+':'+d.col+' '+d.msg+'\\n  fix: '+d.fix).join('\\n'));\n",
                "error.name='BrainCompileError';error.diagnostics=diagnostics;\n",
                "error.code=first.code??null;error.file=first.file??file;error.line=first.line??null;error.column=first.col??null;error.fix=first.fix??null;\n",
                "export default undefined;\nthrow error;\n",
            ), diags.join(","), json_str(url.trim_start_matches('/')));
            http::respond(s, 200, http::mime("x.js"), js.as_bytes(), head)
        }
        Err(diags) => {
            // INTERNALS §42: a component that does not compile renders a box with its diagnostics and their exact
            // fix in its place, so the rest of the page keeps working (and keeps HMR); `brain build` still fails.
            let _ = &diags;
            // with the hash of the last version that compiled (and registered like any dev component), HMR swaps this box
            // into every live instance and later swaps the fixed component back in, so nothing else loses its state
            let hash = ctx.hashes.lock().unwrap().get(url).cloned();
            let tail = match &hash {
                Some(h) => format!(
                    "BrainCompileError.h={};export default dev(BrainCompileError);\n",
                    json_str(h)
                ),
                None => "export default BrainCompileError;\n".into(),
            };
            let js = format!(
                "{}const d=[{}],f={};console.error('brain: '+f+'\\n'+d.map(x=>x.code+' '+x.line+':'+x.col+' '+x.msg+'\\n  fix: '+x.fix).join('\\n'));\nfunction BrainCompileError(){{const e=document.createElement('div');e.setAttribute('role','alert');e.dataset.brainError=f;\ne.style.cssText='border:2px solid #c42b1c;border-radius:10px;padding:12px 14px;margin:8px 0;font:13px/1.45 ui-monospace,SFMono-Regular,monospace;background:#fff5f4;color:#1d1d1f;white-space:pre-wrap';\ne.textContent=f+' does not compile\\n\\n'+d.map(x=>x.code+' '+x.line+':'+x.col+'  '+x.msg+'\\nfix: '+x.fix).join('\\n\\n');return e}}\n{tail}",
                if hash.is_some() {
                    format!(
                        "import {{ dev }} from \"{}/@brain/brain-dev.js\";\n",
                        ctx.pre
                    )
                } else {
                    String::new()
                },
                diags.join(","),
                json_str(url.trim_start_matches('/'))
            );
            http::respond(s, 200, http::mime("x.js"), js.as_bytes(), head)
        }
    }
}

/// One diagnostic (`brain check --json` shape) for the terminal: `level CODE file:line:col msg` and the fix below it.
fn human_diag(d: &str) -> String {
    let Ok(j) = brain_lex::json::Json::parse(d) else {
        return d.to_string();
    };
    let s = |k: &str| match j.get(k) {
        Some(brain_lex::json::Json::Str(v)) => v.clone(),
        _ => String::new(),
    };
    let n = |k: &str| match j.get(k) {
        Some(brain_lex::json::Json::Num(v)) => *v as u64,
        _ => 0,
    };
    let level = match s("level") {
        l if l.is_empty() => "error".to_string(),
        l => l,
    };
    let fix = s("fix");
    let fix = if fix.is_empty() {
        String::new()
    } else {
        format!("\n    fix: {fix}")
    };
    format!(
        "{level} {} {}:{}:{} {}{fix}",
        s("code"),
        s("file"),
        n("line"),
        n("col"),
        s("msg")
    )
}

fn broadcast_ev(ctx: &Ctx, msg: &str) {
    ctx.emit(msg);
    broadcast(&ctx.clients, msg);
}

fn broadcast(clients: &Clients, msg: &str) {
    let frame = ws::text_frame(msg);
    clients
        .lock()
        .unwrap()
        .retain_mut(|c| c.write_all(&frame).is_ok());
}

fn ignored(rel: &Path) -> bool {
    // What the framework itself writes at runtime: `brain start` logs here, so reloading on it never stops.
    if rel.starts_with("logs") || rel.starts_with("docs/app") {
        return true;
    }
    rel.components().any(|c| {
        let c = c.as_os_str().to_string_lossy();
        c == "dist" || c == "node_modules" || c.starts_with('.')
    })
}

fn queue_watch_event(root: &Path, tx: &std::sync::mpsc::SyncSender<notify::Result<notify::Event>>, overflow: &AtomicBool, event: notify::Result<notify::Event>) {
    // Native backends can lose events before our channel sees them. Queue even a pathless/access
    // rescan signal to wake the worker, and use the same full-cache recovery as channel overflow.
    if matches!(&event, Ok(event) if event.need_rescan()) {
        overflow.store(true, Ordering::Release);
    }
    // Linux reports source reads too. Discard them before the bounded mutation queue: overflowing on
    // access events would turn ordinary compilation/imports into a full document reload.
    if matches!(&event, Ok(event) if event.kind.is_access() && !event.need_rescan()) {
        return;
    }
    // Runtime traces/logs and generated output are ignored by on_change too. They must not displace
    // actual source edits and turn an ignored write burst into lost-mutation recovery.
    // Match on_change's effective identity, not just a lexical alias such as logs -> views. Resolve
    // only already-ignored candidates, at most eight paths; missing/uncertain/large batches stay queued.
    let ignored_path = |path: &Path| path.strip_prefix(root).is_ok_and(|relative| relative != Path::new(".brain_env")
        && (ignored(relative) || relative.to_string_lossy().ends_with(".test.brain")));
    if matches!(&event, Ok(event) if !event.need_rescan() && !event.paths.is_empty() && event.paths.len() <= 8
        && event.paths.iter().all(|path| ignored_path(path))
        && event.paths.iter().all(|path| path.canonicalize().is_ok_and(|resolved| ignored_path(&resolved)))) {
        return;
    }
    if matches!(tx.try_send(event), Err(std::sync::mpsc::TrySendError::Full(_))) {
        overflow.store(true, Ordering::Release);
    }
}

fn start_watch(ctx: Arc<Ctx>, threads: &Threads) -> io::Result<notify::RecommendedWatcher> {
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    let overflow = Arc::new(AtomicBool::new(false));
    let overflowed = overflow.clone();
    let watch_root = ctx.root.clone();
    let mut w = notify::recommended_watcher(move |event| {
        queue_watch_event(&watch_root, &tx, &overflowed, event);
    })
    .map_err(io::Error::other)?;
    w.watch(&ctx.root, RecursiveMode::Recursive)
        .map_err(io::Error::other)?;
    threads.spawn(Role::Watch, move || {
        loop {
            // Collapses the burst of events from one save into a single batch.
            let Ok(first) = rx.recv() else { return };
            let mut paths: Vec<PathBuf> = Vec::new();
            let debug = std::env::var_os("BRAIN_DEBUG_WATCH").is_some();
            let mut push = |ev: notify::Result<notify::Event>| {
                if debug {
                    eprintln!("[watch] {ev:?}");
                }
                if let Ok(ev) = ev
                    && !ev.kind.is_access()
                {
                    paths.extend(ev.paths);
                }
            };
            push(first);
            let batch_started = Instant::now();
            loop {
                if batch_started.elapsed() >= Duration::from_millis(50) {
                    break;
                }
                match rx.recv_timeout(Duration::from_millis(15)) {
                    Ok(ev) => push(ev),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            if overflow.swap(false, Ordering::AcqRel) {
                ctx.cache.lock().unwrap().clear();
                ctx.env_cache.lock().unwrap().take();
                let _ = ctx.env_vars();
                ctx.sidecar.lock().unwrap().take();
                broadcast(&ctx.clients, "{\"type\":\"reload\",\"path\":null,\"reason\":\"watch-overflow\"}");
            }
            paths.sort();
            paths.dedup();
            for p in paths {
                on_change(&ctx, &p);
            }
        }
    })?;
    Ok(w)
}

fn on_change(ctx: &Ctx, path: &Path) {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let Ok(rel) = path.strip_prefix(&ctx.root) else {
        return;
    };
    if rel == Path::new(".brain_env") {
        // new values: re-resolve on next use, restart the load sidecar, recompile (PUBLIC_* are inlined), reload
        *ctx.env_cache.lock().unwrap() = None;
        // re-resolve at once: the PUBLIC_* constants the compiler substitutes come from it
        let _ = ctx.env_vars();
        *ctx.sidecar.lock().unwrap() = None;
        ctx.cache.lock().unwrap().clear();
        ctx.log("info", "[env] .brain_env changed → reload".into());
        return broadcast_ev(ctx, "{\"type\":\"reload\",\"path\":\"/.brain_env\"}");
    }
    if ignored(rel) || path.is_dir() {
        return;
    }
    let url = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
    // Tests are not app modules: `brain test` runs them, not the dev server's browser.
    if url.ends_with(".test.brain") {
        return;
    }
    if path.extension().is_none_or(|e| e != "brain") {
        // Parent mtimes do not change when their tokens or imported modules do.
        ctx.cache.lock().unwrap().retain(|_, (_, compiled)| {
            !compiled
                .deps
                .iter()
                .any(|dep| dep == &path || (dep.is_dir() && path.starts_with(dep)))
        });
        if path.extension().is_some_and(|e| e == "jsbrain") {
            ctx.sidecar.lock().unwrap().take(); // discard Node's immutable module-version cache on server edits
        }
        ctx.log("info", format!("[hmr] {url} → reload"));
        return broadcast_ev(
            ctx,
            &format!("{{\"type\":\"reload\",\"path\":{}}}", json_str(&url)),
        );
    }
    let (prev, dependents) = {
        let mut cache = ctx.cache.lock().unwrap();
        // whoever imports this file (or reads tokens) is dropped and re-checked right after this compile
        let dependents: Vec<PathBuf> = cache
            .iter()
            .filter(|(_, (_, c))| {
                c.deps
                    .iter()
                    .any(|d| d == &path || (d.is_dir() && path.starts_with(d)))
            })
            .map(|(p, _)| p.clone())
            .collect();
        for d in &dependents {
            cache.remove(d);
        }
        (cache.remove(&path).map(|(_, c)| c), dependents)
    };
    let t = Instant::now();
    match compile_cached(ctx, &path) {
        Ok(c) => {
            if prev
                .as_ref()
                .is_some_and(|old| old.server_js != c.server_js)
            {
                ctx.sidecar.lock().unwrap().take();
            }
            let (js, css) = match &prev {
                Some(p) => (p.js != c.js, p.css != c.css),
                None => (true, true),
            };
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            ctx.log(
                "info",
                format!("[hmr] {url} {ms:.2} ms (js:{js} css:{css})"),
            );
            if js || css {
                broadcast_ev(
                    ctx,
                    &format!(
                        "{{\"type\":\"update\",\"path\":{},\"js\":{js},\"css\":{css},\"ms\":{ms:.3}}}",
                        json_str(&url)
                    ),
                );
            }
            // warnings and info of the file (an empty list clears what the overlay showed for it), on the overlay
            // and in the terminal
            if let Some(all) = ctx.file_diags(&path) {
                for d in &all {
                    ctx.log(
                        if d.contains("\"level\":\"info\"") {
                            "info"
                        } else {
                            "warn"
                        },
                        human_diag(d),
                    );
                }
                broadcast_ev(
                    ctx,
                    &format!(
                        "{{\"type\":\"diags\",\"file\":{},\"all\":[{}]}}",
                        json_str(url.trim_start_matches('/')),
                        all.join(",")
                    ),
                );
            }
        }
        Err(diags) => {
            ctx.log("error", format!("[hmr] {url} → {} error(s)", diags.len()));
            for d in &diags {
                ctx.log("error", human_diag(d));
            }
            let all = ctx
                .file_diags(&path)
                .map(|a| format!(",\"all\":[{}]", a.join(",")))
                .unwrap_or_default();
            broadcast_ev(
                ctx,
                &format!(
                    "{{\"type\":\"error\",\"file\":{},\"diags\":[{}]{all}}}",
                    json_str(url.trim_start_matches('/')),
                    diags.join(",")
                ),
            );
            return;
        }
    }
    // A changed contract (props, emits, tokens) can break the importers: re-check them now so the error shows up
    // on this save instead of on the next request.
    for d in dependents {
        if let Err(diags) = compile_cached(ctx, &d) {
            let rel = d
                .strip_prefix(&ctx.root)
                .unwrap_or(&d)
                .to_string_lossy()
                .replace('\\', "/");
            ctx.log(
                "error",
                format!("[hmr] {rel} (importer) → {} error(s)", diags.len()),
            );
            for d in &diags {
                ctx.log("error", human_diag(d));
            }
            broadcast_ev(
                ctx,
                &format!("{{\"type\":\"error\",\"diags\":[{}]}}", diags.join(",")),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    struct WatchFixture(PathBuf);
    impl WatchFixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("brain-watch-admission-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "owned fixture").unwrap();
            path
        }
    }
    impl Drop for WatchFixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    #[test]
    fn ignored_runtime_writes_do_not_overflow_the_mutation_queue() {
        use notify::event::ModifyKind;
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        let overflow = AtomicBool::new(false);
        let fixture = WatchFixture::new();
        for relative in [".brain/trace/events.jsonl", "logs/app.log", "docs/app/data.md", "dist/chunk.js", "node_modules/example/index.js"] {
            let path = fixture.file(relative);
            for _ in 0..128 {
                let event = notify::Event::new(notify::EventKind::Modify(ModifyKind::Any)).add_path(path.clone());
                queue_watch_event(&fixture.0, &tx, &overflow, Ok(event));
            }
        }
        assert!(!overflow.load(Ordering::Acquire), "ignored framework writes must not request lost-source recovery");
        assert!(rx.try_recv().is_err(), "ignored runtime writes must leave capacity for actual source changes");
    }

    #[test]
    fn watch_admission_preserves_environment_edits_and_mixed_source_renames() {
        use notify::event::ModifyKind;
        let fixture = WatchFixture::new();
        let root = fixture.0.as_path();
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let overflow = AtomicBool::new(false);
        for paths in [vec![root.join(".brain_env")], vec![root.join("views/home.brain")],
            vec![root.join(".editor-temp"), root.join("views/home.brain")],
            vec![PathBuf::from("/other/source.brain")], vec![]] {
            let mut event = notify::Event::new(notify::EventKind::Modify(ModifyKind::Any));
            event.paths = paths;
            queue_watch_event(root, &tx, &overflow, Ok(event.clone()));
            assert_eq!(rx.try_recv().unwrap().unwrap(), event);
        }
        for path in [fixture.file("comp/example.test.brain"), fixture.file(".brain/cache/source.js")] {
            queue_watch_event(root, &tx, &overflow, Ok(notify::Event::new(notify::EventKind::Modify(ModifyKind::Any)).add_path(path)));
        }
        assert!(rx.try_recv().is_err());
        assert!(!overflow.load(Ordering::Acquire));
    }

    #[cfg(unix)]
    #[test]
    fn watch_admission_keeps_ignored_aliases_to_source_and_environment() {
        use notify::event::ModifyKind;
        let fixture = WatchFixture::new();
        fixture.file("views/home.brain");
        let env = fixture.file(".brain_env");
        std::os::unix::fs::symlink(fixture.0.join("views"), fixture.0.join("logs")).unwrap();
        std::fs::create_dir(fixture.0.join(".brain")).unwrap();
        std::os::unix::fs::symlink(env, fixture.0.join(".brain/env-alias")).unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let overflow = AtomicBool::new(false);
        for path in [fixture.0.join("logs/home.brain"), fixture.0.join(".brain/env-alias")] {
            let event = notify::Event::new(notify::EventKind::Modify(ModifyKind::Any)).add_path(path);
            queue_watch_event(&fixture.0, &tx, &overflow, Ok(event.clone()));
            assert_eq!(rx.try_recv().unwrap().unwrap(), event);
        }
        assert!(!overflow.load(Ordering::Acquire));
    }

    #[test]
    fn watch_admission_keeps_uncertain_large_and_rescan_events() {
        use notify::event::{AccessKind, Flag, ModifyKind};
        let fixture = WatchFixture::new();
        let ignored = fixture.file(".brain/cache/source.js");
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let overflow = AtomicBool::new(false);
        let mut large = notify::Event::new(notify::EventKind::Modify(ModifyKind::Any));
        large.paths = vec![ignored.clone(); 9];
        for event in [large,
            notify::Event::new(notify::EventKind::Modify(ModifyKind::Any)).add_path(fixture.0.join(".brain/missing")),
            notify::Event::new(notify::EventKind::Modify(ModifyKind::Any)).add_path(ignored.clone()).set_flag(Flag::Rescan),
            notify::Event::new(notify::EventKind::Access(AccessKind::Read)).add_path(ignored).set_flag(Flag::Rescan)] {
            queue_watch_event(&fixture.0, &tx, &overflow, Ok(event.clone()));
            assert_eq!(rx.try_recv().unwrap().unwrap(), event);
            assert_eq!(overflow.load(Ordering::Acquire), event.need_rescan());
            overflow.store(false, Ordering::Release);
        }
        assert!(!overflow.load(Ordering::Acquire));
    }

    #[test]
    fn native_pathless_rescan_wakes_the_worker_and_requests_full_recovery() {
        use notify::event::{AccessKind, Flag};
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let overflow = AtomicBool::new(false);
        for kind in [notify::EventKind::Any, notify::EventKind::Access(AccessKind::Read)] {
            let event = notify::Event::new(kind).set_flag(Flag::Rescan);
            queue_watch_event(Path::new("/project"), &tx, &overflow, Ok(event.clone()));
            assert_eq!(rx.try_recv().unwrap().unwrap(), event, "a rescan must wake the consumer without a source path");
            assert!(overflow.swap(false, Ordering::AcqRel), "the consumer must enter its existing full-cache recovery");
        }
    }

    #[test]
    fn watch_access_bursts_do_not_consume_mutation_capacity_or_force_reload() {
        use notify::event::{AccessKind, ModifyKind};
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        let overflow = AtomicBool::new(false);
        for _ in 0..256 {
            queue_watch_event(Path::new("/project"), &tx, &overflow, Ok(notify::Event::new(notify::EventKind::Access(AccessKind::Read))));
        }
        assert!(!overflow.load(Ordering::Acquire), "reading source must not signal lost mutations or reload the document");
        assert!(rx.try_recv().is_err(), "access events must not occupy the bounded mutation queue");
        let changed = notify::Event::new(notify::EventKind::Modify(ModifyKind::Any));
        queue_watch_event(Path::new("/project"), &tx, &overflow, Ok(changed.clone()));
        assert_eq!(rx.try_recv().unwrap().unwrap(), changed);
        for _ in 0..65 {
            queue_watch_event(Path::new("/project"), &tx, &overflow, Ok(changed.clone()));
        }
        assert!(overflow.load(Ordering::Acquire), "lost real mutations must retain full reload recovery");
    }

    #[test]
    fn watch_access_filter_preserves_errors_and_all_mutation_families() {
        use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind};
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let overflow = AtomicBool::new(false);
        for access in [AccessKind::Any, AccessKind::Read, AccessKind::Open(AccessMode::Read), AccessKind::Close(AccessMode::Write), AccessKind::Other] {
            queue_watch_event(Path::new("/project"), &tx, &overflow, Ok(notify::Event::new(notify::EventKind::Access(access))));
        }
        assert!(rx.try_recv().is_err());
        for kind in [notify::EventKind::Any, notify::EventKind::Create(CreateKind::Any), notify::EventKind::Modify(ModifyKind::Any), notify::EventKind::Remove(RemoveKind::Any), notify::EventKind::Other] {
            queue_watch_event(Path::new("/project"), &tx, &overflow, Ok(notify::Event::new(kind)));
            assert_eq!(rx.try_recv().unwrap().unwrap().kind, kind);
        }
        queue_watch_event(Path::new("/project"), &tx, &overflow, Err(notify::Error::generic("owned watch error")));
        assert!(rx.try_recv().unwrap().is_err());
        assert!(!overflow.load(Ordering::Acquire));
    }

    #[test]
    fn inspector_metrics_are_capped_by_count_and_bytes_not_only_age() {
        let mut m = std::collections::VecDeque::new();
        for i in 0..(super::METRICS_MAX * 3) {
            super::push_metric(&mut m, 1_000, format!("{{\"t\":\"m\",\"i\":{i}}}"));
        }
        assert_eq!(m.len(), super::METRICS_MAX);
        assert!(m.back().unwrap().1.ends_with(&format!("{}}}", super::METRICS_MAX * 3 - 1)), "the newest batch is kept");
        let big = "x".repeat(super::METRICS_BYTES / 4 + 1);
        for _ in 0..10 {
            super::push_metric(&mut m, 1_000, big.clone());
        }
        assert!(m.iter().map(|(_, t)| t.len()).sum::<usize>() <= super::METRICS_BYTES);
        assert_eq!(m.back().unwrap().1.len(), big.len());
        super::push_metric(&mut m, 1_000 + super::METRICS_MS + 1, "late".into());
        assert_eq!(m.len(), 1, "older than the window is dropped");
    }
    #[test]
    fn the_watcher_skips_what_the_framework_writes_but_not_app_files() {
        use std::path::Path;
        for skipped in ["logs/app.log", "docs/app/data.md", "dist/x.js", "node_modules/a/b.js", ".brain/x"] {
            assert!(super::ignored(Path::new(skipped)), "{skipped}");
        }
        for watched in ["views/home.brain", "server/logs.jsbrain", "views/logs/index.brain", "docs/guide.md", "content/a.md"] {
            assert!(!super::ignored(Path::new(watched)), "{watched}");
        }
    }

    use super::*;

    #[test]
    fn dev_health_aliases_map_get_and_head_to_native_probes() {
        use std::io::{Read, Write};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!("brain-dev-health-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
        let _cleanup = Cleanup(root.clone());
        let opts = || Options { root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(),
            runtime: vec![], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None };
        let compile: CompileFn = Arc::new(|_, _| Ok(Compiled { server: Default::default(), js: String::new(), css: String::new(), deps: vec![], server_js: String::new() }));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let target = format!("http://{}", listener.local_addr().unwrap());
        let backend = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let mut seen = Vec::new();
            while seen.len() < 6 && start.elapsed() < Duration::from_secs(10) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        // BSD sockets inherit the listener's nonblocking mode.
                        socket.set_nonblocking(false).unwrap();
                        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        let request = http::read_request(socket.try_clone().unwrap()).unwrap();
                        let known = native_proxy::is_probe(&request.path);
                        http::respond_with(&mut socket, if known { 503 } else { 404 }, "application/json", b"{\"code\":\"DRAINING\"}", request.method == "HEAD", &[]).unwrap();
                        seen.push((request.method, request.path));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
                    Err(error) => panic!("owned backend accept failed: {error}"),
                }
            }
            seen
        });
        let proxy = native_proxy::NativeProxy::new(&target).unwrap();
        let mut dev = start_dev_with_native_proxy(opts(), compile.clone(), false, None, Some(proxy)).unwrap();
        let ask = |address, method: &str, path: &str| {
            let mut socket = std::net::TcpStream::connect(address).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            socket.write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            let mut response = String::new(); socket.read_to_string(&mut response).unwrap(); response
        };
        let mut responses = Vec::new();
        let mut expected = Vec::new();
        for path in ["/health", "/health/live", "/health/ready"] {
            for method in ["GET", "HEAD"] {
                responses.push((method, ask(dev.addr, method, &format!("/_brain{path}"))));
                expected.push((method.to_string(), path.to_string()));
            }
        }
        dev.stop();
        assert_eq!(backend.join().unwrap(), expected);
        for (method, response) in responses {
            assert!(response.starts_with("HTTP/1.1 503") && response.contains("no-store"), "{response}");
            if method == "HEAD" { assert!(response.ends_with("\r\n\r\n"), "{response}"); }
        }
        let mut alone = start_dev(opts(), compile, false, None).unwrap();
        let response = ask(alone.addr, "GET", "/_brain/health");
        let unknown = ask(alone.addr, "GET", "/_brain/health/other");
        alone.stop();
        assert!(response.starts_with("HTTP/1.1 200") && response.contains("no-store"), "{response}");
        assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");
    }

    #[test]
    fn dev_rpc_and_form_refusals_name_the_seen_origin_host_and_exact_fix() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!("brain-dev-origin-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
        let _cleanup = Cleanup(root.clone());
        let opts = Options {
            root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(),
            runtime: vec![], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None,
        };
        let compile: CompileFn = Arc::new(|_, _| Ok(Compiled { server: Default::default(), js: String::new(), css: String::new(), deps: vec![], server_js: String::new() }));
        let mut dev = start_dev(opts, compile, false, None).unwrap();
        let post = |path: &str, headers: &str| {
            use std::io::{Read, Write};
            let mut stream = std::net::TcpStream::connect(dev.addr).unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            write!(stream, "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{headers}Content-Length: 2\r\nConnection: close\r\n\r\n[]", dev.addr.port()).unwrap();
            let mut out = String::new();
            let _ = stream.read_to_string(&mut out);
            out
        };
        let rpc = post("/_r/abc/save", "X-Brain-RPC: 1\r\nOrigin: https://shop.example\r\n");
        assert!(rpc.starts_with("HTTP/1.1 403"), "{rpc}");
        for needle in ["Cross-origin request", "\"brainCode\":\"B0741\"", "\"seenOrigin\":\"https://shop.example\"", "\"host\":\"127.0.0.1:", "proxy_set_header Host $host", "brain explain B0741"] {
            assert!(rpc.contains(needle), "{needle} missing in {rpc}");
        }
        dev.stop();
    }

    #[test]
    fn explicit_native_proxy_preserves_an_absent_environment_loader_scope() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!("brain-native-env-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
        let _cleanup = Cleanup(root.clone());
        let opts = || Options {
            root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(),
            runtime: vec![], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None,
        };
        let compile: CompileFn = Arc::new(|_, _| Ok(Compiled { server: Default::default(), js: String::new(), css: String::new(), deps: vec![], server_js: String::new() }));
        let mut legacy = start_dev(opts(), compile.clone(), false, None).unwrap();
        assert!(legacy.ctx.env_vars().is_empty(), "An absent loader must not expose ambient process variables");
        legacy.stop();
        drop(legacy);
        let proxy = native_proxy::NativeProxy::new("http://127.0.0.1:9").unwrap();
        let mut explicit = start_dev_with_native_proxy(opts(), compile, false, None, Some(proxy)).unwrap();
        let values = explicit.ctx.env_vars();
        // Never format the actual collection on failure: it could contain host credentials if this regresses.
        assert!(values.len() == 1 && values[0].0 == "BRAIN_BACKEND_URL" && values[0].1 == "http://127.0.0.1:9",
            "An explicit native authority may add only its own backend setting to an empty environment scope");
        explicit.stop();
    }

    #[test]
    fn strip_meta_removes_only_that_tag() {
        assert_eq!(
            strip_meta(
                "<meta http base=\"/api\">\n<html><meta httpx=1><meta charset=\"utf-8\"></html>",
                "http"
            ),
            "\n<html><meta httpx=1><meta charset=\"utf-8\"></html>"
        );
        assert_eq!(strip_meta("<meta http>x", "http"), "x");
    }

    #[test]
    fn net_line_keeps_query_names_and_drops_values() {
        let l = net_line(r#"{"t":"brain:net","method":"GET","url":"/app/users?token=s3cret&page=2","status":200,"ms":12.4,"size":1300}"#).unwrap();
        assert_eq!(l, "[net] GET /app/users?token&page 200 12 ms 1.3 KB");
        assert!(!l.contains("s3cret"));
        assert_eq!(
            net_line(
                r#"{"t":"brain:net","method":"POST","url":"/api/x","status":422,"ms":3,"size":40}"#
            )
            .unwrap(),
            "[net] POST /api/x 422 3 ms 40 B"
        );
    }

    #[test]
    fn resolve_rejects_traversal() {
        let r = Path::new("/app");
        assert_eq!(
            resolve(r, "/views/home.brain"),
            Some(PathBuf::from("/app/views/home.brain"))
        );
        assert_eq!(resolve(r, "/../etc/passwd"), None);
        assert_eq!(resolve(r, "/a/./b"), Some(PathBuf::from("/app/a/b")));
    }

    #[test]
    fn extracts_script_body() {
        assert_eq!(script_body("<script>\n test()\n</script>\n"), "\n test()\n");
        assert_eq!(script_body("<script type=\"x\">a</script>"), "a");
    }

    #[test]
    fn prepends_after_head_open() {
        assert_eq!(
            prepend_head("<html><head lang=x><link></head>", "<style>a</style>"),
            "<html><head lang=x><style>a</style><link></head>"
        );
    }

    #[test]
    fn injects_client_in_head() {
        let h = inject_client("<html><head><title>x</title></head><body></body></html>");
        assert!(h.contains(&format!("{CLIENT_TAG}</head>")));
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;

    #[test]
    fn rpc_inventory_skips_links_and_reports_incomplete_scans() {
        let root = std::env::temp_dir().join(format!("brain-rpc-inventory-{}", std::process::id()));
        std::fs::create_dir_all(root.join("comp")).unwrap();
        std::fs::write(root.join("comp/real.brain"), "").unwrap();
        std::fs::write(root.join("comp/ignored.test.brain"), "").unwrap();
        #[cfg(unix)] {
            std::os::unix::fs::symlink(&root, root.join("comp/loop")).unwrap();
            std::os::unix::fs::symlink(root.join("comp/real.brain"), root.join("linked.brain")).unwrap();
        }
        let mut files=Vec::new(); assert!(rpc_candidates(&root, &mut files));
        assert_eq!(files, [root.join("comp/real.brain")]);
        let mut deep=root.clone(); for _ in 0..34 { deep=deep.join("nested"); }
        std::fs::create_dir_all(deep).unwrap();
        assert!(!rpc_candidates(&root, &mut Vec::new()));
        assert!(!rpc_candidates(&root.join("missing"), &mut Vec::new()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dev_rpc_uses_exact_owned_metadata_and_invalidates_changed_candidates() {
        let root = std::env::temp_dir().join(format!("brain-rpc-metadata-{}", std::process::id()));
        std::fs::create_dir_all(root.join("comp")).unwrap();
        let candidate = root.join("comp/candidate.brain");
        std::fs::write(&candidate, "decoy").unwrap();
        let compiler: CompileFn = Arc::new(|source, _| Ok(Compiled {
            js: "const example = \"abc123/save\";".into(), css: String::new(), deps: vec![],
            server_js: "export function save() { return 'saved' }\nexport const $server=['save'];".into(),
            server: brain_lex::ServerMetadata { rpc: if source == "owned" { vec!["abc123/save".into()] } else { vec![] }, ..Default::default() },
        }));
        let opts = Options { root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(),
            runtime: vec![("server/server.mjs".into(), include_str!("../../../runtime/server/server.mjs").into()), ("server/native-sockets.mjs".into(), include_str!("../../../runtime/server/native-sockets.mjs").into()), ("server/llm.mjs".into(), include_str!("../../../runtime/server/llm.mjs").into())], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None };
        let dev = start_dev(opts, compiler, false, None).unwrap();
        let call = |name: &str| {
            let mut client = TcpStream::connect(dev.addr).unwrap();
            client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            client.write_all(format!("POST /_r/abc123/{name} HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nX-Brain-RPC: 1\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]", dev.addr, dev.addr).as_bytes()).unwrap();
            let mut response=String::new(); client.read_to_string(&mut response).unwrap(); response
        };
        let response=call("save"); assert!(response.starts_with("HTTP/1.1 404"), "{response}");
        std::fs::write(&candidate, "owned").unwrap();
        let response=call("save"); assert!(response.starts_with("HTTP/1.1 200") && response.contains("saved"), "{response}");
        let response=call("load"); assert!(response.starts_with("HTTP/1.1 404"), "{response}");
        std::fs::write(root.join("comp/duplicate.brain"), "owned").unwrap();
        let response=call("save"); assert!(response.starts_with("HTTP/1.1 404") && response.contains("Ambiguous"), "{response}");
        std::fs::remove_file(root.join("comp/duplicate.brain")).unwrap();
        std::fs::write(&candidate, "decoy").unwrap();
        let response=call("save"); assert!(response.starts_with("HTTP/1.1 404"), "{response}");
        drop(dev);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dev_pages_without_seo_still_include_compiler_attribution() {
        let root = std::env::temp_dir().join(format!("brain-dev-attribution-{}", std::process::id()));
        std::fs::create_dir_all(root.join("views")).unwrap();
        std::fs::write(root.join("app.brain"), "<html><head><title>Example</title></head><body><slot /></body></html>").unwrap();
        std::fs::write(root.join("views/index.brain"), "<main>Example</main>").unwrap();
        let compiler: CompileFn = Arc::new(|_, _| Ok(Compiled { server: Default::default(), js: String::new(), css: String::new(), deps: vec![], server_js: String::new() }));
        let opts = Options { root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(), runtime: vec![], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None };
        let dev = start_dev(opts, compiler, false, None).unwrap();
        let mut client = TcpStream::connect(dev.addr).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        client.write_all(format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", dev.addr.port()).as_bytes()).unwrap();
        let mut response = String::new(); client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert_eq!(response.matches(brain_lex::branding::HTML).count(), 1, "{response}");
        assert!(response.find(brain_lex::branding::HTML).unwrap() < response.find("</head>").unwrap());
        drop(dev);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_public_files_precede_dev_app_page_fallback() {
        let root = std::env::temp_dir().join(format!("brain-dev-public-priority-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("public/.well-known")).unwrap();
        std::fs::write(root.join("app.brain"), "<html><head><title>App</title></head><body><slot /></body></html>").unwrap();
        std::fs::write(root.join("public/verification"), "public-verification").unwrap();
        std::fs::write(root.join("public/.well-known/verification"), "well-known-verification").unwrap();
        let compiler: CompileFn = Arc::new(|src, path| Ok(Compiled { server: Default::default(),js: String::new(), css: String::new(), deps: vec![], server_js: if path.extension().is_some_and(|ext|ext=="jsbrain") {src.into()} else {String::new()}}));
        let opts = Options { root: root.clone(), host: "127.0.0.1".into(), port:0, head_prepend:String::new(),
            runtime:vec![("server/server.mjs".into(), include_str!("../../../runtime/server/server.mjs").into()), ("server/native-sockets.mjs".into(), include_str!("../../../runtime/server/native-sockets.mjs").into()), ("server/llm.mjs".into(), include_str!("../../../runtime/server/llm.mjs").into())], env:None, base:"/".into(), diagnose:None, problems:None, upstream:None, mock:None };
        let dev = start_dev(opts, compiler, false, None).unwrap();
        for (path, value) in [("/verification", "public-verification"), ("/.well-known/verification", "well-known-verification")] {
            for method in ["GET", "HEAD"] {
                let mut client = TcpStream::connect(dev.addr).unwrap();
                client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                client.write_all(format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",dev.addr.port()).as_bytes()).unwrap();
                let mut response = String::new(); client.read_to_string(&mut response).unwrap();
                assert!(response.starts_with("HTTP/1.1 200"), "{response}");
                let body = response.split_once("\r\n\r\n").unwrap().1;
                assert_eq!(body, if method == "HEAD" { "" } else { value });
            }
        }
        std::fs::create_dir_all(root.join("views/secure")).unwrap();
        std::fs::write(root.join("views/secure/index.brain"),"<main>Private</main>").unwrap();
        std::fs::write(root.join("views/secure/_guard.jsbrain"),"export default () => false").unwrap();
        std::fs::write(root.join("public/secure"),"stale-private-output").unwrap();
        std::fs::write(root.join("brain.json"),r#"{"i18n":{"default":"en","locales":["en","es"]}}"#).unwrap();
        std::fs::write(root.join("views/secure/[slug].brain"),"<main>Private item</main>").unwrap();
        std::fs::create_dir_all(root.join("public/es")).unwrap();
        std::fs::write(root.join("public/es/secure"),"stale-private-output").unwrap();
        // Replace the colliding extensionless file with a directory for the dynamic case.
        std::fs::remove_file(root.join("public/secure")).unwrap();
        std::fs::create_dir_all(root.join("public/secure")).unwrap();
        std::fs::write(root.join("public/secure/report.txt"),"stale-private-output").unwrap();
        for path in ["/secure", "/es/secure", "/secure/report.txt"] {
            for method in ["GET", "HEAD"] {
                let mut client = TcpStream::connect(dev.addr).unwrap();
                client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                client.write_all(format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",dev.addr.port()).as_bytes()).unwrap();
                let mut response=String::new();client.read_to_string(&mut response).unwrap();
                assert!(response.starts_with("HTTP/1.1 403"),"{path}: {response}");
                assert!(!response.contains("stale-private-output"));
            }
        }
        drop(dev);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn connection_admission_is_bounded_and_releases_slots_on_unwind() {
        let active = Arc::new(AtomicUsize::new(0));
        let permits: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| connection_permit(&active).unwrap())
            .collect();
        assert!(connection_permit(&active).is_none());
        drop(permits);
        assert_eq!(active.load(Ordering::Acquire), 0);
        let _ = std::panic::catch_unwind(|| {
            let _permit = connection_permit(&active).unwrap();
            panic!("simulated handler failure");
        });
        assert_eq!(active.load(Ordering::Acquire), 0);
    }
    #[test]
    fn trickling_requests_cannot_extend_the_total_read_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sender = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            for _ in 0..20 {
                if stream.write_all(b"a").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let (stream, _) = listener.accept().unwrap();
        let start = Instant::now();
        let mut reader = RequestReader {
            stream: &stream,
            deadline: start + Duration::from_millis(100),
        };
        let mut byte = [0u8];
        while reader.read(&mut byte).is_ok_and(|n| n > 0) {}
        assert!(start.elapsed() < Duration::from_secs(1));
        drop(stream);
        sender.join().unwrap();
    }

    #[test]
    fn dev_sitemap_stops_rendering_at_its_time_budget() {
        assert!(dev_seo_renders(0, Duration::ZERO));
        assert!(dev_seo_renders(DEV_SEO_RENDER_MAX - 1, DEV_SEO_RENDER_BUDGET - Duration::from_millis(1)));
        assert!(!dev_seo_renders(1, DEV_SEO_RENDER_BUDGET));
        assert!(!dev_seo_renders(DEV_SEO_RENDER_MAX, Duration::ZERO));
    }

    #[test]
    fn dev_sitemap_and_llms_txt_match_the_build_shape() {
        // parity with `brain build`: image entries, hreflang alternates with x-default, llms.txt per locale
        let root = std::env::temp_dir().join(format!("brain-dev-seo-parity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (f, src) in [
            ("brain.json", "{\"i18n\":{\"default\":\"en\",\"locales\":[\"en\",\"es\"]}}"),
            ("i18n/en.json", "{\"hi\":\"Hi\"}"),
            ("i18n/es.json", "{\"hi\":\"Hola\"}"),
            ("views/home.brain", "<main><img src=\"/hero.png\" alt=\"Hero\"><img src={props.photo} alt=\"\"></main>"),
            ("views/about.brain", "<main><p></p></main>"),
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, src).unwrap();
        }
        let root = root.canonicalize().unwrap();
        let compiler: CompileFn = Arc::new(|src, _| Ok(Compiled { server: Default::default(), js: String::new(), css: String::new(), deps: vec![], server_js: src.into() }));
        let opts = Options {
            root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(),
            runtime: vec![], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None,
        };
        let dev = start_dev(opts, compiler, false, None).unwrap();
        let get = |path: &str| {
            use std::io::{Read, Write};
            let addr = dev.url().trim_start_matches("http://").trim_end_matches('/').to_string();
            let mut c = TcpStream::connect(&addr).unwrap();
            write!(c, "GET {path} HTTP/1.1\r\nHost: shop.local:5180\r\nConnection: close\r\n\r\n").unwrap();
            let mut out = String::new();
            c.read_to_string(&mut out).unwrap();
            out
        };
        let sm = get("/sitemap.xml");
        assert!(sm.starts_with("HTTP/1.1 200"), "{sm}");
        assert!(sm.contains("<image:image><image:loc>http://shop.local:5180/hero.png</image:loc></image:image>"), "{sm}");
        assert!(!sm.contains("props.photo"), "{sm}");
        assert!(sm.contains("<loc>http://shop.local:5180/es/about</loc>"), "{sm}");
        assert!(sm.contains("hreflang=\"es\" href=\"http://shop.local:5180/es/about\"") && sm.contains("hreflang=\"x-default\" href=\"http://shop.local:5180/about\""), "{sm}");
        let llms = get("/llms.txt");
        assert!(llms.starts_with("HTTP/1.1 200") && llms.contains("text/plain"), "{llms}");
        assert!(llms.contains("(http://shop.local:5180/about)") && !llms.contains("/es/about"), "{llms}");
        let es = get("/es/llms.txt");
        assert!(es.contains("(http://shop.local:5180/es/about)"), "{es}");
        drop(dev);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dev_answers_one_byte_range_of_a_public_file() {
        let root = std::env::temp_dir().join(format!("brain-dev-range-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("public")).unwrap();
        std::fs::create_dir_all(root.join("views")).unwrap();
        std::fs::write(root.join("views/home.brain"), "<p></p>").unwrap();
        std::fs::write(root.join("public/clip.mp4"), (0u8..100).collect::<Vec<u8>>()).unwrap();
        let root = root.canonicalize().unwrap();
        let compiler: CompileFn = Arc::new(|src, _| Ok(Compiled { server: Default::default(), js: String::new(), css: String::new(), deps: vec![], server_js: src.into() }));
        let opts = Options {
            root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(),
            runtime: vec![], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None,
        };
        let dev = start_dev(opts, compiler, false, None).unwrap();
        let ask = |method: &str, extra: &str| {
            use std::io::{Read, Write};
            let addr = dev.url().trim_start_matches("http://").trim_end_matches('/').to_string();
            let mut c = TcpStream::connect(&addr).unwrap();
            write!(c, "{method} /clip.mp4 HTTP/1.1\r\nHost: localhost\r\n{extra}Connection: close\r\n\r\n").unwrap();
            let mut out = Vec::new();
            c.read_to_end(&mut out).unwrap();
            let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            (String::from_utf8_lossy(&out[..split]).to_lowercase(), out[split + 4..].to_vec())
        };
        let (h, body) = ask("GET", "Range: bytes=2-4\r\n");
        assert!(h.starts_with("http/1.1 206") && h.contains("content-range: bytes 2-4/100") && h.contains("accept-ranges: bytes"), "{h}");
        assert_eq!(body, vec![2, 3, 4]);
        let (h, body) = ask("GET", "Range: bytes=-3\r\n");
        assert!(h.contains("content-range: bytes 97-99/100"), "{h}");
        assert_eq!(body, vec![97, 98, 99]);
        let (h, _) = ask("GET", "Range: bytes=100-\r\n");
        assert!(h.starts_with("http/1.1 416") && h.contains("content-range: bytes */100"), "{h}");
        for extra in ["Range: bytes=0-1, 5-6\r\n", "Range: bytes=0-1\r\nIf-Range: \"x\"\r\n", ""] {
            let (h, body) = ask("GET", extra);
            assert!(h.starts_with("http/1.1 200") && h.contains("accept-ranges: bytes"), "{extra}: {h}");
            assert_eq!(body.len(), 100, "{extra}");
        }
        let (h, body) = ask("HEAD", "Range: bytes=0-1\r\n");
        assert!(h.starts_with("http/1.1 206") && h.contains("content-length: 2"), "{h}");
        assert!(body.is_empty());
        drop(dev);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dev_serves_robots_and_sitemap_with_live_sitemap_urls() {
        // zero config: no build step, the dev server answers /robots.txt and /sitemap.xml like a production build
        let root = std::env::temp_dir().join(format!("brain-dev-seo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (f, src) in [
            ("views/home.brain", "<p></p>"),
            ("views/about.brain", "<p></p>"),
            ("views/thanks.brain", "<meta seo robots=\"noindex\"><p></p>"),
            ("views/404.brain", "<p></p>"),
            ("views/blog/[slug].brain", "export async function sitemap({ env }) { return [{ slug: 'hello world', lastmod: '2026-10-01' }, { slug: 'second' }, {}] }\n"),
            ("views/users/[id].brain", "export async function load() { return {} }\n"),
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, src).unwrap();
        }
        let root = root.canonicalize().unwrap();
        let compiler: CompileFn = Arc::new(|src, path| Ok(Compiled { server: brain_lex::ServerMetadata { sitemap: path.ends_with("blog/[slug].brain"), load: path.ends_with("users/[id].brain"), ..Default::default() }, js: String::new(), css: String::new(), deps: vec![], server_js: src.into() }));
        let opts = Options {
            root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(),
            runtime: vec![], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None,
        };
        let dev = start_dev(opts, compiler, false, None).unwrap();
        let get = |path: &str| {
            use std::io::{Read, Write};
            let addr = dev.url().trim_start_matches("http://").trim_end_matches('/').to_string();
            let mut c = TcpStream::connect(&addr).unwrap();
            write!(c, "GET {path} HTTP/1.1\r\nHost: shop.local:5180\r\nConnection: close\r\n\r\n").unwrap();
            let mut out = String::new();
            c.read_to_string(&mut out).unwrap();
            out
        };
        let robots = get("/robots.txt");
        assert!(robots.starts_with("HTTP/1.1 200") && robots.contains("Sitemap: http://shop.local:5180/sitemap.xml"), "{robots}");
        let sm = get("/sitemap.xml");
        assert!(sm.starts_with("HTTP/1.1 200") && sm.contains("application/xml"), "{sm}");
        assert!(sm.contains("<loc>http://shop.local:5180/</loc>") && sm.contains("<loc>http://shop.local:5180/about</loc>"), "{sm}");
        assert!(sm.contains("<loc>http://shop.local:5180/blog/hello%20world</loc><lastmod>2026-10-01</lastmod>"), "{sm}");
        assert!(sm.contains("<loc>http://shop.local:5180/blog/second</loc>"), "{sm}");
        assert!(!sm.contains("thanks") && !sm.contains("404") && !sm.contains("/users/"), "{sm}");
        // a file in public/ still wins
        std::fs::create_dir_all(root.join("public")).unwrap();
        std::fs::write(root.join("public/robots.txt"), "User-agent: *\nDisallow: /private\n").unwrap();
        assert!(get("/robots.txt").contains("Disallow: /private"));
        drop(dev);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fixed_views_are_read_back_and_titled_like_the_build() {
        let root = std::env::temp_dir().join(format!("brain-dev-fixed-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(super::fixed_view_of("<script>{\"v\":\"/views/_error.brain\",\"l\":[]}</script>").as_deref(), Some("views/_error.brain"));
        assert_eq!(super::fixed_view_of("<main role=\"alert\"><h1>Not found</h1></main>"), None);
        assert_eq!(super::titled(&root, "Oops"), "Oops");
        std::fs::write(root.join("app.brain"), "<meta seo titleTemplate=\"%s · Site\">\n<slot />\n").unwrap();
        assert_eq!(super::titled(&root, "Oops"), "Oops · Site");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn replaced_source_with_preserved_mtime_is_recompiled_without_a_watch_event() {
        let root = std::env::temp_dir().join(format!("brain-dev-mtime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let source = root.join("view.brain");
        std::fs::write(&source, "first").unwrap();
        let stamp = std::fs::metadata(&source).unwrap().modified().unwrap();
        let compiler: CompileFn = Arc::new(|src, _| {
            Ok(Compiled { server: Default::default(), js: String::new(), css: String::new(), deps: vec![], server_js: src.into() })
        });
        let opts = Options {
            root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(),
            runtime: vec![], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None,
        };
        let dev = start_dev(opts, compiler, false, None).unwrap();
        assert_eq!(compile_cached(&dev.ctx, &source).unwrap().server_js, "first");
        // An editor or checkout may replace the file atomically while keeping its timestamp; no watch event arrives here.
        let staged = root.join("view.brain.tmp");
        std::fs::write(&staged, "second edit").unwrap();
        std::fs::File::options().write(true).open(&staged).unwrap().set_modified(stamp).unwrap();
        std::fs::rename(&staged, &source).unwrap();
        assert_eq!(std::fs::metadata(&source).unwrap().modified().unwrap(), stamp);
        assert_eq!(compile_cached(&dev.ctx, &source).unwrap().server_js, "second edit");
        // Equal-length in-place writes can preserve every metadata field used by a cache stamp.
        let previous = compile_cached(&dev.ctx, &source).unwrap();
        assert!(Arc::ptr_eq(&previous, &compile_cached(&dev.ctx, &source).unwrap()));
        std::fs::write(&source, "third  edit").unwrap();
        std::fs::File::options().write(true).open(&source).unwrap().set_modified(stamp).unwrap();
        assert_eq!(std::fs::metadata(&source).unwrap().modified().unwrap(), stamp);
        assert_eq!(compile_cached(&dev.ctx, &source).unwrap().server_js, "third  edit");
        // A timestamp-only save should keep the compiled value when the content is identical.
        let current = compile_cached(&dev.ctx, &source).unwrap();
        std::fs::File::options().write(true).open(&source).unwrap()
            .set_modified(stamp + std::time::Duration::from_secs(10)).unwrap();
        assert!(Arc::ptr_eq(&current, &compile_cached(&dev.ctx, &source).unwrap()));
        drop(dev);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn concurrent_compile_invalidation_and_owned_socket_cleanup() {
        let root = std::env::temp_dir().join(format!("brain-dev-lifecycle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let source = root.join("view.brain");
        let dependency = root.join("tokens.cssbrain");
        std::fs::write(&source, "first").unwrap();
        std::fs::write(&dependency, "tokens").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let dep = dependency.clone();
        let compiler: CompileFn = Arc::new(move |src, _| {
            count.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5));
            Ok(Compiled { server: Default::default(),
                js: "export default function(){}".into(),
                css: String::new(),
                deps: vec![dep.clone()],
                server_js: src.into(),
            })
        });
        let opts = Options {
            root: root.clone(),
            host: "127.0.0.1".into(),
            port: 0,
            head_prepend: String::new(),
            runtime: vec![],
            env: None,
            base: "/".into(),
            diagnose: None,
            problems: None,
            upstream: None,
            mock: None,
        };
        let mut dev = start_dev(opts, compiler, false, None).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let ctx = &dev.ctx;
                let source = &source;
                scope.spawn(move || {
                    assert!(compile_cached(ctx, source).is_ok());
                });
            }
        });
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "identical concurrent requests compile once"
        );
        on_change(&dev.ctx, &dependency);
        assert!(compile_cached(&dev.ctx, &source).is_ok());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a dependency edit invalidates an unchanged parent"
        );
        *dev.ctx.sidecar.lock().unwrap() = Some(crate::load::Sidecar::spawn(&root, &[]).unwrap());
        std::fs::write(&source, "second").unwrap();
        on_change(&dev.ctx, &source);
        assert!(
            dev.ctx.sidecar.lock().unwrap().is_none(),
            "server edits discard old Node module versions"
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let (reader, _) = listener.accept().unwrap();
        dev.ctx
            .clients
            .lock()
            .unwrap()
            .push(reader.try_clone().unwrap());
        dev.stop();
        let mut data = Vec::new();
        assert!(
            client.read_to_end(&mut data).is_ok(),
            "stop must shut down the reader's shared socket, not just drop its clone"
        );
        drop(reader);
        drop(dev);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn waiting_for_a_running_job_has_its_own_deadline() {
        let job = Mutex::new(());
        let held = job.lock().unwrap();
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let start = Instant::now();
                    assert!(lock_for(&job, Duration::from_millis(30)).is_none());
                    assert!(start.elapsed() < Duration::from_secs(1));
                })
                .join()
                .unwrap();
        });
        drop(held);
        assert!(lock_for(&job, Duration::from_millis(30)).is_some());
    }

    #[test]
    fn a_server_only_module_requested_by_the_browser_fails_with_its_reason() {
        // A `<meta server>` module has no client code. A browser test that imports it got an empty module and the
        // engine's "does not provide an export named 'default'"; it must say the module is server-only and what to do.
        let root=std::env::temp_dir().join(format!("brain-server-only-{}-{}",std::process::id(),now_ms()));
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("lib/rank.jsbrain"),"server").unwrap();
        std::fs::write(root.join("lib/plain.jsbrain"),"client").unwrap();
        let root=root.canonicalize().unwrap();
        let compiler:CompileFn=Arc::new(|source,_|Ok(if source=="server"{Compiled{server:Default::default(),js:String::new(),css:String::new(),deps:Vec::new(),server_js:"export default {}".into()}}else{Compiled{server:Default::default(),js:"export default {}".into(),css:String::new(),deps:Vec::new(),server_js:String::new()}}));
        let options=Options{root:root.clone(),host:"127.0.0.1".into(),port:0,head_prepend:String::new(),runtime:Vec::new(),env:None,base:"/".into(),diagnose:None,problems:None,upstream:None,mock:None};
        let mut dev=start_dev(options,compiler,false,None).unwrap();
        let request=|address,path:&str|{let mut socket=TcpStream::connect(address).unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();socket.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();let mut bytes=Vec::new();socket.read_to_end(&mut bytes).unwrap();String::from_utf8(bytes).unwrap()};
        let server_only=request(dev.addr,"/lib/rank.jsbrain");
        assert!(server_only.starts_with("HTTP/1.1 200"),"{server_only}");
        assert!(server_only.contains("export default undefined"),"{server_only}");
        assert!(server_only.contains("const file=\"lib/rank.jsbrain\"")&&server_only.contains("is a <meta server> module: it runs only on the server"),"{server_only}");
        assert!(server_only.contains("stub("),"{server_only}");
        assert!(!server_only.contains("export default {}"),"server code must never reach the browser: {server_only}");
        let plain=request(dev.addr,"/lib/plain.jsbrain");
        assert!(plain.contains("export default {}")&&!plain.contains("<meta server>"),"{plain}");
        dev.stop();drop(dev);std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn durable_agent_feedback_precedes_http_fallback_and_websocket_publication() {
        let root=std::env::temp_dir().join(format!("brain-feedback-order-{}-{}",std::process::id(),now_ms()));
        std::fs::create_dir_all(root.join("comp")).unwrap();
        std::fs::write(root.join("comp/example.brain"),"bad").unwrap();
        let root=root.canonicalize().unwrap();
        let sockets=TcpListener::bind("127.0.0.1:0").unwrap();
        let mut browser=TcpStream::connect(sockets.local_addr().unwrap()).unwrap();
        browser.set_read_timeout(Some(Duration::from_millis(25))).unwrap();
        let (outgoing,_)=sockets.accept().unwrap();
        let notification_probe=Mutex::new(browser.try_clone().unwrap());
        let checked=Arc::new(AtomicUsize::new(0));let marker=checked.clone();let observed_root=root.clone();
        let callback:EventFn=Arc::new(move|message|{
            if message.contains("\"type\":\"compiler-feedback\""){
                let persisted=crate::feedback::poll(&observed_root,None,0,64).unwrap();
                assert!(matches!(persisted.get("events"),Some(brain_lex::Json::Arr(events))if !events.is_empty()));
                marker.fetch_add(1,Ordering::SeqCst);
            }
            if message=="{\"type\":\"test-order\"}" {
                let mut byte=[0u8];let error=notification_probe.lock().unwrap().read(&mut byte).unwrap_err();
                assert!(matches!(error.kind(),io::ErrorKind::WouldBlock|io::ErrorKind::TimedOut),"websocket data reached the browser before the agent callback");
            }
        });
        let compiler:CompileFn=Arc::new(|source,_|if source=="bad"{Err(vec!["{\"code\":\"B0201\",\"file\":\"comp/example.brain\",\"line\":1,\"col\":1,\"msg\":\"private-diagnostic-value\",\"fix\":\"private-source-value\"}".into()])}else{Ok(Compiled{server:Default::default(),js:"export default function(){}".into(),css:String::new(),deps:Vec::new(),server_js:String::new()})});
        let options=Options{root:root.clone(),host:"127.0.0.1".into(),port:0,head_prepend:String::new(),runtime:Vec::new(),env:None,base:"/".into(),diagnose:None,problems:None,upstream:None,mock:None};
        let mut dev=start_dev(options,compiler,false,Some(callback)).unwrap();
        let request=|address|{let mut socket=TcpStream::connect(address).unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();socket.write_all(b"GET /comp/example.brain HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();let mut bytes=Vec::new();socket.read_to_end(&mut bytes).unwrap();String::from_utf8(bytes).unwrap()};
        let failed=request(dev.addr);assert!(failed.contains("BrainCompileError"));assert_eq!(checked.load(Ordering::SeqCst),1);
        let feedback=crate::feedback::poll(&root,None,0,64).unwrap().stringify();assert!(!feedback.contains("private-diagnostic-value"));assert!(!feedback.contains("private-source-value"));
        dev.ctx.clients.lock().unwrap().push(outgoing);
        broadcast_ev(&dev.ctx,"{\"type\":\"test-order\"}");
        let mut frame=[0u8;128];assert!(browser.read(&mut frame).unwrap()>0);
        std::fs::write(root.join("comp/example.brain"),"fixed").unwrap();let fixed=request(dev.addr);assert!(fixed.contains("export default function"));assert_eq!(checked.load(Ordering::SeqCst),2);
        dev.stop();drop(dev);drop(browser);std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn brain_json_server_body_limit_and_expect_100_continue() {
        let root = std::env::temp_dir().join(format!("brain-dev-body-limit-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("brain.json"), r#"{"name":"x","server":{"maxBodyKB":2}}"#).unwrap();
        let compiler: CompileFn = Arc::new(|_, _| Ok(Compiled { server: Default::default(), js: String::new(), css: String::new(), deps: vec![], server_js: String::new() }));
        let opts = Options { root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(), runtime: vec![], env: None, base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None };
        let mut dev = start_dev(opts, compiler, false, None).unwrap();
        let connect = || { let s = TcpStream::connect(dev.addr).unwrap(); s.set_read_timeout(Some(Duration::from_secs(3))).unwrap(); s };
        // within the limit: the interim 100 Continue arrives before the client sends the body, then the final response
        let mut s = connect();
        s.write_all(b"POST /_r/h1/save HTTP/1.1\r\nHost: localhost\r\nExpect: 100-continue\r\nContent-Length: 1024\r\nConnection: close\r\n\r\n").unwrap();
        let mut interim = [0u8; 25];
        s.read_exact(&mut interim).unwrap();
        assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
        s.write_all(&[b'x'; 1024]).unwrap();
        let mut rest = String::new();
        s.read_to_string(&mut rest).unwrap();
        assert!(rest.starts_with("HTTP/1.1 403"), "{rest}"); // the RPC handler answered: no X-Brain-RPC header
        // past the limit: 413 at once (no 100 Continue), and the body names the setting to raise
        let mut s = connect();
        s.write_all(b"POST /_r/h1/save HTTP/1.1\r\nHost: localhost\r\nExpect: 100-continue\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 413"), "{out}");
        assert!(out.contains("Body larger than 2 KB; raise brain.json server.maxBodyKB"), "{out}");
        dev.stop();
        drop(dev);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_native_backend_is_proxied_same_origin_and_runtime_routes_stay_local(){
        let root=std::env::temp_dir().join(format!("brain-native-dev-proxy-{}-{}",std::process::id(),now_ms()));std::fs::create_dir_all(&root).unwrap();
        let backend=TcpListener::bind("127.0.0.1:0").unwrap();let backend_url=format!("http://{}",backend.local_addr().unwrap());
        let authority=std::thread::spawn(move||{let (mut socket,_)=backend.accept().unwrap();socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();let request=http::read_request(socket.try_clone().unwrap()).unwrap();assert_eq!(request.path,"/_brain/auth/me");assert_eq!(request.header("x-forwarded-for"),Some("untrusted, 127.0.0.1"));http::respond(&mut socket,200,"application/json",b"{\"user\":null}",false).unwrap();});
        let env:EnvLoader=Arc::new(move||vec![("BRAIN_BACKEND_URL".into(),backend_url.clone())]);
        let compiler:CompileFn=Arc::new(|_,_|panic!("native API/runtime routing must not compile application source"));
        let opts=Options{root:root.clone(),host:"127.0.0.1".into(),port:0,head_prepend:String::new(),runtime:vec![("brain.js".into(),"export const localRuntime=true".into())],env:Some(env),base:"/".into(),diagnose:None,problems:None,upstream:None,mock:None};
        let mut dev=start_dev(opts,compiler,false,None).unwrap();
        let request=|path:&str,extra:&str|{let mut socket=TcpStream::connect(dev.addr).unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();socket.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n{extra}Connection: close\r\n\r\n").as_bytes()).unwrap();let mut bytes=Vec::new();socket.read_to_end(&mut bytes).unwrap();String::from_utf8(bytes).unwrap()};
        let response=request("/_brain/auth/me","X-Forwarded-For: untrusted\r\n");assert!(response.starts_with("HTTP/1.1 200"));assert!(response.ends_with("{\"user\":null}"));assert!(response.to_ascii_lowercase().contains("x-request-id:"));authority.join().unwrap();
        let runtime=request("/@brain/brain.js","");assert!(runtime.starts_with("HTTP/1.1 200"));assert!(runtime.contains("localRuntime"));
        let rejected=request("/api/items","Content-Length: 1048577\r\n");assert!(rejected.starts_with("HTTP/1.1 413"));
        dev.stop();drop(dev);std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dev_proxy_forwards_an_api_key_so_the_backend_can_identify_the_caller(){
        let root=std::env::temp_dir().join(format!("brain-native-dev-key-{}-{}",std::process::id(),now_ms()));std::fs::create_dir_all(&root).unwrap();
        let backend=TcpListener::bind("127.0.0.1:0").unwrap();let backend_url=format!("http://{}",backend.local_addr().unwrap());
        let authority=std::thread::spawn(move||{let (mut socket,_)=backend.accept().unwrap();socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();let request=http::read_request(socket.try_clone().unwrap()).unwrap();assert_eq!(request.path,"/api/notes");assert_eq!(request.header("x-api-key"),Some("bk_live_sample"));assert_eq!(request.header("origin"),None);http::respond(&mut socket,200,"application/json",b"{\"items\":[]}",false).unwrap();});
        let env:EnvLoader=Arc::new(move||vec![("BRAIN_BACKEND_URL".into(),backend_url.clone())]);
        let compiler:CompileFn=Arc::new(|_,_|panic!("native API routing must not compile application source"));
        let opts=Options{root:root.clone(),host:"127.0.0.1".into(),port:0,head_prepend:String::new(),runtime:vec![],env:Some(env),base:"/".into(),diagnose:None,problems:None,upstream:None,mock:None};
        let mut dev=start_dev(opts,compiler,false,None).unwrap();
        let mut socket=TcpStream::connect(dev.addr).unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        socket.write_all(b"GET /api/notes HTTP/1.1\r\nHost: localhost\r\nX-API-Key: bk_live_sample\r\nConnection: close\r\n\r\n").unwrap();
        let mut bytes=Vec::new();socket.read_to_end(&mut bytes).unwrap();
        assert!(String::from_utf8(bytes).unwrap().starts_with("HTTP/1.1 200"));authority.join().unwrap();
        dev.stop();drop(dev);std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn closed_hmr_sockets_leave_the_client_registry_without_a_broadcast() {
        let root=std::env::temp_dir().join(format!("brain-hmr-registry-{}-{}",std::process::id(),now_ms()));std::fs::create_dir_all(&root).unwrap();
        let compiler:CompileFn=Arc::new(|_,_|panic!("HMR sockets never compile application source"));
        let opts=Options{root:root.clone(),host:"127.0.0.1".into(),port:0,head_prepend:String::new(),runtime:Vec::new(),env:None,base:"/".into(),diagnose:None,problems:None,upstream:None,mock:None};
        let mut dev=start_dev(opts,compiler,false,None).unwrap();
        for _ in 0..8 {
            let mut socket=TcpStream::connect(dev.addr).unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            socket.write_all(b"GET /@brain/hmr HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").unwrap(); // gitleaks:allow -- RFC WebSocket example nonce, not a credential
            let mut head=[0u8;12];socket.read_exact(&mut head).unwrap();assert_eq!(&head,b"HTTP/1.1 101");
            // Masked client close frame with an empty payload; the server answers and ends the drain loop.
            socket.write_all(&[0x88,0x80,0,0,0,0]).unwrap();
            let mut rest=Vec::new();let _=socket.read_to_end(&mut rest);
        }
        let deadline=Instant::now()+Duration::from_secs(3);
        while !dev.ctx.clients.lock().unwrap().is_empty() && Instant::now()<deadline {std::thread::sleep(Duration::from_millis(10));}
        let registered=dev.ctx.clients.lock().unwrap().len();
        assert_eq!(registered,0,"closed HMR connections must not keep their socket in the broadcast registry");
        dev.stop();drop(dev);std::fs::remove_dir_all(root).unwrap();
    }

}

#[cfg(test)]
#[path = "value_module_diagnostic_tests.rs"]
mod value_module_diagnostic_tests;
