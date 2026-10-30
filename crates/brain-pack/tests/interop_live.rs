//! Static-prop client widgets must mount on load without a watch or user event.
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn client_widget_autoimports_interop_and_mounts_without_an_event() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(
        std::env::temp_dir().join(format!("brain-pack-interop-{}-{stamp}", std::process::id())),
    );
    let root = &scratch.0;
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::write(root.join("app.brain"),
        "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Widget</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(root.join("views/index.brain"),
        "<meta seo title=\"Widget\" description=\"A client widget with static props.\" robots=\"noindex\">\n<script>import Widget from '../comp/widget.brain'</script>\n<main><h1>Widget</h1><Widget /></main>").unwrap();
    std::fs::write(
        root.join("comp/widget.brain"),
        r#"<meta client height="320px">
<script>
  let host = ref()
  interop(() => host, { mount: (node, props) => {
    node.textContent = props.title
    return () => { node.textContent = '' }
  } }, { title: 'Ready' })
</script>
<div bind:ref={host} aria-label="Widget"></div>
<style>div { min-height: 320px; }</style>
"#,
    )
    .unwrap();
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let siblings = std::fs::read_dir(&runtime)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "js")
                && !["brain.js", "brain-dev.js", "brain-inspect.js"]
                    .iter()
                    .any(|name| path.ends_with(name))
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(path).unwrap(),
            )
        })
        .collect();
    let compile = |source: &str, path: &Path| {
        let options = brain_codegen::Options {
            dev: false,
            root: Some(root.clone()),
            ..Default::default()
        };
        let output = brain_codegen::compile_with(source, path, &options)
            .map_err(|diags| format!("{}: {diags:?}", path.display()))?;
        if path == Path::new("comp/widget.brain") {
            assert!(
                output.js.contains("brain-interop.js"),
                "interop must be auto-imported"
            );
        }
        Ok(brain_pack::Unit {
            js: output.js,
            css: output.css,
            server_js: output.server_js,
            server_fns: output.server_fns,
        })
    };
    let input = brain_pack::Input {
        root: root.clone(),
        compile: &compile,
        runtime_core: brain_min::minify_runtime_core(
            &std::fs::read_to_string(runtime.join("brain.js")).unwrap(),
        )
        .unwrap(),
        runtime_siblings: siblings,
        layer_order: String::new(),
        minify: true,
        site: None,
        shim: std::fs::read_to_string(runtime.join("shim/dom.js")).ok(),
        generator: String::new(),
        server_module: None,
        cache: Default::default(),
        server_mjs: None,
        progress: None,
        post: None,
    };
    let output = brain_pack::build(
        &input,
        &[("/".into(), "views/index.brain".into())],
        &|_| vec![],
        &|_| vec![],
    )
    .unwrap();
    let html = &output.files["index.html"];
    assert!(
        html.contains("e.$d=m("),
        "static widget must mount on load: {html}"
    );
    assert!(
        !html.contains("import{h}"),
        "widget must not wait for island activation: {html}"
    );
    assert!(
        !output
            .warnings
            .iter()
            .any(|warning| warning.contains("prerender threw")),
        "{:?}",
        output.warnings
    );
}
