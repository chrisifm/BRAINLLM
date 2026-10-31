//! `brain upgrade` end to end: a new app records `brainVersion`, the dry run writes nothing and lists the manifests
//! between the recorded version and the target, `--apply` advances the recorded version, and the exact-version
//! package-pin preview keeps working.
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn brain(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// Every app file outside caches, with its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !matches!(name.as_str(), ".brain" | "dist" | "node_modules") {
                    stack.push(p);
                }
            } else {
                out.insert(p.strip_prefix(root).unwrap().to_path_buf(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

fn recorded(app: &Path) -> String {
    let bj: Value = serde_json::from_str(&std::fs::read_to_string(app.join("brain.json")).unwrap()).unwrap();
    bj["brainVersion"].as_str().unwrap_or_default().to_string()
}

#[test]
fn upgrade_reports_the_manifests_since_the_recorded_version_and_apply_advances_it() {
    let tmp = std::env::temp_dir().join(format!("bupgrade-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let app = tmp.join("app");
    let me = env!("CARGO_PKG_VERSION");
    assert_eq!(recorded(&app), me, "brain new records the version the app was made with");

    // an app made with 0.5.0
    let bj = std::fs::read_to_string(app.join("brain.json")).unwrap();
    std::fs::write(app.join("brain.json"), bj.replace(&format!("\"brainVersion\": \"{me}\""), "\"brainVersion\": \"0.5.0\"")).unwrap();
    assert_eq!(recorded(&app), "0.5.0");

    // dry run (the default): a report, and not one byte written
    let before = snapshot(&app);
    let out = brain(&app, &["upgrade", "--json", "--to", "0.6.1"]);
    assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
    let r = json(&out);
    assert_eq!(snapshot(&app), before, "--dry-run must not write");
    assert_eq!((r["schemaVersion"].as_u64(), r["mode"].as_str()), (Some(1), Some("dry-run")));
    assert_eq!((r["from"].as_str(), r["fromSource"].as_str(), r["to"].as_str()), (Some("0.5.0"), Some("brain.json brainVersion"), Some("0.6.1")));
    let versions: Vec<&str> = r["releases"].as_array().unwrap().iter().map(|v| v["version"].as_str().unwrap()).collect();
    assert_eq!(versions, ["0.6.0", "0.6.1"], "manifests in (from, to]");
    assert!(r["releases"].as_array().unwrap().iter().all(|v| v["entries"].as_u64() > Some(0)));
    let codemods = r["codemods"].as_array().unwrap();
    assert!(codemods.iter().any(|c| c["id"] == "record-version" && c["status"] == "planned"), "{codemods:?}");
    assert!(r["gates"].as_array().unwrap().iter().any(|g| g["name"] == "check" && g["status"] == "ran"));
    assert!(!r["adopt"].as_array().unwrap().is_empty() || !r["alreadyUsing"].as_array().unwrap().is_empty());

    // the markdown checklist is the default output
    let md = String::from_utf8(brain(&app, &["upgrade", "--dry-run", "--to", "0.6.1"]).stdout).unwrap();
    assert!(md.starts_with("# BRAINLLM upgrade 0.5.0 → 0.6.1 (dry-run)") && md.contains("## Codemods") && md.contains("## Gates"), "{md}");
    assert_eq!(snapshot(&app), before);

    // --apply runs the safe codemods and records the target
    let out = brain(&app, &["upgrade", "--apply", "--no-doctor", "--json", "--to", "0.6.1"]);
    assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
    let r = json(&out);
    assert_eq!(r["mode"], "apply");
    assert!(r["codemods"].as_array().unwrap().iter().any(|c| c["id"] == "record-version" && c["status"] == "applied"));
    assert_eq!(recorded(&app), "0.6.1");
    let after = std::fs::read_to_string(app.join("brain.json")).unwrap();
    assert_eq!(after.replace("\"brainVersion\": \"0.6.1\"", ""), bj.replace(&format!("\"brainVersion\": \"{me}\""), ""), "only the version changed");

    // nothing left between 0.6.1 and 0.6.1
    let r = json(&brain(&app, &["upgrade", "--json", "--to", "0.6.1"]));
    assert_eq!(r["releases"], serde_json::json!([]));
    assert!(r["codemods"].as_array().unwrap().iter().any(|c| c["id"] == "record-version" && c["status"] == "up-to-date"));

    // the exact-version package-pin preview is unchanged
    let out = brain(&app, &["upgrade", me, "--json"]);
    assert!(out.status.success(), "{out:?}");
    assert!(json(&out)["operations"].is_array());

    // usage errors exit 2 with a JSON error
    let out = brain(&app, &["upgrade", "--apply", "--dry-run", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(json(&out)["error"].as_str().unwrap().contains("exclude each other"));
    assert_eq!(brain(&app, &["upgrade", "--bogus"]).status.code(), Some(2));
    assert_eq!(brain(&app, &["upgrade", "--from", "latest", "--json"]).status.code(), Some(2));

    let _ = std::fs::remove_dir_all(&tmp);
}
