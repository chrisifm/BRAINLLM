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
    assert!(run(&dir, &["add", "locale", "es"]).status.success());
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
