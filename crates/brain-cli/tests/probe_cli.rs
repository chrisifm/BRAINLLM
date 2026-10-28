//! #284 through the CLI (INTERNALS §30.2): `brain build` passes RUNTIME's runaway probe (shim/probe.js) to the
//! bundler, so a list that grows with no input fails the build with B0907 naming the view; a bounded clock builds.
use std::process::{Command, Output, Stdio};

fn brain(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn runaway_list_fails_brain_build_and_a_clock_does_not() {
    if Command::new("node").arg("-v").output().is_err() {
        eprintln!("skipped: no Node");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("bprobe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let dir = tmp.join("app");
    let clock = "<meta seo title=\"Clock\" description=\"A clock that ticks once a second\">\n<script>\n  let n = sig(0)\n  fx(() => {\n    const id = setInterval(() => { n++ }, 1000)\n    return () => clearInterval(id)\n  })\n</script>\n<main><h1>Clock</h1><p>{n}</p></main>\n";
    std::fs::write(dir.join("views/clock.brain"), clock).unwrap();
    let o = brain(&dir, &["build"]);
    assert!(o.status.success(), "{}", text(&o));
    let grow = "<meta seo title=\"Grow\" description=\"A list that grows with no input\">\n<script>\n  let rows = sig([])\n  const grow = list => [...list, list.length]\n  fx(() => {\n    const id = setInterval(() => { rows = grow(rows) }, 100)\n    return () => clearInterval(id)\n  })\n</script>\n<main><h1>Grow</h1><ul>{#each rows as r (r)}<li>{r}</li>{/each}</ul></main>\n";
    std::fs::write(dir.join("views/grow.brain"), grow).unwrap();
    let o = brain(&dir, &["build"]);
    let t = text(&o);
    assert!(!o.status.success(), "{t}");
    assert!(t.contains("B0907") && t.contains("views/grow.brain"), "{t}");
    let _ = std::fs::remove_dir_all(&tmp);
}
