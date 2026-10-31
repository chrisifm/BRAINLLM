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

const TASKS: &str = r#"<meta tools="add_task: Add a task (text:str!, due:date); clear_done: Remove completed tasks">
<script>
let items = sig([])
const add_task = async ({ text, due }) => { items = [...items, { id: items.length + 1, text, due }]; return items.length }
function clear_done() { items = []; return 'cleared' }
</script>
<p>{items.length}</p>"#;

#[test]
fn imperative_tools_compile_to_register_tools_and_declarative_forms_compile() {
    // #106 (INTERNALS §32): `<meta tools>` ships. The schema comes from the parameter list and `execute` is the
    // same-named function; the runtime's brain-mcp.js registers them while the component is mounted.
    let o = brain_codegen::compile(TASKS, Path::new("comp/tasks.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("import { registerTools } from \"/@brain/brain-mcp.js\";"), "{}", o.js);
    let add = r#"{ name: "add_task", description: "Add a task", inputSchema: { type: "object", properties: { "text": { type: "string" }, "due": { type: "string", format: "date" } }, required: ["text"] }, execute: add_task }"#;
    let clear = r#"{ name: "clear_done", description: "Remove completed tasks", inputSchema: { type: "object", properties: {}, required: [] }, execute: clear_done }"#;
    assert!(o.js.contains(&format!("registerTools([{add}, {clear}]);")), "{}", o.js);
    // Every parameter type of the mini-language maps to its JSON Schema.
    let all = r#"<meta tools="save_all: Save (a:str!, b:num, c:bool, d:list, e:obj, f:email, g:url, h:date, i:any)">
<script>const save_all = ({ a, b, c, d, e, f, g, h, i }) => a</script>
<p>x</p>"#;
    let o = brain_codegen::compile(all, Path::new("comp/all.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    let props = r#"properties: { "a": { type: "string" }, "b": { type: "number" }, "c": { type: "boolean" }, "d": { type: "array" }, "e": { type: "object" }, "f": { type: "string", format: "email" }, "g": { type: "string", format: "uri" }, "h": { type: "string", format: "date" }, "i": {} }, required: ["a"]"#;
    assert!(o.js.contains(props), "{}", o.js);
    // The declarative form is browser-native and compiles as before.
    let form = r#"<form tool="search_tasks" tool-description="Search tasks"><input aria-label="Query" name="q" /></form>"#;
    let o = brain_codegen::compile(form, Path::new("comp/search.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("toolname=\\\"search_tasks\\\" tooldescription=\\\"Search tasks\\\""), "{}", o.js);
    assert!(!o.js.contains("brain-mcp"), "{}", o.js);
}

#[test]
fn imperative_tool_without_its_function_or_with_other_params_does_not_compile() {
    // Never a registration pointing at a missing function, and never B0128 now that the runtime ships it.
    let missing = TASKS.replace("function clear_done()", "function clear_all()");
    let d = brain_codegen::compile(&missing, Path::new("comp/tasks.brain")).expect_err("B1701");
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["B1701"], "{d:#?}");
    // The function reads a parameter the tool does not declare (agents never send it), or ignores a declared one.
    let reads_more = TASKS.replace("({ text, due })", "({ text, due, owner })");
    let d = brain_codegen::compile(&reads_more, Path::new("comp/tasks.brain")).expect_err("B1704");
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["B1704"], "{d:#?}");
    assert!(d[0].msg.contains("`owner`"), "{d:#?}");
    let reads_less = TASKS.replace("({ text, due })", "({ text })").replace(", due }]", " }]");
    let d = brain_codegen::compile(&reads_less, Path::new("comp/tasks.brain")).expect_err("B1704");
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["B1704"], "{d:#?}");
    assert!(d[0].msg.contains("`due`"), "{d:#?}");
}

#[test]
fn imperative_tool_runs_its_function_on_the_runtime() {
    // End to end: the compiled component on runtime/brain.js + brain-mcp.js with a fake browser model context.
    if std::process::Command::new("node").arg("--version").output().is_err() {
        eprintln!("node is not installed: skipping");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let runtime = format!("file://{}", root.join("runtime/brain.js").display());
    let opts = brain_codegen::Options { runtime: runtime.clone(), ..Default::default() };
    let o = brain_codegen::compile_with(TASKS, Path::new("comp/tasks.brain"), &opts).unwrap_or_else(|d| panic!("{d:#?}"));
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("meta-tools-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tasks.mjs"), &o.js).unwrap();
    let script = format!(
        r#"const {{ install }} = await import({shim:?})
const doc = install()
const reg = new Map()
doc.modelContext = {{ registerTool: t => (reg.set(t.name, t), {{ unregister: () => reg.delete(t.name) }}) }}
const {{ mount }} = await import({runtime:?})
const Tasks = (await import({comp:?})).default
const host = doc.body.appendChild(doc.createElement('div'))
const stop = mount(Tasks, host)
const out = [[...reg.keys()].join(',')]
out.push(JSON.stringify(await reg.get('add_task').execute({{ text: 'Buy milk', due: '2026-11-02' }})))
out.push(host.textContent)
out.push(JSON.stringify(await reg.get('add_task').execute({{}})))
out.push(JSON.stringify(await reg.get('clear_done').execute({{}})))
out.push(host.textContent)
stop()
out.push(reg.size)
console.log(out.join(' | '))"#,
        shim = format!("file://{}", root.join("runtime/shim/dom.js").display()),
        comp = format!("file://{}", dir.join("tasks.mjs").display()),
    );
    let r = std::process::Command::new("node").args(["--input-type=module", "-e", &script]).output().unwrap();
    let so = String::from_utf8_lossy(&r.stdout);
    assert!(r.status.success(), "{so}\n{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(
        so.trim(),
        r#"add_task,clear_done | {"content":[{"type":"text","text":"1"}]} | 1 | {"content":[{"type":"text","text":"text: Required"}]} | {"content":[{"type":"text","text":"cleared"}]} | 0 | 0"#
    );
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
