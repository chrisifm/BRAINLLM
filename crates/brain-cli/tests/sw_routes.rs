//! INTERNALS §35.8: `brain.json` `cache.routes` (declarative service worker data strategies) and `cache.update:
//! "prompt"` reach dist/sw.js; invalid entries stop the build with a fix; `updateReady()`/`applyUpdate()` are
//! auto-imported from brain-update.js only by the views that use them.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// Adds keys to the scaffold's `"cache": { … }` object.
fn with_cache(cfg: &str, extra: &str) -> String {
    let out = cfg.replacen("\"precacheKB\": 2048 }", &format!("\"precacheKB\": 2048, {extra} }}"), 1);
    assert_ne!(out, cfg, "scaffold cache object changed: {cfg}");
    out
}

#[test]
fn routes_and_update_prompt_reach_the_worker() {
    let tmp = std::env::temp_dir().join(format!("bswroutes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    let cfg = std::fs::read_to_string(dir.join("brain.json")).unwrap();

    std::fs::write(dir.join("brain.json"), with_cache(&cfg, r#""routes": [{ "match": "/api/products", "strategy": "fastest" }]"#)).unwrap();
    let o = brain(&dir, &["build"]);
    assert!(!o.status.success() && text(&o).contains("stale-while-revalidate"), "{}", text(&o));

    std::fs::write(
        dir.join("brain.json"),
        with_cache(&cfg, r#""update": "prompt", "routes": [{ "match": "/api/products/*", "strategy": "stale-while-revalidate", "maxEntries": 50, "maxAge": "1h" }, { "match": "/api/session", "strategy": "network-only" }]"#),
    )
    .unwrap();
    let about = std::fs::read_to_string(dir.join("views/about.brain")).unwrap();
    let banner = "<main class=\"page\">\n  {#if updateReady()}<p role=\"status\">A new version is ready. <button onclick={() => applyUpdate()}>Reload</button></p>{/if}";
    std::fs::write(dir.join("views/about.brain"), about.replacen("<main class=\"page\">", banner, 1)).unwrap();
    let o = brain(&dir, &["build"]);
    assert!(o.status.success(), "{}", text(&o));
    let sw = std::fs::read_to_string(dir.join("dist/sw.js")).unwrap();
    assert!(sw.contains("routes:[{match:\"/api/products/\",prefix:true,strategy:\"stale-while-revalidate\",maxEntries:50,maxAge:3600,timeout:3000},{match:\"/api/session\",prefix:false,strategy:\"network-only\""), "{sw}");
    assert!(sw.contains("update:\"prompt\"") && sw.contains("brain:skip-waiting"), "{sw}");
    let chunks: Vec<String> = std::fs::read_dir(dir.join("dist/_b")).unwrap().filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok()).collect();
    assert!(chunks.iter().any(|c| c.contains("brain:skip-waiting")), "brain-update.js is emitted for the view that uses it");
    let _ = std::fs::remove_dir_all(&tmp);
}
