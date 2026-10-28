//! QA #101 (SPEC rules with no check): `--line` only for borders (B0308), no other top-level folders (B0131), no `.env`
//! (B0732).
use std::path::Path;

fn css(style: &str) -> Vec<brain_parse::Diag> {
    let src = format!("<p class=\"a\">x</p>\n<style>{style}</style>\n");
    let c = brain_parse::parse(&src, Path::new("comp/x.brain")).unwrap();
    brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0308").collect()
}

#[test]
fn line_token_only_for_borders() {
    let d = css(".a { background: var(--line); }");
    assert_eq!(d.len(), 1, "{d:#?}");
    assert_eq!(d[0].edits[0].text, "--surface");
    let d = css(".a { color: var(--line); }");
    assert!(d.len() == 1 && d[0].edits.is_empty(), "{d:#?}");
    assert!(css(".a { border: 1px solid var(--line); border-bottom-color: var(--line); outline: 2px solid var(--line); box-shadow: 0 0 0 1px var(--line); }").is_empty());
}

#[test]
fn folders_and_dot_env() {
    let root = std::env::temp_dir().join(format!("brain-layout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (rel, src) in [("app.brain", "<!doctype html>\n<html lang=\"en\">\n<head><title>x</title></head>\n<body><slot /></body>\n</html>\n"), ("foo/x.brain", "<p>x</p>\n"), ("foo/y.brain", "<p>y</p>\n"), ("comp/ok.brain", "<p>ok</p>\n"), (".env", "SECRET_KEY=do-not-print-me\n")] {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
    let d = brain_check::doctor::doctor(&root);
    std::fs::remove_dir_all(&root).ok();
    let folders: Vec<_> = d.iter().filter(|d| d.code == "B0131").collect();
    assert_eq!(folders.len(), 1, "one diagnostic per folder: {d:#?}");
    assert!(folders[0].msg.contains("`foo/`") && folders[0].fix.contains("comp/"), "{:#?}", folders[0]);
    let env: Vec<_> = d.iter().filter(|d| d.code == "B0732").collect();
    assert_eq!(env.len(), 1, "{d:#?}");
    assert!(d.iter().all(|d| !d.msg.contains("do-not-print-me") && !d.fix.contains("do-not-print-me")), "a value leaked: {d:#?}");
}

#[test]
fn kit_layout_spacing_uses_the_scale() {
    let ctx = brain_check::Ctx { tokens: Some(vec![("--space-2".into(), "8px".into()), ("--space-4".into(), "16px".into())]), ..Default::default() };
    let run = |m: &str| {
        let src = format!("{m}\n");
        let c = brain_parse::parse(&src, Path::new("comp/l.brain")).unwrap();
        brain_check::check(&c, &ctx).into_iter().filter(|d| d.code == "B0219" || d.code == "B0203").collect::<Vec<_>>()
    };
    assert!(run("<Stack gap=\"3\"><Row gap=\"0\" justify=\"between\"><p>a</p></Row><Container size=\"lg\" gutter=\"4\"><Spacer /></Container></Stack>").is_empty());
    let d = run("<Stack gap=\"20px\"><p>a</p></Stack>");
    assert!(d.len() == 1 && d[0].edits.is_empty() && d[0].fix.contains("0–6"), "{d:#?}");
    let d = run("<Grid gap=\"16px\"><p>a</p></Grid>");
    assert_eq!(d[0].edits[0].text, "4", "{d:#?}");
    let d = run("<Row gap=\"var(--space-2)\"><p>a</p></Row>");
    assert_eq!(d[0].edits[0].text, "2", "{d:#?}");
    let d = run("<Container size=\"huge\"><p>a</p></Container>");
    assert_eq!(d.len(), 1, "{d:#?}");
}

#[test]
fn kit_widgets_keep_their_state_wherever_they_are_embedded() {
    let table = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/ui/table.brain")).unwrap();
    let c = brain_parse::parse(&table, Path::new("ui/table.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.code == "B0116").collect();
    assert!(d.is_empty(), "{d:#?}");
    // The author's own ui/ still gets B0116, even a copy of the kit that was edited.
    let edited = table.replacen("<meta", "<meta data-x=\"1\"", 0) + "\n";
    let c = brain_parse::parse(&edited, Path::new("ui/table.brain")).unwrap();
    assert!(brain_check::check(&c, &brain_check::Ctx::default()).iter().any(|d| d.code == "B0116"));
}

#[test]
fn contrast_on_the_resolved_cascade() {
    let tokens = vec![("--accent".to_string(), "#0071e3".to_string()), ("--on-accent".to_string(), "#ffffff".to_string()), ("--muted".to_string(), "#6e6e73".to_string()), ("--fg".to_string(), "#1a1a1a".to_string()), ("--bg".to_string(), "#ffffff".to_string())];
    let ctx = brain_check::Ctx { tokens: Some(tokens), ..Default::default() };
    let run = |css: &str| {
        let src = format!("<div class=\"card\"><p class=\"badge\">New</p><p>Body</p></div>\n<style>{css}</style>\n");
        let c = brain_parse::parse(&src, Path::new("comp/card.brain")).unwrap();
        brain_check::check(&c, &ctx).into_iter().filter(|d| d.code == "B0408").collect::<Vec<_>>()
    };
    // `.card p` (0,1,1) beats `.badge` (0,1,0): muted text on the accent fill.
    let d = run(".badge { background: var(--accent); color: var(--on-accent); } .card p { color: var(--muted); }");
    assert_eq!(d.len(), 1, "{d:#?}");
    assert!(d[0].msg.contains("`.card p`") && d[0].msg.contains("`.badge`"), "{}", d[0].msg);
    // A more specific badge rule keeps its own color: no report.
    assert!(run(".card .badge { background: var(--accent); color: var(--on-accent); } .card p { color: var(--muted); }").is_empty());
    // Plain body text on the page background is fine.
    assert!(run(".card { background: var(--bg); } .card p { color: var(--fg); }").is_empty());
}
