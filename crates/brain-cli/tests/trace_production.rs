//! INTERNALS §48: `brain trace` is development only. A production build carries none of its code, endpoints or headers.
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn brain(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).output().unwrap()
}

fn files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() { files(&path, out) } else { out.push(path) }
    }
}

const MARKERS: &[&str] = &["brain-trace", "__brainTrace", "x-brain-trace", "brain:trace", "trace.db", "trace-spool", "Report-Only", "rid;desc"];

#[test]
fn nothing_of_the_capture_reaches_a_production_build() {
    let tmp = std::env::temp_dir().join(format!("brain-trace-prod-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let app = tmp.join("app");
    assert!(std::fs::read_to_string(app.join(".gitignore")).unwrap().lines().any(|line| line == ".brain/"), "the store stays out of git");
    // a server function makes the build emit server.mjs as well
    std::fs::write(app.join("views/save.brain"), "<meta seo keyword=\"save page\" title=\"Save page\" description=\"A page with a server function, so the build emits the production server.\" canonical=\"/save\">\n<script>\n  server: async function save(ctx, value) { return value }\n  let done = sig('')\n  const send = async () => { done = await save('x') }\n</script>\n<main><h1>Save page</h1><p>{done}</p><button type=\"button\" on:click={send}>Send</button></main>\n").unwrap();
    let built = brain(&app, &["build"]);
    assert!(built.status.success(), "{}{}", String::from_utf8_lossy(&built.stdout), String::from_utf8_lossy(&built.stderr));
    let mut output = Vec::new();
    files(&app.join("dist"), &mut output);
    assert!(output.iter().any(|path| path.file_name().is_some_and(|name| name == "server.mjs")), "control: the server is part of what was scanned");
    assert!(output.len() > 5, "control: {output:?}");
    for path in &output {
        let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
        for marker in MARKERS {
            assert!(!text.contains(marker), "{marker} in {}", path.display());
        }
    }
    assert!(!app.join(".brain/trace.db").exists(), "a build records nothing");
    // control: the same binary does carry the module, and serves it to `brain run`, so the scan can see these names
    let module = include_str!("../../../runtime/brain-trace.js");
    assert!(module.contains("x-brain-trace") && module.contains("__brainTrace"), "control: the markers are the real ones");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn the_cli_answers_quietly_and_names_its_fix() {
    let tmp = std::env::temp_dir().join(format!("brain-trace-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(brain(&tmp, &["new", "app"]).status.success());
    let app = tmp.join("app");
    let empty = brain(&app, &["trace", "--json"]);
    assert!(empty.status.success(), "{}", String::from_utf8_lossy(&empty.stderr));
    assert!(String::from_utf8_lossy(&empty.stdout).contains("\"schemaVersion\":1"));
    let wrong = brain(&app, &["trace", "--bogus"]);
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("fix:"));
    let explained = brain(&app, &["explain", "B1842"]);
    assert!(explained.status.success(), "{}", String::from_utf8_lossy(&explained.stderr));
    let _ = std::fs::remove_dir_all(&tmp);
}
