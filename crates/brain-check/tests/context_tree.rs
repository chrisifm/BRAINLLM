//! #195: `use(K)` needs a `provide(K, …)` above it on EVERY render path, not just somewhere in the app.

use std::path::{Path, PathBuf};

fn app(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ctx-tree-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (p, s) in files {
        let f = root.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    root
}

fn codes(root: &Path, rel: &str) -> Vec<&'static str> {
    let src = std::fs::read_to_string(root.join(rel)).unwrap();
    let c = brain_parse::parse(&src, Path::new(rel)).unwrap();
    let (ctx, _) = brain_check::load_ctx(&root.join(rel), &c);
    brain_check::check(&c, &ctx).into_iter().map(|d| d.code).collect()
}

const KEY: &str = "export const Theme = key('theme')\n";
const CHILD: &str = "<script>\nimport { Theme } from '../channels/theme.jsbrain'\nlet theme = use(Theme)\n</script>\n<p>{theme}</p>";
const SEO: &str = "<meta seo title=\"T\" description=\"D\">\n";

#[test]
fn use_without_provider_on_one_path_is_b0807() {
    let with = format!("{SEO}<script>\nimport {{ Theme }} from '../channels/theme.jsbrain'\nimport Child from '../comp/child.brain'\nlet t = sig('dark')\nprovide(Theme, t)\n</script>\n<main><h1>A</h1><Child /></main>");
    let without = format!("{SEO}<script>\nimport Child from '../comp/child.brain'\n</script>\n<main><h1>B</h1><Child /></main>");
    let shell = "<html lang=\"en\"><head></head><body><slot /></body></html>";
    // Provided on the only path: fine.
    let ok = app("ok", &[("app.brain", shell), ("channels/theme.jsbrain", KEY), ("comp/child.brain", CHILD), ("views/a.brain", &with)]);
    assert!(!codes(&ok, "comp/child.brain").contains(&"B0807"));
    // A second view renders the child without a provider: some path has no Theme.
    let bad = app("bad", &[("app.brain", shell), ("channels/theme.jsbrain", KEY), ("comp/child.brain", CHILD), ("views/a.brain", &with), ("views/b.brain", &without)]);
    assert!(codes(&bad, "comp/child.brain").contains(&"B0807"));
    // Providing it in app.brain covers every view.
    let shell_provides = "<script>\nimport { Theme } from './channels/theme.jsbrain'\nlet t = sig('dark')\nprovide(Theme, t)\n</script>\n<html lang=\"en\"><head></head><body><slot /></body></html>";
    let top = app("top", &[("app.brain", shell_provides), ("channels/theme.jsbrain", KEY), ("comp/child.brain", CHILD), ("views/b.brain", &without)]);
    assert!(!codes(&top, "comp/child.brain").contains(&"B0807"));
}
