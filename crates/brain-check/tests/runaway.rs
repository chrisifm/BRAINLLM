//! #92 (INTERNALS §30): B0905 unbounded growth and B0906 timers/listeners/subscriptions without cleanup.

use brain_check::Ctx;
use std::path::Path;

fn codes(src: &str) -> Vec<&'static str> {
    let c = brain_parse::parse(src, Path::new("comp/x.brain")).unwrap_or_else(|d| panic!("{d:?}"));
    let d = brain_check::check(&c, &Ctx::default());
    for x in &d {
        assert!(!x.fix.is_empty() && !x.msg.is_empty(), "{x}");
    }
    d.into_iter().map(|d| d.code).collect()
}

/// Only the §30 codes: the snippets use `window`/`bus` loosely (B0201/B0806 are other rules' business).
fn with(script: &str) -> Vec<&'static str> {
    codes(&format!("<script>\n{script}\n</script>\n<p>{{items.length}}</p>")).into_iter().filter(|c| matches!(*c, "B0905" | "B0906")).collect()
}

#[test]
fn b0906_resources_outside_fx_or_without_cleanup() {
    assert_eq!(with("let items = sig([])\nsetInterval(() => log(1), 1000)"), ["B0906"]);
    assert_eq!(with("let items = sig([])\nfx(() => { setInterval(() => log(1), 1000) })"), ["B0906"]);
    assert_eq!(with("let items = sig([])\nfx(() => { window.addEventListener('resize', () => log(1)) })"), ["B0906"]);
    assert_eq!(with("let items = sig([])\nconst start = () => setInterval(() => log(1), 1000)"), ["B0906"]);
    assert_eq!(with("let items = sig([])\nfx(() => source.subscribe((v) => log(v)))").len(), 0);
    assert_eq!(with("let items = sig([])\nsetTimeout(() => log(1), 10)"), ["B0906"]);
}

#[test]
fn allowed_patterns() {
    // Cleanup returned from fx.
    assert!(with("let items = sig([])\nfx(() => { const id = setInterval(() => log(1), 1000); return () => clearInterval(id) })").is_empty());
    // One-shot timer in an event handler.
    assert!(with("let items = sig([])\nconst flash = () => setTimeout(() => log(1), 300)").is_empty());
    // A listener removed through an AbortSignal.
    assert!(with("let items = sig([])\nfx(() => { const c = new AbortController(); window.addEventListener('resize', () => log(1), { signal: c.signal }); return () => c.abort() })").is_empty());
    // bus().on unsubscribes with the component.
    assert!(with("let items = sig([])\nconst t = bus('toast')\nt.on((m) => log(m))").is_empty());
    // Appends from a user event are not runaway growth.
    assert!(codes("<script>\nlet items = sig([])\nconst add = () => { items = [...items, 1] }\n</script>\n<button on:click={add}>{items.length}</button>").is_empty());
}

#[test]
fn b0905_unbounded_append() {
    let tick = "let items = sig([])\nfx(() => { const id = setInterval(() => { items = [...items, Date.now()] }, 1000); return () => clearInterval(id) })";
    assert_eq!(with(tick), ["B0905"]);
    let named = "let items = sig([])\nconst push = () => { items = items.concat([1]) }\nfx(() => { const id = setInterval(push, 1000); return () => clearInterval(id) })";
    assert_eq!(with(named), ["B0905"]);
    let bus = "let items = sig([])\nconst t = bus('log')\nt.on((m) => { items = [...items, m] })";
    assert_eq!(with(bus), ["B0905"]);
    // Bounded: slice or a length check.
    assert!(with("let items = sig([])\nfx(() => { const id = setInterval(() => { items = [...items, 1].slice(-50) }, 1000); return () => clearInterval(id) })").is_empty());
    assert!(with("let items = sig([])\nfx(() => { const id = setInterval(() => { if (items.length < 50) items = [...items, 1] }, 1000); return () => clearInterval(id) })").is_empty());
}
