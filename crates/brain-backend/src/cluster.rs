//! State shared by every backend process of one deployment.
//!
//! A single process keeps rate-limit windows in memory and fans socket events out locally. With `cluster.shared`
//! (or `BRAIN_BACKEND_PROCESSES` above 1) rate windows live in Postgres (`_brain_rate_windows`, one atomic upsert per
//! hit) and committed resource changes are relayed to the sockets of every process with `LISTEN/NOTIFY`.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::time::Duration;
use uuid::Uuid;

/// Postgres notification channel for committed resource changes.
pub const CHANNEL: &str = "brain_resource_changes";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cluster {
    /// Keep rate windows in Postgres and relay socket events between processes.
    pub shared: bool,
}

impl Cluster {
    /// Parses the optional `cluster` object of `server/platform.jsbrain`: `{ shared: true }`.
    pub fn from_contract(value: Option<&Value>) -> Result<Self, String> {
        let Some(value) = value else { return Ok(Self::default()) };
        let map = value.as_object().ok_or("platform.cluster must be an object such as { shared: true }")?;
        if let Some(key) = map.keys().find(|k| k.as_str() != "shared") {
            return Err(format!("platform.cluster has the unknown key `{key}`; the only key is `shared`"));
        }
        let shared = match map.get("shared") {
            None => false,
            Some(v) => v.as_bool().ok_or("platform.cluster.shared must be true or false")?,
        };
        Ok(Self { shared })
    }
    pub fn schema(&self) -> Value {
        json!({ "shared": self.shared, "rateStore": if self.shared { "postgres" } else { "memory" }, "socketFanout": if self.shared { "postgres-notify" } else { "process" } })
    }
}

/// Outcome of one shared rate-limit hit: `Err(seconds until the window resets)` when over budget.
pub type Hit = Result<(), u64>;

/// Counts one request against `count` per `window` for `(scope, subject)` in a fixed window shared by all processes.
/// The subject (an address, user or key id) is stored only as a SHA-256 digest.
pub async fn hit(pool: &PgPool, scope: &str, subject: &str, count: u32, window: Duration) -> Result<Hit, String> {
    let digest = Sha256::digest(subject.as_bytes()).to_vec();
    let millis = i64::try_from(window.as_millis()).unwrap_or(i64::MAX).max(1);
    let row = sqlx::query(concat!(
        "INSERT INTO public._brain_rate_windows AS w(scope,subject,window_start,window_ms,hits) ",
        "VALUES ($1,$2,now(),$3,1) ON CONFLICT (scope,subject) DO UPDATE SET ",
        "hits=CASE WHEN w.window_start+make_interval(secs=>w.window_ms/1000.0)<=now() OR w.window_ms<>$3 THEN 1 ELSE w.hits+1 END,",
        "window_start=CASE WHEN w.window_start+make_interval(secs=>w.window_ms/1000.0)<=now() OR w.window_ms<>$3 THEN now() ELSE w.window_start END,",
        "window_ms=$3 ",
        "RETURNING hits, ceil(extract(epoch FROM w.window_start+make_interval(secs=>w.window_ms/1000.0)-now()))::bigint AS remaining"
    ))
    .bind(scope)
    .bind(digest)
    .bind(millis)
    .fetch_one(pool)
    .await
    .map_err(crate::db::safe)?;
    let hits: i64 = row.get("hits");
    if hits > i64::from(count) {
        let remaining: i64 = row.get("remaining");
        return Ok(Err(remaining.max(1) as u64));
    }
    Ok(Ok(()))
}

/// Removes expired windows in bounded batches; runs every five minutes while the process lives.
pub(crate) fn spawn_sweeper(pool: PgPool) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;
            let _ = sqlx::query("DELETE FROM public._brain_rate_windows WHERE ctid IN (SELECT ctid FROM public._brain_rate_windows WHERE window_start+make_interval(secs=>window_ms/1000.0)<now()-interval '1 minute' LIMIT 5000)")
                .execute(&pool)
                .await;
        }
    });
}

/// A committed resource change as relayed between processes. Row values are never included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub origin: Uuid,
    pub resource: String,
    pub id: Uuid,
    pub owner: Uuid,
    pub operation: String,
}

impl Change {
    pub fn encode(&self) -> String {
        json!({"o": self.origin, "r": self.resource, "i": self.id, "w": self.owner, "p": self.operation}).to_string()
    }
    pub fn decode(text: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(text).ok()?;
        Some(Self {
            origin: v["o"].as_str()?.parse().ok()?,
            resource: v["r"].as_str()?.to_string(),
            id: v["i"].as_str()?.parse().ok()?,
            owner: v["w"].as_str()?.parse().ok()?,
            operation: v["p"].as_str()?.to_string(),
        })
    }
}

/// Publishes one change to every listening process. Failures only lose a cache-invalidation hint.
pub(crate) fn publish(pool: &PgPool, change: Change) {
    let pool = pool.clone();
    tokio::spawn(async move {
        let _ = sqlx::query("SELECT pg_notify($1,$2)").bind(CHANNEL).bind(change.encode()).execute(&pool).await;
    });
}

/// Listens for changes from other processes and hands each one to `deliver`. Reconnects after connection loss;
/// events sent while disconnected are lost, which socket clients already handle by resynchronizing.
pub(crate) fn spawn_listener<F>(pool: PgPool, origin: Uuid, deliver: F)
where
    F: Fn(Change) + Send + Sync + 'static,
{
    tokio::spawn(async move {
        loop {
            let mut listener = match sqlx::postgres::PgListener::connect_with(&pool).await {
                Ok(listener) => listener,
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            if listener.listen(CHANNEL).await.is_err() {
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            loop {
                match listener.try_recv().await {
                    Ok(Some(note)) => {
                        if let Some(change) = Change::decode(note.payload()).filter(|c| c.origin != origin) {
                            deliver(change);
                        }
                    }
                    Ok(None) => continue,
                    Err(_) => break,
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract_and_change_roundtrip() {
        assert_eq!(Cluster::from_contract(None).unwrap(), Cluster::default());
        assert!(Cluster::from_contract(Some(&json!({"shared": true}))).unwrap().shared);
        assert!(Cluster::from_contract(Some(&json!({"shared": 1}))).is_err());
        assert!(Cluster::from_contract(Some(&json!({"redis": true}))).is_err());
        let change = Change { origin: Uuid::new_v4(), resource: "posts".into(), id: Uuid::new_v4(), owner: Uuid::new_v4(), operation: "update".into() };
        assert_eq!(Change::decode(&change.encode()), Some(change));
        assert_eq!(Change::decode("{}"), None);
    }
}
