//! A public dynamic route rendered per request is only found by crawlers through sitemap.xml: without `paths()` or
//! `sitemap()` the build warns with B1837, naming the route and the fix. `noindex` and guarded views stay silent.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

const POST: &str = "<meta seo title=\"Blog post\" keyword=\"blog post\" description=\"Blog post: one article of the blog with its title and a short summary for readers.\">\n<script>\n  export async function load(ctx) { return { slug: ctx.params.slug } }\n</script>\n<main><h1>Blog post</h1><p>{props.slug}</p></main>\n";

#[test]
fn dynamic_route_without_sitemap_warns_with_b1837() {
    let tmp = std::env::temp_dir().join(format!("bunlisted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    let o = brain(&dir, &["build", "--site", "https://shop.acme.dev"]);
    let t = text(&o);
    assert!(o.status.success() && !t.contains("B1837"), "the scaffold lists every public route: {t}");
    for (f, src) in [
        ("views/blog/[slug].brain", POST.to_string()),
        ("views/orders/[id].brain", POST.replace("<meta seo title", "<meta seo robots=\"noindex\" title")),
        ("views/team/[id].brain", POST.to_string()),
        ("views/team/_guard.jsbrain", "export default function guard(ctx) { return ctx.user ? true : '/login' }\n".to_string()),
    ] {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
    let o = brain(&dir, &["build", "--site", "https://shop.acme.dev"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    let lines: Vec<&str> = t.lines().filter(|l| l.contains("B1837")).collect();
    assert_eq!(lines.len(), 1, "{t}");
    assert!(lines[0].contains("views/blog/[slug].brain") && lines[0].contains("/blog/:slug") && lines[0].contains("fix:") && lines[0].contains("sitemap("), "{t}");
    // listing the pages silences it
    std::fs::write(dir.join("views/blog/[slug].brain"), POST.replace("</script>", "  export async function sitemap() { return [{ slug: 'hello-world' }] }\n</script>")).unwrap();
    let t = text(&brain(&dir, &["build", "--site", "https://shop.acme.dev"]));
    assert!(!t.contains("B1837"), "{t}");
    let _ = std::fs::remove_dir_all(&tmp);
}
