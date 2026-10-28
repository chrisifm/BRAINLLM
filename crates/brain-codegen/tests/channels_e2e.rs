//! #30: QA's channels-app (INTERNALS §12, the 8 channels) compiled and run on runtime/brain.js + brain-ch.js in
//! Node on the DOM shim. Skipped when `node` is missing.

use std::path::Path;
use std::process::Command;

#[test]
fn eight_channels_run_on_the_runtime() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("node is not installed: skipping");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let app = root.join("bench/fixtures-pending/channels-app");
    let runtime = root.join("runtime/brain.js");
    let runtime_url = format!("file://{}", runtime.display());
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("channels-e2e");
    let _ = std::fs::remove_dir_all(&out);
    for rel in [
        "channels/cart.jsbrain",
        "channels/theme.jsbrain",
        "channels/toast.jsbrain",
        "comp/cart-summary.brain",
        "comp/confirm.brain",
        "comp/product-card.brain",
        "comp/qty-input.brain",
        "comp/toaster.brain",
        "ui/panel.brain",
        "ui/price.brain",
        "views/shop.brain",
    ] {
        let src = std::fs::read_to_string(app.join(rel)).unwrap();
        let opts = brain_codegen::Options { runtime: runtime_url.clone(), root: Some(app.clone()), import_suffix: ".js".into(), ..Default::default() };
        let o = brain_codegen::compile_with(&src, Path::new(rel), &opts).unwrap_or_else(|d| panic!("{rel}: {d:#?}"));
        let dest = out.join(format!("{rel}.js"));
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(dest, o.js).unwrap();
    }
    let r = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/channels_e2e.mjs"))
        .arg(&out)
        .arg(root.join("runtime/shim/dom.js"))
        .env("BRAIN_RUNTIME_URL", &runtime_url)
        .output()
        .unwrap();
    let so = String::from_utf8_lossy(&r.stdout);
    let se = String::from_utf8_lossy(&r.stderr);
    assert!(r.status.success() && so.contains("ok channels"), "channels e2e failed:\n{so}\n{se}");
}
