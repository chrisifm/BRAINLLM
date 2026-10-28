//! Production smoke: lazy code stays in its own emitted module and the interaction trigger is usable HTML.
use std::path::Path;
use std::process::{Command, Stdio};

fn brain(root: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(root).stdin(Stdio::null()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{args:?}: {text}");
    text
}

#[test]
fn lazy_interaction_and_img_survive_production_packaging() {
    let root = std::env::temp_dir().join(format!("brain-lazy-build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    brain(&root, &["new", "app"]);
    let app = root.join("app");
    std::fs::write(app.join("comp/chart.brain"), "<p>Lazy chart payload</p><style>p { font-style: italic; }</style>").unwrap();
    let page = std::fs::read_to_string(app.join("views/about.brain")).unwrap()
        .replace("<main", "<script>\nimport Chart from '../comp/chart.brain'\n</script>\n<main")
        .replace("</main>", "<Chart lazy=\"interaction\" /><Img src=\"/favicon.svg\" alt=\"Application symbol\" width=\"64\" height=\"64\" /></main>");
    std::fs::write(app.join("views/about.brain"), page).unwrap();
    brain(&app, &["build", "--progress=off"]);
    let html = std::fs::read_to_string(app.join("dist/about/index.html")).unwrap();
    assert!(html.contains("Load component") && html.contains("data-lazy"), "missing accessible lazy trigger: {html}");
    assert!(html.contains("decoding=\"async\"") && html.contains("fetchpriority=\"high\""), "missing Img attributes");
    assert!(html.contains("font-style:italic"), "lazy component CSS remains in the route stylesheet");
    let mut lazy_chunk = None;
    let mut has_dynamic_import = false;
    for entry in std::fs::read_dir(app.join("dist/_b")).unwrap().flatten() {
        if entry.path().extension().is_some_and(|e| e == "js") {
            let src = std::fs::read_to_string(entry.path()).unwrap();
            if src.contains("Lazy chart payload") { lazy_chunk = Some(entry.file_name().to_string_lossy().into_owned()); }
            has_dynamic_import |= src.contains("import(");
        }
    }
    let chunk = lazy_chunk.expect("the lazy component must be emitted");
    assert!(has_dynamic_import, "production retains its dynamic import");
    assert!(!html.contains(&format!("href=\"/_b/{chunk}\"")), "lazy-only chunk must not be preloaded by the initial route");
    std::fs::remove_dir_all(root).unwrap();
}
