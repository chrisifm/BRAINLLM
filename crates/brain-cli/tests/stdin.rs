//! Non-interactive commands never wait on stdin: an agent's tool often leaves it open (SEO #167). Each command runs
//! with a pipe that is never closed and must exit on its own.
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run(dir: &std::path::Path, args: &[&str]) -> (bool, Duration) {
    let t = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let _keep_open = child.stdin.take(); // held for the whole run: EOF never arrives
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return (s.success(), t.elapsed());
        }
        if t.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("`brain {}` waited on stdin", args.join(" "));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn commands_exit_with_stdin_open() {
    let dir = std::env::temp_dir().join(format!("brain-stdin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(run(&dir, &["new", "."]).0, "brain new");
    for args in [&["check", "--json"][..], &["map", "--json"], &["build", "--progress=off"]] {
        let (ok, took) = run(&dir, args);
        assert!(ok, "brain {}", args.join(" "));
        assert!(took < Duration::from_secs(30), "brain {} took {took:?}", args.join(" "));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
