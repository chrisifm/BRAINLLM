//! `brain run --base /app/` (#148): dev serves the app under its base exactly like production.
#![cfg(unix)]
use std::io::{Read, Write};
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

/// The port from `brain run → http://127.0.0.1:<port>…` (stderr in plain mode).
pub fn run_port(c: &mut std::process::Child) -> u16 {
    use std::io::BufRead;
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
fn dev_serves_everything_under_the_base() {
    let tmp = std::env::temp_dir().join(format!("bb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "site"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("site");
    // the app's HTTP policy (§40): applied in dev as `http({…})`, never left as markup
    let app = std::fs::read_to_string(dir.join("app.brain")).unwrap();
    std::fs::write(dir.join("app.brain"), format!("<meta http base=\"/api\" timeout=\"5s\" login=\"/login\">\n{app}")).unwrap();
    std::fs::create_dir_all(dir.join("api")).unwrap();
    std::fs::write(dir.join("api/_http.jsbrain"), "http({ headers: { 'x-app': 'site' } })\n").unwrap();
    // --port 0: the OS picks a free port and `brain run` prints it (no race with parallel suites)
    let mut c = Command::new(bin).args(["run", "--base", "/app/", "--port", "0"]).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    let port = run_port(&mut c);
    let t = Instant::now();
    while !get(port, "/app/@brain/ping").contains("200 OK") {
        assert!(t.elapsed() < Duration::from_secs(20), "dev server did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    let home = get(port, "/app/");
    assert!(home.starts_with("HTTP/1.1 200"), "{home}");
    assert!(home.contains("<meta name=\"brain-base\" content=\"/app/\">"), "{home}");
    // api/_http.jsbrain runs after the policy and before the view (its own module script, in that order)
    let (pol, hdr) = (home.find("http({ base:").expect("policy"), home.find("import'/app/api/_http.jsbrain'").expect("api/_http.jsbrain"));
    assert!(pol < hdr && hdr < home.find("id=\"brain-route\"").unwrap(), "{home}");
    // app.brain's styles live in the `view` layer, as in the build (unlayered CSS would beat every view's rules)
    assert!(home.contains("<style>@layer view{") && !home.contains("<style>.top"), "{home}");
    assert!(!home.contains("<meta http") && home.contains("import{http}from'/app/@brain/brain-data.js';http({ base: \"/api\", timeout: \"5s\", login: \"/login\" });"), "{home}");
    assert!(home.contains("href=\"/app/about\"") && home.contains("from'/app/@brain/client.js'") && !home.contains("href=\"/about\""), "{home}");
    assert!(get(port, "/app").contains("Location: /app/"));
    assert!(get(port, "/").starts_with("HTTP/1.1 404") && get(port, "/about").starts_with("HTTP/1.1 404"), "outside the base");
    let layout = get(port, "/app/views/_layout.brain");
    assert!(layout.contains("\"/app/@brain/brain.js\"") && !layout.contains("\"/@brain/brain.js\""), "{layout}");
    // a dynamic route under the base: its load runs (server modules resolve /@brain/*) and route().params gets its meta
    let post = get(port, "/app/posts/hello-world");
    assert!(post.starts_with("HTTP/1.1 200") && post.contains("<meta name=\"brain-params\" content='{\"id\":\"hello-world\"}'>") && post.contains("\"v\":\"/views/posts/[id].brain\""), "{post}");
    // the loading state of a route (§22, #235): the nearest _loading.brain and its layouts, under the base too
    let ld = get(port, "/app/@brain/loading?path=%2Fapp%2Fposts%2Fhello-world");
    assert!(ld.contains("{\"v\":\"/views/_loading.brain\",\"l\":[\"/views/_layout.brain\"]}"), "{ld}");
    // a guard's redirect stays under the base
    let acc = get(port, "/app/account");
    assert!(acc.starts_with("HTTP/1.1 303") && acc.contains("Location: /app/login"), "{acc}");
    let bad = Command::new(bin).args(["run", "--base", "app"]).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    assert!(!bad.status.success() && String::from_utf8_lossy(&bad.stderr).contains("starts and ends with `/`"));
    let _ = c.kill();
    let _ = c.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}
