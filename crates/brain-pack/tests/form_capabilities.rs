//! Compiler-selected synchronous forms omit async-check machinery in final emitted assets.
use std::path::{Path, PathBuf};
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn build(source: &str, extra: &[(&str, &str)]) -> (Scratch, brain_pack::Output) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(
        std::env::temp_dir().join(format!("brain-form-capability-{}-{stamp}", std::process::id())),
    );
    let root = &scratch.0;
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::write(root.join("app.brain"),"<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Native helpers</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(root.join("views/index.brain"), source).unwrap();
    for (path, content) in extra {
        let destination = root.join(path);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(destination, content).unwrap();
    }
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
        server_mjs: std::fs::read_to_string(runtime.join("server/server.mjs")).ok(),
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
    (scratch, output)
}

#[test]
fn synchronous_literal_options_omit_only_optional_checks_in_final_assets() {
    let source=r#"<meta seo title="Form capabilities" description="A private form capability fixture." robots="noindex"><script>
      server: async function save(ctx,v) {return v}
      let f=form({name:'str!',amount:'num!',enabled:'bool'}, {submit:save})
    </script><main><h1>Form capabilities</h1><form on:submit={f.submit}><input name="name" bind:value={f.v.name} aria-label="Name"><input type="number" name="amount" bind:value={f.v.amount} aria-label="Amount"><input type="checkbox" name="enabled" bind:checked={f.v.enabled} aria-label="Enabled"><button>Save</button></form></main>"#;
    let (_scratch,output)=build(source,&[]);
    let row=output.ledger.iter().find(|row|row.route=="/").unwrap();
    let asset=row.assets.iter().find(|asset|asset.source.as_deref()==Some("runtime/brain-form.js")).unwrap();
    let emitted=&output.files[&asset.path];
    assert!(emitted.contains("formScalarsSync"),"tracked helper export must survive minification");
    assert!(!emitted.contains("form.check_failed") && !emitted.contains("Could not check this value"),"unused remote-check capability must not ship");
    assert!(emitted.contains("form.error"),"generic submit error behavior must remain");
    assert_eq!(asset.bytes.raw,emitted.len());
    assert_eq!(asset.bytes.gzip,output.binary.get(&format!("{}.gz",asset.path)).map_or(emitted.len(),|gzip|gzip.len().min(emitted.len())));
    // The same schema with a real asynchronous check must retain that capability.
    let dynamic=source.replace("{submit:save}","{check:{name:async value=>''},submit:save}");
    let (_other,checked)=build(&dynamic,&[]);
    let checked_row=checked.ledger.iter().find(|row|row.route=="/").unwrap();
    let checked_asset=checked_row.assets.iter().find(|asset|asset.source.as_deref()==Some("runtime/brain-form.js")).unwrap();
    let checked_source=&checked.files[&checked_asset.path];
    assert!(checked_source.contains("formScalars") && checked_source.contains("form.check_failed"));
    eprintln!("Emitted form capability gzip bytes: synchronous={}, async={}",asset.bytes.gzip,checked_asset.bytes.gzip);
    assert!(checked_asset.bytes.gzip>asset.bytes.gzip,"the omitted capability must reduce actual compressed emitted bytes");
}
