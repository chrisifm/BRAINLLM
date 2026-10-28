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
