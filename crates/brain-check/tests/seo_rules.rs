//! #104 (INTERNALS §32): B1601–B1604 metadata rules, B1701–B1704 WebMCP tools, `paths()` and `ViewSeo`.

use brain_check::seo::{view_info, SeoValue};
use brain_check::Ctx;
use std::path::Path;

fn codes_in(path: &str, src: &str, ctx: &Ctx) -> Vec<&'static str> {
    let c = brain_parse::parse(src, Path::new(path)).unwrap_or_else(|d| panic!("{d:?}"));
    let d = brain_check::check(&c, ctx);
    for x in &d {
        assert!(!x.fix.is_empty() && !x.msg.is_empty(), "{x}");
    }
    d.into_iter().map(|d| d.code).filter(|c| c.starts_with("B16") || c.starts_with("B17") || *c == "B0117" || *c == "B0111").collect()
}

fn view(seo: &str) -> String {
    format!("<meta seo {seo}>\n<script>export async function load(ctx) {{ return {{ u: {{ name: 'A' }} }} }}</script>\n<main><h1>Users</h1></main>")
}

#[test]
fn b16xx_metadata() {
    let ctx = Ctx::default();
    assert!(codes_in("views/u.brain", &view("title=\"Users\" description=\"All users.\" canonical=\"/u\""), &ctx).is_empty());
    assert!(codes_in("views/u.brain", &view("title={props.u.name} description={`About ${props.u.name}`}"), &ctx).is_empty());
    assert_eq!(codes_in("views/u.brain", &view("title=\"Users\""), &ctx), ["B1601"]);
    let long = "x".repeat(61);
    assert_eq!(codes_in("views/u.brain", &view(&format!("title=\"{long}\" description=\"d\"")), &ctx), ["B1602"]);
    assert_eq!(codes_in("views/u.brain", &view("title=\"T\" description=\"d\" jsonld={{ name: props.u.name }}"), &ctx), ["B1603"]);
    assert!(codes_in("views/u.brain", &view("title=\"T\" description=\"d\" jsonld={{ '@type': 'Person', name: props.u.name }}"), &ctx).is_empty());
    // An undeclared name in a seo expression is caught like anywhere else.
    assert_eq!(codes_in("views/u.brain", &view("title={user.name} description=\"d\""), &ctx), ["B0111"]);
    let mut ctx = Ctx::default();
    ctx.canonicals.insert("/u".into(), vec!["views/u.brain".into(), "views/other.brain".into()]);
    assert_eq!(codes_in("views/u.brain", &view("title=\"T\" description=\"d\" canonical=\"/u\""), &ctx), ["B1604"]);
}

fn b0502_in(path: &str, seo: &str, ctx: &Ctx) -> bool {
    let src = view(seo);
    let c = brain_parse::parse(&src, Path::new(path)).unwrap_or_else(|d| panic!("{d:?}"));
    brain_check::check(&c, ctx).iter().any(|d| d.code == "B0502")
}

#[test]
fn b0502_indexable_view_names_its_query() {
    let ctx = Ctx::default();
    let seo = "title=\"Users\" description=\"All users.\"";
    assert!(b0502_in("views/u.brain", seo, &ctx));
    assert!(!b0502_in("views/u.brain", &format!("keyword=\"users\" {seo}"), &ctx));
    // pages that should not rank are not asked for a query
    assert!(!b0502_in("views/u.brain", &format!("{seo} robots=\"noindex, follow\""), &ctx));
    assert!(!b0502_in("views/_offline.brain", seo, &ctx));
    assert!(!b0502_in("views/u/[id].brain", seo, &ctx));
    assert!(!b0502_in("views/404.brain", seo, &ctx));
    let guarded = Ctx { guarded: true, ..Ctx::default() };
    assert!(!b0502_in("views/u.brain", seo, &guarded));
    // the warning tells the author exactly what to write
    let c = brain_parse::parse(&view(seo), Path::new("views/u.brain")).unwrap();
    let d = brain_check::check(&c, &ctx).into_iter().find(|d| d.code == "B0502").unwrap();
    assert_eq!(d.level, brain_parse::Severity::Warning);
    assert!(d.fix.contains("keyword=\\\"") || d.fix.contains("keyword=\""), "{}", d.fix);
}

#[test]
fn b0502_guard_folder_is_detected_from_disk() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("b0502-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("views/account")).unwrap();
    std::fs::write(root.join("brain.json"), "{}").unwrap();
    std::fs::write(root.join("views/account/_guard.jsbrain"), "export default (ctx) => ctx.user ? null : '/login'\n").unwrap();
    let src = view("title=\"Account\" description=\"Your account.\"");
    for (rel, expect) in [("views/account/index.brain", false), ("views/open.brain", true)] {
        std::fs::write(root.join(rel), &src).unwrap();
        let c = brain_parse::parse(&src, Path::new(rel)).unwrap_or_else(|d| panic!("{d:?}"));
        let (ctx, _) = brain_check::load_ctx(&root.join(rel), &c);
        assert_eq!(ctx.guarded, !expect, "{rel}");
        assert_eq!(brain_check::check(&c, &ctx).iter().any(|d| d.code == "B0502"), expect, "{rel}");
    }
}

#[test]
fn paths_only_in_dynamic_views() {
    let src = "<meta seo title=\"T\" description=\"d\">\n<script>export async function paths() { return [] }</script>\n<main><h1>x</h1></main>";
    assert!(codes_in("views/u/[id].brain", src, &Ctx::default()).is_empty());
    assert_eq!(codes_in("views/u.brain", src, &Ctx::default()), ["B0117"]);
    // sitemap() (pages listed at request time) follows the same rule
    let sm = src.replace("paths()", "sitemap()");
    assert!(codes_in("views/u/[id].brain", &sm, &Ctx::default()).is_empty());
    assert_eq!(codes_in("views/u.brain", &sm, &Ctx::default()), ["B0117"]);
    // both in one view: two contracts for the same pages
    let both = src.replace("</script>", "export async function sitemap() { return [] }</script>");
    assert_eq!(codes_in("views/u/[id].brain", &both, &Ctx::default()), ["B0117"]);
}

#[test]
fn b17xx_tools() {
    let ctx = Ctx::default();
    let comp = |tools: &str, script: &str| format!("<meta tools=\"{tools}\">\n<script>{script}</script>\n<p>x</p>");
    assert!(codes_in("comp/t.brain", &comp("add_task: Add a task (text:str!)", "const add_task = ({ text }) => text"), &ctx).is_empty());
    assert_eq!(codes_in("comp/t.brain", &comp("add_task: Add a task (text:str!)", "const other = 1"), &ctx), ["B1701"]);
    assert_eq!(codes_in("comp/t.brain", &comp("addTask: Add a task (text:str!)", "const addTask = ({ text }) => 1"), &ctx), ["B1702"]);
    assert_eq!(codes_in("comp/t.brain", &comp("add_task: Add (text)", "const add_task = ({ text }) => 1"), &ctx), ["B1703"]);
    assert_eq!(codes_in("comp/t.brain", &comp("a: A; a: B", "const a = () => 1"), &ctx), ["B1702"]);
    assert_eq!(codes_in("comp/t.brain", "<form tool=\"Search\"><input aria-label=\"q\" /></form>", &ctx), ["B1702"]);
}

#[test]
fn b1704_tool_params_match_the_function() {
    // #106: the agent sends exactly the declared parameters; the function destructures exactly those.
    let ctx = Ctx::default();
    let comp = |tools: &str, script: &str| format!("<meta tools=\"{tools}\">\n<script>{script}</script>\n<p>x</p>");
    let ok = |tools: &str, script: &str| assert!(codes_in("comp/t.brain", &comp(tools, script), &ctx).is_empty(), "{tools} / {script}");
    ok("add_task: Add a task (text:str!, due:date)", "const add_task = async ({ text, due }) => text");
    ok("add_task: Add a task (text:str!, due:date)", "async function add_task({ due, text = '' }) { return text }");
    ok("add_task: Add a task (text:str!, due:date)", "const add_task = ({ text, due: when } = {}) => when");
    // Not destructured (a whole-object parameter or a rest element): the checker cannot tell, so it trusts the author.
    ok("add_task: Add a task (text:str!)", "const add_task = (args) => args.text");
    ok("add_task: Add a task (text:str!, due:date)", "const add_task = ({ text, ...rest }) => rest");
    ok("clear_done: Remove completed tasks", "const clear_done = () => 1");
    let bad = |tools: &str, script: &str| codes_in("comp/t.brain", &comp(tools, script), &ctx);
    assert_eq!(bad("add_task: Add a task (text:str!)", "const add_task = ({ text, due }) => due"), ["B1704"]);
    assert_eq!(bad("add_task: Add a task (text:str!, due:date)", "const add_task = ({ text }) => text"), ["B1704"]);
    assert_eq!(bad("add_task: Add a task (text:str!)", "function add_task() { return 1 }"), ["B1704"]);
    assert_eq!(bad("clear_done: Remove completed tasks", "const clear_done = ({ all }) => all"), ["B1704"]);
    // No function at all is B1701 only.
    assert_eq!(bad("add_task: Add a task (text:str!)", "const other = ({ x }) => x"), ["B1701"]);
}

#[test]
fn view_info_for_the_seo_module() {
    let src = "<meta seo title={props.p.title} description=\"About.\" canonical=\"/p\" keyword=\"rust\">\n<script>export async function load(ctx) { return { p: { title: 'T' } } }</script>\n<main><h1>Hello <b>world</b></h1><img src=\"/a.png\" alt=\"A\"><a href=\"https://x.dev\">X</a><a href=\"/in\">In</a></main>";
    let c = brain_parse::parse(src, Path::new("views/p.brain")).unwrap();
    let v = view_info(&c);
    assert_eq!(v.attrs[0], ("title".to_string(), SeoValue::Expr("props.p.title".into())));
    assert_eq!(v.canonical.as_deref(), Some("/p"));
    assert_eq!(v.keyword.as_deref(), Some("rust"));
    assert_eq!((v.headings[0].0, v.headings[0].1.as_str()), (1, "Hello world"));
    assert!(v.headings[0].2.line == 3 && v.headings[0].2.col > 1);
    assert_eq!((v.images[0].0.as_str(), v.images[0].1.as_deref()), ("/a.png", Some("A")));
    assert_eq!(v.links.iter().map(|l| (l.0.as_str(), l.2)).collect::<Vec<_>>(), [("https://x.dev", true), ("/in", false)]);
}
