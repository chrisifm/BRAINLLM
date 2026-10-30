//! The live-resource handle is metadata-reactive without changing signal kinds.
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

fn source(binding: &str) -> String {
    format!(
        "<script>\nconst rows=resource(() => Promise.resolve([]))\n{binding} updates=liveResource(rows,{{rooms:[{{resource:'posts'}}],scope:()=> 'public-user:1'}})\nlet label=drv(()=> updates.state + ':' + updates.connected)\n</script>\n<p>{{label}}</p>"
    )
}

#[test]
fn native_handle_argument_and_reactive_getter_metadata_compile_without_new_signal_kinds() {
    for binding in ["const", "let"] {
        let output = brain_codegen::compile(&source(binding), Path::new("comp/live.brain"))
            .unwrap_or_else(|diags| panic!("{diags:#?}"));
        assert!(output.js.contains("liveResource(rows,"), "{}", output.js);
        assert!(!output.js.contains("liveResource(rows.v,"));
        assert!(output.js.contains("from \"/@brain/brain-live.js\""));
        assert!(
            output.js.contains("drv("),
            "The live getter must not be constant-folded: {}",
            output.js
        );
        assert!(output.js.contains("budgetOwner"));
    }
    let output = brain_codegen::compile("<script>const liveResource=(value)=>value\nconst rows=resource(()=> Promise.resolve([]))\nconst result=liveResource(rows)</script><p>Custom function</p>", Path::new("comp/custom.brain")).unwrap();
    assert!(!output.js.contains("brain-live.js"));
    assert!(
        output.js.contains("liveResource(rows.v)"),
        "An author function must keep ordinary expression semantics"
    );
}

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
static NEXT: AtomicU64 = AtomicU64::new(0);
fn fixture() -> Fixture {
    let path = std::env::temp_dir().join(format!(
        "brain-live-render-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    Fixture(path)
}
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn drain(mut pipe: impl Read) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buffer = [0u8; 4096];
    while let Ok(count) = pipe.read(&mut buffer) {
        if count == 0 {
            break;
        }
        let keep = count.min(65536usize.saturating_sub(kept.len()));
        kept.extend_from_slice(&buffer[..keep]);
    }
    kept
}

#[test]
fn compiled_dom_tracks_live_state_for_const_let_and_derived_reads() {
    let fixture = fixture();
    let framework = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::create_dir(fixture.0.join("runtime")).unwrap();
    fs::write(fixture.0.join("package.json"), "{\"type\":\"module\"}").unwrap();
    for item in fs::read_dir(framework.join("runtime")).unwrap().flatten() {
        let path = item.path();
        if path.extension().is_some_and(|extension| extension == "js") {
            fs::copy(
                &path,
                fixture.0.join("runtime").join(path.file_name().unwrap()),
            )
            .unwrap();
        }
    }
    fs::copy(
        framework.join("runtime/shim/dom.js"),
        fixture.0.join("dom.js"),
    )
    .unwrap();
    for binding in ["const", "let"] {
        let options = brain_codegen::Options {
            runtime: "./runtime/brain.js".into(),
            ..Default::default()
        };
        let output =
            brain_codegen::compile_with(&source(binding), Path::new("comp/live.brain"), &options)
                .unwrap_or_else(|diags| panic!("{diags:#?}"));
        fs::write(fixture.0.join(format!("{binding}.mjs")), output.js).unwrap();
    }
    fs::File::create(fixture.0.join("proof.mjs"))
        .unwrap()
        .write_all(include_bytes!("live_resource.mjs"))
        .unwrap();
    let mut child = Owned(
        Command::new("node")
            .arg(fixture.0.join("proof.mjs"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Node is required for the owned compiled DOM proof"),
    );
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let out = std::thread::spawn(move || drain(stdout));
    let err = std::thread::spawn(move || drain(stderr));
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            panic!("Owned live-resource DOM proof exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let output = String::from_utf8_lossy(&out.join().unwrap()).into_owned();
    let error = String::from_utf8_lossy(&err.join().unwrap()).into_owned();
    assert!(status.success(), "{output}\n{error}");
    assert!(output.contains("Live compiled DOM: PASS"));
}
