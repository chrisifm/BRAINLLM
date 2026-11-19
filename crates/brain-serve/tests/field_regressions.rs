use brain_serve::{CompileFn, Compiled, Options, load::{Sidecar, file_url}, start_dev};
use std::{io::{Read, Write}, path::PathBuf, sync::{Arc, atomic::{AtomicU64, Ordering}}, time::Duration};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("brain-field-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

#[test]
fn multiline_private_context_keeps_json_strings_and_plural_objects() {
    let dir = Temp::new();
    let file = dir.0.join("page.mjs");
    std::fs::write(&file, "export function load(ctx) { return { dict: ctx.dict, text: ctx.text, n: ctx.n } }").unwrap();
    let mut sidecar = Sidecar::spawn(&dir.0, &[]).unwrap();
    let context = r#"{
        "dict": { "items": {
            "one": "one item",
            "other": "{n} items"
        } },
        "text": "a \"quote\" and \\ slash\nline\ttab",
        "n": 1e2
    }"#;
    let expected = r#"{"dict":{"items":{"one":"one item","other":"{n} items"}},"text":"a \"quote\" and \\ slash\nline\ttab","n":100}"#;
    for input in [context.to_string(), context.replace('\n', "\r\n")] {
        assert_eq!(sidecar.call(&file_url(&file), "load", &input).unwrap(), expected);
    }
    assert_eq!(sidecar.call(&file_url(&file), "load", "{\"n\":2}").unwrap(), "{\"n\":2}");
}

#[test]
fn canonical_dev_probes_support_get_head_base_paths_and_public_precedence() {
    let dir = Temp::new();
    let compile: CompileFn = Arc::new(|_, _| Ok(Compiled { server: Default::default(), js: String::new(), css: String::new(), deps: vec![], server_js: String::new() }));
    for base in ["/", "/app"] {
        let opts = Options { root: dir.0.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: String::new(), runtime: vec![], env: None, base: base.into(), diagnose: None, problems: None, upstream: None, mock: None };
        let mut dev = start_dev(opts, compile.clone(), false, None).unwrap();
        let ask = |method: &str, path: &str| {
            let mut socket = std::net::TcpStream::connect(dev.addr).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            socket.write_all(format!("{method} {}{path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n", base.trim_end_matches('/')).as_bytes()).unwrap();
            let mut out = String::new(); socket.read_to_string(&mut out).unwrap(); out
        };
        for path in ["/health", "/health/live", "/health/ready"] {
            for method in ["GET", "HEAD"] {
                let response = ask(method, path);
                assert!(response.starts_with("HTTP/1.1 200") && response.contains("no-store"), "{response}");
                if method == "HEAD" { assert!(response.ends_with("\r\n\r\n"), "{response}"); }
                else { assert!(response.ends_with("{\"ok\":true}"), "{response}"); }
            }
        }
        assert!(ask("GET", "/health/unknown").starts_with("HTTP/1.1 404"));
        std::fs::create_dir_all(dir.0.join("public")).unwrap();
        std::fs::write(dir.0.join("public/health"), "authored health content").unwrap();
        let authored = ask("GET", "/health");
        assert!(authored.ends_with("authored health content"), "{authored}");
        dev.stop();
        std::fs::remove_file(dir.0.join("public/health")).unwrap();
    }
}
