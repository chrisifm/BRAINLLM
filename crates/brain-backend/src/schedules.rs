//! Declared cron schedules that enqueue jobs. Any number of scheduler processes may run: each slot is enqueued with the
//! idempotency key `cron:<name>:<slot>`, so the unique (queue, key) index lets exactly one of them create the job.
use crate::jobs::{Enqueue, Queue};
use chrono::{DateTime, Duration as Span, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

/// A schedule slot (UTC).
pub type Slot = DateTime<Utc>;
pub use uuid::Uuid as JobId;
pub fn now() -> Slot {
    Utc::now()
}

/// `server/cron/<name>.jsbrain`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CronSchedule {
    /// Five fields (minute hour day month weekday) or six with leading seconds, evaluated in UTC.
    pub schedule: String,
    /// Job (queue) to enqueue.
    pub job: String,
    #[serde(default = "empty")]
    pub payload: Value,
    /// A slot is still enqueued this many seconds after its time (scheduler restarts); older slots are skipped.
    #[serde(default = "catch_up")]
    pub catch_up: i64,
    #[serde(default)]
    pub priority: i16,
}
fn empty() -> Value {
    Value::Object(Default::default())
}
fn catch_up() -> i64 {
    300
}

impl CronSchedule {
    pub fn parsed(&self) -> Result<cron::Schedule, String> {
        let fields = self.schedule.split_whitespace().count();
        let expr = match fields {
            5 => format!("0 {}", self.schedule),
            6 => self.schedule.clone(),
            _ => return Err(format!("schedule `{}` needs 5 fields (minute hour day month weekday)", self.schedule)),
        };
        cron::Schedule::from_str(&expr).map_err(|e| format!("schedule `{}` is not a valid cron expression: {e}", self.schedule))
    }
    pub fn validate(&self) -> Result<(), String> {
        self.parsed()?;
        if !crate::contract::identifier(&self.job) {
            return Err("job must be a declared job name".into());
        }
        if !(0..=86400).contains(&self.catch_up) || !(-100..=100).contains(&self.priority) {
            return Err("catchUp 0..86400 seconds and priority -100..100".into());
        }
        if !self.payload.is_object() || self.payload.to_string().len() > 65536 {
            return Err("payload must be an object <=64 KB".into());
        }
        Ok(())
    }
    /// The latest slot at or before `now`, if it is still inside the catch-up window.
    pub fn due(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let schedule = self.parsed().ok()?;
        // `after(now)` iterates forward from now; stepping back yields the most recent slot <= now.
        let slot = schedule.after(&(now + Span::milliseconds(1))).next_back()?;
        (now - slot <= Span::seconds(self.catch_up)).then_some(slot)
    }
    pub fn next(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.parsed().ok()?.after(&now).next()
    }
}

/// Enqueue every due slot once. Returns `(schedule, slot, job id)` for each due schedule, including slots another
/// process already enqueued (same id).
pub async fn tick(queue: &Queue, schedules: &[(String, CronSchedule)], now: DateTime<Utc>) -> Result<Vec<(String, DateTime<Utc>, Uuid)>, String> {
    let mut out = Vec::new();
    for (name, s) in schedules {
        let Some(slot) = s.due(now) else { continue };
        let key = format!("cron:{name}:{}", slot.format("%Y-%m-%dT%H:%M:%SZ"));
        let id = queue
            .enqueue_with(&s.job, s.payload.clone(), &Enqueue { key: Some(&key), priority: s.priority, ..Enqueue::default() })
            .await?;
        out.push((name.clone(), slot, id));
    }
    Ok(out)
}

/// Scheduler loop: ticks at start, then sleeps until the next slot (at most 30 s) until shutdown.
pub async fn run(
    queue: Queue,
    schedules: Vec<(String, CronSchedule)>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    on_enqueue: impl Fn(&str, DateTime<Utc>, Uuid),
) -> Result<(), String> {
    for (name, s) in &schedules {
        s.validate().map_err(|e| format!("server/cron/{name}.jsbrain: {e}"))?;
    }
    let mut seen = std::collections::HashSet::new();
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let now = Utc::now();
        for (name, slot, id) in tick(&queue, &schedules, now).await? {
            if seen.insert((name.clone(), slot)) {
                on_enqueue(&name, slot, id);
            }
        }
        seen.retain(|(_, slot)| now - *slot <= Span::days(2));
        let next = schedules.iter().filter_map(|(_, s)| s.next(now)).min();
        let wait = next.map_or(Duration::from_secs(30), |t| (t - Utc::now()).to_std().unwrap_or_default()).min(Duration::from_secs(30));
        tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            _ = tokio::time::sleep(wait + Duration::from_millis(20)) => {}
        }
    }
}
