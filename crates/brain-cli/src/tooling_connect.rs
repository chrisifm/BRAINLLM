//! A local stdio configuration proposal plus an actual handshake against one owned executable child.
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_OUTPUT: usize = 64 * 1024;
const INITIALIZE: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"brain-connect-probe\",\"version\":\"1\"}}}\n";

pub fn run(root: &Path, executable: &Path) -> Result<Value, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "cannot start MCP probe")?;
    runtime.block_on(probe(root, executable))
}

async fn probe(root: &Path, executable: &Path) -> Result<Value, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "cannot resolve MCP working directory")?;
    let executable = executable
        .canonicalize()
        .map_err(|_| "cannot resolve current brain executable")?;
    if !root.is_dir() || !executable.is_file() {
        return Err("connect needs a directory and the current brain executable".into());
    }
    let mut child = tokio::process::Command::new(&executable)
        .args(["mcp", "--profile", "minimal"])
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "cannot start owned MCP probe")?;
    let mut stdin = child.stdin.take().ok_or("MCP probe stdin is unavailable")?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("MCP probe stdout is unavailable")?;
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        stdin.write_all(INITIALIZE).await.map_err(|_| "cannot write MCP initialize request")?;
        stdin.flush().await.map_err(|_| "cannot flush MCP initialize request")?;
        let mut bytes = Vec::new();
        let mut block = [0u8; 1024];
        loop {
            let count = stdout.read(&mut block).await.map_err(|_| "cannot read MCP initialize response")?;
            if count == 0 { return Err("MCP child exited before initialize response"); }
            bytes.extend_from_slice(&block[..count]);
            if bytes.len() > MAX_OUTPUT { return Err("MCP initialize response exceeds 64 KiB"); }
            if let Some(end) = bytes.iter().position(|b| *b == b'\n') {
                let value: Value = serde_json::from_slice(&bytes[..end]).map_err(|_| "MCP initialize response is not JSON")?;
                let result = &value["result"];
                if value["jsonrpc"] != "2.0" || value["id"] != 1 || value.get("error").is_some() || result["protocolVersion"] != "2025-06-18" || result["serverInfo"]["name"] != "brain" || !result["capabilities"].is_object() { return Err("MCP initialize response does not match the brain protocol contract"); }
                stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n").await.map_err(|_| "cannot finish MCP initialization")?;
                stdin.shutdown().await.map_err(|_| "cannot close MCP probe input")?;
                return Ok(json!({"protocolVersion":result["protocolVersion"],"serverInfo":result["serverInfo"],"capabilities":result["capabilities"]}));
            }
        }
    }).await;
    // No tools are invoked. Reap the only process this operation owns on every success/failure path.
    drop(stdin);
    let _ = child.start_kill();
    let reaped = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    if !matches!(reaped, Ok(Ok(_))) {
        return Err("cannot reap owned MCP probe after termination".into());
    }
    let handshake = outcome
        .map_err(|_| "MCP initialize reached its 5 second deadline")?
        .map_err(str::to_string)?;
    Ok(
        json!({"schemaVersion":1,"ok":true,"transport":"stdio","handshake":handshake,"probeProcessReaped":true,"clientConnected":false,"configurationWritten":false,"configuration":{"mcpServers":{"brain":{"command":executable.to_string_lossy(),"args":["mcp"],"cwd":root.to_string_lossy()}}},"scope":"Only this owned local MCP child completed initialize. This does not prove that an external agent client is configured, restarted or connected. Configuration is a local proposal and may contain private filesystem paths."}),
    )
}
