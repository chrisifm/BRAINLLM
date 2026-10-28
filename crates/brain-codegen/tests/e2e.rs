//! Compiles counter, todo, user-form, badge and dashboard and runs them on runtime/brain.js (Node + DOM shim).
//! Skipped when `node` is not installed.

use std::path::Path;
use std::process::Command;

#[test]
fn counter_and_todo_run_on_the_runtime() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("node is not installed: skipping e2e");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let runtime = std::env::var("BRAIN_RUNTIME").map(|p| root.join(p)).unwrap_or_else(|_| root.join("runtime/brain.js"));
    let runtime_url = format!("file://{}", runtime.display());
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("e2e");
    std::fs::create_dir_all(&out).unwrap();
    for rel in ["comp/counter.brain", "comp/todo.brain", "comp/user-form.brain", "ui/badge.brain", "views/dashboard.brain", "lib/format.jsbrain"] {
        let src = std::fs::read_to_string(root.join("examples").join(rel)).unwrap();
        let opts = brain_codegen::Options { runtime: runtime_url.clone(), root: Some(root.join("examples")), import_suffix: ".js".into(), ..Default::default() };
        let o = brain_codegen::compile_with(&src, Path::new(rel), &opts).unwrap_or_else(|d| panic!("{d:?}"));
        let dest = out.join(format!("{rel}.js"));
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        if !o.server_js.is_empty() {
            std::fs::write(out.join(format!("{rel}.server.js")), &o.server_js).unwrap();
        }
        std::fs::write(dest, o.js).unwrap();
    }
    // Dev build of the counter (HMR: dev() wrapper + named sigs), twice to simulate an edit.
    let dev_url = format!("file://{}", runtime.with_file_name("brain-dev.js").display());
    let src = std::fs::read_to_string(root.join("examples/comp/counter.brain")).unwrap();
    for (name, src) in [("counter.dev.js", src.clone()), ("counter.dev2.js", src.replace("× 2 =", "double ="))] {
        let opts = brain_codegen::Options { dev: true, runtime: runtime_url.clone(), runtime_dev: dev_url.clone(), root: Some(root.join("examples")), ..Default::default() };
        let o = brain_codegen::compile_with(&src, Path::new("comp/counter.brain"), &opts).unwrap_or_else(|d| panic!("{d:?}"));
        std::fs::write(out.join(name), o.js).unwrap();
    }
    std::fs::create_dir_all(out.join("lib")).unwrap();
    std::fs::copy(root.join("examples/lib/api.js"), out.join("lib/api.js")).unwrap();
    let r = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e.mjs"))
        .arg(&out)
        .arg(root.join("runtime/shim/dom.js"))
        .env("BRAIN_RUNTIME_URL", &runtime_url)
        .env("BRAIN_DEV_URL", &dev_url)
        .output()
        .unwrap();
    let so = String::from_utf8_lossy(&r.stdout);
    let se = String::from_utf8_lossy(&r.stderr);
    assert!(r.status.success(), "e2e failed:\n{so}\n{se}");
    assert!(so.contains("ok counter") && so.contains("ok todo") && so.contains("ok dashboard") && so.contains("ok dev-hmr"), "{so}");
}
