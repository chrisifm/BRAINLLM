//! Budgets are a build gate, including JSON/progress-off output modes.
use std::process::{Command, Stdio};
use std::path::Path;
use brain_mcp::Json;
fn run(root: &Path, args: &[&str], expected: bool) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(root).stdin(Stdio::null()).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.success(), expected, "{args:?}: {stdout} {}", String::from_utf8_lossy(&out.stderr));
    if expected { stdout } else { stdout + &String::from_utf8_lossy(&out.stderr) }
}
#[test]
fn build_budget_measures_output_and_fails_when_route_exceeds_it() {
    let root = std::env::temp_dir().join(format!("brain-budget-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    run(&root, &["new", "app"], true);
    let app = root.join("app");
    let config = std::fs::read_to_string(app.join("brain.json")).unwrap();
    let set_budget = |budget: &str| {
        let Json::Obj(mut entries) = Json::parse(&config).unwrap() else { panic!("object config") };
        entries.retain(|(name,_)| name != "budget");
        entries.push(("budget".into(), Json::parse(budget).unwrap()));
        std::fs::write(app.join("brain.json"), Json::Obj(entries).stringify()).unwrap();
    };
    set_budget(r#"{"viewKB":100000}"#);
    let out = Json::parse(run(&app, &["build", "--json", "--progress=off"], true).trim()).unwrap();
    let Json::Arr(rows) = out.get("report").unwrap() else { panic!("report array") };
    for row in rows {
        let route = row.get("route").unwrap().as_str().unwrap();
        // Exact emitted HTML transfer bytes, including the raw fallback when gzip is absent.
        let rel = if route == "/" { "index.html".into() } else { format!("{}/index.html", route.trim_matches('/')) };
        let path = app.join("dist").join(rel);
        if !path.exists() { continue; }
        let raw = std::fs::metadata(&path).unwrap().len();
        let gz = std::fs::metadata(format!("{}.gz", path.display())).map(|m| m.len()).unwrap_or(raw);
        assert_eq!(row.get("htmlGz"), Some(&Json::Num(gz.min(raw) as f64)));
    }
    set_budget(r#"{"viewKB":0}"#);
    assert!(run(&app, &["build", "--json", "--progress=off"], false).contains("budget.viewKB exceeded on /"));
    set_budget(r#"{"mountMs":16}"#);
    assert!(run(&app, &["build", "--progress=off"], false).contains("requires browser measurements"));
    set_budget(r#"{"heapMB":20}"#);
    assert!(run(&app, &["build", "--progress=off"], false).contains("requires browser measurements"));
    std::fs::remove_dir_all(root).unwrap();
}
