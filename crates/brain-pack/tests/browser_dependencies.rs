//! Actual production graph: installed ESM, named export removal, retained effects and Node evaluation of emitted SDK chunk.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct App(PathBuf);
impl App {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "brain-browser-deps-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("views")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/demo/dist")).unwrap();
        std::fs::write(root.join("node_modules/demo/package.json"),r#"{"name":"demo","version":"1.0.0","exports":{"browser":"./dist/index.mjs","default":"./dist/node.cjs"},"sideEffects":true}"#).unwrap();
        std::fs::write(root.join("node_modules/demo/dist/index.mjs"),format!("export function used(){{return 7}} export function omitted(){{return '{}'}} globalThis.sdkEffect = (globalThis.sdkEffect || 0) + 1; console.log;", "unused-text-".repeat(800))).unwrap();
        std::fs::write(root.join("views/home.brain"),"<meta seo title=\"SDK example\" description=\"Offline SDK fixture\"><script>import { used } from 'demo'; let n = sig(0)</script><button on:click={() => n = used()}>{n}</button>").unwrap();
        Self(root)
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn build(app: &App) -> brain_pack::Output {
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let core =
        brain_min::minify_runtime_core(&std::fs::read_to_string(runtime.join("brain.js")).unwrap())
            .unwrap();
    let compile = |source: &str, file: &Path| -> Result<brain_pack::Unit, String> {
        let o = brain_codegen::compile_with(
            source,
            file,
            &brain_codegen::Options {
                root: Some(app.0.clone()),
                ..Default::default()
            },
        )
        .map_err(|d| format!("{d:?}"))?;
        Ok(brain_pack::Unit {
            js: o.js,
            css: o.css,
            server_js: o.server_js,
            server_fns: o.server_fns,
        })
    };
    let input = brain_pack::Input {
        root: app.0.clone(),
        compile: &compile,
        runtime_core: core,
        runtime_siblings: std::fs::read_dir(&runtime)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "js")
                    && !["brain-router.js", "brain-dev.js", "brain-inspect.js"]
                        .iter()
                        .any(|name| path.ends_with(name))
            })
            .map(|path| {
                (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(path).unwrap(),
                )
            })
            .collect(),
        layer_order: String::new(),
        minify: true,
        site: None,
        shim: None,
        generator: String::new(),
        server_module: None,
        cache: Default::default(),
        server_mjs: None,
        progress: None,
        post: None,
    };
    brain_pack::build(
        &input,
        &[("/".into(), "views/home.brain".into())],
        &|_| vec![],
        &|_| vec![],
    )
    .unwrap()
}
#[test]
fn production_emits_only_reachable_esm_and_preserves_sdk_contracts_and_effects() {
    let app = App::new();
    let output = build(&app);
    let sdk = output
        .files
        .iter()
        .find(|(name, source)| name.starts_with("_b/") && source.contains("sdkEffect"))
        .expect("emitted SDK chunk");
    assert!(sdk.1.contains("used"));
    let measured = output
        .ledger
        .iter()
        .flat_map(|page| &page.assets)
        .find(|asset| asset.path == *sdk.0)
        .expect("MJS SDK counted in the route ledger");
    assert_eq!(measured.kind, "js");
    assert_eq!(measured.bytes.raw, sdk.1.len());
    assert!(
        output
            .report
            .iter()
            .any(|route| route.js_min >= sdk.1.len())
    );
    assert!(!sdk.1.contains("omitted"));
    assert!(
        sdk.1.contains("console.log"),
        "SDK log reads are external effects, not app debug calls"
    );
    assert!(
        sdk.1.len() < 1000,
        "real module byte reduction: {}",
        sdk.1.len()
    );
    assert!(
        output
            .optimizations
            .iter()
            .any(|o| o.stage == "foreign-esm-tree-shake"
                && o.source == "node_modules/demo/dist/index.mjs")
    );
    let run = app.0.join("emitted");
    std::fs::create_dir_all(run.join("_b")).unwrap();
    std::fs::write(run.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::create_dir_all(run.join(sdk.0).parent().unwrap()).unwrap();
    std::fs::write(run.join(sdk.0), sdk.1).unwrap();
    let script = format!(
        "const sdk=await import({}); if(sdk.used()!==7 || globalThis.sdkEffect!==1 || 'omitted' in sdk)process.exit(1)",
        brain_mcp::Json::str(run.join(sdk.0).to_string_lossy()).stringify()
    );
    let status = std::process::Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .status()
        .expect("Node required for emitted module regression");
    assert!(status.success());
}

#[test]
fn sdk_wasm_and_raw_workers_preserve_reachable_layout_and_immutable_namespace() {
    let app = App::new();
    let dir = app.0.join("node_modules/demo/dist");
    std::fs::write(dir.join("index.mjs"),"export function used(){return 7} export function wasmURL(){return new URL('module.wasm',import.meta.url).href} export function workerURL(){return new URL('worker.mjs',import.meta.url).href} export function proxyURL(){return new URL('proxy.js',import.meta.url).href}").unwrap();
    std::fs::write(dir.join("worker.mjs"),"export function proxyURL(){const filename='proxy.js';return new URL(filename,import.meta.url).href}").unwrap();
    std::fs::write(
        dir.join("proxy.js"),
        "globalThis.rawProxyRuns = (globalThis.rawProxyRuns || 0) + 1;",
    )
    .unwrap();
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    wasm.extend([0, 0x81, 0x04, 0]);
    wasm.extend([0; 512]);
    std::fs::write(dir.join("module.wasm"), &wasm).unwrap();
    std::fs::write(app.0.join("views/home.brain"),"<meta seo title=\"SDK assets\" description=\"Offline asset fixture\"><script>import { used,wasmURL,workerURL,proxyURL } from 'demo'; let n=sig(0)</script><button on:click={() => n=used()+wasmURL().length+workerURL().length+proxyURL().length}>{n}</button>").unwrap();
    let output = build(&app);
    let (wasm_key, bytes) = output
        .binary
        .iter()
        .find(|(name, _)| name.ends_with("/dist/module.wasm"))
        .expect("emitted Wasm asset");
    assert_eq!(bytes, &wasm);
    assert!(output.files["_headers"].contains("'wasm-unsafe-eval'"));
    assert!(!output.files["_headers"].contains("'unsafe-eval'"));
    assert!(wasm_key.starts_with("_b/npm/"));
    assert!(output.binary.contains_key(&format!("{wasm_key}.gz")));
    assert!(output.binary.contains_key(&format!("{wasm_key}.br")));
    let (worker_key, worker) = output
        .files
        .iter()
        .find(|(name, _)| name.ends_with("/dist/worker.mjs"))
        .expect("raw SDK worker");
    let (proxy_key, proxy) = output
        .files
        .iter()
        .find(|(name, _)| name.ends_with("/dist/proxy.js"))
        .expect("raw SDK proxy");
    assert_eq!(
        Path::new(worker_key).parent(),
        Path::new(proxy_key).parent()
    );
    assert!(!worker.contains("$serve"));
    assert!(!proxy.contains("$serve"));
    let emitted = app.0.join("asset-check");
    for (name, source) in [(worker_key, worker), (proxy_key, proxy)] {
        std::fs::create_dir_all(emitted.join(name).parent().unwrap()).unwrap();
        std::fs::write(emitted.join(name), source).unwrap();
    }
    std::fs::write(emitted.join(wasm_key), &wasm).unwrap();
    let script = format!(
        "import fs from 'node:fs';const worker=await import({});if(!fs.existsSync(new URL(worker.proxyURL()))||!WebAssembly.validate(fs.readFileSync({})))process.exit(1)",
        brain_mcp::Json::str(emitted.join(worker_key).to_string_lossy()).stringify(),
        brain_mcp::Json::str(emitted.join(wasm_key).to_string_lossy()).stringify()
    );
    assert!(
        std::process::Command::new("node")
            .args(["--input-type=module", "-e", &script])
            .status()
            .unwrap()
            .success()
    );
    let first_prefix = wasm_key.clone();
    *wasm.last_mut().unwrap() = 1;
    std::fs::write(dir.join("module.wasm"), wasm).unwrap();
    let changed = build(&app);
    assert!(
        !changed.binary.contains_key(&first_prefix),
        "asset changes invalidate immutable namespace"
    );
}
#[test]
fn literal_url_assets_exclude_custom_url_bindings_and_binary_worker_wrappers() {
    let source = "const wasm = new URL('./engine.wasm',import.meta.url);const worker = new URL('./parse.worker.js',import.meta.url);";
    assert_eq!(brain_pack::url_refs(source).unwrap().len(), 2);
    assert_eq!(brain_pack::worker_refs(source).unwrap().len(), 1);
    assert!(
        brain_pack::url_refs("function f(URL){return new URL('./private.wasm',import.meta.url)}")
            .unwrap()
            .is_empty()
    );
}

/// Optional public SDK artifact fixture. Set explicitly in the local verification command; never a build network fetch.
#[test]
#[ignore = "requires an explicitly verified official SQLite SDK artifact and local Chrome"]
fn official_sqlite_sdk_installed_esm_and_wasm_build_without_remote_execution() {
    let package = std::env::var_os("BRAIN_SQLITE_SDK_FIXTURE")
        .expect("set verified SDK package directory explicitly");
    let package = PathBuf::from(package);
    let app = App::new();
    let installed = app.0.join("node_modules/@sqlite.org/sqlite-wasm");
    std::fs::create_dir_all(installed.join("dist")).unwrap();
    for file in [
        "package.json",
        "dist/index.mjs",
        "dist/sqlite3-worker1.mjs",
        "dist/sqlite3-opfs-async-proxy.js",
        "dist/sqlite3.wasm",
    ] {
        let source = package.join(file);
        let meta = std::fs::symlink_metadata(&source).unwrap();
        assert!(meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 1024 * 1024);
        std::fs::copy(source, installed.join(file)).unwrap();
    }
    std::fs::create_dir(app.0.join("lib")).unwrap();
    std::fs::write(app.0.join("lib/data.worker.jsbrain"),"import initialize from '@sqlite.org/sqlite-wasm'; const db=sqlite(initialize,{namespace:'proof',name:'data'}); export default {query:(sql,bind=[])=>db.query(sql,bind),execute:(sql,bind=[])=>db.execute(sql,bind),batch:statements=>db.batch(statements),close:()=>db.close()}").unwrap();
    std::fs::write(app.0.join("views/home.brain"),"<meta seo title=\"SQLite SDK\" description=\"Offline SDK graph fixture\"><script>import db from '../lib/data.worker.jsbrain'; let status=sig('idle'); const load=async()=>{const rows=await db.query('SELECT 1 AS value');status=String(rows[0].value)}</script><button on:click={load}>{status}</button>").unwrap();

    let output = build(&app);
    assert!(
        output
            .binary
            .keys()
            .any(|file| file.ends_with("/dist/sqlite3.wasm"))
    );
    assert!(
        output
            .files
            .keys()
            .any(|file| file.ends_with("/dist/sqlite3-worker1.mjs"))
    );
    assert!(
        output
            .files
            .keys()
            .any(|file| file.ends_with("/dist/sqlite3-opfs-async-proxy.js"))
    );
    let proxy = output
        .files
        .iter()
        .find(|(file, _)| file.ends_with("/dist/sqlite3-opfs-async-proxy.js"))
        .unwrap();
    assert!(!proxy.1.contains("$serve"));
    let emitted = app.0.join("compiled-browser");
    for (path, source) in &output.files {
        let file = emitted.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, source).unwrap();
    }
    for (path, bytes) in &output.binary {
        let file = emitted.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    let browser = std::process::Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sqlite-compiled-browser.mjs"))
        .arg(&emitted)
        .output()
        .expect("Node and Chrome required for explicit official SDK browser fixture");
    assert!(
        browser.status.success(),
        "compiled SQLite browser failed: {} {}",
        String::from_utf8_lossy(&browser.stdout),
        String::from_utf8_lossy(&browser.stderr)
    );
    println!(
        "compiled SQLite browser headers/proof: {}",
        String::from_utf8_lossy(&browser.stdout).trim()
    );
    println!(
        "official SQLite SDK built: {} text assets, {} binary assets; Wasm present",
        output.files.len(),
        output.binary.len()
    );
}
