//! `ctx.llm` end to end: a built app calls a local fake provider through RPC; the key never reaches `dist/` client files.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const KEY: &str = "synthetic-llm-provider-key";

fn brain(args: &[&str], cwd: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(cwd).stdin(Stdio::null()).output().unwrap()
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_dir() {
            files(&e.path(), out);
        } else {
            out.push(e.path());
        }
    }
}

fn app(name: &str, script: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("brain-llm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(brain(&["new", "demo"], &dir).status.success());
    let app = dir.join("demo");
    std::fs::write(app.join("views/ask.brain"), script).unwrap();
    app
}

const SERVER_VIEW: &str = "<meta seo title=\"Ask\" description=\"Ask the model a question from the server\">\n<script>\nlet answer = sig('')\nserver: async function ask(ctx, q) { const r = await ctx.llm.complete({ prompt: q }); return { text: r.text, tokens: r.usage.outputTokens } }\n</script>\n<main><h1>Ask</h1><button on:click={async () => { answer = (await ask('hi')).text }}>Ask</button><p>{answer}</p></main>\n";

#[test]
fn check_rejects_browser_use_and_warns_without_a_key() {
    let app = app("check", "<meta seo title=\"Ask\" description=\"Ask the model a question from the browser\">\n<script>\nconst f = (ctx) => ctx.llm.complete({ prompt: 'x' })\n</script>\n<main><h1>Ask</h1><button on:click={() => f({})}>Ask</button></main>\n");
    let out = brain(&["check", "--json"], &app);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success() && text.contains("B0920"), "{text}");
    std::fs::write(app.join("views/ask.brain"), SERVER_VIEW).unwrap();
    let out = brain(&["check", "--json"], &app);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("B0923") && !text.contains("B0920"), "{text}");
    let env = std::fs::read_to_string(app.join(".brain_env")).unwrap();
    std::fs::write(app.join(".brain_env"), format!("{env}\nPUBLIC_OPENAI_API_KEY=x\n")).unwrap();
    let text = String::from_utf8_lossy(&brain(&["check", "--json"], &app).stdout).into_owned();
    assert!(text.contains("B0920"), "{text}");
}

#[test]
fn a_built_app_reaches_the_provider_and_the_key_stays_on_the_server() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    }
    let app = app("build", SERVER_VIEW);
    let env = std::fs::read_to_string(app.join(".brain_env")).unwrap();
    std::fs::write(app.join(".brain_env"), format!("{env}\nANTHROPIC_MODEL=configured-model\nANTHROPIC_API_KEY?=\n")).unwrap();
    let out = brain(&["build"], &app);
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));

    let mut all = Vec::new();
    files(&app.join("dist"), &mut all);
    let (mut server_side, mut client_side) = (0, 0);
    for f in &all {
        let rel = f.strip_prefix(app.join("dist")).unwrap().to_string_lossy().to_string();
        if rel == "server.mjs" || rel.starts_with("_server") {
            server_side += 1;
            continue;
        }
        client_side += 1;
        let bytes = std::fs::read(f).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for leak in ["ANTHROPIC", "OPENAI", "llmClient", "api.anthropic.com", "ctx.llm"] {
            assert!(!text.contains(leak), "{leak} reached the client file {rel}");
        }
    }
    assert!(server_side > 0 && client_side > 0);
    assert!(app.join("dist/_server/llm.mjs").exists());

    // the fake provider: one Messages answer, and it records what it saw
    let provider = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let provider_port = provider.local_addr().unwrap().port();
    let fake = std::thread::spawn(move || {
        let (mut c, _) = provider.accept().unwrap();
        c.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = c.read(&mut buf).unwrap_or(0);
            seen.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&seen);
            if n == 0 || text.contains("\"messages\"") && text.trim_end().ends_with('}') {
                break;
            }
        }
        let body = r#"{"model":"served","content":[{"type":"text","text":"from the fake provider"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":4}}"#;
        write!(c, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        String::from_utf8_lossy(&seen).into_owned()
    });

    let port_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = port_probe.local_addr().unwrap().port();
    drop(port_probe);
    let mut server = Command::new("node")
        .arg("dist/server.mjs")
        .envs([("PORT", port.to_string()), ("HOST", "127.0.0.1".to_string()), ("ANTHROPIC_API_KEY", KEY.to_string()), ("ANTHROPIC_MODEL", "configured-model".to_string()), ("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{provider_port}"))])
        .current_dir(&app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let server_dir = app.join("dist/_server");
    let module = std::fs::read_dir(&server_dir).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "mjs") && std::fs::read_to_string(p).is_ok_and(|t| t.contains("ask") && t.contains("ctx.llm"))).expect("the view's server module");
    let id = format!("{}/ask", &module.file_stem().unwrap().to_string_lossy()[..4]);
    let body = "[\"hi\"]";
    let mut answer = String::new();
    for _ in 0..100 {
        if let Ok(mut c) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            write!(c, "POST /_r/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nX-Brain-RPC: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            c.read_to_string(&mut answer).unwrap();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = server.kill();
    let _ = server.wait();
    assert!(answer.contains("from the fake provider") && answer.contains("\"tokens\":4"), "{answer}");
    assert!(!answer.contains(KEY));
    let seen = fake.join().unwrap().to_lowercase();
    assert!(seen.contains(&format!("x-api-key: {KEY}")), "{seen}");
    assert!(seen.contains("\"model\":\"configured-model\""), "{seen}");
}
