//! `brain trace` (INTERNALS §49): in development the backend appends what it did for a request to
//! `.brain/trace-spool/backend.ndjson`, and `brain run` or `brain trace` move those lines into the store. The caller
//! installs it only outside production. Statements carry placeholders, never bound values; the store redacts the rest.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata};

/// What one app may leave unread when nothing is draining the spool.
const SPOOL_CAP: u64 = 16 * 1024 * 1024;
static SPOOL: OnceLock<PathBuf> = OnceLock::new();

/// Starts the capture for this process. A process has one global subscriber, so a second call changes nothing.
pub fn install(root: &std::path::Path) {
    let dir = root.join(".brain/trace-spool");
    if std::fs::create_dir_all(&dir).is_ok() && SPOOL.set(dir.join("backend.ndjson")).is_ok() {
        let _ = tracing::subscriber::set_global_default(Queries);
    }
}

fn append(event: serde_json::Value) {
    let Some(path) = SPOOL.get() else { return };
    if std::fs::metadata(path).is_ok_and(|file| file.len() > SPOOL_CAP) {
        return;
    }
    // reopened every time: the reader claims the file by renaming it
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{event}");
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// A finished request, by route template: the raw path and query stay out, as in the backend's own log lines.
pub(crate) fn request(rid: &str, method: &str, route: &str, status: u16, ms: f64) {
    if SPOOL.get().is_some() {
        append(serde_json::json!({ "ts": now(), "module": "backend", "kind": "request", "request_id": rid, "route": route, "status": status, "duration_ms": ms, "data": { "method": method } }));
    }
}

#[derive(Default)]
struct Statement {
    sql: String,
    rows_returned: u64,
    rows_affected: u64,
    seconds: f64,
}

impl Visit for Statement {
    fn record_str(&mut self, field: &Field, value: &str) {
        // sqlx sends a short statement (COMMIT) only as its summary
        if field.name() == "db.statement" || (field.name() == "summary" && self.sql.is_empty()) {
            let value = value.trim();
            if !value.is_empty() {
                self.sql = value.to_string();
            }
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if matches!(field.name(), "db.statement" | "summary") {
            let text = format!("{value:?}");
            let text = text.trim_matches('"').trim();
            if !text.is_empty() && (field.name() == "db.statement" || self.sql.is_empty()) {
                self.sql = text.to_string();
            }
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "rows_returned" => self.rows_returned = value,
            "rows_affected" => self.rows_affected = value,
            _ => {}
        }
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == "elapsed_secs" {
            self.seconds = value;
        }
    }
}

struct Queries;

impl tracing::Subscriber for Queries {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "sqlx::query"
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        // workers and pollers also query: only what serves a request belongs to a trace
        let Some(rid) = crate::http::request_reference() else { return };
        let mut statement = Statement::default();
        event.record(&mut statement);
        append(serde_json::json!({ "ts": now(), "module": "backend", "kind": "query", "request_id": rid, "duration_ms": statement.seconds * 1000.0, "data": { "statement": statement.sql, "rowsReturned": statement.rows_returned, "rowsAffected": statement.rows_affected } }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_statement_keeps_sql_rows_and_time() {
        let mut seen = Statement::default();
        tracing::subscriber::with_default(Capture, || {
            tracing::event!(target: "sqlx::query", tracing::Level::DEBUG, summary = "select", db.statement = "SELECT 1 WHERE id = $1", rows_affected = 0u64, rows_returned = 1u64, elapsed_secs = 0.5f64);
        });
        STATE.with_borrow_mut(|state| std::mem::swap(state, &mut seen));
        assert_eq!(seen.sql, "SELECT 1 WHERE id = $1");
        assert_eq!(seen.rows_returned, 1);
        assert_eq!(seen.seconds, 0.5);
    }

    thread_local! { static STATE: std::cell::RefCell<Statement> = std::cell::RefCell::default(); }
    struct Capture;
    impl tracing::Subscriber for Capture {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool { metadata.target() == "sqlx::query" }
        fn new_span(&self, _: &Attributes<'_>) -> Id { Id::from_u64(1) }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
        fn event(&self, event: &Event<'_>) { STATE.with_borrow_mut(|state| event.record(state)) }
    }
}
