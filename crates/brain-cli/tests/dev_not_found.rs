//! F11: `throw notFound()` / `return notFound()` in a view's `load` answers 404 with views/404.brain under `brain run`,
//! as server.mjs does in production.
#![cfg(unix)]
use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn get(port: u16, path: &str) -> String {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes());
    let mut b = String::new();
    let _ = s.read_to_string(&mut b);
    b
}

fn run_port(c: &mut std::process::Child) -> u16 {
    let err = c.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for l in std::io::BufReader::new(err).lines().map_while(Result::ok) {
            let _ = tx.send(l);
        }
    });
    loop {
        let l = rx.recv_timeout(Duration::from_secs(120)).expect("brain run printed no address");
        if let Some(p) = l.split("127.0.0.1:").nth(1).and_then(|r| r.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|p| p.parse().ok()) {
            return p;
        }
    }
}

#[test]
fn dev_load_not_found_is_a_404_page() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    }
    let tmp = std::env::temp_dir().join(format!("bnf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "site"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("site");
    std::fs::create_dir_all(dir.join("views/jobs")).unwrap();
    std::fs::write(
        dir.join("views/jobs/[slug].brain"),
        "<meta seo title=\"Job\" description=\"A job\">\n<script>\nexport async function load(ctx) {\n  if (ctx.params.slug === 'gone') throw notFound()\n  if (ctx.params.slug === 'old') return notFound()\n  return { slug: ctx.params.slug }\n}\n</script>\n<main><h1>{props.slug}</h1></main>\n",
    )
    .unwrap();
    std::fs::write(dir.join("views/404.brain"), "<meta seo title=\"Not found\" description=\"Not found\">\n<main><h1>Lost page</h1></main>\n").unwrap();
    let mut c = Command::new(bin).args(["run", "--port", "0"]).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    let port = run_port(&mut c);
    let t = Instant::now();
    while !get(port, "/@brain/ping").contains("200 OK") {
        assert!(t.elapsed() < Duration::from_secs(20), "dev server did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    for p in ["/jobs/gone", "/jobs/old"] {
        let r = get(port, p);
        assert!(r.starts_with("HTTP/1.1 404") && r.contains("/views/404.brain"), "{p}: {r}");
    }
    let ok = get(port, "/jobs/backend");
    assert!(ok.starts_with("HTTP/1.1 200") && ok.contains("\"v\":\"/views/jobs/[slug].brain\""), "{ok}");
    let _ = c.kill();
    let _ = c.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}
