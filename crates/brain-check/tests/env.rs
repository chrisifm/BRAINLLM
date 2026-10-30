//! #137 (INTERNALS §36): `.brain_env` parser, precedence, expansion and B0720–B0724. No value ever appears in a diagnostic.

use brain_check::env::{check_file, empty_keys, parse, unused_keys};
use brain_check::Ctx;
use std::path::Path;

const ENV: &str = "# shared\nPUBLIC_SITE_NAME=Nimbus\nAPI_TIMEOUT_MS=8000\nPUBLIC_API_URL=\"http://localhost/api\" # comment\n\n[development]\nDATABASE_URL=postgres://localhost/${PUBLIC_SITE_NAME}_dev\n\n[production]\nPUBLIC_API_URL='https://nimbus.dev/api'\nDATABASE_URL=            # from the host\n";

fn none(_: &str) -> Option<String> {
    None
}

#[test]
fn sections_precedence_and_expansion() {
    let f = parse(ENV);
    assert!(f.errors.is_empty(), "{:?}", f.errors);
    assert_eq!(f.environments(), ["development", "production"]);
    let dev = f.resolve("development", &none);
    assert_eq!(dev["PUBLIC_API_URL"], "http://localhost/api");
    assert_eq!(dev["DATABASE_URL"], "postgres://localhost/Nimbus_dev");
    assert_eq!(dev["API_TIMEOUT_MS"], "8000");
    let prod = f.resolve("production", &none);
    assert_eq!(prod["PUBLIC_API_URL"], "https://nimbus.dev/api");
    assert_eq!(prod["DATABASE_URL"], "");
    // The process environment wins over the file.
    let host = |k: &str| (k == "DATABASE_URL").then(|| "postgres://db/prod".to_string());
    assert_eq!(f.resolve("production", &host)["DATABASE_URL"], "postgres://db/prod");
    // An unknown environment is the shared block.
    assert_eq!(f.resolve("preview", &none)["PUBLIC_API_URL"], "http://localhost/api");
    assert_eq!(f.public("production", &none), vec![("PUBLIC_API_URL".to_string(), "https://nimbus.dev/api".to_string()), ("PUBLIC_SITE_NAME".to_string(), "Nimbus".to_string())]);
}

#[test]
fn b0724_malformed_lines_with_line_numbers() {
    let src = "GOOD=1\nlower=1\nNO_EQUALS\n[bad section\nQUOTED=\"open\n[ok]\nGOOD=2\nGOOD=3\n";
    let d = check_file(src, ".brain_env");
    let got: Vec<(&str, u32)> = d.iter().map(|d| (d.code, d.line)).collect();
    assert_eq!(got, [("B0724", 2), ("B0724", 3), ("B0724", 4), ("B0724", 5), ("B0724", 8)]);
}

#[test]
fn b0720_empty_in_build_env_and_b0723_public_secret() {
    let d = empty_keys(ENV, ".brain_env", "production", &none);
    assert_eq!(d.iter().map(|d| (d.code, d.line)).collect::<Vec<_>>(), [("B0720", 11)]);
    assert!(empty_keys(ENV, ".brain_env", "development", &none).is_empty());
    // Built at runtime so no key-shaped literal lands in the repository (GitHub push protection).
    let secret = format!("PUBLIC_STRIPE={}{}\nPUBLIC_OK=hello\n", "sk_live_", "51HxQabcdefghijklmnopqrstuvwxyz0123");
    let d = check_file(&secret, ".brain_env");
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["B0723"]);
    // No value in any diagnostic.
    for x in d.iter().chain(empty_keys(ENV, ".brain_env", "production", &none).iter()) {
        assert!(!x.msg.contains("sk_live") && !x.fix.contains("sk_live") && !x.msg.contains("postgres"), "{x}");
    }
}

#[test]
fn b0721_used_but_not_declared_and_b0722_declared_but_unused() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("brain_env-{}", std::process::id()));
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::write(root.join(".brain_env"), "PUBLIC_SITE_NAME=Nimbus\nUNUSED_KEY=1\n\n[production]\nPUBLIC_SITE_NAME=Prod\n").unwrap();
    std::fs::write(root.join("app.brain"), "<html lang=\"en\"><head></head><body><slot /></body></html>").unwrap();
    let view = "<meta seo title=\"T\" description=\"D\">\n<script>\nconst name = import.meta.env.PUBLIC_SITE_NAME\nconst api = import.meta.env.PUBLIC_API_URL\n</script>\n<main><h1>{name} {api}</h1></main>";
    std::fs::write(root.join("views/x.brain"), view).unwrap();
    let c = brain_parse::parse(view, Path::new("views/x.brain")).unwrap();
    let (ctx, _): (Ctx, _) = brain_check::load_ctx(&root.join("views/x.brain"), &c);
    let d = brain_check::check(&c, &ctx);
    let b = d.iter().find(|d| d.code == "B0721").expect("B0721");
    assert_eq!(b.file, ".brain_env");
    assert!(b.msg.contains("PUBLIC_API_URL") && b.msg.contains("views/x.brain:4"), "{}", b.msg);
    assert_eq!(b.edits[0].text, "PUBLIC_API_URL=\n");
    assert_eq!(b.edits[0].line, 3);
    let fixed = brain_parse::Diag::apply(&std::fs::read_to_string(root.join(".brain_env")).unwrap(), &b.edits).unwrap();
    assert!(fixed.starts_with("PUBLIC_SITE_NAME=Nimbus\nUNUSED_KEY=1\nPUBLIC_API_URL=\n"), "{fixed}");
    let u = unused_keys(&root);
    assert_eq!(u.iter().map(|d| (d.code, d.msg.contains("UNUSED_KEY"), d.is_info())).collect::<Vec<_>>(), [("B0722", true, true)]);
}

#[test]
fn optional_framework_environment_keys_do_not_become_required_application_secrets() {
    let root=Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("managed-env-{}",std::process::id()));
    std::fs::create_dir_all(&root).unwrap();std::fs::write(root.join(".brain_env"),"PUBLIC_SITE_NAME=Example\n").unwrap();
    let info=brain_check::env::load(&root).unwrap();
    for key in ["NODE_ENV","BRAIN_BACKEND_URL","BRAIN_JWT_SECRET"] {assert!(info.keys.contains(key));}
    assert!(!info.keys.contains("APPLICATION_SECRET"));
    assert!(empty_keys(&info.src,brain_check::env::FILE,"production",&|_|None).is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
