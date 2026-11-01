//! `brain edge plan` reads the build and changes nothing; each rule must match what the server actually sends.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

#[test]
fn the_plan_reads_the_build_and_changes_nothing() {
    let tmp = std::env::temp_dir().join(format!("edge-plan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    std::fs::create_dir_all(dir.join("views")).unwrap();
    std::fs::write(dir.join("views/kept.brain"), "<meta cache=\"60\" />\n<meta seo title=\"Kept page\" description=\"A page kept for a minute, with enough words to describe it.\" />\n<script server>\n  export async function load() { return { name: 'item' } }\n</script>\n<main><h1>Kept</h1><p>{props.name}</p></main>\n").unwrap();

    let _ = std::fs::remove_dir_all(dir.join("dist"));
    let missing = brain(&dir, &["edge", "plan"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("brain build"), "{missing:?}");

    let build = brain(&dir, &["build"]);
    assert!(build.status.success(), "{build:?}");
    let manifest = std::fs::read(dir.join("dist/_server/manifest.json")).unwrap();
    let o = brain(&dir, &["edge", "plan", "--json"]);
    assert!(o.status.success(), "{o:?}");
    assert_eq!(std::fs::read(dir.join("dist/_server/manifest.json")).unwrap(), manifest, "the plan writes nothing");
    let plan: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(plan["schemaVersion"], 1);
    let rules = plan["rules"].as_array().unwrap();
    let rule = |path: &str| rules.iter().find(|r| r["path"] == path).unwrap_or_else(|| panic!("no rule for {path}: {plan}"));
    assert_eq!(rule("/kept")["action"], "cache");
    assert_eq!(rule("/kept")["seconds"], 60);
    assert_eq!(rule("/kept")["bypassWhen"], serde_json::json!(["cookie", "authorization"]));
    assert_eq!(rule("/api/*")["action"], "origin");
    assert_eq!(rule("/_b/*")["immutable"], true);

    // a site with no page rendered per request has no server manifest and still has a plan
    std::fs::remove_file(dir.join("views/kept.brain")).unwrap();
    std::fs::remove_dir_all(dir.join("dist")).unwrap();
    assert!(brain(&dir, &["build"]).status.success());
    if !dir.join("dist/_server/manifest.json").exists() {
        let o = brain(&dir, &["edge", "plan"]);
        assert!(o.status.success(), "{o:?}");
        assert!(String::from_utf8_lossy(&o.stdout).contains("/_b/*"), "{o:?}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn another_edge_operation_is_a_usage_error() {
    let dir = std::env::temp_dir();
    let o = brain(&dir, &["edge", "check"]);
    assert!(!o.status.success());
}
