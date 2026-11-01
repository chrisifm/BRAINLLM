//! B0797: a field whose name says it is internal, returned to every reader by default, warns until its exposure is stated.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}
fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn internal_looking_fields_warn_until_their_exposure_is_stated() {
    let tmp = std::env::temp_dir().join(format!("brain-public-fields-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    assert!(!text(&brain(&dir, &["check"])).contains("B0797"), "a new app is silent");
    let resource = dir.join("server/resources/items");
    std::fs::create_dir_all(&resource).unwrap();
    std::fs::write(resource.join("policy.jsbrain"), "<meta server>\nexport default { read: ['public'], create: ['admin'], update: ['admin'], delete: ['admin'] }\n").unwrap();
    let schema = |fields: &str| std::fs::write(resource.join("schema.jsbrain"), format!("<meta server>\nexport default {{ title: 'str!', {fields} }}\n")).unwrap();

    for field in ["password_hash", "apikey", "internal_notes", "legacy_id", "reset_token", "external_id"] {
        schema(&format!("{field}: 'str'"));
        let run = brain(&dir, &["check"]);
        let out = text(&run);
        assert!(out.contains("B0797") && out.contains(field) && out.contains("server/resources/items/schema.jsbrain"), "{field}: {out}");
        assert!(run.status.success(), "a warning does not fail the check: {out}");
    }
    // readonly is still returned to readers
    schema("session_token: { type: 'str', access: 'readonly' }");
    assert!(text(&brain(&dir, &["check"])).contains("B0797"));
    for stated in [
        "password_hash: 'str private'",
        "api_key: 'str writeonly'",
        "internal_notes: { type: 'str', visible: ['admin'] }",
        "legacy_id: { type: 'str', visible: ['public'] }",
        "tokens: 'int'",
        "hashtag: 'str'",
        "internally: 'str'",
    ] {
        schema(stated);
        let out = text(&brain(&dir, &["check"]));
        assert!(!out.contains("B0797"), "{stated}: {out}");
    }
    // a server-only resource answers no reader
    schema("password_hash: 'str'");
    std::fs::write(resource.join("policy.jsbrain"), "<meta server>\nexport default { api: false, read: ['admin'], create: ['admin'], update: ['admin'], delete: ['admin'] }\n").unwrap();
    assert!(!text(&brain(&dir, &["check"])).contains("B0797"));
    let _ = std::fs::remove_dir_all(&tmp);
}
