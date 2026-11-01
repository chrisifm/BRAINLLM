//! Its own executable: the idle timeout is one process-wide setting, like the session length.
use brain_backend::{
    auth::{Auth, Credentials},
    jwt::{Config, Jwt},
};
use uuid::Uuid;

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn an_unused_session_stops_working_for_every_credential() {
    assert!(brain_backend::sessions::configure_idle_seconds(10).is_err(), "below the range");
    brain_backend::sessions::configure_idle_seconds(600).unwrap();
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use the disposable Postgres harness; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let email = format!("idle-{}@example.test", Uuid::new_v4().simple());
    let credentials = || Credentials { email: email.clone(), password: "synthetic-idle-account-password".into() };
    let user = auth.signup(credentials()).await.unwrap();
    let jwt = Jwt::new(pool.clone(), Config::new("synthetic-jwt-signing-secret-32-bytes".into(), "test-issuer".into(), "test-audience".into()).unwrap());
    let (signed, opaque) = auth.login(credentials()).await.unwrap().unwrap();
    let tokens = jwt.issue(signed, &opaque).await.unwrap();
    let age = async |interval: &str| {
        sqlx::query("UPDATE _brain_sessions SET last_used_at=now()-$2::interval WHERE user_id=$1").bind(user.id).bind(interval).execute(&pool).await.unwrap();
    };
    let used = async || sqlx::query_scalar::<_, f64>("SELECT EXTRACT(EPOCH FROM now()-last_used_at)::float8 FROM _brain_sessions WHERE user_id=$1").bind(user.id).fetch_one(&pool).await.unwrap();

    // Use within the window keeps it alive, and is written back only once it is a minute old.
    age("5 minutes").await;
    assert!(auth.user(Some(&opaque)).await.unwrap().is_some());
    assert!(used().await < 5.0, "the session's last use moved");
    age("30 seconds").await;
    assert!(jwt.user(Some(&tokens.access_token)).await.unwrap().is_some());
    assert!(used().await >= 30.0, "a recent use costs no write");
    age("5 minutes").await;
    assert!(jwt.user(Some(&tokens.access_token)).await.unwrap().is_some());
    assert!(used().await < 5.0);

    // Past the window nothing brings it back: not the cookie, not the access token, not the refresh token.
    age("11 minutes").await;
    assert!(auth.user(Some(&opaque)).await.unwrap().is_none());
    assert!(jwt.user(Some(&tokens.access_token)).await.unwrap().is_none());
    assert!(jwt.refresh(Some(&tokens.refresh_token)).await.is_err() || jwt.refresh(Some(&tokens.refresh_token)).await.unwrap().is_none());
    assert_eq!(brain_backend::sessions::list(&pool, user.id, Uuid::new_v4()).await.unwrap().len(), 0);
}
