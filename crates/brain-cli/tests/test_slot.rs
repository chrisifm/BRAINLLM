//! `mount(Comp, props, { slot, slots })` in `*.test.brain`: a component whose accessible text comes only from its
//! slot is tested with that text, so the accessibility audit sees it (no false B0406).
#![cfg(unix)]
use std::process::{Command, Stdio};

#[test]
fn slot_content_reaches_the_component_and_the_accessibility_audit() {
    let tmp = std::env::temp_dir().join(format!("bslot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "app"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("app");
    std::fs::write(
        dir.join("comp/link-button.brain"),
        "<meta props=\"href:str\">\n<a href={href} class=\"btn\"><slot /></a>\n<p><slot name=\"hint\" /></p>\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("comp/link-button.test.brain"),
        "<script>\n  import LinkButton from './link-button.brain'\n\n  test('renders the slot text as the link text', async () => {\n    await mount(LinkButton, { href: '/next' }, { slot: 'Continue', slots: { hint: 'Opens the next step' } })\n    expect(text('a')).toBe('Continue')\n    expect(text('p')).toBe('Opens the next step')\n  })\n</script>\n",
    )
    .unwrap();
    // The test page (test-api.js) is the same for both engines; hide Node so the owned headless Chrome lane runs
    // and the test never opens tabs in a connected Viewer browser.
    let path: Vec<_> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).filter(|d| !d.join("node").exists()).collect();
    let o = Command::new(bin)
        .args(["test", "comp/link-button.test.brain", "--json"])
        .env("PATH", std::env::join_paths(path).unwrap())
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    if out.contains("Chrome") && out.contains("error") && !out.contains("\"tests\"") {
        eprintln!("skipped: no Chrome");
        return;
    }
    assert!(o.status.success(), "{out}");
    assert!(!out.contains("B0406"), "the audit must see the slot text: {out}");
    let _ = std::fs::remove_dir_all(&tmp);
}
