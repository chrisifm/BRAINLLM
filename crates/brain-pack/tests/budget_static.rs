//! Preventive runtime imports must not turn static production pages into interactive islands.
use std::path::{Path, PathBuf};
struct App(PathBuf);
impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn static_and_constant_pages_keep_zero_js_while_real_handlers_keep_execution_budgets() {
    let root = std::env::temp_dir().join(format!("brain-budget-static-{}", std::process::id()));
    std::fs::create_dir_all(root.join("views")).unwrap();
    let app = App(root.canonicalize().unwrap());
    std::fs::write(
        app.0.join("app.brain"),
        "<html lang=\"en\"><head><title>Fixture</title></head><body><slot /></body></html>",
    )
    .unwrap();
    for (file, source) in [
        ("static", "<main><h1>Static output</h1></main>"),
        (
            "constant",
            "<script>const label = 'Constant output'</script><main><h1>{label}</h1></main>",
        ),
        (
            "interactive",
            "<script>let count = sig(0)</script><main><button on:click={() => count++}>{count}</button></main>",
        ),
    ] {
        std::fs::write(app.0.join(format!("views/{file}.brain")), format!("<meta seo title=\"Runtime fixture\" description=\"Runtime fixture\" robots=\"noindex\">\n{source}")).unwrap();
    }
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let core =
        brain_min::minify_runtime_core(&std::fs::read_to_string(runtime.join("brain.js")).unwrap())
            .unwrap();
    let siblings = std::fs::read_dir(&runtime)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "js")
                && !p.ends_with("brain-dev.js")
                && !p.ends_with("brain-inspect.js")
        })
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(p).unwrap(),
            )
        })
        .collect();
    let compile_root = app.0.clone();
    let compile = move |source: &str, path: &Path| -> Result<brain_pack::Unit, String> {
        let output = brain_codegen::compile_with(
            source,
            path,
            &brain_codegen::Options {
                root: Some(compile_root.clone()),
                ..Default::default()
            },
        )
        .map_err(|d| format!("{d:?}"))?;
        if path == Path::new("views/interactive.brain") {
            assert!(output.js.contains("brain-budget.js"));
        } else {
            assert!(!output.js.contains("brain-budget.js"));
        }
        Ok(brain_pack::Unit {
            js: output.js,
            css: output.css,
            server_js: output.server_js,
            server_fns: output.server_fns,
        })
    };
    let input = brain_pack::Input {
        root: app.0.clone(),
        compile: &compile,
        runtime_core: core,
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
    let views = vec![
        ("/".into(), "views/static.brain".into()),
        ("/constant".into(), "views/constant.brain".into()),
        ("/interactive".into(), "views/interactive.brain".into()),
    ];
    let measured = brain_pack::impact::measure(&input, &views, &|_| vec![], &|_| vec![]).unwrap();
    assert_eq!(measured["views/static.brain"].js_gz, 0);
    assert_eq!(measured["views/constant.brain"].js_gz, 0);
    assert!(measured["views/interactive.brain"].js_gz > 0);
    assert!(
        measured["views/static.brain"]
            .html
            .contains("Static output")
    );
}
