use std::path::Path;
fn output(script: &str) -> brain_codegen::Output {
    let source = format!(
        "<meta seo title=\"Static forms\" description=\"Static form compilation and dynamic fallback contract.\" canonical=\"/example\"><script>{script}</script><main><p>Example</p></main>"
    );
    brain_codegen::compile(&source, Path::new("views/example.brain")).unwrap()
}
#[test]
fn flat_literal_schemas_use_exact_tracked_metadata_and_keep_server_submit_identity() {
    let result = output(
        "server: async function save(ctx,v) { return v }; let f = form({email:'email!',password:'str!',nickname:'str'}, {submit: async v => await save(v)})",
    );
    assert!(result.js.contains("formStaticSync as"));
    assert!(result.js.contains("\"email\":[\"email\",true]"));
    assert!(result.js.contains("\"password\":[\"str\",true]"));
    assert!(result.js.contains("\"nickname\":[\"str\",false]"));
    assert!(result.js.contains("o.submit.id = save.id"));
    assert!(result.server_js.contains("email!"));
    assert!(result.js.contains("budgetOwner"));
}
#[test]
fn dynamic_ambiguous_bounded_nested_and_imported_schemas_keep_original_parser() {
    for script in [
        "let rules={email:'email!'};let f=form(rules)",
        "let f=form({name:'str!<=80'})",
        "let f=form({nested:{name:'str!'}})",
        "let rules={};let f=form({...rules,email:'email!'})",
        "let f=form({email:'str',email:'email!'})",
        "let f=form({['email']:'email!'})",
        "import {form} from 'custom-package';let f=form({email:'email!'})",
    ] {
        assert!(
            !output(script).js.contains("formStaticSync as"),
            "unexpected normalization for {script}"
        );
    }
}
#[test]
fn unsupported_static_schema_preserves_submit_metadata_and_mixed_form_calls() {
    let result = output(
        "server: async function save(ctx,v) { return v };let a=form({name:'str!<=80'},{submit:async v=>await save(v)});let b=form({email:'email!'})",
    );
    assert!(result.js.contains("formStaticSync as"));
    assert!(result.js.contains("o.submit.id = save.id"));
    assert!(result.js.contains("form("));
}

#[test]
fn unbounded_bool_and_num_literals_keep_types_and_bounded_rules_fall_back() {
    let result = output("let f=form({accepted:'bool!',optional:'bool',score:'num!',extra:'num'})");
    assert!(result.js.contains("formScalarsSync as"));
    for (key, kind, req) in [("accepted", "bool", true), ("optional", "bool", false), ("score", "num", true), ("extra", "num", false)] {
        assert!(result.js.contains(&format!("\"{key}\":[\"{kind}\",{req}]")));
    }
    for rule in ["num>=0", "num!<=100", "[bool]", "bool!<=2"] {
        assert!(!output(&format!("let f=form({{field:'{rule}'}})")).js.contains("formScalarsSync as"));
    }
}

#[test]
fn check_validate_unknown_and_receiver_callbacks_keep_dynamic_capability() {
    for options in [
        "{check:{name:async v=>''}}",
        "{validate:v=>({})}",
        "{submit:function(v){return v}}",
        "{submit:unknown}",
        "{...options}",
        "{['submit']:async v=>v}",
        "{__proto__:options}",
        "{init:{},init:{}}",
        "options",
    ] {
        let result=output(&format!("let options={{}};let unknown=()=>{{}};let f=form({{name:'str!'}},{options})"));
        assert!(result.js.contains("formStatic as"),"unsafe sync selection for {options}: {}",result.js);
        assert!(!result.js.contains("formStaticSync as"));
    }
    let result=output("server:async function save(ctx,v){return v};let f=form({name:'str!'}, {submit:save});function inner(save){return form({name:'str!'}, {submit:save})}");
    assert!(result.js.contains("formStaticSync as"));
    assert!(result.js.contains("formStatic as"),"the shadowed callback is not the server stub");
}
