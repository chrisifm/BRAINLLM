//! Production-output preview is temporary, bounded and independent of production supervisor state.
#![cfg(unix)]
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Preview {
    child: Child,
    dir: std::path::PathBuf,
}
impl Drop for Preview {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            unsafe {
                libc::kill(self.child.id() as i32, libc::SIGTERM);
            }
            let until = Instant::now() + Duration::from_secs(5);
            while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn preview(name: &str, server: bool) -> (Preview, u16) {
    let dir = std::env::temp_dir().join(format!("brain-preview-{}-{name}", std::process::id()));
    std::fs::create_dir_all(dir.join("dist/about")).unwrap();
    std::fs::write(dir.join("app.brain"), "<html><body><slot /></body></html>").unwrap();
    std::fs::write(dir.join("dist/index.html"), "preview-home").unwrap();
    std::fs::write(dir.join("dist/about/index.html"), "preview-about").unwrap();
    std::fs::write(
        dir.join("dist/_headers"),
        "/*\n  X-Preview-Test: production\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join(".brain/run")).unwrap();
    std::fs::write(
        dir.join(".brain/run/brain.pid"),
        "untouched-production-owner",
    )
    .unwrap();
    if server {
        std::fs::write(
            dir.join(".brain_env"),
            "[production]\nPREVIEW_TEST_VALUE=production-value\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("dist/server.mjs"),
            r#"
import {createServer} from 'node:http';
const s=createServer((q,r)=>r.end('preview-ssr '+process.env.PREVIEW_TEST_VALUE));
s.listen({fd:Number(process.env.BRAIN_LISTEN_FD)},()=>console.log('brain:ready'));
process.on('SIGTERM',()=>s.close(()=>process.exit(0)));
"#,
        )
        .unwrap();
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain"))
        .args(["preview", "--port", "0", "--json", "--stop-on-stdin-close"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let out = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::BufReader::new(out).read_line(&mut line);
        let _ = tx.send(line);
    });
    let owned = Preview { child, dir };
    let line = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("preview readiness deadline");
    assert!(line.contains("\"ready\":true"), "{line}");
    let port = line
        .split("127.0.0.1:")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    (owned, port)
}
fn get(port: u16, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    s.read_to_string(&mut response).unwrap();
    response
}
fn stop(mut p: Preview, port: u16, eof: bool) {
    if eof {
        drop(p.child.stdin.take());
    } else {
        unsafe {
            libc::kill(p.child.id() as i32, libc::SIGINT);
        }
    }
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = p.child.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            break;
        }
        assert!(Instant::now() < until, "preview did not stop");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "preview still owns its port"
    );
    assert_eq!(
        std::fs::read_to_string(p.dir.join(".brain/run/brain.pid")).unwrap(),
        "untouched-production-owner"
    );
    assert!(!p.dir.join("logs").exists());
}
#[test]
fn static_preview_serves_production_routes_and_headers_then_closes_on_eof() {
    let (p, port) = preview("static", false);
    let response = get(port, "/about");
    assert!(
        response.contains("preview-about")
            && response
                .to_lowercase()
                .contains("x-preview-test: production"),
        "{response}"
    );
    // Slow/incomplete clients must not retain a preview operation after its owner closes stdin.
    let slow: Vec<_> = (0..8)
        .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
        .collect();
    stop(p, port, true);
    drop(slow);
}
#[test]
fn server_preview_loads_production_environment_and_closes_on_sigint() {
    let (p, port) = preview("server", true);
    assert!(get(port, "/").contains("preview-ssr production-value"));
    stop(p, port, false);
}
#[test]
fn preview_rejects_public_bindings() {
    let out = Command::new(env!("CARGO_BIN_EXE_brain"))
        .args(["preview", "--host", "0.0.0.0"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn static_preview_does_not_serve_file_or_directory_symlink_targets() {
    use std::os::unix::fs::symlink;
    let (p, port) = preview("symlink-confinement", false);
    std::fs::create_dir_all(p.dir.join("private")).unwrap();
    std::fs::write(p.dir.join("private/secret.txt"), "PRIVATE_SENTINEL").unwrap();
    symlink(p.dir.join("private/secret.txt"), p.dir.join("dist/leak.txt")).unwrap();
    symlink(p.dir.join("private"), p.dir.join("dist/linked")).unwrap();
    for path in ["/leak.txt", "/linked/secret.txt"] {
        let response = get(port, path);
        assert!(response.starts_with("HTTP/1.1 404"), "{response}");
        assert!(!response.contains("PRIVATE_SENTINEL"));
    }
    assert!(get(port, "/about").ends_with("preview-about"));
    stop(p, port, true);
}
