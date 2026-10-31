//! The rules of a server handler: `brain check` reports what `brain backend serve` and `brain jobs work` refuse at boot.
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn brain(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").env("DATABASE_URL", "postgres://nobody@127.0.0.1:1/unreachable").stdin(Stdio::null()).output().unwrap()
}
fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn check_and_the_worker_agree_on_what_a_handler_may_be() {
    let tmp = std::env::temp_dir().join(format!("brain-handler-rules-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    let job = dir.join("server/jobs/probe");
    std::fs::create_dir_all(&job).unwrap();
    std::fs::write(job.join("schema.jsbrain"), "<meta server>\nexport default { x: 'str' }\n").unwrap();
    let handler = job.join("handler.jsbrain");

    // `for ... of` is the loop the language has; the documented exclusions are errors.
    std::fs::write(&handler, "<meta server>\nexport default async function handle(ctx) { let n = 0; for (const x of [1]) { n += x } return { n } }\n").unwrap();
    let fine = text(&brain(&dir, &["check"]));
    assert!(!fine.contains("B0772") && !fine.contains("B0773") && !fine.contains("B0110"), "{fine}");

    std::fs::write(&handler, "<meta server>\nconst a = 1\n").unwrap();
    let silent = brain(&dir, &["check"]);
    assert!(text(&silent).contains("B0772"), "{}", text(&silent));
    assert!(silent.status.success(), "a warning does not fail the check: {}", text(&silent));
    let worker = brain(&dir, &["jobs", "work", "probe"]);
    assert!(!worker.status.success() && text(&worker).contains("B0772"), "{}", text(&worker));

    std::fs::write(&handler, "<meta server>\nconst key = 'sk-abcdefghijklmnopqrstuvwx1234'\nexport default async function handle(ctx) { return { key } }\n").unwrap();
    let secret = brain(&dir, &["check"]);
    assert!(!secret.status.success() && text(&secret).contains("B0773"), "{}", text(&secret));
    assert!(!text(&secret).contains("abcdefghijkl"), "a diagnostic never repeats the value");
    let worker = brain(&dir, &["jobs", "work", "probe"]);
    assert!(!worker.status.success() && text(&worker).contains("secret literal") && !text(&worker).contains("abcdefghijkl"), "{}", text(&worker));

    let _ = std::fs::remove_dir_all(&tmp);
}
