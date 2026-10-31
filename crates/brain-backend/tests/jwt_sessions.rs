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
    // A concurrent refresh with the same token gets a short access token and no refresh token; the family lives.
    let late = jwt.refresh(Some(&first.refresh_token)).await.unwrap().expect("grace answer");
    assert!(late.refresh_token.is_empty());
    assert_eq!(jwt.user(Some(&late.access_token)).await.unwrap().unwrap().id, user.id);
    assert!(jwt.user(Some(&second.access_token)).await.unwrap().is_some());
    // Past the window the same replay is theft: the family and its session end.
    sqlx::query("UPDATE _brain_refresh_tokens SET used_at=now()-interval '1 minute' WHERE used_at IS NOT NULL AND family_id IN (SELECT id FROM _brain_refresh_families WHERE user_id=$1)")
        .bind(user.id).execute(&pool).await.unwrap();
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

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn refresh_grace_is_capped_and_ends_when_the_successor_was_used() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use the disposable Postgres harness");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let email = format!("grace-{}@example.test", Uuid::new_v4().simple());
    let credentials = || Credentials { email: email.clone(), password: "synthetic-jwt-account-password".into() };
    auth.signup(credentials()).await.unwrap();
    let key = "synthetic-jwt-signing-secret-32-bytes";
    let jwt = Jwt::new(pool.clone(), Config::new(key.into(), "i".into(), "a".into()).unwrap());
    let issue = async || { let (user, opaque) = auth.login(credentials()).await.unwrap().unwrap(); jwt.issue(user, &opaque).await.unwrap() };

    // the successor was already used: the old token is a replay even inside the window
    let a = issue().await;
    let b = jwt.refresh(Some(&a.refresh_token)).await.unwrap().unwrap();
    let c = jwt.refresh(Some(&b.refresh_token)).await.unwrap().unwrap();
    assert!(jwt.refresh(Some(&a.refresh_token)).await.unwrap().is_none());
    assert!(jwt.user(Some(&c.access_token)).await.unwrap().is_none(), "the family is revoked");

    // the budget of late answers is finite
    let a = issue().await;
    let b = jwt.refresh(Some(&a.refresh_token)).await.unwrap().unwrap();
    for _ in 0..8 {
        assert!(jwt.refresh(Some(&a.refresh_token)).await.unwrap().is_some());
    }
    assert!(jwt.refresh(Some(&a.refresh_token)).await.unwrap().is_none());
    assert!(jwt.user(Some(&b.access_token)).await.unwrap().is_none());

    // concurrent refreshes: one rotation, every caller answered, the session survives
    let a = issue().await;
    let answers = futures_util::future::join_all((0..4).map(|_| jwt.refresh(Some(&a.refresh_token)))).await;
    let answers: Vec<_> = answers.into_iter().map(|r| r.unwrap().expect("answered")).collect();
    assert_eq!(answers.iter().filter(|t| !t.refresh_token.is_empty()).count(), 1);
    for t in &answers {
        assert!(jwt.user(Some(&t.access_token)).await.unwrap().is_some());
    }

    // a rotated signing key: tokens of the previous key keep working, and a key that is gone does not
    let a = issue().await;
    let rotated = Jwt::new(pool.clone(), Config::new("synthetic-new-signing-secret-32-bytes!".into(), "i".into(), "a".into()).unwrap().with_previous(vec![key.into()]).unwrap());
    assert!(rotated.user(Some(&a.access_token)).await.unwrap().is_some());
    let gone = Jwt::new(pool.clone(), Config::new("synthetic-new-signing-secret-32-bytes!".into(), "i".into(), "a".into()).unwrap());
    assert!(gone.user(Some(&a.access_token)).await.unwrap().is_none());
    assert!(Config::new(key.into(), "i".into(), "a".into()).unwrap().with_previous(vec!["short".into()]).is_err());
}

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn login_lockout_counts_by_email_and_client_with_a_cap_per_email() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use the disposable Postgres harness");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let email = format!("lock-{}@example.test", Uuid::new_v4().simple());
    let good = || Credentials { email: email.clone(), password: "synthetic-jwt-account-password".into() };
    let bad = || Credentials { email: email.clone(), password: "synthetic-wrong-password".into() };
    auth.signup(good()).await.unwrap();
    let (attacker, owner): (std::net::IpAddr, std::net::IpAddr) = ("192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap());
    for _ in 0..11 {
        assert!(auth.login_from(bad(), Some(attacker)).await.unwrap().is_none());
    }
    assert!(auth.login_from(good(), Some(attacker)).await.unwrap().is_none(), "the attacker's client is locked");
    assert!(auth.login_from(good(), Some(owner)).await.unwrap().is_some(), "the owner still signs in from elsewhere");
    // spread over many clients, the cap per email still holds
    sqlx::query("UPDATE _brain_login_limits SET attempts=100 WHERE key=$1").bind(brain_backend::auth::digest(&email)).execute(&pool).await.unwrap();
    assert!(auth.login_from(good(), Some(owner)).await.unwrap().is_none());
}
