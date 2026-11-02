//! A rebuild under a running server: the previous build's hashed chunks stay for a day, then go.
use std::{collections::BTreeSet, fs, path::Path, process::{Command, Stdio}};

fn brain(dir: &Path, args: &[&str]) {
    let out = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
}
fn chunks(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir.join("dist/_b")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()
}

#[test]
fn stale_chunks_outlive_a_rebuild_for_a_day() {
    let tmp = std::env::temp_dir().join(format!("brain-rebuild-chunks-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    brain(&tmp, &["new", "app"]);
    let dir = tmp.join("app");
    let view = dir.join("views/about.brain");
    brain(&dir, &["build"]);
    let first = chunks(&dir);

    let source = fs::read_to_string(&view).unwrap();
    fs::write(&view, source.replacen("</main>", "<p>Changed</p></main>", 1)).unwrap();
    brain(&dir, &["build"]);
    let second = chunks(&dir);
    assert!(first.is_subset(&second), "the previous chunks are still there: {:?}", first.difference(&second).collect::<Vec<_>>());
    let stale: Vec<_> = second.difference(&first).collect();
    assert!(!stale.is_empty(), "the edit produced a new chunk");

    let ledger = dir.join(".brain/stale-chunks.json");
    let recorded: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&fs::read_to_string(&ledger).unwrap()).unwrap();
    assert!(!recorded.is_empty());
    let aged: serde_json::Map<String, serde_json::Value> = recorded.keys().map(|k| (k.clone(), 1000.into())).collect();
    fs::write(&ledger, serde_json::to_string(&aged).unwrap()).unwrap();
    brain(&dir, &["build"]);
    assert!(recorded.keys().all(|k| !dir.join("dist").join(k).exists()), "past the day they are gone");
    assert_eq!(fs::read_to_string(&ledger).unwrap(), "{}");
    let _ = fs::remove_dir_all(&tmp);
}
