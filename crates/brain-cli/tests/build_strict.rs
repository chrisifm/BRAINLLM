//! `brain build --strict`: an output the source does not fully describe fails the build instead of only warning.
#![cfg(unix)]
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str], path: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_brain"));
    c.args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null());
    if let Some(p) = path {
        c.env("PATH", p);
    }
    c.output().unwrap()
}
fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn strict_fails_a_degraded_build_and_invalid_limits_stop_any_build() {
    let tmp = std::env::temp_dir().join(format!("brain-build-strict-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"], None).status.success());
    let dir = tmp.join("app");
    let strict = brain(&dir, &["build", "--strict"], None);
    assert!(strict.status.success(), "a complete build passes strict: {}", text(&strict));

    // a component that throws while prerendering: the page ships its fallback instead of its content
    std::fs::create_dir_all(dir.join("comp")).unwrap();
    std::fs::write(dir.join("comp/boom.brain"), "<script>\nconst boom = () => { throw Error('exploded') }\nlet v = sig(boom())\n</script>\n<p>{v}</p>").unwrap();
    std::fs::write(dir.join("views/boomed.brain"), "<meta seo title=\"Boomed page title here\" description=\"A page whose component throws during the build.\">\n<main><h1>Boomed</h1><Boom /></main>\n").unwrap();
    let lax = brain(&dir, &["build"], None);
    assert!(lax.status.success(), "{}", text(&lax));
    assert!(text(&lax).contains("during prerender"), "{}", text(&lax));
    let cut = brain(&dir, &["build", "--strict"], None);
    assert!(!cut.status.success() && text(&cut).contains("--strict") && text(&cut).contains("during prerender"), "{}", text(&cut));
    let json = brain(&dir, &["build", "--strict", "--json"], None);
    assert!(!json.status.success(), "the JSON path honours --strict too: {}", text(&json));
    std::fs::remove_file(dir.join("views/boomed.brain")).unwrap();

    let manifest = std::fs::read_to_string(dir.join("brain.json")).unwrap();
    std::fs::write(dir.join("brain.json"), manifest.replacen("\"docs\": true,", "\"docs\": true,\n  \"server\": { \"maxBodyKB\": 0 },", 1)).unwrap();
    let limits = brain(&dir, &["build"], None);
    assert!(!limits.status.success() && text(&limits).contains("B0707"), "{}", text(&limits));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A page listed by `paths()` is published to be read: a failed render must not reach `dist/` as an empty 200.
#[test]
fn a_paths_page_that_cannot_render_stops_the_build() {
    let tmp = std::env::temp_dir().join(format!("brain-build-paths-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"], None).status.success());
    let dir = tmp.join("app");
    std::fs::create_dir_all(dir.join("views/cat")).unwrap();
    let view = |load: &str| format!("<meta seo title=\"Category page title\" description=\"A category page with its own description text.\">\n<script>\n  export async function paths() {{ return [{{ slug: 'art' }}] }}\n  export async function load() {{ {load} }}\n</script>\n<h1>Category</h1>\n");
    std::fs::write(dir.join("views/cat/[slug].brain"), view("throw Object.assign(new Error('backend down'), { status: 503 })")).unwrap();
    let failed = brain(&dir, &["build"], None);
    assert!(!failed.status.success(), "{}", text(&failed));
    assert!(text(&failed).contains("/cat/art") && text(&failed).contains("backend down") && text(&failed).contains("sitemap()"), "{}", text(&failed));
    std::fs::write(dir.join("views/cat/[slug].brain"), view("return {}")).unwrap();
    let fine = brain(&dir, &["build"], None);
    assert!(fine.status.success(), "{}", text(&fine));
    assert!(std::fs::read_to_string(dir.join("dist/cat/art/index.html")).unwrap().contains(">Category</h1>"));
    let _ = std::fs::remove_dir_all(&tmp);
}
