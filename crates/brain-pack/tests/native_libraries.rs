//! Native helpers are shaken in emitted output and resolve from frontend and server modules.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Scratch {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self::with_stamp(stamp)
    }
    fn with_stamp(stamp: u128) -> Self {
        // Clock resolution cannot guarantee uniqueness across parallel tests. Reserve
        // ownership before writing anything so another test cannot reuse or remove it.
        for _ in 0..128 {
            let sequence = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "brain-native-pack-{}-{stamp}-{sequence}",
                std::process::id()
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot reserve native fixture directory: {error}"),
            }
        }
        panic!("native fixture directory reservation exceeded its retry limit");
    }
}
fn build(source: &str, extra: &[(&str, &str)]) -> (Scratch, brain_pack::Output) {
    let scratch = Scratch::new();
    let root = &scratch.0;
    std::fs::create_dir_all(root.join("views")).unwrap();
    std::fs::write(root.join("app.brain"),"<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Native helpers</title></head><body><slot /></body></html>").unwrap();
    std::fs::write(root.join("views/index.brain"), source).unwrap();
    for (path, content) in extra {
        let destination = root.join(path);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(destination, content).unwrap();
    }
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let siblings = std::fs::read_dir(&runtime)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "js")
                && !["brain.js", "brain-dev.js", "brain-inspect.js"]
                    .iter()
                    .any(|name| path.ends_with(name))
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(path).unwrap(),
            )
        })
        .collect();
    let compile = |source: &str, path: &Path| {
        let options = brain_codegen::Options {
            dev: false,
            root: Some(root.clone()),
            ..Default::default()
        };
        let output = brain_codegen::compile_with(source, path, &options)
            .map_err(|diags| format!("{}: {diags:?}", path.display()))?;
        Ok(brain_pack::Unit {
            js: output.js,
            css: output.css,
            server_js: output.server_js,
            server_fns: output.server_fns,
        })
    };
    let input = brain_pack::Input {
        root: root.clone(),
        compile: &compile,
        runtime_core: brain_min::minify_runtime_core(
            &std::fs::read_to_string(runtime.join("brain.js")).unwrap(),
        )
        .unwrap(),
        runtime_siblings: siblings,
        layer_order: String::new(),
        minify: true,
        site: None,
        shim: std::fs::read_to_string(runtime.join("shim/dom.js")).ok(),
        generator: String::new(),
        server_module: None,
        cache: Default::default(),
        server_mjs: std::fs::read_to_string(runtime.join("server/server.mjs")).ok(),
        progress: None,
        post: None,
    };
    let output = brain_pack::build(
        &input,
        &[("/".into(), "views/index.brain".into())],
        &|_| vec![],
        &|_| vec![],
    )
    .unwrap();
    (scratch, output)
}
// Execute the actual final minified module in Node, never an unminified runtime source.
fn execute_final_module(root: &Path, name: &str, source: &str, assertion: &str) {
    assert!(
        brain_pack::specifiers(source).unwrap().is_empty(),
        "pure native fixture cannot pull sibling modules"
    );
    let path = root.join(format!("{name}-final.mjs"));
    std::fs::write(&path, source).unwrap();
    let script = format!(
        "import {{ pathToFileURL }} from 'node:url'; const m=await import(pathToFileURL(process.argv[1]).href); {assertion}"
    );
    let node = std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into());
    let mut child = std::process::Command::new(node)
        .args(["--input-type=module", "-e", &script])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("Node is required for emitted-helper semantic checks");
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "final minified {name} module failed its known vector"
            );
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("final {name} module exceeded 10 seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn date_parse_only_ships_its_exact_runtime_dependencies() {
    let (scratch, output) = build(
        r#"<meta seo title="Date" description="A date parser fixture." robots="noindex">
<script>
  let stamp = sig(dateParse('2024-01-01'))
</script>
<main><h1>Date</h1><p>{stamp}</p><button on:click={() => stamp = dateParse('2025-01-01')}>Next year</button></main>"#,
        &[],
    );
    let row = output.ledger.iter().find(|row| row.route == "/").unwrap();
    let asset = row
        .assets
        .iter()
        .find(|asset| asset.source.as_deref() == Some("runtime/brain-time.js"))
        .unwrap_or_else(|| {
            let assets: Vec<_> = row.assets.iter().take(16).map(|asset| (&asset.path, &asset.source, asset.bytes.raw)).collect();
            let helpers: Vec<_> = output.files.iter().filter(|(path,code)| path.starts_with("_b/") && code.contains("dateParse: invalid calendar date")).map(|(path,code)| (path,code.len())).take(4).collect();
            panic!("date helper is attributed to its emitted module; asset metadata={assets:?}; final helper paths={helpers:?}; zero_js={}", output.files["index.html"].contains("brain-zero-js"));
        });
    let chunk = &output.files[&asset.path];
    assert!(
        chunk.contains("dateParse: invalid calendar date"),
        "the used parser must survive shaking"
    );
    assert!(
        !chunk.contains("dateZone:")
            && !chunk.contains("dateRange:")
            && !chunk.contains("DateTimeFormat"),
        "unused date helpers must not ship: {chunk}"
    );
    assert!(!row
        .assets
        .iter()
        .any(|asset| asset.source.as_deref() == Some("runtime/brain-hash.js")));
    assert!(
        !output
            .files
            .iter()
            .any(|(path, source)| path.starts_with("_b/") && source.contains("SHA-512")),
        "date-only fixture must not pull cryptography"
    );
    execute_final_module(
        &scratch.0,
        "date",
        chunk,
        "if(m.dateParse('2024-01-01')!==1704067200000)throw Error('date vector mismatch')",
    );
    assert_eq!(asset.bytes.raw, chunk.len());
    assert_eq!(
        asset.bytes.gzip,
        output
            .binary
            .get(&format!("{}.gz", asset.path))
            .map_or(chunk.len(), |bytes| bytes.len().min(chunk.len()))
    );
    assert_eq!(
        asset.bytes.brotli,
        output
            .binary
            .get(&format!("{}.br", asset.path))
            .map_or(chunk.len(), |bytes| bytes.len().min(chunk.len()))
    );
    assert!(row.conservative_js.raw >= asset.bytes.raw);
    assert!(
        output.files["index.html"].contains("1704067200000"),
        "prerender executes the real date helper"
    );
}
#[test]
fn browser_and_rpc_server_resolve_the_same_native_hash_module() {
    let (scratch, output) = build(
        r#"<meta seo title="Hash" description="Native hashing in frontend and server code." robots="noindex">
<script>
  server: async function serverDigest() { return await hashText('server') }
  let digest = sig('')
  const calculate = async () => {
    digest = await hashText('browser')
    digest = await serverDigest()
  }
</script>
<main><h1>Hash</h1><p>{digest}</p><button on:click={calculate}>Calculate</button></main>"#,
        &[],
    );
    let row = output.ledger.iter().find(|row| row.route == "/").unwrap();
    let asset = row
        .assets
        .iter()
        .find(|asset| asset.source.as_deref() == Some("runtime/brain-hash.js"))
        .expect("frontend hash helper emitted");
    assert!(output.files[&asset.path].contains("SHA-512"));
    execute_final_module(
        &scratch.0,
        "hash",
        &output.files[&asset.path],
        "if(await m.hashText('abc')!=='ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')throw Error('SHA-256 vector mismatch')",
    );
    let server = output
        .files
        .iter()
        .find(|(path, source)| {
            path.starts_with("_server/")
                && path.ends_with(".mjs")
                && source.contains("serverDigest")
        })
        .expect("RPC server module exists");
    let references = brain_pack::specifiers(server.1).unwrap();
    assert!(
        references
            .iter()
            .any(|(_, _, path)| path == &format!("/{}", asset.path)),
        "server import must resolve to emitted native hash chunk: {}",
        server.1
    );
    assert!(
        !server.1.contains("/@brain/"),
        "no development-only import may remain"
    );
    assert!(!row
        .assets
        .iter()
        .any(|asset| asset.source.as_deref() == Some("runtime/brain-time.js")));
}

#[test]
fn nested_server_only_helpers_are_emitted_without_client_edges_or_private_source() {
    let (_scratch, output) = build(
        r#"<meta seo title="Server helper" description="Server-only native dependencies." robots="noindex">
<script>
  import { serverValue } from '../server/native.jsbrain'
  server: async function calculateServer() { return await serverValue() }
  let result = sig('')
  const calculate = async () => { result = JSON.stringify(await calculateServer()) }
</script>
<main><h1>Server helper</h1><p>{result}</p><button on:click={calculate}>Calculate on server</button></main>"#,
        &[(
            "server/native.jsbrain",
            r#"<meta server>
export async function serverValue() {
  return { stamp: dateParse('2024-01-01'), digest: await hashText('PRIVATE_SERVER_ONLY_SENTINEL') }
}"#,
        )],
    );
    let row = output.ledger.iter().find(|row| row.route == "/").unwrap();
    assert!(
        !row.assets.iter().any(|asset| matches!(
            asset.source.as_deref(),
            Some("runtime/brain-time.js" | "runtime/brain-hash.js")
        )),
        "server roots cannot become client ledger/preload edges"
    );
    assert!(
        !output
            .files
            .iter()
            .any(|(path, source)| path.starts_with("_b/")
                && source.contains("PRIVATE_SERVER_ONLY_SENTINEL")),
        "private helper source cannot enter browser chunks"
    );
    let server = output
        .files
        .iter()
        .find(|(path, source)| {
            path.starts_with("_server/") && source.contains("PRIVATE_SERVER_ONLY_SENTINEL")
        })
        .expect("private source stays in server module");
    let refs = brain_pack::specifiers(server.1).unwrap();
    let native: Vec<_> = refs
        .iter()
        .filter(|(_, _, path)| path.starts_with("/_b/"))
        .collect();
    assert_eq!(
        native.len(),
        3,
        "both pure helpers and their execution-budget dependency must resolve in the server: {}",
        server.1
    );
    for (_, _, path) in native {
        let source = &output.files[path.trim_start_matches('/')];
        assert!(
            source.contains("export"),
            "server-only imports must preserve live exports"
        );
    }
}

#[test]
fn parallel_fixture_directories_have_independent_cleanup() {
    let fixtures: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..16)
            .map(|_| scope.spawn(|| Scratch::with_stamp(0)))
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    });
    let paths: std::collections::HashSet<_> = fixtures.iter().map(|fixture| &fixture.0).collect();
    assert_eq!(paths.len(), fixtures.len());
    let mut fixtures = fixtures.into_iter();
    let first = fixtures.next().unwrap();
    let removed = first.0.clone();
    drop(first);
    assert!(!removed.exists());
    for fixture in fixtures {
        assert!(
            fixture.0.is_dir(),
            "another fixture must retain its directory"
        );
    }
}
