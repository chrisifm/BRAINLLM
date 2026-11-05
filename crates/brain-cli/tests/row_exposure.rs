//! B0749: an unconditioned public read of a resource with a state field warns in `brain check`, a condition or a
//! declared `expose_all_rows` silences it, and a fresh app never carries it.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}
fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}
const WRITES: &str = "create: ['admin'], update: ['admin'], delete: ['admin']";

#[test]
fn ordinary_enums_do_not_imply_a_publication_state() {
    let tmp = std::env::temp_dir().join(format!("brain-row-exposure-enums-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    assert!(brain(&dir, &["add", "resource", "items", "--fields", "title:str!"]).status.success());
    let resource = dir.join("server/resources/items");
    std::fs::write(resource.join("policy.jsbrain"), format!("<meta server>\nexport default {{ read: ['public'], {WRITES} }}\n")).unwrap();
    for required in ["", "!"] {
        std::fs::write(resource.join("schema.jsbrain"), format!("<meta server>\nexport default {{ title: 'str!', category: 'enum(red|blue){required}' }}\n")).unwrap();
        let output = brain(&dir, &["check", "--json"]);
        let report = text(&output);
        assert!(output.status.success(), "{report}");
        assert!(!report.contains("B0749"), "a category is not a publication state: {report}");
    }
    // Optional state fields still warrant the warning: optionality does not establish safe public access.
    std::fs::write(resource.join("schema.jsbrain"), "<meta server>\nexport default { title: 'str!', status: 'enum(draft|published)' }\n").unwrap();
    let report = text(&brain(&dir, &["check", "--json"]));
    assert!(report.contains("B0749"), "{report}");
    std::fs::remove_dir_all(tmp).unwrap();
}

#[test]
fn public_read_of_a_resource_with_state_warns_until_it_is_conditioned_or_declared() {
    let tmp = std::env::temp_dir().join(format!("brain-row-exposure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    let fresh = text(&brain(&dir, &["check"]));
    assert!(!fresh.contains("B0749"), "a fresh app must not carry the warning: {fresh}");

    let added = brain(&dir, &["add", "resource", "posts", "--fields", "title:str! status:str!"]);
    assert!(added.status.success(), "{}", text(&added));
    let policy = dir.join("server/resources/posts/policy.jsbrain");
    let source = std::fs::read_to_string(&policy).unwrap();
    let header = source.lines().filter(|line| line.starts_with("<meta")).collect::<Vec<_>>().join("\n");
    let write = |body: String| std::fs::write(&policy, format!("{header}\nexport default {{ {body} }}\n")).unwrap();

    write(format!("read: ['public'], {WRITES}"));
    let warned = text(&brain(&dir, &["check"]));
    assert!(warned.contains("B0749") && warned.contains("server/resources/posts/policy.jsbrain"), "{warned}");
    assert!(warned.contains("expose_all_rows") && warned.contains("where"), "the warning names both ways out: {warned}");

    write(format!("read: [{{ public: {{ where: {{ status: 'published' }} }} }}, 'admin'], {WRITES}"));
    let conditioned = text(&brain(&dir, &["check"]));
    assert!(!conditioned.contains("B0749"), "{conditioned}");

    write(format!("read: ['public'], expose_all_rows: true, {WRITES}"));
    let declared = text(&brain(&dir, &["check"]));
    assert!(!declared.contains("B0749"), "{declared}");

    write(format!("read: [{{ public: {{ where: {{ missing: 'x' }} }} }}], {WRITES}"));
    let invalid = brain(&dir, &["check"]);
    let message = text(&invalid);
    assert!(!invalid.status.success() && message.contains("`missing` is not a declared field") && message.contains("fix:"), "{message}");
    let _ = std::fs::remove_dir_all(&tmp);
}
