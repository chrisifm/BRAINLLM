//! #123 (INTERNALS §35): `*.worker.jsbrain` modules run in a Web Worker; importers get async stubs.
//! Snapshots in tests/fixtures/worker/*.out (`UPDATE_SNAPSHOTS=1` rewrites them).

use std::path::Path;

fn compile(rel: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/worker");
    let src = std::fs::read_to_string(root.join(rel)).unwrap();
    let opts = brain_codegen::Options { root: Some(root), ..Default::default() };
    brain_codegen::compile_with(&src, Path::new(rel), &opts).unwrap_or_else(|d| panic!("{rel}: {d:#?}")).js
}

fn snapshot(rel: &str, got: &str) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/worker").join(format!("{}.out", rel.replace('/', "__")));
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() || !file.exists() {
        std::fs::write(&file, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&file).unwrap();
    assert_eq!(got, want, "{rel} differs from {}", file.display());
}

#[test]
fn worker_chunk_serves_its_exports() {
    let js = compile("lib/stats.worker.jsbrain");
    assert!(js.contains("const $default = { mean, sorted };\nexport default $default;"), "{js}");
    assert!(js.contains("import { serve as $serve } from \"/@brain/brain-worker.js\";\n$serve({ default: $default });"), "{js}");
    snapshot("lib/stats.worker.jsbrain", &js);
    let js = compile("lib/parse.worker.jsbrain");
    assert!(js.contains("$serve({ parseCsv });"), "{js}");
    snapshot("lib/parse.worker.jsbrain", &js);
}

#[test]
fn importer_gets_async_stubs() {
    let js = compile("comp/report.brain");
    assert_eq!(js.matches("import { worker as $worker }").count(), 1, "{js}");
    assert!(js.contains("const stats = { mean: (...a) => $w0.call(\"default.mean\", a), sorted: (...a) => $w0.call(\"default.sorted\", a) };"), "{js}");
    assert!(js.contains("const parseCsv = (...a) => $w1.call(\"parseCsv\", a);"), "{js}");
    assert!(!js.contains("import stats from"), "the worker module is never imported on the main thread:\n{js}");
    snapshot("comp/report.brain", &js);
}

#[test]
fn api_type_names_become_imported_objects() {
    // #36 (INTERNALS §15): `out: '[User]'` → `out: [User]` with `User` from the app's types.jsbrain.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/types");
    let src = std::fs::read_to_string(root.join("api/users.jsbrain")).unwrap();
    let opts = brain_codegen::Options { root: Some(root), ..Default::default() };
    let js = brain_codegen::compile_with(&src, Path::new("api/users.jsbrain"), &opts).unwrap_or_else(|d| panic!("{d:#?}")).js;
    assert!(js.contains("import { User } from \"../types.jsbrain\";"), "{js}");
    assert!(js.contains("{ get: { out: [User], cache: '10m' }, post: { in: User, out: User } }"), "{js}");
}
