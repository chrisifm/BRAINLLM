//! #104 (INTERNALS §32): dynamic `<meta seo>`, `<head>` block, `paths()`, and WebMCP tools.

use std::path::Path;

const VIEW: &str = r#"<meta seo title={props.post.title} description={`${props.post.summary}`} canonical="/blog/post" og:image="/og.png" keyword="rust" jsonld={{ '@type': 'Article', headline: props.post.title }}>
<script>
export async function load(ctx) { return { post: { title: 'Hi', summary: 'S' } } }
export async function paths() { return [{ slug: 'hi' }] }
</script>
<head>
  <link rel="alternate" hreflang="es" href={`/es/blog/${props.post.title}`} />
  <meta name="author" content="Ada" />
</head>
<main><h1>{props.post.title}</h1></main>"#;

#[test]
fn seo_and_head_reach_server_and_client() {
    let o = brain_codegen::compile(VIEW, Path::new("views/blog/[slug].brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    // Server: load, paths, seo(props), head(props).
    for want in ["export async function load(ctx)", "export async function paths()", "export const seo = (props) => ({ \"title\": props.post.title", "\"og:image\": \"/og.png\"", "\"jsonld\": { '@type': 'Article', headline: props.post.title }", "export const head = (props) =>"] {
        assert!(o.server_js.contains(want), "missing {want}:\n{}", o.server_js);
    }
    assert!(!o.server_js.contains("keyword"), "keyword is audit-only:\n{}", o.server_js);
    // Client: no load/paths, seo/head for the router, the <head> block is not rendered in the body.
    assert!(!o.js.contains("function paths") && !o.js.contains("function load"), "{}", o.js);
    assert!(o.js.contains(".seo = ($p) => ({ \"title\": $p.post.title"), "{}", o.js);
    assert!(o.js.contains(".head = ($p) =>") && o.js.contains("hreflang=\"es\""), "{}", o.js);
    assert!(!o.js.contains("tpl(\"<link"), "{}", o.js);
}

#[test]
fn head_block_escapes_interpolations_when_run() {
    let o = brain_codegen::compile(VIEW, Path::new("views/blog/[slug].brain")).unwrap();
    let line = o.server_js.lines().find(|l| l.starts_with("export const head")).unwrap();
    let f = line.trim_start_matches("export const head = ").trim_end_matches(';');
    let script = format!("const head = {f};\nconsole.log(head({{ post: {{ title: '\"><script>' }} }}))");
    let out = std::process::Command::new("node").arg("-e").arg(&script).output();
    let Ok(out) = out else { return };
    let html = String::from_utf8_lossy(&out.stdout);
    assert!(html.contains("href=\"/es/blog/&#34;&#62;&#60;script&#62;\""), "{html}");
    assert!(html.contains("<meta name=\"author\" content=\"Ada\">"), "{html}");
}

#[test]
fn imperative_tools_are_planned_and_declarative_forms_compile() {
    // #184: `<meta tools>` needs the runtime's brain-mcp.js, not shipped yet → B0128, never a dangling import.
    let src = r#"<meta tools="add_task: Add a task (text:str!)">
<script>
let items = sig([])
const add_task = async ({ text }) => { items = [...items, { id: Date.now(), text }] }
</script>
<p>{items.length}</p>"#;
    let d = brain_codegen::compile(src, Path::new("comp/tasks.brain")).expect_err("planned");
    assert!(d.iter().any(|d| d.code == "B0128"), "{d:#?}");
    // The declarative form is browser-native and compiles today.
    let form = r#"<form tool="search_tasks" tool-description="Search tasks"><input aria-label="Query" name="q" /></form>"#;
    let o = brain_codegen::compile(form, Path::new("comp/search.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("toolname=\\\"search_tasks\\\" tooldescription=\\\"Search tasks\\\""), "{}", o.js);
}

#[test]
fn faq_marker_survives_in_the_html() {
    // brain-seo (#110) builds FAQPage JSON-LD from `<details>` inside an element marked `faq`.
    let o = brain_codegen::compile("<section faq><details><summary>Q?</summary><p>A.</p></details></section>", Path::new("comp/faq.brain")).unwrap();
    assert!(o.js.contains(" faq><details>"), "{}", o.js);
}

#[test]
fn title_template_from_app_brain_is_applied_at_compile_time() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("title_template-{}", std::process::id()));
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::write(root.join("app.brain"), "<meta seo titleTemplate=\"%s · Acme\">\n<html lang=\"en\"><head></head><body><slot /></body></html>\n").unwrap();
    let opts = brain_codegen::Options { root: Some(root.clone()), ..Default::default() };
    let stat = "<meta seo title=\"Pricing\" description=\"Plans.\">\n<main><h1>Pricing</h1></main>";
    let o = brain_codegen::compile_with(stat, Path::new("views/pricing.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("\"title\": \"Pricing · Acme\""), "{}", o.js);
    assert!(o.server_js.contains("\"title\": \"Pricing · Acme\""), "{}", o.server_js);
    let dynamic = "<meta seo title={props.p.name} description=\"d\">\n<script>export async function load(ctx) { return { p: { name: 'X' } } }</script>\n<main><h1>x</h1></main>";
    let o = brain_codegen::compile_with(dynamic, Path::new("views/p.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("\"title\": \"\" + ($p.p.name) + \" · Acme\""), "{}", o.js);
    // Without app.brain nothing changes.
    let o = brain_codegen::compile(stat, Path::new("views/pricing.brain")).unwrap();
    assert!(o.js.contains("\"title\": \"Pricing\""), "{}", o.js);
}
