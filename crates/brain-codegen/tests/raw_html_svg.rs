//! #34: `{@html sanitize(x)}` (B0210) and per-instance id encapsulation of SVG references
//! (`url(#id)`, `href="#id"`), verified on the runtime's DOM shim.

use std::path::Path;
use std::process::Command;

const HTML: &str = r#"<script>
let body = sig('<b>hi</b>')
</script>
<p>Intro {@html sanitize(body)} end</p>"#;

const SVG: &str = r##"<svg viewBox="0 0 10 10" aria-hidden="true">
  <defs>
    <linearGradient id="g"><stop offset="0" stop-color="red" /></linearGradient>
    <clipPath id="c"><rect width="10" height="10" /></clipPath>
    <path id="p" d="M0 0 L10 10" />
  </defs>
  <g fill="url(#g)" clip-path="url('#c')" style="stroke: url(#g); opacity: 1">
    <rect width="10" height="10" />
    <use href="#p" />
    <use xlink:href="#p" />
    <rect fill="url(#other)" width="1" height="1" />
  </g>
</svg>"##;

fn compile(src: &str, path: &str, runtime: &str) -> String {
    let opts = brain_codegen::Options { runtime: runtime.into(), ..Default::default() };
    brain_codegen::compile_with(src, Path::new(path), &opts).unwrap_or_else(|d| panic!("{d:#?}")).js
}

#[test]
fn raw_html_compiles_to_html_from_brain_data() {
    let js = compile(HTML, "comp/raw.brain", "./brain.js");
    assert!(js.contains("import { sanitize, html } from \"./brain-data.js\";") || js.contains("import { html, sanitize }"), "{js}");
    assert!(js.contains("html($h") && js.contains("() => sanitize(body"), "{js}");
}

#[test]
fn raw_html_without_sanitize_is_b0210() {
    let src = "<script>\nlet body = sig('x')\n</script>\n<div>{@html body}</div>";
    let d = brain_codegen::compile(src, Path::new("comp/raw.brain")).expect_err("B0210 expected");
    assert!(d.iter().any(|d| d.code == "B0210"), "{d:#?}");
}

#[test]
fn svg_references_follow_the_instance_id() {
    let js = compile(SVG, "comp/logo.brain", "./brain.js");
    for want in [r#""url(#g" + $u + ")""#, r#""url('#c" + $u + "')""#, r#""stroke: url(#g" + $u + "); opacity: 1""#, r##""#p" + $u"##] {
        assert!(js.contains(want), "missing {want} in:\n{js}");
    }
    assert!(js.contains("\"href\", \"#p\" + $u") && !js.contains("\"xlink:href\""), "{js}");
    // A reference to an id this component does not declare stays literal.
    assert!(js.contains("url(#other)"), "{js}");
}

#[test]
fn svg_references_resolve_on_the_runtime() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("node is not installed: skipping e2e");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let runtime = format!("file://{}", root.join("runtime/brain.js").display());
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("raw_html_svg-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("logo.js"), compile(SVG, "comp/logo.brain", &runtime)).unwrap();
    std::fs::write(out.join("raw.js"), compile(HTML, "comp/raw.brain", &runtime)).unwrap();
    let script = r#"
import assert from 'node:assert/strict'
const [dir, shim, rt] = process.argv.slice(2)
const { install } = await import('file://' + shim)
const doc = install()
const { mount } = await import(rt)
const walk = (n, out = []) => { for (const c of n.childNodes) { if (c.nodeType === 1) out.push(c); walk(c, out) } return out }
const Logo = (await import('file://' + dir + '/logo.js')).default
const ids = []
for (let i = 0; i < 2; i++) {
  const host = doc.createElement('div'); doc.body.appendChild(host); mount(Logo, host, {})
  const els = walk(host)
  const byId = id => els.find(e => e.getAttribute('id') === id)
  const grad = els.find(e => (e.tagName || e.nodeName).toLowerCase() === 'lineargradient').getAttribute('id')
  const clip = els.find(e => (e.tagName || e.nodeName).toLowerCase() === 'clippath').getAttribute('id')
  const g = els.find(e => (e.tagName || e.nodeName).toLowerCase() === 'g')
  assert.equal(g.getAttribute('fill'), `url(#${grad})`)
  assert.ok(byId(g.getAttribute('fill').slice(5, -1)), 'fill must resolve to the gradient')
  assert.equal(g.getAttribute('clip-path'), `url('#${clip}')`)
  assert.equal(g.getAttribute('style'), `stroke: url(#${grad}); opacity: 1`)
  for (const u of els.filter(e => (e.tagName || e.nodeName).toLowerCase() === 'use')) assert.ok(byId(u.getAttribute('href').slice(1)), 'use must resolve')
  ids.push(grad)
}
assert.notEqual(ids[0], ids[1], 'each instance gets its own ids')
const Raw = (await import('file://' + dir + '/raw.js')).default
const host = doc.createElement('div'); doc.body.appendChild(host); mount(Raw, host, {})
assert.equal(host.textContent.replace(/\s+/g, ' ').trim(), 'Intro hi end')
console.log('ok svg-refs')
"#;
    std::fs::write(out.join("run.mjs"), script).unwrap();
    let r = Command::new("node").arg(out.join("run.mjs")).arg(&out).arg(root.join("runtime/shim/dom.js")).arg(&runtime).output().unwrap();
    let so = String::from_utf8_lossy(&r.stdout);
    assert!(r.status.success() && so.contains("ok svg-refs"), "{so}\n{}", String::from_utf8_lossy(&r.stderr));
}

#[test]
fn view_ids_are_public_fragments_without_suffix() {
    let view = "<meta seo title=\"Pricing\" description=\"Plans.\">\n<main><h1>Pricing</h1><a href=\"#faq\">FAQ</a><h2 id=\"faq\">Questions</h2></main>";
    let js = compile(view, "views/pricing.brain", "./brain.js");
    assert!(js.contains("<h2 id=\\\"faq\\\">") && !js.contains("$u"), "{js}");
    let layout = "<nav id=\"top\"><a href=\"#top\">Top</a></nav><slot />";
    assert!(!compile(layout, "views/_layout.brain", "./brain.js").contains("$u"));
    // A reusable component still gets per-instance ids.
    assert!(compile("<label for=\"q\">Q</label><input id=\"q\" />", "comp/search.brain", "./brain.js").contains("$u"));
}

#[test]
fn raw_code_blocks_are_literal_text() {
    // SEO #167: code samples with braces and tags, written as-is inside `<pre raw>` / `<code raw>`.
    let src = "<meta seo title=\"Docs\" description=\"Code sample.\">\n<main><h1>Docs</h1><pre><code raw>let n = sig(0)\n<button on:click={n++}>{n}</button> &amp; a & b</code></pre></main>";
    let o = brain_codegen::compile(src, Path::new("views/docs.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("<code>let n = sig(0)\\n&lt;button on:click={n++}&gt;{n}&lt;/button&gt; &amp; a &amp; b</code>"), "{}", o.js);
    assert!(!o.js.contains("raw"), "the `raw` marker is not emitted: {}", o.js);
}

#[test]
fn raw_samples_are_opaque_to_block_scanning_and_keep_whitespace() {
    // SEO #167: a component sample with <meta>/<script>/<style> inside `<code raw>` is not a block of the file.
    let src = "<meta seo title=\"Docs\" description=\"Code.\">\n<main><h1>Docs</h1>\n<pre><code raw><meta props=\"label:str='Clicks'\">\n<script>\n  let n = sig(0)\n</script>\n<button on:click={n++}>{n}</button>\n<style>\n  button { color: red }\n</style></code></pre>\n<pre>keep   these\n  lines</pre>\n</main>\n<style>main { color: var(--fg); }</style>\n";
    let o = brain_codegen::compile(src, Path::new("views/docs.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.warnings.iter().all(|d| d.code != "B0125"), "{:?}", o.warnings);
    assert!(o.js.contains("&lt;script&gt;\\n  let n = sig(0)\\n&lt;/script&gt;"), "{}", o.js);
    assert!(o.js.contains("<pre>keep   these\\n  lines</pre>"), "whitespace in <pre> is content: {}", o.js);
    assert!(o.css.contains("color: var(--fg)") && !o.css.contains("color: red"), "{}", o.css);
    // The block hashes ignore the sample too: editing it is a markup change, not a style one.
    let b1 = brain_codegen::blocks::blocks(src, Path::new("views/docs.brain"));
    let b2 = brain_codegen::blocks::blocks(&src.replace("color: red", "color: blue"), Path::new("views/docs.brain"));
    assert_eq!(brain_codegen::blocks::change(&b1, &b2), brain_codegen::blocks::Change::Component);
}
