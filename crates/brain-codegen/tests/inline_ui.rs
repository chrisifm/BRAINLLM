//! #34 (INTERNALS §13): stateless `ui/` components are inlined in production — no call, no props object — and keep
//! their scope hash and CSS; stateful ones, dev builds and uses with events stay regular calls.

use std::path::Path;
use std::process::Command;

fn app(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("inline_ui-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    for (p, src) in files {
        let f = dir.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, src).unwrap();
    }
    dir
}

const PILL: &str = r#"<meta props="tone:str='info' label:str">
<span class="pill" data-tone={tone}>{label}: <slot /></span>
<style>.pill { color: var(--fg); }</style>"#;

const SCRIPTED: &str = r#"<meta props="label:str">
<script>
let upper = drv(() => label.toUpperCase())
</script>
<b>{upper}</b>"#;

const VIEW: &str = r#"<script>
import Pill from '../ui/pill.brain'
import Toggle from '../ui/toggle.brain'
let n = sig(1)
</script>
<div>
  <Pill label="Count">{n}</Pill>
  <Pill tone="ok" label={`n=${n}`}>static</Pill>
  <Toggle label="More" />
  <button on:click={n++}>+</button>
</div>"#;

fn compile(root: &Path, dev: bool, runtime: &str) -> String {
    let opts = brain_codegen::Options { root: Some(root.to_path_buf()), dev, runtime: runtime.into(), ..Default::default() };
    brain_codegen::compile_with(VIEW, Path::new("comp/view.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}")).js
}

fn files(name: &str) -> std::path::PathBuf {
    app(name, &[("ui/pill.brain", PILL), ("ui/toggle.brain", SCRIPTED), ("comp/view.brain", VIEW)])
}

#[test]
fn stateless_ui_is_inlined_in_production() {
    let root = files("prod");
    let js = compile(&root, false, "./brain.js");
    let hash = brain_codegen::component_hash("ui/pill.brain");
    assert!(!js.contains("Pill("), "Pill must not be called:\n{js}");
    assert!(!js.contains("ui/pill.brain"), "no import or chunk is left for an inlined component:\n{js}");
    assert!(js.contains(&format!("data-b=\\\"{hash}\\\" class=\\\"pill\\\"")), "child scope hash on the root:\n{js}");
    // Default prop, static prop and reactive prop.
    assert!(js.contains("attr($e") && js.contains("'info'") && js.contains("\"ok\""), "{js}");
    assert!(js.contains("() => (`n=${n.v}`)"), "{js}");
    // A component with a script is still a call.
    assert!(js.contains("import Toggle from \"../ui/toggle.brain\";") && js.contains("Toggle({"), "{js}");
}

#[test]
fn dev_builds_keep_the_call_for_hmr() {
    let root = files("dev");
    let js = compile(&root, true, "./brain.js");
    assert!(js.contains("Pill({"), "{js}");
}

#[test]
fn inlined_component_runs_with_slot_and_reactive_props() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("node is not installed: skipping e2e");
        return;
    }
    let root = files("e2e");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let runtime = format!("file://{}", repo.join("runtime/brain.js").display());
    let out = root.join("out");
    std::fs::create_dir_all(out.join("comp")).unwrap();
    std::fs::create_dir_all(out.join("ui")).unwrap();
    let opts = |dev| brain_codegen::Options { root: Some(root.clone()), dev, runtime: runtime.clone(), import_suffix: ".js".into(), ..Default::default() };
    for (rel, src) in [("comp/view.brain", VIEW), ("ui/pill.brain", PILL), ("ui/toggle.brain", SCRIPTED)] {
        let o = brain_codegen::compile_with(src, Path::new(rel), &opts(false)).unwrap_or_else(|d| panic!("{d:#?}"));
        std::fs::write(out.join(format!("{rel}.js")), o.js).unwrap();
    }
    let script = r#"
import assert from 'node:assert/strict'
const [dir, shim, rt] = process.argv.slice(2)
const { install, MouseEvent } = await import('file://' + shim)
const doc = install()
const { mount } = await import(rt)
const View = (await import('file://' + dir + '/comp/view.brain.js')).default
const host = doc.createElement('div'); doc.body.appendChild(host); mount(View, host, {})
const spans = []
const walk = n => { for (const c of n.childNodes) { if (c.nodeType === 1 && (c.tagName || c.nodeName).toLowerCase() === 'span') spans.push(c); walk(c) } }
walk(host)
const t = el => el.textContent.replace(/\s+/g, ' ').trim()
assert.equal(spans.length, 2)
assert.equal(spans[0].getAttribute('data-tone'), 'info')
assert.equal(spans[1].getAttribute('data-tone'), 'ok')
assert.equal(t(spans[0]), 'Count: 1')
assert.equal(t(spans[1]), 'n=1: static')
const btns = []
const wb = n => { for (const c of n.childNodes) { if (c.nodeType === 1 && (c.tagName || c.nodeName).toLowerCase() === 'button') btns.push(c); wb(c) } }
wb(host)
btns[btns.length - 1].dispatchEvent(new MouseEvent('click', { bubbles: true }))
assert.equal(t(spans[0]), 'Count: 2')
assert.equal(t(spans[1]), 'n=2: static')
const bs = []
const wbb = n => { for (const c of n.childNodes) { if (c.nodeType === 1 && (c.tagName || c.nodeName).toLowerCase() === 'b') bs.push(c); wbb(c) } }
wbb(host)
assert.equal(t(bs[0]), 'MORE')
console.log('ok inline-ui')
"#;
    std::fs::write(out.join("run.mjs"), script).unwrap();
    let r = Command::new("node").arg(out.join("run.mjs")).arg(&out).arg(repo.join("runtime/shim/dom.js")).arg(&runtime).output().unwrap();
    let so = String::from_utf8_lossy(&r.stdout);
    assert!(r.status.success() && so.contains("ok inline-ui"), "{so}\n{}", String::from_utf8_lossy(&r.stderr));
}

#[test]
fn inlined_component_css_travels_in_the_parent() {
    let root = files("css");
    let opts = brain_codegen::Options { root: Some(root.clone()), ..Default::default() };
    let o = brain_codegen::compile_with(VIEW, Path::new("comp/view.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    let hash = brain_codegen::component_hash("ui/pill.brain");
    assert!(o.css.contains(&format!("@scope ([data-b=\"{hash}\"])")) && o.css.contains(".pill"), "{}", o.css);
}

#[test]
fn slot_content_keeps_the_writers_scope() {
    // #194 (INTERNALS §7): with the same class in parent and child, the child's CSS must not style the slotted nodes.
    let root = app("slot-scope", &[
        ("comp/card.brain", "<div class=\"title\"><slot /></div>\n<style>.title { color: var(--bad); }</style>"),
        ("comp/page.brain", "<script>\nimport Card from './card.brain'\n</script>\n<Card><p class=\"title\">Hello</p></Card>\n<style>.title { color: var(--fg); }</style>"),
    ]);
    let src = std::fs::read_to_string(root.join("comp/page.brain")).unwrap();
    let opts = brain_codegen::Options { root: Some(root.clone()), ..Default::default() };
    let o = brain_codegen::compile_with(&src, Path::new("comp/page.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    let parent = brain_codegen::component_hash("comp/page.brain");
    // The slotted <p> carries the parent's data-b: the child's `@scope (…) to ([data-b])` stops before it.
    assert!(o.js.contains(&format!("<p data-b=\\\"{parent}\\\" class=\\\"title\\\">Hello</p>")), "{}", o.js);
}
