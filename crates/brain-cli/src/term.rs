//! Terminal output for `brain build` and `brain run` (INTERNALS §23, "Rich terminal output"): a colored, animated
//! view on an interactive terminal and exact plain lines everywhere else. Hand-written ANSI, no UI crate.
//!
//! The build never waits on the terminal: phases report through atomics and one thread redraws at most every 50 ms.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nothing but errors (`--progress=off`, `--quiet`).
    Off,
    /// One line per phase and a text summary: stable for agents, logs and CI.
    Plain,
    /// Animated bar, colors and charts.
    Rich,
}

/// Picks the mode. Plain whenever output is read by a machine: not a TTY, `CI`, `NO_COLOR`, `TERM=dumb`, `--json`,
/// under MCP (`BRAIN_MCP`); `--quiet` is off; `--progress=off|plain|rich` wins over everything.
pub fn detect(args: &[String], is_tty: bool, env: &dyn Fn(&str) -> Option<String>) -> Mode {
    if let Some(p) = args.iter().find_map(|a| a.strip_prefix("--progress=")) {
        return match p {
            "off" => Mode::Off,
            "rich" => Mode::Rich,
            _ => Mode::Plain,
        };
    }
    if args.iter().any(|a| a == "--quiet" || a == "-q") {
        return Mode::Off;
    }
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    if !is_tty || args.iter().any(|a| a == "--json") || set("CI") || set("NO_COLOR") || set("BRAIN_MCP") || env("TERM").as_deref() == Some("dumb") {
        return Mode::Plain;
    }
    Mode::Rich
}

/// Unicode glyphs when the locale is UTF-8, ASCII otherwise.
pub fn utf8(env: &dyn Fn(&str) -> Option<String>) -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"].iter().find_map(|k| env(k).filter(|v| !v.is_empty())).is_some_and(|v| {
        let v = v.to_ascii_lowercase();
        v.contains("utf-8") || v.contains("utf8")
    })
}

pub fn env_var(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

pub fn stderr_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}

// ---------- colors ----------
pub struct Paint {
    on: bool,
    pub utf8: bool,
}

impl Paint {
    pub fn new(mode: Mode, utf8: bool) -> Paint {
        Paint { on: mode == Mode::Rich, utf8 }
    }
    fn wrap(&self, code: &str, s: &str) -> String {
        if self.on { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
    }
    pub fn accent(&self, s: &str) -> String {
        self.wrap("38;5;33", s)
    }
    pub fn ok(&self, s: &str) -> String {
        self.wrap("32", s)
    }
    pub fn warn(&self, s: &str) -> String {
        self.wrap("33", s)
    }
    pub fn bad(&self, s: &str) -> String {
        self.wrap("31", s)
    }
    pub fn dim(&self, s: &str) -> String {
        self.wrap("2", s)
    }
    pub fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }
    pub fn css(&self, s: &str) -> String {
        self.wrap("38;5;170", s)
    }
    pub fn html(&self, s: &str) -> String {
        self.wrap("38;5;245", s)
    }
    pub fn check(&self) -> &'static str {
        if self.utf8 { "✓" } else { "ok" }
    }
    pub fn cross(&self) -> &'static str {
        if self.utf8 { "✗" } else { "x" }
    }
    pub fn diamond(&self) -> &'static str {
        if self.utf8 { "◆" } else { "*" }
    }
    pub fn block(&self) -> char {
        if self.utf8 { '█' } else { '#' }
    }
    pub fn shade(&self) -> char {
        if self.utf8 { '░' } else { '.' }
    }
}

pub fn header(p: &Paint, what: &str) -> String {
    format!("{} {} {}", p.accent(p.diamond()), p.bold(&format!("brain {}", env!("CARGO_PKG_VERSION"))), p.dim(what))
}

// ---------- phases ----------
struct State {
    phase: Mutex<Option<(String, Instant)>>,
    done: AtomicUsize,
    total: AtomicUsize,
    stop: AtomicBool,
    /// a phase line was printed by the reporter: the drawer must clear its line first
    lines: Mutex<Vec<String>>,
}

/// Phase reporter. `event(phase, done, total)` is cheap and thread-safe (the gzip workers call it).
pub struct Progress {
    mode: Mode,
    paint: Arc<Paint>,
    st: Arc<State>,
    drawer: Option<std::thread::JoinHandle<()>>,
    started: Instant,
    pub phases: Mutex<Vec<(String, Duration, usize)>>,
}

const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

impl Progress {
    pub fn new(mode: Mode, utf8: bool) -> Progress {
        let paint = Arc::new(Paint::new(mode, utf8));
        let st = Arc::new(State { phase: Mutex::new(None), done: AtomicUsize::new(0), total: AtomicUsize::new(0), stop: AtomicBool::new(false), lines: Mutex::new(Vec::new()) });
        let drawer = (mode == Mode::Rich).then(|| {
            let (st, paint) = (st.clone(), paint.clone());
            std::thread::spawn(move || {
                let mut frame = 0usize;
                let mut err = std::io::stderr();
                loop {
                    let stop = st.stop.load(Ordering::Relaxed);
                    let pending: Vec<String> = std::mem::take(&mut *st.lines.lock().unwrap());
                    let mut out = String::new();
                    for l in pending {
                        out.push_str(&format!("\r\x1b[2K{l}\n"));
                    }
                    if !stop {
                        if let Some((name, _)) = st.phase.lock().unwrap().as_ref() {
                            let (d, t) = (st.done.load(Ordering::Relaxed), st.total.load(Ordering::Relaxed));
                            let spin = if paint.utf8 { SPIN[frame % SPIN.len()] } else { ["|", "/", "-", "\\"][frame % 4] };
                            out.push_str(&format!("\r\x1b[2K{} {:<10} {} {}", paint.accent(spin), name, bar(&paint, d, t, 24), paint.dim(&if t > 0 { format!("{d}/{t}") } else { format!("{d}") })));
                        }
                    } else {
                        out.push_str("\r\x1b[2K");
                    }
                    let _ = err.write_all(out.as_bytes());
                    let _ = err.flush();
                    if stop {
                        break;
                    }
                    frame += 1;
                    // parked, not slept: end() unparks it so closing never waits for the next frame
                    std::thread::park_timeout(Duration::from_millis(50));
                }
            })
        });
        Progress { mode, paint, st, drawer, started: Instant::now(), phases: Mutex::new(Vec::new()) }
    }

    pub fn paint(&self) -> &Paint {
        &self.paint
    }

    /// Progress of `phase`: `done` of `total` items (0 = unknown). A new phase name closes the previous one.
    pub fn event(&self, phase: &str, done: usize, total: usize) {
        if self.mode == Mode::Off {
            return;
        }
        let mut cur = self.st.phase.lock().unwrap();
        if cur.as_ref().is_none_or(|(n, _)| n != phase) {
            if let Some((n, t0)) = cur.take() {
                self.finish_line(&n, t0.elapsed(), self.st.done.load(Ordering::Relaxed));
            }
            *cur = Some((phase.to_string(), Instant::now()));
            self.st.done.store(0, Ordering::Relaxed);
        }
        self.st.done.fetch_max(done, Ordering::Relaxed);
        self.st.total.store(total, Ordering::Relaxed);
    }

    fn finish_line(&self, name: &str, took: Duration, items: usize) {
        self.phases.lock().unwrap().push((name.to_string(), took, items));
        let p = &self.paint;
        let line = match self.mode {
            Mode::Rich => format!("{} {:<10} {}", p.ok(p.check()), name, p.dim(&ms(took))),
            _ if items > 0 => format!("phase {name} {} {items} items", ms(took)),
            _ => format!("phase {name} {}", ms(took)),
        };
        match self.mode {
            Mode::Rich => self.st.lines.lock().unwrap().push(line),
            Mode::Plain => eprintln!("{line}"),
            Mode::Off => {}
        }
    }

    /// Closes the running phase and stops the drawer (idempotent).
    pub fn end(&mut self) {
        if let Some((n, t0)) = self.st.phase.lock().unwrap().take() {
            self.finish_line(&n, t0.elapsed(), self.st.done.load(Ordering::Relaxed));
        }
        self.st.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.drawer.take() {
            h.thread().unpark();
            let _ = h.join();
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.end();
    }
}

/// Terminal width for the rich view.
fn columns() -> usize {
    if let Some(n) = env_var("COLUMNS").and_then(|c| c.parse().ok()) {
        return n;
    }
    std::process::Command::new("stty")
        .arg("size")
        .stdin(std::fs::File::open("/dev/tty").map(std::process::Stdio::from).unwrap_or(std::process::Stdio::null()))
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok()?.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(100)
}

/// A fix that lists alternatives (`… existing one: a, b, c`) keeps the 5 closest to the name in the message
/// (`token \`--x\``) by edit distance, then `+N more`. Rich mode only: plain and --json keep the full list.
pub fn short_fix(msg: &str, fix: &str) -> String {
    let Some((head, list)) = fix.split_once(": ").filter(|(_, l)| l.contains(", ")) else { return fix.to_string() };
    let wanted = msg.split('`').nth(1).unwrap_or("");
    let mut items: Vec<&str> = list.split(", ").map(str::trim).collect();
    if items.len() <= 5 {
        return fix.to_string();
    }
    items.sort_by_key(|x| (edit_distance(wanted, x), x.len()));
    format!("{head}: {} +{} more", items[..5].join(", "), items.len() - 5)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// `s` cut to `max` chars with an ellipsis (rich mode only: plain output is never shortened).
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max - 1).collect::<String>()) }
}

pub fn ms(d: Duration) -> String {
    let m = d.as_secs_f64() * 1000.0;
    if m < 10.0 { format!("{m:.1} ms") } else if m < 10_000.0 { format!("{m:.0} ms") } else { format!("{:.1} s", m / 1000.0) }
}

pub fn kb(bytes: usize) -> String {
    if bytes < 1024 { format!("{bytes} B") } else { format!("{:.1} KB", bytes as f64 / 1024.0) }
}

/// `▕████░░░░▏`-style bar of `width` cells; unknown total → a full dim bar.
fn bar(p: &Paint, done: usize, total: usize, width: usize) -> String {
    let fill = if total == 0 { 0 } else { (done.min(total) * width) / total };
    let a: String = std::iter::repeat_n(p.block(), fill).collect();
    let b: String = std::iter::repeat_n(p.shade(), width - fill).collect();
    format!("{}{}", p.accent(&a), p.dim(&b))
}

// ---------- build summary ----------
pub struct ViewSize {
    pub route: String,
    pub js_gz: usize,
    pub css_gz: usize,
    pub html_gz: usize,
}

pub struct Summary<'a> {
    pub views: &'a [ViewSize],
    /// per-view gz budget in bytes (`brain.json` budget.viewKB)
    pub budget: Option<usize>,
    pub written: usize,
    pub unchanged: usize,
    pub signature: Option<&'a str>,
    pub seo: Option<(u32, u32)>,
    pub took: Duration,
    pub warnings: usize,
}

/// The summary after a build: a stacked bar per view (rich) or one line per view (plain), then the totals.
pub fn summary(p: &Paint, s: &Summary) -> String {
    let mut o = String::new();
    let max = s.views.iter().map(|v| v.js_gz + v.css_gz + v.html_gz).max().unwrap_or(1).max(1);
    let wroute = s.views.iter().map(|v| v.route.len()).max().unwrap_or(1).clamp(1, 28);
    let total: usize = s.views.iter().map(|v| v.js_gz + v.css_gz + v.html_gz).sum();
    if p.on {
        o.push('\n');
        for v in s.views {
            let t = v.js_gz + v.css_gz + v.html_gz;
            let w = 30usize;
            let cells = |n: usize| if n == 0 { 0 } else { ((n * w) / max).max(1) };
            let (j, c, h) = (cells(v.js_gz), cells(v.css_gz), cells(v.html_gz));
            let seg = |n: usize| std::iter::repeat_n(p.block(), n).collect::<String>();
            let over = s.budget.is_some_and(|b| t > b);
            let size = if over { p.bad(&format!("{} over budget", kb(t))) } else { kb(t) };
            o.push_str(&format!("  {:<wroute$}  {}{}{}{}  {size}\n", v.route, p.accent(&seg(j)), p.css(&seg(c)), p.html(&seg(h)), " ".repeat(w.saturating_sub(j + c + h)),));
        }
        o.push_str(&format!("  {}  {} js  {} css  {} html\n", " ".repeat(wroute), p.accent(&p.block().to_string()), p.css(&p.block().to_string()), p.html(&p.block().to_string())));
    } else {
        for v in s.views {
            let t = v.js_gz + v.css_gz + v.html_gz;
            let over = if s.budget.is_some_and(|b| t > b) { " over-budget" } else { "" };
            o.push_str(&format!("page {} js {} css {} html {} total {}{over}\n", v.route, v.js_gz, v.css_gz, v.html_gz, t));
        }
    }
    let mut tail = vec![format!("{} pages", s.views.len()), format!("{} gz total", kb(total))];
    if let Some((lo, hi)) = s.seo {
        tail.push(format!("SEO {lo}–{hi}"));
    }
    tail.push(format!("{} written, {} unchanged", s.written, s.unchanged));
    if s.warnings > 0 {
        tail.push(p.warn(&format!("{} warning{}", s.warnings, if s.warnings == 1 { "" } else { "s" })));
    }
    if p.on {
        o.push_str(&format!("\n{} {}  {}\n", p.ok(p.check()), tail.join(p.dim(" · ").as_str()), p.dim(&ms(s.took))));
    } else {
        o.push_str(&format!("summary {} · {}\n", tail.join(" · "), ms(s.took)));
    }
    if let Some(sig) = s.signature {
        o.push_str(&format!("build {sig}\n"));
    }
    o
}

// ---------- dev panel ----------
/// Sparkline of rebuild times (last 20), colored by speed.
pub fn sparkline(p: &Paint, ms: &[f64]) -> String {
    let marks: &[char] = if p.utf8 { &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'] } else { &['_', '.', '-', '=', '#'] };
    let last: Vec<f64> = ms.iter().rev().take(20).rev().copied().collect();
    let max = last.iter().copied().fold(0.0f64, f64::max).max(0.001);
    last.iter()
        .map(|v| {
            let i = ((v / max) * (marks.len() - 1) as f64).round() as usize;
            let c = marks[i.min(marks.len() - 1)].to_string();
            if *v < 5.0 { p.ok(&c) } else if *v < 50.0 { p.warn(&c) } else { p.bad(&c) }
        })
        .collect()
}

/// `brain run` in rich mode: header and panel once, then one colored line per save and a status line kept at the
/// bottom (last rebuild, sparkline of the last 20, error and warning counters).
pub struct DevView {
    p: Paint,
    st: Mutex<DevState>,
    /// terminal width, read once at start (COLUMNS, else `stty size`, else 100)
    cols: usize,
}

#[derive(Default)]
struct DevState {
    times: Vec<f64>,
    /// file → (errors, warnings) of its last compile
    issues: std::collections::BTreeMap<String, (usize, usize)>,
}

impl DevView {
    pub fn new(utf8: bool) -> DevView {
        DevView { p: Paint::new(Mode::Rich, utf8), st: Mutex::default(), cols: columns() }
    }

    pub fn boot(&self, app: &str, url: &str, lan: Option<&str>, views: usize, took: Duration) {
        let p = &self.p;
        let mut o = format!("{}\n", header(p, &format!("run · {app}")));
        o.push_str(&format!("{} {}\n", bar(p, 1, 1, 16), p.dim(&format!("ready in {}", ms(took)))));
        o.push_str(&format!("  {}  {}\n", p.dim("Local"), p.accent(url)));
        o.push_str(&format!("  {}    {}\n", p.dim("LAN"), lan.map_or_else(|| p.dim("off (use --host 0.0.0.0)"), |l| p.accent(l))));
        o.push_str(&format!("  {}  {views}\n\n", p.dim("Views")));
        let mut err = std::io::stderr();
        let _ = err.write_all(o.as_bytes());
        let _ = err.write_all(self.status(&self.st.lock().unwrap()).as_bytes());
        let _ = err.flush();
    }

    fn status(&self, st: &DevState) -> String {
        let p = &self.p;
        let (e, w) = st.issues.values().fold((0, 0), |(a, b), (x, y)| (a + x, b + y));
        let last = st.times.last().map_or("—".to_string(), |t| format!("{t:.1} ms"));
        let errs = if e > 0 { p.bad(&format!("{e} errors")) } else { p.ok("0 errors") };
        let warns = if w > 0 { p.warn(&format!("{w} warnings")) } else { p.dim("0 warnings") };
        format!("\r\x1b[2K{} {}  {}  {} · {}", p.dim("last"), last, sparkline(p, &st.times), errs, warns)
    }

    /// `✗ CODE file:line:col msg` and the fix below it, colored by level.
    fn diag(&self, d: &brain_mcp::Json) -> String {
        let p = &self.p;
        let g = |k: &str| d.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
        let n = |k: &str| match d.get(k) {
            Some(brain_mcp::Json::Num(x)) => *x as usize,
            _ => 0,
        };
        let (mark, code) = match g("level").as_str() {
            "warning" => (p.warn("!"), p.warn(&g("code"))),
            "info" => (p.dim("i"), p.dim(&g("code"))),
            _ => (p.bad(p.cross()), p.bad(&g("code"))),
        };
        let fix = if g("fix").is_empty() { String::new() } else { format!("\n    {}", p.dim(&clip(&format!("fix: {}", short_fix(&g("msg"), &g("fix"))), self.cols.saturating_sub(4).max(40)))) };
        format!("{mark} {code} {}:{}:{} {}{fix}", g("file"), n("line"), n("col"), g("msg"))
    }

    /// A dev server event (`brain_serve::EventFn` JSON): update, error, diags, reload, client-error.
    pub fn event(&self, ev: &str) {
        let Ok(j) = brain_mcp::Json::parse(ev) else { return };
        let p = &self.p;
        let s = |k: &str| j.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
        let mut st = self.st.lock().unwrap();
        let line = match s("type").as_str() {
            "update" => {
                let path = s("path").trim_start_matches('/').to_string();
                let t = match j.get("ms") {
                    Some(brain_mcp::Json::Num(n)) => *n,
                    _ => 0.0,
                };
                st.times.push(t);
                if st.times.len() > 20 {
                    st.times.remove(0);
                }
                st.issues.remove(&path);
                let kind: Vec<&str> = [("js", "js"), ("css", "css")].iter().filter(|(k, _)| matches!(j.get(k), Some(brain_mcp::Json::Bool(true)))).map(|(_, n)| *n).collect();
                format!("{} {} {}{}", p.ok(p.check()), path, p.dim(&format!("{t:.1} ms")), p.dim(&format!(" · {}", kind.join("+"))))
            }
            // a compile error: its errors, and the file's warnings counted (not repeated: the next good save lists them)
            "error" => {
                let file = s("file");
                let ds = match j.get("diags") {
                    Some(brain_mcp::Json::Arr(ds)) => ds.clone(),
                    _ => Vec::new(),
                };
                let warns = match j.get("all") {
                    Some(brain_mcp::Json::Arr(a)) => a.iter().filter(|d| d.get("level").and_then(brain_mcp::Json::as_str) == Some("warning")).count(),
                    _ => 0,
                };
                if !file.is_empty() {
                    st.issues.insert(file, (0, warns));
                }
                let lines: Vec<String> = ds
                    .iter()
                    .map(|d| {
                        let f = d.get("file").and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
                        st.issues.entry(f).or_default().0 += 1;
                        self.diag(d)
                    })
                    .collect();
                lines.join("\n")
            }
            // a good compile: the file's warnings and info (none = the file is clean)
            "diags" => {
                let ds = match j.get("all") {
                    Some(brain_mcp::Json::Arr(ds)) => ds.clone(),
                    _ => Vec::new(),
                };
                let warns = ds.iter().filter(|d| d.get("level").and_then(brain_mcp::Json::as_str) == Some("warning")).count();
                st.issues.insert(s("file"), (0, warns));
                if ds.is_empty() {
                    let out = format!("\r\x1b[2K{}", self.status(&st));
                    let mut err = std::io::stderr();
                    let _ = err.write_all(out.as_bytes());
                    let _ = err.flush();
                    return;
                }
                ds.iter().map(|d| self.diag(d)).collect::<Vec<_>>().join("\n")
            }
            "reload" => format!("{} {} {}", p.accent("↻"), s("path").trim_start_matches('/'), p.dim("full reload")),
            "client-error" => format!("{} {}", p.bad(p.cross()), p.bad("browser error (see the overlay and brain_errors)")),
            _ => return,
        };
        let out = format!("\r\x1b[2K{line}\n{}", self.status(&st));
        let mut err = std::io::stderr();
        let _ = err.write_all(out.as_bytes());
        let _ = err.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let v: Vec<(String, String)> = pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        move |k| v.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone())
    }
    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_machine_condition_is_plain() {
        let none = env(&[]);
        assert_eq!(detect(&args(&[]), true, &none), Mode::Rich);
        assert_eq!(detect(&args(&[]), false, &none), Mode::Plain, "not a TTY");
        assert_eq!(detect(&args(&[]), true, &env(&[("CI", "true")])), Mode::Plain);
        assert_eq!(detect(&args(&[]), true, &env(&[("NO_COLOR", "1")])), Mode::Plain);
        assert_eq!(detect(&args(&[]), true, &env(&[("TERM", "dumb")])), Mode::Plain);
        assert_eq!(detect(&args(&[]), true, &env(&[("BRAIN_MCP", "1")])), Mode::Plain, "under MCP");
        assert_eq!(detect(&args(&["--json"]), true, &none), Mode::Plain);
        assert_eq!(detect(&args(&["--quiet"]), true, &none), Mode::Off);
        assert_eq!(detect(&args(&["--progress=rich"]), false, &env(&[("CI", "1")])), Mode::Rich, "the flag wins");
        assert_eq!(detect(&args(&["--progress=plain"]), true, &none), Mode::Plain);
        assert_eq!(detect(&args(&["--progress=off"]), true, &none), Mode::Off);
        assert_eq!(detect(&args(&[]), true, &env(&[("CI", "")])), Mode::Rich, "empty CI is unset");
    }

    #[test]
    fn utf8_from_locale() {
        assert!(utf8(&env(&[("LANG", "en_US.UTF-8")])));
        assert!(utf8(&env(&[("LC_ALL", "C.utf8"), ("LANG", "C")])));
        assert!(!utf8(&env(&[("LANG", "C")])));
        assert!(!utf8(&env(&[])));
        let p = Paint::new(Mode::Rich, false);
        assert_eq!((p.check(), p.block()), ("ok", '#'));
    }

    #[test]
    fn plain_summary_snapshot() {
        let p = Paint::new(Mode::Plain, true);
        let views = [ViewSize { route: "/".into(), js_gz: 2200, css_gz: 300, html_gz: 500 }, ViewSize { route: "/about".into(), js_gz: 0, css_gz: 300, html_gz: 400 }];
        let s = Summary { views: &views, budget: Some(2900), written: 3, unchanged: 9, signature: Some("k3j9x0aa"), seo: Some((84, 97)), took: Duration::from_millis(42), warnings: 1 };
        assert_eq!(
            summary(&p, &s),
            "page / js 2200 css 300 html 500 total 3000 over-budget\npage /about js 0 css 300 html 400 total 700\nsummary 2 pages · 3.6 KB gz total · SEO 84–97 · 3 written, 9 unchanged · 1 warning · 42 ms\nbuild k3j9x0aa\n"
        );
        // plain mode never emits escape codes
        assert!(!summary(&p, &s).contains('\x1b'));
        let rich = summary(&Paint::new(Mode::Rich, true), &s);
        assert!(rich.contains("\x1b[31m2.9 KB over budget") && rich.contains('█'), "{rich}");
    }

    #[test]
    fn plain_phase_lines() {
        let mut pr = Progress::new(Mode::Plain, true);
        pr.event("compile", 1, 2);
        pr.event("compile", 2, 2);
        pr.event("compress", 5, 5);
        pr.end();
        let ph = pr.phases.lock().unwrap();
        assert_eq!(ph.iter().map(|(n, _, i)| (n.as_str(), *i)).collect::<Vec<_>>(), [("compile", 2), ("compress", 5)]);
    }

    #[test]
    fn fix_lists_are_shortened_to_the_closest() {
        let msg = "token `--spcae-2` does not exist";
        let fix = "declare it in tokens.cssbrain or use an existing one: --space-1, --space-2, --space-3, --radius, --fg, --muted, --accent, --line";
        assert_eq!(short_fix(msg, fix), "declare it in tokens.cssbrain or use an existing one: --space-2, --space-1, --space-3, --accent, --line +3 more");
        assert_eq!(short_fix("x", "add a label"), "add a label", "no list: untouched");
        assert_eq!(clip("abcdef", 4), "abc…");
    }

    #[test]
    fn sparkline_scales() {
        let p = Paint::new(Mode::Plain, true);
        assert_eq!(sparkline(&p, &[1.0, 2.0, 4.0, 8.0]), "▂▃▅█");
        assert_eq!(sparkline(&Paint::new(Mode::Plain, false), &[1.0, 4.0]), ".#");
    }
}
