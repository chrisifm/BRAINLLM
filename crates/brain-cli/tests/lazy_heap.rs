//! `brain test --heap` after a lazy child in every mode (visible, idle, interaction, eager) and a `visible(ref)` signal:
//! nothing stays alive once the parent unmounts. The visible mode used to leak parent and child: Chrome keeps an
//! IntersectionObserver that delivered entries, and its callback closed over the instance.
#![cfg(unix)]
use std::process::{Command, Stdio};

#[test]
fn lazy_children_and_visible_signals_release_their_instances() {
    let tmp = std::env::temp_dir().join(format!("blazyheap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "app"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("app/comp/lz");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("child.brain"), "<p>child</p>\n").unwrap();
    let modes = [("visible", " lazy"), ("idle", " lazy=\"idle\""), ("interaction", " lazy=\"interaction\""), ("eager", "")];
    for (name, attr) in modes {
        std::fs::write(dir.join(format!("{name}.brain")), format!("<script>\n  import Child from './child.brain'\n</script>\n<section><Child{attr} /></section>\n")).unwrap();
    }
    std::fs::write(
        dir.join("seen.brain"),
        "<script>\n  const box = ref()\n  const seen = visible(box)\n</script>\n<div bind:ref={box}>{seen.v ? 'in view' : 'away'}</div>\n",
    )
    .unwrap();
    for name in ["visible", "idle", "interaction", "eager", "seen"] {
        let comp = format!("{}{}", name[..1].to_uppercase(), &name[1..]);
        std::fs::write(
            dir.join(format!("{name}.test.brain")),
            // every mode builds its child before the unmount, so the leak check covers the built component too
            format!(
                "<script>\n  import {comp} from './{name}.brain'\n\n  test('mounts and unmounts', async () => {{\n    await mount({comp}, {{}})\n{}    await waitFor(() => count('{}') > 0)\n  }})\n</script>\n",
                if name == "interaction" { "    await click('section > *')\n" } else { "" },
                if name == "seen" { "div" } else { "p" },
            ),
        )
        .unwrap();
    }
    let path: Vec<_> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).filter(|d| !d.join("node").exists()).collect();
    let o = Command::new(bin)
        .args(["test", "comp/lz", "--heap", "--json"])
        .env("PATH", std::env::join_paths(path).unwrap())
        .current_dir(tmp.join("app"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    if out.contains("Chrome") && out.contains("error") && !out.contains("\"tests\"") {
        eprintln!("skipped: no Chrome");
        return;
    }
    for name in ["visible", "idle", "interaction", "eager", "seen"] {
        assert!(out.contains(&format!("comp/lz/{name}.test.brain")), "{name} did not run: {out}");
    }
    assert!(!out.contains("B0601"), "an unmounted instance stayed alive: {out}");
    assert!(o.status.success(), "{out}");
    let _ = std::fs::remove_dir_all(&tmp);
}
