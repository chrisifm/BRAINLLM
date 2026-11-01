//! INTERNALS §20 in `brain build` (#47): the native linter runs on the whole app; errors stop the build with a
//! summary, warnings are listed with their count, and `brain.json "lint"` lowers or turns off quality rules only.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn build_lists_warnings_and_stops_on_errors_with_a_summary() {
    let tmp = std::env::temp_dir().join(format!("bblint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    // a warning (B0130) in a component nobody imports still shows up: the build lints the app, like `brain check`
    std::fs::write(dir.join("comp/warn.brain"), "<script>\n  let el = ref()\n  mounted(() => { log(el.v) })\n</script>\n\n<p bind:ref={el}>Warn</p>\n").unwrap();
    let o = brain(&dir, &["build"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    let warning_lines: Vec<_> = t.lines().filter(|line| line.starts_with("warning: ")).collect();
    assert!(warning_lines.iter().any(|line| line.starts_with("warning: B0130 comp/warn.brain:3:")), "{t}");
    assert_eq!(warning_lines.iter().filter(|line| line.starts_with("warning: B") && !line.starts_with("warning: B0736")).count(), 1, "one source diagnostic (B0736: the scaffold's placeholder production origin); production cleanup feedback is a separate, explicit build notice: {t}");
    assert!(t.contains(&format!("{} warning", warning_lines.len())), "summary includes both diagnostics and production feedback: {t}");
    // an error anywhere stops it: every error with file:line:col and fix, then the counts
    std::fs::write(dir.join("comp/bad.brain"), "<script>\n  let x = \n</script>\n<p>{x}</p>\n").unwrap();
    let o = brain(&dir, &["build"]);
    let t = text(&o);
    assert!(!o.status.success());
    assert!(t.contains("B0002 comp/bad.brain:3:1") && t.contains("; fix: ") && t.contains("build stopped: 1 error(s), 1 warning(s)"), "{t}");
    std::fs::remove_file(dir.join("comp/bad.brain")).unwrap();
    // "lint": a quality rule can be turned off; an override outside B09–B13 is ignored
    let cfg = std::fs::read_to_string(dir.join("brain.json")).unwrap();
    let with = |lint: &str| cfg.trim_end().trim_end_matches('}').trim_end().to_string() + &format!(",\n  \"lint\": {lint}\n}}\n");
    std::fs::write(dir.join("comp/loop.brain"), "<script>\n  let n = sig(0)\n  fx(() => { n = 1 })\n</script>\n<p>{n}</p>\n").unwrap();
    let base = text(&brain(&dir, &["check", "--json"]));
    let code = ["B1004", "B0901", "B1304"].into_iter().find(|c| base.contains(c));
    if let Some(code) = code {
        std::fs::write(dir.join("brain.json"), with(&format!("{{\"{code}\": \"off\", \"B0130\": \"off\"}}"))).unwrap();
        let t = text(&brain(&dir, &["check", "--json"]));
        assert!(!t.contains(code), "{code} turned off: {t}");
        assert!(t.contains("B0130"), "B0130 is not a quality rule: the override is ignored: {t}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn error_views_without_a_title_still_say_what_happened_and_stay_out_of_the_index() {
    let tmp = std::env::temp_dir().join(format!("bberrtitle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    std::fs::write(dir.join("views/_error.brain"), "<main role=\"alert\"><h1>Oops</h1></main>\n").unwrap();
    std::fs::write(dir.join("views/loaded.brain"), "<meta seo title=\"Loaded\" description=\"A view with a load.\">\n<script>\nexport async function load() { return {} }\n</script>\n<main><h1>Loaded</h1></main>\n").unwrap();
    std::fs::write(dir.join("views/410.brain"), "<meta seo title=\"Sold out\" description=\"This item was sold and will not come back.\" robots=\"index\">\n<main><h1>Gone</h1></main>\n").unwrap();
    let o = brain(&dir, &["build"]);
    assert!(o.status.success(), "{}", text(&o));
    let mut templates: Vec<String> = Vec::new();
    let mut pending = vec![dir.join("dist")];
    while let Some(folder) = pending.pop() {
        for path in std::fs::read_dir(folder).unwrap().map(|e| e.unwrap().path()) {
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|x| x == "html") {
                templates.push(std::fs::read_to_string(path).unwrap());
            }
        }
    }
    let error = templates.iter().find(|t| t.contains("<h1>Oops") || t.contains("<title>Something went wrong"));
    let error = error.unwrap_or_else(|| panic!("no error template in {} pages", templates.len()));
    assert!(error.contains("<title>Something went wrong"), "{error}");
    assert!(error.contains("name=\"robots\" content=\"noindex\""), "{error}");
    // what the view names wins
    let sold = templates.iter().find(|t| t.contains("<title>Sold out")).expect("the view's own title");
    assert!(!sold.contains("noindex"), "{sold}");
    // an ordinary view is left alone
    assert!(templates.iter().any(|t| t.contains("<title>Loaded") && !t.contains("noindex")));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn check_reports_stale_links_by_source_line_and_titles_that_repeat_the_site_name() {
    let tmp = std::env::temp_dir().join(format!("bbstale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    let clean = text(&brain(&dir, &["check"]));
    assert!(!clean.contains("B0776") && !clean.contains("B0777"), "a new app has neither: {clean}");
    std::fs::write(dir.join("views/users.brain"), "<meta seo title=\"Users\" description=\"Everyone here.\">\n<main><h1>Users</h1></main>\n").unwrap();
    std::fs::create_dir_all(dir.join("views/items")).unwrap();
    std::fs::write(dir.join("views/items/[id].brain"), "<meta seo title=\"Item\" description=\"One item here.\">\n<main><h1>Item</h1></main>\n").unwrap();
    std::fs::write(dir.join("public/file.pdf"), "x").unwrap();
    std::fs::write(
        dir.join("comp/nav.brain"),
        "<nav>\n  <a href=\"/gone\">A</a> <a href=\"/also-gone\">B</a> <a href=\"/gone\">C</a>\n  <a href=\"/users\">U</a> <a href=\"/users/\">U</a> <a href=\"/items/7?x=1#top\">I</a>\n  <a href=\"/file.pdf\">F</a> <a href=\"/feed.xml\">X</a> <a href=\"https://example.com/gone\">E</a> <a href=\"#here\">H</a> <a href=\"/api/export\">Z</a>\n  <a href=\"/third\">D</a>\n</nav>\n",
    )
    .unwrap();
    let t = text(&brain(&dir, &["check"]));
    assert_eq!(t.matches("B0776").count(), 2, "one finding per source line: {t}");
    assert!(t.contains("comp/nav.brain:2:1 warning B0776") && t.contains("`/gone`, `/also-gone`"), "{t}");
    assert!(t.contains("comp/nav.brain:5:1 warning B0776") && t.contains("`/third`"), "{t}");
    let app = std::fs::read_to_string(dir.join("app.brain")).unwrap();
    let suffix = app.split("titleTemplate=\"%s").nth(1).and_then(|r| r.split('"').next()).expect("the new app has a titleTemplate").trim_start_matches([' ', '·', '-', '|']).to_string();
    std::fs::write(dir.join("views/users.brain"), format!("<meta seo title=\"Users · {suffix}\" description=\"Everyone here.\">\n<main><h1>Users</h1></main>\n")).unwrap();
    let t = text(&brain(&dir, &["check"]));
    assert!(t.contains("views/users.brain:1:1 warning B0777"), "{t}");
    let _ = std::fs::remove_dir_all(&tmp);
}


/// `brain doctor` and `brain build` must agree on the site native sockets use: a placeholder origin is "no site" for
/// SEO, never for sockets, whose exact allow-list is the author's explicit declaration.
#[test]
fn native_sockets_use_the_declared_site_even_when_it_is_a_placeholder_for_seo() {
    let tmp = std::env::temp_dir().join(format!("bsocketsite-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    let cfg = std::fs::read_to_string(dir.join("brain.json")).unwrap();
    let with = |allowed: &str| cfg.trim_end().trim_end_matches('}').trim_end().to_string() + &format!(",\n  \"backend\": {{ \"origins\": [\"{allowed}\"], \"sockets\": {{ \"allowedOrigins\": [\"{allowed}\"] }} }}\n}}\n");
    let env = std::fs::read_to_string(dir.join(".brain_env")).unwrap().replace("PUBLIC_SITE_URL=https://example.com", "PUBLIC_SITE_URL=http://127.0.0.1:5280");
    std::fs::write(dir.join(".brain_env"), env).unwrap();
    std::fs::write(dir.join("brain.json"), with("http://127.0.0.1:5280")).unwrap();
    let o = brain(&dir, &["build"]);
    let t = text(&o);
    assert!(o.status.success(), "the declared origin is listed: {t}");
    assert!(t.contains("B0736"), "SEO still treats it as no public site: {t}");
    // not listed: the refusal names the value and the list instead of saying the site is missing
    std::fs::write(dir.join("brain.json"), with("https://forum.example.org")).unwrap();
    let o = brain(&dir, &["build"]);
    let t = text(&o);
    assert!(!o.status.success(), "{t}");
    assert!(t.contains("B0745") && t.contains("http://127.0.0.1:5280") && t.contains("https://forum.example.org"), "{t}");
    assert!(!t.contains("require PUBLIC_SITE_URL"), "the value is set: {t}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn add_channel_session_matches_the_session_store_the_guides_use() {
    let tmp = std::env::temp_dir().join(format!("bchansession-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    std::fs::remove_file(dir.join("channels/session.jsbrain")).unwrap();
    assert!(brain(&dir, &["add", "channel", "session"]).status.success());
    let made = std::fs::read_to_string(dir.join("channels/session.jsbrain")).unwrap();
    assert!(made.contains("store({ user: null })"), "{made}");
    // any other name, and an explicit flag, keep their meaning
    assert!(brain(&dir, &["add", "channel", "theme"]).status.success());
    assert!(std::fs::read_to_string(dir.join("channels/theme.jsbrain")).unwrap().contains("key('theme')"));
    let _ = std::fs::remove_dir_all(&tmp);
}
