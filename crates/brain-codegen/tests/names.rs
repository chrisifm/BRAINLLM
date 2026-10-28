//! `Output.names` (SPEC §11 `--protect`): classes and vars a component owns, and dynamic ones.
use std::path::Path;

fn names(rel: &str, src: &str) -> brain_codegen::Names {
    let c = brain_parse::parse(src, Path::new(rel)).unwrap();
    brain_codegen::names(&c)
}

fn example(rel: &str) -> brain_codegen::Names {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    names(rel, &std::fs::read_to_string(root.join(rel)).unwrap())
}

#[test]
fn counter_and_todo() {
    assert_eq!(example("comp/counter.brain").classes, ["hot"]);
    let t = example("comp/todo.brain");
    assert_eq!(t.classes, ["done", "empty", "on", "todo"]);
    assert!(t.vars.is_empty() && t.dynamic.is_empty());
}

#[test]
fn vars_and_dynamic_classes() {
    let n = names("comp/x.brain", "<script>let x = sig('b')</script><p class={'a-' + x} class:on={true}>.</p><i class=\"k {x}\"></i><style>.p { --mine: 1px; color: var(--fg) } .q:hover { }</style>");
    assert_eq!(n.vars, ["--mine"]);
    assert_eq!(n.classes, ["on", "p", "q"]);
    assert_eq!(n.dynamic, ["*", "a-", "k"]);
}
