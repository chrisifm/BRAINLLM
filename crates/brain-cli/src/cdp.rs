//! Our own CDP client over `--remote-debugging-pipe`: Chrome reads commands from fd 3 and writes to fd 4;
//! each JSON message ends in `\0`. No WebSocket, no port and no dependencies.

use brain_mcp::Json;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

unsafe extern "C" {
    fn pipe(fds: *mut i32) -> i32;
    fn dup2(src: i32, dst: i32) -> i32;
    fn close(fd: i32) -> i32;
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
}
const F_DUPFD: i32 = 0;

pub struct Chrome {
    child: Child,
    w: File,
    /// Every message from Chrome, read by a thread of its own: a call can give up on its answer without blocking the
    /// pipe (a late answer is discarded by id).
    rx: Receiver<Vec<u8>>,
    next: u64,
    timed_out: Arc<AtomicBool>,
    watchdog: Option<(mpsc::Sender<()>, std::thread::JoinHandle<()>)>,
    /// Longest wait for one answer; `None` = only the session watchdog (`brain test`, whose evaluations run tests).
    pub deadline: Option<Duration>,
    /// Modal dialogs the page opened (and we dismissed) since the caller last took them: `confirm: Leave this page?…`.
    pub dialogs: Vec<String>,
    _profile: TempDir,
}

/// Throwaway Chrome profile; deleted on drop.
struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `BRAIN_CHROME` or the usual macOS and Linux paths.
pub fn find_chrome() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("BRAIN_CHROME") {
        return Some(PathBuf::from(p));
    }
    let mac = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
    ];
    if let Some(p) = mac.iter().map(PathBuf::from).find(|p| p.is_file()) {
        return Some(p);
    }
    let path = std::env::var_os("PATH")?;
    ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"]
        .iter()
        .flat_map(|n| std::env::split_paths(&path).map(move |d| d.join(n)))
        .find(|p| p.is_file())
}

fn os_pipe() -> io::Result<[i32; 2]> {
    let mut fds = [0i32; 2];
    if unsafe { pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fds)
}

impl Chrome {
    /// Launches headless Chrome. `timeout` kills the process if the whole session hangs.
    pub fn launch(timeout: Duration) -> io::Result<Chrome> {
        let exe = find_chrome().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Chrome not found; fix: install Google Chrome or set BRAIN_CHROME=/path/to/binary")
        })?;
        let profile = std::env::temp_dir().join(format!("brain-test-{}-{}", std::process::id(), nanos()));
        std::fs::create_dir_all(&profile)?;
        let profile = TempDir(profile);
        let to_chrome = os_pipe()?; // [read (Chrome, fd 3), write (us)]
        let from_chrome = match os_pipe() {
            Ok(pipe) => pipe,
            Err(error) => {
                unsafe { close(to_chrome[0]); close(to_chrome[1]); }
                return Err(error);
            }
        }; // [read (us), write (Chrome, fd 4)]
        let (c_read, c_write) = (to_chrome[0], from_chrome[1]);
        let mut cmd = Command::new(exe);
        cmd.args([
            "--headless",
            "--remote-debugging-pipe",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-background-networking",
            "--mute-audio",
            // a fresh profile would try to store "Chrome Safe Storage" in the macOS keychain and pop an alert
            "--use-mock-keychain",
            "--password-store=basic",
        ])
        .arg(format!("--user-data-dir={}", profile.0.display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        let ours = [to_chrome[1], from_chrome[0]];
        // SAFETY: between fork and exec only async-signal-safe functions are called (fcntl, dup2, close).
        unsafe {
            cmd.pre_exec(move || {
                // Move to high fds first, so dup2 to 3/4 does not clobber an end that is still needed.
                let r = fcntl(c_read, F_DUPFD, 10);
                let w = fcntl(c_write, F_DUPFD, 10);
                if r < 0 || w < 0 || dup2(r, 3) < 0 || dup2(w, 4) < 0 {
                    return Err(io::Error::last_os_error());
                }
                for fd in [r, w, c_read, c_write, ours[0], ours[1]] {
                    if fd != 3 && fd != 4 {
                        close(fd);
                    }
                }
                Ok(())
            });
        }
        // This private browser and its workers belong to this session only.
        cmd.process_group(0);
        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(error) => {
                for fd in [to_chrome[0], to_chrome[1], from_chrome[0], from_chrome[1]] {
                    unsafe { close(fd); }
                }
                return Err(error);
            }
        };
        unsafe {
            close(c_read);
            close(c_write);
        }
        let w = unsafe { File::from_raw_fd(to_chrome[1]) };
        let mut r = BufReader::new(unsafe { File::from_raw_fd(from_chrome[0]) });
        let (tx, rx) = mpsc::sync_channel(2);
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            while matches!((&mut r).take(32 * 1024 * 1024 + 1).read_until(0, &mut buf), Ok(n) if n > 0) {
                if buf.len() > 32 * 1024 * 1024 { break; }
                buf.pop_if(|b| *b == 0);
                if tx.send(std::mem::take(&mut buf)).is_err() {
                    break;
                }
            }
        });
        let timed_out = Arc::new(AtomicBool::new(false));
        let (pid, flag) = (child.id(), timed_out.clone());
        let (cancel, cancelled) = mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            if cancelled.recv_timeout(timeout) == Err(RecvTimeoutError::Timeout) {
                flag.store(true, Ordering::SeqCst);
                // The child is not reaped until Drop cancels and joins this watchdog, so its PID cannot be reused.
                unsafe { kill(-(pid as i32), 9); }
            }
        });
        Ok(Chrome { child, w, rx, next: 0, timed_out, watchdog: Some((cancel, watchdog)), deadline: None, dialogs: Vec::new(), _profile: profile })
    }

    /// Sends a command and waits for its response; intermediate events are discarded.
    pub fn call(&mut self, session: Option<&str>, method: &str, params: Json) -> Result<Json, String> {
        self.next += 1;
        let id = self.next;
        let mut msg = vec![("id".to_string(), Json::Num(id as f64)), ("method".into(), Json::str(method)), ("params".into(), params)];
        if let Some(s) = session {
            msg.push(("sessionId".into(), Json::str(s)));
        }
        let mut bytes = Json::Obj(msg).stringify().into_bytes();
        bytes.push(0);
        self.w.write_all(&bytes).and_then(|_| self.w.flush()).map_err(|e| self.io_err(e))?;
        let start = std::time::Instant::now();
        // what the page did while we waited, to say why an answer never came
        let mut seen: Vec<String> = Vec::new();
        loop {
            let left = match self.deadline {
                Some(d) => match d.checked_sub(start.elapsed()) {
                    Some(l) => l,
                    None => return Err(self.late(method, d, &seen)),
                },
                None => Duration::from_secs(24 * 3600),
            };
            let buf = match self.rx.recv_timeout(left) {
                Ok(b) => b,
                Err(RecvTimeoutError::Timeout) => match self.deadline {
                    Some(d) => return Err(self.late(method, d, &seen)),
                    None => continue,
                },
                Err(RecvTimeoutError::Disconnected) => return Err(self.io_err(io::Error::new(io::ErrorKind::UnexpectedEof, "Chrome closed the pipe"))),
            };
            let v = Json::parse(&String::from_utf8_lossy(&buf))?;
            if v.get("id") != Some(&Json::Num(id as f64)) {
                if let Some(ev) = v.get("method").and_then(Json::as_str) {
                    // a modal dialog stops the page (and every evaluation) until someone answers it: dismiss it
                    if ev == "Page.javascriptDialogOpening" {
                        let p = |k: &str| v.get("params").and_then(|p| p.get(k)).and_then(Json::as_str).unwrap_or("").to_string();
                        let kind = if p("type").is_empty() { "alert".to_string() } else { p("type") };
                        seen.push(format!("a `{kind}` dialog (dismissed)"));
                        let msg = if kind == "beforeunload" && p("message").is_empty() { "Leave this page? (a dirty form blocked the navigation; it stayed)".to_string() } else { p("message") };
                        self.dialogs.push(format!("{kind}: {msg}"));
                        self.send(v.get("sessionId").and_then(Json::as_str), "Page.handleJavaScriptDialog", Json::obj([("accept", Json::Bool(false))]))?;
                    } else if ev == "Runtime.executionContextDestroyed" || ev == "Page.frameNavigated" {
                        if !seen.iter().any(|s| s.starts_with("a navigation")) {
                            seen.push("a navigation (the page context was destroyed)".into());
                        }
                    }
                }
                continue;
            }
            if let Some(e) = v.get("error") {
                return Err(format!("CDP {method}: {}", e.get("message").and_then(Json::as_str).unwrap_or("error")));
            }
            return Ok(v.get("result").cloned().unwrap_or(Json::Null));
        }
    }

    /// Writes a command without waiting for its answer (it is discarded when it arrives).
    fn send(&mut self, session: Option<&str>, method: &str, params: Json) -> Result<(), String> {
        self.next += 1;
        let mut msg = vec![("id".to_string(), Json::Num(self.next as f64)), ("method".into(), Json::str(method)), ("params".into(), params)];
        if let Some(s) = session {
            msg.push(("sessionId".into(), Json::str(s)));
        }
        let mut bytes = Json::Obj(msg).stringify().into_bytes();
        bytes.push(0);
        self.w.write_all(&bytes).and_then(|_| self.w.flush()).map_err(|e| self.io_err(e))
    }

    fn late(&self, method: &str, d: Duration, seen: &[String]) -> String {
        let why = if seen.is_empty() { "the page is busy or the promise never settles".to_string() } else { format!("meanwhile the page had {}", seen.join(" and ")) };
        format!("timed out: no answer to {method} in {} s ({why}); fix: return a value that settles sooner, and read the result in a separate call after a navigation", d.as_secs())
    }

    fn io_err(&self, e: io::Error) -> String {
        if self.timed_out.load(Ordering::SeqCst) {
            "timed out: Chrome did not respond; fix: raise --timeout or make sure the test finishes".into()
        } else {
            format!("pipe CDP: {e}")
        }
    }

    /// Opens a tab and returns its sessionId (flatten mode).
    pub fn new_page(&mut self) -> Result<String, String> {
        let t = self.call(None, "Target.createTarget", Json::obj([("url", Json::str("about:blank"))]))?;
        let target = t.get("targetId").cloned().ok_or("Target.createTarget without targetId")?;
        let s = self.call(None, "Target.attachToTarget", Json::obj([("targetId", target), ("flatten", Json::Bool(true))]))?;
        Ok(s.get("sessionId").and_then(Json::as_str).ok_or("attachToTarget without sessionId")?.to_string())
    }

    /// `Runtime.evaluate` with await and return by value; a throw in the page becomes Err.
    pub fn eval(&mut self, session: &str, expr: &str) -> Result<Json, String> {
        let r = self
            .call(
                Some(session),
                "Runtime.evaluate",
                Json::obj([("expression", Json::str(expr)), ("awaitPromise", Json::Bool(true)), ("returnByValue", Json::Bool(true))]),
            )
            .map_err(|e| {
                if e.contains("navigated or closed") || e.contains("context was destroyed") {
                    "context destroyed by navigation: the page navigated while the expression was pending; fix: trigger the action in one call (brain_click or brain_eval without awaiting it) and read the new page with another".to_string()
                } else {
                    e
                }
            })?;
        if let Some(ex) = r.get("exceptionDetails") {
            let msg = ex.get("exception").and_then(|e| e.get("description")).and_then(Json::as_str).or_else(|| ex.get("text").and_then(Json::as_str));
            return Err(msg.unwrap_or("exception in the page").to_string());
        }
        Ok(r.get("result").and_then(|r| r.get("value")).cloned().unwrap_or(Json::Null))
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        if let Some((cancel, watchdog)) = self.watchdog.take() {
            let _ = cancel.send(());
            let _ = watchdog.join();
        }
        // A blocked browser cannot answer Browser.close. Kill only our private process group, including renderers.
        unsafe { kill(-(self.child.id() as i32), 9); }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn dropping_a_blocked_browser_cancels_its_watchdog_and_reaps_the_child() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 60"]).process_group(0);
        let child = cmd.spawn().unwrap();
        let pid = child.id();
        let (cancel, cancelled) = mpsc::channel();
        let flag = Arc::new(AtomicBool::new(false));
        let watchdog_flag = flag.clone();
        let watchdog = std::thread::spawn(move || {
            if cancelled.recv_timeout(Duration::from_secs(60)) == Err(RecvTimeoutError::Timeout) {
                watchdog_flag.store(true, Ordering::SeqCst);
            }
        });
        let (_sender, rx) = mpsc::channel();
        let profile = std::env::temp_dir().join(format!("brain-browser-drop-{}-{}", std::process::id(), nanos()));
        std::fs::create_dir_all(&profile).unwrap();
        let chrome = Chrome { child, w: File::create("/dev/null").unwrap(), rx, next: 0,
            timed_out: flag.clone(), watchdog: Some((cancel, watchdog)), deadline: None,
            dialogs: vec![], _profile: TempDir(profile.clone()) };
        let start = std::time::Instant::now();
        drop(chrome);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(!flag.load(Ordering::SeqCst));
        assert!(!profile.exists());
        assert_eq!(unsafe { kill(pid as i32, 0) }, -1, "the owned child must be reaped");
    }
}
