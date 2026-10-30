//! Compiled feature markers opt into exact GIS CSP sources and page-scoped opener policy.
use brain_pack::cache::{self, CacheCfg, PageRef};
use std::{collections::BTreeMap, path::PathBuf};
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn marker_is_real_head_metadata_not_comments_raw_text_attributes_or_templates() {
    for source in [
        "<html><head><meta name='brain-google-signin' content='1'></head></html>",
        "<HEAD><META content=1 NAME=brain-google-signin></HEAD>",
    ] {
        assert!(cache::google_signin(source))
    }
    for source in ["<head><!-- <meta name='brain-google-signin' content='1'> --></head>","<head><script>const text=\"<meta name='brain-google-signin' content='1'>\"</script></head>","<head><link title=\"<meta name='brain-google-signin' content='1'>\"></head>","<head><template><meta name='brain-google-signin' content='1'></template></head>","<head><title><meta name='brain-google-signin' content='1'></title></head>","<head></head><body><meta name='brain-google-signin' content='1'></body>","<head><meta name='other' name='brain-google-signin' content='1'></head>"] {assert!(!cache::google_signin(source),"{source}")}
}
#[test]
fn generated_hosts_keep_opener_policy_on_google_pages_and_preserve_default_everywhere_else() {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = Scratch(std::env::temp_dir().join(format!(
        "brain-google-headers-{}-{time}",
        std::process::id()
    )));
    std::fs::create_dir_all(&root.0).unwrap();
    let pages = [
        PageRef {
            route: "/",
            file: "index.html",
        },
        PageRef {
            route: "/signin",
            file: "signin.html",
        },
    ];
    let mut files=BTreeMap::from([("index.html".into(),"<html><head><title>Home</title></head><body>Home</body></html>".into()),("signin.html".into(),"<html><head><meta name='brain-google-signin' content='1'></head><body>Sign in</body></html>".into())]);
    let policy = cache::csp(&[], &cache::google_csp());
    assert!(policy.contains("frame-src 'self' https://accounts.google.com/gsi/"));
    let script = policy.split(';').find(|part| part.trim_start().starts_with("script-src ")).unwrap();
    assert!(!script.split_whitespace().any(|source| source == "https:"), "broad schemes must not replace an exact GIS source: {policy}");
    assert!(script.split_whitespace().any(|source| source == "https://accounts.google.com/gsi/client"));
    cache::apply(
        &root.0,
        &CacheCfg {
            base: "/app/".into(),
            sw: false,
            offline: false,
            ..Default::default()
        },
        &pages,
        &cache::google_csp(),
        &mut files,
        &mut BTreeMap::new(),
    );
    let headers = &files["_headers"];
    assert!(headers.contains("/app/signin\n") && headers.contains("/app/signin.html\n"));
    let home = headers
        .split("/app/\n")
        .nth(1)
        .unwrap()
        .split("/app/signin\n")
        .next()
        .unwrap();
    assert!(
        home.contains("Cross-Origin-Opener-Policy: same-origin\n")
            && !home.contains("same-origin-allow-popups")
    );
    let google = headers
        .split("/app/signin\n")
        .nth(1)
        .unwrap()
        .split("/app/signin/\n")
        .next()
        .unwrap();
    assert!(google.contains("Cross-Origin-Opener-Policy: same-origin-allow-popups"));
    let nginx = &files["_brain/nginx.conf"];
    assert!(
        nginx.contains("location = /app/signin {") && nginx.contains("location = /app/signin/ {")
    );
    assert!(nginx.contains("$uri/index.html /app/signin.html =404"));
    assert_eq!(nginx.matches("same-origin-allow-popups").count(), 3);
    assert!(nginx.contains("Cross-Origin-Opener-Policy \"same-origin\" always"));
    let caddy = &files["_brain/Caddyfile"];
    assert!(caddy.contains("@google_signin path /app/signin /app/signin.html /app/signin/"));
    assert!(caddy.contains("header @ordinary_opener Cross-Origin-Opener-Policy same-origin"));
}
