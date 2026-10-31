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

#[test]
fn optional_keys_are_tolerated_and_the_winning_line_decides() {
    let src = "CALL_LIMIT?=\nGATEWAY_KEY?=abc\nREQUIRED=\nSTRICT_LATER?=\nPUBLIC_NOTE?=\n\n[production]\nSTRICT_LATER=\n";
    let f = parse(src);
    assert!(f.errors.is_empty(), "{:?}", f.errors);
    let names = |e: &str| empty_keys(src, ".brain_env", e, &none).into_iter().map(|d| d.msg).collect::<Vec<_>>().join("|");
    let dev = names("development");
    assert!(dev.contains("REQUIRED") && !dev.contains("CALL_LIMIT") && !dev.contains("STRICT_LATER"), "{dev}");
    assert!(names("production").contains("STRICT_LATER"), "a section line overrides the shared optional one");
    let run = f.runtime("development", &none);
    assert!(!run.contains_key("CALL_LIMIT"), "an empty optional key reads as unset");
    assert_eq!(run["GATEWAY_KEY"], "abc");
    assert!(run.contains_key("REQUIRED"), "negative: a required empty key stays");
    let public = f.public("development", &none);
    assert!(public.iter().any(|(k, v)| k == "PUBLIC_NOTE" && v == brain_check::env::UNSET), "{public:?}");
}

#[test]
fn reads_with_a_default_or_a_condition_are_guarded_and_plain_reads_are_not() {
    let src = "a(process.env.A1 || 1); b(process.env.A2 ?? 1); if (process.env.A3) c(); d(ctx.env.A4 ? 1 : 0); e(process.env['A5'] || 1); f(process.env.PLAIN); g(process.env.CMP === 'x'); h(ctx.env.PLAIN2);";
    let got: std::collections::BTreeMap<String, bool> = brain_check::env::reads(src).into_iter().map(|r| (r.key, r.guarded)).collect();
    for k in ["A1", "A2", "A3", "A4", "A5"] {
        assert_eq!(got.get(k), Some(&true), "{k}: {got:?}");
    }
    for k in ["PLAIN", "PLAIN2"] {
        assert_eq!(got.get(k), Some(&false), "{k}: {got:?}");
    }
}

#[test]
fn env_reads_come_from_tokens_not_from_text() {
    let keys = |src: &str| brain_check::env::reads(src).into_iter().map(|r| r.key).collect::<Vec<_>>();
    let none: Vec<String> = Vec::new();
    for src in [
        "// process.env.IN_LINE_COMMENT\nconst a = 1\n",
        "/* ctx.env.IN_BLOCK_COMMENT */\n",
        "const a = 'import.meta.env.IN_STRING'\n",
        "const a = \"process.env.IN_STRING\"\n",
        "const a = `set process.env.IN_TEMPLATE first`\n",
        "const a = /process.env.IN_REGEX/\n",
        "const a = myctx.env.NOT_CTX\n",
        "const a = my.process.env.NOT_PROCESS\n",
        "const a = process.environment.NOT_ENV\n",
    ] {
        assert_eq!(keys(src), none, "{src}");
    }
    assert_eq!(keys("const a = `v ${process.env.IN_EXPRESSION} w`\n"), ["IN_EXPRESSION"]);
    assert_eq!(keys("const a = process\n  .env\n  .SPLIT_LINES\n"), ["SPLIT_LINES"]);
    assert_eq!(keys("const a = ctx.env['BRACKET_KEY']\nconst b = ctx.env?.OPTIONAL_KEY\n"), ["BRACKET_KEY", "OPTIONAL_KEY"]);
    assert_eq!(keys("<meta server>\nexport default () => import.meta.env.AFTER_META\n"), ["AFTER_META"]);
    // a file the lexer cannot read to the end still reports what it reads
    assert_eq!(keys("#!/usr/bin/env node\nconst a = process.env.AFTER_SHEBANG\n"), ["AFTER_SHEBANG"]);
    // the byte offset is that of the read
    let src = "// process.env.NO\nconst a = process.env.YES\n";
    let read = brain_check::env::reads(src).remove(0);
    assert_eq!(&src[read.at..read.at + 7], "process");
}

#[test]
fn env_reads_in_markup_only_see_scripts_and_expressions() {
    let keys = |src: &str| brain_check::env::reads_in(src, true).into_iter().map(|r| r.key).collect::<Vec<_>>();
    let none: Vec<String> = Vec::new();
    for src in [
        "<p>set process.env.IN_PROSE in the host</p>\n",
        "<!-- ctx.env.IN_HTML_COMMENT -->\n<p>x</p>\n",
        "<a title=\"process.env.IN_ATTR\">x</a>\n",
        "<style>.a::after { content: 'process.env.IN_CSS' }</style>\n<p>x</p>\n",
        "<script>\n  // ctx.env.IN_SCRIPT_COMMENT\n</script>\n<p>{'ctx.env.IN_VALUE_STRING'}</p>\n",
    ] {
        assert_eq!(keys(src), none, "{src}");
    }
    assert_eq!(keys("<script>\n  const a = process.env.IN_SCRIPT\n</script>\n<p>{ctx.env.IN_VALUE}</p>\n<a title=\"x {ctx.env.IN_MIXED}\">y</a>\n{#if ctx.env.IN_BLOCK}z{/if}\n"), ["IN_SCRIPT", "IN_VALUE", "IN_MIXED", "IN_BLOCK"]);
}

#[test]
fn a_guard_in_another_statement_counts() {
    let guarded = |src: &str| brain_check::env::reads(src).remove(0).guarded;
    for src in [
        "const key = ctx.env.API_KEY\nif (!key) return null\n",
        "const key = ctx.env.API_KEY\nif (key) use(key)\n",
        "const key = process.env.API_KEY;\nreturn key || 'none'\n",
        "let key = process.env.API_KEY\nconst a = key ?? 'none'\n",
        "const key = Number(process.env.LIMIT)\nreturn key || 10\n",
    ] {
        assert!(guarded(src), "{src}");
    }
    for src in [
        "const key = ctx.env.API_KEY\nreturn send(key)\n",
        "const key = ctx.env.API_KEY\nconst other = 1\nif (other) use(key)\n",
        "const key = ctx.env.API_KEY + 'x'\nif (key) use(key)\n",
    ] {
        assert!(!guarded(src), "{src}");
    }
}
