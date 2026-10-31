//! Durable Postgres queue: at-least-once delivery, leased ownership and bounded workers. Handlers must be idempotent.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::{future::Future, time::{Duration, Instant}};
use uuid::Uuid;

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
}
impl Default for QueuePolicy {
    fn default() -> Self {
        Self { priority: 0, attempts: 3, backoff: Backoff::default(), timeout: 60, lease: 90, drain: 0, concurrency: 2 }
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
            .bind(job.id).bind(job.lease).bind(lease_seconds.clamp(1, 3600) as f64).execute(&self.pool).await.map(|r| r.rows_affected() == 1).map_err(crate::db::safe)
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
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(revived)
    }
    /// Job counts by state; `delayed` are pending jobs not yet due.
    pub async fn stats(&self, queue: &str) -> Result<Value, String> {
        if !crate::contract::identifier(queue) {
            return Err("invalid queue".into());
        }
        sqlx::query_scalar("SELECT jsonb_build_object('queue',$1::text,'pending',count(*) FILTER (WHERE state='pending' AND available_at<=now()),'delayed',count(*) FILTER (WHERE state='pending' AND available_at>now()),'running',count(*) FILTER (WHERE state='running'),'succeeded',count(*) FILTER (WHERE state='succeeded'),'dead',count(*) FILTER (WHERE state='dead')) FROM _brain_jobs WHERE queue=$1")
            .bind(queue).fetch_one(&self.pool).await.map_err(crate::db::safe)
    }
    /// `SKIP LOCKED` lets independent processes claim disjoint jobs. Expired work can be delivered again.
    pub async fn claim(&self, queue: &str, lease_seconds: i32) -> Result<Option<Job>, String> {
        if !crate::contract::identifier(queue) || !(1..=3600).contains(&lease_seconds) {
            return Err("invalid queue or lease (1..3600 seconds)".into());
        }
        sqlx::query("UPDATE _brain_jobs SET state='dead',finished_at=now(),last_error='Worker lease expired after final attempt',lease_token=NULL,lease_until=NULL WHERE queue=$1 AND state='running' AND lease_until<now() AND attempts>=max_attempts").bind(queue).execute(&self.pool).await.map_err(crate::db::safe)?;
        let row = sqlx::query("WITH next AS (SELECT id FROM _brain_jobs WHERE queue=$1 AND attempts<max_attempts AND ((state='pending' AND available_at<=now()) OR (state='running' AND lease_until<now())) ORDER BY priority DESC,available_at,created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE _brain_jobs j SET state='running',attempts=attempts+1,lease_token=$2,lease_until=now()+make_interval(secs=>$3) FROM next WHERE j.id=next.id RETURNING j.id,j.queue,j.payload,j.attempts,j.lease_token")
            .bind(queue).bind(Uuid::new_v4()).bind(lease_seconds as f64).fetch_optional(&self.pool).await.map_err(crate::db::safe)?;
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
            .bind(job.id).bind(job.lease).execute(&self.pool).await.map(|r|r.rows_affected()==1).map_err(crate::db::safe)
    }
    /// Do not persist arbitrary handler errors: they may contain credentials or payload data.
    pub async fn fail(&self, job: &Job) -> Result<bool, String> {
        self.fail_after(job, Backoff::default().seconds(job.attempt)).await
    }
    /// Release a failed attempt: retry after `delay` seconds, or `dead` when attempts are exhausted.
    pub async fn fail_after(&self, job: &Job, delay: u64) -> Result<bool, String> {
        sqlx::query("UPDATE _brain_jobs SET state=CASE WHEN attempts>=max_attempts THEN 'dead' ELSE 'pending' END,available_at=now()+make_interval(secs=>$3),finished_at=CASE WHEN attempts>=max_attempts THEN now() ELSE NULL END,last_error='Handler failed or timed out; inspect the queue handler.jsbrain',lease_token=NULL,lease_until=NULL WHERE id=$1 AND state='running' AND lease_token=$2 AND lease_until>now()")
            .bind(job.id).bind(job.lease).bind(delay.min(86400) as f64).execute(&self.pool).await.map(|r|r.rows_affected()==1).map_err(crate::db::safe)
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
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..policy.concurrency {
            let (q, name, h, mut stop, p) = (self.clone(), queue.to_string(), handler.clone(), shutdown.clone(), policy.clone());
            tasks.spawn(async move {
                loop {
                    if *stop.borrow() { break }
                    let job = tokio::select! {
                        biased;
                        _ = stop.changed() => break,
                        job = q.claim(&name, p.lease as i32) => job?,
                    };
                    let Some(job) = job else {
                        tokio::select! { _ = stop.changed() => break, _ = tokio::time::sleep(Duration::from_millis(250)) => {} }
                        continue;
                    };
                    if q.run_one(&job, &p, h(job.clone()), &mut stop).await? { break }
                }
                Ok::<(), String>(())
            });
        }
        while let Some(result) = tasks.join_next().await {
            if !matches!(result, Ok(Ok(()))) {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return Err("worker stopped after a queue or task failure".into());
            }
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
        let mut handler = std::pin::pin!(handler);
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
                    if !self.heartbeat(job, p.lease).await? { return Ok(stopping) }
                }
            }
        };
        if matches!(outcome, Some(Ok(()))) {
            self.ack(job).await?;
        } else {
            self.fail_after(job, p.backoff.seconds(job.attempt)).await?;
        }
        Ok(stopping)
    }
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
    Ok(id)
}
