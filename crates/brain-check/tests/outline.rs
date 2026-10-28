//! #81 (INTERNALS §27): outline without bodies, focused subgraph and task context within a budget.

use std::path::Path;

fn app() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/fixtures-pending/channels-app").canonicalize().unwrap()
}

#[test]
fn outline_lists_declarations_without_bodies() {
    let g = brain_check::outline::graph(&app());
    let card = &g.nodes["comp/product-card.brain"];
    assert_eq!(card.kind, "component");
    assert_eq!(card.props, ["product:obj"]);
    assert_eq!(card.emits, ["add"]);
    assert!(card.state.contains(&("qty".into(), "sig")));
    assert_eq!(card.children, ["Price", "QtyInput"]);
    let qty = &g.nodes["comp/qty-input.brain"];
    assert_eq!(qty.props, ["value:num~", "label:str"]);
    let j = card.to_json();
    assert!(j.starts_with("{\"file\":\"comp/product-card.brain\",\"name\":\"ProductCard\",\"kind\":\"component\""), "{j}");
    assert!(!j.contains("button"), "outline carries no markup: {j}");
    let theme = &g.nodes["channels/theme.jsbrain"];
    assert_eq!((theme.kind, theme.exports.clone()), ("module", vec!["Theme".to_string()]));
}

#[test]
fn focus_by_file_component_or_route() {
    let g = brain_check::outline::graph(&app());
    let f = g.focus("comp/product-card.brain");
    assert!(f.contains(&"ui/price.brain".to_string()) && f.contains(&"comp/qty-input.brain".to_string()) && f.contains(&"views/shop.brain".to_string()), "{f:?}");
    assert_eq!(g.resolve("/shop").as_deref(), Some("views/shop.brain"));
    assert_eq!(g.resolve("QtyInput").as_deref(), Some("comp/qty-input.brain"));
    let j = g.to_json(Some(&f));
    assert!(j.contains("\"edges\":{") && !j.contains("\"file\":\"comp/toaster.brain\""), "{j}");
}

#[test]
fn context_ranks_by_task_and_respects_budget() {
    let g = brain_check::outline::graph(&app());
    let ctx = g.context("change the qty input stepper value", 10_000);
    assert_eq!(ctx.first().map(|c| c.0.as_str()), Some("comp/qty-input.brain"), "{ctx:?}");
    let small = g.context("empty the cart and show a toast", 200);
    assert!(small.iter().map(|c| c.2).sum::<usize>() <= 200, "{small:?}");
    assert!(g.context("cart toast", 100_000).iter().any(|c| c.0 == "channels/toast.jsbrain"));
}
