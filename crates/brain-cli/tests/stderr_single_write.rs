//! A diagnostic line reaches stderr in one write, so a log collector that stamps each write cannot split it. A datagram
//! socket as stderr shows the writes: each one arrives as its own datagram.
#![cfg(unix)]
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixDatagram;
use std::process::{Command, Stdio};
use std::time::Duration;

fn writes(socket: &UnixDatagram) -> Vec<String> {
    socket.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    let mut buffer = [0u8; 65536];
    let mut seen = Vec::new();
    while let Ok(n) = socket.recv(&mut buffer) {
        seen.push(String::from_utf8_lossy(&buffer[..n]).into_owned());
    }
    seen
}

#[test]
fn an_error_line_is_one_write() {
    let (read, write) = UnixDatagram::pair().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_brain")).args(["build", "--not-a-flag"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(OwnedFd::from(write)).status().unwrap();
    assert!(!status.success());
    let seen = writes(&read);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(seen[0].starts_with("error: ") && seen[0].ends_with('\n') && seen[0].len() > "error: \n".len(), "{seen:?}");
}

#[test]
fn a_dev_server_log_line_is_one_write() {
    let dir = std::env::temp_dir().join(format!("brain-stderr-write-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "site"]).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success());
    let site = dir.join("site");
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let (read, write) = UnixDatagram::pair().unwrap();
    let mut server = Command::new(bin).args(["dev", "--port", &port.to_string()]).current_dir(&site).stdin(Stdio::null()).stdout(Stdio::null()).stderr(OwnedFd::from(write)).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let seen = writes(&read);
    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!seen.is_empty(), "the dev server logs to stderr");
    assert!(seen.iter().all(|w| w.ends_with('\n') && w.len() > 1), "{seen:?}");
}
