use brain_check::{ApiContract, Ctx, MethodContract};
#[path = "../src/performance_advice.rs"]
mod performance_advice;
use brain_parse::{Diag, Severity};
use std::path::Path;
fn check(source: &str, ctx: &Ctx) -> Vec<Diag> {
    let component = brain_parse::parse(source, Path::new("comp/performance.brain"))
        .unwrap_or_else(|e| panic!("{e:?}"));
    let result = performance_advice::check(&component, ctx);
    assert!(result.iter().all(|d| d.level == Severity::Warning
        && !d.msg.is_empty()
        && !d.fix.is_empty()
        && d.edits.is_empty()));
    result
}
fn has(source: &str, code: &str) -> bool {
    check(source, &Ctx::default())
        .iter()
        .any(|d| d.code == code)
}

#[test]
fn serial_await_is_conditional_advice_and_known_loop_carried_dependencies_are_skipped() {
    assert!(has(
        "<script>const collect = async (items) => { for (const item of items) { const result = await read(item) } }</script><p>Fixture</p>",
        "B1301"
    ));
    for source in [
        "<script>const collect = async (items) => { let cursor = 0; for (const item of items) { cursor = await read(cursor) } }</script><p>Fixture</p>",
        "<script>const collect = async (items) => { const state = {}; for (const item of items) { state.cursor = await read(state.cursor) } }</script><p>Fixture</p>",
        "<script>const collect = async (items) => { for (const item of items) { schedule(async () => await read(item)) } }</script><p>Fixture</p>",
    ] {
        assert!(!has(source, "B1301"), "{source}");
    }
}

#[test]
fn same_collection_nested_loops_are_structural_and_mutation_shadowing_aliases_exits_are_skipped() {
    assert!(has(
        "<script>const rows = [1,2]; for (const first of rows) { for (const second of rows) { log(first,second) } }</script><p>Fixture</p>",
        "B1303"
    ));
    for source in [
        "<script>const rows = [1,2]; const other = [3,4]; for (const first of rows) { for (const second of other) { log(first,second) } }</script><p>Fixture</p>",
        "<script>const rows = [1,2]; for (const first of rows) { rows.pop(); for (const second of rows) { log(first,second) } }</script><p>Fixture</p>",
        "<script>const rows = [1,2]; const alias = rows; alias.pop(); for (const first of rows) { for (const second of rows) { log(first,second) } }</script><p>Fixture</p>",
        "<script>const rows = [1,2]; for (const first of rows) { const rows = [3]; for (const second of rows) { log(first,second) } }</script><p>Fixture</p>",
        "<script>const rows = [1,2]; for (const first of rows) { for (const second of rows) { break } }</script><p>Fixture</p>",
    ] {
        assert!(!has(source, "B1303"), "{source}");
    }
}

#[test]
fn only_proven_array_traversals_independent_of_each_bindings_get_render_advice() {
    assert!(has(
        "<script>const shared = [1,2]; const rows = [1,2]</script>{#each rows as row (row)}<p>{row}: {shared.filter(x => x > 1).length}</p>{/each}",
        "B1305"
    ));
    for source in [
        "<script>const shared = [1,2]; const rows = [1,2]</script>{#each rows as row (row)}<p>{shared.filter(x => x > row).length}</p>{/each}",
        "<script>const shared = props.items; const rows = [1,2]</script>{#each rows as row (row)}<p>{shared.filter(x => x > 1).length}</p>{/each}",
        "<script>const make=() => { const shared = [1,2] }; const rows=[1,2]</script>{#each rows as row (row)}<p>{shared.filter(x => x > 1).length}</p>{/each}",
        "<script>const shared = [1,2]; shared.push(3); const rows = [1,2]</script>{#each rows as row (row)}<p>{shared.filter(x => x > 1).length}</p>{/each}",
        "<script>const shared = [1,2]; const alias=shared; const rows = [1,2]</script>{#each rows as row (row)}<p>{shared.filter(x => x > 1).length}</p>{/each}",
        "<script>const shared = [1,2]; const rows = [1,2]</script>{#each rows as row (row)}<p>{shared.filter(x => effect(x)).length}</p>{/each}",
        "<script>const shared = [1,2]; const rows = [1,2]</script>{#each rows as row (row)}<button type=\"button\" on:click={() => shared.filter(x => x > 1)}>Check</button>{/each}",
    ] {
        assert!(!has(source, "B1305"), "{source}");
    }
}

fn api_context() -> Ctx {
    let mut ctx = Ctx::default();
    let mut contract = ApiContract::default();
    contract
        .methods
        .insert("get".into(), MethodContract::default());
    ctx.api_contracts
        .insert("../api/items.jsbrain".into(), contract);
    ctx
}
#[test]
fn per_item_read_requires_real_declared_get_contract_and_direct_unshadowed_receiver() {
    let source = "<script>import items from '../api/items.jsbrain'\nconst collect = async (ids) => { for (const id of ids) { await items.get(id) } }</script><p>Fixture</p>";
    assert!(
        check(source, &api_context())
            .iter()
            .any(|d| d.code == "B1312")
    );
    assert!(
        !check(source, &Ctx::default())
            .iter()
            .any(|d| d.code == "B1312")
    );
    for body in [
        "const collect = async (ids, items) => { for (const id of ids) { await items.get(id) } }",
        "const alias=items; const collect = async (ids) => { for (const id of ids) { await alias.get(id) } }",
        "items = other; const collect = async (ids) => { for (const id of ids) { await items.get(id) } }",
        "items.get = other; const collect = async (ids) => { for (const id of ids) { await items.get(id) } }",
        "const alias = items; alias.get = other; const collect = async (ids) => { for (const id of ids) { await items.get(id) } }",
        "const collect = async (ids) => { for (const id of ids) { await items.get('fixed') } }",
        "const collect = async (ids) => { for (const id of ids) { await items.post(id) } }",
    ] {
        let source = format!(
            "<script>import items from '../api/items.jsbrain'\n{body}</script><p>Fixture</p>"
        );
        assert!(
            !check(&source, &api_context())
                .iter()
                .any(|d| d.code == "B1312"),
            "{source}"
        );
    }
}

#[test]
fn potential_layout_thrashing_requires_proven_dom_origin_and_straight_line_write_then_read() {
    assert!(has(
        "<script>let panel=ref(); const rows=[1,2]; mounted(() => { for (const row of rows) { panel.style.width=row+'px'; const width=panel.offsetWidth } })</script><div bind:ref={panel}>Fixture</div>",
        "B1313"
    ));
    assert!(has(
        "<script>const panel=document.querySelector('#panel'); const rows=[1,2]; for (const row of rows) { panel.style.width = row + 'px'; const width = panel.offsetWidth }</script><p>Fixture</p>",
        "B1313"
    ));
    for source in [
        "<script>import ref from './mock.jsbrain'\nlet panel=ref(); const rows=[1,2]; mounted(() => { for (const row of rows) { panel.style.width=row+'px'; const width=panel.offsetWidth } })</script><div bind:ref={panel}>Fixture</div>",
        "<script>let panel=ref(); const rows=[1,2]; mounted(() => { for (const row of rows) { panel.style.width=row+'px'; const width=panel.offsetWidth } })</script><Panel bind:ref={panel} />",
        "<script>const panel=props.panel; const rows=[1,2]; for (const row of rows) { panel.style.width = row + 'px'; const width = panel.offsetWidth }</script><p>Fixture</p>",
        "<script>import document from './mock.jsbrain'\nconst panel=document.querySelector('#panel'); const rows=[1,2]; for (const row of rows) { panel.style.width = row + 'px'; const width = panel.offsetWidth }</script><p>Fixture</p>",
        "<script>document.querySelector = custom; const panel=document.querySelector('#panel'); const rows=[1,2]; for (const row of rows) { panel.style.width = row + 'px'; const width = panel.offsetWidth }</script><p>Fixture</p>",
        "<script>const panel=document.querySelector('#panel'); const alias=panel; const rows=[1,2]; for (const row of rows) { alias.style.width = row + 'px'; const width = alias.offsetWidth }</script><p>Fixture</p>",
        "<script>const panel=document.querySelector('#panel'); const alias=panel; const rows=[1,2]; for (const row of rows) { panel.style.width = row + 'px'; const width = panel.offsetWidth }</script><p>Fixture</p>",
        "<script>const panel=document.querySelector('#panel'); const rows=[1,2]; for (const row of rows) { const width = panel.offsetWidth; panel.style.width = row + 'px' }</script><p>Fixture</p>",
        "<script>const collect = (document) => { const panel=document.querySelector('#panel'); const rows=[1,2]; for (const row of rows) { panel.style.width = row + 'px'; const width = panel.offsetWidth } }</script><p>Fixture</p>",
        "<script>const panel=document.querySelector('#panel'); const rows=[1,2]; const update = async () => { for (const row of rows) { panel.style.width = row + 'px'; await tick(); const width = panel.offsetWidth } }</script><p>Fixture</p>",
        "<script>const panel=document.querySelector('#panel'); const rows=[1,2]; for (const row of rows) { panel.style.width = row + 'px'; unknownEffect(); const width = panel.offsetWidth }</script><p>Fixture</p>",
    ] {
        assert!(!has(source, "B1313"), "{source}");
    }
}

#[test]
fn wrapped_value_escapes_invalidate_api_array_and_dom_provenance() {
    for wrapper in [
        "items",
        "({ api: items })",
        "[{ nested: [items] }]",
        "(ready ? {api:items} : {})",
        "(0, {api:items})",
    ] {
        let source = format!(
            "<script>import items from '../api/items.jsbrain'\nconfigure({wrapper}); const collect=async(ids)=>{{for(const id of ids){{await items.get(id)}}}}</script><p>Fixture</p>"
        );
        assert!(
            !check(&source, &api_context())
                .iter()
                .any(|d| d.code == "B1312"),
            "{source}"
        );
    }
    for binding in [
        "const box={rows}; box.rows.pop()",
        "const box=[{rows}]; box[0].rows.pop()",
        "holder={rows}",
        "const give=()=>rows; const box=give();box.pop()",
        "const give=()=>{return {rows}};const box=give();box.rows.pop()",
        "const give=(box={rows})=>box; const box=give();box.rows.pop()",
        "for(const box of [{rows}]){box.rows.pop()}",
    ] {
        let source = format!(
            "<script>const rows=[1,2];{binding};for(const first of rows){{for(const second of rows){{log(first,second)}}}}</script><p>Fixture</p>"
        );
        assert!(!has(&source, "B1303"), "{source}");
    }
    for binding in [
        "configure({element:panel})",
        "const box=[{element:panel}]",
        "const give=()=>({element:panel});const box=give()",
        "panel.length=panel;configure({element:panel.length})",
        "panel.getBoundingClientRect=custom",
    ] {
        let source = format!(
            "<script>let panel=ref();const rows=[1,2];{binding};mounted(()=>{{for(const row of rows){{panel.style.width=row+'px';const width=panel.offsetWidth}}}})</script><div bind:ref={{panel}}>Fixture</div>"
        );
        assert!(!has(&source, "B1313"), "{source}");
    }
    for binding in [
        "const alias=document;alias.querySelector=custom",
        "const box={document};box.document.querySelector=custom",
        "configure([{document}])",
        "const alias=window.document;alias.querySelector=custom",
        "const {document:alias}=globalThis;alias.querySelector=custom",
    ] {
        let source = format!(
            "<script>{binding};const panel=document.querySelector('#panel');const rows=[1,2];for(const row of rows){{panel.style.width=row+'px';const width=panel.offsetWidth}}</script><p>Fixture</p>"
        );
        assert!(!has(&source, "B1313"), "{source}");
    }
    assert!(!has(
        "<script>const alias={ref};let panel=ref();const rows=[1,2];mounted(()=>{for(const row of rows){panel.style.width=row+'px';const width=panel.offsetWidth}})</script><div bind:ref={panel}>Fixture</div>",
        "B1313"
    ));
}

#[test]
fn all_function_like_bindings_and_default_expression_mutations_are_visited() {
    for definition in [
        "const loader={async collect(items){for(const id of ['a']){await items.get(id)}}}",
        "const loader={async collect({items}){for(const id of ['a']){await items.get(id)}}}",
        "const loader={async collect(value=async(items)=>{for(const id of ['a']){await items.get(id)}}){return value}}",
    ] {
        let source = format!(
            "<script>import items from '../api/items.jsbrain'\n{definition}</script><p>Fixture</p>"
        );
        assert!(
            !check(&source, &api_context())
                .iter()
                .any(|d| d.code == "B1312"),
            "{source}"
        );
    }
    assert!(!has(
        "<script>const loader={collect(document){const panel=document.querySelector('#panel');const rows=[1,2];for(const row of rows){panel.style.width=row+'px';const width=panel.offsetWidth}}}</script><p>Fixture</p>",
        "B1313"
    ));
    for mutation in [
        "const take=(value=rows.pop())=>value;take()",
        "const {value=rows.pop()}={}",
        "const take=({value=rows.pop()}={})=>value;take()",
        "const box={ [change(rows)]: 1 }",
    ] {
        let source = format!(
            "<script>const rows=[1,2];{mutation};for(const first of rows){{for(const second of rows){{log(first,second)}}}}</script><p>Fixture</p>"
        );
        assert!(!has(&source, "B1303"), "{source}");
    }
}

#[test]
fn shared_identifier_reads_include_object_shorthand_and_computed_keys() {
    assert!(!has(
        "<script>const collect=async(items)=>{let cursor=0;for(const item of items){cursor=await read({cursor})}}</script><p>Fixture</p>",
        "B1301"
    ));
    for value in [
        "shared.map(value=>({row,value}))[0].row",
        "shared.map(value=>({[row]:value}))[0]",
    ] {
        let source = format!(
            "<script>const shared=[1,2];const rows=[1,2]</script>{{#each rows as row (row)}}<p>{{{value}}}</p>{{/each}}"
        );
        assert!(!has(&source, "B1305"), "{source}");
    }
}

#[test]
fn statements_with_ordering_barriers_cannot_resurrect_earlier_nested_writes() {
    for statement in [
        "consume(panel.style.width=row+'px')",
        "const value=(panel.style.width=row+'px',consume())",
        "const value=ready?(panel.style.width=row+'px'):consume()",
        "const value=ready&&(panel.style.width=row+'px')",
        "const {value=(panel.style.width=row+'px')}=source",
    ] {
        let source = format!(
            "<script>let panel=ref();const rows=[1,2];mounted(()=>{{for(const row of rows){{{statement};const width=panel.offsetWidth}}}})</script><div bind:ref={{panel}}>Fixture</div>"
        );
        assert!(!has(&source, "B1313"), "{source}");
    }
}
