//! Author text must survive source-map bookkeeping in both client and server output.
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn node(source: &str) -> brain_parse::Json {
    let mut child = Command::new("node")
        .args(["--input-type=module", "-"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("Node 22+ is required for emitted-value regressions");
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let capture = |stream: Box<dyn Read + Send>| std::thread::spawn(move || {
        let mut result = Vec::new();
        stream.take(65_537).read_to_end(&mut result).unwrap();
        result
    });
    let out = capture(Box::new(stdout));
    let err = capture(Box::new(stderr));
    child.stdin.take().unwrap().write_all(source.as_bytes()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("emitted-value regression exceeded its Node deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let out = out.join().unwrap();
    let err = err.join().unwrap();
    assert!(out.len() <= 65_536 && err.len() <= 65_536, "unexpectedly large emitted-value output");
    assert!(status.success(), "Node evaluation failed: {}", String::from_utf8_lossy(&err));
    brain_parse::Json::parse(String::from_utf8_lossy(&out).trim()).unwrap()
}

fn quoted(value: &str) -> String {
    let mut result = String::new();
    brain_parse::push_json_str(&mut result, value);
    result
}

fn source_line(output: &brain_codegen::Output, needle: &str) -> Option<i64> {
    let map = brain_parse::Json::parse(&output.map).unwrap();
    let mappings = map.get("mappings").and_then(brain_parse::Json::as_str).unwrap();
    let mut original = 0i64;
    for (line, segment) in output.js.lines().zip(mappings.split(';')) {
        let mut fields = Vec::new();
        let (mut value, mut shift) = (0i64, 0);
        for ch in segment.chars() {
            let digit = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".find(ch).unwrap() as i64;
            value += (digit & 31) << shift;
            if digit & 32 != 0 { shift += 5; } else {
                fields.push(if value & 1 == 1 { -(value >> 1) } else { value >> 1 });
                value = 0;
                shift = 0;
            }
        }
        if fields.len() >= 3 { original += fields[2]; }
        if line.contains(needle) { return (fields.len() >= 3).then_some(original + 1); }
    }
    None
}

#[test]
fn parser_discards_author_comments_without_redirecting_maps() {
    let source = "<script>\nlet value = sig('naïve /*@L71*/')\n/*@L999*/\nconst change = () => {\n/*@L888*/\nvalue = 'updated /*@L72*/'\n}\n</script>\n<button on:click={change}>{value}</button>";
    for dev in [false, true] {
        let output = brain_codegen::compile_with(source, Path::new("comp/comments.brain"), &brain_codegen::Options { dev, ..Default::default() }).unwrap();
        assert!(!output.js.contains("/*@L999*/") && !output.js.contains("/*@L888*/"), "{}", output.js);
        assert!(output.js.contains("naïve /*@L71*/") && output.js.contains("updated /*@L72*/"), "{}", output.js);
        assert_eq!(source_line(&output, "const value = sig("), Some(2));
        assert_eq!(source_line(&output, "const change ="), Some(4));
        assert_eq!(source_line(&output, "value.v = 'updated"), Some(6));
    }
}

#[test]
fn regex_statements_after_wrapped_blocks_keep_literal_data() {
    let source = "export default () => {\nif (true) String(1);\n/[/*@L84*/]/.test('/');\nfor (const i of [0]) String(i);\n/[/*@L85*/]/.test('/');\nreturn ({value: 12}).value / 3\n}";
    for dev in [false, true] {
        let output = brain_codegen::compile_with(source, Path::new("lib/regex.jsbrain"), &brain_codegen::Options { dev, ..Default::default() }).unwrap();
        for line in [84, 85] { assert!(output.js.contains(&format!("/[/*@L{line}*/]/")), "{}", output.js); }
    }
}

#[test]
fn client_and_server_modules_keep_author_literal_values_in_both_modes() {
    let source = r#"export default {
  single: '/*@L71*/',
  double: "/*@L72*/",
  template: `before /*@L73*/ ${'interpolation /*@L74*/'} after`,
  nested: `outer ${`inner /*@L75*/ ${'nested /*@L76*/'}`} tail`,
  regex: /[/*@L77*/]/.source,
  escaped: 'don\'t /*@L78*/',
  unicode: 'naïve 😀 /*@L79*/ façade',
  occupied: '/*@L0:1*/ and /*@L1:2*/',
  publicEnv: import.meta.env.PUBLIC_MARKER,
  malformed: '/*@Lnot-a-line*/',
  unfinished: '/*@L42 without a close',
  division: ({value: 12}).value / 3,
  block: `function ${(() => {
    const value = 'naïve /*@L80*/'
    return value
  })()} tail`,
  conditional: (() => {
    let value = ''
    if (true) value = /[/*@L81*/]/.source
    if (true) (/[/*@L82*/]/.test('/'));
    /[/*@L84*/]/.test('/')
    for (const i of [0]) (/[/*@L83*/]/.test('/'));
    /[/*@L85*/]/.test('/')
    return value
  })(),
}"#;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    for dev in [false, true] {
        for server in [false, true] {
            let source = if server { format!("<meta server>\n{source}") } else { source.to_string() };
            let output = brain_codegen::compile_with(&source, Path::new("lib/markers.jsbrain"), &brain_codegen::Options { dev, public_env: vec![("PUBLIC_MARKER".into(), "/*@L2:9*/".into())], ..Default::default() })
                .unwrap_or_else(|diags| panic!("{diags:#?}"));
            let js = if server { &output.server_js } else { &output.js };
            for line in [82, 83, 84, 85] { assert!(js.contains(&format!("/[/*@L{line}*/]/")), "{js}"); }
            let values = node(&format!("import {{pathToFileURL}} from 'node:url';\nconst base = pathToFileURL({} + '/runtime/').href;\nconst code = {}.replaceAll('/@brain/', base);\nconst value = await import('data:text/javascript,' + encodeURIComponent(code));\nconsole.log(JSON.stringify(value.default));", quoted(&root.to_string_lossy()), quoted(js)));
            assert_eq!(values.get("division"), Some(&brain_parse::Json::Num(4.0)));
            for (key, expected) in [
                ("single", "/*@L71*/"), ("double", "/*@L72*/"),
                ("template", "before /*@L73*/ interpolation /*@L74*/ after"),
                ("nested", "outer inner /*@L75*/ nested /*@L76*/ tail"),
                ("regex", "[/*@L77*/]"), ("escaped", "don't /*@L78*/"),
                ("unicode", "naïve 😀 /*@L79*/ façade"),
                ("occupied", "/*@L0:1*/ and /*@L1:2*/"),
                ("publicEnv", "/*@L2:9*/"), ("malformed", "/*@Lnot-a-line*/"),
                ("unfinished", "/*@L42 without a close"),
                ("block", "function naïve /*@L80*/ tail"),
                ("conditional", "[/*@L81*/]"),
            ] {
                assert_eq!(values.get(key).and_then(brain_parse::Json::as_str), Some(expected), "{key}, dev={dev}, server={server}");
            }
        }
    }
}

#[test]
fn inline_child_literals_remain_author_data() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let parent = Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(parent).unwrap();
    let directory = (0..128).find_map(|_| {
        let path = parent.join(format!("source-marker-inline-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::create_dir(&path) { Ok(()) => Some(path), Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None, Err(error) => panic!("{error}") }
    }).expect("an exclusive source marker fixture directory");
    let root = Scratch(directory);
    std::fs::create_dir(root.0.join("ui")).unwrap();
    std::fs::create_dir(root.0.join("comp")).unwrap();
    std::fs::write(root.0.join("ui/label.brain"), "<p>inline /*@L0:7*/ /*@L71*/ naïve</p>").unwrap();
    let source = "<script>import Label from '../ui/label.brain'</script><Label />";
    std::fs::write(root.0.join("comp/host.brain"), source).unwrap();
    let output = brain_codegen::compile_with(source, Path::new("comp/host.brain"), &brain_codegen::Options { root: Some(root.0.clone()), ..Default::default() })
        .unwrap_or_else(|diags| panic!("{diags:#?}"));
    assert!(output.js.contains("inline /*@L0:7*/ /*@L71*/ naïve"), "{}", output.js);
}

#[test]
fn rendered_html_keeps_marker_shaped_text_and_runtime_budget_imports() {
    let source = "<script>\nlet n = sig(0)\nconst label = '/*@L2*/'\n</script>\n<main><p>html /*@L75*/ naïve {label}</p><button on:click={n++}>Count {n}</button></main>";
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    for dev in [false, true] {
        let output = brain_codegen::compile_with(source, Path::new("comp/markers.brain"), &brain_codegen::Options { dev, ..Default::default() })
            .unwrap_or_else(|diags| panic!("{diags:#?}"));
        assert!(output.js.contains("brain-budget.js"), "budget instrumentation must remain enabled");
        let script = format!(
            "import {{pathToFileURL}} from 'node:url';\nconst base = pathToFileURL({} + '/runtime/').href;\nconst {{install}} = await import(base + 'shim/dom.js');\ninstall();\nconst code = {}.replaceAll('/@brain/', base);\nconst component = (await import('data:text/javascript,' + encodeURIComponent(code))).default;\nconst {{mount}} = await import(base + 'brain.js');\nconst target = document.createElement('div');\nconst unmount = mount(component, target);\nconsole.log(JSON.stringify({{html: target.innerHTML, wiring: component.$hmr?.wiring ?? null}}));\nunmount();",
            quoted(&root.to_string_lossy()), quoted(&output.js)
        );
        let rendered = node(&script);
        assert!(rendered.get("html").and_then(brain_parse::Json::as_str).unwrap().contains("html /*@L75*/ naïve"), "{rendered:?}");
        assert!(rendered.get("html").and_then(brain_parse::Json::as_str).unwrap().contains("/*@L2*/"), "{rendered:?}");
        if dev {
            let wiring = rendered.get("wiring").and_then(brain_parse::Json::as_str).expect("dev HMR wiring");
            assert!(wiring.contains("const label = '/*@L2*/';"), "{wiring}");
            assert!(!wiring.contains("/*@L2*/  const n") && !wiring.contains("/*@L3*/"), "{wiring}");
        }
    }
}
