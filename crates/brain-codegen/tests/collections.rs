//! #135 (§33): collection readers are server-only primitives. In `load()`/`paths()` they need no import (the server
//! module imports them from `brain:content`); on the client they are B0705; `getCollection`/`getEntry` normalize.
use std::path::Path;

const VIEW: &str = "<meta seo title=\"Blog\" description=\"Every post of the blog, newest first.\">
<script>
  export async function load(ctx) {
    return { posts: entries('blog'), tags: tags('blog') }
  }
</script>
<main><h1>Blog</h1>{#each props.posts as p (p.slug)}<a href={`/blog/${p.slug}`}>{p.title}</a>{/each}</main>
";

fn codes(file: &str, src: &str) -> Vec<brain_parse::Diag> {
    let c = brain_parse::parse(src, Path::new(file)).unwrap_or_else(|d| panic!("{d:?}"));
    brain_check::check(&c, &brain_check::Ctx::default())
}

#[test]
fn readers_need_no_import_on_the_server() {
    assert!(codes("views/blog.brain", VIEW).iter().all(|d| d.level != brain_parse::Severity::Error), "{:#?}", codes("views/blog.brain", VIEW));
    let o = brain_codegen::compile(VIEW, Path::new("views/blog.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.server_js.contains("import { entries, tags } from \"brain:content\";"), "{}", o.server_js);
    assert!(!o.js.contains("brain:content"), "client stays clean: {}", o.js);

    // An explicit import (today's scaffold) is not duplicated.
    let explicit = VIEW.replace("<script>\n", "<script>\n  import { entries, tags } from 'brain:content'\n");
    let o = brain_codegen::compile(&explicit, Path::new("views/blog.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert_eq!(o.server_js.matches("brain:content").count(), 1, "{}", o.server_js);
}

#[test]
fn client_reads_are_b0705_and_astro_names_normalize() {
    let d = codes("comp/list.brain", "<script>\n  const posts = entries('blog')\n</script>\n<p>{posts.length}</p>\n");
    let e: Vec<_> = d.iter().filter(|d| d.code == "B0705").collect();
    assert!(e.len() == 1 && e[0].fix.contains("load(ctx)"), "{d:#?}");

    let astro = VIEW.replace("entries('blog')", "getCollection('blog')");
    let d: Vec<_> = codes("views/blog.brain", &astro).into_iter().filter(|d| d.code == "B0706").collect();
    assert!(d.len() == 1 && d[0].edits[0].text == "entries", "{d:#?}");
    let o = brain_codegen::compile(&astro, Path::new("views/blog.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.server_js.contains("posts: entries('blog')") && o.server_js.contains("import { entries, tags } from \"brain:content\";"), "{}", o.server_js);
}
