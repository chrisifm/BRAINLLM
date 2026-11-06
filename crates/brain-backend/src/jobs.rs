//! Durable Postgres queue: at-least-once delivery, leased ownership and bounded workers. Handlers must be idempotent.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Row};
use std::{future::Future, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use uuid::Uuid;

/// `NOTIFY` channel of ready jobs; the payload is the queue name. Delivered at commit, so a rolled back job wakes nobody.
const READY_CHANNEL: &str = "_brain_jobs_ready";
/// How often a running worker or scheduler refreshes its row in `_brain_workers`.
const WORKER_BEAT: Duration = Duration::from_secs(15);
/// A worker seen within this many seconds counts as alive: three missed beats.
pub const WORKER_FRESH_SECONDS: i64 = 45;
/// Idle poll with a live wake-up connection: it only finds delayed jobs, retries and expired leases.
const IDLE_POLL: Duration = Duration::from_millis(1000);
/// Idle poll without one (a pooler that does not support LISTEN, a lost connection).
const IDLE_POLL_NO_WAKE: Duration = Duration::from_millis(250);
/// Each final mutation and the worker-row cleanup gets at most one second, including connection checkout.
const CLEANUP_BUDGET: Duration = Duration::from_secs(1);
async fn bounded_database<T>(operation: &'static str, future: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    match tokio::time::timeout(CLEANUP_BUDGET, future).await {
        Ok(result) => result,
        Err(_) => {
            crate::db::diagnostic(&serde_json::json!({"schemaVersion":1,"event":"worker-database-deadline",
                "operation":operation,"phase":"cleanup","private":true,"fix":"Inspect database locks and connection availability; the uncertain mutation was not retried","valuesCaptured":false}));
            Err("Worker database cleanup deadline expired; inspect sanitized server diagnostics".into())
        }
    }
}

/// Retries only obtaining a connection, never a statement whose commit status may be unknown.
#[derive(Clone, Copy)]
struct AcquireRecovery {
    attempts: usize,
    budget: Duration,
    delay: Duration,
}
const CLAIM_ACQUIRE_RECOVERY: AcquireRecovery = AcquireRecovery {
    attempts: 3,
    budget: Duration::from_secs(15),
    delay: Duration::from_millis(250),
};

async fn acquire_for_worker<T, F, Fut>(
    mut acquire: F,
    stop: &mut tokio::sync::watch::Receiver<bool>,
    recovery: AcquireRecovery,
) -> Result<Option<T>, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, sqlx::Error>>,
{
    let deadline = tokio::time::Instant::now() + recovery.budget;
    for attempt in 1..=recovery.attempts {
        if *stop.borrow() || stop.has_changed().is_err() { return Ok(None) }
        let result = tokio::select! {
            biased;
            _ = stop.changed() => return Ok(None),
            result = tokio::time::timeout_at(deadline, acquire()) => result.unwrap_or(Err(sqlx::Error::PoolTimedOut)),
        };
        match result {
            Ok(connection) => return Ok(Some(connection)),
            Err(error) => {
                let retry = matches!(error, sqlx::Error::PoolTimedOut)
                    && attempt < recovery.attempts && tokio::time::Instant::now() < deadline;
                crate::db::diagnostic(&serde_json::json!({
                    "schemaVersion":1,"event":"worker-database-acquire","operation":"queue.claim",
                    "phase":"acquire","attempt":attempt,"retry":retry,"private":true,"fix":"Inspect pool occupancy and database availability; no claim SQL has been sent","valuesCaptured":false,
                }));
                let message = crate::db::safe_operation(error, "queue.claim", "acquire");
                if !retry { return Err(message) }
                let next = (tokio::time::Instant::now() + recovery.delay * attempt as u32).min(deadline);
                tokio::select! {
                    biased;
                    _ = stop.changed() => return Ok(None),
                    _ = tokio::time::sleep_until(next) => {},
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("Worker database acquisition deadline expired; inspect sanitized server diagnostics".into());
                }
            }
        }
    }
    Err("Worker database acquisition attempts exhausted; inspect sanitized server diagnostics".into())
}

/// Dropping a worker must also cancel its owned liveness and LISTEN tasks.
struct BackgroundTask(tokio::task::JoinHandle<()>);
impl Drop for BackgroundTask {
    fn drop(&mut self) { self.0.abort(); }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub queue: String,
    pub payload: Value,
    pub attempt: i32,
    pub lease: Uuid,
}
/// Per-queue policy declared in `server/jobs/<name>/queue.jsbrain`; every key is optional.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct QueuePolicy {
    /// Default enqueue priority, -100..100; higher runs first.
    pub priority: i16,
    /// Attempts before a job becomes `dead`, 1..10.
    pub attempts: i32,
    pub backoff: Backoff,
    /// Seconds one attempt may run, 1..3600. Longer jobs keep their lease alive automatically.
    pub timeout: u64,
    /// Lease seconds, 10..600, renewed every third of the lease while the handler runs.
    pub lease: u64,
    /// Seconds a stopping worker lets in-flight jobs finish before releasing them, 0..600.
    pub drain: u64,
    /// Workers per process, 1..8.
    pub concurrency: usize,
    /// Identity of `ctx.backend` in the handler: absent or `key` uses BRAIN_JOBS_API_KEY, `system` BRAIN_JOBS_SYSTEM_KEY.
    #[serde(rename = "runAs", skip_serializing_if = "Option::is_none")]
    pub run_as: Option<String>,
}
impl Default for QueuePolicy {
    fn default() -> Self {
        Self { priority: 0, attempts: 3, backoff: Backoff::default(), timeout: 60, lease: 90, drain: 0, concurrency: 2, run_as: None }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackoffKind {
    Fixed,
    Exponential,
}
/// Retry delay: `fixed` waits `delay` seconds; `exponential` waits `delay * 2^(attempt-1)`, capped at `max`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Backoff {
    #[serde(rename = "type")]
    pub kind: BackoffKind,
    pub delay: u64,
    pub max: u64,
}
impl Default for Backoff {
    fn default() -> Self {
        Self { kind: BackoffKind::Exponential, delay: 2, max: 300 }
    }
}
impl Backoff {
    pub fn seconds(&self, attempt: i32) -> u64 {
        match self.kind {
            BackoffKind::Fixed => self.delay,
            BackoffKind::Exponential => self.delay.saturating_mul(1u64 << (attempt.clamp(1, 20) - 1)).min(self.max),
        }
    }
}
impl QueuePolicy {
    pub fn validate(&self) -> Result<(), String> {
        let checks = [
            ((-100..=100).contains(&self.priority), "priority: -100..100"),
            ((1..=10).contains(&self.attempts), "attempts: 1..10"),
            ((1..=3600).contains(&self.timeout), "timeout: 1..3600 seconds"),
            ((10..=600).contains(&self.lease), "lease: 10..600 seconds"),
            (self.drain <= 600, "drain: 0..600 seconds"),
            ((1..=8).contains(&self.concurrency), "concurrency: 1..8"),
            (self.backoff.delay <= 86400 && self.backoff.max <= 86400, "backoff delay and max: 0..86400 seconds"),
            (self.backoff.kind == BackoffKind::Fixed || self.backoff.delay <= self.backoff.max, "backoff: delay must not exceed max"),
            (self.run_as.as_deref().is_none_or(|r| matches!(r, "key" | "system")), "runAs: 'key' or 'system'"),
        ];
        match checks.iter().find(|(ok, _)| !ok) {
            Some((_, rule)) => Err(format!("invalid queue policy; fix: {rule}")),
            None => Ok(()),
        }
    }
}
/// Options for one enqueue. `attempts: None` uses 3.
#[derive(Clone, Debug, Default)]
pub struct Enqueue<'a> {
    pub key: Option<&'a str>,
    pub attempts: Option<i32>,
    pub priority: i16,
    /// Run no earlier than this many seconds from now, 0..2592000 (30 days).
    pub delay_seconds: i64,
}
#[derive(Clone)]
pub struct Queue {
    pool: PgPool,
}
impl Queue {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn enqueue(
        &self,
        queue: &str,
        payload: Value,
        key: Option<&str>,
        max_attempts: i32,
    ) -> Result<Uuid, String> {
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        let id = enqueue_tx(&mut tx, queue, payload, key, max_attempts).await?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(id)
    }
    pub async fn enqueue_with(&self, queue: &str, payload: Value, options: &Enqueue<'_>) -> Result<Uuid, String> {
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        let id = enqueue_tx_with(&mut tx, queue, payload, options).await?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(id)
    }
    /// Extend a running job's lease. `false` means the lease was lost (expired or reclaimed): stop and do not ack.
    pub async fn heartbeat(&self, job: &Job, lease_seconds: u64) -> Result<bool, String> {
        sqlx::query("UPDATE _brain_jobs SET lease_until=now()+make_interval(secs=>$3) WHERE id=$1 AND state='running' AND lease_token=$2 AND lease_until>now()")
            .bind(job.id).bind(job.lease).bind(lease_seconds.clamp(1, 3600) as f64).execute(&self.pool).await.map(|r| r.rows_affected() == 1).map_err(|error| crate::db::safe_operation(error, "queue.heartbeat", "query"))
    }
    /// Dead letters of one queue, newest first. Payloads are omitted, like `status`.
    pub async fn dead(&self, queue: &str, limit: i64) -> Result<Vec<Value>, String> {
        if !crate::contract::identifier(queue) {
            return Err("invalid queue".into());
        }
        sqlx::query_scalar("SELECT jsonb_build_object('id',id,'queue',queue,'attempts',attempts,'maxAttempts',max_attempts,'priority',priority,'lastError',last_error,'finishedAt',finished_at) FROM _brain_jobs WHERE queue=$1 AND state='dead' ORDER BY finished_at DESC,id LIMIT $2")
            .bind(queue).bind(limit.clamp(1, 1000)).fetch_all(&self.pool).await.map_err(crate::db::safe)
    }
    /// Move dead jobs back to `pending` with fresh attempts: one job (`id`) or up to `limit` of the queue.
    /// Respects the queue capacity; returns how many were revived.
    pub async fn retry_dead(&self, queue: &str, id: Option<Uuid>, limit: i64) -> Result<u64, String> {
        if !crate::contract::identifier(queue) {
            return Err("invalid queue".into());
        }
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        sqlx::query("SELECT pg_advisory_xact_lock(728196411, hashtext($1))").bind(queue).execute(&mut *tx).await.map_err(crate::db::safe)?;
        let active: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM (SELECT 1 FROM _brain_jobs WHERE queue=$1 AND state IN ('pending','running') LIMIT $2) bounded")
            .bind(queue).bind(QUEUE_CAPACITY).fetch_one(&mut *tx).await.map_err(crate::db::safe)?;
        let room = (QUEUE_CAPACITY - active).min(limit.clamp(1, 1000));
        if room <= 0 {
            return Err(format!("job queue `{queue}` is full ({QUEUE_CAPACITY} active jobs); drain workers before retrying"));
        }
        let revived = sqlx::query("UPDATE _brain_jobs SET state='pending',attempts=0,available_at=now(),finished_at=NULL,last_error=NULL WHERE id IN (SELECT id FROM _brain_jobs WHERE queue=$1 AND state='dead' AND ($2::uuid IS NULL OR id=$2) ORDER BY finished_at,id LIMIT $3 FOR UPDATE)")
            .bind(queue).bind(id).bind(room).execute(&mut *tx).await.map_err(crate::db::safe)?.rows_affected();
        notify_ready(&mut tx, queue).await?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(revived)
    }
    /// Job counts by state; `delayed` are pending jobs not yet due, `oldestWaitSeconds` is how long the oldest due job
    /// has waited and `workers` how many processes working this queue were seen recently.
    pub async fn stats(&self, queue: &str) -> Result<Value, String> {
        if !crate::contract::identifier(queue) {
            return Err(format!("invalid queue `{queue}`; fix: a declared job name, like `brain jobs stats order_receipts`"));
        }
        sqlx::query_scalar("SELECT jsonb_build_object('queue',$1::text,'pending',count(*) FILTER (WHERE state='pending' AND available_at<=now()),'delayed',count(*) FILTER (WHERE state='pending' AND available_at>now()),'running',count(*) FILTER (WHERE state='running'),'succeeded',count(*) FILTER (WHERE state='succeeded'),'dead',count(*) FILTER (WHERE state='dead'),'oldestWaitSeconds',coalesce(floor(extract(epoch FROM now()-min(available_at) FILTER (WHERE state='pending' AND available_at<=now()))),0)::bigint,'workers',(SELECT count(*) FROM public._brain_workers w WHERE w.kind='worker' AND w.queue=$1 AND w.seen_at>now()-make_interval(secs=>$2))) FROM _brain_jobs WHERE queue=$1")
            .bind(queue).bind(WORKER_FRESH_SECONDS as f64).fetch_one(&self.pool).await.map_err(crate::db::safe)
    }
    /// Queues holding active jobs (the reserved outbound webhook queue has its own in-process worker).
    pub async fn active_queues(&self) -> Result<Vec<String>, String> {
        sqlx::query_scalar("SELECT DISTINCT queue FROM _brain_jobs WHERE state IN ('pending','running') AND queue<>$1 ORDER BY queue")
            .bind(crate::webhooks::OUTBOUND_QUEUE).fetch_all(&self.pool).await.map_err(crate::db::safe)
    }
    /// Record that a worker or scheduler process is alive. Rows of processes gone for a day are removed here.
    pub async fn beat(&self, id: Uuid, kind: &str, queue: &str) -> Result<(), String> {
        sqlx::query("INSERT INTO public._brain_workers(id,kind,queue,pid) VALUES ($1,$2,$3,$4) ON CONFLICT (id) DO UPDATE SET seen_at=now()")
            .bind(id).bind(kind).bind(queue).bind(std::process::id() as i32).execute(&self.pool).await.map_err(|error| crate::db::safe_operation(error, "queue.worker.beat", "query"))?;
        sqlx::query("DELETE FROM public._brain_workers WHERE seen_at<now()-interval '1 day'").execute(&self.pool).await.map_err(|error| crate::db::safe_operation(error, "queue.worker.beat", "query"))?;
        Ok(())
    }
    /// A clean stop leaves no row behind, so the next liveness check is right at once.
    pub async fn leave(&self, id: Uuid) -> Result<(), String> {
        sqlx::query("DELETE FROM public._brain_workers WHERE id=$1").bind(id).execute(&self.pool).await.map(|_| ()).map_err(|error| crate::db::safe_operation(error, "queue.worker.leave", "query"))
    }
    /// Scheduler processes seen recently.
    pub async fn schedulers(&self) -> Result<i64, String> {
        sqlx::query_scalar("SELECT count(*)::bigint FROM public._brain_workers WHERE kind='scheduler' AND seen_at>now()-make_interval(secs=>$1)")
            .bind(WORKER_FRESH_SECONDS as f64).fetch_one(&self.pool).await.map_err(crate::db::safe)
    }
    /// `SKIP LOCKED` lets independent processes claim disjoint jobs. Expired work can be delivered again.
    pub async fn claim(&self, queue: &str, lease_seconds: i32) -> Result<Option<Job>, String> {
        if !crate::contract::identifier(queue) || !(1..=3600).contains(&lease_seconds) {
            return Err("invalid queue or lease (1..3600 seconds)".into());
        }
        let mut connection = self.pool.acquire().await.map_err(|error| crate::db::safe_operation(error, "queue.claim", "acquire"))?;
        Self::claim_on(&mut connection, queue, lease_seconds).await
    }
    /// The connection is acquired before either mutation. Errors after this boundary are never retried here.
    async fn claim_on(connection: &mut PgConnection, queue: &str, lease_seconds: i32) -> Result<Option<Job>, String> {
        sqlx::query("UPDATE _brain_jobs SET state='dead',finished_at=now(),last_error='Worker lease expired after final attempt',lease_token=NULL,lease_until=NULL WHERE queue=$1 AND state='running' AND lease_until<now() AND attempts>=max_attempts").bind(queue).execute(&mut *connection).await.map_err(|error| crate::db::safe_operation(error, "queue.claim", "execute"))?;
        let row = sqlx::query("WITH next AS (SELECT id FROM _brain_jobs WHERE queue=$1 AND attempts<max_attempts AND ((state='pending' AND available_at<=now()) OR (state='running' AND lease_until<now())) ORDER BY priority DESC,available_at,created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE _brain_jobs j SET state='running',attempts=attempts+1,lease_token=$2,lease_until=now()+make_interval(secs=>$3) FROM next WHERE j.id=next.id RETURNING j.id,j.queue,j.payload,j.attempts,j.lease_token")
            .bind(queue).bind(Uuid::new_v4()).bind(lease_seconds as f64).fetch_optional(&mut *connection).await.map_err(|error| crate::db::safe_operation(error, "queue.claim", "execute"))?;
        Ok(row.map(|r| Job {
            id: r.get("id"),
            queue: r.get("queue"),
            payload: r.get("payload"),
            attempt: r.get("attempts"),
            lease: r.get("lease_token"),
        }))
    }
    pub async fn ack(&self, job: &Job) -> Result<bool, String> {
        sqlx::query("UPDATE _brain_jobs SET state='succeeded',finished_at=now(),lease_token=NULL,lease_until=NULL,last_error=NULL WHERE id=$1 AND state='running' AND lease_token=$2 AND lease_until>now()")
            .bind(job.id).bind(job.lease).execute(&self.pool).await.map(|r|r.rows_affected()==1).map_err(|error| crate::db::safe_operation(error, "queue.ack", "query"))
    }
    /// Do not persist arbitrary handler errors: they may contain credentials or payload data.
    pub async fn fail(&self, job: &Job) -> Result<bool, String> {
        self.fail_after(job, Backoff::default().seconds(job.attempt)).await
    }
    /// Release a failed attempt: retry after `delay` seconds, or `dead` when attempts are exhausted.
    pub async fn fail_after(&self, job: &Job, delay: u64) -> Result<bool, String> {
        sqlx::query("UPDATE _brain_jobs SET state=CASE WHEN attempts>=max_attempts THEN 'dead' ELSE 'pending' END,available_at=now()+make_interval(secs=>$3),finished_at=CASE WHEN attempts>=max_attempts THEN now() ELSE NULL END,last_error='Handler failed or timed out; inspect the queue handler.jsbrain',lease_token=NULL,lease_until=NULL WHERE id=$1 AND state='running' AND lease_token=$2 AND lease_until>now()")
            .bind(job.id).bind(job.lease).bind(delay.min(86400) as f64).execute(&self.pool).await.map(|r|r.rows_affected()==1).map_err(|error| crate::db::safe_operation(error, "queue.fail", "query"))
    }
    pub async fn status(&self, id: Uuid) -> Result<Option<Value>, String> {
        sqlx::query_scalar("SELECT jsonb_build_object('id',id,'queue',queue,'state',state,'attempts',attempts,'maxAttempts',max_attempts,'priority',priority,'lastError',last_error) FROM _brain_jobs WHERE id=$1")
            .bind(id).fetch_optional(&self.pool).await.map_err(crate::db::safe)
    }
    /// Explicit retention; never deletes active work. Idempotency keys expire with retained job records.
    pub async fn prune(&self) -> Result<u64, String> {
        sqlx::query("DELETE FROM _brain_jobs WHERE id IN (SELECT id FROM _brain_jobs WHERE state IN ('succeeded','dead') AND finished_at<now()-interval '30 days' ORDER BY finished_at LIMIT 1000)").execute(&self.pool).await.map(|r|r.rows_affected()).map_err(crate::db::safe)
    }
    /// Fixed worker count, no prefetch backlog. A dropped handler future must cancel its owned I/O/children.
    pub async fn work<F, Fut>(
        &self,
        queue: &str,
        concurrency: usize,
        shutdown: tokio::sync::watch::Receiver<bool>,
        handler: F,
    ) -> Result<(), String>
    where
        F: Fn(Job) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let policy = QueuePolicy { concurrency, ..QueuePolicy::default() };
        self.work_with(queue, &policy, shutdown, handler).await
    }
    /// Workers driven by a [`QueuePolicy`]: priority order, per-attempt timeout, automatic lease renewal for long jobs,
    /// configured backoff and a drain grace period on shutdown (no new claims; in-flight jobs may finish).
    pub async fn work_with<F, Fut>(
        &self,
        queue: &str,
        policy: &QueuePolicy,
        shutdown: tokio::sync::watch::Receiver<bool>,
        handler: F,
    ) -> Result<(), String>
    where
        F: Fn(Job) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        if !crate::contract::identifier(queue) {
            return Err("workers need a valid queue and concurrency 1..8".into());
        }
        policy.validate().map_err(|e| format!("workers need a valid queue and concurrency 1..8; {e}"))?;
        // Liveness: best effort, a missing row must never stop work. `claim` reports a real database failure.
        let worker_id = Uuid::new_v4();
        let mut startup_stop = shutdown.clone();
        if *startup_stop.borrow() || startup_stop.has_changed().is_err() { return Ok(()) }
        tokio::select! {
            biased;
            _ = startup_stop.changed() => return Ok(()),
            _ = bounded_database("queue.worker.start", self.beat(worker_id, "worker", queue)) => {},
        }
        let beat = {
            let (q, name) = (self.clone(), queue.to_string());
            BackgroundTask(tokio::spawn(async move {
                loop {
                    tokio::time::sleep(WORKER_BEAT).await;
                    let _ = q.beat(worker_id, "worker", &name).await;
                }
            }))
        };
        let wake = Arc::new(tokio::sync::Notify::new());
        let listening = Arc::new(AtomicBool::new(false));
        let listener = BackgroundTask(tokio::spawn(wake_on_ready(self.pool.clone(), queue.to_string(), wake.clone(), listening.clone())));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..policy.concurrency {
            let (q, name, h, mut stop, p, wake, listening) = (self.clone(), queue.to_string(), handler.clone(), shutdown.clone(), policy.clone(), wake.clone(), listening.clone());
            tasks.spawn(async move {
                loop {
                    if *stop.borrow() { break }
                    // Armed before the claim, so a job committed between an empty claim and the sleep still wakes it.
                    let woken = wake.notified();
                    tokio::pin!(woken);
                    woken.as_mut().enable();
                    let Some(mut connection) = acquire_for_worker(|| q.pool.acquire(), &mut stop, CLAIM_ACQUIRE_RECOVERY).await? else { break };
                    let job = tokio::select! {
                        biased;
                        _ = stop.changed() => break,
                        job = Self::claim_on(&mut connection, &name, p.lease as i32) => job?,
                    };
                    drop(connection);
                    // Claims again at once while there is work; waits only on an empty queue.
                    let Some(job) = job else {
                        let idle = if listening.load(Ordering::Relaxed) { IDLE_POLL } else { IDLE_POLL_NO_WAKE };
                        tokio::select! { _ = stop.changed() => break, _ = &mut woken => {}, _ = tokio::time::sleep(idle) => {} }
                        continue;
                    };
                    if q.run_one(&job, &p, h(job.clone()), &mut stop).await? { break }
                }
                Ok::<(), String>(())
            });
        }
        let mut failed = false;
        while let Some(result) = tasks.join_next().await {
            if !matches!(result, Ok(Ok(()))) {
                failed = true;
                tasks.abort_all();
            }
        }
        drop(beat);
        drop(listener);
        let _ = bounded_database("queue.worker.leave", self.leave(worker_id)).await;
        if failed {
            return Err("worker stopped after a queue or task failure".into());
        }
        Ok(())
    }
    /// Run one claimed attempt; returns whether shutdown was requested meanwhile.
    async fn run_one<Fut: Future<Output = Result<(), String>>>(
        &self,
        job: &Job,
        p: &QueuePolicy,
        handler: Fut,
        stop: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<bool, String> {
        let deadline = Instant::now() + Duration::from_secs(p.timeout);
        let mut handler = Box::pin(handler);
        let mut beat = tokio::time::interval(Duration::from_secs((p.lease / 3).max(1)));
        beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        beat.tick().await;
        let mut stopping = *stop.borrow();
        let mut drain_until = stopping.then(|| Instant::now() + Duration::from_secs(p.drain));
        let outcome = loop {
            let limit = drain_until.map_or(deadline, |d| d.min(deadline));
            tokio::select! {
                biased;
                result = &mut handler => break Some(result),
                _ = tokio::time::sleep_until(limit.into()) => break None,
                _ = stop.changed(), if !stopping => {
                    stopping = true;
                    drain_until = Some(Instant::now() + Duration::from_secs(p.drain));
                }
                _ = beat.tick() => {
                    // A lost lease means another worker may own the job now: abandon without ack or fail.
                    let renewal = self.heartbeat(job, p.lease);
                    let renewal_deadline = Instant::now() + CLEANUP_BUDGET;
                    tokio::pin!(renewal);
                    loop {
                        let limit = drain_until.map_or(deadline, |d| d.min(deadline)).min(renewal_deadline);
                        tokio::select! {
                            biased;
                            _ = tokio::time::sleep_until(limit.into()) => {
                                // The renewal may have reached Postgres: never acknowledge or release an uncertain lease.
                                crate::db::diagnostic(&serde_json::json!({"schemaVersion":1,
                                    "event":"worker-lease-renewal-deadline","operation":"queue.heartbeat",
                                    "phase":"execute","private":true,"fix":"Inspect database locks and connection availability; the handler was canceled without acknowledging the uncertain lease","valuesCaptured":false}));
                                return Ok(stopping);
                            }
                            _ = stop.changed(), if !stopping => {
                                stopping = true;
                                drain_until = Some(Instant::now() + Duration::from_secs(p.drain));
                            }
                            renewed = &mut renewal => {
                                if !renewed? { return Ok(stopping) }
                                break;
                            }
                        }
                    }
                }
            }
        };
        // Cancel owned handler I/O before releasing its lease for another worker.
        drop(handler);
        if matches!(outcome, Some(Ok(()))) {
            bounded_database("queue.ack", self.ack(job)).await?;
        } else {
            bounded_database("queue.fail", self.fail_after(job, p.backoff.seconds(job.attempt))).await?;
        }
        Ok(stopping)
    }
}

/// Wake idle workers when a job of `queue` commits. Uses its own connection: a listener held from the shared pool could
/// starve the claims. When LISTEN does not work the workers simply keep polling at the shorter interval.
async fn wake_on_ready(pool: PgPool, queue: String, wake: Arc<tokio::sync::Notify>, listening: Arc<AtomicBool>) {
    let own = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect_lazy_with((*pool.connect_options()).clone());
    loop {
        if let Ok(mut listener) = sqlx::postgres::PgListener::connect_with(&own).await
            && listener.listen(READY_CHANNEL).await.is_ok()
        {
            listening.store(true, Ordering::Relaxed);
            // A reconnect may have missed a commit: claim once.
            wake.notify_waiters();
            while let Ok(note) = listener.recv().await {
                if note.payload() == queue {
                    wake.notify_waiters();
                }
            }
        }
        listening.store(false, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
async fn notify_ready(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, queue: &str) -> Result<(), String> {
    sqlx::query("SELECT pg_notify($1,$2)").bind(READY_CHANNEL).bind(queue).execute(&mut **tx).await.map(|_| ()).map_err(crate::db::safe)
}

/// Maximum pending or running jobs per queue.
pub const QUEUE_CAPACITY: i64 = 10000;

/// Transactional outbox used by resource hooks: the row and its job commit or roll back together.
pub async fn enqueue_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    queue: &str,
    payload: Value,
    key: Option<&str>,
    max_attempts: i32,
) -> Result<Uuid, String> {
    enqueue_tx_with(tx, queue, payload, &Enqueue { key, attempts: Some(max_attempts), ..Enqueue::default() }).await
}
/// [`enqueue_tx`] with priority and delay.
pub async fn enqueue_tx_with(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    queue: &str,
    payload: Value,
    options: &Enqueue<'_>,
) -> Result<Uuid, String> {
    let (key, max_attempts) = (options.key, options.attempts.unwrap_or(3));
    if !(-100..=100).contains(&options.priority) || !(0..=2_592_000).contains(&options.delay_seconds) {
        return Err("invalid job; priority -100..100 and delay 0..2592000 seconds".into());
    }
    if !crate::contract::identifier(queue)
        || !(1..=10).contains(&max_attempts)
        || key.is_some_and(|k| k.is_empty() || k.len() > 128)
        || payload.to_string().len() > 65536
    {
        return Err("invalid job; queue must be an identifier, payload <=64 KB, key 1..128 bytes, attempts 1..10".into());
    }
    // Serialize the capacity check per queue only (two-int key space): producers on different queues never wait
    // for each other, and the transaction-level lock is released on commit or rollback.
    sqlx::query("SELECT pg_advisory_xact_lock(728196411, hashtext($1))")
        .bind(queue)
        .execute(&mut **tx)
        .await
        .map_err(crate::db::safe)?;
    if let Some(key) = key {
        let id: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM _brain_jobs WHERE queue=$1 AND idempotency_key=$2")
                .bind(queue)
                .bind(key)
                .fetch_optional(&mut **tx)
                .await
                .map_err(crate::db::safe)?;
        if let Some(id) = id {
            return Ok(id);
        }
    }
    // Bounded count: stops at the cap instead of scanning every active job (partial indexes on queue+state).
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM (SELECT 1 FROM _brain_jobs WHERE queue=$1 AND state IN ('pending','running') LIMIT $2) bounded",
    )
    .bind(queue)
    .bind(QUEUE_CAPACITY)
    .fetch_one(&mut **tx)
    .await
    .map_err(crate::db::safe)?;
    if pending >= QUEUE_CAPACITY {
        return Err(format!("job queue `{queue}` is full ({QUEUE_CAPACITY} active jobs); drain workers before retrying"));
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO _brain_jobs(id,queue,payload,max_attempts,idempotency_key,priority,available_at) VALUES ($1,$2,$3,$4,$5,$6,now()+make_interval(secs=>$7))").bind(id).bind(queue).bind(payload).bind(max_attempts).bind(key).bind(options.priority).bind(options.delay_seconds as f64).execute(&mut **tx).await.map_err(crate::db::safe)?;
    if options.delay_seconds == 0 {
        notify_ready(tx, queue).await?;
    }
    Ok(id)
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn fast_recovery() -> AcquireRecovery {
        AcquireRecovery { attempts: 3, budget: Duration::from_secs(1), delay: Duration::from_millis(1) }
    }

    #[tokio::test]
    async fn only_pre_statement_pool_timeouts_are_retried() {
        let (_send, mut stop) = tokio::sync::watch::channel(false);
        let mut attempts = 0;
        let result = acquire_for_worker(|| {
            attempts += 1;
            std::future::ready(if attempts < 3 { Err(sqlx::Error::PoolTimedOut) } else { Ok(42) })
        }, &mut stop, fast_recovery()).await.unwrap();
        assert_eq!((result, attempts), (Some(42), 3));
        for error in [sqlx::Error::PoolClosed, sqlx::Error::Protocol("synthetic-private-detail".into()),
            sqlx::Error::Io(std::io::Error::other("synthetic-private-detail"))] {
            let mut error = Some(error);
            let mut attempts = 0;
            let result = acquire_for_worker(|| {
                attempts += 1;
                std::future::ready(Err::<(), _>(error.take().expect("must not retry this error")))
            }, &mut stop, fast_recovery()).await;
            assert!(result.is_err());
            assert_eq!(attempts, 1);
        }
    }

    #[tokio::test]
    async fn acquisition_recovery_has_attempt_and_absolute_time_limits() {
        let (_send, mut stop) = tokio::sync::watch::channel(false);
        let mut attempts = 0;
        let result = acquire_for_worker(|| {
            attempts += 1;
            std::future::ready(Err::<(), _>(sqlx::Error::PoolTimedOut))
        }, &mut stop, fast_recovery()).await;
        assert!(result.is_err());
        assert_eq!(attempts, 3);
        let recovery = AcquireRecovery { budget: Duration::from_millis(10), ..fast_recovery() };
        let result = tokio::time::timeout(Duration::from_secs(1), acquire_for_worker(
            || std::future::pending::<Result<(), sqlx::Error>>(), &mut stop, recovery,
        )).await.expect("one absolute deadline bounds an acquisition that never resolves");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_cancels_acquisition_and_backoff_without_another_attempt() {
        for pending in [false, true] {
            let (send, mut stop) = tokio::sync::watch::channel(false);
            let (entered, received) = tokio::sync::oneshot::channel();
            let mut entered = Some(entered);
            let attempts = Arc::new(AtomicUsize::new(0));
            let count = attempts.clone();
            let task = tokio::spawn(async move {
                acquire_for_worker(|| {
                    count.fetch_add(1, Ordering::SeqCst);
                    if let Some(entered) = entered.take() { let _ = entered.send(()); }
                    async move {
                        if pending { std::future::pending::<()>().await; }
                        Err::<(), _>(sqlx::Error::PoolTimedOut)
                    }
                }, &mut stop, AcquireRecovery { delay: Duration::from_secs(10), ..CLAIM_ACQUIRE_RECOVERY }).await
            });
            received.await.unwrap();
            send.send(true).unwrap();
            let result = tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap().unwrap();
            assert_eq!(result, None);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn dropping_background_task_cancels_its_future() {
        let (entered, received) = tokio::sync::oneshot::channel();
        let (held, dropped) = tokio::sync::oneshot::channel::<()>();
        let background = BackgroundTask(tokio::spawn(async move {
            let _held = held;
            let _ = entered.send(());
            std::future::pending::<()>().await;
        }));
        received.await.unwrap();
        drop(background);
        assert!(tokio::time::timeout(Duration::from_secs(1), dropped).await.unwrap().is_err());
    }

    #[tokio::test]
    #[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
    async fn postgres_acquisition_recovers_before_claim_without_consuming_an_attempt() {
        let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("use the owned backend test harness");
        let setup = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&setup, &[]).await.unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(1)
            .acquire_timeout(Duration::from_millis(30)).connect(&url).await.unwrap();
        let queue = format!("recover_{}", Uuid::new_v4().simple());
        let q = Queue::new(setup.clone());
        let id = q.enqueue(&queue, serde_json::json!({}), None, 3).await.unwrap();
        let held = pool.acquire().await.unwrap();
        let (_send, mut stop) = tokio::sync::watch::channel(false);
        let (entered, mut received) = tokio::sync::mpsc::channel(3);
        let task = tokio::spawn({
            let pool = pool.clone();
            async move {
                acquire_for_worker(|| {
                    let _ = entered.try_send(());
                    pool.acquire()
                }, &mut stop, fast_recovery()).await
            }
        });
        tokio::time::timeout(Duration::from_secs(2), received.recv()).await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(2), received.recv()).await.unwrap().unwrap();
        let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM _brain_jobs WHERE id=$1")
            .bind(id).fetch_one(&setup).await.unwrap();
        assert_eq!(attempts, 0, "waiting for a connection executes no claim SQL");
        drop(held);
        let mut connection = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap().unwrap();
        let job = Queue::claim_on(&mut connection, &queue, 30).await.unwrap().unwrap();
        assert_eq!((job.id, job.attempt), (id, 1));
        drop(connection);
        assert!(q.ack(&job).await.unwrap());
        assert!(q.claim(&queue, 30).await.unwrap().is_none());
        sqlx::query("DELETE FROM _brain_jobs WHERE id=$1").bind(id).execute(&setup).await.unwrap();
        pool.close().await;
        setup.close().await;
    }

    async fn owned_test_pools() -> (PgPool, PgPool, String) {
        let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("use the owned backend test harness");
        let setup = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&setup, &[]).await.unwrap();
        let application = format!("queue_test_{}", Uuid::new_v4().simple());
        let options = url.parse::<sqlx::postgres::PgConnectOptions>().unwrap().application_name(&application);
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(2).connect_with(options).await.unwrap();
        (setup, pool, application)
    }

    async fn wait_for_lock(setup: &PgPool, application: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock')")
                    .bind(application).fetch_one(setup).await.unwrap();
                if waiting { break }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("the owned renewal must actually be blocked before shutdown");
    }

    #[tokio::test]
    #[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
    async fn postgres_blocked_renewal_obeys_shutdown_and_preserves_uncertain_lease() {
        let (setup, pool, application) = owned_test_pools().await;
        let q = Queue::new(pool.clone());
        let name = format!("renew_{}", Uuid::new_v4().simple());
        let id = q.enqueue(&name, serde_json::json!({}), None, 3).await.unwrap();
        let job = q.claim(&name, 30).await.unwrap().unwrap();
        let mut lock = setup.begin().await.unwrap();
        sqlx::query("SELECT id FROM _brain_jobs WHERE id=$1 FOR UPDATE").bind(id).fetch_one(&mut *lock).await.unwrap();
        let (send, mut stop) = tokio::sync::watch::channel(false);
        let (held, dropped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn({
            let q = q.clone();
            async move {
                // The private attempt driver accepts an accelerated heartbeat; public policy admission is unchanged.
                let policy = QueuePolicy { lease: 3, timeout: 30, drain: 0, ..Default::default() };
                q.run_one(&job, &policy, async move {
                    let _held = held;
                    std::future::pending::<Result<(), String>>().await
                }, &mut stop).await
            }
        });
        wait_for_lock(&setup, &application).await;
        send.send(true).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap());
        assert!(dropped.await.is_err(), "the uncertain lease cancels its handler");
        lock.rollback().await.unwrap();
        let row: (String, i32) = sqlx::query_as("SELECT state,attempts FROM _brain_jobs WHERE id=$1").bind(id).fetch_one(&setup).await.unwrap();
        assert_eq!(row, ("running".into(), 1), "neither ack nor release is safe after an uncertain renewal");
        sqlx::query("DELETE FROM _brain_jobs WHERE id=$1").bind(id).execute(&setup).await.unwrap();
        pool.close().await;
        setup.close().await;
    }

    #[tokio::test]
    #[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
    async fn postgres_blocked_ack_has_a_finite_cleanup_budget() {
        let (setup, pool, application) = owned_test_pools().await;
        let q = Queue::new(pool.clone());
        let name = format!("ack_{}", Uuid::new_v4().simple());
        let id = q.enqueue(&name, serde_json::json!({}), None, 3).await.unwrap();
        let job = q.claim(&name, 30).await.unwrap().unwrap();
        let mut lock = setup.begin().await.unwrap();
        sqlx::query("SELECT id FROM _brain_jobs WHERE id=$1 FOR UPDATE").bind(id).fetch_one(&mut *lock).await.unwrap();
        let (_send, mut stop) = tokio::sync::watch::channel(false);
        let task = tokio::spawn({
            let q = q.clone();
            async move { q.run_one(&job, &QueuePolicy::default(), async { Ok(()) }, &mut stop).await }
        });
        wait_for_lock(&setup, &application).await;
        let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
        assert!(result.unwrap_err().contains("cleanup deadline"));
        lock.rollback().await.unwrap();
        sqlx::query("DELETE FROM _brain_jobs WHERE id=$1").bind(id).execute(&setup).await.unwrap();
        pool.close().await;
        setup.close().await;
    }

    #[tokio::test]
    #[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
    async fn postgres_canceling_worker_releases_its_listener_connection() {
        let (setup, pool, application) = owned_test_pools().await;
        let q = Queue::new(pool.clone());
        let name = format!("cancel_{}", Uuid::new_v4().simple());
        let id = q.enqueue(&name, serde_json::json!({}), None, 3).await.unwrap();
        let (_send, stop) = tokio::sync::watch::channel(false);
        let (entered, mut received) = tokio::sync::mpsc::channel(1);
        let task = tokio::spawn({
            let (q, name) = (q.clone(), name.clone());
            async move { q.work(&name, 1, stop, move |_| {
                let entered = entered.clone();
                async move { let _ = entered.send(()).await; std::future::pending::<Result<(), String>>().await }
            }).await }
        });
        tokio::time::timeout(Duration::from_secs(3), received.recv()).await.unwrap().unwrap();
        // Observe the separate LISTEN connection, not merely the handler's pool connection.
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let listeners: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND query ILIKE 'LISTEN%'")
                    .bind(&application).fetch_one(&setup).await.unwrap();
                if listeners > 0 { break }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        pool.close().await;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let connections: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1")
                    .bind(&application).fetch_one(&setup).await.unwrap();
                if connections == 0 { break }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("canceling a worker must not leave its LISTEN connection detached");
        sqlx::query("DELETE FROM _brain_jobs WHERE id=$1").bind(id).execute(&setup).await.unwrap();
        setup.close().await;
    }

    #[tokio::test]
    #[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
    async fn postgres_sql_errors_are_not_acquisition_retries() {
        let (setup, pool, _) = owned_test_pools().await;
        let error = sqlx::query("SELECT 1 / 0").execute(&pool).await.unwrap_err();
        assert_eq!(crate::db::sqlstate(&error).as_deref(), Some("22012"));
        let mut error = Some(error);
        let mut attempts = 0;
        let (_send, mut stop) = tokio::sync::watch::channel(false);
        let result = acquire_for_worker(|| {
            attempts += 1;
            std::future::ready(Err::<(), _>(error.take().expect("SQL errors must not be retried")))
        }, &mut stop, fast_recovery()).await;
        assert!(result.is_err());
        assert_eq!(attempts, 1);
        pool.close().await;
        setup.close().await;
    }

    #[tokio::test]
    #[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
    async fn postgres_timed_out_handler_is_dropped_before_lease_release() {
        let (setup, pool, application) = owned_test_pools().await;
        let q = Queue::new(pool.clone());
        let name = format!("release_{}", Uuid::new_v4().simple());
        let id = q.enqueue(&name, serde_json::json!({}), None, 3).await.unwrap();
        let job = q.claim(&name, 30).await.unwrap().unwrap();
        let mut lock = setup.begin().await.unwrap();
        sqlx::query("SELECT id FROM _brain_jobs WHERE id=$1 FOR UPDATE").bind(id).fetch_one(&mut *lock).await.unwrap();
        let (_send, mut stop) = tokio::sync::watch::channel(false);
        let (held, mut dropped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn({
            let q = q.clone();
            async move {
                q.run_one(&job, &QueuePolicy { timeout: 1, ..Default::default() }, async move {
                    let _held = held;
                    std::future::pending::<Result<(), String>>().await
                }, &mut stop).await
            }
        });
        wait_for_lock(&setup, &application).await;
        assert!(matches!(dropped.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Closed)),
            "owned handler work must be canceled before the lease-release query starts");
        lock.rollback().await.unwrap();
        assert!(!tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap());
        let state: String = sqlx::query_scalar("SELECT state FROM _brain_jobs WHERE id=$1")
            .bind(id).fetch_one(&setup).await.unwrap();
        assert_eq!(state, "pending");
        sqlx::query("DELETE FROM _brain_jobs WHERE id=$1").bind(id).execute(&setup).await.unwrap();
        pool.close().await;
        setup.close().await;
    }
}
