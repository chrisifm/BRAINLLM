//! A literal translation lookup has no reactive inputs; reactive placeholders and user helpers still track.
use std::{path::Path,process::Command};

#[test]
fn translations_without_reactive_arguments_do_not_allocate_effects_or_capture_a_previous_locale() {
    let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let options=brain_codegen::Options {runtime:format!("file://{}/runtime/brain.js",root.display()),..Default::default()};
    let compile=|source:&str,file:&str|brain_lex::Json::str(brain_codegen::compile_with(source,Path::new(file),&options).unwrap_or_else(|d|panic!("{d:?}")).js).stringify();
    let static_js=compile("<p>{t('example.label')}</p>","comp/static-label.brain");
    let control_js=compile("<p>Default label</p>","comp/control-label.brain");
    let opaque_js=compile("<p>{(() => t('example.label'))()}</p>","comp/opaque-label.brain");
    let reactive_js=compile("<script>let n=sig(0)</script><p>{t('example.count', { n })}</p><button on:click={n++}>Next</button>","comp/reactive-label.brain");
    let local_js=compile("<script>let n=sig(0)\nfunction t(){return 'Local ' + n}</script><p>{t()}</p><button on:click={n++}>Next</button>","comp/local-label.brain");
    let script=format!(r#"
import assert from 'node:assert/strict';
import {{install,MouseEvent}} from 'file://{root}/runtime/shim/dom.js';
const doc=install();
const {{mount,_rt}}=await import('file://{root}/runtime/brain.js');
const {{add}}=await import('file://{root}/runtime/brain-i18n.js');
add('en',{{'example.label':'Default label','example.count':'Count {{n}}'}});
add('es',{{'example.label':'Translated label'}});
const load=async source=>(await import('data:text/javascript,'+encodeURIComponent(source))).default;
const Static=await load({static_js});
const effects=node=>(node.e?1:0)+(node.c||[]).reduce((n,child)=>n+effects(child),0);
let baseline;
const Control=await load({control_js}),Opaque=await load({opaque_js});
globalThis.__brainLocale='en';
const controlHost=doc.createElement('div');doc.body.appendChild(controlHost);
const controlDispose=mount(props=>{{const nodes=Control(props);baseline=effects(_rt().owner());return nodes}},controlHost);
controlDispose();controlHost.remove();
const opaqueHost=doc.createElement('div');doc.body.appendChild(opaqueHost);
const opaqueDispose=mount(props=>{{const nodes=Opaque(props);assert.ok(effects(_rt().owner())>baseline,'the measurement must detect a retained opaque-call effect');return nodes}},opaqueHost);
opaqueDispose();opaqueHost.remove();
for(const [locale,text] of [['en','Default label'],['es','Translated label'],['en','Default label']]){{
 globalThis.__brainLocale=locale;
 const host=doc.createElement('div');doc.body.appendChild(host);
 const dispose=mount(props=>{{const nodes=Static(props);assert.equal(effects(_rt().owner()),baseline,'literal translation must not add a reactive effect beyond the ordinary component guards');return nodes}},host);
 assert.equal(host.textContent,text);dispose();host.remove();
}}
const walk=n=>Array.from(n.childNodes).flatMap(c=>(c.nodeType===1?[c]:[]).concat(walk(c)));
for(const [source,before,after] of [[{reactive_js},'Count 0','Count 1'],[{local_js},'Local 0','Local 1']]){{
 const C=await load(source),host=doc.createElement('div');doc.body.appendChild(host);
 const dispose=mount(C,host);
 const nodes=walk(host),p=nodes.find(n=>n.tagName.toLowerCase()==='p'),button=nodes.find(n=>n.tagName.toLowerCase()==='button');
 assert.equal(p.textContent,before);button.dispatchEvent(new MouseEvent('click',{{bubbles:true}}));await Promise.resolve();assert.equal(p.textContent,after);dispose();host.remove();
}}
"#,root=root.display());
    let result=Command::new("node").args(["--input-type=module","--eval",&script]).output().unwrap();
    assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
}
