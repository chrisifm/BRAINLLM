//! Durable Postgres queue: at-least-once delivery, leased ownership and bounded workers. Handlers must be idempotent.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::{future::Future, time::Duration};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub queue: String,
    pub payload: Value,
    pub attempt: i32,
    pub lease: Uuid,
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
    /// `SKIP LOCKED` lets independent processes claim disjoint jobs. Expired work can be delivered again.
    pub async fn claim(&self, queue: &str, lease_seconds: i32) -> Result<Option<Job>, String> {
        if !crate::contract::identifier(queue) || !(1..=3600).contains(&lease_seconds) {
            return Err("invalid queue or lease (1..3600 seconds)".into());
        }
        sqlx::query("UPDATE _brain_jobs SET state='dead',finished_at=now(),last_error='Worker lease expired after final attempt',lease_token=NULL,lease_until=NULL WHERE queue=$1 AND state='running' AND lease_until<now() AND attempts>=max_attempts").bind(queue).execute(&self.pool).await.map_err(crate::db::safe)?;
        let row = sqlx::query("WITH next AS (SELECT id FROM _brain_jobs WHERE queue=$1 AND attempts<max_attempts AND ((state='pending' AND available_at<=now()) OR (state='running' AND lease_until<now())) ORDER BY available_at,created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE _brain_jobs j SET state='running',attempts=attempts+1,lease_token=$2,lease_until=now()+make_interval(secs=>$3) FROM next WHERE j.id=next.id RETURNING j.id,j.queue,j.payload,j.attempts,j.lease_token")
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
        let delay = 2_i32.pow(job.attempt.clamp(1, 10) as u32).min(300);
        sqlx::query("UPDATE _brain_jobs SET state=CASE WHEN attempts>=max_attempts THEN 'dead' ELSE 'pending' END,available_at=now()+make_interval(secs=>$3),finished_at=CASE WHEN attempts>=max_attempts THEN now() ELSE NULL END,last_error='Handler failed or timed out; inspect the queue handler.jsbrain',lease_token=NULL,lease_until=NULL WHERE id=$1 AND state='running' AND lease_token=$2 AND lease_until>now()")
            .bind(job.id).bind(job.lease).bind(delay as f64).execute(&self.pool).await.map(|r|r.rows_affected()==1).map_err(crate::db::safe)
    }
    pub async fn status(&self, id: Uuid) -> Result<Option<Value>, String> {
        sqlx::query_scalar("SELECT jsonb_build_object('id',id,'queue',queue,'state',state,'attempts',attempts,'maxAttempts',max_attempts,'lastError',last_error) FROM _brain_jobs WHERE id=$1")
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
        if !(1..=8).contains(&concurrency) || !crate::contract::identifier(queue) {
            return Err("workers need a valid queue and concurrency 1..8".into());
        }
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..concurrency {
            let (q, name, h, mut stop) = (
                self.clone(),
                queue.to_string(),
                handler.clone(),
                shutdown.clone(),
            );
            tasks.spawn(async move {
                loop {
                    if *stop.borrow() { break }
                    let job = tokio::select! {
                        biased;
                        _ = stop.changed() => break,
                        job = q.claim(&name,90) => job?,
                    };
                    let Some(job) = job else {
                        tokio::select! { _ = stop.changed() => break, _ = tokio::time::sleep(Duration::from_millis(250)) => {} }
                        continue;
                    };
                    let outcome = tokio::select! {
                        biased;
                        _ = stop.changed() => { let _ = q.fail(&job).await; break },
                        result = tokio::time::timeout(Duration::from_secs(60), h(job.clone())) => result,
                    };
                    if matches!(outcome, Ok(Ok(()))) { q.ack(&job).await?; } else { q.fail(&job).await?; }
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
    sqlx::query("INSERT INTO _brain_jobs(id,queue,payload,max_attempts,idempotency_key) VALUES ($1,$2,$3,$4,$5)").bind(id).bind(queue).bind(payload).bind(max_attempts).bind(key).execute(&mut **tx).await.map_err(crate::db::safe)?;
    Ok(id)
}
