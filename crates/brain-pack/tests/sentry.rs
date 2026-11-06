//! Public opt-in boundary, private artifact inventory and deterministic build identity.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64,Ordering};
use brain_lex::Json;
struct Fixture(PathBuf);
impl Drop for Fixture { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
fn fixture() -> Fixture {
    static NEXT:AtomicU64=AtomicU64::new(0);
    let root=std::env::temp_dir().join(format!("brain-sentry-pack-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::write(root.join("app.brain"),"<html lang=\"en\"><head><title>Example</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(root.join("views/home.brain"),"<meta seo title=\"Example\" description=\"Synthetic example\">\n<script>\nexport async function load() { return { ok: true } }\nconst fail = () => { throw new TypeError('synthetic browser error') }\n</script>\n<main><h1>Example</h1><button on:click={fail}>Test</button></main>").unwrap();
    Fixture(root)
}
fn build(root:&Path, static_export:bool)->Result<brain_pack::Output,String> {
    let rt=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let compile=|source:&str,file:&Path| {
        let out=brain_codegen::compile_with(source,file,&brain_codegen::Options{dev:false,root:Some(root.into()),..Default::default()}).map_err(|diags|format!("Synthetic app compilation failed: {:?}",diags.iter().map(|d|(&d.code,&d.msg)).collect::<Vec<_>>()))?;
        Ok(brain_pack::Unit{js:out.js,css:out.css,server_js:out.server_js,server:out.server,server_fns:out.server_fns})
    };
    let siblings=std::fs::read_dir(&rt).unwrap().flatten().map(|entry|entry.path()).filter(|p|p.extension().is_some_and(|ext|ext=="js")&&!p.ends_with("brain-dev.js")&&!p.ends_with("brain-inspect.js")).map(|p|(p.file_name().unwrap().to_string_lossy().into_owned(),std::fs::read_to_string(p).unwrap())).collect();
    let input=brain_pack::Input{root:root.into(),compile:&compile,runtime_core:brain_min::minify_runtime_core(&std::fs::read_to_string(rt.join("brain.js")).unwrap()).unwrap(),runtime_siblings:siblings,layer_order:String::new(),minify:true,site:None,shim:Some(std::fs::read_to_string(rt.join("shim/dom.js")).unwrap()),generator:String::new(),server_module:None,cache:Default::default(),server_mjs:Some(std::fs::read_to_string(rt.join("server/server.mjs")).unwrap()),progress:None,post:None};
    brain_pack::build_with_options(&input,&[("/".into(),"views/home.brain".into())],&|_|vec![],&|_|vec![],&brain_pack::BuildOptions{static_export,..Default::default()})
}
#[test]
fn disabled_adds_no_browser_module_or_private_artifacts_and_static_optin_fails_before_output() {
    let f=fixture();
    let output=build(&f.0,false).unwrap();
    assert!(!output.files.values().any(|code|code.contains("startSentry")));
    assert!(!output.files.contains_key("_server/sentry.mjs"));assert!(!f.0.join(".brain/sentry").exists());
    std::fs::write(f.0.join("brain.json"),"{\"sentry\":false}").unwrap();
    let explicitly_disabled=build(&f.0,false).unwrap();assert_eq!(output.files,explicitly_disabled.files);assert_eq!(output.binary,explicitly_disabled.binary);
    std::fs::write(f.0.join("brain.json"),"{\"sentry\":{\"enabled\":true}}").unwrap();
    assert!(build(&f.0,true).err().unwrap().contains("cannot be used with static output"));assert!(!f.0.join(".brain/sentry").exists());
    std::fs::write(f.0.join("views/home.brain"),"<meta seo title=\"Example\" description=\"Synthetic example\"><main><h1>Example</h1></main>").unwrap();
    assert!(build(&f.0,false).err().unwrap().contains("requires existing Node/SSR output"));assert!(!f.0.join(".brain/sentry").exists());
}
#[test]
fn optin_maps_stay_private_while_final_debug_ids_hashes_sri_and_compression_agree() {
    let f=fixture();std::fs::write(f.0.join("brain.json"),"{\"sentry\":{\"enabled\":true}}").unwrap();
    let output=build(&f.0,false).unwrap();
    assert!(output.files.contains_key("server.mjs"));assert!(output.files.contains_key("_server/sentry.mjs"));
    assert!(output.files["server.mjs"].contains("import('./_server/sentry.mjs')"));
    assert!(!output.files.keys().chain(output.binary.keys()).any(|p|p.ends_with(".map")||p.contains("sentry/")));
    let private=f.0.join(".brain/sentry").join(&output.signature);
    let manifest=Json::parse(&output.files["_server/manifest.json"]).unwrap();
    let Json::Obj(catalog)=manifest.get("sentry").unwrap().get("assets").unwrap() else {panic!("Sentry catalog must be an object")};
    assert!(!catalog.is_empty());
    let mut reporter=None;
    for (url,id) in catalog {
        let id=id.as_str().unwrap();let key=url.trim_start_matches('/');let code=&output.files[key];
        assert!(code.contains(&format!("//# debugId={id}")));
        assert_eq!(key,format!("_b/{}.js",brain_pack::content_hash(code)));
        let map=std::fs::read_to_string(private.join(format!("{id}.js.map"))).unwrap();
        let parsed=Json::parse(&map).unwrap();assert_eq!(parsed.get("debug_id").and_then(Json::as_str),Some(id));
        assert!(!map.contains("sourcesContent"));assert!(!map.contains("server async function"));
        assert!(!parsed.get("mappings").unwrap().as_str().unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(private.join(format!("{id}.js"))).unwrap(),*code);
        if let Some(compressed)=output.binary.get(&format!("{key}.gz")) { assert_eq!(*compressed,brain_pack::gzip::gzip(code.as_bytes(),15)); }
        if code.contains("function startSentry(") {reporter=Some(key.to_string());}
    }
    assert!(output.files.values().any(|text|text.contains("startSentry({base:")));
    let reporter=reporter.expect("reporter module");
    assert!(output.ledger.iter().any(|row|row.assets.iter().any(|asset|asset.path==reporter)),"route ledger must include the real optional reporter cost");
    let integrity=brain_pack::sha::integrity(output.files[&reporter].as_bytes());
    assert!(output.pages[0].html.contains(&format!("href=\"/{reporter}\" integrity=\"{integrity}\"")));
    let node_dir=f.0.join("node-browser");std::fs::create_dir_all(node_dir.join("_b")).unwrap();
    std::fs::write(node_dir.join("package.json"),"{\"type\":\"module\"}").unwrap();
    for (path,code) in &output.files {if path.starts_with("_b/")&&path.ends_with(".js") {std::fs::write(node_dir.join(path),code).unwrap();}}
    let script=r#"import{register}from'node:module';import{pathToFileURL}from'node:url';import{readFileSync}from'node:fs';const base=pathToFileURL(process.argv[1]+'/').href;register('data:text/javascript,'+encodeURIComponent('export async function resolve(s,c,next){if(s.startsWith("/_b/"))return{url:new URL("."+s,'+JSON.stringify(base)+').href,shortCircuit:true};return next(s,c)}'));const {startSentry}=await import(new URL(process.argv[2],base));let capture,sent;const env={window:null,location:{origin:'https://example.com'},navigator:{},AbortController,setTimeout,clearTimeout,addEventListener(){},removeEventListener(){},fetch(url,options){sent=JSON.parse(options.body);return Promise.resolve()}};env.window=env;env[Symbol.for('brain:consent-state')]={analytics:true};const assets=JSON.parse(process.argv[3]);const state=startSentry({base:'',assets:Object.fromEntries(Object.entries(assets).map(([path,id])=>[path.slice(1),id]))},env,fn=>{capture=fn;return()=>{}});const asset=Object.keys(assets)[0];capture({name:'TypeError',stack:'TypeError: private value\n    at synthetic (https://example.com'+asset+':3:9)'});await new Promise(ok=>setImmediate(ok));state.stop();if(!sent||sent.frames[0].debugId!==assets[asset])process.exit(2);console.log('built reporter consent/capture/cleanup OK')"#;
    let assets=manifest.get("sentry").unwrap().get("assets").unwrap().stringify();
    let node=std::process::Command::new("node").args(["--input-type=module","-e",script]).arg(&node_dir).arg(&reporter).arg(assets).output().unwrap();
    assert!(node.status.success(),"built reporter: {}",String::from_utf8_lossy(&node.stderr));
    assert!(String::from_utf8_lossy(&node.stdout).contains("built reporter consent/capture/cleanup OK"));
    let repeated=build(&f.0,false).unwrap();assert_eq!(output.files,repeated.files);assert_eq!(output.binary,repeated.binary);assert_eq!(output.signature,repeated.signature);
    eprintln!("Sentry opt-in route: raw={} minified={} gzip={}",output.report[0].js_raw,output.report[0].js_min,output.report[0].js_gz);
    std::fs::write(f.0.join("brain.json"),"{\"sentry\":false}").unwrap();
    let disabled=build(&f.0,false).unwrap();
    eprintln!("Sentry disabled route: raw={} minified={} gzip={}; delta minified={} gzip={} brotli={}",disabled.report[0].js_raw,disabled.report[0].js_min,disabled.report[0].js_gz,output.report[0].js_min as isize-disabled.report[0].js_min as isize,output.report[0].js_gz as isize-disabled.report[0].js_gz as isize,output.ledger[0].conservative_js.brotli as isize-disabled.ledger[0].conservative_js.brotli as isize);
    let reporter_bytes=output.ledger[0].assets.iter().find(|asset|asset.path==reporter).unwrap();
    eprintln!("Sentry reporter module: minified={} gzip={} brotli={}",reporter_bytes.bytes.raw,reporter_bytes.bytes.gzip,reporter_bytes.bytes.brotli);
    eprintln!("Sentry whole route HTML+external JS delta: raw={} gzip={} brotli={}",(output.ledger[0].html.raw+output.ledger[0].conservative_js.raw) as isize-(disabled.ledger[0].html.raw+disabled.ledger[0].conservative_js.raw) as isize,(output.ledger[0].html.gzip+output.ledger[0].conservative_js.gzip) as isize-(disabled.ledger[0].html.gzip+disabled.ledger[0].conservative_js.gzip) as isize,(output.ledger[0].html.brotli+output.ledger[0].conservative_js.brotli) as isize-(disabled.ledger[0].html.brotli+disabled.ledger[0].conservative_js.brotli) as isize);
}
#[test]
fn unsupported_collector_base_fails_at_build_instead_of_silently_disabling_capture() {
 let f=fixture();
 for base in ["/preview~/".to_string(),format!("/{}/","a".repeat(257)),"/a/../b/".to_string()] {
  std::fs::write(f.0.join("brain.json"),format!("{{\"base\":{},\"sentry\":{{\"enabled\":true}}}}",Json::str(&base).stringify())).unwrap();
  assert!(build(&f.0,false).err().unwrap().contains("Optional browser collectors"));
  assert!(!f.0.join(".brain/sentry").exists());
 }
}
