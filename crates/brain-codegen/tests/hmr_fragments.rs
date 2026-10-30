//! Exact compiled wiring metadata, never source-wide string replacement or production overhead.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct App(PathBuf);
impl App {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "brain-hmr-proof-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("comp")).unwrap();
        std::fs::write(
            path.join("app.brain"),
            "<html lang=\"en\"><head><title>Fixture</title></head><body><slot /></body></html>",
        )
        .unwrap();
        Self(path)
    }
    fn compile(&self, source: &str, dev: bool) -> String {
        let runtime = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../runtime")
            .canonicalize()
            .unwrap();
        brain_codegen::compile_with(
            source,
            Path::new("comp/example.brain"),
            &brain_codegen::Options {
                dev,
                root: Some(self.0.clone()),
                runtime: runtime.join("brain.js").to_string_lossy().into_owned(),
                runtime_dev: runtime.join("brain-dev.js").to_string_lossy().into_owned(),
                ..Default::default()
            },
        )
        .unwrap_or_else(|d| panic!("{d:?}"))
        .js
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn source(label: &str, title: &str, step: u8) -> String {
    format!(
        "<script>let count = sig(0); const inc = () => count += {step}</script><button title=\"{title}\" on:click={{inc}}>{label} {{count}}</button>"
    )
}
#[test]
fn only_generated_template_literals_are_masked_and_static_edits_have_identical_wiring() {
    let app = App::new();
    let first = app.compile(&source("Before", "old", 1), true);
    let second = app.compile(&source("After", "new", 1), true);
    let logic = app.compile(&source("After", "new", 2), true);
    for (name, code) in [
        ("before.mjs", first),
        ("after.mjs", second),
        ("logic.mjs", logic),
    ] {
        std::fs::write(app.0.join(name), code).unwrap();
    }
    std::fs::write(app.0.join("assert.mjs"),"import assert from 'node:assert/strict';import A from './before.mjs';import B from './after.mjs';import C from './logic.mjs';assert.equal(A.$hmr.version,1);assert.equal(A.$hmr.wiring,B.$hmr.wiring);assert.notDeepEqual(A.$hmr.templates,B.$hmr.templates);assert.notEqual(B.$hmr.wiring,C.$hmr.wiring);assert.ok(A.$hmr.wiring.includes('__brain_static_template__'));assert.ok(A.$hmr.wiring.length<=65536);").unwrap();
    let output = std::process::Command::new("node")
        .arg(app.0.join("assert.mjs"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn production_has_no_fragment_metadata_or_dev_fragment_import() {
    let app = App::new();
    let output = app.compile(&source("Production", "title", 1), false);
    assert!(!output.contains(".$hmr"));
    assert!(!output.contains("fragment as"));
    assert!(!output.contains("__brain_static_template__"));
}
#[test]
fn excessive_metadata_declines_proof_without_blocking_compilation() {
    let app = App::new();
    let text = "x".repeat(65537);
    let output = app.compile(&format!("<p>{text}</p>"), true);
    assert!(!output.contains(".$hmr ="));
    assert!(output.contains("export default dev("));
}
