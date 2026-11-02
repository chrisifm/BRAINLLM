//! A project's declared port (`--port`, then `PORT` from the process or `.brain_env`) belongs to that project: `brain run`
//! and `brain start` never move to another one. A previous brain of this same project still holding it is stopped; any
//! other owner is left alone and named in the error, because this process cannot know what stopping it would break.
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

/// The port this command must use, or `None` when nothing declares one and the command's default applies.
pub fn declared(args: &[String], root: &Path, env: &str) -> Result<Option<u16>, String> {
    let (raw, source) = match crate::flag(args, "--port") {
        Some(port) => (port, "--port"),
        None => match crate::env::value(root, env, "PORT") {
            Some(port) => (port, "PORT"),
            None => return Ok(None),
        },
    };
    raw.trim().parse().map(Some).map_err(|_| format!("invalid {source} `{raw}`; fix: a number 1-65535"))
}

/// Free `port` for this project, or say who holds it. Returns once a bind succeeds; the socket is released for the caller.
pub fn claim(root: &Path, host: &str, port: u16) -> Result<(), String> {
    if port == 0 || TcpListener::bind((host, port)).is_ok() {
        return Ok(());
    }
    let holders = owners::of_port(port);
    if holders.is_empty() {
        // Not visible to this user (or no lsof/ss/Windows): the owner may still be exiting, so report only after the wait.
        return brain_serve::listen::bind_waiting(host, port, brain_serve::listen::RESTART_WAIT).map(drop).map_err(|e| e.to_string());
    }
    let stoppable = owners::stoppable(root, &holders).map_err(|reason| format!("cannot listen on {host}:{port}: {reason}; fix: stop it, or set another PORT in .brain_env or --port"))?;
    eprintln!("port {port}: stopping the previous brain of this project (pid {})", join(&stoppable));
    owners::signal(&stoppable, false);
    if released(host, port, brain_serve::listen::RESTART_WAIT) {
        return Ok(());
    }
    owners::signal(&stoppable, true);
    if released(host, port, Duration::from_secs(2)) {
        return Ok(());
    }
    Err(format!("cannot listen on {host}:{port}: pid {} did not release it after being stopped; fix: find the owner with `lsof -nP -iTCP:{port} -sTCP:LISTEN`, or pick another --port", join(&stoppable)))
}

fn released(host: &str, port: u16, wait: Duration) -> bool {
    let started = Instant::now();
    loop {
        if TcpListener::bind((host, port)).is_ok() {
            return true;
        }
        if started.elapsed() >= wait {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn join(pids: &[u32]) -> String {
    pids.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
}

#[cfg(unix)]
mod owners {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    fn output(program: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn lsof(args: &[&str]) -> Option<String> {
        output("lsof", args).or_else(|| output("/usr/sbin/lsof", args)).or_else(|| output("/usr/bin/lsof", args))
    }

    pub fn of_port(port: u16) -> Vec<u32> {
        let mut pids: Vec<u32> = match lsof(&["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"]) {
            Some(text) => text.lines().filter_map(|line| line.trim().parse().ok()).collect(),
            None => output("ss", &["-ltnpH", &format!("sport = :{port}")])
                .unwrap_or_default()
                .split("pid=")
                .skip(1)
                .filter_map(|rest| rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok())
                .collect(),
        };
        pids.sort_unstable();
        pids.dedup();
        pids
    }

    fn ps(pid: u32, field: &str) -> Option<String> {
        output("ps", &["-o", &format!("{field}="), "-p", &pid.to_string()]).map(|text| text.trim().to_string()).filter(|text| !text.is_empty())
    }

    fn parent(pid: u32) -> Option<u32> {
        ps(pid, "ppid")?.parse().ok()
    }

    fn cwd(pid: u32) -> Option<PathBuf> {
        if let Ok(path) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
            return Some(path);
        }
        let text = lsof(&["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])?;
        text.lines().find_map(|line| line.strip_prefix('n')).map(PathBuf::from)
    }

    fn is_brain(pid: u32) -> bool {
        ps(pid, "comm").is_some_and(|name| name.rsplit('/').next() == Some("brain"))
    }

    fn belongs_to(pid: u32, root: &Path) -> bool {
        is_brain(pid) && cwd(pid).and_then(|dir| crate::scaffold::find_root(&dir)).and_then(|found| found.canonicalize().ok()).as_deref() == root.canonicalize().ok().as_deref()
    }

    /// PM2's daemon above `pid`: it would restart what we stop, and the two would keep taking the port from each other.
    fn supervisor(pid: u32) -> Option<u32> {
        let mut current = pid;
        for _ in 0..16 {
            current = parent(current).filter(|up| *up > 1)?;
            if ps(current, "command").is_some_and(|command| command.contains("God Daemon")) {
                return Some(current);
            }
        }
        None
    }

    /// Every holder must be a brain of this project or a worker it spawned. The error names the first that is not.
    pub fn stoppable(root: &Path, holders: &[u32]) -> Result<Vec<u32>, String> {
        let own = supervisor(std::process::id());
        for pid in holders {
            let mine = belongs_to(*pid, root) || parent(*pid).is_some_and(|up| holders.contains(&up) && belongs_to(up, root));
            if !mine {
                let name = ps(*pid, "comm").unwrap_or_else(|| "unknown".into());
                return Err(format!("the port is held by pid {pid} ({name}), which is not a brain of this project, so it was left running"));
            }
            let managed = supervisor(*pid);
            if managed.is_some() && managed != own {
                return Err(format!("the port is held by this project's pid {pid}, which PM2 manages and would restart, so it was left running; use `pm2 stop` first"));
            }
        }
        Ok(holders.to_vec())
    }

    pub fn signal(pids: &[u32], force: bool) {
        for pid in pids {
            // SAFETY: kill(2) takes plain integers and reports failure through its return value, which is not needed here:
            // a process that is already gone is the outcome being asked for.
            unsafe { libc::kill(*pid as libc::pid_t, if force { libc::SIGKILL } else { libc::SIGTERM }) };
        }
    }
}

#[cfg(not(unix))]
mod owners {
    use std::path::Path;
    pub fn of_port(_port: u16) -> Vec<u32> {
        Vec::new()
    }
    pub fn stoppable(_root: &Path, _holders: &[u32]) -> Result<Vec<u32>, String> {
        Ok(Vec::new())
    }
    pub fn signal(_pids: &[u32], _force: bool) {}
}
