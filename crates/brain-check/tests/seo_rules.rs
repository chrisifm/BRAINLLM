//! #104 (INTERNALS §32): B1601–B1604 metadata rules, B1701–B1703 WebMCP tools, `paths()` and `ViewSeo`.

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
    assert_eq!(codes_in("comp/t.brain", &comp("addTask: Add a task (text:str!)", "const addTask = () => 1"), &ctx), ["B1702"]);
    assert_eq!(codes_in("comp/t.brain", &comp("add_task: Add (text)", "const add_task = () => 1"), &ctx), ["B1703"]);
    assert_eq!(codes_in("comp/t.brain", &comp("a: A; a: B", "const a = () => 1"), &ctx), ["B1702"]);
    assert_eq!(codes_in("comp/t.brain", "<form tool=\"Search\"><input aria-label=\"q\" /></form>", &ctx), ["B1702"]);
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
