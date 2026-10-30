//! Real owned Postgres acceptance; never falls back to a developer database.
use brain_backend::{
    auth::{Auth, Credentials},
    db,
    links::{self, Create},
};
use sqlx::ConnectOptions;
use std::str::FromStr;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires an explicitly owned disposable PostgreSQL cluster"]
async fn short_links_are_unique_owner_scoped_revocable_and_expiring() {
    let url =
        std::env::var("BRAIN_BACKEND_TEST_URL").expect("explicit owned test cluster required");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_links_{}", Uuid::new_v4().simple());
    // Audited identifier: fixed ASCII prefix and UUID hex, with no request data.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&base)
        .await
        .unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url)
        .unwrap()
        .database(&name);
    let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
    db::migrate(&pool, &[]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let owner = auth
        .signup(Credentials {
            email: "owner@example.test".into(),
            password: "Synthetic test password".into(),
        })
        .await
        .unwrap();
    let other = auth
        .signup(Credentials {
            email: "other@example.test".into(),
            password: "Another synthetic password".into(),
        })
        .await
        .unwrap();
    let origins = vec!["https://example.test".into()];
    let input = || Create {
        target: "https://example.test/guide?view=public".into(),
        code: Some("public-guide".into()),
        expires_in: Some(3600),
    };
    let (first, second) = tokio::join!(
        links::create(&pool, owner.id, input(), &origins),
        links::create(&pool, owner.id, input(), &origins)
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "one code has exactly one committed owner record"
    );
    let link = first.or(second).unwrap();
    assert_eq!(
        links::resolve(&pool, &link.code, &origins)
            .await
            .unwrap()
            .as_deref(),
        Some("https://example.test/guide?view=public")
    );
    assert!(!links::revoke(&pool, link.id, other.id).await.unwrap());
    assert!(links::revoke(&pool, link.id, owner.id).await.unwrap());
    assert!(
        links::resolve(&pool, &link.code, &origins)
            .await
            .unwrap()
            .is_none()
    );
    let expires = links::create(
        &pool,
        owner.id,
        Create {
            target: "https://example.test/".into(),
            code: None,
            expires_in: Some(60),
        },
        &origins,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE _brain_short_links SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(expires.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        links::resolve(&pool, &expires.code, &origins)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        links::resolve(&pool, "../unsafe", &origins)
            .await
            .unwrap()
            .is_none()
    );
    pool.close().await;
    // The database name is generated above and is owned by this test only.
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
        .execute(&base)
        .await
        .unwrap();
    base.close().await;
}
