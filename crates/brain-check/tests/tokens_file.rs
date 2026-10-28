//! `load_ctx` finds `tokens.cssbrain` (SPEC §10) and still accepts the legacy `tokens.brain.css`.
use std::path::Path;

fn codes_in(token_file: &str) -> Vec<&'static str> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("tok-{}", token_file.replace('.', "-")));
    std::fs::create_dir_all(dir.join("comp")).unwrap();
    std::fs::write(dir.join(token_file), ":root { --space-2: 8px; }\n").unwrap();
    let src = "<p>x</p>\n<style>p { padding: var(--nope) }</style>\n";
    let c = brain_parse::parse(src, Path::new("comp/x.brain")).unwrap();
    let (ctx, deps) = brain_check::load_ctx(&dir.join("comp/x.brain"), &c);
    assert!(deps.contains(&dir.join(token_file)));
    assert!(deps.contains(&dir.join("comp/tokens.cssbrain")), "negative dependencies detect a new nearer token file");
    brain_check::check(&c, &ctx).into_iter().map(|d| d.code).collect()
}

#[test]
fn cssbrain_and_legacy_token_files() {
    assert_eq!(codes_in("tokens.cssbrain"), ["B0302"]);
    assert_eq!(codes_in("tokens.brain.css"), ["B0302"]);
}
