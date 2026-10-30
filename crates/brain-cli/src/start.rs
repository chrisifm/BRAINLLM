//! `brain start|stop|reload|status` (INTERNALS §34): the production process manager. One way for the model:
//! `brain build` then `brain start`. Static apps are served from Rust (brain-serve's `prod`); apps with
//! `dist/server.mjs` run under a supervisor that binds the port once and starts N Node children sharing that socket
//! as fd 3 (`BRAIN_LISTEN_FD=3`). Reload is zero-downtime (new children ready before the old ones get SIGTERM).
//! Control goes through `.brain/run/brain.sock` (a short `/tmp` path when that one is too long); logs to `logs/app.log` (10 MB or daily, 7 kept).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub struct Opts {
    pub host: String,
    pub port: u16,
    pub instances: usize,
    pub max_rss_kb: u64,
    pub env: String,
    /// Test hook: health and RSS intervals in ms (production: 10 s and 5 s).
    pub health_ms: u64,
    pub rss_ms: u64,
    /// First restart delay (production 100 ms, doubling up to 30 s).
    pub backoff_ms: u64,
}

pub fn parse_mem(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_uppercase();
    let (n, mul) = match s.chars().last()? {
        'G' => (&s[..s.len() - 1], 1024 * 1024),
        'M' => (&s[..s.len() - 1], 1024),
        'K' => (&s[..s.len() - 1], 1),
        _ => (&s[..], 1024),
    };
    n.trim().parse::<u64>().ok().map(|v| v * mul)
}

fn run_dir(root: &Path) -> PathBuf {
    root.join(".brain/run")
}

/// The control socket: `.brain/run/brain.sock`, or `/tmp/brain-<hash of the app path>.sock` when that path would not
/// fit in a Unix socket address (104 bytes on macOS, 108 on Linux; deep app paths and macOS temp dirs exceed it).
fn sock_path(root: &Path) -> PathBuf {
    let p = run_dir(root).join("brain.sock");
    if p.as_os_str().len() < 100 {
        return p;
    }
    let abs = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    PathBuf::from(format!("/tmp/brain-{}.sock", brain_pack::content_hash(&abs.to_string_lossy())))
}

// ---------- signals (handlers only touch atomics: async-signal-safe) ----------
static SIG_STOP: AtomicBool = AtomicBool::new(false);
static SIG_RELOAD: AtomicBool = AtomicBool::new(false);

extern "C" fn on_stop(_: libc::c_int) {
    SIG_STOP.store(true, Ordering::SeqCst);
}
extern "C" fn on_reload(_: libc::c_int) {
    SIG_RELOAD.store(true, Ordering::SeqCst);
}

fn install_signals() {
    // SAFETY: plain C handlers that only store to atomics
    unsafe {
        libc::signal(libc::SIGTERM, on_stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_reload as *const () as libc::sighandler_t);
    }
}

// ---------- logs ----------
struct Log {
    dir: PathBuf,
    file: Option<std::fs::File>,
    day: u64,
    tty: bool,
}

impl Log {
    fn open(root: &Path, tty: bool) -> Log {
        let dir = root.join("logs");
        let _ = std::fs::create_dir_all(&dir);
        let mut l = Log { dir, file: None, day: today(), tty };
        l.file = std::fs::OpenOptions::new().create(true).append(true).open(l.dir.join("app.log")).ok();
        l
    }
    fn line(&mut self, s: &str) {
        let size = std::fs::metadata(self.dir.join("app.log")).map_or(0, |m| m.len());
        if size > 10 * 1024 * 1024 || today() != self.day {
            self.rotate();
        }
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{s}");
        }
        if self.tty {
            eprintln!("{s}");
        }
    }
    fn rotate(&mut self) {
        self.file = None;
        for i in (1..7).rev() {
            let _ = std::fs::rename(self.dir.join(format!("app.log.{i}")), self.dir.join(format!("app.log.{}", i + 1)));
        }
        let _ = std::fs::rename(self.dir.join("app.log"), self.dir.join("app.log.1"));
        let _ = std::fs::remove_file(self.dir.join("app.log.8"));
        self.day = today();
        self.file = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("app.log")).ok();
    }
}

fn today() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400)
}

fn now_s() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn event(log: &Mutex<Log>, what: &str, extra: &str) {
    log.lock().unwrap().line(&format!("{{\"t\":{},\"src\":\"brain-start\",\"event\":{}{extra}}}", now_s(), brain_serve::json_str(what)));
}

// ---------- children ----------
struct Kid {
    child: Child,
    started: Instant,
    ready: Arc<AtomicBool>,
    /// Why the supervisor itself stopped this child (`memory`, `unhealthy`): such exits are not crashes.
    why: Option<&'static str>,
}

fn spawn_child(root: &Path, fd: i32, vars: &[(String, String)], log: &Arc<Mutex<Log>>) -> std::io::Result<Kid> {
    let node = std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into());
    let mut cmd = Command::new(node);
    // the worker exits when its supervisor is gone (killed with SIGKILL, OOM): its parent pid changes. Otherwise an
    // orphan keeps the port and the next `brain start` cannot bind. Comparing with the pid it started with also works
    // when the supervisor itself is pid 1 (a container).
    cmd.arg("--import").arg("data:text/javascript,const%20p%3Dprocess.ppid%3BsetInterval(()%3D%3E%7Bif(process.ppid!%3D%3Dp)process.exit(0)%7D%2C500).unref()");
    cmd.arg(root.join("dist/server.mjs"))
        .current_dir(root.join("dist"))
        .envs(vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .env("NODE_ENV", "production")
        .env("BRAIN_LISTEN_FD", "3")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: only async-signal-safe calls between fork and exec: dup2 makes the listening socket fd 3 in the child
    // (dup2 clears FD_CLOEXEC on the new descriptor, so it survives exec)
    unsafe {
        cmd.pre_exec(move || {
            if fd == 3 {
                // already fd 3: dup2(3, 3) is a no-op that keeps FD_CLOEXEC, so clear the flag explicitly
                if libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::dup2(fd, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    let ready = Arc::new(AtomicBool::new(false));
    let (out, err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    for (pipe, is_out) in [(Box::new(out) as Box<dyn Read + Send>, true), (Box::new(err) as Box<dyn Read + Send>, false)] {
        let (log, ready) = (log.clone(), ready.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if is_out && line.starts_with("brain:ready") {
                    ready.store(true, Ordering::SeqCst);
                }
                log.lock().unwrap().line(&line);
            }
        });
    }
    Ok(Kid { child, started: Instant::now(), ready, why: None })
}

fn term(k: &mut Kid) {
    // SAFETY: plain kill(2) on our own child
    unsafe {
        libc::kill(k.child.id() as i32, libc::SIGTERM);
    }
}

/// Waits until every child printed `brain:ready`; fails as soon as one of them exits, or at `timeout`.
fn wait_ready(kids: &mut [Kid], timeout: Duration) -> Result<(), String> {
    let t = Instant::now();
    while t.elapsed() < timeout {
        for k in kids.iter_mut() {
            if let Ok(Some(st)) = k.child.try_wait() {
                return Err(format!("a new instance exited before it was ready ({})", exit_text(&st)));
            }
        }
        if kids.iter().all(|k| k.ready.load(Ordering::SeqCst)) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!("the new instances did not print `brain:ready` within {} s", timeout.as_secs()))
}

fn exit_text(st: &std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (st.code(), st.signal()) {
        (Some(c), _) => format!("exit code {c}"),
        (None, Some(sig)) => format!("killed by signal {sig}"),
        _ => "unknown exit".into(),
    }
}

/// The last `n` lines of logs/app.log, for stderr when the supervisor gives up.
fn log_tail(root: &Path, n: usize) -> String {
    let all = std::fs::read_to_string(root.join("logs/app.log")).unwrap_or_default();
    let lines: Vec<&str> = all.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Sleeps `d` in small steps, returning early when a stop was requested.
fn nap(d: Duration, stop: &AtomicBool) {
    let t = Instant::now();
    while t.elapsed() < d && !stop.load(Ordering::SeqCst) && !SIG_STOP.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A reload request: `brain reload` waits on `result`, so a failed reload exits 1 (§34).
#[derive(Default)]
struct Reload {
    pending: AtomicBool,
    result: Mutex<Option<Result<(), String>>>,
}

impl Reload {
    fn finish(&self, r: Result<(), String>) {
        *self.result.lock().unwrap() = Some(r);
    }
}

fn rss_kb(pid: u32) -> Option<u64> {
    let o = Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).stdin(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&o.stdout).trim().parse().ok()
}

fn healthy(host: &str, port: u16) -> bool {
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&format!("{host}:{port}").parse().unwrap_or_else(|_| ([127, 0, 0, 1], port).into()), Duration::from_secs(2)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.write_all(format!("GET /_health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes());
    let mut b = String::new();
    let _ = s.read_to_string(&mut b);
    b.starts_with("HTTP/1.1 200")
}

// ---------- status shared with the control socket ----------
#[derive(Default)]
struct Status {
    mode: String,
    pids: Vec<u32>,
    started: u64,
    restarts: u64,
    memory_restarts: u64,
    last_exit: Option<(i32, String)>,
    rss_kb: Vec<u64>,
    build: String,
    url: String,
}

impl Status {
    fn json(&self) -> String {
        use brain_mcp::Json;
        Json::obj([
            ("ok", Json::Bool(true)),
            ("mode", Json::str(&self.mode)),
            ("pid", Json::Num(std::process::id() as f64)),
            ("url", Json::str(&self.url)),
            ("build", Json::str(&self.build)),
            ("uptimeS", Json::Num(now_s().saturating_sub(self.started) as f64)),
            ("instances", Json::Arr(self.pids.iter().zip(self.rss_kb.iter().chain(std::iter::repeat(&0))).map(|(p, r)| Json::obj([("pid", Json::Num(*p as f64)), ("rssKB", Json::Num(*r as f64))])).collect())),
            ("restarts", Json::Num(self.restarts as f64)),
            ("memoryRestarts", Json::Num(self.memory_restarts as f64)),
            (
                "lastExit",
                match &self.last_exit {
                    Some((c, why)) => Json::obj([("code", Json::Num(*c as f64)), ("reason", Json::str(why))]),
                    None => Json::Null,
                },
            ),
        ])
        .stringify()
    }
}

/// Control socket: one line per connection (`status` | `reload` | `stop`), one JSON line back.
fn control(path: PathBuf, status: Arc<Mutex<Status>>, stop: Arc<AtomicBool>, reload: Arc<Reload>) -> std::io::Result<()> {
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path)?;
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut s = s;
            let mut line = String::new();
            let _ = BufReader::new(&s).read_line(&mut line);
            let reply = match line.trim() {
                "status" => status.lock().unwrap().json(),
                "reload" => {
                    // answer only once the supervisor finished it: ok with the new build, or the reason it failed
                    *reload.result.lock().unwrap() = None;
                    reload.pending.store(true, Ordering::SeqCst);
                    let t = Instant::now();
                    let r = loop {
                        if let Some(r) = reload.result.lock().unwrap().take() {
                            break r;
                        }
                        if t.elapsed() > Duration::from_secs(38) {
                            break Err("the reload did not finish in 38 s".into());
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    };
                    match r {
                        Ok(()) => format!("{{\"ok\":true,\"reload\":\"done\",\"build\":{}}}", brain_serve::json_str(&status.lock().unwrap().build)),
                        Err(e) => format!("{{\"ok\":false,\"error\":{}}}", brain_serve::json_str(&format!("reload failed: {e}; the old instances keep serving; see logs/app.log"))),
                    }
                }
                "stop" => {
                    stop.store(true, Ordering::SeqCst);
                    "{\"ok\":true,\"stop\":\"started\"}".into()
                }
                other => format!("{{\"ok\":false,\"error\":{}}}", brain_serve::json_str(&format!("unknown command `{other}`"))),
            };
            let _ = writeln!(s, "{reply}");
        }
    });
    Ok(())
}

/// Sends one command to a running `brain start` and returns its JSON reply.
pub fn send(root: &Path, cmd: &str) -> Result<String, String> {
    let p = sock_path(root);
    let mut s = UnixStream::connect(&p).map_err(|_| "no `brain start` is running for this app (.brain/run/brain.sock is missing); fix: run `brain start`".to_string())?;
    writeln!(s, "{cmd}").map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(40)));
    let mut out = String::new();
    BufReader::new(s).read_line(&mut out).map_err(|e| e.to_string())?;
    Ok(out.trim().to_string())
}

/// Runs in the foreground until stopped. Exit codes: 0 clean stop · 1 crash loop or never ready.
pub fn start(root: &Path, o: &Opts) -> Result<i32, String> {
    let dist = root.join("dist");
    if !["index.html", "index.brain", "server.mjs"].iter().any(|f| dist.join(f).is_file()) {
        return Err("dist/ has no build; fix: run `brain build` first".into());
    }
    let listener = TcpListener::bind((o.host.as_str(), o.port)).map_err(|e| format!("cannot listen on {}:{}: {e}; fix: pick another --port", o.host, o.port))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    // Resolve the selected environment once, before publishing readiness or control files.
    // A compiled manifest enables sockets; the presence of a helper never enables them.
    let static_gateway = if !dist.join("server.mjs").is_file() {
        let target = crate::env::value(root, &o.env, "BRAIN_BACKEND_URL");
        Some(brain_serve::prod::StaticGateway::load(&dist, &brain_pack::read_base(root), target.as_deref())
            .map_err(|error| format!("Cannot initialize the production static gateway: {error}"))?)
    } else {
        None
    };
    let rd = run_dir(root);
    std::fs::create_dir_all(&rd).map_err(|e| e.to_string())?;
    std::fs::write(rd.join("brain.pid"), std::process::id().to_string()).map_err(|e| e.to_string())?;
    let tty = { use std::io::IsTerminal; std::io::stderr().is_terminal() };
    let log = Arc::new(Mutex::new(Log::open(root, tty)));
    let url = format!("http://{}:{port}{}", o.host, brain_pack::read_base(root).trim_end_matches('/'));
    let stop = Arc::new(AtomicBool::new(false));
    let reload = Arc::new(Reload::default());
    let build = brain_serve::prod::signature(&dist).unwrap_or_default();
    let status = Arc::new(Mutex::new(Status { started: now_s(), build: build.clone(), url: url.clone(), ..Default::default() }));
    install_signals();
    control(sock_path(root), status.clone(), stop.clone(), reload.clone()).map_err(|e| e.to_string())?;
    println!("brain start → {url}  (build {build})");

    let code = if let Some(gateway) = static_gateway {
        // static: Rust serves dist/
        status.lock().unwrap().mode = "static".into();
        event(&log, "ready", &format!(",\"mode\":\"static\",\"url\":{}", brain_serve::json_str(&url)));
        let (st, rl) = (stop.clone(), reload.clone());
        let watcher = std::thread::spawn(move || {
            while !st.load(Ordering::SeqCst) {
                if SIG_STOP.load(Ordering::SeqCst) {
                    st.store(true, Ordering::SeqCst);
                }
                // nothing to reload: files are read per request
                SIG_RELOAD.store(false, Ordering::SeqCst);
                if rl.pending.swap(false, Ordering::SeqCst) {
                    rl.finish(Ok(()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let r = brain_serve::prod::serve_static_with_gateway(listener, gateway, stop.clone());
        stop.store(true, Ordering::SeqCst);
        let _ = watcher.join();
        r.map_err(|e| e.to_string())?;
        0
    } else {
        supervise(root, o, listener, &log, &status, &stop, &reload)
    };
    event(&log, "stopped", &format!(",\"code\":{code}"));
    let _ = std::fs::remove_file(sock_path(root));
    let _ = std::fs::remove_file(rd.join("brain.pid"));
    Ok(code)
}

fn supervise(root: &Path, o: &Opts, listener: TcpListener, log: &Arc<Mutex<Log>>, status: &Arc<Mutex<Status>>, stop: &Arc<AtomicBool>, reload: &Arc<Reload>) -> i32 {
    use std::os::fd::AsRawFd;
    let fd = listener.as_raw_fd();
    let vars = crate::env::select(root, &o.env);
    status.lock().unwrap().mode = "server".into();
    let mut kids: Vec<Kid> = Vec::new();
    for _ in 0..o.instances.max(1) {
        match spawn_child(root, fd, &vars, log) {
            Ok(k) => kids.push(k),
            Err(e) => {
                event(log, "spawn-failed", &format!(",\"error\":{}", brain_serve::json_str(&e.to_string())));
                eprintln!("brain start: cannot start node ({e}); fix: install Node.js or set BRAIN_NODE to its path");
                return 1;
            }
        }
    }
    // status lists the instances from the moment they exist (a status right after start must not be empty)
    status.lock().unwrap().pids = kids.iter().map(|k| k.child.id()).collect();
    let give_up = |kids: &mut Vec<Kid>, what: &str| {
        eprintln!("brain start: {what}; last 50 log lines (logs/app.log):\n{}", log_tail(root, 50));
        for k in kids.iter_mut() {
            let _ = k.child.kill();
            let _ = k.child.wait();
        }
        1
    };
    let restarts = AtomicU64::new(0);
    let mut recent: Vec<Instant> = Vec::new(); // crashes in the last 60 s
    let base = Duration::from_millis(o.backoff_ms.max(1));
    let mut backoff = base;
    let mut announced = false;
    let (mut last_health, mut last_rss, mut health_fails) = (Instant::now(), Instant::now(), 0u32);
    loop {
        if SIG_STOP.load(Ordering::SeqCst) {
            stop.store(true, Ordering::SeqCst);
        }
        if SIG_RELOAD.swap(false, Ordering::SeqCst) {
            reload.pending.store(true, Ordering::SeqCst);
        }
        if stop.load(Ordering::SeqCst) {
            for k in &mut kids {
                term(k);
            }
            let t = Instant::now();
            for k in &mut kids {
                while t.elapsed() < Duration::from_secs(12) && matches!(k.child.try_wait(), Ok(None)) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                let _ = k.child.kill();
                let _ = k.child.wait();
            }
            return 0;
        }
        // first readiness: announced once every child printed brain:ready; one that never does in 30 s ends the start
        if !announced && kids.iter().all(|k| k.ready.load(Ordering::SeqCst)) {
            announced = true;
            event(log, "ready", &format!(",\"instances\":{}", kids.len()));
        }
        if kids.iter().any(|k| !k.ready.load(Ordering::SeqCst) && k.started.elapsed() > Duration::from_secs(30)) {
            event(log, "never-ready", "");
            return give_up(&mut kids, "an instance never printed `brain:ready` in 30 s");
        }
        if reload.pending.swap(false, Ordering::SeqCst) {
            // zero downtime: new children on the same socket first, the old ones drain after they are ready
            let mut fresh = Vec::new();
            let mut err = None;
            for _ in 0..kids.len().max(1) {
                match spawn_child(root, fd, &crate::env::select(root, &o.env), log) {
                    Ok(k) => fresh.push(k),
                    Err(e) => err = Some(format!("cannot start node ({e})")),
                }
            }
            let r = match err {
                Some(e) => Err(e),
                None => wait_ready(&mut fresh, Duration::from_secs(30)),
            };
            match r {
                Ok(()) => {
                    for k in &mut kids {
                        term(k);
                    }
                    let old = std::mem::replace(&mut kids, fresh);
                    std::thread::spawn(move || {
                        for mut k in old {
                            let _ = k.child.wait();
                        }
                    });
                    status.lock().unwrap().build = brain_serve::prod::signature(&root.join("dist")).unwrap_or_default();
                    event(log, "reloaded", "");
                    reload.finish(Ok(()));
                }
                Err(e) => {
                    for k in &mut fresh {
                        let _ = k.child.kill();
                        let _ = k.child.wait();
                    }
                    event(log, "reload-failed", &format!(",\"reason\":{}", brain_serve::json_str(&format!("{e}; the old ones keep serving"))));
                    reload.finish(Err(e));
                }
            }
        }
        // exits → restart. Crashes (an exit code, or dying before ready) back off and count toward the crash loop
        // (10 in 60 s); a child killed from outside (kill -9, OOM killer), ready or not, or stopped by the supervisor
        // (memory, health) restarts at once and does not count.
        for i in 0..kids.len() {
            let Ok(Some(st)) = kids[i].child.try_wait() else { continue };
            let code = st.code().unwrap_or(-1);
            let was_ready = kids[i].ready.load(Ordering::SeqCst);
            let planned = kids[i].why;
            // a signal the supervisor did not send (kill -9, the OOM killer) is not a crash of the app's code, ready or not:
            // a worker killed between listening and printing brain:ready must not count toward the crash loop
            let external = st.code().is_none();
            let why: String = match planned {
                Some(w) => w.into(),
                None if external => format!("killed from outside ({})", exit_text(&st)),
                None if !was_ready => format!("died before ready ({})", exit_text(&st)),
                None => format!("crashed ({})", exit_text(&st)),
            };
            status.lock().unwrap().last_exit = Some((code, why.clone()));
            event(log, "exit", &format!(",\"code\":{code},\"reason\":{}", brain_serve::json_str(&why)));
            if planned.is_none() && !external {
                recent.retain(|t| t.elapsed() < Duration::from_secs(60));
                recent.push(Instant::now());
                if recent.len() >= 10 {
                    event(log, "crash-loop", ",\"crashes\":10,\"windowS\":60");
                    return give_up(&mut kids, "crash loop (10 crashes in 60 s)");
                }
                // a child that served (reached ready) earns a fresh backoff; one that never did doubles it
                if was_ready {
                    backoff = base;
                }
                nap(backoff, stop);
                // a child that never got ready is a broken build or config: cap at 2 s so the crash loop is reported
                // in ~11 s instead of ~1 min; one that served may be hitting a transient failure, so wait up to 30 s
                backoff = (backoff * 2).min(Duration::from_secs(if was_ready { 30 } else { 2 }));
            }
            if stop.load(Ordering::SeqCst) || SIG_STOP.load(Ordering::SeqCst) {
                break;
            }
            match spawn_child(root, fd, &vars, log) {
                Ok(k) => {
                    kids[i] = k;
                    restarts.fetch_add(1, Ordering::SeqCst);
                    let mut s = status.lock().unwrap();
                    if planned == Some("memory") {
                        s.memory_restarts += 1;
                    }
                    s.pids = kids.iter().map(|k| k.child.id()).collect();
                    s.restarts = restarts.load(Ordering::SeqCst);
                    drop(s);
                    event(log, "restart", &format!(",\"reason\":{}", brain_serve::json_str(&why)));
                }
                Err(e) => {
                    event(log, "spawn-failed", &format!(",\"error\":{}", brain_serve::json_str(&e.to_string())));
                    return give_up(&mut kids, &format!("cannot restart node ({e})"));
                }
            }
        }
        // liveness: /_health every health_ms; 3 failures in a row → restart every child
        if last_health.elapsed() >= Duration::from_millis(o.health_ms) && kids.iter().all(|k| k.ready.load(Ordering::SeqCst)) {
            last_health = Instant::now();
            if healthy(&o.host, listener.local_addr().map(|a| a.port()).unwrap_or(o.port)) {
                health_fails = 0;
            } else {
                health_fails += 1;
                if health_fails >= 3 {
                    event(log, "unhealthy", ",\"reason\":\"/_health failed 3 times\"");
                    health_fails = 0;
                    for k in &mut kids {
                        k.why = Some("unhealthy");
                        term(k);
                    }
                }
            }
        }
        // memory: RSS every rss_ms; over --max-memory → graceful restart of that child
        if last_rss.elapsed() >= Duration::from_millis(o.rss_ms) {
            last_rss = Instant::now();
            let mut rss = Vec::new();
            for k in &mut kids {
                let r = rss_kb(k.child.id()).unwrap_or(0);
                rss.push(r);
                if r > o.max_rss_kb && k.why.is_none() {
                    event(log, "memory", &format!(",\"rssKB\":{r},\"limitKB\":{}", o.max_rss_kb));
                    k.why = Some("memory");
                    term(k);
                }
            }
            status.lock().unwrap().rss_kb = rss;
        }
        {
            let mut s = status.lock().unwrap();
            s.pids = kids.iter().map(|k| k.child.id()).collect();
            s.restarts = restarts.load(Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `brain start --install`: a systemd unit (Linux) or a launchd plist (macOS) in .brain/run/, plus the one command
/// that enables it. Never runs sudo.
pub fn install(root: &Path, args: &str) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "app".into());
    let rd = run_dir(root);
    std::fs::create_dir_all(&rd).map_err(|e| e.to_string())?;
    let root_s = root.display().to_string();
    // services start with a bare PATH (launchd: /usr/bin:/bin:/usr/sbin:/sbin): put node's directory first
    let path = service_path();
    let node = std::env::var("BRAIN_NODE").ok();
    let argv: Vec<String> = std::iter::once(exe.display().to_string()).chain(["start".to_string()]).chain(args.split_whitespace().map(str::to_string)).collect();
    if cfg!(target_os = "macos") {
        let label = format!("dev.brainllm.{name}");
        let argv: String = argv.iter().map(|a| format!("    <string>{}</string>\n", xml(a))).collect();
        let mut env = format!("    <key>PATH</key><string>{}</string>\n", xml(&path));
        if let Some(n) = &node {
            env.push_str(&format!("    <key>BRAIN_NODE</key><string>{}</string>\n", xml(n)));
        }
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key><string>{}</string>\n  <key>ProgramArguments</key>\n  <array>\n{argv}  </array>\n  <key>EnvironmentVariables</key>\n  <dict>\n{env}  </dict>\n  <key>WorkingDirectory</key><string>{}</string>\n  <key>RunAtLoad</key><true/>\n  <key>KeepAlive</key><true/>\n</dict>\n</plist>\n",
            xml(&label),
            xml(&root_s)
        );
        let f = rd.join(format!("{label}.plist"));
        std::fs::write(&f, plist).map_err(|e| e.to_string())?;
        Ok(format!("wrote {}\nenable it with:\n  cp {} ~/Library/LaunchAgents/ && launchctl load -w ~/Library/LaunchAgents/{label}.plist", f.display(), f.display()))
    } else {
        // run as the user who installs it, never as root
        let user = std::env::var("USER").ok().filter(|u| !u.is_empty() && u != "root").or_else(|| {
            let o = Command::new("id").arg("-un").output().ok()?;
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|u| !u.is_empty())
        });
        let user_line = user.map(|u| format!("User={u}\n")).unwrap_or_default();
        let exec: Vec<String> = argv.iter().map(|a| sd_quote(a)).collect();
        let mut env = format!("Environment={}\n", sd_quote(&format!("PATH={path}")));
        if let Some(n) = &node {
            env.push_str(&format!("Environment={}\n", sd_quote(&format!("BRAIN_NODE={n}"))));
        }
        let unit = format!(
            "[Unit]\nDescription=BRAINLLM app {name}\nAfter=network.target\n\n[Service]\n{user_line}WorkingDirectory={}\n{env}ExecStart={}\nExecReload=/bin/kill -HUP $MAINPID\nRestart=always\nKillSignal=SIGTERM\nTimeoutStopSec=15\n\n[Install]\nWantedBy=multi-user.target\n",
            sd_quote(&root_s),
            exec.join(" ")
        );
        let f = rd.join(format!("brain-{name}.service"));
        std::fs::write(&f, unit).map_err(|e| e.to_string())?;
        Ok(format!("wrote {}\nenable it with:\n  sudo cp {} /etc/systemd/system/ && sudo systemctl enable --now brain-{name}", f.display(), f.display()))
    }
}

/// PATH for the service: the directory of the node that `brain start` would run now, then the current PATH.
fn service_path() -> String {
    let cur = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
    let node = std::env::var("BRAIN_NODE").ok().map(PathBuf::from).or_else(|| std::env::split_paths(&cur).map(|d| d.join("node")).find(|p| p.is_file()));
    let mut dirs: Vec<String> = Vec::new();
    if let Some(d) = node.as_deref().and_then(Path::parent) {
        dirs.push(d.display().to_string());
    }
    for d in cur.split(':').filter(|d| !d.is_empty()) {
        if !dirs.iter().any(|x| x == d) {
            dirs.push(d.to_string());
        }
    }
    dirs.join(":")
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// One systemd word: double-quoted, with `\`, `"` escaped and `$`/`%` doubled so they stay literal.
fn sd_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "$$").replace('%', "%%"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_sizes() {
        assert_eq!(parse_mem("300M"), Some(300 * 1024));
        assert_eq!(parse_mem("1G"), Some(1024 * 1024));
        assert_eq!(parse_mem("512k"), Some(512));
        assert_eq!(parse_mem("x"), None);
    }
}
