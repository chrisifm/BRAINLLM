//! Languages in the CLI (§37): `brain add locale`, `brain i18n --missing`, and dev serving `/<locale>/…` with its
//! dictionary and `<html lang dir>`.
#![cfg(unix)]
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn add_locale_missing_and_dev_routes() {
    let tmp = std::env::temp_dir().join(format!("bi18n-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    let run = |dir: &std::path::Path, args: &[&str]| Command::new(bin).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    assert!(run(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    let original_config = std::fs::read(dir.join("brain.json")).unwrap();
    std::fs::create_dir_all(dir.join(".brain")).unwrap();
    let lock=std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(".brain/edit.lock")).unwrap();
    lock.try_lock().unwrap();
    let denied=run(&dir, &["add", "locale", "es"]);
    assert!(!denied.status.success(), "locale generation must respect another writer");
    assert_eq!(std::fs::read(dir.join("brain.json")).unwrap(), original_config);
    assert!(!dir.join("i18n/es.json").exists());
    lock.unlock().unwrap();
    drop(lock);
    let added=run(&dir, &["add", "locale", "es"]);
    assert!(added.status.success(), "{}{}", String::from_utf8_lossy(&added.stdout), String::from_utf8_lossy(&added.stderr));
    // QA #280: the first extra language moves the templates' fixed text to keys, so the new app stays without
    // errors or B1403; es.json starts as the copy to translate (B1402 only)
    let chk = String::from_utf8_lossy(&run(&dir, &["check", "--json"]).stdout).to_string();
    assert!(!chk.contains("\"level\":\"error\"") && !chk.contains("B1403"), "{chk}");
    let en = std::fs::read_to_string(dir.join("i18n/en.json")).unwrap();
    assert!(en.contains("\"Page not found\"") && std::fs::read_to_string(dir.join("i18n/es.json")).unwrap() == en);
    assert!(std::fs::read_to_string(dir.join("views/404.brain")).unwrap().contains("{t('404."));
    // the scaffold's tests still pass: brain test loads the default language's texts
    let t = run(&dir, &["test"]);
    let tout = String::from_utf8_lossy(&t.stdout).to_string() + &String::from_utf8_lossy(&t.stderr);
    assert!(t.status.success() || tout.contains("Chrome not found"), "{tout}");
    assert!(run(&dir, &["add", "locale", "ar"]).status.success());
    assert!(std::fs::read_to_string(dir.join("brain.json")).unwrap().contains("\"locales\": [\"en\", \"es\", \"ar\"]"));
    assert!(!run(&dir, &["add", "locale", "es"]).status.success(), "twice is refused");
    // --missing: es loses one key and translates another; ar keeps the copy (everything untranslated)
    let en = std::fs::read_to_string(dir.join("i18n/en.json")).unwrap();
    let line = |k: &str| en.lines().find(|l| l.trim_start().starts_with(&format!("\"{k}\""))).unwrap().to_string();
    let es = en.replace(&line("404.page_not_found"), "").replace(&line("about.home_page"), &line("about.home_page").replace("\"home page\"", "\"HOME-ES\""));
    let es = es.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n").replace(",\n}", "\n}").replace(",,", ",") + "\n";
    std::fs::write(dir.join("i18n/es.json"), &es).unwrap();
    let o = run(&dir, &["i18n", "--missing", "--json"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(!o.status.success() && out.contains("\"es\":{\"missing\":[\"404.page_not_found\"]") && out.contains("\"untranslated\":[\"403."), "{out}");
    // back to a complete es.json (one key translated) for the dev server
    std::fs::write(dir.join("i18n/es.json"), en.replace("\"home page\"", "\"HOME-ES\"")).unwrap();
    // dev: /es/about is /about in Spanish, with the dictionary and the page language
    let mut c = Command::new(bin).args(["run", "--port", "0"]).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    let mut err = BufReader::new(c.stderr.take().unwrap());
    let port: u16 = loop {
        let mut l = String::new();
        assert!(err.read_line(&mut l).unwrap() > 0, "brain run printed no address");
        if let Some(p) = l.split("127.0.0.1:").nth(1).and_then(|r| r.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|p| p.parse().ok()) {
            break p;
        }
    };
    let get = |p: &str| {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        s.write_all(format!("GET {p} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        let mut b = String::new();
        let _ = s.read_to_string(&mut b);
        b
    };
    let es = get("/es/about");
    assert!(es.starts_with("HTTP/1.1 200") && es.contains("lang=\"es\" dir=\"ltr\" data-default-lang=\"en\"") && es.contains("\"about.home_page\":\"HOME-ES\""), "{es}");
    assert!(get("/ar/about").contains("lang=\"ar\" dir=\"rtl\""));
    assert!(get("/about").contains("lang=\"en\""));
    let _ = c.kill();
    let _ = c.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 1 → 2 languages is `brain add locale` alone: afterwards no fixed text is left (B1403), every key appears once in
/// each locale file, keys are scoped by view, and the `<meta seo>` texts are keys too.
#[test]
fn add_locale_moves_every_text_once() {
    let tmp = std::env::temp_dir().join(format!("bi18n-once-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    let run = |dir: &std::path::Path, args: &[&str]| Command::new(bin).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    assert!(run(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    // Two components whose keys share a scope (`tag`): the same text must be inserted once, not twice.
    for d in ["comp", "widgets"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
        std::fs::write(dir.join(d).join("tag.brain"), "<a href=\"/posts\" title=\"Open the list\">Read more</a>\n").unwrap();
    }
    let added = run(&dir, &["add", "locale", "es"]);
    assert!(added.status.success(), "{}{}", String::from_utf8_lossy(&added.stdout), String::from_utf8_lossy(&added.stderr));
    let chk = String::from_utf8_lossy(&run(&dir, &["check", "--json"]).stdout).to_string();
    assert!(!chk.contains("\"level\":\"error\"") && !chk.contains("B1403"), "{chk}");
    let en = std::fs::read_to_string(dir.join("i18n/en.json")).unwrap();
    let keys: Vec<&str> = en.lines().filter_map(|l| l.trim_start().strip_prefix('"').and_then(|r| r.split('"').next())).collect();
    let mut uniq = keys.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(keys.len(), uniq.len(), "duplicate keys in en.json:\n{en}");
    assert!(keys.iter().all(|k| k.is_ascii()), "{en}");
    assert!(keys.contains(&"tag.read_more") && keys.contains(&"tag.open_the_list"), "{en}");
    assert!(!keys.iter().any(|k| k.starts_with("index.")), "views/*/index.brain keys are scoped by their folder: {en}");
    assert!(keys.iter().any(|k| k.starts_with("account.")) && keys.iter().any(|k| k.starts_with("posts.")), "{en}");
    let about = std::fs::read_to_string(dir.join("views/about.brain")).unwrap();
    assert!(about.contains("title={t('about.meta_title')}") && about.contains("description={t('about.meta_description')}"), "{about}");
    assert_eq!(std::fs::read_to_string(dir.join("i18n/es.json")).unwrap(), en);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Adding English to a site whose default is another language: the framework's own keys (`form.*`, `error_state.*`)
/// get their built-in English texts instead of a copy of the default language; the app's keys are copied to translate.
#[test]
fn add_locale_en_fills_framework_texts_in_english() {
    let dir = std::env::temp_dir().join(format!("bi18n-en-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("views")).unwrap();
    std::fs::create_dir_all(dir.join("i18n")).unwrap();
    std::fs::write(dir.join("brain.json"), "{\n  \"i18n\": { \"default\": \"es\", \"locales\": [\"es\"] }\n}\n").unwrap();
    std::fs::write(dir.join("app.brain"), "<html lang=\"es\"><head><meta charset=\"utf-8\"></head><body><slot /></body></html>\n").unwrap();
    std::fs::write(dir.join("views/index.brain"), "<meta seo title={t('home.title')} description={t('home.title')}>\n<main><h1>{t('home.title')}</h1></main>\n").unwrap();
    // Tagged English stand-ins for the default language (the repository is English only).
    let es = "{\n  \"home.title\": \"Hello [es]\",\n  \"form.required\": \"Required [es]\",\n  \"form.min_chars\": \"At least {n} chars [es]\",\n  \"error_state.retry\": \"Retry [es]\",\n  \"form.custom\": \"Custom [es]\"\n}\n";
    std::fs::write(dir.join("i18n/es.json"), es).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    let o = Command::new(bin).args(["add", "locale", "en"]).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    assert!(o.status.success(), "{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let en = std::fs::read_to_string(dir.join("i18n/en.json")).unwrap();
    for want in [
        "\"home.title\": \"Hello [es]\"",
        "\"form.required\": \"Required\"",
        "\"form.min_chars\": { \"one\": \"At least {n} character\", \"other\": \"At least {n} characters\" }",
        "\"error_state.retry\": \"Retry\"",
        "\"form.custom\": \"Custom [es]\"",
    ] {
        assert!(en.contains(want), "missing {want} in:\n{en}");
    }
    assert_eq!(std::fs::read_to_string(dir.join("i18n/es.json")).unwrap(), es, "the default file is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}
