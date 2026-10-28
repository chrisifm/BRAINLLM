//! Generated JS/CSS of every example, in prod and dev. Regenerate with
//! `UPDATE_SNAPSHOTS=1 cargo test -p brain-codegen --test snapshots`.

use std::path::Path;

const FILES: &[&str] = &["comp/counter.brain", "comp/todo.brain", "comp/user-form.brain", "ui/badge.brain", "views/dashboard.brain", "views/index.brain", "lib/format.jsbrain"];

#[test]
fn generated_code_matches_snapshots() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let snaps = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let update = std::env::var("UPDATE_SNAPSHOTS").is_ok();
    let mut bad = Vec::new();
    for rel in FILES {
        let src = std::fs::read_to_string(examples.join(rel)).unwrap();
        for dev in [false, true] {
            let opts = brain_codegen::Options { dev, root: Some(examples.clone()), ..Default::default() };
            let o = brain_codegen::compile_with(&src, Path::new(rel), &opts).unwrap_or_else(|d| panic!("{rel}: {d:?}"));
            let mut got = format!("{}\n/* ---- css ---- */\n{}", o.js, o.css);
            if !o.server_js.is_empty() {
                got.push_str(&format!("\n/* ---- server ---- */\n{}", o.server_js));
            }
            let name = format!("{}{}.out", rel.replace('/', "__"), if dev { ".dev" } else { "" });
            let path = snaps.join(&name);
            if update || !path.exists() {
                std::fs::write(&path, &got).unwrap();
            } else if std::fs::read_to_string(&path).unwrap() != got {
                bad.push(format!("{name} differs:\n{got}"));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n\n"));
}

#[test]
fn web_component_target() {
    let src = r#"<meta props="start:num=0 label:str='Clicks'">
<meta export="wc" tag="brain-counter" form>
<script>let n = sig(start)</script>
<button on:click={n++}>{label}: {n}</button>
<style>button { color: var(--fg) }</style>"#;
    let o = brain_codegen::compile(src, Path::new("comp/counter.brain")).unwrap();
    assert!(o.js.contains("import { define } from \"/@brain/brain-wc.js\";"), "{}", o.js);
    assert!(o.js.contains("Counter.p = [\"start\", \"label\"];"), "{}", o.js);
    assert!(o.js.contains("Counter.form = true;"), "{}", o.js);
    assert!(o.js.contains("define(\"brain-counter\", Counter, \"button {\\n  color: var(--fg);\\n}\\n\");"), "{}", o.js);
}

#[test]
fn static_ids_get_an_instance_suffix() {
    let src = r#"<label for="q">Search</label><input id="q" aria-describedby="q-help ext" /><p id="q-help">Help</p>"#;
    let o = brain_codegen::compile(src, Path::new("comp/search.brain")).unwrap();
    assert!(o.js.contains("let $n = 0;"), "{}", o.js);
    assert!(o.js.contains("const $u = $n++ ? \"~\" + $n : \"\";"), "{}", o.js);
    assert!(o.js.contains("attr($e1, \"for\", \"q\" + $u);"), "{}", o.js);
    assert!(o.js.contains("\"id\", \"q\" + $u);"), "{}", o.js);
    assert!(o.js.contains("\"aria-describedby\", \"q-help\" + $u + \" \" + \"ext\");"), "{}", o.js);
    assert!(!o.js.contains("id=\\\"q\\\""), "{}", o.js);
}

#[test]
fn computed_ids_get_an_instance_suffix() {
    // #308 N1: a FAQ in comp/ with `id={'faq-' + it.id}` repeated on a page must not produce duplicate ids.
    let src = r#"<meta props="items:list">
{#each items as it (it.id)}<button aria-controls={'faq-' + it.id}>{it.q}</button><div id={'faq-' + it.id}>{it.a}</div><p id="n-{it.id}">x</p>{/each}"#;
    let o = brain_codegen::compile(src, Path::new("comp/faq.brain")).unwrap_or_else(|d| panic!("{d:?}"));
    assert!(o.js.contains("const $u = $n++ ? \"~\" + $n : \"\";"), "{}", o.js);
    assert!(o.js.contains("const $sx = (v, u) =>"), "{}", o.js);
    assert!(o.js.contains("\"aria-controls\", () => $sx(") || o.js.contains("\"aria-controls\", $sx("), "{}", o.js);
    assert!(o.js.contains("\"id\", () => $sx(") || o.js.contains("\"id\", $sx("), "{}", o.js);
    assert_eq!(o.js.matches("$sx(").count(), 3, "{}", o.js);
    // A view renders once per document: its computed ids stay as written.
    let view = src.replace("<meta props=\"items:list\">", "<script>\n  let items = sig([])\n</script>");
    let v = brain_codegen::compile(&view, Path::new("views/faq.brain"));
    if let Ok(v) = v {
        assert!(!v.js.contains("$sx"), "{}", v.js);
    }
}

#[test]
fn computed_reference_to_a_static_id_gets_the_suffix() {
    let src = r#"<script>
  let open = sig(false)
</script>
<button aria-controls={open ? 'panel' : null} on:click={open = !open}>T</button><div id="panel" hidden={!open}>x</div>"#;
    let o = brain_codegen::compile(src, Path::new("comp/disclosure.brain")).unwrap_or_else(|d| panic!("{d:?}"));
    assert!(o.js.contains("const $ix = new Set([\"panel\"]);"), "{}", o.js);
    assert!(o.js.contains("$sx("), "{}", o.js);
}
