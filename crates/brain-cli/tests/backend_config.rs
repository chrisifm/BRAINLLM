//! Settings `brain backend serve` refuses are named before anyone starts it, by `brain check` and `brain doctor` (B0751),
//! and startup reports the same code with the exact variables and no value.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").env_remove("GOOGLE_CLIENT_ID").env_remove("BRAIN_JWT_SECRET").stdin(Stdio::null()).output().unwrap()
}
fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}
fn app(name: &str) -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!("bcfg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    tmp.join("app")
}
fn add_env(dir: &std::path::Path, lines: &str) {
    let path = dir.join(".brain_env");
    let env = std::fs::read_to_string(&path).unwrap();
    assert!(env.contains("[development]\n"), "{env}");
    std::fs::write(&path, env.replacen("[development]\n", &format!("[development]\n{lines}\n"), 1)).unwrap();
}
const CLIENT: &str = "synthetic-client-id-0001.apps.googleusercontent.com";

#[test]
fn google_without_jwt_is_named_by_check_doctor_and_startup_without_values() {
    let dir = app("google");
    std::fs::create_dir_all(dir.join("server/resources")).unwrap();
    assert!(!text(&brain(&dir, &["check"])).contains("B0751"), "a fresh app has no finding");
    add_env(&dir, &format!("GOOGLE_CLIENT_ID={CLIENT}\nBRAIN_MAIL_RESEND_KEY=synthetic-mail-provider-key\nBRAIN_DB_HTTP_POOL=900"));
    let check = text(&brain(&dir, &["check"]));
    for expected in ["B0751", "BRAIN_JWT_SECRET", "BRAIN_MAIL_FROM", "BRAIN_DB_HTTP_POOL must be 1..64"] {
        assert!(check.contains(expected), "{expected}: {check}");
    }
    let doctor = text(&brain(&dir, &["doctor"]));
    assert!(doctor.contains("B0751") && doctor.contains("BRAIN_JWT_SECRET"), "{doctor}");
    let serve = brain(&dir, &["backend", "serve"]);
    let out = text(&serve);
    assert!(!serve.status.success());
    assert!(out.contains("B0751") && out.contains("BRAIN_JWT_SECRET") && out.contains("fix:"), "{out}");
    for output in [&check, &doctor, &out] {
        assert!(!output.contains(CLIENT) && !output.contains("synthetic-mail-provider-key"), "no value leaks: {output}");
    }
}

#[test]
fn an_app_without_the_native_backend_may_use_the_same_key_for_itself() {
    let dir = app("plain");
    add_env(&dir, &format!("GOOGLE_CLIENT_ID={CLIENT}"));
    let _ = std::fs::remove_dir_all(dir.join("server/resources"));
    let _ = std::fs::remove_dir_all(dir.join("server/jobs"));
    let _ = std::fs::remove_dir_all(dir.join("server/routes"));
    assert!(!text(&brain(&dir, &["check"])).contains("B0751"));
}
