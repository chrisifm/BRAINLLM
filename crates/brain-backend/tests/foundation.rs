//! Synthetic owned-cluster acceptance. Explicit env only; no developer database fallback.
use brain_backend::{
    auth,
    contract::{Field, Hooks, Policy, Resource},
    db,
};
use sqlx::ConnectOptions;
use std::{collections::BTreeMap, str::FromStr};
use uuid::Uuid;
fn resource() -> Resource {
    Resource {
        name: "search_fixture".into(),
        fields: BTreeMap::from([
            (
                "title".into(),
                Field {
                    kind: "str".into(),
                    required: true,
                    ..Default::default()
                },
            ),
            (
                "private_note".into(),
                Field {
                    kind: "str".into(),
                    required: false,
                    ..Default::default()
                },
            ),
        ]),
        search: vec!["title".into()],
        policy: Policy::default(),
        hooks: Hooks::default(),
        constraints: Default::default(),
        query: Default::default(),
    }
}
#[test]
fn search_contract_never_infers_fields_or_accepts_unknown_objects_or_duplicates() {
    let mut r = resource();
    assert!(r.validate().is_ok());
    r.search = vec!["missing".into()];
    assert!(r.validate().is_err());
    r.search = vec!["title".into(), "title".into()];
    assert!(r.validate().is_err());
    r.search = Vec::new();
    assert!(r.validate().is_ok());
    let mut encoded = serde_json::to_value(&r).unwrap();
    encoded.as_object_mut().unwrap().remove("search");
    let decoded: Resource = serde_json::from_value(encoded).unwrap();
    assert!(decoded.search.is_empty());
}
async fn database(base: &sqlx::PgPool, url: &str) -> sqlx::PgPool {
    // Audited generated identifier: fixed ASCII prefix and UUID hex, never a request value.
    let name = format!("brain_foundation_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(base)
        .await
        .unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(url)
        .unwrap()
        .database(&name);
    db::connect(options.to_url_lossy().as_str()).await.unwrap()
}
#[tokio::test]
#[ignore = "Run with the explicitly owned disposable Postgres harness"]
async fn postgres_migrations_search_scope_session_serialization_and_storage_outbox() {
    let url =
        std::env::var("BRAIN_BACKEND_TEST_URL").expect("owned disposable harness URL required");
    let base = db::connect(&url).await.unwrap();
    let pool = database(&base, &url).await;
    db::migrate(&pool, &[]).await.unwrap();
    db::migrate(&pool, &[]).await.unwrap();
    db::verify(&pool, &[]).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _brain_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        13
    );
    assert_eq!(
        sqlx::query_as::<_, (i32, String)>(
            "SELECT version,name FROM _brain_migrations ORDER BY version"
        )
        .fetch_all(&pool)
        .await
        .unwrap(),
        vec![
            (1, "backend".into()),
            (2, "jobs".into()),
            (3, "foundation".into()),
            (4, "links".into()),
            (5, "auth_flows".into()),
            (6, "webhooks".into()),
            (7, "search_empty_fields".into()),
            (8, "analytics".into()),
            (9, "domain_queues".into()),
            (10, "api_keys".into()),
            (11, "data_queries".into()),
            (12, "data_queries_slugs".into()),
            (13, "endpoints_platform".into())
        ]
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SHOW statement_timeout")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "5s"
    );
    let r = resource();
    db::migrate(&pool, &[r.clone()]).await.unwrap();
    let owners = [Uuid::new_v4(), Uuid::new_v4()];
    for (n, owner) in owners.iter().enumerate() {
        sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role)VALUES($1,$2,'synthetic-unusable','candidate')").bind(owner).bind(format!("owner{n}@example.test")).execute(&pool).await.unwrap();
    }
    // 100000 synthetic rows, batched under the runtime statement deadline. No private payload in logs.
    for batch in 0..20 {
        sqlx::query("INSERT INTO _brain_res_search_fixture(id,owner,data)SELECT gen_random_uuid(),CASE WHEN n%2=0 THEN $1 ELSE $2 END,jsonb_build_object('title',CASE WHEN n%1000=0 THEN 'Caf\u{e9} architecture selective' ELSE 'Common article number '||n::text END,'private_note','hidden_private_needle') FROM generate_series($3::integer,$4::integer)n")
            .bind(owners[0]).bind(owners[1]).bind(batch*5000).bind(batch*5000+4999).execute(&pool).await.unwrap();
    }
    sqlx::query("ANALYZE _brain_res_search_fixture")
        .execute(&pool)
        .await
        .unwrap();
    let rows = db::list(&pool, &r, Some(owners[0]), "cafe", 25, 0)
        .await
        .unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|row| row["owner"] == owners[0].to_string()));
    assert!(
        db::list(&pool, &r, Some(owners[1]), "selective", 25, 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        db::list(&pool, &r, Some(owners[0]), "hidden_private_needle", 25, 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        db::list(&pool, &r, Some(owners[0]), "CAF\u{c9}", 25, 0)
            .await
            .unwrap()
            .len()
            > 0
    );
    assert!(
        !db::list(&pool, &r, Some(owners[0]), "architecure", 25, 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !db::list(&pool, &r, Some(owners[0]), "ca", 25, 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        db::list(
            &pool,
            &r,
            Some(owners[0]),
            "word ".repeat(17).as_str(),
            25,
            0
        )
        .await
        .is_err()
    );
    // Actual measured plans: no enable_seqscan manipulation and no fabricated index-use assertion.
    let plan:serde_json::Value=sqlx::query_scalar("EXPLAIN(ANALYZE,BUFFERS,FORMAT JSON)SELECT id FROM _brain_res_search_fixture WHERE owner=$1 AND search_vector @@ websearch_to_tsquery('simple','selective')").bind(owners[0]).fetch_one(&pool).await.unwrap();
    // PostgreSQL 18 reports EXPLAIN row counts as JSON decimals (for example 100.00).
    let actual_rows = plan[0]["Plan"]["Actual Rows"]
        .as_f64()
        .expect("Measured EXPLAIN Actual Rows must be numeric");
    assert!(actual_rows.is_finite() && actual_rows > 0.0);
    // Same transaction cap lock, twenty competing creators, at most ten sessions.
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..20 {
        let p = pool.clone();
        let owner = owners[0];
        tasks.spawn(async move{
        let mut tx=p.begin().await.unwrap();auth::reserve_session(&mut tx,owner).await.unwrap();
        sqlx::query("INSERT INTO _brain_sessions(token_hash,user_id,expires_at)VALUES($1,$2,now()+interval '7 days')").bind(auth::digest(&Uuid::new_v4().to_string())).bind(owner).execute(&mut *tx).await.unwrap();tx.commit().await.unwrap();
    });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _brain_sessions WHERE user_id=$1")
            .bind(owners[0])
            .fetch_one(&pool)
            .await
            .unwrap(),
        10
    );
    // Expired surviving limit keys reset inside the upsert, independent of the bounded cleanup batch.
    let auth = auth::Auth::new(pool.clone()).await.unwrap();
    sqlx::query("INSERT INTO _brain_login_limits(key,attempts,expires_at)SELECT md5(n::text),10,now()-interval '1 second' FROM generate_series(1,1000)n").execute(&pool).await.unwrap();
    let email = "owner1@example.test";
    sqlx::query("INSERT INTO _brain_login_limits(key,attempts,expires_at)VALUES($1,10,now()-interval '1 second')").bind(auth::digest(email)).execute(&pool).await.unwrap();
    assert!(
        auth.login(auth::Credentials {
            email: email.into(),
            password: "not the real password".into()
        })
        .await
        .unwrap()
        .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT attempts FROM _brain_login_limits WHERE key=$1")
            .bind(auth::digest(email))
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    // Upload/user cascade persists deletion identities, never claims the filesystem was deleted.
    let upload = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO _brain_uploads(id,user_id,media_type,bytes)VALUES($1,$2,'image/png',12)",
    )
    .bind(upload)
    .bind(owners[0])
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM _brain_uploads WHERE id=$1")
        .bind(upload)
        .execute(&pool)
        .await
        .unwrap();
    let first = db::claim_media_deletions(&pool, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(first.upload_id, upload);
    assert!(db::ack_media_deletion(&pool, &first).await.unwrap());
    assert!(!db::ack_media_deletion(&pool, &first).await.unwrap());
    let summary = db::cleanup(&pool, 7).await.unwrap();
    assert_eq!(summary["deleted"], 7);
    assert_eq!(summary["possiblyMore"], true);
    let checksum: String =
        sqlx::query_scalar("SELECT checksum FROM _brain_migrations WHERE version=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE _brain_migrations SET checksum='changed' WHERE version=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        db::migrate(&pool, &[r.clone()])
            .await
            .unwrap_err()
            .contains("checksum")
    );
    assert!(db::verify(&pool, &[r]).await.is_err());
    sqlx::query("UPDATE _brain_migrations SET checksum=$1 WHERE version=1")
        .bind(checksum)
        .execute(&pool)
        .await
        .unwrap();
    // Legacy two-migration baseline is adopted only after structural checks and retains fixture rows.
    let legacy = database(&base, &url).await;
    sqlx::raw_sql(include_str!("../migrations/0001_backend.sql"))
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../migrations/0002_jobs.sql"))
        .execute(&legacy)
        .await
        .unwrap();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role)VALUES($1,'legacy@example.test','preserve-exact','candidate')").bind(id).execute(&legacy).await.unwrap();
    db::migrate(&legacy, &[]).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT password_hash FROM _brain_users WHERE id=$1")
            .bind(id)
            .fetch_one(&legacy)
            .await
            .unwrap(),
        "preserve-exact"
    );
    let bad = database(&base, &url).await;
    sqlx::raw_sql("CREATE TABLE _brain_users(id text PRIMARY KEY)")
        .execute(&bad)
        .await
        .unwrap();
    assert!(
        db::migrate(&bad, &[])
            .await
            .unwrap_err()
            .contains("_brain_users")
    );
    bad.close().await;
    legacy.close().await;
    pool.close().await;
    base.close().await;
}
