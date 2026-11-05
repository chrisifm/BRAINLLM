//! Caller-owned IDs must keep their public names across component instances.
use std::{path::Path, process::Command};

#[test]
fn direct_prop_ids_and_references_keep_caller_identity_with_local_ids_encapsulated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let source = r#"<meta props="heading:str='fallback' label:str items:list">
<section aria-labelledby={label}><h2 id={heading}>Example</h2><label for="local">Local</label><input id="local"/><p aria-describedby={heading} aria-details="local" aria-errormessage={'local'}>Description</p>
{#each items as heading (heading)}<span id={heading}>Local item</span>{/each}</section>"#;
    let opts = brain_codegen::Options { runtime: format!("file://{}/runtime/brain.js", root.display()), ..Default::default() };
    let output = brain_codegen::compile_with(source, Path::new("comp/public-heading.brain"), &opts).unwrap_or_else(|d|panic!("{d:?}"));
    let js = brain_lex::json::Json::str(output.js).stringify();
    let script = format!(r#"
import assert from 'node:assert/strict';
import {{install}} from 'file://{root}/runtime/shim/dom.js';
const doc=install();
const {{mount,sig}}=await import('file://{root}/runtime/brain.js');
const C=(await import('data:text/javascript,'+encodeURIComponent({js}))).default;
const walk=(node,tag)=>Array.from(node.childNodes).flatMap(n=>(n.nodeType===1&&n.tagName.toLowerCase()===tag?[n]:[]).concat(walk(n,tag)));
const id=sig('public-second');
const first=mount(C,doc.body,{{heading:'public-first',label:'external-title',items:['row']}});
const second=mount(C,doc.body,{{get heading(){{return id.v}},label:'external-title',items:['row']}});
assert.deepEqual(walk(doc.body,'h2').map(n=>n.getAttribute('id')),['public-first','public-second']);
assert.deepEqual(walk(doc.body,'section').map(n=>n.getAttribute('aria-labelledby')),['external-title','external-title']);
assert.deepEqual(walk(doc.body,'p').map(n=>n.getAttribute('aria-describedby')),['public-first','public-second']);
assert.deepEqual(walk(doc.body,'p').map(n=>n.getAttribute('aria-details')),['local','local~2']);
assert.deepEqual(walk(doc.body,'p').map(n=>n.getAttribute('aria-errormessage')),['local','local~2']);
assert.deepEqual(walk(doc.body,'input').map(n=>n.getAttribute('id')),['local','local~2']);
assert.deepEqual(walk(doc.body,'label').map(n=>n.getAttribute('for')),['local','local~2']);
assert.deepEqual(walk(doc.body,'span').map(n=>n.getAttribute('id')),['row','row~2']);
id.v='public-updated'; await Promise.resolve();
assert.equal(walk(doc.body,'h2')[1].getAttribute('id'),'public-updated');
assert.equal(walk(doc.body,'p')[1].getAttribute('aria-describedby'),'public-updated');
const third=mount(C,doc.body,{{label:'external-title',items:[]}});
const fourth=mount(C,doc.body,{{heading:null,label:'external-title',items:[]}});
assert.deepEqual(walk(doc.body,'h2').slice(2).map(n=>n.getAttribute('id')),['fallback~3','fallback~4']);
assert.deepEqual(walk(doc.body,'p').slice(2).map(n=>n.getAttribute('aria-describedby')),['fallback~3','fallback~4']);
first();second();third();fourth();
"#,root=root.display());
    let result=Command::new("node").args(["--input-type=module","--eval",&script]).output().unwrap();
    assert!(result.status.success(),"{}\n{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
}
