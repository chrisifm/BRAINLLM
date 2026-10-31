//! §41.6 slot props: the child passes `<slot item={x} />`, the parent reads `{#slot item}…{/slot}` (RUNTIME contract:
//! `$p.children({ get item() { … } })` / `children: ($s) => …`).
use std::path::Path;

#[test]
fn child_passes_getters_and_parent_reads_them() {
    let child = "<meta props=\"items:list\">\n<ul>{#each items as it (it.id)}<li><slot item={it} /></li>{/each}</ul>\n";
    let o = brain_codegen::compile(child, Path::new("comp/list.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("$p.children({ get \"item\"() { return it.v; } })") || o.js.contains("$p.children({ get \"item\"() { return it; } })"), "{}", o.js);

    let root = std::env::temp_dir().join(format!("brain-slot-props-{}", std::process::id()));
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::write(root.join("comp/list.brain"), child).unwrap();
    let parent = "<script>\n  import List from './list.brain'\n  let xs = sig([{ id: 1, name: 'a' }])\n</script>\n<List items={xs}>{#slot item}<p>{item.name}</p>{/slot}</List>\n";
    std::fs::write(root.join("comp/page.brain"), parent).unwrap();
    let o = brain_codegen::compile_with(parent, &root.join("comp/page.brain"), &brain_codegen::Options::default());
    std::fs::remove_dir_all(&root).ok();
    let o = o.unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(o.js.contains("children: ($s) =>"), "{}", o.js);
    assert!(o.js.contains("const item = drv(() => $s?.[\"item\"]);"), "{}", o.js);
    assert!(o.js.contains("item.v.name"), "{}", o.js);
}

#[test]
fn slot_scope_rules() {
    let c = |src: &str| {
        let c = brain_parse::parse(src, Path::new("comp/p.brain")).unwrap_or_else(|d| panic!("{d:?}"));
        brain_check::check(&c, &brain_check::Ctx::default()).into_iter().map(|d| d.code).collect::<Vec<_>>()
    };
    let base = "<script>\n  import List from './list.brain'\n</script>\n";
    // Mixed with other default content.
    assert!(c(&format!("{base}<List><p>x</p>{{#slot item}}<p>{{item}}</p>{{/slot}}</List>\n")).contains(&"B0218"));
    // Slot names are read-only.
    assert!(c(&format!("{base}<List>{{#slot item}}<button on:click={{item = 1}}>{{item}}</button>{{/slot}}</List>\n")).contains(&"B0112"));
    // Names are in scope inside the block only.
    let d = c(&format!("{base}<List>{{#slot item}}<p>{{item}}</p>{{/slot}}</List><p>{{item}}</p>\n"));
    assert_eq!(d.iter().filter(|x| **x == "B0111").count(), 1, "{d:?}");
}

#[test]
fn conditional_named_slot_content_fills_its_named_slot() {
    let root = std::env::temp_dir().join(format!("brain-slot-if-{}", std::process::id()));
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::write(root.join("comp/sheet.brain"), "<section><slot /><footer><slot name=\"footer\" /></footer></section>\n").unwrap();
    let parent = "<script>\n  import Sheet from './sheet.brain'\n  let slug = sig('acme')\n</script>\n<Sheet><p>Body</p>{#if slug}<a slot=\"footer\" href=\"/public\">Public page</a>{:else}<span slot=\"footer\">Draft</span>{/if}</Sheet>\n";
    std::fs::write(root.join("comp/page.brain"), parent).unwrap();
    let o = brain_codegen::compile_with(parent, &root.join("comp/page.brain"), &brain_codegen::Options::default());
    std::fs::remove_dir_all(&root).ok();
    let js = o.unwrap_or_else(|d| panic!("{d:#?}")).js;
    let (default, footer) = (js.find("children: () =>").expect(&js), js.find("\"slot:footer\": () =>").expect(&js));
    assert!(footer < default, "{js}");
    let (named, rest) = (&js[footer..default], &js[default..]);
    // The conditional (both branch templates) lives in slot:footer; the default slot only renders the body.
    assert!(named.contains("$t0()") && named.contains("$t1()") && !rest.contains("$t0()") && rest.contains("$t3()"), "{js}");
    assert!(!js.contains("slot=\\\"footer"), "the slot attribute must not reach the DOM\n{js}");
}
