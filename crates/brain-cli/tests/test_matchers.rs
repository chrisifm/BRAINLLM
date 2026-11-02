//! The `*.test.brain` matcher list (`toBe`, `toEqual`, `toBeTruthy`, `toBeFalsy`, `toContain`, each with `.not`) and
//! `waitFor`, which retries an assertion after each render instead of a fixed sleep. A failing matcher names what it
//! expected, and a `waitFor` that never passes reports its last failure.
#![cfg(unix)]
use std::process::{Command, Stdio};

#[test]
fn matchers_and_wait_for_pass_and_fail_with_their_reason() {
    let tmp = std::env::temp_dir().join(format!("bmatch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "app"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("app");
    std::fs::write(
        dir.join("comp/later.brain"),
        "<script>\n  let note = sig('waiting')\n  mounted(() => { const t = setTimeout(() => note = 'ready', 150); return () => clearTimeout(t) })\n</script>\n<p>{note}</p>\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("comp/later.test.brain"),
        concat!(
            "<script>\n  import Later from './later.brain'\n\n",
            "  test('matchers pass', async () => {\n",
            "    expect(false).toBeFalsy()\n    expect('').toBeFalsy()\n    expect(1).not.toBeFalsy()\n",
            "    expect('job title').toContain('title')\n    expect(['a', 'b']).toContain('b')\n    expect(['a']).not.toContain('z')\n",
            "    expect(2).not.toBe(3)\n    expect({ a: 1 }).not.toEqual({ a: 2 })\n    expect(0).not.toBeTruthy()\n  })\n\n",
            "  test('waits for text set by a timer', async () => {\n",
            "    await mount(Later)\n    expect(text('p')).toBe('waiting')\n",
            "    await waitFor(() => expect(text('p')).toBe('ready'))\n  })\n\n",
            "  test('a negated matcher fails with its reason', async () => {\n    expect('abc').not.toContain('b')\n  })\n\n",
            "  test('toContain refuses a number', async () => {\n    expect(5).toContain(5)\n  })\n\n",
            "  test('waitFor reports its last failure', async () => {\n",
            "    await mount(Later)\n    await waitFor(() => expect(text('p')).toBe('never'), { timeout: 400 })\n  })\n",
            "</script>\n",
        ),
    )
    .unwrap();
    let path: Vec<_> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).filter(|d| !d.join("node").exists()).collect();
    let o = Command::new(bin)
        .args(["test", "comp/later.test.brain", "--json"])
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
    let test = |name: &str| {
        let at = out.find(&format!("\"name\":\"{name}\"")).unwrap_or_else(|| panic!("no test {name}: {out}"));
        let end = out[at..].find('}').map_or(out.len(), |e| at + e);
        out[at..end].to_string()
    };
    assert!(test("matchers pass").contains("\"ok\":true"), "{out}");
    assert!(test("waits for text set by a timer").contains("\"ok\":true"), "{out}");
    let negated = test("a negated matcher fails with its reason");
    assert!(negated.contains("\"ok\":false") && negated.contains("expected not a value containing"), "{negated}");
    assert!(test("toContain refuses a number").contains("toContain needs a string or an array"), "{out}");
    let waited = test("waitFor reports its last failure");
    assert!(waited.contains("waitFor: still failing after 400 ms") && waited.contains("never"), "{waited}");
    assert!(!o.status.success(), "three tests fail on purpose: {out}");
    let _ = std::fs::remove_dir_all(&tmp);
}
