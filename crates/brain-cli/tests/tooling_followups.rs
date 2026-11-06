#[path = "../src/http_observer.rs"]
mod http_observer;
#[path = "../src/adoption_audit.rs"]
mod adoption_audit;
#[path = "../src/practices.rs"]
mod practices;
#[path = "../src/tooling_bench.rs"]
mod tooling_bench;
#[path = "../src/tooling_connect.rs"]
mod tooling_connect;

use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "brain-tooling-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn write(&self, name: &str, text: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn adoption_inventory_preserves_sources_and_never_returns_private_values_or_runs_scripts() {
    let f = Fixture::new();
    f.write("package.json", r#"{"name":"private-package-value","dependencies":{"react":"private-version-value"},"scripts":{"audit":"create-private-marker"}}"#);
    f.write(
        "src/pages/item.tsx",
        "throw new Error('private-source-value')",
    );
    f.write("src/item.test.ts", "private-test-value");
    f.write(".brain_env", "PRIVATE_KEY=private-environment-value");
    f.write("node_modules/ignored.js", "private-dependency-value");
    let before = fs::read(f.0.join("src/pages/item.tsx")).unwrap();
    let report = adoption_audit::inventory(&f.0).unwrap();
    assert_eq!(report["frameworks"], json!(["React"]));
    assert_eq!(report["sourceCount"], 2);
    assert_eq!(report["routeCandidates"], json!(["src/pages/item.tsx"]));
    assert_eq!(report["testCandidates"], json!(["src/item.test.ts"]));
    assert!(!report.to_string().contains("private-"));
    assert_eq!(report["inspection"]["executedSource"], false);
    assert!(!f.0.join(".brain").exists());
    assert_eq!(fs::read(f.0.join("src/pages/item.tsx")).unwrap(), before);
}

#[test]
fn adoption_refuses_symlink_package_and_oversized_metadata_and_skips_source_symlinks() {
    let f = Fixture::new();
    f.write("package.json", &" ".repeat(256 * 1024 + 1));
    assert!(
        adoption_audit::inventory(&f.0)
            .unwrap_err()
            .contains("256 KiB")
    );
    fs::remove_file(f.0.join("package.json")).unwrap();
    #[cfg(unix)]
    {
        f.write("outside.txt", "private-outside-value");
        std::os::unix::fs::symlink(f.0.join("outside.txt"), f.0.join("linked.js")).unwrap();
        let report = adoption_audit::inventory(&f.0).unwrap();
        assert_eq!(report["skippedSymlinks"], 1);
        assert_eq!(report["sourceCount"], 0);
        std::os::unix::fs::symlink(f.0.join("outside.txt"), f.0.join("package.json")).unwrap();
        assert!(adoption_audit::inventory(&f.0).is_err());
    }
}

#[test]
fn adoption_requires_explicit_audit_and_bounds_tree_depth() {
    assert!(adoption_audit::run(&[]).is_err());
    let f = Fixture::new();
    f.write(&format!("{}end.js", "part/".repeat(22)), "ignored");
    assert!(
        adoption_audit::inventory(&f.0)
            .unwrap_err()
            .contains("depth")
    );
}

#[test]
fn adoption_never_returns_an_apparently_complete_truncated_source_inventory() {
    let f = Fixture::new();
    for index in 0..2049 {
        f.write(&format!("src/item-{index}.js"), "");
    }
    assert!(
        adoption_audit::inventory(&f.0)
            .unwrap_err()
            .contains("2048 source files")
    );
}

#[test]
fn practices_pagination_is_complete_sorted_and_uses_actual_catalog_records() {
    let mut cursor = 0;
    let mut codes = Vec::new();
    loop {
        let result = practices::query(&json!({"limit":7,"cursor":cursor})).unwrap();
        for row in result["practices"].as_array().unwrap() {
            let code = row["code"].as_str().unwrap();
            let actual = brain_lex::codes::CODES
                .iter()
                .find(|r| r.0 == code)
                .unwrap();
            assert_eq!(row["title"], actual.2);
            assert_eq!(row["example"], actual.4);
            codes.push(code.to_string());
        }
        match result["nextCursor"].as_u64() {
            Some(next) => cursor = next,
            None => break,
        }
    }
    assert_eq!(codes.len(), brain_lex::codes::CODES.len());
    assert!(codes.windows(2).all(|pair| pair[0] < pair[1]));
    for request in [
        json!({"limit":0}),
        json!({"limit":101}),
        json!({"cursor":99999}),
        json!({"family":"unknown"}),
        json!({"codePrefix":"B\u{e9}"}),
        json!({"unknown":1}),
        json!({"cursor":1.5}),
    ] {
        assert!(practices::query(&request).is_err(), "{request}");
    }
    let css = practices::query(&json!({"family":"css","limit":100})).unwrap();
    assert!(
        css["practices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["code"].as_str().unwrap().starts_with("B03"))
    );
}

#[test]
fn only_optional_webmcp_discovery_nudges_can_be_lowered_and_never_promoted_to_errors() {
    for code in ["B1706", "B1707", "B1708"] {
        assert!(practices::adjustable(code));
        assert_eq!(practices::nudge_level(code, Some("off"), "warning"), "off");
        assert_eq!(
            practices::nudge_level(code, Some("error"), "warning"),
            "warning"
        );
    }
    for code in [
        "B1701", "B1702", "B1703", "B1704", "B1705", "B1709", "B0401", "B0701",
    ] {
        assert!(!practices::adjustable(code));
        assert_eq!(practices::nudge_level(code, Some("off"), "error"), "error");
    }
}

#[test]
fn bench_and_connect_reject_invalid_inputs_before_external_work() {
    for args in [
        vec![],
        vec!["--url", "https://example.com/", "--requests", "21"],
        vec!["--url", "https://example.com/", "--requests", "0"],
        vec!["--url", "https://example.com/?secret=value"],
    ] {
        assert!(
            tooling_bench::run(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err()
        );
    }
    let f = Fixture::new();
    assert!(tooling_connect::run(&f.0, &f.0.join("missing-executable")).is_err());
}

#[cfg(unix)]
#[test]
fn connect_bounds_handshake_output_and_reaps_its_owned_child_on_timeout() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.write("probe", "#!/bin/sh\nprintf '%070000d' 0\n");
    let executable = f.0.join("probe");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        tooling_connect::run(&f.0, &executable)
            .unwrap_err()
            .contains("64 KiB")
    );
    f.write(
        "probe",
        "#!/bin/sh\nprintf '%s' \"$$\" > probe.pid\nwhile read line; do :; done\n",
    );
    let started = std::time::Instant::now();
    assert!(
        tooling_connect::run(&f.0, &executable)
            .unwrap_err()
            .contains("5 second")
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(7));
    let pid = fs::read_to_string(f.0.join("probe.pid"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    // Signal 0 only checks existence; it does not send a signal or terminate another process.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
