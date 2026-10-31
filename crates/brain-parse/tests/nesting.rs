//! Pathological nesting is a diagnostic, never a stack overflow (which aborts the CLI and the MCP server).
//! Each case runs on a 64 MiB thread so the bound, not the default stack, is what is tested.

use std::path::Path;

fn parse_deep(src: String, file: &'static str) -> Result<(), Vec<brain_parse::Diag>> {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || brain_parse::parse(&src, Path::new(file)).map(drop))
        .unwrap()
        .join()
        .unwrap()
}

fn rejected(src: String, file: &'static str, code: &str) {
    let ds = parse_deep(src, file).expect_err("deep input must be rejected");
    assert_eq!(ds[0].code, code, "{}", ds[0]);
    assert!(ds[0].msg.contains("nested too deeply"), "{}", ds[0]);
}

const N: usize = 100_000;

#[test]
fn deep_script_nesting_is_a_diagnostic() {
    let module = "lib/x.jsbrain";
    rejected(format!("export const a = {}1{}", "(".repeat(N), ")".repeat(N)), module, "B0002");
    rejected(format!("export const a = {}", "[".repeat(N)), module, "B0002");
    rejected(format!("export const a = {}", "{a:".repeat(N)), module, "B0002");
    rejected("{".repeat(N), module, "B0002");
    rejected(format!("{}x", "if (a) ".repeat(N)), module, "B0002");
    rejected(format!("export const a = {}1", "!".repeat(N)), module, "B0002");
    rejected(format!("export const a = {}1", "x => ".repeat(N)), module, "B0002");
    rejected(format!("export const a = {}1", "b ? c : ".repeat(N)), module, "B0002");
    rejected(format!("export const a = {}", "`${".repeat(N)), module, "B0002");
    rejected(format!("let {}a = 1", "[".repeat(N)), module, "B0002");
}

#[test]
fn long_chains_that_deepen_the_ast_are_a_diagnostic() {
    let module = "lib/x.jsbrain";
    rejected(format!("export const a = 1{}", " + 1".repeat(N)), module, "B0002");
    rejected(format!("export const a = 2{}", " ** 2".repeat(N)), module, "B0002");
    rejected(format!("export const a = b{}", ".c".repeat(N)), module, "B0002");
    rejected(format!("export const a = b{}", "()".repeat(N)), module, "B0002");
}

#[test]
fn deep_markup_and_css_nesting_is_a_diagnostic() {
    rejected(format!("{}{}", "<div>".repeat(N), "</div>".repeat(N)), "comp/x.brain", "B0003");
    rejected("{#if a}".repeat(N), "comp/x.brain", "B0003");
    rejected(format!("<p>x</p>\n<style>{}{}</style>", "p{".repeat(N), "}".repeat(N)), "comp/x.brain", "B0003");
}

#[test]
fn realistic_nesting_still_parses() {
    let module = "lib/x.jsbrain";
    parse_deep(format!("export const a = {}1{}", "(".repeat(100), ")".repeat(100)), module).unwrap();
    parse_deep(format!("export const a = 1{}", " + 1".repeat(1500)), module).unwrap();
    parse_deep(format!("export const a = b{}", ".c()".repeat(500)), module).unwrap();
    parse_deep(format!("export const f = () => {{ {} }}", "if (a) { ".repeat(100) + &"}".repeat(100)), module).unwrap();
    parse_deep(format!("<p>{}x{}</p>", "<b>".repeat(200), "</b>".repeat(200)), "comp/x.brain").unwrap();
    parse_deep(format!("<p>x</p>\n<style>{}color: red;{}</style>", "p{".repeat(200), "}".repeat(200)), "comp/x.brain").unwrap();
}
