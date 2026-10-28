//! Temporary local preview of the actual production output. No supervisor state or installation.
use std::io::{Read, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

struct Signals(libc::sighandler_t, libc::sighandler_t);
impl Signals {
    fn install() -> Self {
        STOP.store(false, Ordering::Relaxed);
        // SAFETY: handlers only update an atomic, and are restored before returning.
        unsafe {
            Self(
                libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t),
                libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t),
            )
        }
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGINT, self.0);
            libc::signal(libc::SIGTERM, self.1);
        }
    }
}
struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        // Only this child's private process group belongs to preview. Kill descendants too, then reap.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(200));
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}
fn cancelled(eof: bool) -> bool {
    if STOP.load(Ordering::Relaxed) {
        return true;
    }
    if !eof {
        return false;
    }
    let mut fd = libc::pollfd {
        fd: 0,
        events: libc::POLLIN | libc::POLLHUP,
        revents: 0,
    };
    // Nonblocking readiness check: consume at most one chunk without retaining input.
    if unsafe { libc::poll(&mut fd, 1, 0) } > 0 {
        if fd.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return true;
        }
        if fd.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut bytes = [0u8; 1024];
            return unsafe { libc::read(0, bytes.as_mut_ptr().cast(), bytes.len()) } <= 0;
        }
    }
    false
}
fn ready(url: &str, mode: &str, json: bool) -> Result<(), String> {
    let line = if json {
        format!(
            "{{\"schemaVersion\":1,\"ready\":true,\"mode\":{},\"url\":{}}}",
            brain_serve::json_str(mode),
            brain_serve::json_str(url)
        )
    } else {
        format!("brain preview -> {url} ({mode}; Ctrl+C to stop)")
    };
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}")
        .and_then(|_| out.flush())
        .map_err(|e| format!("cannot report preview readiness: {e}"))
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let mut host: IpAddr = "127.0.0.1".parse().unwrap();
    let mut port = 4173u16;
    let mut env = "production".to_string();
    let (mut json, mut eof) = (false, false);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--stop-on-stdin-close" => eof = true,
            "--port" | "--host" | "--env" => {
                let flag = &args[i];
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                match flag.as_str() {
                    "--port" => {
                        port = v
                            .parse()
                            .map_err(|_| "--port must be between 0 and 65535")?
                    }
                    "--host" => {
                        host = if v == "localhost" {
                            "127.0.0.1".parse().unwrap()
                        } else {
                            v.parse()
                                .map_err(|_| "--host must be a loopback IP or localhost")?
                        }
                    }
                    _ => env = v.clone(),
                }
            }
            other => {
                return Err(format!(
                    "unknown preview option `{other}`; use --port, --host, --env, --json or --stop-on-stdin-close"
                ));
            }
        }
        i += 1;
    }
    if !host.is_loopback() {
        return Err("preview only binds loopback; use brain start for production hosting".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let dist = root.join("dist");
    if !["index.html", "index.brain", "server.mjs"]
        .iter()
        .any(|f| dist.join(f).is_file())
    {
        return Err(
            "dist/ has no production entry point; run brain build before brain preview".into(),
        );
    }
    let listener = TcpListener::bind((host, port))
        .map_err(|e| format!("cannot bind preview: {e}; choose another --port"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let base = brain_pack::read_base(&root);
    let url = format!("http://{addr}/{}", base.trim_matches('/'));
    let _signals = Signals::install();
    if !dist.join("server.mjs").is_file() {
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let handler = Arc::new(brain_serve::prod::Static::load(&dist, &base));
        let mut tasks: Vec<(std::thread::JoinHandle<()>, TcpStream)> = Vec::new();
        ready(&url, "static", json)?;
        let result = loop {
            let mut n = 0;
            while n < tasks.len() {
                if tasks[n].0.is_finished() {
                    let _ = tasks.swap_remove(n).0.join();
                } else {
                    n += 1;
                }
            }
            if cancelled(eof) {
                break Ok(());
            }
            if tasks.len() < 8 {
                match listener.accept() {
                    Ok((s, _)) => {
                        let _ = s.set_nonblocking(false);
                        let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
                        let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
                        let cancel = match s.try_clone() {
                            Ok(s) => s,
                            Err(e) => break Err(e.to_string()),
                        };
                        let handler = handler.clone();
                        tasks.push((
                            std::thread::spawn(move || {
                                let _ = handler.handle(s);
                            }),
                            cancel,
                        ));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => break Err(e.to_string()),
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        drop(listener);
        for (_, stream) in &tasks {
            let _ = stream.shutdown(Shutdown::Both);
        }
        for (task, _) in tasks {
            let _ = task.join();
        }
        return result;
    }
    let fd = listener.as_raw_fd();
    let mut cmd = Command::new(std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into()));
    cmd.arg("--import").arg("data:text/javascript,const%20p%3Dprocess.ppid%3BsetInterval(()%3D%3E%7Bif(process.ppid!%3D%3Dp)process.exit(0)%7D%2C500).unref()");
    cmd.arg(dist.join("server.mjs"))
        .current_dir(&dist)
        .envs(crate::env::select(&root, &env))
        .env("BRAIN_LISTEN_FD", "3")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .process_group(0);
    unsafe {
        cmd.pre_exec(move || {
            if (fd == 3 && libc::fcntl(3, libc::F_SETFD, 0) < 0)
                || (fd != 3 && libc::dup2(fd, 3) < 0)
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut worker = Worker(cmd.spawn().map_err(|e| {
        format!("cannot start preview worker: {e}; install Node 22+ or set BRAIN_NODE")
    })?);
    let mut output = worker.0.stdout.take().unwrap();
    unsafe {
        let flags = libc::fcntl(output.as_raw_fd(), libc::F_GETFL);
        if flags < 0 || libc::fcntl(output.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) < 0
        {
            return Err("cannot monitor preview worker output".into());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut line = Vec::new();
    let mut announced = false;
    loop {
        if cancelled(eof) {
            return Ok(());
        }
        if let Some(status) = worker.0.try_wait().map_err(|e| e.to_string())? {
            return Err(format!(
                "preview worker exited ({status}); inspect the server error and run brain build"
            ));
        }
        let mut bytes = [0u8; 4096];
        match output.read(&mut bytes) {
            Ok(n) => {
                for &b in &bytes[..n] {
                    if b == b'\n' {
                        if !announced && line.starts_with(b"brain:ready") {
                            ready(&url, "server", json)?;
                            announced = true;
                        }
                        line.clear();
                    } else if line.len() < 4096 {
                        line.push(b);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("cannot read preview readiness: {e}")),
        }
        if !announced && Instant::now() >= deadline {
            return Err(
                "preview worker was not ready within 30 seconds; inspect dist/server.mjs".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
