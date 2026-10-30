//! Actual JS execution protects effects, scope and export contracts; report facts
//! are checked separately so renaming cannot masquerade as source removal.
use brain_min::{CleanupAction as Action, CleanupKind as Kind, Minified};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path,PathBuf};
use std::process::{Command,Stdio};
fn node(code:&str,cwd:Option<&Path>)->String {
    let mut command=Command::new("node");command.args(["--input-type=module","-e",code]);
    if let Some(cwd)=cwd{command.current_dir(cwd);}
    let out=command.output().expect("Node is required for production minifier semantics");
    assert!(out.status.success(),"JS execution failed: {}",String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}
fn min(src:&str)->Minified {brain_min::minify_module_with_report(src,true).unwrap()}
fn equivalent(src:&str)->Minified {
    let out=min(src);assert_eq!(node(src,None),node(&out.code,None));out
}
#[test]
fn removes_actual_bindings_but_preserves_calls_getters_patterns_iterators_and_coercion() {
    let src=include_str!("fixtures/cleanup/effects.mjs");let out=equivalent(src);
    assert!(out.report.facts.iter().any(|f|f.binding.as_deref()==Some("unusedLiteral")&&f.action==Action::Removed));
    assert!(out.report.facts.iter().any(|f|f.binding.as_deref()==Some("unusedCall")&&f.action==Action::InitializerRetained));
    assert!(out.report.facts.iter().any(|f|f.binding.as_deref()==Some("unusedPattern")&&f.action==Action::Retained));
    assert!(out.report.facts.iter().any(|f|f.binding.as_deref()==Some("unusedIteration")&&f.action==Action::Retained));
    assert!(!out.report.to_json().contains("PRIVATE_BODY_MARKER_NOT_A_CREDENTIAL"));
}
#[test]
fn retains_tdz_missing_global_and_coercion_exceptions() {
    let out=equivalent(include_str!("fixtures/cleanup/tdz.mjs"));
    assert_eq!(node(&out.code,None),"[\"ReferenceError\",\"ReferenceError\",\"TypeError\",\"TypeError\"]");
    assert!(out.report.facts.iter().any(|f|f.action==Action::InitializerRetained));
}
#[test]
fn direct_eval_preserves_lexical_names_and_reports_why_proof_was_refused() {
    let src="export function run(){const privateLocal=41;return eval('privateLocal+1')}process.stdout.write(String(run()))";
    let out=equivalent(src);assert_eq!(out.report.removed_bindings,0);
    assert!(out.report.facts.iter().any(|f|f.kind==Kind::Scope&&f.action==Action::Retained));
    assert!(!out.report.to_json().contains("privateLocal+1"));
}
#[test]
fn preserves_shadowed_console_and_effectful_global_log_arguments_including_spreads() {
    equivalent("globalThis.events=[];function run(console){const unused=1;console.log('local')}run({log(v){globalThis.events.push(v)}});process.stdout.write(JSON.stringify(globalThis.events))");
    let src="globalThis.events=[];console.log=()=>{};function effect(n){globalThis.events.push(n);return n}const it={[Symbol.iterator](){globalThis.events.push('iterator');return[1,2][Symbol.iterator]()}};const unused=console.log(effect('first'),'PRIVATE_DEBUG_MARKER',...it,effect('last'));process.stdout.write(JSON.stringify(globalThis.events))";
    let out=equivalent(src);assert!(!out.code.contains("console.log("));
    assert!(!out.report.to_json().contains("PRIVATE_DEBUG_MARKER"));
    assert!(out.report.facts.iter().any(|f|f.kind==Kind::DebugLog));
    equivalent("globalThis.events=[];console.log=()=>{};function effect(n){globalThis.events.push(n);return n}globalThis.kept=1\nconsole.log(effect('asi'));try{const unused=console.log(effect('member')).foo}catch(e){globalThis.events.push(e.name)}const result=console.log(effect('power'))**2;globalThis.events.push(Number.isNaN(result));console.log(effect('optional'))?.[effect('never')];process.stdout.write(JSON.stringify(globalThis.events))");
}
#[test]
fn nested_rewrites_do_not_claim_initializer_retention_when_enclosing_function_is_removed() {
    let src="globalThis.events=[];function effect(){globalThis.events.push('effect');return 1}const unused=()=>{const inner=effect();return inner};export const keep=1;process.stdout.write(JSON.stringify(globalThis.events))";
    let out=equivalent(src);assert_eq!(out.report.retained_effects,0);
    assert!(!out.report.facts.iter().any(|f|f.binding.as_deref()==Some("inner")));
    let src="globalThis.events=[];function effect(){globalThis.events.push('effect');return 1}export function run(){const unused=(()=>{const inner=effect();return 1})();}run();process.stdout.write(JSON.stringify(globalThis.events))";
    let out=equivalent(src);
    for name in ["unused","inner"] {assert_eq!(out.report.facts.iter().filter(|f|f.binding.as_deref()==Some(name)&&f.action!=Action::Retained).count(),1);}
}
#[test]
fn handles_multiple_declarators_without_reordering_effects_and_keeps_loop_headers() {
    let src="globalThis.events=[];function effect(x){globalThis.events.push(x);return x}const unused=effect('a'), kept=effect('b'), other=effect('c');for(let index=0;index<2;index++){globalThis.events.push(kept)}process.stdout.write(JSON.stringify(globalThis.events))";
    let out=equivalent(src);assert_eq!(out.report.facts.iter().filter(|f|f.action==Action::InitializerRetained).count(),2);
    equivalent("globalThis.events=[];for(;;){globalThis.events.push(1);break}process.stdout.write(JSON.stringify(globalThis.events))");
}
struct Temp(PathBuf);
impl Temp {fn new()->Self {static NEXT:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);let p=std::env::temp_dir().join(format!("brain-cleanup-imports-{}-{}",std::process::id(),NEXT.fetch_add(1,std::sync::atomic::Ordering::Relaxed)));std::fs::create_dir(&p).unwrap();Self(p)}}
impl Drop for Temp {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
#[test]
fn unused_import_bindings_preserve_real_module_evaluation_and_partial_import_syntax() {
    let tmp=Temp::new();std::fs::write(tmp.0.join("effect.mjs"),"process.stdout.write('module-effect\\n');export default 1;export const unused=2;export const used=3;").unwrap();
    for src in [
        "import Default,{unused as alias}from'./effect.mjs';process.stdout.write('done')",
        "import*as ns from'./effect.mjs';process.stdout.write('done')",
        "import Default,{unused as alias,used as live}from'./effect.mjs';process.stdout.write(String(live))",
    ] {
        let out=min(src);assert_eq!(node(src,Some(&tmp.0)),node(&out.code,Some(&tmp.0)));
        assert!(out.report.removed_import_bindings>0);
        assert!(out.report.facts.iter().any(|f|f.kind==Kind::Import));
        assert!(!out.report.to_json().contains("effect.mjs"));
    }
    std::fs::write(tmp.0.join("data.json"),"{\"ok\":true}").unwrap();
    let source="import data from'./data.json'with{type:'json'};process.stdout.write('ready')";
    let out=min(source);assert_eq!(node(source,Some(&tmp.0)),node(&out.code,Some(&tmp.0)));assert_eq!(out.report.removed_import_bindings,1);
}
#[test]
fn exported_bindings_parameter_defaults_and_live_shadowing_remain_contracts() {
    let src="export const kept=1;export function run(a=2,b=3){const unused=4;{const kept=7;return[a+b,kept]}}";
    let out=min(src);assert!(out.code.contains("export const kept="));assert!(out.code.contains("export function run("));
    for source in [src,out.code.as_str()] {
        let loader=format!("const ns=await import('data:text/javascript,'+encodeURIComponent({:?}));process.stdout.write(JSON.stringify([Object.keys(ns),ns.run(),ns.run.length]))",source);
        assert_eq!(node(&loader,None),"[[\"kept\",\"run\"],[5,7],0]");
    }
    assert!(!out.report.facts.iter().any(|f|f.binding.as_deref()==Some("kept")&&f.action!=Action::Retained));
}
#[test]
fn tree_shaking_preserves_class_evaluation_import_effects_and_tdz() {
    let src="globalThis.events=[];class Unused extends (globalThis.events.push('extends'),Object){static[globalThis.events.push('key')](){}}const unused={get value(){globalThis.events.push('get');return 1}}.value;process.stdout.write(JSON.stringify(globalThis.events))";
    let out=brain_min::tree_shake_with_report(src,&HashSet::new()).unwrap();assert_eq!(node(src,None),node(&out.code,None));
    let out=brain_min::tree_shake_with_report(include_str!("fixtures/cleanup/tdz.mjs"),&HashSet::new()).unwrap();assert_eq!(node(&out.code,None),node(include_str!("fixtures/cleanup/tdz.mjs"),None));
}
#[test]
fn report_counts_are_not_inferred_from_name_mangling_and_coordinates_are_input_positions() {
    let src="export function run(longParameter){\n const liveValue=longParameter+1;return liveValue}\nconst unused=7;";
    let out=min(src);assert_eq!(out.report.removed_bindings,1);assert_eq!(out.report.facts.len(),1);
    let f=&out.report.facts[0];assert_eq!(f.binding.as_deref(),Some("unused"));assert_eq!(f.line,Some(3));assert_eq!(&src[f.start..f.end],"unused");
    let debug=brain_min::minify_module_with_report(src,false).unwrap();assert_eq!(debug.report.removed_bindings,0);assert!(debug.report.facts.is_empty());
    let mut child=Command::new("node").args(["-e","let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{const j=JSON.parse(s);if(j.coordinateSpace!=='input-js'||j.removedBindings!==1)process.exit(1)})"]).stdin(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(out.report.to_json().as_bytes()).unwrap();assert!(child.wait().unwrap().success());
}
#[test]
fn bounded_feedback_keeps_total_counts_and_marks_omissions_without_failing_the_build() {
    let count=brain_min::cleanup::MAX_FACTS+32;let mut src=String::new();
    for n in 0..count {src.push_str(&format!("const unused_{n}=1;\n"));}src.push_str("export const keep=1;");
    let out=min(&src);assert_eq!(out.report.removed_bindings,count);assert_eq!(out.report.facts.len(),brain_min::cleanup::MAX_FACTS);
    assert!(out.report.facts_truncated);assert_eq!(out.report.omitted_facts,32);assert!(out.report.to_json().contains("\"factsTruncated\":true"));
}
#[test]
fn failures_are_typed_redacted_and_legacy_wrappers_remain_compatible() {
    let src="const value='PRIVATE_UNTERMINATED_LITERAL";
    let error=brain_min::minify_module_with_report(src,true).unwrap_err();assert_eq!(error.code,"MINIFY_LEX_ERROR");assert!(!error.to_string().contains("PRIVATE_"));assert!(!error.to_json().contains("PRIVATE_"));
    assert!(brain_min::minify_module(src,true).is_err());
    assert!(brain_min::minify_module_with_report("export",true).is_err());
    assert_eq!(brain_min::minify_module("export const keep=1",true).unwrap(),min("export const keep=1").code);
}

#[test]
fn preserves_derived_this_and_lexical_reads_across_execution_boundaries() {
    equivalent("globalThis.events=[];class Derived extends Object{constructor(){try{const unused=this}catch(e){globalThis.events.push(e.name)}super()}}new Derived();process.stdout.write(JSON.stringify(globalThis.events))");
    equivalent("globalThis.events=[];function run(){const unused=[later]}try{run()}catch(e){globalThis.events.push(e.name)}let later=1;process.stdout.write(JSON.stringify(globalThis.events))");
    equivalent("globalThis.events=[];console.log=()=>{};try{switch(2){case 1:let later=1;break;case 2:console.log(later);break}}catch(e){globalThis.events.push(e.name)}process.stdout.write(JSON.stringify(globalThis.events))");
}
#[test]
fn imported_live_binding_reads_still_throw_in_a_real_module_cycle() {
    let tmp=Temp::new();let source="import{value}from'./b.mjs';const unused=value;export const other=1;";
    std::fs::write(tmp.0.join("a.mjs"),source).unwrap();
    std::fs::write(tmp.0.join("b.mjs"),"import{other}from'./a.mjs';export const value=1;").unwrap();
    let entry="try{await import('./b.mjs');process.stdout.write('ok')}catch(e){process.stdout.write(e.name)}";
    assert_eq!(node(entry,Some(&tmp.0)),"ReferenceError");
    let min=min(source);std::fs::write(tmp.0.join("a.mjs"),&min.code).unwrap();
    assert_eq!(node(entry,Some(&tmp.0)),"ReferenceError");
    let shaken=brain_min::tree_shake_with_report(source,&HashSet::from(["other".to_owned()])).unwrap();
    std::fs::write(tmp.0.join("a.mjs"),&shaken.code).unwrap();assert_eq!(node(entry,Some(&tmp.0)),"ReferenceError");
}
