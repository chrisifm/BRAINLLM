//! Compiler-only plain-store specialization never changes the author-visible store API.
use std::path::Path;
fn compile(source: &str, file: &str) -> brain_codegen::Output {
    brain_codegen::compile(source, Path::new(file)).unwrap_or_else(|diags| panic!("{diags:#?}"))
}
#[test]
fn closed_plain_calls_use_an_exact_internal_import_and_preserve_channel_identity() {
    for options in ["", ", {}"] {
        let output = compile(
            &format!("export default store({{user:null}}{options})\n"),
            "channels/session.jsbrain",
        );
        assert!(
            output.js.contains("plainStore as $brainBudgetPlainStore"),
            "{}",
            output.js
        );
        assert!(output.js.contains("$brainBudgetPlainStore("));
        assert!(output.js.contains("\"channels/session\""));
        assert!(!output.js.contains("import { store }"));
    }
}
#[test]
fn options_and_unknown_calls_retain_the_complete_generic_store_path() {
    for call in [
        "store({user:null},{persist:true})",
        "store({user:null},{sync:'tab'})",
        "store({user:null},{name:'custom'})",
        "store({user:null},undefined)",
        "store({user:null},{},'explicit')",
        "store(...values)",
    ] {
        let output = compile(
            &format!("const values=[{{user:null}},{{persist:true}}]\nexport default {call}\n"),
            "channels/unknown.jsbrain",
        );
        assert!(
            !output.js.contains("plainStore as"),
            "{call}: {}",
            output.js
        );
        assert!(output.js.contains("from \"/@brain/brain-ch.js\""));
    }
    let output = compile(
        "const factory=store\nexport default factory({user:null})\n",
        "channels/alias.jsbrain",
    );
    assert!(!output.js.contains("plainStore as"));
    assert!(output.js.contains("import { store }"));
}
#[test]
fn lexical_shadowing_never_selects_the_native_helper() {
    let output = compile(
        "const store=(value)=>value\nexport default store({count:1})\n",
        "lib/local.jsbrain",
    );
    assert!(!output.js.contains("plainStore as"));
    assert!(!output.js.contains("brain-ch.js"));
}
#[test]
fn specialized_component_stores_keep_preventive_owner_allocation_hooks() {
    let output = compile(
        "<script>\nconst Counter=key('Counter',null)\nconst state=provide(Counter,store({count:0}))\nconst increment=()=>{state.count++}\n</script>\n<button on:click={increment}>{state.count}</button>",
        "comp/counter.brain",
    );
    assert!(output.js.contains("plainStore as"));
    assert!(output.js.contains("budgetOwner"), "{}", output.js);
}
