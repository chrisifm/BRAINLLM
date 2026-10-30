//! Default login's native branch runs compiled code; no backend password/token is stored by the browser fixture.
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[test]
fn default_login_uses_native_endpoints_without_demo_fallback_or_persistent_auth_state() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let path = std::env::temp_dir().join(format!("brain-native-login-{}", std::process::id()));
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let app = Scratch(path);
    for directory in ["views", "server", "channels", "ui"] {
        std::fs::create_dir_all(app.0.join(directory)).unwrap();
    }
    std::fs::write(
        app.0.join("app.brain"),
        "<html lang=\"en\"><head><title>Login fixture</title></head><body><slot /></body></html>",
    )
    .unwrap();
    let source = include_str!("../../brain-cli/src/templates/new/views/login.brain");
    let auth = include_str!("../../brain-cli/src/templates/new/server/auth.jsbrain");
    let session = include_str!("../../brain-cli/src/templates/new/channels/session.jsbrain");
    assert!(
        !session.contains("sync:"),
        "Signed-in user hints must never be persisted or synchronized as authority"
    );
    for (name, text) in [
        ("views/login.brain", source),
        ("server/auth.jsbrain", auth),
        ("channels/session.jsbrain", session),
        (
            "ui/button.brain",
            include_str!("../../../runtime/ui/button.brain"),
        ),
    ] {
        std::fs::write(app.0.join(name), text).unwrap();
    }
    let options = brain_codegen::Options {
        root: Some(app.0.clone()),
        runtime: format!("file://{}", project.join("runtime/brain.js").display()),
        import_suffix: ".mjs".into(),
        ..Default::default()
    };
    for (name, text) in [
        ("views/login.brain", source),
        ("server/auth.jsbrain", auth),
        ("channels/session.jsbrain", session),
        (
            "ui/button.brain",
            include_str!("../../../runtime/ui/button.brain"),
        ),
    ] {
        let output = brain_codegen::compile_with(text, Path::new(name), &options)
            .unwrap_or_else(|d| panic!("{name}: {d:?}"));
        let code = if name.starts_with("server/") {
            output.server_js.clone()
        } else {
            output.js.clone()
        };
        std::fs::write(app.0.join(format!("{name}.mjs")), code).unwrap();
        if name == "views/login.brain" {
            assert!(
                !output.js.contains("DEMO_PASSWORD")
                    && !output.js.contains("BRAIN_JWT_SECRET")
                    && !output.js.contains("ctx.session.set")
            );
            std::fs::write(app.0.join("views/login.server.mjs"), output.server_js).unwrap();
        }
    }
    let mut child = Command::new(std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into()))
        .arg(project.join("crates/brain-codegen/tests/native_login.mjs"))
        .arg(&app.0)
        .arg(project.join("runtime"))
        .env("NODE_ENV", "development")
        .env("DEMO_PASSWORD", "synthetic-login-demo-password")
        .env("BRAIN_BACKEND_URL", "")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Node is required by the native login regression");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "Compiled native login regression failed");
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Compiled native login exceeded its 10-second deadline")
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
