//! Read-only local agent compiler feed. No implicit agent process or source mutation.
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
fn cursor(options: &Value) -> Result<(Option<&str>, u64, usize)> {
    if !options.is_object() {
        return Err("feedback options must be an object".into());
    }
    let epoch = match options.get("afterEpoch") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or("afterEpoch must be a string or null")?),
    };
    let after = options
        .get("afterSequence")
        .map_or(Some(0), Value::as_u64)
        .ok_or("afterSequence must be a nonnegative integer")?;
    let limit = options
        .get("limit")
        .map_or(Some(64), Value::as_u64)
        .filter(|n| *n > 0 && *n <= 128)
        .ok_or("feedback limit must be 1..128")? as usize;
    Ok((epoch, after, limit))
}
/// Bounded MCP/CLI polling with explicit reset/gap detection.
pub fn query(root: &Path, options: &Value) -> Result<Value> {
    let (epoch, after, limit) = cursor(options)?;
    let result = brain_serve::feedback::poll(root, epoch, after, limit)?;
    serde_json::from_str(&result.stringify())
        .map_err(|_| "invalid compiler feedback response".into())
}
/// Writes at most 128 event records and bounded control lines; returns the continuation cursor.
pub fn watch(root: &Path, options: &Value, writer: &mut dyn Write) -> Result<Value> {
    let (initial_epoch, initial_sequence, _) = cursor(options)?;
    let duration = options
        .get("durationMs")
        .map_or(Some(5000), Value::as_u64)
        .filter(|n| *n >= 150 && *n <= 30000)
        .ok_or("feedback watch durationMs must be 150..30000")?;
    let max = options
        .get("maxRecords")
        .map_or(Some(128), Value::as_u64)
        .filter(|n| *n > 0 && *n <= 128)
        .ok_or("feedback watch maxRecords must be 1..128")? as usize;
    let started = Instant::now();
    let deadline = Duration::from_millis(duration);
    let mut epoch = initial_epoch.map(str::to_string);
    let mut sequence = initial_sequence;
    let mut records = 0usize;
    let mut gaps = 0usize;
    let mut last_control = String::new();
    loop {
        if started.elapsed() >= deadline || records >= max {
            break;
        }
        let batch = query(
            root,
            &json!({"afterEpoch":epoch,"afterSequence":sequence,"limit":(max-records).min(64)}),
        )?;
        if batch["gap"] == true || batch["reset"] == true || batch["available"] != true {
            let control = json!({"type":"feedback-status","available":batch["available"],"epoch":batch["epoch"],"latestSequence":batch["latestSequence"],"reset":batch["reset"],"gap":batch["gap"],"pendingPublication":batch["pendingPublication"]});
            let line = control.to_string();
            if line != last_control && gaps < 128 {
                writer
                    .write_all(line.as_bytes())
                    .and_then(|_| writer.write_all(b"\n"))
                    .map_err(|_| "cannot write compiler feedback stream")?;
                last_control = line;
                gaps += 1;
            }
        }
        if let Some(current) = batch["epoch"].as_str() {
            epoch = Some(current.into());
        }
        for event in batch["events"]
            .as_array()
            .into_iter()
            .flatten()
            .take(max - records)
        {
            let line = serde_json::to_vec(&json!({"type":"compiler-feedback","event":event}))
                .map_err(|_| "cannot encode compiler feedback event")?;
            if line.len() > 32768 {
                return Err("compiler feedback event exceeds stream budget".into());
            }
            writer
                .write_all(&line)
                .and_then(|_| writer.write_all(b"\n"))
                .map_err(|_| "cannot write compiler feedback event")?;
            records += 1;
        }
        if let Some(next) = batch["nextSequence"].as_u64() {
            sequence = next;
        }
        writer
            .flush()
            .map_err(|_| "cannot flush compiler feedback stream")?;
        if records >= max || started.elapsed() >= deadline {
            break;
        }
        if batch["hasMore"] != true {
            std::thread::sleep(
                Duration::from_millis(150).min(deadline.saturating_sub(started.elapsed())),
            );
        }
    }
    Ok(
        json!({"schemaVersion":1,"records":records,"controlRecords":gaps,"afterEpoch":epoch,"afterSequence":sequence,"deadlineReached":started.elapsed()>=deadline,"limitReached":records>=max,"agentSpawned":false,"sourceEdited":false}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn watch_is_finite_and_reports_absent_store_without_creating_it() {
        let root =
            std::env::temp_dir().join(format!("brain-feedback-watch-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut output = Vec::new();
        let started = Instant::now();
        let result = watch(
            &root,
            &json!({"durationMs":150,"maxRecords":2}),
            &mut output,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(result["records"], 0);
        assert_eq!(result["agentSpawned"], false);
        assert!(!root.join(".brain").exists());
        assert_eq!(String::from_utf8(output).unwrap().lines().count(), 1);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn invalid_watch_and_poll_budgets_fail_before_waiting() {
        let root = Path::new(".");
        assert!(query(root, &json!({"limit":0})).is_err());
        assert!(watch(root, &json!({"durationMs":30001}), &mut Vec::new()).is_err());
        assert!(query(root, &json!({"afterSequence":-1})).is_err());
    }
}
