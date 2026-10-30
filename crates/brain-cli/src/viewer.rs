//! Optional real connected Viewer browser runner. A failed connected bridge never becomes a headless success.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const CLIENT: &str = include_str!("templates/viewer-client.mjs");
const MAX_OUTPUT: usize = 2 * 1024 * 1024;
fn invoke(input: &Value, timeout: Duration) -> Result<Value, String> {
    let bytes = serde_json::to_vec(input).map_err(|_| "cannot encode Viewer request")?;
    if bytes.len() > 65536 {
        return Err("Viewer request exceeds 64 KiB".into());
    }
    let script = format!("{CLIENT}\nawait viewerMain();\n");
    let mut child = Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Viewer runner needs Node.js with fetch support")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Viewer output pipe is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Viewer diagnostics pipe is unavailable")?;
    let output = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(MAX_OUTPUT as u64 + 1).read_to_end(&mut bytes);
        (result, bytes)
    });
    let diagnostics = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.take(65537).read_to_end(&mut bytes);
        bytes.len()
    });
    let written = child
        .stdin
        .take()
        .ok_or("Viewer input pipe is unavailable")
        .and_then(|mut pipe| {
            pipe.write_all(&bytes)
                .map_err(|_| "cannot deliver Viewer input")
        });
    if written.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        let _ = output.join();
        let _ = diagnostics.join();
        return Err("Viewer input failed; no headless fallback was attempted".into());
    }
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let (read, bytes) = output.join().map_err(|_| "Viewer output reader failed")?;
    let stderr_size = diagnostics.join().unwrap_or(65537);
    if status.is_none() {
        return Err("Viewer runner deadline exceeded; owned-tab cleanup is unknown and no headless fallback was attempted".into());
    }
    if !status.unwrap().success()
        || read.is_err()
        || bytes.len() > MAX_OUTPUT
        || stderr_size > 65536
    {
        return Err("Viewer runner failed or exceeded its bounded output; inspect local setup without exposing browser profile data".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "Viewer runner returned an invalid structured response".into())
}
/// Probe projects only service version, connected state and userScripts capability; never profiles or tabs.
pub fn probe() -> Result<Value, String> {
    match invoke(&json!({"op":"probe","profile":std::env::var("BRAIN_VIEWER_PROFILE").ok()}), Duration::from_secs(5)) {
        Err(error) if error == "Viewer runner needs Node.js with fetch support" => Ok(
            json!({"ok":true,"available":false,"engine":"viewer","reason":"node-runner-unavailable","fallbackEligible":true}),
        ),
        result => result,
    }
}
/// Drives the existing native declared scenario API in freshly opened owned tabs.
pub fn run(base: &str, root: &Path, files: &[PathBuf], timeout: Duration) -> Result<Value, String> {
    if files.is_empty() || files.len() > 32 {
        return Err("Viewer suite needs 1..32 declared test files".into());
    }
    let mut relative = Vec::new();
    for path in files {
        let path = path
            .strip_prefix(root)
            .map_err(|_| "Viewer test file is outside the app")?
            .to_string_lossy()
            .replace('\\', "/");
        crate::changes::source_path(root, &path)?;
        if !path.ends_with(".test.brain") {
            return Err("Viewer runner accepts declared .test.brain suites only".into());
        }
        relative.push(path);
    }
    let ms = timeout.as_millis().clamp(2000, 180000) as u64;
    invoke(
        &json!({"op":"run","base":base,"files":relative,"timeoutMs":ms,"profile":std::env::var("BRAIN_VIEWER_PROFILE").ok()}),
        Duration::from_millis(ms + 7000),
    )
}
