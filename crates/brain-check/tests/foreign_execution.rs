use std::path::Path;
fn diagnostics(source: &str, file: &str) -> Vec<brain_parse::Diag> {
    let component = brain_parse::parse(source, Path::new(file)).unwrap();
    brain_check::check(&component, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0912").collect()
}
#[test]
fn foreign_main_thread_import_risk_has_exact_source_and_worker_action() {
    let findings = diagnostics("<script>\nimport compute from './compute.mjs?mode=fast'\nconst run = () => compute()\n</script><button on:click={run}>Run</button>", "comp/main.brain");
    assert_eq!(findings.len(), 1);
    assert!(findings[0].is_warning());
    assert_eq!(findings[0].line, 2);
    assert!(findings[0].fix.contains(".worker.jsbrain"));
    assert!(findings[0].edits.is_empty());
}
#[test]
fn workers_server_and_compiled_modules_do_not_receive_main_thread_risk_claims() {
    assert!(diagnostics("import compute from './compute.mjs'; export const run = () => compute()", "lib/math.worker.jsbrain").is_empty());
    assert!(diagnostics("<meta server>\nimport compute from './compute.js'; export const run = () => compute()", "server/math.jsbrain").is_empty());
    assert!(diagnostics("import compute from './compute.jsbrain'; export const run = () => compute()", "lib/math.jsbrain").is_empty());
}
