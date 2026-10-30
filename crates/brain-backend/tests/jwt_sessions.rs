//! Disposable Postgres only: rotation, replay revocation, version changes and disabled users.
use brain_backend::{
    auth::{Auth, Credentials},
    jwt::{Config, Jwt},
};
use uuid::Uuid;
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn database_sessions_enforce_refresh_rotation_reuse_revocation_and_auth_version() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL")
        .expect("Use the disposable Postgres harness; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let email = format!("jwt-{}@example.test", Uuid::new_v4().simple());
    let credentials = || Credentials {
        email: email.clone(),
        password: "synthetic-jwt-account-password".into(),
    };
    let user = auth.signup(credentials()).await.unwrap();
    let jwt = Jwt::new(
        pool.clone(),
        Config::new(
            "synthetic-jwt-signing-secret-32-bytes".into(),
            "test-issuer".into(),
            "test-audience".into(),
        )
        .unwrap(),
    );
    let (signed, opaque) = auth.login(credentials()).await.unwrap().unwrap();
    let first = jwt.issue(signed, &opaque).await.unwrap();
    assert_eq!(
        jwt.user(Some(&first.access_token))
            .await
            .unwrap()
            .unwrap()
            .id,
        user.id
    );
    let second = jwt
        .refresh(Some(&first.refresh_token))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first.refresh_token, second.refresh_token);
    assert!(
        jwt.user(Some(&second.access_token))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        jwt.refresh(Some(&first.refresh_token))
            .await
            .unwrap()
            .is_none()
    );
    assert!(jwt.user(Some(&first.access_token)).await.unwrap().is_none());
    assert!(
        jwt.user(Some(&second.access_token))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        jwt.refresh(Some(&second.refresh_token))
            .await
            .unwrap()
            .is_none()
    );
    assert!(auth.user(Some(&opaque)).await.unwrap().is_none());
    let (signed, opaque) = auth.login(credentials()).await.unwrap().unwrap();
    let versioned = jwt.issue(signed, &opaque).await.unwrap();
    sqlx::query("UPDATE _brain_users SET auth_version=auth_version+1 WHERE id=$1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        jwt.user(Some(&versioned.access_token))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        jwt.refresh(Some(&versioned.refresh_token))
            .await
            .unwrap()
            .is_none()
    );
    let (signed, opaque) = auth.login(credentials()).await.unwrap().unwrap();
    let disabled = jwt.issue(signed, &opaque).await.unwrap();
    sqlx::query("UPDATE _brain_users SET disabled_at=now() WHERE id=$1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        jwt.user(Some(&disabled.access_token))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        jwt.refresh(Some(&disabled.refresh_token))
            .await
            .unwrap()
            .is_none()
    );
    assert!(auth.user(Some(&opaque)).await.unwrap().is_none());
    assert!(auth.login(credentials()).await.unwrap().is_none());
    sqlx::query("DELETE FROM _brain_users WHERE id=$1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
