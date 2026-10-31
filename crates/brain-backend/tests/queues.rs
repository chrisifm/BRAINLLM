//! Application queues: enqueue concurrency, capacity and (later) priorities, retries, dead letters and schedules.
use brain_backend::{db, jobs};
use serde_json::json;
use std::time::Duration;

async fn pool() -> sqlx::PgPool {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL")
        .expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let pool = db::connect(&url).await.unwrap();
    db::migrate(&pool, &[]).await.unwrap();
    pool
}

/// Producers on different queues never wait for each other; producers on one queue still serialize their capacity check.
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_enqueue_locks_per_queue_not_globally() {
    let pool = pool().await;
    let run = uuid::Uuid::new_v4().simple().to_string()[..10].to_string();
    let (first, second) = (format!("emails_{run}"), format!("reports_{run}"));
    let mut a = pool.begin().await.unwrap();
    jobs::enqueue_tx(&mut a, &first, json!({"n":1}), None, 3).await.unwrap();
    // Transaction A is still open and holds its queue lock.
    let mut b = pool.begin().await.unwrap();
    let other = tokio::time::timeout(Duration::from_secs(3), jobs::enqueue_tx(&mut b, &second, json!({"n":2}), None, 3)).await;
    assert!(other.is_ok_and(|r| r.is_ok()), "a different queue must not wait for an open producer");
    b.commit().await.unwrap();
    let mut c = pool.begin().await.unwrap();
    let same = tokio::time::timeout(Duration::from_millis(500), jobs::enqueue_tx(&mut c, &first, json!({"n":3}), None, 3)).await;
    assert!(same.is_err(), "the same queue serializes its capacity check");
    drop(c);
    a.commit().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_jobs WHERE queue=ANY($1)")
        .bind(vec![first.clone(), second.clone()]).fetch_one(&pool).await.unwrap();
    assert_eq!(count, 2);
    sqlx::query("DELETE FROM _brain_jobs WHERE queue=ANY($1)").bind(vec![first, second]).execute(&pool).await.unwrap();
}

fn queue_name(prefix: &str) -> String {
    format!("{prefix}_{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}
async fn state(pool: &sqlx::PgPool, id: uuid::Uuid) -> String {
    sqlx::query_scalar("SELECT state FROM _brain_jobs WHERE id=$1").bind(id).fetch_one(pool).await.unwrap()
}

/// Higher priority is claimed first; equal priority keeps FIFO; delayed jobs wait for their time.
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_priorities_and_delays_order_claims() {
    let pool = pool().await;
    let q = jobs::Queue::new(pool.clone());
    let name = queue_name("prio");
    let enqueue = |n: i64, priority: i16, delay: i64| {
        let (q, name) = (q.clone(), name.clone());
        async move { q.enqueue_with(&name, json!({"n":n}), &jobs::Enqueue { priority, delay_seconds: delay, ..Default::default() }).await.unwrap() }
    };
    enqueue(1, 0, 0).await;
    enqueue(2, -5, 0).await;
    enqueue(3, 10, 0).await;
    enqueue(4, 0, 0).await;
    enqueue(5, 100, 3600).await;
    let mut order = Vec::new();
    while let Some(job) = q.claim(&name, 30).await.unwrap() {
        order.push(job.payload["n"].as_i64().unwrap());
        q.ack(&job).await.unwrap();
    }
    assert_eq!(order, [3, 1, 4, 2], "priority desc, then FIFO; the delayed job is not due");
    let stats = q.stats(&name).await.unwrap();
    assert_eq!((stats["succeeded"].as_i64(), stats["delayed"].as_i64()), (Some(4), Some(1)), "{stats}");
    assert!(q.enqueue_with(&name, json!({}), &jobs::Enqueue { priority: 101, ..Default::default() }).await.is_err());
    sqlx::query("DELETE FROM _brain_jobs WHERE queue=$1").bind(&name).execute(&pool).await.unwrap();
}

/// Exhausted attempts land in the dead letters; they can be listed and revived with fresh attempts.
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_dead_letters_list_and_retry() {
    let pool = pool().await;
    let q = jobs::Queue::new(pool.clone());
    let name = queue_name("dead");
    let id = q.enqueue_with(&name, json!({"n":1}), &jobs::Enqueue { attempts: Some(2), ..Default::default() }).await.unwrap();
    let backoff = jobs::Backoff { kind: jobs::BackoffKind::Fixed, delay: 0, max: 0 };
    for _ in 0..2 {
        let job = q.claim(&name, 30).await.unwrap().expect("job is due");
        assert!(q.fail_after(&job, backoff.seconds(job.attempt)).await.unwrap());
    }
    assert_eq!(state(&pool, id).await, "dead");
    assert!(q.claim(&name, 30).await.unwrap().is_none(), "dead jobs are never claimed");
    let dead = q.dead(&name, 10).await.unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0]["attempts"], 2);
    assert!(dead[0].get("payload").is_none(), "dead letters omit payloads");
    assert_eq!(q.retry_dead(&name, Some(id), 10).await.unwrap(), 1);
    let job = q.claim(&name, 30).await.unwrap().expect("revived job is due");
    assert_eq!(job.attempt, 1, "a revived job starts with fresh attempts");
    q.ack(&job).await.unwrap();
    assert_eq!(q.retry_dead(&name, None, 10).await.unwrap(), 0);
    let exp = jobs::Backoff::default();
    assert_eq!([1, 2, 3, 9].map(|a| exp.seconds(a)), [2, 4, 8, 300]);
    sqlx::query("DELETE FROM _brain_jobs WHERE queue=$1").bind(&name).execute(&pool).await.unwrap();
}

/// A job longer than its lease keeps the lease alive and is delivered exactly once to concurrent workers.
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_long_jobs_renew_their_lease() {
    let pool = pool().await;
    let q = jobs::Queue::new(pool.clone());
    let name = queue_name("long");
    let id = q.enqueue(&name, json!({}), None, 1).await.unwrap();
    let runs = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let policy = jobs::QueuePolicy { lease: 10, timeout: 60, concurrency: 3, ..Default::default() };
    let (stop, recv) = tokio::sync::watch::channel(false);
    let counter = runs.clone();
    let worker = tokio::spawn({
        let (q, name) = (q.clone(), name.clone());
        async move {
            q.work_with(&name, &policy, recv, move |_job| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(14)).await;
                    Ok(())
                }
            }).await
        }
    });
    let start = std::time::Instant::now();
    while state(&pool, id).await != "succeeded" && start.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    stop.send(true).unwrap();
    worker.await.unwrap().unwrap();
    assert_eq!(state(&pool, id).await, "succeeded");
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1, "the renewed lease prevents a second delivery");
    sqlx::query("DELETE FROM _brain_jobs WHERE queue=$1").bind(&name).execute(&pool).await.unwrap();
}

/// Stopping a worker with `drain` lets the in-flight job finish; without it the attempt is released for retry.
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_workers_drain_in_flight_jobs() {
    let pool = pool().await;
    let q = jobs::Queue::new(pool.clone());
    for (drain, expected) in [(5, "succeeded"), (0, "pending")] {
        let name = queue_name("drain");
        let id = q.enqueue(&name, json!({}), None, 3).await.unwrap();
        let policy = jobs::QueuePolicy { drain, concurrency: 1, ..Default::default() };
        let (stop, recv) = tokio::sync::watch::channel(false);
        let (started, mut started_rx) = tokio::sync::mpsc::channel::<()>(1);
        let worker = tokio::spawn({
            let (q, name) = (q.clone(), name.clone());
            async move {
                q.work_with(&name, &policy, recv, move |_job| {
                    let started = started.clone();
                    async move {
                        let _ = started.send(()).await;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        Ok(())
                    }
                }).await
            }
        });
        tokio::time::timeout(Duration::from_secs(10), started_rx.recv()).await.unwrap();
        stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(10), worker).await.unwrap().unwrap().unwrap();
        assert_eq!(state(&pool, id).await, expected, "drain {drain}");
        sqlx::query("DELETE FROM _brain_jobs WHERE queue=$1").bind(&name).execute(&pool).await.unwrap();
    }
}

#[test]
fn queue_policy_validation_names_the_rule() {
    let bad = jobs::QueuePolicy { lease: 5, ..Default::default() };
    assert!(bad.validate().unwrap_err().contains("lease: 10..600"));
    let parsed: Result<jobs::QueuePolicy, _> = serde_json::from_value(json!({"attempts": 5, "backoff": {"type": "fixed", "delay": 30}, "extra": 1}));
    assert!(parsed.is_err(), "unknown keys are rejected");
    let parsed: jobs::QueuePolicy = serde_json::from_value(json!({"attempts": 5, "backoff": {"type": "fixed", "delay": 30}})).unwrap();
    assert_eq!((parsed.attempts, parsed.backoff.seconds(4), parsed.timeout), (5, 30, 60));
}

fn every_five_minutes(job: &str) -> brain_backend::schedules::CronSchedule {
    serde_json::from_value(json!({"schedule": "*/5 * * * *", "job": job, "payload": {"report": "daily"}})).unwrap()
}

#[test]
fn cron_slots_respect_the_catch_up_window() {
    use chrono::TimeZone;
    let s = every_five_minutes("reports");
    s.validate().unwrap();
    let at = |h, m, sec| chrono::Utc.with_ymd_and_hms(2026, 10, 31, h, m, sec).unwrap();
    assert_eq!(s.due(at(1, 10, 0)), Some(at(1, 10, 0)), "a slot is due at its exact time");
    assert_eq!(s.due(at(1, 12, 30)), Some(at(1, 10, 0)), "inside the 300 s catch-up window");
    let strict: brain_backend::schedules::CronSchedule = serde_json::from_value(json!({"schedule": "0 3 * * *", "job": "reports", "catchUp": 60})).unwrap();
    assert_eq!(strict.due(at(3, 0, 59)), Some(at(3, 0, 0)));
    assert_eq!(strict.due(at(3, 5, 0)), None, "a missed slot outside the window is skipped, not run late");
    assert_eq!(s.next(at(1, 10, 0)), Some(at(1, 15, 0)));
    let bad: brain_backend::schedules::CronSchedule = serde_json::from_value(json!({"schedule": "every day", "job": "reports"})).unwrap();
    assert!(bad.validate().unwrap_err().contains("5 fields"));
    assert!(serde_json::from_value::<brain_backend::schedules::CronSchedule>(json!({"schedule": "* * * * *", "job": "x", "when": 1})).is_err());
}

/// Many scheduler processes ticking the same slot create exactly one job; the next slot creates the next job.
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_cron_runs_each_slot_once_across_schedulers() {
    use chrono::TimeZone;
    let pool = pool().await;
    let q = jobs::Queue::new(pool.clone());
    let job = queue_name("cron");
    let schedules = vec![("daily_report".to_string(), every_five_minutes(&job))];
    let now = chrono::Utc.with_ymd_and_hms(2026, 10, 31, 1, 11, 0).unwrap();
    let ticks = (0..8).map(|_| {
        let (q, schedules) = (q.clone(), schedules.clone());
        tokio::spawn(async move { brain_backend::schedules::tick(&q, &schedules, now).await.unwrap() })
    });
    let mut ids = std::collections::HashSet::new();
    for t in ticks {
        for (_, _, id) in t.await.unwrap() {
            ids.insert(id);
        }
    }
    assert_eq!(ids.len(), 1, "eight concurrent schedulers agree on one job for the slot");
    brain_backend::schedules::tick(&q, &schedules, now + chrono::Duration::minutes(5)).await.unwrap();
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as("SELECT idempotency_key,payload FROM _brain_jobs WHERE queue=$1 ORDER BY idempotency_key")
        .bind(&job).fetch_all(&pool).await.unwrap();
    assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["cron:daily_report:2026-10-31T01:10:00Z", "cron:daily_report:2026-10-31T01:15:00Z"]);
    assert_eq!(rows[0].1["report"], "daily");
    sqlx::query("DELETE FROM _brain_jobs WHERE queue=$1").bind(&job).execute(&pool).await.unwrap();
}
