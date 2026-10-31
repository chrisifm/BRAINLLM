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
