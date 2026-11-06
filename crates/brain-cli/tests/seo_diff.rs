//! CLI migration coverage using synthetic local XML and owned loopback HTTP fixtures only.
#[path = "../src/http_observer.rs"]
mod http_observer;
#[path = "../src/seo_diff.rs"]
mod seo_diff;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "brain-seo-diff-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("dist")).unwrap();
        Self(dir)
    }
    fn write(&self, name: &str, body: &str) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    fn run(&self, from: &str) -> (i32, serde_json::Value) {
        let (code, text) = seo_diff::run(
            Some(&self.0),
            &["diff".into(), "--from".into(), from.into(), "--json".into()],
        );
        (code, serde_json::from_str(&text).unwrap())
    }
    fn baseline(&self) -> String {
        self.0.join("baseline.xml").to_str().unwrap().into()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn set(urls: &[&str]) -> String {
    format!(
        "<?xml version=\"1.0\"?><urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">{}</urlset>",
        urls.iter()
            .map(|u| format!("<url><loc>{u}</loc></url>"))
            .collect::<String>()
    )
}
fn index(urls: &[&str]) -> String {
    format!(
        "<sitemapindex>{}</sitemapindex>",
        urls.iter()
            .map(|u| format!("<sitemap><loc>{u}</loc></sitemap>"))
            .collect::<String>()
    )
}

#[test]
fn compares_actual_missing_paths_queries_and_host_migrations_without_writes() {
    let f = Fixture::new();
    f.write(
        "baseline.xml",
        &set(&[
            "https://old.example.com/a?x=1&amp;y=2",
            "https://old.example.com/gone",
            "https://old.example.com/gone",
        ]),
    );
    let built = set(&[
        "https://example.com/a?x=1&amp;y=2",
        "https://example.com/new",
    ]);
    f.write("dist/sitemap.xml", &built);
    let before = std::fs::read_dir(f.0.join("dist")).unwrap().count();
    let (code, report) = f.run(&f.baseline());
    assert_eq!(code, 1);
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["previous"], 2);
    assert_eq!(report["current"], 2);
    assert_eq!(report["retained"], 1);
    assert_eq!(report["missingCount"], 1);
    assert_eq!(report["missing"], serde_json::json!(["/gone"]));
    assert_eq!(
        std::fs::read_to_string(f.0.join("dist/sitemap.xml")).unwrap(),
        built
    );
    assert_eq!(std::fs::read_dir(f.0.join("dist")).unwrap().count(), before);
    assert_eq!(f.run(&f.baseline()).1, report, "deterministic");
    f.write(
        "dist/sitemap.xml",
        &set(&[
            "https://example.com/a?x=1&amp;y=2",
            "https://example.com/gone",
        ]),
    );
    assert_eq!(f.run(&f.baseline()).0, 0);
    f.write(
        "dist/sitemap.xml",
        &set(&[
            "https://example.com/a?x=2&amp;y=2",
            "https://example.com/gone",
        ]),
    );
    assert_eq!(f.run(&f.baseline()).1["missingCount"], 1);
}

#[test]
fn rejects_malformed_mixed_origins_and_credential_inputs_without_echoing_them() {
    let f = Fixture::new();
    f.write("dist/sitemap.xml", &set(&[]));
    for body in [
        "<urlset><url><loc>https://example.com/a</loc></url>",
        "<urlset><url></url></urlset>",
        "<urlset><url><loc>https://example.com/a</loc><loc>https://example.com/b</loc></url></urlset>",
        "<!DOCTYPE urlset [<!ENTITY hidden SYSTEM 'secret'>]><urlset/>",
        "<urlset><url><loc>https://example.com/a&unknown;</loc></url></urlset>",
        "<urlset/><urlset/>",
        "<urlset invalid=unquoted/>",
        "<urlset//>",
    ] {
        f.write("baseline.xml", body);
        assert_eq!(f.run(&f.baseline()).0, 2, "{body}");
    }
    f.write(
        "baseline.xml",
        &set(&["https://example.com/a", "https://other.example.com/b"]),
    );
    assert_eq!(f.run(&f.baseline()).0, 2);
    let mut credential_url = reqwest::Url::parse("https://example.com/sitemap.xml").unwrap();
    credential_url.set_username("user").unwrap();
    credential_url.set_password(Some("private-secret")).unwrap();
    for source in [
        credential_url.as_str(),
        "file:///private-secret",
        "/missing-private-secret.xml",
    ] {
        let (code, report) = f.run(source);
        assert_eq!(code, 2);
        assert!(!report.to_string().contains("private-secret"));
    }
}

#[test]
fn indexes_load_confined_local_children_and_ignore_extension_locs() {
    let f = Fixture::new();
    f.write(
        "baseline.xml",
        &index(&[
            "https://old.example.com/app/part.xml",
            "https://old.example.com/app/part.xml",
        ]),
    );
    f.write("part.xml", "<urlset xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"><url><loc>https://old.example.com/a</loc><image:image><image:loc>https://images.example.com/image.png</image:loc></image:image></url></urlset>");
    f.write(
        "dist/sitemap.xml",
        &index(&["https://example.com/app/part.xml"]),
    );
    f.write("dist/part.xml", &set(&["https://example.com/a"]));
    assert_eq!(f.run(&f.baseline()).0, 0);
    for child in [
        "https://old.example.com/app/../secret.xml",
        "https://old.example.com/app/%2e%2e/secret.xml",
        "https://other.example.com/app/part.xml?secret=1",
    ] {
        f.write(
            "baseline.xml",
            &index(&[child, "https://old.example.com/app/part.xml"]),
        );
        assert_eq!(f.run(&f.baseline()).0, 2, "{child}");
    }
    f.write(
        "baseline.xml",
        &index(&["https://old.example.com/app/part.xml"]),
    );
    f.write(
        "part.xml",
        &index(&["https://old.example.com/app/part.xml"]),
    );
    assert_eq!(f.run(&f.baseline()).0, 2);
    f.write("part.xml", &set(&["https://other.example.com/a"]));
    assert_eq!(f.run(&f.baseline()).0, 2);
}

#[cfg(unix)]
#[test]
fn rejects_local_symlinks_including_a_symlinked_dist_directory() {
    let f = Fixture::new();
    f.write("real.xml", &set(&[]));
    std::os::unix::fs::symlink(f.0.join("real.xml"), f.0.join("baseline.xml")).unwrap();
    assert_eq!(f.run(&f.baseline()).0, 2);
    std::fs::remove_file(f.0.join("baseline.xml")).unwrap();
    f.write("baseline.xml", &set(&[]));
    std::fs::remove_dir(f.0.join("dist")).unwrap();
    std::fs::create_dir(f.0.join("real-dist")).unwrap();
    f.write("real-dist/sitemap.xml", &set(&[]));
    std::os::unix::fs::symlink(f.0.join("real-dist"), f.0.join("dist")).unwrap();
    assert_eq!(f.run(&f.baseline()).0, 2);
}

struct Server {
    origin: String,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(routes: impl FnOnce(&str) -> BTreeMap<String, (u16, String, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let routes = routes(&origin);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let requests = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let handle = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            while !done.load(Ordering::Relaxed) && Instant::now() < deadline {
                if let Ok((mut socket, _)) = listener.accept() {
                    socket
                        .set_read_timeout(Some(Duration::from_millis(500)))
                        .unwrap();
                    let mut data = [0; 4096];
                    if let Ok(n) = socket.read(&mut data) {
                        let request = String::from_utf8_lossy(&data[..n])
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("")
                            .to_string();
                        requests.lock().unwrap().push(request.clone());
                        let (status, body, headers) = routes.get(&request).cloned().unwrap_or((
                            404,
                            String::new(),
                            String::new(),
                        ));
                        let response = format!(
                            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
                            body.len()
                        );
                        let _ = socket.write_all(response.as_bytes());
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        Self {
            origin,
            seen,
            stop,
            handle: Some(handle),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap();
    }
}

#[test]
fn remote_sitemaps_fetch_only_same_origin_children_and_never_follow_redirects() {
    let f = Fixture::new();
    f.write("dist/sitemap.xml", &set(&["https://example.com/a"]));
    let server = Server::new(|origin| {
        BTreeMap::from([
            (
                "/sitemap.xml".into(),
                (200, index(&[&format!("{origin}/part.xml")]), String::new()),
            ),
            (
                "/part.xml".into(),
                (200, set(&[&format!("{origin}/a")]), String::new()),
            ),
            (
                "/redirect.xml".into(),
                (302, String::new(), "Location: /sitemap.xml\r\n".into()),
            ),
            (
                "/cross.xml".into(),
                (200, index(&["http://example.com/never.xml"]), String::new()),
            ),
        ])
    });
    assert_eq!(f.run(&format!("{}/sitemap.xml", server.origin)).0, 0);
    assert_eq!(
        server.seen.lock().unwrap().as_slice(),
        ["/sitemap.xml", "/part.xml"]
    );
    assert_eq!(f.run(&format!("{}/redirect.xml", server.origin)).0, 2);
    assert_eq!(f.run(&format!("{}/cross.xml", server.origin)).0, 2);
    assert_eq!(
        server.seen.lock().unwrap().as_slice(),
        ["/sitemap.xml", "/part.xml", "/redirect.xml", "/cross.xml"]
    );
}

#[test]
fn local_index_never_fetches_its_http_children() {
    let f = Fixture::new();
    f.write("dist/sitemap.xml", &set(&[]));
    let server = Server::new(|_| BTreeMap::new());
    f.write(
        "baseline.xml",
        &index(&[&format!("{}/absent.xml", server.origin)]),
    );
    assert_eq!(f.run(&f.baseline()).0, 2);
    assert!(server.seen.lock().unwrap().is_empty());
}

#[test]
fn rejects_oversized_inputs_and_too_many_children() {
    let f = Fixture::new();
    f.write("dist/sitemap.xml", &set(&[]));
    f.write("baseline.xml", &" ".repeat(8 * 1024 * 1024 + 1));
    assert_eq!(f.run(&f.baseline()).0, 2);
    let children: Vec<String> = (0..129)
        .map(|i| format!("https://example.com/part-{i}.xml"))
        .collect();
    f.write(
        "baseline.xml",
        &index(&children.iter().map(String::as_str).collect::<Vec<_>>()),
    );
    assert_eq!(f.run(&f.baseline()).0, 2);
}

#[test]
fn invalid_flags_and_missing_dist_are_errors() {
    let f = Fixture::new();
    f.write("baseline.xml", &set(&[]));
    assert_eq!(f.run(&f.baseline()).0, 2);
    for args in [
        vec!["diff", "--from"],
        vec!["diff", "--from=a", "--from=b"],
        vec!["diff", "--url=http://example.com"],
        vec!["diff", "--from=a", "stray"],
        vec!["diff", "--from=a", "--json", "--json"],
        vec!["diff", "--from=a", "diff"],
    ] {
        assert_eq!(
            seo_diff::run(
                Some(Path::new(&f.0)),
                &args.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            )
            .0,
            2
        );
    }
}
