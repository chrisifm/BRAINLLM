//! §9 in the dev server (#24): cross-origin isolated pages, the inspector buffer at GET /@brain/metrics, and
//! `brain test --heap` reporting a component instance that stays alive after its test unmounted it.
#![cfg(unix)]
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn isolation_metrics_and_heap_leaks() {
    let tmp = std::env::temp_dir().join(format!("binsp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bin = env!("CARGO_BIN_EXE_brain");
    assert!(Command::new(bin).args(["new", "app"]).current_dir(&tmp).stdout(Stdio::null()).status().unwrap().success());
    let dir = tmp.join("app");
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
    let page = get("/about");
    assert!(page.contains("Cross-Origin-Opener-Policy: same-origin") && page.contains("Cross-Origin-Embedder-Policy: credentialless"), "{page}");
    let m = get("/@brain/metrics");
    assert!(m.contains("\"batches\":[") && m.contains("\"leaks\":["), "{m}");
    let _ = c.kill();
    let _ = c.wait();
    // a module-level cache that keeps the component's node: alive after unmount → B0601
    std::fs::write(dir.join("lib/cache.jsbrain"), "const all = []\nexport default { keep: (x) => all.push(x), size: () => all.length }\n").unwrap();
    std::fs::write(dir.join("comp/leaky.brain"), "<script>\n  import cache from '../lib/cache.jsbrain'\n  let el = ref()\n  mounted(() => { cache.keep(el) })\n</script>\n\n<p bind:ref={el}>Leaky</p>\n").unwrap();
    std::fs::write(dir.join("comp/leaky.test.brain"), "<script>\n  import Leaky from './leaky.brain'\n\n  test('renders', async () => {\n    await mount(Leaky, {})\n    expect(text('p')).toBe('Leaky')\n  })\n</script>\n").unwrap();
    let o = Command::new(bin).args(["test", "comp/leaky.test.brain", "--heap", "--json"]).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    if out.contains("Chrome") && out.contains("error") {
        eprintln!("skipped: no Chrome");
        return;
    }
    assert!(!o.status.success() && out.contains("\"code\":\"B0601\"") && out.contains("comp/leaky.brain"), "{out}");
    let _ = std::fs::remove_dir_all(&tmp);
}
