//! `brain add collection` (INTERNALS §33): the generated views read `brain:content`, pass check untouched and build to
//! prerendered entry and tag pages; a broken frontmatter is reported by check with its file.
use std::process::{Command, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr))
}

#[test]
fn a_new_collection_checks_and_builds() {
    let tmp = std::env::temp_dir().join(format!("bcol-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).0);
    let dir = tmp.join("app");
    let (ok, out) = brain(&dir, &["add", "collection", "blog"]);
    assert!(ok, "{out}");
    for f in ["content/blog/hello-world.md", "views/blog/index.brain", "views/blog/page/[n].brain", "views/blog/[slug].brain", "views/blog/tag/[tag].brain"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    assert!(std::fs::read_to_string(dir.join("app.brain")).unwrap().contains("<a href=\"/blog\">Blog</a>"), "linked from the nav");
    let (ok, chk) = brain(&dir, &["check"]);
    assert!(ok && chk.contains("0 error(s), 0 warning(s)"), "{chk}");
    let (ok, b) = brain(&dir, &["build", "--progress=off"]);
    assert!(ok, "{b}");
    let entry = std::fs::read_to_string(dir.join("dist/blog/hello-world/index.html")).unwrap();
    assert!(entry.contains("<h1>First entry of Blog") && entry.contains("min read"), "prerendered entry: {entry}");
    assert!(dir.join("dist/blog/tag/welcome/index.html").is_file() && dir.join("dist/blog/rss.xml").is_file());
    std::fs::write(dir.join("content/blog/broken.md"), "---\ntitle: Broken\ndate: not-a-date\n---\nx\n").unwrap();
    let (ok, chk) = brain(&dir, &["check"]);
    assert!(!ok && chk.contains("content/blog/broken.md") && chk.contains("B1823"), "{chk}");
    let (ok, again) = brain(&dir, &["add", "collection", "blog"]);
    assert!(!ok && again.contains("already exists"), "{again}");
    let _ = std::fs::remove_dir_all(&tmp);
}
