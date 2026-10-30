//! Payload failures must retain independent, fresh SEO evidence without weakening the build gate.
#![cfg(unix)]
use std::{fs, path::{Path, PathBuf}, process::{Command, Stdio}, sync::atomic::{AtomicU64, Ordering}, time::{Duration, Instant}};
use std::os::unix::process::CommandExt;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct App(PathBuf);
impl Drop for App { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn command(root: &Path, args: &[&str]) -> (bool, String) {
    let log = root.join(format!("doctor-command-{}.log", NEXT.fetch_add(1, Ordering::Relaxed)));
    let file = fs::OpenOptions::new().write(true).create_new(true).open(&log).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(root)
        .env("NO_COLOR", "1").stdin(Stdio::null()).stdout(file.try_clone().unwrap()).stderr(file)
        .process_group(0).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if Instant::now() >= deadline {
            // Only this test's private CLI process group is canceled, never a borrowed browser or service.
            unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL); }
            let _ = child.wait();
            panic!("doctor CLI exceeded its owned 120-second deadline");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(fs::metadata(&log).unwrap().len() <= 2 * 1024 * 1024, "bounded synthetic fixture output");
    let output = fs::read_to_string(&log).unwrap();
    fs::remove_file(log).unwrap();
    (status.success(), output)
}
#[test]
fn budget_failure_reports_current_seo_but_never_reuses_stale_pages_after_source_errors() {
    let dir = std::env::temp_dir().join(format!("brain-doctor-budget-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir(&dir).unwrap();
    let _cleanup = App(dir.clone());
    let (ok, output) = command(&dir, &["new", "app"]);
    assert!(ok, "{output}");
    let app = dir.join("app");
    // This regression is about build/SEO stage independence, not a proof of browser or heap execution.
    fs::remove_file(app.join("counter.test.brain")).unwrap();
    fs::remove_file(app.join("views/login.test.brain")).unwrap();
    let mut config: serde_json::Value = serde_json::from_str(&fs::read_to_string(app.join("brain.json")).unwrap()).unwrap();
    config["budget"]["viewKB"] = serde_json::json!(1);
    let config = serde_json::to_string_pretty(&config).unwrap();
    fs::write(app.join("brain.json"), &config).unwrap();
    fs::create_dir_all(app.join("dist")).unwrap();
    let stale = "<!doctype html><title>Stale doctor sentinel</title>";
    fs::write(app.join("dist/index.html"), stale).unwrap();
    let (ok, output) = command(&app, &["doctor", "--json"]);
    assert!(!ok, "a budget failure must not become a successful doctor");
    let report: serde_json::Value = serde_json::from_str(&output).unwrap_or_else(|e| panic!("{e}: {output}"));
    assert_eq!(report["ok"], false);
    assert_eq!(report["build"]["ok"], false);
    assert_eq!(report["build"]["budget"]["ok"], false);
    assert_eq!(report["build"]["freshOutput"], true);
    assert!(report["build"]["error"].as_str().unwrap().contains("budget.viewKB exceeded"));
    assert!(!report["build"]["report"].as_array().unwrap().is_empty());
    assert!(!report["build"]["ledger"].as_array().unwrap().is_empty());
    assert!(!report["seo"]["pages"].as_array().unwrap().is_empty(), "fresh SEO must run despite budget failure: {output}");
    assert!(report["heap"]["skipped"].is_string(), "no test files is not a measured zero-leak result");
    assert_ne!(fs::read_to_string(app.join("dist/index.html")).unwrap(), stale);
    assert_eq!(fs::read_to_string(app.join("brain.json")).unwrap(), config, "doctor must never raise a budget");
    let (ok, output) = command(&app, &["build", "--json"]);
    assert!(!ok && output.contains("budget.viewKB exceeded"), "normal builds remain gated: {output}");
    fs::write(app.join("comp/broken.brain"), "<script>\nlet broken =\n</script>\n<p>Broken</p>\n").unwrap();
    fs::write(app.join("dist/index.html"), stale).unwrap();
    let (ok, output) = command(&app, &["doctor", "--json"]);
    assert!(!ok);
    let report: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert!(report["check"]["errors"].as_u64().unwrap() > 0);
    assert_eq!(report["build"]["freshOutput"], false);
    assert!(report["seo"]["skipped"].as_str().unwrap().contains("no fresh build output"));
    assert!(report["seo"]["pages"].is_null());
    assert_eq!(fs::read_to_string(app.join("dist/index.html")).unwrap(), stale);
}
