//! #260 end to end (INTERNALS §11.6, QA conformance 135): `brain build` renames the class names of component
//! stylesheets in the CSS, the templates and `class:x` toggles, so no source class name reaches dist/.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|x| x == "js" || x == "css" || x == "html" || x == "mjs") {
            out.push(p);
        }
    }
}

#[test]
fn no_source_class_name_reaches_dist() {
    let tmp = std::env::temp_dir().join(format!("bclasses-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    std::fs::write(
        dir.join("comp/zebra-card.brain"),
        "<script>\n  let on = sig(true)\n</script>\n<div class=\"zebra-shell\"><button type=\"button\" class=\"zebra-title\" class:zebra-active={on} on:click={on = !on}>Zebra</button></div>\n<style>\n  .zebra-shell { padding: var(--space-2) }\n  .zebra-title { font-weight: 600 }\n  .zebra-active { color: var(--accent) }\n</style>\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("views/zebra.brain"),
        "<meta seo title=\"Zebra\" description=\"A page that renders the zebra card\">\n<script>\n  import ZebraCard from '../comp/zebra-card.brain'\n</script>\n<main><h1>Zebra</h1><ZebraCard /></main>\n",
    )
    .unwrap();
    let o = brain(&dir, &["build"]);
    assert!(o.status.success(), "{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let mut all = Vec::new();
    files(&dir.join("dist"), &mut all);
    assert!(!all.is_empty());
    for p in &all {
        let s = std::fs::read_to_string(p).unwrap_or_default();
        for name in ["zebra-shell", "zebra-title", "zebra-active"] {
            assert!(!s.contains(name), "`{name}` survived in {}", p.display());
        }
    }
    let page = std::fs::read_to_string(dir.join("dist/zebra/index.html")).unwrap();
    assert!(page.contains("Zebra"), "{page}");
    let _ = std::fs::remove_dir_all(&tmp);
}
