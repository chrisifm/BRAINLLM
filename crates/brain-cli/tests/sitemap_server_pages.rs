//! A public page rendered per request exists for crawlers as much as a prerendered one: the build lists it in sitemap.xml
//! and llms.txt, and writes the site origin to the server manifest so that `sitemap()` URLs have an absolute address even
//! when the build itself lists nothing. A sitemap that ends up with no URL at all is reported, never silent.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn view(title: &str, extra: &str, body: &str) -> String {
    format!(
        "<meta seo title=\"{title}\" {extra} description=\"{title}: a page of this site with enough words to describe what it holds.\" />\n<script server>\n  export async function load() {{ return {{ name: 'item' }} }}\n{body}</script>\n<main><h1>{title}</h1><p>{{props.name}}</p></main>\n"
    )
}

fn app(name: &str) -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    std::fs::remove_dir_all(dir.join("views")).unwrap();
    std::fs::create_dir_all(dir.join("views")).unwrap();
    dir
}

fn write(dir: &std::path::Path, files: &[(&str, String)]) {
    for (f, src) in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
}

#[test]
fn public_pages_rendered_per_request_are_listed_and_private_ones_are_not() {
    let dir = app("bsmserver");
    write(&dir, &[
        ("views/index.brain", view("Home", "", "")),
        ("views/jobs/index.brain", view("Jobs", "", "")),
        ("views/hidden/index.brain", view("Hidden", "robots=\"noindex\"", "")),
        ("views/team/index.brain", view("Team", "", "")),
        ("views/team/_guard.jsbrain", "export default function guard(ctx) { return ctx.user ? true : '/login' }\n".into()),
    ]);
    let o = brain(&dir, &["build", "--site", "https://shop.acme.dev"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    let sm = std::fs::read_to_string(dir.join("dist/sitemap.xml")).unwrap();
    assert!(sm.contains("<loc>https://shop.acme.dev/</loc>") && sm.contains("<loc>https://shop.acme.dev/jobs</loc>"), "{sm}");
    assert!(!sm.contains("/hidden") && !sm.contains("/team"), "{sm}");
    assert!(!t.contains("B1900"), "{t}");
    let llms = std::fs::read_to_string(dir.join("dist/llms.txt")).unwrap();
    assert!(llms.contains("(https://shop.acme.dev/jobs)") && !llms.contains("jobs.md") && !llms.contains("/team"), "{llms}");
    let manifest = std::fs::read_to_string(dir.join("dist/_server/manifest.json")).unwrap();
    assert!(manifest.contains("\"site\":\"https://shop.acme.dev\""), "{manifest}");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn a_sitemap_without_urls_is_reported_unless_a_view_fills_it_at_request_time() {
    let dir = app("bsmempty");
    write(&dir, &[("views/index.brain", view("Home", "robots=\"noindex\"", ""))]);
    let t = text(&brain(&dir, &["build", "--site", "https://shop.acme.dev"]));
    assert!(t.lines().any(|l| l.contains("B1900") && l.contains("fix:")), "{t}");
    write(&dir, &[("views/jobs/[slug].brain", view("Job", "", "  export async function sitemap() { return [{ slug: 'a' }] }\n"))]);
    let t = text(&brain(&dir, &["build", "--site", "https://shop.acme.dev"]));
    assert!(!t.contains("B1900"), "{t}");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn a_page_rendered_per_request_repeats_its_ids_and_its_size() {
    let dir = app("bsmids");
    write(&dir, &[
        ("comp/nav.brain", "<nav id=\"main\"><a href=\"/\">Home</a></nav>\n".to_string()),
        ("views/index.brain", view("Home", "", "").replace("<script server>\n", "<script server>\n  import Nav from '../comp/nav.brain'\n").replace("<main>", "<Nav />\n<main>")),
    ]);
    let o = brain(&dir, &["build", "--site", "https://shop.acme.dev"]);
    assert!(o.status.success(), "{}", text(&o));
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut server = Command::new("node").arg("dist/server.mjs").env("PORT", port.to_string()).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let get = || {
        use std::io::{Read, Write};
        for _ in 0..100 {
            if let Ok(mut c) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                c.write_all(b"GET / HTTP/1.1\r\nHost: shop.acme.dev\r\nConnection: close\r\n\r\n").unwrap();
                let mut body = String::new();
                c.read_to_string(&mut body).unwrap();
                return body;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("no server");
    };
    let (first, second) = (get(), get());
    let _ = server.kill();
    let _ = server.wait();
    assert!(first.contains("id=\"main\"") && second.contains("id=\"main\""), "the first instance of each page keeps its id:\n{second}");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn every_page_closes_with_the_credit_unless_the_app_turns_it_off() {
    let dir = app("bsmcredit");
    write(&dir, &[
        ("views/index.brain", view("Home", "", "")),
        ("views/about.brain", "<meta seo title=\"About\" description=\"About this site: who makes it and what it is for, in a few words.\" />\n<main><h1>About</h1></main>\n".to_string()),
    ]);
    assert!(brain(&dir, &["build", "--site", "https://shop.acme.dev"]).status.success());
    let line = format!("By <a href=\"https://chrisifm.github.io/BRAINLLM/\" rel=\"nofollow\">BRAINLLM</a> {}</p>", env!("CARGO_PKG_VERSION"));
    let about = std::fs::read_to_string(dir.join("dist/about.html")).or_else(|_| std::fs::read_to_string(dir.join("dist/about/index.html"))).unwrap();
    assert!(about.contains(&line), "{about}");
    let credit = about.split("<p data-brain-credit").nth(1).expect("credit signature").split("</p>").next().unwrap();
    assert!(!credit.contains("margin:"), "the signature leaves margins to the app: {credit}");
    assert!(credit.contains("padding:8px 16px calc(8px + env(safe-area-inset-bottom))"), "the signature clears the device home indicator: {credit}");
    let served = std::fs::read_dir(dir.join("dist/_server/pages")).unwrap().flatten().map(|f| std::fs::read_to_string(f.path()).unwrap()).collect::<Vec<_>>();
    assert!(served.iter().any(|p| p.contains(&line)), "a page rendered per request has it too");
    let json = std::fs::read_to_string(dir.join("brain.json")).unwrap();
    assert!(json.contains("\"seo\": { "));
    std::fs::write(dir.join("brain.json"), json.replace("\"seo\": { ", "\"seo\": { \"credit\": false, ")).unwrap();
    assert!(brain(&dir, &["build", "--site", "https://shop.acme.dev"]).status.success());
    let about = std::fs::read_to_string(dir.join("dist/about.html")).or_else(|_| std::fs::read_to_string(dir.join("dist/about/index.html"))).unwrap();
    assert!(!about.contains("data-brain-credit"), "{about}");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn the_credit_line_does_not_spend_the_view_budget() {
    let dir = app("bsmcreditsize");
    write(&dir, &[("views/index.brain", view("Home", "", ""))]);
    let total = |credit: bool| {
        let site = if credit { "" } else { ",\"seo\":{\"credit\":false}" };
        std::fs::write(dir.join("brain.json"), format!("{{\"budget\":{{\"viewKB\":0.001}}{site}}}")).unwrap();
        let t = text(&brain(&dir, &["build", "--site", "https://shop.acme.dev"]));
        let at = t.find("exceeded on /: ").unwrap_or_else(|| panic!("{t}")) + 15;
        t[at..].split(' ').next().unwrap().parse::<usize>().unwrap()
    };
    // the two brain.json files give two build ids (`<meta name="brain-build">`): their compressed size may differ by a
    // byte or two, the ~200-byte credit line may not
    let (with, without) = (total(true), total(false));
    assert!(with.abs_diff(without) <= 4, "{with} vs {without}");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn the_word_noindex_outside_robots_keeps_a_page_listed() {
    let dir = app("bsmword");
    let said = view("Said", "", "").replace("<script server>\n", "<script server>\n  // this page used to be noindex\n");
    assert!(said.contains("noindex"));
    write(&dir, &[
        ("views/index.brain", view("Home", "", "")),
        ("views/said/index.brain", said),
        ("views/hidden/index.brain", view("Hidden", "robots=\"noindex\"", "")),
    ]);
    let t = text(&brain(&dir, &["build", "--site", "https://shop.acme.dev"]));
    let sm = std::fs::read_to_string(dir.join("dist/sitemap.xml")).unwrap_or_else(|_| panic!("{t}"));
    assert!(sm.contains("/said") && !sm.contains("/hidden"), "{sm}");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}
