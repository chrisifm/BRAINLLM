//! Opt-in changes private server metadata/CSP, never the legacy resource graph.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> Fixture {
    let path = std::env::temp_dir().join(format!(
        "brain-pack-live-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    fs::create_dir(path.join("views")).unwrap();
    fs::write(
        path.join("app.brain"),
        "<html lang=\"en\"><head><title>Live resources</title></head><body><slot /></body></html>",
    )
    .unwrap();
    Fixture(path)
}
fn build(
    root: &Path,
    live: bool,
    options: Option<&brain_pack::BuildOptions>,
    omit_live_module: bool,
) -> brain_pack::Output {
    let source = if live {
        "<script>const rows=resource(()=> Promise.resolve([]))\nconst updates=liveResource(rows,{rooms:[{resource:'posts'}],scope:()=> null})</script><main><p>{updates.state}</p></main>"
    } else {
        "<script>const rows=resource(()=> Promise.resolve([]))</script><main><p>{rows.done ? 'Done' : 'Waiting'}</p></main>"
    };
    let source = format!("<meta seo title=\"Live resource fixture\" description=\"Native live resource fixture output\" canonical=\"/\">\n{source}");
    fs::write(root.join("views/index.brain"), source).unwrap();
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let siblings = fs::read_dir(&runtime)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "js")
                && !path.ends_with("brain-dev.js")
                && (!omit_live_module || !path.ends_with("brain-live.js"))
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read_to_string(path).unwrap(),
            )
        })
        .collect();
    let compile = |source: &str, path: &Path| {
        let options = brain_codegen::Options {
            root: Some(root.to_path_buf()),
            ..Default::default()
        };
        let output = brain_codegen::compile_with(source, path, &options)
            .map_err(|diags| format!("{diags:#?}"))?;
        Ok(brain_pack::Unit {
            js: output.js,
            css: output.css,
            server_js: output.server_js,
            server_fns: output.server_fns,
        })
    };
    let input = brain_pack::Input {
        root: root.to_path_buf(),
        compile: &compile,
        runtime_core: brain_min::minify_runtime_core(
            &fs::read_to_string(runtime.join("brain.js")).unwrap(),
        )
        .unwrap(),
        runtime_siblings: siblings,
        layer_order: String::new(),
        minify: true,
        site: Some("https://app.example.test/nested/".into()),
        shim: fs::read_to_string(runtime.join("shim/dom.js")).ok(),
        generator: String::new(),
        server_module: None,
        cache: Default::default(),
        server_mjs: fs::read_to_string(runtime.join("server/server.mjs")).ok(),
        progress: None,
        post: None,
    };
    let views = [("/".into(), "views/index.brain".into())];
    match options {
        Some(options) => {
            brain_pack::build_with_options(&input, &views, &|_| vec![], &|_| vec![], options)
        }
        None => brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]),
    }
    .unwrap()
}
#[test]
fn unused_live_support_adds_no_asset_bytes_or_output_changes_to_existing_resource_apps() {
    let root = fixture();
    let before = build(&root.0, false, None, true);
    let after = build(
        &root.0,
        false,
        Some(&brain_pack::BuildOptions::default()),
        false,
    );
    assert!(
        before.files == after.files,
        "Adding an unused live module must not alter emitted text"
    );
    assert!(
        before.binary == after.binary,
        "Adding an unused live module must not alter emitted asset bytes"
    );
    assert_eq!(before.signature, after.signature);
    assert!(!after.files.contains_key("_server/manifest.json"));
    assert!(after.ledger.iter().all(|row| {
        row.assets
            .iter()
            .all(|asset| asset.source.as_deref() != Some("runtime/brain-live.js"))
    }));
}
#[test]
fn opted_static_app_has_private_hashed_configuration_and_exact_public_socket_csp_without_node_conversion()
 {
    let root = fixture();
    let options = brain_pack::BuildOptions {
        native_sockets: Some(
            brain_pack::NativeSocketOptions::new("https://app.example.test").unwrap(),
        ),
    };
    let first = build(&root.0, true, Some(&options), false);
    let manifest = brain_mcp::Json::parse(&first.files["_server/manifest.json"]).unwrap();
    assert_eq!(
        manifest.get("nativeSockets"),
        Some(&brain_mcp::Json::Bool(true))
    );
    assert_eq!(
        manifest
            .get("publicOrigin")
            .and_then(brain_mcp::Json::as_str),
        Some("https://app.example.test")
    );
    assert!(
        !first.files.contains_key("server.mjs"),
        "Enabling a native bridge must not force a static app to Node/SSR"
    );
    assert!(first.files["_headers"].contains("wss://app.example.test"));
    assert!(!first.files["_headers"].contains("ws:*"));
    assert!(first.ledger.iter().any(|row| {
        row.assets
            .iter()
            .any(|asset| asset.source.as_deref() == Some("runtime/brain-live.js"))
    }));
    let second = build(&root.0, true, Some(&options), false);
    assert_eq!(first.signature, second.signature);
    let other = build(
        &root.0,
        true,
        Some(&brain_pack::BuildOptions {
            native_sockets: Some(
                brain_pack::NativeSocketOptions::new("http://localhost:4000").unwrap(),
            ),
        }),
        false,
    );
    assert_ne!(
        first.signature, other.signature,
        "Changing only typed socket options must invalidate the build signature"
    );
    assert!(other.files["_headers"].contains("ws://localhost:4000"));
}
#[test]
fn exact_socket_csp_preserves_explicit_sources_and_legacy_policy() {
    let socket = brain_pack::NativeSocketOptions::new("https://app.example.test").unwrap();
    let mut extra = vec![(
        "connect-src".into(),
        "wss://third-party.example.test".into(),
    )];
    extra.extend(brain_pack::cache::native_socket_csp(&socket));
    let csp = brain_pack::cache::csp(&[], &extra);
    assert!(csp.contains("wss://app.example.test"));
    assert!(csp.contains("wss://third-party.example.test"));
    assert!(
        csp.contains(" wss:"),
        "Broad legacy policy hardening is a separate compatibility change"
    );
    assert!(!csp.contains("ws:*") && !csp.contains("wss:*"));
}
