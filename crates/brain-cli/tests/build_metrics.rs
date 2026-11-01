//! Per-stage observations of `brain build`: same JSON in every terminal mode, and never part of the output.
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

fn build(dir: &Path, args: &[&str]) -> (serde_json::Value, String) {
    let mut all = vec!["build", "--json", "--site", "https://example.dev"];
    all.extend(args);
    let o = Command::new(env!("CARGO_BIN_EXE_brain")).args(&all).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    (serde_json::from_slice(&o.stdout).unwrap(), String::from_utf8_lossy(&o.stderr).into_owned())
}

fn dist(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, at: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(at).unwrap().flatten() {
            if e.path().is_dir() {
                walk(base, &e.path(), out);
            } else {
                out.insert(e.path().strip_prefix(base).unwrap().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&dir.join("dist"), &dir.join("dist"), &mut out);
    out
}

#[test]
fn stages_and_counters_exist_without_a_terminal_and_never_reach_the_output() {
    let tmp = std::env::temp_dir().join(format!("bmetrics-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let new = Command::new(env!("CARGO_BIN_EXE_brain")).args(["new", "app"]).current_dir(&tmp).stdin(Stdio::null()).output().unwrap();
    assert!(new.status.success());
    let dir = tmp.join("app");

    let (cold, _) = build(&dir, &["--quiet"]);
    let metrics = &cold["metrics"];
    let stages: Vec<&str> = metrics["stages"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    for stage in ["check", "compile", "minify", "pack", "write"] {
        assert!(stages.contains(&stage), "{stage} in {stages:?}");
    }
    assert!(metrics["wallMs"].as_f64().unwrap() > 0.0);
    for stage in metrics["stages"].as_array().unwrap() {
        assert!(stage["wallMs"].is_number() && stage["cpuMs"].is_number() && stage["items"].is_number(), "{stage}");
    }
    let counter = |m: &serde_json::Value, k: &str| m["counters"][k].as_u64().unwrap_or_else(|| panic!("counter {k}"));
    assert!(counter(metrics, "compileRuns") > 0, "a cold build compiles");
    assert_eq!(counter(metrics, "compileCacheHits") + counter(metrics, "compileCacheMisses") > 0, true);

    let first = dist(&dir);
    // The first build of a new app creates private files that a dependency listing observes once.
    build(&dir, &[]);
    let (warm, _) = build(&dir, &[]);
    assert_eq!(counter(&warm["metrics"], "compileRuns"), 0, "a warm build executes no compilation: {}", warm["metrics"]);
    assert!(counter(&warm["metrics"], "compileCacheHits") > 0);
    assert_eq!(cold["build"], warm["build"], "timings never enter the signature");
    assert_eq!(first, dist(&dir), "same bytes whatever the observations were");
    for (name, bytes) in &first {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains("wallMs") && !text.contains("cpuMs"), "{name} carries timings");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
