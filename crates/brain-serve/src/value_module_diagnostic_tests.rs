//! Invalid value modules must fail at import, before callers attempt resource methods.
use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "brain-module-diagnostic-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("api")).unwrap();
        std::fs::create_dir_all(path.join("comp")).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn options(&self) -> Options {
        Options {
            root: self.0.clone(),
            host: "127.0.0.1".into(),
            port: 0,
            head_prepend: String::new(),
            runtime: vec![],
            env: None,
            base: "/".into(),
            diagnose: None,
            problems: None,
            upstream: None,
            mock: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request(address: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut bytes = Vec::new();
    stream.take(256 * 1024).read_to_end(&mut bytes).unwrap();
    let response = String::from_utf8(bytes).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    response.split_once("\r\n\r\n").unwrap().1.into()
}

fn imported(js: &str) -> serde_json::Value {
    use std::process::{Command, Stdio};
    let script = format!(
        r#"
      const source={};
      const dependency='data:text/javascript,'+encodeURIComponent(source);
      const importer='import value from '+JSON.stringify(dependency)+';export default value';
      try {{ const value=await import('data:text/javascript,'+encodeURIComponent(importer));
        console.log(JSON.stringify({{resolved:true,type:typeof value.default}}));
      }} catch(error) {{ console.log(JSON.stringify({{resolved:false,name:error.name,message:error.message,code:error.code,file:error.file,line:error.line,column:error.column,fix:error.fix,diagnostics:error.diagnostics}})); }}
    "#,
        json_str(js)
    );
    let mut child = Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node is required for actual ESM import verification");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Owned Node module import exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.len() < 65536 && output.stderr.len() < 65536);
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn invalid_value_module_import_rejects_with_original_checker_diagnostics_and_recovers() {
    let fixture = Fixture::new();
    let source = "export default api('/_brain/auth/me', { get: { cache: 'never' } })";
    std::fs::write(fixture.0.join("api/account.jsbrain"), source).unwrap();
    std::fs::write(
        fixture.0.join("comp/broken.brain"),
        "<script>let value =</script>",
    )
    .unwrap();
    let root = fixture.0.clone();
    let compile: CompileFn = Arc::new(move |source, path| {
        let path = path.strip_prefix(&root).unwrap();
        let parsed = brain_parse::parse(source, path).map_err(|diags| {
            diags
                .iter()
                .map(brain_lex::Diag::to_json)
                .collect::<Vec<_>>()
        })?;
        let errors: Vec<_> = brain_check::check(&parsed, &brain_check::Ctx::default())
            .into_iter()
            .filter(|diag| diag.level == brain_lex::Severity::Error)
            .map(|diag| diag.to_json())
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Compiled {
            js: "export default {get(){return 'fixed'}}".into(),
            css: String::new(),
            deps: vec![],
            server_js: String::new(),
        })
    });
    let ast = brain_parse::parse(source, Path::new("api/account.jsbrain")).unwrap();
    let expected: Vec<serde_json::Value> = brain_check::check(&ast, &brain_check::Ctx::default())
        .into_iter()
        .filter(|diag| diag.level == brain_lex::Severity::Error)
        .map(|diag| serde_json::from_str(&diag.to_json()).unwrap())
        .collect();
    assert!(expected.iter().any(|diag| diag["code"] == "B1102"));
    let observed_root = fixture.0.clone();
    let observed = Arc::new(AtomicUsize::new(0));
    let notifications = observed.clone();
    let reloads = Arc::new(AtomicUsize::new(0));
    let reloaded = reloads.clone();
    let event: EventFn = Arc::new(move |message| {
        if message.contains("\"type\":\"reload\"") && message.contains("api/account.jsbrain") {
            reloaded.fetch_add(1, Ordering::SeqCst);
        }
        if message.contains("\"type\":\"compiler-feedback\"") {
            let durable = crate::feedback::poll(&observed_root, None, 0, 64).unwrap();
            assert!(
                matches!(durable.get("events"), Some(brain_lex::Json::Arr(events)) if !events.is_empty())
            );
            notifications.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut dev = start_dev(fixture.options(), compile, false, Some(event)).unwrap();
    let invalid = request(dev.addr, "/api/account.jsbrain");
    assert!(
        observed.load(Ordering::SeqCst) > 0,
        "durable feedback must precede HTTP delivery"
    );
    let component = request(dev.addr, "/comp/broken.brain");
    assert!(component.contains("function BrainCompileError()"));
    assert!(
        component.contains("document.createElement"),
        "component-local fallback remains intact"
    );
    let failure = imported(&invalid);
    assert_eq!(
        failure["resolved"], false,
        "a value module must not silently resolve to a component fallback: {failure}"
    );
    assert_eq!(failure["name"], "BrainCompileError");
    assert_eq!(failure["diagnostics"], serde_json::json!(expected));
    assert_eq!(failure["code"], expected[0]["code"]);
    assert_eq!(failure["file"], expected[0]["file"]);
    assert_eq!(failure["line"], expected[0]["line"]);
    assert_eq!(failure["column"], expected[0]["col"]);
    assert_eq!(failure["fix"], expected[0]["fix"]);
    assert!(failure["message"].as_str().unwrap().contains("B1102"));
    assert!(!invalid.contains("document.createElement"));
    std::fs::write(
        fixture.0.join("api/account.jsbrain"),
        "export default api('/_brain/auth/me', { get: {} })",
    )
    .unwrap();
    on_change(&dev.ctx, &fixture.0.join("api/account.jsbrain"));
    assert_eq!(
        reloads.load(Ordering::SeqCst),
        1,
        "value-module edits retain the full-reload recovery path"
    );
    let fixed = request(dev.addr, "/api/account.jsbrain");
    assert_eq!(
        imported(&fixed)["resolved"],
        true,
        "fresh requests recover after the source is fixed"
    );
    dev.stop();
}
