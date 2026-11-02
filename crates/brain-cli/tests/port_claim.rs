//! A declared port belongs to its project: `brain run` never moves off it, replaces a previous brain of the same project,
//! and leaves every other owner running.
#![cfg(unix)]
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn app(name: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("brain-port-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let ok = Command::new(env!("CARGO_BIN_EXE_brain")).args(["new", "app"]).current_dir(&base).stdin(Stdio::null()).stdout(Stdio::null()).status().unwrap().success();
    assert!(ok);
    (base.join("app"), base)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn spawn(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_brain")).arg("run").arg("--no-inspect").args(args).current_dir(dir).env_remove("PORT").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap()
}

/// The port it announced, or the failure output if it exited first.
fn announced(child: &mut Child) -> Result<u16, String> {
    let mut lines = BufReader::new(child.stderr.take().unwrap());
    let mut seen = String::new();
    loop {
        let mut line = String::new();
        if lines.read_line(&mut line).unwrap_or(0) == 0 {
            return Err(seen);
        }
        seen.push_str(&line);
        if line.contains("brain run →") {
            std::thread::spawn(move || { let mut sink = String::new(); while lines.read_line(&mut sink).unwrap_or(0) > 0 { sink.clear(); } });
            let port = line.split("127.0.0.1:").nth(1).and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next()).unwrap();
            return Ok(port.parse().unwrap());
        }
    }
}

fn exits(child: &mut Child, wait: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < wait {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn a_second_run_of_the_same_project_takes_the_port_over() {
    let (dir, base) = app("same");
    let port = free_port().to_string();
    let mut first = spawn(&dir, &["--port", &port]);
    assert_eq!(announced(&mut first).unwrap().to_string(), port);
    let mut second = spawn(&dir, &["--port", &port]);
    assert_eq!(announced(&mut second).expect("second run").to_string(), port, "it must not move to the next port");
    assert!(exits(&mut first, Duration::from_secs(15)), "the previous brain must be stopped");
    let _ = second.kill();
    let _ = second.wait();
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn another_owner_is_named_and_left_running() {
    let (dir, base) = app("foreign");
    let holder = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = holder.local_addr().unwrap().port().to_string();
    let mut run = spawn(&dir, &["--port", &port]);
    let failure = announced(&mut run).expect_err("it must not move to the next port");
    assert!(failure.contains(&format!("held by pid {}", std::process::id())), "{failure}");
    assert!(failure.contains("left running") && failure.contains("PORT"), "{failure}");
    assert!(!run.wait().unwrap().success());
    assert!(std::net::TcpStream::connect(("127.0.0.1", holder.local_addr().unwrap().port())).is_ok(), "the owner keeps the port");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_brain_of_another_project_is_left_running() {
    let (first_dir, first_base) = app("other-a");
    let (second_dir, second_base) = app("other-b");
    let port = free_port().to_string();
    let mut first = spawn(&first_dir, &["--port", &port]);
    announced(&mut first).unwrap();
    let mut second = spawn(&second_dir, &["--port", &port]);
    let failure = announced(&mut second).expect_err("it must not take another project's port");
    assert!(failure.contains("not a brain of this project"), "{failure}");
    assert!(first.try_wait().unwrap().is_none(), "the other project keeps running");
    let _ = first.kill();
    let _ = first.wait();
    let _ = std::fs::remove_dir_all(first_base);
    let _ = std::fs::remove_dir_all(second_base);
}

#[test]
fn brain_env_declares_the_port() {
    let (dir, base) = app("declared");
    let port = free_port();
    let env = std::fs::read_to_string(dir.join(".brain_env")).unwrap_or_default();
    std::fs::write(dir.join(".brain_env"), format!("PORT={port}\n{env}")).unwrap();
    let mut run = spawn(&dir, &[]);
    assert_eq!(announced(&mut run).unwrap(), port);
    let _ = run.kill();
    let _ = run.wait();
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn an_invalid_declared_port_is_reported() {
    let (dir, base) = app("invalid");
    let env = std::fs::read_to_string(dir.join(".brain_env")).unwrap_or_default();
    std::fs::write(dir.join(".brain_env"), format!("PORT=abc\n{env}")).unwrap();
    let mut run = spawn(&dir, &[]);
    let failure = announced(&mut run).expect_err("an invalid port must stop the command");
    assert!(failure.contains("invalid PORT"), "{failure}");
    let _ = std::fs::remove_dir_all(base);
}
