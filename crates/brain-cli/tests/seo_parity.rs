//! The same view prerendered and rendered per request describes itself to crawlers the same way.
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

const VIEW: &str = "<meta seo title={props.t} description={props.d} og:image=\"/card.png\" schema=\"Article\">\n<script>\n{PATHS}  export async function load(ctx) { return { t: 'Post ' + ctx.params.id, d: 'A post description with enough words for the audit length rules here.' } }\n</script>\n<h1>{props.t}</h1>\n";
const SHELL: &str = "<meta seo titleTemplate=\"%s · Parity\">\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Parity</title>\n</head>\n<body><slot /></body>\n</html>\n";

fn write(root: &Path, file: &str, text: &str) {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn head_of(body: &str) -> String {
    body.split("</head>").next().unwrap().to_string()
}

fn value(head: &str, attr: &str, key: &str) -> Option<String> {
    let needle = format!("{attr}=\"{key}\" content=\"");
    let at = head.find(&needle)? + needle.len();
    Some(head[at..].split('"').next()?.to_string())
}

fn get(port: u16, path: &str, cookie: &str) -> (String, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/html\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let mut response = out.as_str();
    while response.starts_with("HTTP/1.1 103") {
        response = response.split_once("\r\n\r\n").unwrap().1;
    }
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    (headers.to_ascii_lowercase(), body.to_string())
}

#[test]
fn a_view_says_the_same_to_crawlers_with_or_without_a_server() {
    let root = std::env::temp_dir().join(format!("brain-seo-parity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write(&root, "brain.json", "{\"name\":\"Parity\",\"site\":\"https://example.dev\",\"seo\":{\"og\":\"off\"}}\n");
    write(&root, "app.brain", SHELL);
    write(&root, "tokens.cssbrain", ":root { --accent: #0062c4; }\n");
    write(&root, "views/index.brain", "<meta seo title=\"Home title for readers\" keyword=\"home\" description=\"Home description with enough words for the audit length rules.\">\n<h1>Home</h1>\n");
    write(&root, "views/fixed/[id].brain", &VIEW.replace("{PATHS}", "  export async function paths() { return [{ id: 'a' }] }\n"));
    write(&root, "views/live/[id].brain", &VIEW.replace("{PATHS}", ""));
    write(&root, "views/members/_guard.jsbrain", "export default async function guard() { return true }\n");
    write(&root, "views/members/[id].brain", &VIEW.replace("{PATHS}", ""));
    let bin = env!("CARGO_BIN_EXE_brain");
    let built = Command::new(bin).arg("build").current_dir(&root).stdin(Stdio::null()).output().unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));

    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let mut server = Command::new("node").arg("dist/server.mjs").env("PORT", port.to_string()).current_dir(&root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut lines = BufReader::new(server.stdout.take().unwrap()).lines();
    assert!(lines.any(|l| l.is_ok_and(|l| l.contains("listening") || l.contains("brain:ready"))));
    let (_, fixed) = get(port, "/fixed/a", "");
    let (live_headers, live) = get(port, "/live/a", "");
    let (member_headers, _) = get(port, "/members/a", "");
    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&root);

    let (fixed, live) = (head_of(&fixed), head_of(&live));
    for (attr, key) in [("property", "og:title"), ("property", "og:description"), ("property", "og:image"), ("property", "og:image:alt"), ("property", "og:site_name"), ("property", "og:locale"), ("name", "twitter:card"), ("name", "robots"), ("name", "theme-color"), ("name", "description")] {
        assert_eq!(value(&live, attr, key), value(&fixed, attr, key), "{key} differs\nfixed: {fixed}\nlive: {live}");
        assert!(value(&live, attr, key).is_some(), "{key} is missing: {live}");
    }
    assert_eq!(value(&live, "property", "og:image").as_deref(), Some("https://example.dev/card.png"));
    assert_eq!(value(&live, "property", "og:title").as_deref(), Some("Post a · Parity"), "{live}");
    assert!(live.contains("<title>Post a · Parity</title>"), "{live}");
    for tag in ["rel=\"manifest\"", "/sections.json"] {
        assert_eq!(live.contains(tag), fixed.contains(tag), "{tag}\nfixed: {fixed}\nlive: {live}");
    }
    assert!(!live_headers.contains("x-robots-tag"), "{live_headers}");
    assert!(member_headers.contains("x-robots-tag: noindex"), "a guarded page stays out of the index: {member_headers}");
}

#[test]
fn only_the_query_a_view_declares_is_part_of_its_identity() {
    let root = std::env::temp_dir().join(format!("brain-seo-query-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write(&root, "brain.json", "{\"name\":\"Query\",\"site\":\"https://example.dev\",\"seo\":{\"og\":\"off\"}}\n");
    write(&root, "app.brain", SHELL);
    write(&root, "tokens.cssbrain", ":root { --accent: #0062c4; }\n");
    write(&root, "views/index.brain", "<meta seo title=\"Home title for readers\" keyword=\"home\" description=\"Home description with enough words for the audit length rules.\">\n<h1>Home</h1>\n");
    write(&root, "views/list/[kind].brain", "<meta seo query=\"page\" title={props.t} description={props.t}>\n<script>\n  export async function load(ctx) { return { t: 'A listing of ' + ctx.params.kind + ' with enough words for the audit' } }\n</script>\n<h1>{props.t}</h1>\n");
    write(&root, "views/about.brain", "<meta seo query=\"page\" title=\"About title for readers\" keyword=\"about\" description=\"About description with enough words for the audit length rules.\">\n<h1>About</h1>\n");
    let built = Command::new(env!("CARGO_BIN_EXE_brain")).args(["build", "--json"]).current_dir(&root).stdin(Stdio::null()).output().unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    assert!(String::from_utf8_lossy(&built.stdout).contains("B0503 views/about.brain"), "a prerendered page cannot vary by query");
    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let mut server = Command::new("node").arg("dist/server.mjs").env("PORT", port.to_string()).current_dir(&root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut lines = BufReader::new(server.stdout.take().unwrap()).lines();
    assert!(lines.any(|l| l.is_ok_and(|l| l.contains("listening") || l.contains("brain:ready"))));
    let (_, landing) = get(port, "/list/shoes?page=2&utm_source=mail&dialog=size", "");
    let (_, modal) = get(port, "/list/shoes?dialog=size", "");
    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&root);
    let (landing, modal) = (head_of(&landing), head_of(&modal));
    assert!(landing.contains("<link rel=\"canonical\" href=\"https://example.dev/list/shoes?page=2\">"), "{landing}");
    assert_eq!(value(&landing, "property", "og:url").as_deref(), Some("https://example.dev/list/shoes?page=2"), "{landing}");
    assert!(modal.contains("<link rel=\"canonical\" href=\"https://example.dev/list/shoes\">"), "{modal}");
    assert!(!landing.contains("brain-query") && !modal.contains("brain-query"), "{landing}");
}
