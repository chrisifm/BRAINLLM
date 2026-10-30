use std::collections::{BTreeMap, HashMap};
use std::io::Write;

#[test]
fn ledger_removes_future_plan_before_reading_offsets_and_distinguishes_dynamic_imports() {
    let html = r#"<html><head><script type="application/json" id="brain-next">{"assets":["/_b/future.js"]}</script><script type="module">import '/_b/entry.js'</script></head><body></body></html>"#;
    let files: BTreeMap<String,String> = [
        ("_b/entry.js".into(),format!("import {{ value }} from '/_b/shared.js';export const next = () => import('/_b/lazy.js');console.log(value);export const text='{}';", "compressible".repeat(100))),
        ("_b/shared.js".into(),format!("export const value='{}';", "shared".repeat(100))),
        ("_b/lazy.js".into(),format!("export const later='{}';", "deferred".repeat(100))),
        ("_b/future.js".into(),"export const future=true".into()),
    ].into();
    let mut binary = BTreeMap::new();
    for (path, source) in &files {
        binary.insert(
            format!("{path}.gz"),
            brain_pack::gzip::gzip(source.as_bytes(), 1),
        );
        let mut compressed = Vec::new();
        {
            let mut writer = brotli::CompressorWriter::new(&mut compressed, 4096, 4, 22);
            writer.write_all(source.as_bytes()).unwrap();
        }
        binary.insert(format!("{path}.br"), compressed);
    }
    let ledger = brain_pack::ledger::page(
        "/",
        "index.html",
        html,
        "/",
        &files,
        &binary,
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        ledger.assets.len(),
        3,
        "future navigation plan is not an asset dependency"
    );
    assert!(
        ledger
            .assets
            .iter()
            .find(|asset| asset.path == "_b/entry.js")
            .unwrap()
            .initial
    );
    assert!(
        ledger
            .assets
            .iter()
            .find(|asset| asset.path == "_b/shared.js")
            .unwrap()
            .initial
    );
    assert!(
        !ledger
            .assets
            .iter()
            .find(|asset| asset.path == "_b/lazy.js")
            .unwrap()
            .initial
    );
    assert_eq!(
        ledger.entry_js.raw,
        files["_b/entry.js"].len() + files["_b/shared.js"].len()
    );
    assert_eq!(ledger.deferred_js.raw, files["_b/lazy.js"].len());
    assert_eq!(
        ledger.conservative_js.raw,
        ledger.entry_js.raw + ledger.deferred_js.raw
    );
    assert!(ledger.entry_js.gzip < ledger.entry_js.raw);
    assert!(ledger.entry_js.brotli < ledger.entry_js.raw);
    assert!(ledger.to_json().contains("\"initial\":false"));
}

#[test]
fn json_metadata_and_island_state_are_deferred_but_preloads_are_initial() {
    let html = r#"<link rel="modulepreload" href="/app/_b/preloaded.js"><script type="b/state">{"view":"/app/_b/view.js"}</script>"#;
    let files = [
        ("_b/preloaded.js".into(), "export const ready = true".into()),
        ("_b/view.js".into(), "export default () => null".into()),
    ]
    .into();
    let ledger = brain_pack::ledger::page(
        "/app/",
        "index.html",
        html,
        "/app/",
        &files,
        &BTreeMap::new(),
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(ledger.inline_js_bytes, 0);
    assert_eq!(
        ledger.assets.iter().filter(|asset| asset.initial).count(),
        1
    );
    assert_eq!(
        ledger.entry_js.gzip, ledger.entry_js.raw,
        "missing codec means raw serving fallback"
    );
}

#[test]
fn zero_js_proof_rejects_external_uppercase_and_legacy_script_tags() {
    for html in [
        "<SCRIPT SRC='/widget.js'></SCRIPT>",
        "<script type='text/ecmascript'>run()</script>",
        "<script type='module'></script>",
    ] {
        assert!(brain_pack::cache::has_executable_scripts(html));
    }
    assert!(!brain_pack::cache::has_executable_scripts(
        "<SCRIPT TYPE='application/ld+json'>{}</SCRIPT><script type='b/state'>{}</script>"
    ));
}
