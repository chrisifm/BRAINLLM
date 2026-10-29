//! One test per checker rule outside the QA fixtures (B01xx, B02xx, B0302).

use brain_check::{Ctx, Imported};
use brain_parse::ast::{Prop, PropType};
use std::path::Path;

fn codes_with(path: &str, src: &str, ctx: &Ctx) -> Vec<&'static str> {
    let c = brain_parse::parse(src, Path::new(path)).unwrap_or_else(|d| panic!("{:?}", d));
    let d = brain_check::check(&c, ctx);
    for x in &d {
        assert!(!x.fix.is_empty(), "empty fix: {x}");
    }
    // B10xx (overengineering warnings) have their own tests; these snippets leave helpers unused on purpose.
    d.into_iter().map(|d| d.code).filter(|c| !c.starts_with("B10")).collect()
}

fn codes(src: &str) -> Vec<&'static str> {
    codes_with("comp/x.brain", src, &Ctx::default())
}

#[test]
fn script_rules() {
    assert_eq!(codes("<script>let n = sig(0)\nconst f = () => this.n</script><p>x</p>"), ["B0102"]);
    assert_eq!(codes("<script>var n = 1</script>{n}"), ["B0103"]);
    assert_eq!(codes("<script>let count = sig(0)</script><p>{cuont}</p>"), ["B0111"]);
    assert_eq!(codes("<script>const r = eval('1')</script>{r}"), ["B0105"]);
    // `fetch` is a known global; in client code it only gets the B1107 nudge towards `api()`.
    assert_eq!(codes("<script>const t = Math.max(1, 2); fetch('/x'); new IntersectionObserver(() => {})</script>{t}"), ["B1107"]);
}

#[test]
fn reactivity_rules() {
    assert_eq!(codes("<script>let n = sig(0)\nlet d = drv(() => n * 2)\nconst f = () => { d = 3 }</script><p>x</p>"), ["B0112"]);
    assert_eq!(codes("<meta props=\"a:num=0\"><script>const f = () => { a = 3 }</script><p>x</p>"), ["B0112"]);
    assert_eq!(codes("<script>let xs = sig([])\nconst add = (x) => xs.push(x)</script><p>x</p>"), ["B0113"]);
    assert_eq!(codes("<script>let u = sig({})\nconst f = () => { u.name = 'a' }</script><p>x</p>"), ["B0113"]);
    assert_eq!(codes("<script>let xs = sig([])</script>{#each xs as it (it.id)}<button on:click={it.done = true}>x</button>{/each}"), ["B0114"]);
    assert_eq!(codes("<script>const f = () => { let n = sig(0) }</script><p>x</p>"), ["B0115"]);
    assert_eq!(codes_with("ui/x.brain", "<script>let n = sig(0)</script>{n}", &Ctx::default()), ["B0116"]);
    assert_eq!(codes("<script>export async function load(ctx) { return {} }</script><p>x</p>"), ["B0117"]);
    assert_eq!(codes("<script>const k = 'x'\nconst f = () => emit(k)</script><p>x</p>"), ["B0118"]);
    // Fine: reassigning sigs, immutable updates, writes inside handlers.
    assert!(codes("<script>let xs = sig([])\nlet n = sig(0)\nconst add = (x) => { xs = [...xs, x]; n++ }</script><button on:click={add(1)}>+</button>").is_empty());
}

#[test]
fn markup_rules() {
    assert_eq!(codes("<script>let xs = sig([])</script><ul>{#each xs as x}<li>{x}</li>{/each}</ul>"), ["B0202"]);
    assert_eq!(codes("<Card />"), ["B0203"]);
    assert_eq!(codes("<script>let d = drv(() => 1)</script><input aria-label=\"x\" bind:value={d} />"), ["B0204"]);
    assert_eq!(codes("<script>let s = sig('')</script><input aria-label=\"x\" bind:text={s} />"), ["B0205"]);
    assert_eq!(codes("<script>let n = sig(0)</script><button on:click>{n}</button>"), ["B0208"]);
    assert_eq!(codes("<p title=\"a\" title=\"b\">x</p>"), ["B0209"]);

    let mut ctx = Ctx::default();
    let prop = |name: &str, optional: bool| Prop { name: name.into(), ty: PropType::Str, optional, default: None, bindable: false, span: Default::default() };
    ctx.imports.insert("./card.brain".into(), Imported::Brain(vec![prop("title", false), prop("sub", true)]));
    let src = "<script>import Card from './card.brain'</script>";
    assert_eq!(codes_with("comp/x.brain", &format!("{src}<Card title=\"a\" titel=\"b\" />"), &ctx), ["B0206"]);
    assert_eq!(codes_with("comp/x.brain", &format!("{src}<Card sub=\"a\" />"), &ctx), ["B0207"]);
    ctx.imports.insert("./card.brain".into(), Imported::Missing);
    assert_eq!(codes_with("comp/x.brain", &format!("{src}<Card />"), &ctx), ["B0119"]);
}

#[test]
fn css_tokens_and_mixins() {
    let ctx = Ctx { tokens: Some(vec![("--space-2".into(), "8px".into())]), ..Ctx::default() };
    assert_eq!(codes_with("comp/x.brain", "<p>x</p><style>p { padding: var(--space-3) }</style>", &ctx), ["B0302"]);
    assert!(codes_with("comp/x.brain", "<p>x</p><style>p { --mine: 1px; padding: var(--space-2) var(--mine) }</style>", &ctx).is_empty());
    assert_eq!(codes("<p>x</p><style>p { @include nope; }</style>"), ["B0303"]);
}

#[test]
fn diag_json_is_spec_shape() {
    let c = brain_parse::parse("<script>const f = () => this.n</script><p>x</p>", Path::new("comp/counter.brain")).unwrap();
    let all = brain_check::check(&c, &Ctx::default());
    let d = all.iter().find(|d| d.code == "B0102").unwrap();
    assert!(d.to_json().starts_with(r#"{"code":"B0102","file":"comp/counter.brain","line":1,"col":25,"msg":"#), "{}", d.to_json());
}

#[test]
fn contrast_resolves_color_mix_and_rejects_unverifiable() {
    let ctx = Ctx { tokens: Some(vec![("--fg".into(), "#1a1a1a".into()), ("--accent".into(), "#0071e3".into()), ("--on".into(), "#ffffff".into())]), ..Ctx::default() };
    let c = |css: &str| codes_with("comp/x.brain", &format!("<p>x</p><style>p {{ {css} }}</style>"), &ctx);
    assert_eq!(c("background: var(--accent); color: color-mix(in srgb, var(--fg), #000 40%)"), ["B0408"]);
    assert_eq!(c("background: var(--accent); color: currentColor"), ["B0408"]);
    assert!(c("background: var(--accent); color: color-mix(in srgb, var(--on) 60%, #fff)").is_empty());
    assert!(c("background: var(--accent); color: var(--on)").is_empty());
    let src = "<p>x</p><style>p { background: var(--accent); color: color-mix(in srgb, var(--fg), #000 40%) }</style>";
    let comp = brain_parse::parse(src, Path::new("comp/x.brain")).unwrap();
    let d = &brain_check::check(&comp, &ctx)[0];
    assert!(d.msg.contains("(#101010)") && d.msg.contains("4.05:1"), "{}", d.msg);
}

#[test]
fn each_key_fix_depends_on_item_shape() {
    let fix = |src: &str| {
        let c = brain_parse::parse(src, Path::new("comp/x.brain")).unwrap();
        brain_check::check(&c, &Ctx::default()).remove(0).fix
    };
    assert!(fix("<script>let perks = sig(['a'])</script><ul>{#each perks as perk}<li>{perk}</li>{/each}</ul>").contains("key by value: `{#each perks as perk (perk)}`"));
    assert!(fix("<script>let us = sig([])</script><ul>{#each us as u}<li>{u.name}</li>{/each}</ul>").contains("(u.id)"));
}

#[test]
fn scrim_token_for_old_apps_and_synonyms() {
    // #164: `--scrim` is new in the scaffold; an older tokens file gets told what to add, never another token.
    let old = Ctx { tokens: Some(vec![("--fg".into(), "#1a1a1a".into()), ("--bg".into(), "#f5f5f7".into())]), ..Ctx::default() };
    let c = brain_parse::parse("<div class=\"v\">x</div><style>.v { background: var(--scrim); }</style>", Path::new("comp/x.brain")).unwrap();
    let d = brain_check::check(&c, &old);
    let b = d.iter().find(|d| d.code == "B0302").expect("B0302");
    assert!(b.edits.is_empty() && b.fix.contains("--scrim: rgb(15 23 42 / .45)"), "{b:?}");
    // With the token, a synonym normalizes to it.
    let new = Ctx { tokens: Some(vec![("--scrim".into(), "rgb(15 23 42 / .45)".into())]), ..Ctx::default() };
    let c = brain_parse::parse("<div class=\"v\">x</div><style>.v { background: var(--overlay); }</style>", Path::new("comp/x.brain")).unwrap();
    let d = brain_check::check(&c, &new);
    let b = d.iter().find(|d| d.code == "B0302").expect("B0302");
    assert!(b.is_info() && b.edits[0].text == "--scrim", "{b:?}");
}

#[test]
fn dead_scoped_css() {
    let b0307 = |css: &str| codes(&format!("<div class=\"a\"><p>x</p></div><style>{css}</style>")).into_iter().filter(|c| *c == "B0307").count();
    // Out of reach of `@scope ([data-b]) to ([data-b])`.
    assert_eq!(b0307("main { margin: 0 }"), 1);
    assert_eq!(b0307(".a body { margin: 0 }"), 1);
    assert_eq!(b0307("@media (min-width: 40em) { body { margin: 0 } }"), 1);
    assert_eq!(b0307(":where(html) p { margin: 0 }"), 1);
    assert_eq!(b0307(":scope ~ p { margin: 0 }"), 1);
    assert_eq!(b0307(".a { & span { margin: 0 } }"), 1);
    // Leading `body` at the top level is B0301 only.
    assert_eq!(codes("<p>x</p><style>body { margin: 0 }</style>"), ["B0301"]);
    // Live selectors.
    assert_eq!(b0307("div.a > p:not(main):nth-child(2n+1), :scope > p, [data-x=\"body\"] p { margin: 0 }"), 0);
    assert_eq!(b0307(":where(.a) p:is(p, div) { margin: 0 }"), 0);
    assert_eq!(b0307("@keyframes f { from { opacity: 0 } to { opacity: 1 } }"), 0);
    // `{@html}` writes unknown tags: no absent-tag warning.
    assert!(!codes("<script>import { sanitize } from 'brain'\nconst h = '<b>x</b>'</script><div>{@html sanitize(h)}</div><style>b { margin: 0 }</style>").contains(&"B0307"));
}

#[test]
fn navigation_through_location() {
    let diags = |src: &str| {
        let c = brain_parse::parse(src, Path::new("views/login.brain")).unwrap();
        brain_check::check(&c, &Ctx::default())
    };
    let d = diags("<script>\n  const go = () => location.assign('/account')\n</script>\n<button on:click={go}>Go</button>\n");
    let b: Vec<_> = d.iter().filter(|d| d.code == "B0129").collect();
    assert_eq!(b.len(), 1, "{d:?}");
    assert_eq!(b[0].edits[0].text, "navigate('/account')");
    let d = diags("<script>\n  const go = () => { window.location.href = `/post/${1}` }\n  const back = () => location.replace('/')\n</script>\n<button on:click={go}>Go</button><button on:click={back}>Back</button>\n");
    let t: Vec<&str> = d.iter().filter(|d| d.code == "B0129").map(|d| d.edits[0].text.as_str()).collect();
    assert_eq!(t, ["navigate(`/post/${1}`)", "navigate('/', { replace: true })"]);
    // External URLs and protocol-relative ones are fine; navigate/route need no import.
    let d = diags("<script>\n  const go = () => location.assign('https://x.dev')\n  const r = route()\n  const n = () => navigate('/account')\n</script>\n<button on:click={go}>{r.params.id}</button><button on:click={n}>A</button>\n");
    assert!(d.iter().all(|d| d.code == "B0501"), "{d:?}");
}

#[test]
fn replacing_form_fields() {
    let diags = |src: &str| {
        let c = brain_parse::parse(src, Path::new("comp/job-form.brain")).unwrap();
        brain_check::check(&c, &Ctx::default())
    };
    let d = diags("<script>\n  let f = form({ title: 'str!' }, { submit: async (v) => v })\n  mounted(() => { f.v = { title: 'Draft' } })\n</script>\n<form on:submit={f.submit}><input aria-label=\"Title\" bind:value={f.v.title}><button>Save</button></form>\n");
    let b: Vec<_> = d.iter().filter(|d| d.code == "B0133").collect();
    assert_eq!(b.len(), 1, "{d:?}");
    assert_eq!(b[0].edits[0].text, "f.reset({ title: 'Draft' })");
    let d = diags("<script>\n  let f = form({ title: 'str!' }, { submit: async (v) => v })\n  const clear = () => { f.err = {} }\n</script>\n<form on:submit={f.submit}><input aria-label=\"Title\" bind:value={f.v.title}><button on:click={clear}>Save</button></form>\n");
    let b: Vec<_> = d.iter().filter(|d| d.code == "B0133").collect();
    assert!(b.len() == 1 && b[0].edits.is_empty(), "{d:?}");
    // writing one field, f.reset() and an unrelated `x.v = …` are fine
    let d = diags("<script>\n  let f = form({ title: 'str!' }, { submit: async (v) => v })\n  let box = { v: 1 }\n  mounted(() => { f.v.title = 'Draft'; f.reset({ title: 'Draft' }); box.v = 2 })\n</script>\n<form on:submit={f.submit}><input aria-label=\"Title\" bind:value={f.v.title}><button>Save</button></form>\n");
    assert!(!d.iter().any(|d| d.code == "B0133"), "{d:?}");
}

fn a11y(src: &str) -> Vec<&'static str> {
    codes(src).into_iter().filter(|c| c.starts_with("B041")).collect()
}

#[test]
fn link_without_accessible_name() {
    assert_eq!(a11y("<a href=\"/cart\"></a>"), ["B0411"]);
    assert_eq!(a11y("<a href=\"/cart\">   </a>"), ["B0411"]);
    assert_eq!(a11y("<a href=\"/cart\"><span></span></a>"), ["B0411"]);
    assert_eq!(a11y("<a href=\"/cart\"><img src=\"/cart.svg\" alt=\"\"></a>"), ["B0411"]);
    // an icon link with aria-label, link text, an expression or an image with alt all have a name
    assert!(a11y("<a href=\"/cart\" aria-label=\"Cart\"><span class=\"i\"></span></a>").is_empty());
    assert!(a11y("<a href=\"/cart\" aria-labelledby=\"cart-label\"></a>").is_empty());
    assert!(a11y("<a href=\"/cart\">Cart</a>").is_empty());
    assert!(a11y("<script>let n = sig('Cart')</script><a href=\"/cart\">{n}</a>").is_empty());
    assert!(a11y("<a href=\"/\"><img src=\"/acme.svg\" alt=\"Acme home\"></a>").is_empty());
    // an anchor without href is not a link
    assert!(a11y("<a id=\"top\"></a>").is_empty());
}

#[test]
fn generic_link_text() {
    for t in ["click here", "Here", "Read more", "read more…", "More", "link", "Learn more →", "  learn   more. "] {
        assert_eq!(a11y(&format!("<a href=\"/pricing\">{t}</a>")), ["B0412"], "{t}");
    }
    assert_eq!(a11y("<a href=\"/pricing\"><strong>Read more</strong></a>"), ["B0412"]);
    // aria-label overrides the visible text; a descriptive or dynamic text is fine
    assert!(a11y("<a href=\"/pricing\" aria-label=\"Read more about pricing\">Read more</a>").is_empty());
    assert!(a11y("<a href=\"/pricing\">Read the pricing guide</a>").is_empty());
    assert!(a11y("<a href=\"/pricing\">More plans</a>").is_empty());
    assert!(a11y("<script>let t = sig('x')</script><a href=\"/pricing\">Read more {t}</a>").is_empty());
}

#[test]
fn alt_text_that_describes_nothing() {
    for alt in ["hero.png", "IMG_2041.JPG", "photo of team.jpeg", "banner.webp", "logo.svg", "shot.avif", "a.gif", "image", "Photo", "picture", "logo", "icon", "img"] {
        assert_eq!(a11y(&format!("<img src=\"/x.png\" alt=\"{alt}\">")), ["B0413"], "{alt}");
    }
    assert!(a11y("<img src=\"/x.png\" alt=\"\">").is_empty());
    assert!(a11y("<img src=\"/x.png\" alt=\"Acme logo\">").is_empty());
    assert!(a11y("<img src=\"/x.png\" alt=\"The team at the offsite\">").is_empty());
    assert!(a11y("<img src=\"/x.png\" alt=\".png files explained\">").is_empty());
    assert!(brain_check::poor_alt("hero.png") && !brain_check::poor_alt("") && !brain_check::poor_alt("Hero banner"));
}

#[test]
fn iframe_without_title() {
    assert_eq!(a11y("<iframe src=\"https://maps.example.com/embed\"></iframe>"), ["B0414"]);
    assert!(a11y("<iframe src=\"https://maps.example.com/embed\" title=\"Map of our office\"></iframe>").is_empty());
    assert!(a11y("<iframe src=\"https://maps.example.com/embed\" aria-label=\"Map of our office\"></iframe>").is_empty());
}

#[test]
fn video_without_captions() {
    assert_eq!(a11y("<video controls src=\"/demo.mp4\"></video>"), ["B0415"]);
    assert_eq!(a11y("<video src=\"/demo.mp4\" autoplay></video>"), ["B0415"]);
    assert_eq!(a11y("<video controls muted src=\"/demo.mp4\"></video>"), ["B0415"]);
    assert_eq!(a11y("<video controls src=\"/demo.mp4\"><track kind=\"chapters\" src=\"/c.vtt\"></video>"), ["B0415"]);
    assert!(a11y("<video controls src=\"/demo.mp4\"><track kind=\"captions\" src=\"/demo.vtt\" srclang=\"en\" label=\"English\"></video>").is_empty());
    assert!(a11y("<video controls src=\"/demo.mp4\"><track kind=\"subtitles\" src=\"/demo.vtt\" srclang=\"en\" label=\"English\"></video>").is_empty());
    // a muted autoplay loop without controls is decorative
    assert!(a11y("<video autoplay muted loop playsinline src=\"/bg.mp4\"></video>").is_empty());
}
