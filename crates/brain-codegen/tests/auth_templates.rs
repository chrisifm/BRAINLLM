//! The shipped native account templates must compile against explicit project design-system components.
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct App(PathBuf);
impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn recovery_and_verification_templates_use_real_lifecycle_and_project_component_imports() {
    let root = std::env::temp_dir().join(format!(
        "brain-auth-template-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let app = App(root);
    std::fs::create_dir_all(app.0.join("ui")).unwrap();
    std::fs::create_dir_all(app.0.join("views")).unwrap();
    std::fs::write(
        app.0.join("app.brain"),
        "<html lang=\"en\"><head><title>Auth fixture</title></head><body><slot /></body></html>",
    )
    .unwrap();
    for (name, source) in [
        ("button", include_str!("../../../runtime/ui/button.brain")),
        ("field", include_str!("../../../runtime/ui/field.brain")),
        (
            "error-state",
            include_str!("../../../runtime/ui/error-state.brain"),
        ),
    ] {
        std::fs::write(app.0.join(format!("ui/{name}.brain")), source).unwrap();
    }
    let input = include_str!("../../../runtime/ui/input.brain");
    let (meta, body) = input.split_once('\n').unwrap();
    std::fs::write(
        app.0.join("ui/input.brain"),
        format!("{meta}\n<script>import Field from './field.brain'</script>\n{body}"),
    )
    .unwrap();
    for (name, source) in [
        (
            "reset-password",
            include_str!("../../brain-cli/src/templates/auth/reset-password.brain"),
        ),
        (
            "verify-email",
            include_str!("../../brain-cli/src/templates/auth/verify-email.brain"),
        ),
    ] {
        assert!(source.contains("mounted(() =>"));
        assert!(!source.contains("\n  mount("));
        assert!(source.contains("import Button from '../ui/button.brain'"));
        let file = format!("views/{name}.brain");
        std::fs::write(app.0.join(&file), source).unwrap();
        let result = brain_codegen::compile_with(
            source,
            Path::new(&file),
            &brain_codegen::Options {
                root: Some(app.0.clone()),
                ..Default::default()
            },
        );
        let output =
            result.unwrap_or_else(|diagnostics| panic!("{name} must compile: {diagnostics:?}"));
        assert!(
            output.js.contains("mounted"),
            "The lifecycle must be emitted, not a test mount call"
        );
        assert!(
            output.js.contains("../ui/button.brain"),
            "Use the project's button rather than an implicit replacement"
        );
    }
}
