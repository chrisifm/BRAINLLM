//! Per-use literal booleans may remove a conditional only when the selected inline UI body is literal DOM.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
const INDICATOR: &str = r#"<meta props="pulse:bool=false">
<span class="indicator">{#if (pulse)}<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M 1 1 L 11 11 M 11 1 L 1 11" /></svg>{:else}<svg viewBox="0 0 12 12" aria-hidden="true"><circle cx="6" cy="6" r="3" /></svg>{/if}</span>
<style>.indicator { color: var(--fg); }</style>"#;
fn app(label: &str, files: &[(&str, &str)]) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "literal-inline-{label}-{}-{}-{stamp}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for (name, source) in files {
        let file = root.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, source).unwrap();
    }
    root
}
fn compile(root: &Path, file: &str, dev: bool) -> brain_codegen::Output {
    let source = std::fs::read_to_string(root.join(file)).unwrap();
    brain_codegen::compile_with(
        &source,
        Path::new(file),
        &brain_codegen::Options {
            root: Some(root.into()),
            dev,
            ..Default::default()
        },
    )
    .unwrap_or_else(|d| panic!("{file}: {d:#?}"))
}
fn when_count(js: &str) -> usize {
    js.matches("when(null,").count()
}
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn emit(root: &Path, files: &[&str]) -> PathBuf {
    let out = root.join("out");
    let rt = format!("file://{}/runtime/brain.js", repo().display());
    for file in files {
        let source = std::fs::read_to_string(root.join(file)).unwrap();
        let result = brain_codegen::compile_with(
            &source,
            Path::new(file),
            &brain_codegen::Options {
                root: Some(root.into()),
                runtime: rt.clone(),
                import_suffix: ".js".into(),
                ..Default::default()
            },
        )
        .unwrap_or_else(|d| panic!("{file}: {d:#?}"));
        let path = out.join(format!("{file}.js"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, result.js).unwrap();
    }
    out
}
fn node(out: &Path, body: &str) {
    let script = format!(
        "import assert from 'node:assert/strict';\nimport{{install,MouseEvent}}from'file://{}/runtime/shim/dom.js';\nconst doc=install();\nconst{{mount}}=await import('file://{}/runtime/brain.js');\nconst directory={};\n{}",
        repo().display(),
        repo().display(),
        brain_lex::Json::str(out.display().to_string()).stringify(),
        body
    );
    let result = Command::new("node")
        .args(["--input-type=module", "--eval", &script])
        .output()
        .expect("Node is required for DOM acceptance");
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn literal_inline_bool_weight_fixture() {
    let source = "<script>import Indicator from '../ui/indicator.brain'</script>\n<Indicator pulse={false} />";
    let root = app(
        "weight",
        &[
            ("ui/indicator.brain", INDICATOR),
            ("comp/weight.brain", source),
        ],
    );
    let result = compile(&root, "comp/weight.brain", false);
    let file = root.join("generated.js");
    std::fs::write(&file, &result.js).unwrap();
    std::fs::write(root.join("generated.css"), &result.css).unwrap();
    let adopted = brain_codegen::compile_with(
        source,
        Path::new("comp/weight.brain"),
        &brain_codegen::Options {
            root: Some(root.clone()),
            adopt: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|d| panic!("{d:#?}"));
    std::fs::write(root.join("generated-adopt.js"), &adopted.js).unwrap();
    println!("LITERAL_BOOL_WEIGHT_FILE={}", file.display());
    assert_eq!(
        when_count(&adopted.js),
        0,
        "adoption output must use the same literal decision"
    );
    assert_eq!(
        when_count(&result.js),
        0,
        "literal static SVG branch should not call when: {}",
        result.js
    );
    assert!(
        !result.js.contains("M 1 1 L 11 11"),
        "the unreachable SVG must not be generated"
    );
    assert!(result.js.contains("circle"));
    let out = root.join("render");
    std::fs::create_dir_all(&out).unwrap();
    let runtime_prefix = format!("file://{}/runtime/", repo().display());
    for (name, js) in [("plain", &result.js), ("adopt", &adopted.js)] {
        std::fs::write(
            out.join(format!("{name}.mjs")),
            js.replace("/@brain/", &runtime_prefix),
        )
        .unwrap();
    }
    let child_hash = brain_codegen::component_hash("ui/indicator.brain");
    node(
        &out,
        &format!(
            r#"
const {{stamp}}=await import('{runtime_prefix}brain-hydrate.js');
const {{_rt}}=await import('{runtime_prefix}brain.js');
for(const mode of ['plain','adopt']){{
 const C=(await import('file://'+directory+'/'+mode+'.mjs')).default;
 const host=doc.createElement('div');doc.body.appendChild(host);
 const stopStamp=mode==='adopt'?stamp(_rt(),doc):()=>{{}};
 const dispose=mount(C,host,{{}});
 assert(host.querySelector('circle'));assert.equal(host.querySelector('path'),null);
 assert.equal(host.querySelector('span').getAttribute('data-b'),'{child_hash}');
 if(mode==='adopt')assert.match(host.innerHTML,/data-hk=/);
 stopStamp();dispose();assert.equal(host.childNodes.length,0);host.remove();
}}
"#
        ),
    );
}
#[test]
fn mixed_uses_are_folded_individually_and_keep_dynamic_predicate() {
    let source = r#"<script>import Indicator from '../ui/indicator.brain'
let pulse = sig(false)
</script>
<Indicator pulse={false}/><Indicator pulse/><Indicator pulse={pulse}/><button on:click={pulse = !pulse}>Toggle</button>"#;
    let root = app(
        "mixed",
        &[
            ("ui/indicator.brain", INDICATOR),
            ("comp/page.brain", source),
        ],
    );
    let result = compile(&root, "comp/page.brain", false);
    assert_eq!(when_count(&result.js), 1, "{}", result.js);
    assert!(!result.js.contains("Indicator({"));
    assert!(result.css.contains(".indicator"));
    let child_hash = brain_codegen::component_hash("ui/indicator.brain");
    assert!(result.css.contains(&format!("data-b=\"{child_hash}\"")));
}
#[test]
fn defaults_empty_false_and_else_if_select_the_literal_branch() {
    let empty = "<meta props=\"pulse:bool=false\"><i>Before</i>{#if pulse}<b>Unreachable</b>{/if}<i>After</i>";
    let chain = "<meta props=\"first:bool=false next:bool=true\">{#if first}<b>First</b>{:else if next}<b>Chosen</b>{:else}<b>Last</b>{/if}";
    let root = app(
        "chain",
        &[
            ("ui/empty.brain", empty),
            ("ui/chain.brain", chain),
            (
                "comp/page.brain",
                "<script>import Empty from '../ui/empty.brain'\nimport Chain from '../ui/chain.brain'</script><Empty/><Chain/>",
            ),
        ],
    );
    let result = compile(&root, "comp/page.brain", false);
    assert_eq!(when_count(&result.js), 0, "{}", result.js);
    assert!(!result.js.contains("Unreachable"));
    assert!(!result.js.contains("First</b>"));
    assert!(!result.js.contains("Last</b>"));
    assert!(result.js.contains("Chosen"));
}
#[test]
fn ambiguous_content_development_stateful_and_non_ui_keep_the_existing_owner() {
    let root = app(
        "negative",
        &[
            (
                "ui/read.brain",
                "<meta props=\"pulse:bool label:str\">{#if pulse}<span>{label}</span>{/if}",
            ),
            (
                "ui/slot.brain",
                "<meta props=\"pulse:bool\">{#if pulse}<slot/>{/if}",
            ),
            (
                "ui/custom.brain",
                "<meta props=\"pulse:bool\">{#if pulse}<fixture-widget/>{/if}",
            ),
            (
                "ui/unknown.brain",
                "<meta props=\"pulse:bool\">{#if pulse}<fixtureunknown/>{/if}",
            ),
            (
                "ui/customized.brain",
                "<meta props=\"pulse:bool\">{#if pulse}<span is=\"fixture-span\">Custom</span>{/if}",
            ),
            (
                "ui/autofocus.brain",
                "<meta props=\"pulse:bool\">{#if pulse}<span tabindex=\"0\" autofocus>Focus</span>{/if}",
            ),
            (
                "comp/stateful.brain",
                "<meta props=\"pulse:bool\"><script>let n = sig(0)</script>{#if pulse}<button on:click={n++}>{n}</button>{/if}",
            ),
            (
                "comp/ordinary.brain",
                "<meta props=\"pulse:bool\">{#if pulse}<span>Ordinary</span>{/if}",
            ),
            (
                "comp/page.brain",
                "<script>import Read from '../ui/read.brain'\nimport Slot from '../ui/slot.brain'\nimport Custom from '../ui/custom.brain'\nimport Unknown from '../ui/unknown.brain'\nimport Customized from '../ui/customized.brain'\nimport Focus from '../ui/autofocus.brain'\nimport Stateful from './stateful.brain'\nimport Ordinary from './ordinary.brain'</script><Read pulse={true} label=\"Text\"/><Slot pulse={true}><span>Slot</span></Slot><Custom pulse={true}/><Unknown pulse={true}/><Customized pulse={true}/><Focus pulse={true}/><Stateful pulse={false}/><Ordinary pulse={false}/>{#if true}<b>Parent</b>{/if}",
            ),
        ],
    );
    let result = compile(&root, "comp/page.brain", false);
    assert_eq!(when_count(&result.js), 7, "{}", result.js);
    assert!(result.js.contains("Stateful({"));
    assert!(result.js.contains("Ordinary({"));
    assert!(when_count(&compile(&root, "comp/stateful.brain", false).js) > 0);
    assert!(when_count(&compile(&root, "comp/ordinary.brain", false).js) > 0);
    let dev = compile(&root, "comp/page.brain", true);
    assert!(dev.js.contains("Read({") && dev.js.contains("Slot({"));
    assert!(when_count(&compile(&root, "ui/read.brain", true).js) > 0);
}
#[test]
fn owner_sensitive_call_branch_is_not_hoisted_into_the_parent() {
    let source = r#"<script>
import Probe from '../ui/probe.brain'
import Token from '../channels/token.jsbrain'
import Other from '../channels/other.jsbrain'
import Setter from '../ui/setter.brain'
provide(Other, 'other')
</script>
<Probe pulse={true}><Setter/></Probe><p>{use(Token)}</p>"#;
    let root = app(
        "owner",
        &[
            (
                "ui/probe.brain",
                "<meta props=\"pulse:bool\">{#if pulse}<aside><slot/></aside>{/if}",
            ),
            (
                "ui/setter.brain",
                "<script>import setToken from '../lib/set-token.jsbrain'\nimport Token from '../channels/token.jsbrain'\nsetToken(Token)</script><span>Setter</span>",
            ),
            (
                "lib/set-token.jsbrain",
                "export default function setToken(token) { return provide(token, 'inside') }",
            ),
            (
                "channels/token.jsbrain",
                "export default key('fixture', 'outside')",
            ),
            (
                "channels/other.jsbrain",
                "export default key('other-fixture', 'other-default')",
            ),
            ("comp/page.brain", source),
        ],
    );
    let js = compile(&root, "comp/page.brain", false).js;
    assert_eq!(
        when_count(&js),
        1,
        "the provider branch needs its own existing owner: {js}"
    );
    let out = emit(
        &root,
        &[
            "comp/page.brain",
            "channels/token.jsbrain",
            "channels/other.jsbrain",
            "ui/setter.brain",
            "lib/set-token.jsbrain",
        ],
    );
    let core = format!("file://{}/runtime/brain.js", repo().display());
    // A compiled negative control replaces only when's owner semantics with the proposed direct insertion.
    std::fs::write(out.join("unsafe-runtime.mjs"),format!("export * from '{core}';import{{insert}}from'{core}';export function when(p,c,a,b,end){{const f=c()?a:b;if(f)insert(p,f(),end)}}")).unwrap();
    let actual = std::fs::read_to_string(out.join("comp/page.brain.js")).unwrap();
    std::fs::write(
        out.join("comp/unsafe.brain.js"),
        actual.replace(
            &core,
            &format!("file://{}/unsafe-runtime.mjs", out.display()),
        ),
    )
    .unwrap();
    node(
        &out,
        r#"
const seen=[];
for(const name of ['page','unsafe']){
 const C=(await import('file://'+directory+'/comp/'+name+'.brain.js')).default;
 const host=doc.createElement('div');doc.body.appendChild(host);const dispose=mount(C,host,{});
 seen.push(host.querySelector('p').textContent);dispose();host.remove();
}
assert.deepEqual(seen,['outside','inside']);
"#,
    );
}
#[test]
fn literal_dom_and_retained_slot_owners_toggle_scope_and_cleanup_on_the_real_runtime() {
    let source = r#"<script>
import Indicator from '../ui/indicator.brain'
import Slot from '../ui/slot.brain'
import Observer from './observer.brain'
let count = sig(1)
let visible = sig(true)
</script>
<div>{#if visible}<section data-case="false"><Indicator pulse={false}/></section><section data-case="true"><Indicator pulse/></section><section data-case="dynamic"><Indicator pulse={count > 1}/></section><Slot pulse={true}><Observer value={count}/><span class="writer">slot {count}</span></Slot>{/if}<button data-action="inc" on:click={count++}>Increment</button><button data-action="show" on:click={visible = !visible}>Show</button></div>
<style>.writer { color: var(--fg); }</style>"#;
    let root = app(
        "runtime",
        &[
            ("ui/indicator.brain", INDICATOR),
            (
                "ui/slot.brain",
                "<meta props=\"pulse:bool\">{#if pulse}<aside><slot/></aside>{/if}",
            ),
            (
                "channels/life.jsbrain",
                "export default store({ events: [] })",
            ),
            (
                "comp/observer.brain",
                "<meta props=\"value:num\"><script>import life from '../channels/life.jsbrain'\nfx(() => { let n = value; life.events.push('start:' + n); return () => life.events.push('stop:' + n) })</script><em>{value}</em>",
            ),
            ("comp/page.brain", source),
        ],
    );
    let out = emit(
        &root,
        &[
            "comp/page.brain",
            "comp/observer.brain",
            "channels/life.jsbrain",
        ],
    );
    let parent = brain_codegen::component_hash("comp/page.brain");
    node(
        &out,
        &format!(
            r#"
const C=(await import('file://'+directory+'/comp/page.brain.js')).default;
const life=(await import('file://'+directory+'/channels/life.jsbrain.js')).default;
const host=doc.createElement('div');doc.body.appendChild(host);const dispose=mount(C,host,{{}});
const walk=n=>Array.from(n.childNodes).flatMap(c=>(c.nodeType===1?[c]:[]).concat(walk(c)));
const find=(key,value)=>walk(host).find(n=>n.getAttribute(key)===value);
const tag=(node,name)=>walk(node).filter(n=>n.tagName?.toLowerCase()===name);
const events=()=>[...life.events];
assert.equal(tag(find('data-case','false'),'path').length,0);
const fixed=tag(find('data-case','true'),'svg')[0];assert(fixed);
assert.equal(tag(find('data-case','dynamic'),'path').length,0);
assert.equal(find('class','writer').getAttribute('data-b'),'{parent}');
assert.equal(host.querySelector('em').textContent,'1');assert.deepEqual(events(),['start:1']);
const inc=find('data-action','inc'),show=find('data-action','show');
const click=n=>n.dispatchEvent(new MouseEvent('click',{{bubbles:true}}));
click(inc);assert.equal(host.querySelector('em').textContent,'2');assert.equal(find('class','writer').textContent,'slot 2');assert.equal(tag(find('data-case','true'),'svg')[0],fixed);assert.equal(tag(find('data-case','dynamic'),'path').length,1);assert.deepEqual(events(),['start:1','stop:1','start:2']);
click(show);assert.equal(host.querySelector('em'),null);assert.deepEqual(events(),['start:1','stop:1','start:2','stop:2']);
click(inc);assert.deepEqual(events(),['start:1','stop:1','start:2','stop:2']);
click(show);assert.equal(host.querySelector('em').textContent,'3');assert.notEqual(tag(find('data-case','true'),'svg')[0],fixed);assert.deepEqual(events(),['start:1','stop:1','start:2','stop:2','start:3']);
dispose();assert.equal(host.childNodes.length,0);assert.deepEqual(events(),['start:1','stop:1','start:2','stop:2','start:3','stop:3']);
"#
        ),
    );
}
