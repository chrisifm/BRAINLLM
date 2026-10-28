//! Exercise the public CLI contract, not only scaffold helpers.
use std::path::Path;
use std::process::{Command, Stdio};

fn run(root: &Path, args: &[&str], success: bool) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(root).stdin(Stdio::null()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.success(), success, "{args:?}: {text}");
    text
}

#[test]
fn public_module_generator_options_produce_checked_files_and_reject_bad_flags() {
    let root = std::env::temp_dir().join(format!("brain-module-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    run(&root, &["new", "app"], true);
    let app = root.join("app");
    for args in [
        vec!["add", "type", "User", "--fields", "id:num! email:email!"],
        vec!["add", "type", "Order", "--fields", "id:num! owner:User"],
        vec!["add", "channel", "cart", "--store", "{items:[],count:0}"],
        vec!["add", "channel", "events", "--bus"],
        vec!["add", "channel", "theme", "--key"],
        vec!["add", "service", "payments"],
        vec!["add", "server", "users-db"],
        vec!["add", "guard", "admin"],
        vec!["add", "api", "users", "--get", "[User]", "--post", "User:User"],
    ] { run(&app, &args, true); }
    run(&app, &["check"], true);
    let api = std::fs::read_to_string(app.join("api/users.jsbrain")).unwrap();
    run(&app, &["add", "api", "users", "--get", "num"], false);
    assert_eq!(std::fs::read_to_string(app.join("api/users.jsbrain")).unwrap(), api);
    run(&app, &["add", "channel", "bad", "--bus", "--key"], false);
    run(&app, &["add", "server", "../outside"], false);
    run(&app, &["add", "service", "bad", "--get", "num"], false);
    run(&app, &["add", "channel", "bad", "--store", "{value:run()}"], false);
    assert!(!app.join("channels/bad.jsbrain").exists());
    std::fs::remove_dir_all(root).unwrap();
}
