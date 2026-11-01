//! Speculation Rules appear only where a click is a document navigation, list only public static targets, and can be turned off.
use std::path::Path;
use std::process::{Command, Stdio};

fn brain(dir: &Path, args: &[&str]) {
    let out = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}
fn page(app: &Path, route: &str) -> String {
    std::fs::read_to_string(app.join(format!("dist/{route}.html"))).or_else(|_| std::fs::read_to_string(app.join(format!("dist/{route}/index.html")))).unwrap()
}
fn targets(html: &str) -> Option<String> {
    let rules = html.split("<script type=\"speculationrules\">").nth(1)?.split("</script>").next()?;
    assert!(rules.contains("\"prefetch\"") && !rules.contains("prerender"), "{rules}");
    assert!(rules.contains("\"eagerness\":\"moderate\"") && rules.contains("data-prefetch=false"), "{rules}");
    Some(rules.split("\"href_matches\":[").nth(1)?.split(']').next()?.to_string())
}

#[test]
fn rules_cover_public_static_targets_of_document_navigations_only() {
    let dir = std::env::temp_dir().join(format!("brain-speculation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    brain(&dir, &["new", "app"]);
    let app = dir.join("app");
    std::fs::write(app.join("views/_layout.brain"), "<nav aria-label=\"Main\"><a href=\"/\">Home</a> <a href=\"/about\">About</a> <a href=\"/account\">Account</a></nav>\n<slot />\n").unwrap();
    std::fs::write(app.join("views/counter.brain"), "<meta seo keyword=\"counter page\" title=\"Counter page\" description=\"An interactive counter page, so a router is part of this build.\" canonical=\"/counter\">\n<script>\n  let n = sig(0)\n</script>\n<main><h1>Counter page</h1><button type=\"button\" on:click={() => n++}>{n}</button></main>\n").unwrap();
    brain(&app, &["build"]);

    let about = page(&app, "about");
    assert!(about.contains("brain-zero-js"), "control: this page has no router, so its links are document navigations");
    let list = targets(&about).expect("a zero-JS page lists its targets");
    assert!(list.contains("\"/\"") && !list.contains("/about"), "public pages, not itself: {list}");
    assert!(!list.contains("/account") && !list.contains("/_"), "a guarded or internal route is never fetched ahead: {list}");

    let counter = page(&app, "counter");
    assert!(!counter.contains("brain-zero-js"), "control: this page has the router");
    let list = targets(&counter).expect("the router loads zero-JS targets as documents");
    assert!(list.contains("/about") && !list.contains("/counter"), "only targets the router does not take: {list}");

    let config = std::fs::read_to_string(app.join("brain.json")).unwrap();
    std::fs::write(app.join("brain.json"), config.replacen('{', "{\n  \"speculation\": false,", 1)).unwrap();
    brain(&app, &["build"]);
    assert!(targets(&page(&app, "about")).is_none() && targets(&page(&app, "counter")).is_none(), "brain.json opts out");
    let _ = std::fs::remove_dir_all(&dir);
}
