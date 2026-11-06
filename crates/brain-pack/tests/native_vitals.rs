//! Independent opt-in, existing Node output and deterministic consent-gated browser bootstrap.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64,Ordering};
struct Fixture(PathBuf);
impl Drop for Fixture { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
fn fixture() -> Fixture {
    static NEXT:AtomicU64=AtomicU64::new(0);
    let root=std::env::temp_dir().join(format!("brain-native-vitals-pack-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::write(root.join("app.brain"),"<html lang=\"en\"><head><title>Example</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(root.join("views/home.brain"),"<meta seo title=\"Example\" description=\"Synthetic example\">\n<script>\nexport async function load() { return { ok: true } }\nconst fail = () => { throw new TypeError('synthetic browser error') }\n</script>\n<main><h1>Example</h1><button on:click={fail}>Test</button></main>").unwrap();
    Fixture(root)
}
fn build(root:&Path, static_export:bool, configured:bool)->Result<brain_pack::Output,String> {
    let rt=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let compile=|source:&str,file:&Path| {
        let out=brain_codegen::compile_with(source,file,&brain_codegen::Options{dev:false,root:Some(root.into()),..Default::default()}).map_err(|diags|format!("Synthetic app compilation failed: {:?}",diags.iter().map(|d|(&d.code,&d.msg)).collect::<Vec<_>>()))?;
        Ok(brain_pack::Unit{js:out.js,css:out.css,server_js:out.server_js,server:out.server,server_fns:out.server_fns})
    };
    let siblings=std::fs::read_dir(&rt).unwrap().flatten().map(|entry|entry.path()).filter(|p|p.extension().is_some_and(|ext|ext=="js")&&!p.ends_with("brain-dev.js")&&!p.ends_with("brain-inspect.js")).map(|p|(p.file_name().unwrap().to_string_lossy().into_owned(),std::fs::read_to_string(p).unwrap())).collect();
    let input=brain_pack::Input{root:root.into(),compile:&compile,runtime_core:brain_min::minify_runtime_core(&std::fs::read_to_string(rt.join("brain.js")).unwrap()).unwrap(),runtime_siblings:siblings,layer_order:String::new(),minify:true,site:None,shim:Some(std::fs::read_to_string(rt.join("shim/dom.js")).unwrap()),generator:String::new(),server_module:None,cache:Default::default(),server_mjs:Some(std::fs::read_to_string(rt.join("server/server.mjs")).unwrap()),progress:None,post:None};
    brain_pack::build_with_options(&input,&[("/".into(),"views/home.brain".into())],&|_|vec![],&|_|vec![],&brain_pack::BuildOptions{static_export,native_vitals:configured.then(||brain_pack::NativeVitalsOptions::new(vec!["/".into(),"/items/:id".into()],30).unwrap()),..Default::default()})
}
#[test]
fn absent_or_disabled_costs_no_assets_and_enabled_requires_existing_native_node_output() {
    let f=fixture();
    let output=build(&f.0,false,false).unwrap();
    assert!(!output.files.values().any(|code|code.contains("startNativeVitals")));
    std::fs::write(f.0.join("brain.json"),r#"{"vitals":false}"#).unwrap();
    let disabled=build(&f.0,false,false).unwrap();assert_eq!(output.files,disabled.files);assert_eq!(output.binary,disabled.binary);
    std::fs::write(f.0.join("brain.json"),r#"{"vitals":{"enabled":true,"routes":["/","/items/:id"]}}"#).unwrap();
    assert!(build(&f.0,false,false).err().unwrap().contains("configured fixed native backend"));
    assert!(build(&f.0,true,true).err().unwrap().contains("cannot be used with static output"));
    std::fs::write(f.0.join("views/home.brain"),"<meta seo title=\"Example\" description=\"Synthetic example\"><main><h1>Example</h1></main>").unwrap();
    assert!(build(&f.0,false,true).err().unwrap().contains("requires existing Node/SSR output"));
}
#[test]
fn enabled_emits_only_declared_patterns_under_the_base_with_real_asset_cost_and_csp() {
    let f=fixture();std::fs::write(f.0.join("brain.json"),r#"{"base":"/app","vitals":{"enabled":true,"routes":["/","/items/:id"],"retentionDays":30}}"#).unwrap();
    let output=build(&f.0,false,true).unwrap();assert!(output.files.contains_key("server.mjs"));
    assert!(output.pages[0].html.contains("startNativeVitals({base:\"/app\",routes:[\"/\",\"/items/:id\"]})"));
    let (reporter,code)=output.files.iter().find(|(path,code)|path.ends_with(".js")&&code.contains("function startNativeVitals(")).expect("real runtime module");
    assert!(output.ledger.iter().any(|row|row.assets.iter().any(|asset|asset.path==*reporter)));
    assert!(output.pages[0].html.contains(&format!("href=\"/app/{reporter}\" integrity=\"{}\"",brain_pack::sha::integrity(code.as_bytes()))));
    assert!(!output.files.keys().any(|path|path.contains("vitals")&&path.ends_with(".json")));
    assert!(!output.files.values().any(|code|code.contains("/items/private-identifier")));
    let again=build(&f.0,false,true).unwrap();assert_eq!(output.files,again.files);assert_eq!(output.binary,again.binary);
}
#[test]
fn unsupported_base_cannot_compile_an_inactive_native_collector() {
 let f=fixture();
 std::fs::write(f.0.join("brain.json"),r#"{"base":"/preview~/","vitals":{"enabled":true,"routes":["/","/items/:id"],"retentionDays":30}}"#).unwrap();
 assert!(build(&f.0,false,true).err().unwrap().contains("Optional browser collectors"));
}
