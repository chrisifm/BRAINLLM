//! `csp.connectSrc`: the policy `brain build` emits decides, in a real Chrome, whether a page reaches another origin.
#![cfg(unix)]
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn brain(dir: &Path, args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap();
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

/// A new app whose brain.json holds `csp`, or none.
fn app(tag: &str, csp: Option<&str>) -> PathBuf {
    let tmp = std::env::temp_dir().join(format!("brain-connect-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let (ok, out) = brain(&tmp, &["new", "app"]);
    assert!(ok, "{out}");
    let dir = tmp.join("app");
    let manifest = std::fs::read_to_string(dir.join("brain.json")).unwrap();
    assert!(manifest.contains("\"csp\": { \"connectSrc\": [] }"), "a new app starts with its own origin only: {manifest}");
    let edited = match csp {
        Some(list) => manifest.replace("\"csp\": { \"connectSrc\": [] }", &format!("\"csp\": {list}")),
        None => manifest.replace(",\n  \"csp\": { \"connectSrc\": [] }", ""),
    };
    std::fs::write(dir.join("brain.json"), edited).unwrap();
    dir
}

fn policy(dir: &Path) -> String {
    let (ok, out) = brain(dir, &["build"]);
    assert!(ok, "{out}");
    let headers = std::fs::read_to_string(dir.join("dist/_headers")).unwrap();
    headers.lines().find_map(|l| l.trim().strip_prefix("Content-Security-Policy: ")).expect("the build writes the policy").to_string()
}
fn connect(policy: &str) -> &str {
    policy.split("; ").find(|d| d.starts_with("connect-src")).unwrap()
}

#[test]
fn a_new_app_is_silent_and_an_undeclared_list_warns_and_keeps_the_broad_form() {
    let fresh = app("fresh", Some("{ \"connectSrc\": [] }"));
    assert!(!brain(&fresh, &["check"]).1.contains("B0795"));
    assert_eq!(connect(&policy(&fresh)), "connect-src 'self'");

    let open = app("open", None);
    let (ok, out) = brain(&open, &["check"]);
    assert!(ok && out.contains("B0795"), "{out}");
    assert_eq!(connect(&policy(&open)), "connect-src 'self' https: wss:");
}

#[test]
fn invalid_entries_are_errors_and_never_widen_the_policy() {
    let dir = app("invalid", Some("{ \"connectSrc\": [\"https:\", \"https://api.site.test/v1\", \"http://api.site.test\", \"https://ok.site.test\"] }"));
    let (ok, out) = brain(&dir, &["check"]);
    assert!(!ok && out.matches("B0796").count() >= 3, "{out}");
    assert_eq!(connect(&brain_pack::cache::app_csp(&dir, &[], &[])), "connect-src 'self' https://ok.site.test");
}

#[test]
fn a_call_outside_the_list_warns_b0746_and_a_listed_one_is_silent() {
    let dir = app("calls", Some("{ \"connectSrc\": [\"https://api.site.test\", \"wss://*.live.test\"] }"));
    let view = |url: &str| format!("<script>\nexport function ping() {{ return fetch('{url}') }}\n</script>\n<button on:click={{ping}}>Go</button>\n");
    std::fs::write(dir.join("views/home.brain"), view("https://other.site.test/a")).unwrap();
    let (_, out) = brain(&dir, &["check"]);
    assert!(out.contains("B0746") && out.contains("csp.connectSrc"), "{out}");
    std::fs::write(dir.join("views/home.brain"), view("https://api.site.test/a")).unwrap();
    assert!(!brain(&dir, &["check"]).1.contains("B0746"));
}

// ---- real browser ----
fn serve(listener: TcpListener, respond: impl Fn(&str) -> Vec<u8> + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let path = String::from_utf8_lossy(&buf[..n]).split_whitespace().nth(1).unwrap_or("/").to_string();
            let _ = stream.write_all(&respond(&path));
        }
    });
}
fn chrome() -> Option<String> {
    if let Ok(p) = std::env::var("BRAIN_CHROME") {
        return Some(p);
    }
    let mac = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    if Path::new(mac).is_file() {
        return Some(mac.into());
    }
    ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"].iter().find(|n| Command::new(n).arg("--version").stdin(Stdio::null()).output().is_ok_and(|o| o.status.success())).map(|n| n.to_string())
}
/// What the page managed to do, under `policy`, calling `other`.
fn page_result(chrome: &str, policy: &str, other_port: u16) -> String {
    let site = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = site.local_addr().unwrap().port();
    let policy = policy.to_string();
    let script = format!("fetch('http://127.0.0.1:{other_port}/data').then(r=>r.text()).then(t=>{{document.body.textContent='RESULT:'+t}},()=>{{document.body.textContent='RESULT:blocked'}})");
    serve(site, move |path| {
        let (ty, body) = if path == "/t.js" { ("text/javascript", script.clone()) } else { ("text/html", "<!doctype html><title>t</title><body>wait<script src=\"/t.js\"></script>".to_string()) };
        format!("HTTP/1.1 200 OK\r\ncontent-type: {ty}\r\ncontent-security-policy: {policy}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).into_bytes()
    });
    let profile = std::env::temp_dir().join(format!("brain-connect-chrome-{}-{port}", std::process::id()));
    let out = Command::new(chrome)
        .args(["--headless=new", "--no-sandbox", "--disable-gpu", "--no-first-run", "--virtual-time-budget=4000", "--dump-dom"])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("http://127.0.0.1:{port}/"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&profile);
    let dom = String::from_utf8_lossy(&out.stdout).into_owned();
    dom.split("RESULT:").nth(1).map(|t| t.split('<').next().unwrap_or("").to_string()).unwrap_or_else(|| format!("no result: {dom}"))
}

#[test]
fn chrome_enforces_the_declared_list() {
    let Some(chrome) = chrome() else {
        eprintln!("skipped: Chrome not installed");
        return;
    };
    let other = TcpListener::bind("127.0.0.1:0").unwrap();
    let other_port = other.local_addr().unwrap().port();
    serve(other, |_| b"HTTP/1.1 200 OK\r\naccess-control-allow-origin: *\r\ncontent-length: 7\r\nconnection: close\r\n\r\nreached".to_vec());

    let listed = app("listed", Some(&format!("{{ \"connectSrc\": [\"http://127.0.0.1:{other_port}\"] }}")));
    assert_eq!(page_result(&chrome, &policy(&listed), other_port), "reached");

    let closed = app("closed", Some("{ \"connectSrc\": [] }"));
    assert_eq!(page_result(&chrome, &policy(&closed), other_port), "blocked");
}
