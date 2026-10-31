//! B0770: a bucket that does not say which types it takes warns in `brain check`, and keeps compiling.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}
fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn a_bucket_without_types_warns_until_it_lists_them_or_says_any() {
    let tmp = std::env::temp_dir().join(format!("brain-bucket-types-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    assert!(!text(&brain(&dir, &["check"])).contains("B0770"));
    std::fs::create_dir_all(dir.join("server/files")).unwrap();
    let bucket = dir.join("server/files/uploads.jsbrain");
    let write = |body: &str| std::fs::write(&bucket, format!("<meta server>\nexport default {{ create: ['admin']{body} }}\n")).unwrap();

    write("");
    let run = brain(&dir, &["check"]);
    let omitted = text(&run);
    assert!(omitted.contains("B0770") && omitted.contains("server/files/uploads.jsbrain"), "{omitted}");
    assert!(omitted.contains("types: '*'"), "the warning names the explicit opt-out: {omitted}");
    assert!(run.status.success(), "a warning never fails a compiling app: {omitted}");

    write(", types: []");
    assert!(text(&brain(&dir, &["check"])).contains("B0770"));

    write(", types: ['application/pdf']");
    assert!(!text(&brain(&dir, &["check"])).contains("B0770"));

    write(", types: '*'");
    let any = brain(&dir, &["check"]);
    assert!(!text(&any).contains("B0770") && any.status.success(), "{}", text(&any));

    write(", types: 'pdf'");
    let wrong = text(&brain(&dir, &["check"]));
    assert!(wrong.contains("B0735") && wrong.contains("server/files/uploads.jsbrain"), "{wrong}");
    let _ = std::fs::remove_dir_all(&tmp);
}
