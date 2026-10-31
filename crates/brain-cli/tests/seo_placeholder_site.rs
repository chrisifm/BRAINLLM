//! A placeholder site origin (`example.com`, `*.test`, `localhost`…) from `.brain_env` or brain.json is "no site":
//! the build warns with B0736 and a fix, publishes no sitemap and no robots `Sitemap:` line, and `brain doctor` flags
//! it before deploy. An explicit `--site` is the author's intent and is used as given.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn placeholder_origin_is_no_site_with_b0736() {
    let tmp = std::env::temp_dir().join(format!("bsite-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    std::fs::create_dir_all(dir.join("views/blog")).unwrap();
    std::fs::write(
        dir.join("views/blog/[slug].brain"),
        "<meta seo title=\"Blog post\" keyword=\"blog post\" description=\"Blog post: one article of the blog with its title and a short summary for readers.\">\n<script>\n  export async function load(ctx) { return { slug: ctx.params.slug } }\n  export async function sitemap() { return [{ slug: 'hello-world', lastmod: '2026-10-01' }, { slug: 'second-post' }] }\n</script>\n<main><h1>Blog post</h1><p>{props.slug}</p></main>\n",
    )
    .unwrap();
    // the scaffold's own .brain_env ships a placeholder production origin
    let env = std::fs::read_to_string(dir.join(".brain_env")).unwrap();
    assert!(env.contains("example.com"), "{env}");
    let o = brain(&dir, &["build"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("B0736") && t.contains("PUBLIC_SITE_URL") && t.contains("/blog/:slug"), "{t}");
    assert!(!dir.join("dist/sitemap.xml").exists());
    let robots = std::fs::read_to_string(dir.join("dist/robots.txt")).unwrap();
    assert!(!robots.contains("example.com") && !robots.contains("Sitemap:"), "{robots}");
    let d = text(&brain(&dir, &["doctor", "--json"]));
    assert!(d.contains("B0736"), "{d}");
    // a real origin builds the sitemap and silences both
    std::fs::write(dir.join(".brain_env"), env.replace("https://example.com", "https://shop.acme.dev")).unwrap();
    let o = brain(&dir, &["build"]);
    let t = text(&o);
    assert!(o.status.success() && !t.contains("B0736"), "{t}");
    assert!(std::fs::read_to_string(dir.join("dist/sitemap.xml")).unwrap().contains("https://shop.acme.dev/"));
    // an explicit --site is the author's intent
    std::fs::write(dir.join(".brain_env"), &env).unwrap();
    let t = text(&brain(&dir, &["build", "--site", "https://example.test"]));
    assert!(!t.contains("B0736"), "{t}");
    let _ = std::fs::remove_dir_all(&tmp);
}
