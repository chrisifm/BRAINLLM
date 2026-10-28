//! #30 (INTERNALS §12): model props `~`, emits, expose/bind:api and channels (B0801–B0807), on QA's
//! channels-app (valid part must be clean, each invalid case gives its code).

use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/fixtures-pending/channels-app").canonicalize().unwrap()
}

fn check(rel: &str) -> Vec<brain_parse::Diag> {
    let abs = app().join(rel);
    let src = std::fs::read_to_string(&abs).unwrap();
    match brain_parse::parse(&src, Path::new(rel)) {
        Ok(c) => {
            let (ctx, _) = brain_check::load_ctx(&abs, &c);
            brain_check::check(&c, &ctx)
        }
        Err(d) => d,
    }
}

#[test]
fn valid_channels_app_is_clean() {
    for rel in [
        "channels/cart.jsbrain",
        "channels/theme.jsbrain",
        "channels/toast.jsbrain",
        "comp/cart-summary.brain",
        "comp/confirm.brain",
        "comp/product-card.brain",
        "comp/qty-input.brain",
        "comp/toaster.brain",
        "ui/panel.brain",
        "ui/price.brain",
        "views/shop.brain",
    ] {
        let d = check(rel);
        assert!(d.is_empty(), "{rel}: {d:#?}");
    }
}

#[test]
fn each_invalid_case_gives_its_code() {
    for (rel, code, line, col) in [
        ("invalid/channels/b0801-module-state.jsbrain", "B0801", 1, 5),
        ("invalid/comp/b0802-emit-undeclared.brain", "B0802", 3, 27),
        ("invalid/comp/b0803-on-undeclared.brain", "B0803", 6, 26),
        ("invalid/comp/b0804-api-undeclared.brain", "B0804", 5, 37),
        ("invalid/comp/b0805-import-internal.brain", "B0805", 2, 12),
        ("invalid/comp/b0806-store-outside-channels.brain", "B0806", 2, 15),
        ("invalid/comp/b0807-use-without-provide.brain", "B0807", 4, 14),
    ] {
        let d = check(rel);
        assert_eq!(d.iter().map(|d| (d.code, d.line, d.col)).collect::<Vec<_>>(), [(code, line, col)], "{rel}: {d:#?}");
        assert!(!d[0].fix.is_empty());
    }
}

#[test]
fn model_props_and_component_binds() {
    let codes = |src: &str| -> Vec<&'static str> {
        let c = brain_parse::parse(src, Path::new("comp/x.brain")).unwrap();
        brain_check::check(&c, &brain_check::Ctx::default()).into_iter().map(|d| d.code).collect()
    };
    // A `~` prop may be written by the child; a plain one may not.
    assert!(codes("<meta props=\"value:num~\"><script>const inc = () => { value = value + 1 }</script><button on:click={inc}>{value}</button>").is_empty());
    assert_eq!(codes("<meta props=\"value:num\"><script>const inc = () => { value = value + 1 }</script><button on:click={inc}>{value}</button>"), ["B0112"]);
    // Parent side with a known child.
    let mut ctx = brain_check::Ctx::default();
    let prop = |name: &str, bindable: bool| brain_parse::ast::Prop { name: name.into(), ty: brain_parse::ast::PropType::Num, optional: false, default: None, bindable, span: Default::default() };
    ctx.imports.insert("./qty.brain".into(), brain_check::Imported::Brain(vec![prop("value", true), prop("max", false)]));
    let with = |src: &str| -> Vec<&'static str> {
        let c = brain_parse::parse(src, Path::new("comp/p.brain")).unwrap();
        brain_check::check(&c, &ctx).into_iter().map(|d| d.code).collect()
    };
    assert!(with("<script>import Qty from './qty.brain'\nlet n = sig(1)</script><Qty bind:value={n} max={3} />").is_empty());
    assert_eq!(with("<script>import Qty from './qty.brain'\nlet n = sig(1)</script><Qty value={n} bind:max={n} />"), ["B0205"]);
    assert_eq!(with("<script>import Qty from './qty.brain'\nlet n = sig(1)</script><Qty bind:value={n} max={3} bind:min={n} />"), ["B0206"]);
}
