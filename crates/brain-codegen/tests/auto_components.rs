//! Missing component imports normalize from one exact bounded project match, without source writes.
use std::path::{Path, PathBuf};
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct App(PathBuf);
impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl App {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let app = Self(std::env::temp_dir().join(format!(
            "brain-discover-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )));
        app.write(
            "app.brain",
            "<html lang=\"en\"><head><title>App</title></head><body><slot /></body></html>",
        );
        app.write("comp/parent.brain", "");
        app
    }
    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    fn compile(&self, source: &str) -> Result<brain_codegen::Output, Vec<brain_parse::Diag>> {
        brain_codegen::compile_with(
            source,
            Path::new("comp/parent.brain"),
            &brain_codegen::Options {
                root: Some(self.0.clone()),
                dev: true,
                ..Default::default()
            },
        )
    }
}
#[test]
fn multiple_unique_components_use_one_import_edit_and_do_not_write_source() {
    let app = App::new();
    app.write(
        "comp/user-card.brain",
        "<meta props=\"label:str\"><p>{label}</p>",
    );
    app.write("ui/info-badge.brain", "<span>Badge</span>");
    let source = "<!-- <script>fake</script> -->\n<meta props=\"title:str?\">\n<head><meta name=\"example\" content=\"head\"></head>\n<UserCard label=\"A\"/><InfoBadge/><UserCard label=\"B\"/>";
    app.write("comp/parent.brain", source);
    let output = app
        .compile(source)
        .unwrap_or_else(|diags| panic!("{diags:?}"));
    let infos: Vec<_> = output
        .warnings
        .iter()
        .filter(|diag| diag.code == "B0222")
        .collect();
    assert_eq!(infos.len(), 1);
    assert!(infos[0].is_info());
    assert_eq!(infos[0].edits.len(), 1);
    let edit = &infos[0].edits[0];
    assert!(edit.text.contains("import UserCard") && edit.text.contains("import InfoBadge"));
    let fixed = brain_parse::Diag::apply(source, &infos[0].edits).unwrap();
    assert_eq!(fixed.lines().count(), source.lines().count(), "automatic imports must preserve original source line numbers");
    assert_eq!(
        fixed.matches("<script>").count(),
        2,
        "one fake comment tag plus one real script"
    );
    assert!(fixed.rfind("<script>").unwrap() < fixed.find("<head>").unwrap());
    assert!(
        output.js.contains("./user-card.brain") && output.js.contains("../ui/info-badge.brain"),
        "{}",
        output.js
    );
    assert_eq!(
        std::fs::read_to_string(app.0.join("comp/parent.brain")).unwrap(),
        source
    );
    assert!(
        output
            .deps
            .contains(&app.0.join("comp").canonicalize().unwrap())
            && output
                .deps
                .contains(&app.0.join("ui").canonicalize().unwrap()),
        "normalized pass must retain directory invalidation deps"
    );
}
#[test]
fn existing_empty_and_nonempty_scripts_receive_imports_at_the_correct_location() {
    let app = App::new();
    app.write("comp/user-card.brain", "<p>Card</p>");
    for source in [
        "<script>const value = 'A'</script><UserCard/><p>{value}</p>",
        "<script>/* empty */</script><UserCard/>",
        "<script/><UserCard/>",
        "<p>Before</p><script>const value = 'A'</script><UserCard/><p>{value}</p>",
        "<p>{'<script></script>'}</p><UserCard/>",
    ] {
        let output = app
            .compile(source)
            .unwrap_or_else(|diags| panic!("{source}: {diags:?}"));
        assert!(output.js.contains("./user-card.brain"), "{}", output.js);
        assert_eq!(
            output
                .warnings
                .iter()
                .filter(|diag| diag.code == "B0222")
                .count(),
            1
        );
    }
}
#[test]
fn explicit_imports_local_bindings_native_elements_and_builtin_kit_keep_priority() {
    let app = App::new();
    app.write("comp/user-card.brain", "<p>Card</p>");
    app.write("ui/user-card.brain", "<p>Other card</p>");
    app.write("comp/button.brain", "<p>Not the kit button</p>");
    app.write("comp/portal.brain", "<p>Not Portal</p>");
    for source in [
        "<script>import UserCard from './user-card.brain'</script><UserCard/>",
        "<script>const UserCard = () => 'Local'</script><UserCard/>",
        "<button>Native</button><x-widget></x-widget><Button>Kit</Button>",
        "<Portal to=\"#layer\"><p>Portaled</p></Portal><div id=\"layer\"></div>",
    ] {
        let output = app
            .compile(source)
            .unwrap_or_else(|diags| panic!("{source}: {diags:?}"));
        assert!(!output.warnings.iter().any(|diag| diag.code == "B0222"));
    }
}
#[test]
fn ambiguous_missing_and_direct_self_tags_never_guess() {
    let app = App::new();
    app.write("comp/user-card.brain", "<p>Card</p>");
    app.write("features/user-card.brain", "<p>Other</p>");
    let diags = app.compile("<UserCard/>").unwrap_err();
    let diag = diags.iter().find(|diag| diag.code == "B0203").unwrap();
    assert!(
        diag.msg.contains("comp/user-card.brain") && diag.msg.contains("features/user-card.brain"),
        "{diag:?}"
    );
    assert!(!diags.iter().any(|diag| diag.code == "B0222"));
    for source in ["<MissingCard/>", "<Parent/>"] {
        assert!(app
            .compile(source)
            .unwrap_err()
            .iter()
            .any(|diag| diag.code == "B0203"));
    }
}
#[test]
fn discovered_children_receive_real_prop_validation_on_the_next_pass() {
    let app = App::new();
    app.write(
        "comp/user-card.brain",
        "<meta props=\"label:str\"><p>{label}</p>",
    );
    let diags = app.compile("<UserCard wrong=\"A\"/>").unwrap_err();
    assert!(diags
        .iter()
        .any(|diag| diag.code == "B0222" && diag.is_info()));
    assert!(diags.iter().any(|diag| diag.code == "B0206"));
    assert!(diags.iter().any(|diag| diag.code == "B0207"));
}
#[test]
fn metadata_cache_and_directory_deps_invalidate_when_a_second_match_is_created() {
    let app = App::new();
    app.write("comp/user-card.brain", "<p>Card</p>");
    let source = "<UserCard/>";
    let output = app.compile(source).unwrap();
    assert!(output
        .deps
        .contains(&app.0.join("comp").canonicalize().unwrap()));
    app.write("features/user-card.brain", "<p>Ambiguous</p>");
    let diags = app.compile(source).unwrap_err();
    assert!(diags
        .iter()
        .any(|diag| diag.code == "B0203" && diag.msg.contains("ambiguous")));
}
#[test]
fn discovery_refuses_partial_indexes_and_does_not_cross_nested_apps() {
    let app = App::new();
    app.write("comp/user-card.brain", "<p>Card</p>");
    app.write("nested/app.brain", "<slot/>");
    app.write("nested/comp/user-card.brain", "<p>Other app</p>");
    app.compile("<UserCard/>")
        .unwrap_or_else(|diags| panic!("{diags:?}"));
    for index in 0..260 {
        std::fs::create_dir_all(app.0.join(format!("features/f{index}"))).unwrap();
    }
    let diags = app.compile("<UserCard/>").unwrap_err();
    assert!(diags
        .iter()
        .any(|diag| diag.code == "B0203" && diag.msg.contains("incomplete")));
}
#[cfg(unix)]
#[test]
fn symlink_candidates_are_not_followed_or_used_as_partial_unique_matches() {
    let app = App::new();
    app.write("comp/user-card.brain", "<p>Card</p>");
    std::os::unix::fs::symlink(
        app.0.join("comp/user-card.brain"),
        app.0.join("comp/linked.brain"),
    )
    .unwrap();
    let diags = app.compile("<UserCard/>").unwrap_err();
    assert!(diags
        .iter()
        .any(|diag| diag.code == "B0203" && diag.msg.contains("incomplete")));
}
