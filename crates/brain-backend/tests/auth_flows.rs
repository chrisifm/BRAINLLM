//! Account recovery acceptance against an explicitly owned disposable Postgres database. No real email.
use brain_backend::{
    auth::{Auth, Credentials},
    auth_flows::{self, Delivery, Flows, MailConfig, Message, RolesConfig},
    db, jwt,
};
use sqlx::{ConnectOptions, Row};
use std::{
    future::Future,
    pin::Pin,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

#[derive(Default)]
struct Inbox {
    messages: Mutex<Vec<Message>>,
    slow: bool,
}
impl Delivery for Inbox {
    fn deliver(
        &self,
        message: Message,
    ) -> Pin<Box<dyn Future<Output = Result<(), ()>> + Send + '_>> {
        Box::pin(async move {
            self.messages.lock().unwrap().push(message);
            if self.slow {
                tokio::time::sleep(Duration::from_secs(5)).await
            }
            Ok(())
        })
    }
}
impl Inbox {
    fn token(&self) -> String {
        self.messages
            .lock()
            .unwrap()
            .last()
            .expect("Synthetic delivery must be attempted")
            .link
            .split_once("#token=")
            .unwrap()
            .1
            .to_owned()
    }
}

#[test]
fn role_and_mail_configuration_reject_unsafe_or_privileged_signup_and_do_not_accept_client_roles() {
    let mut roles = RolesConfig::default();
    assert!(roles.validate().is_ok());
    roles.signup_role = "admin".into();
    assert!(roles.validate().is_err());
    roles.signup_role = "user".into();
    roles.roles.push("editor".into());
    assert!(roles.validate().is_ok());
    for invalid in ["owner", "public", "Editor", "bad-role", "bad\nrole", ""] {
        let mut bad = roles.clone();
        bad.roles.push(invalid.into());
        assert!(bad.validate().is_err());
    }
    assert!(serde_json::from_value::<Credentials>(serde_json::json!({"email":"test@example.test","password":"synthetic-password","role":"admin"})).is_err());
    for origin in [
        "http://public.example",
        "https://example.test/app/../escape",
        "https://example.test/app/%2e%2e/escape",
        "https://example.test/app/%2fescape",
        "https://example.test/app/%252e%252e/escape",
        "https://example.test/app\\escape",
        "https://example.test/app//escape",
        "https:///example.test",
        "https://user:pass@example.test",
        "https://example.test/#token=x",
        "https://example.test/?x=1",
    ] {
        assert!(
            MailConfig::resend(
                "synthetic-private-resend-key".into(),
                "sender@example.test".into(),
                origin.into()
            )
            .is_err()
        );
    }
    assert!(
        MailConfig::resend(
            "synthetic-private-resend-key".into(),
            "sender@example.test\r\nBcc:other@example.test".into(),
            "https://example.test".into()
        )
        .is_err()
    );
    assert!(
        MailConfig::resend(
            "synthetic-private-resend-key".into(),
            "sender@example.test".into(),
            "https://example.test".into()
        )
        .is_ok()
    );
    let mail = MailConfig::resend(
        "synthetic-private-resend-key".into(),
        "sender@example.test".into(),
        "https://example.test/project/app/".into(),
    )
    .unwrap();
    assert_eq!(mail.public_base_url, "https://example.test/project/app");
}

async fn database() -> sqlx::PgPool {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL")
        .expect("Use the owned disposable Postgres harness; no real database fallback");
    let base = db::connect(&url).await.unwrap();
    // Fixed ASCII prefix plus UUID hex only; never a caller-controlled SQL identifier.
    let name = format!("brain_auth_flows_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&base)
        .await
        .unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url)
        .unwrap()
        .database(&name);
    let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
    base.close().await;
    db::migrate(&pool, &[]).await.unwrap();
    db::verify(&pool, &[]).await.unwrap();
    pool
}
fn credentials(email: &str, password: &str) -> Credentials {
    Credentials {
        email: email.into(),
        password: password.into(),
    }
}
fn email() -> String {
    format!("auth-flow-{}@example.test", Uuid::new_v4().simple())
}

#[tokio::test]
#[ignore = "Run with the explicitly owned disposable Postgres harness; no external mail"]
async fn account_flows_are_single_use_bound_revocable_non_enumerating_and_server_authorized() {
    let pool = database().await;
    let mut roles = RolesConfig::default();
    roles.signup_role = "user".into();
    roles.roles.push("editor".into());
    roles.register(&pool).await.unwrap();
    let auth = Auth::new_with_role(pool.clone(), "user").await.unwrap();
    let address = email();
    let password = "synthetic-original-password";
    let user = auth.signup(credentials(&address, password)).await.unwrap();
    assert_eq!(user.role, "user");
    let other = auth.signup(credentials(&email(), password)).await.unwrap();
    let inbox = Arc::new(Inbox::default());
    let flows = Flows::new(
        pool.clone(),
        auth.clone(),
        roles.clone(),
        "https://app.example.test/project/app",
        Some(inbox.clone()),
    )
    .unwrap();
    let disabled = Flows::new(
        pool.clone(),
        auth.clone(),
        roles.clone(),
        "https://app.example.test",
        None,
    )
    .unwrap();
    assert_eq!(
        disabled.request_reset(&address).await,
        Err(auth_flows::Error::Unavailable)
    );
    assert_eq!(
        disabled.request_reset(&email()).await,
        Err(auth_flows::Error::Unavailable)
    );

    let unknown = email();
    let started = tokio::time::Instant::now();
    let known = async {
        let start = tokio::time::Instant::now();
        flows.request_reset(&address).await.unwrap();
        start.elapsed()
    };
    let missing = async {
        let start = tokio::time::Instant::now();
        flows.request_reset(&unknown).await.unwrap();
        start.elapsed()
    };
    let (known, missing) = tokio::join!(known, missing);
    assert!(known >= Duration::from_millis(2050) && missing >= Duration::from_millis(2050));
    assert!(known.abs_diff(missing) < Duration::from_millis(500));
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(inbox.messages.lock().unwrap().len(), 1);
    assert!(
        inbox.messages.lock().unwrap()[0]
            .link
            .starts_with("https://app.example.test/project/app/reset-password#token=")
    );
    let token = inbox.token();
    let stored=sqlx::query("SELECT token_hash,email_bound,activated_at IS NOT NULL AS active FROM _brain_auth_tokens WHERE user_id=$1 AND purpose='reset'").bind(user.id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        stored.get::<String, _>("token_hash"),
        brain_backend::auth::digest(&token)
    );
    assert_ne!(stored.get::<String, _>("token_hash"), token);
    assert_eq!(stored.get::<String, _>("email_bound"), address);
    assert!(stored.get::<bool, _>("active"));
    let (signed, opaque) = auth
        .login(credentials(&address, password))
        .await
        .unwrap()
        .unwrap();
    let sessions = jwt::Jwt::new(
        pool.clone(),
        jwt::Config::new(
            "synthetic-jwt-signing-key-over-32-bytes".into(),
            "test-issuer".into(),
            "test-audience".into(),
        )
        .unwrap(),
    );
    let jwt = sessions.issue(signed, &opaque).await.unwrap();
    let replacement = "synthetic-replacement-password";
    assert!(!flows.verify_email(user.id, &token).await.unwrap());
    let (first, competing) = tokio::join!(
        flows.complete_reset(&token, replacement.into()),
        flows.complete_reset(&token, replacement.into())
    );
    assert_eq!(
        [first.unwrap(), competing.unwrap()]
            .into_iter()
            .filter(|v| *v)
            .count(),
        1,
        "Concurrent consumers must apply one reset, not two"
    );
    assert!(
        !flows
            .complete_reset(&token, replacement.into())
            .await
            .unwrap()
    );
    assert!(auth.user(Some(&opaque)).await.unwrap().is_none());
    assert!(
        sessions
            .user(Some(&jwt.access_token))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sessions
            .refresh(Some(&jwt.refresh_token))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        auth.login(credentials(&address, password))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        auth.login(credentials(&address, replacement))
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT auth_version FROM _brain_users WHERE id=$1")
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );

    flows.request_verification(user.id).await.unwrap();
    let verification = inbox.token();
    assert!(!flows.verify_email(other.id, &verification).await.unwrap());
    assert!(flows.verify_email(user.id, &verification).await.unwrap());
    assert!(!flows.verify_email(user.id, &verification).await.unwrap());
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT verified FROM _brain_users WHERE id=$1")
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    flows.request_reset(&address).await.unwrap();
    let bound = inbox.token();
    sqlx::query("UPDATE _brain_users SET email=$2 WHERE id=$1")
        .bind(user.id)
        .bind(email())
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !flows
            .complete_reset(&bound, replacement.into())
            .await
            .unwrap()
    );
    sqlx::query("UPDATE _brain_users SET email=$2 WHERE id=$1")
        .bind(user.id)
        .bind(&address)
        .execute(&pool)
        .await
        .unwrap();
    flows.request_reset(&address).await.unwrap();
    let expired = inbox.token();
    sqlx::query(
        "UPDATE _brain_auth_tokens SET expires_at=now()-interval '1 second' WHERE token_hash=$1",
    )
    .bind(brain_backend::auth::digest(&expired))
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        !flows
            .complete_reset(&expired, replacement.into())
            .await
            .unwrap()
    );

    assert!(!flows.assign_role(other.id, user.id, "admin").await.unwrap());
    assert!(
        auth_flows::bootstrap_first_admin(&pool, other.id)
            .await
            .unwrap()
    );
    assert!(
        !auth_flows::bootstrap_first_admin(&pool, user.id)
            .await
            .unwrap()
    );
    assert!(
        !flows.assign_role(other.id, other.id, "user").await.unwrap(),
        "The final enabled admin cannot remove its own recovery path"
    );
    let (_, role_session) = auth
        .login(credentials(&address, replacement))
        .await
        .unwrap()
        .unwrap();
    assert!(
        flows
            .assign_role(other.id, user.id, "editor")
            .await
            .unwrap()
    );
    assert!(auth.user(Some(&role_session)).await.unwrap().is_none());
    assert_eq!(
        auth.login(credentials(&address, replacement))
            .await
            .unwrap()
            .unwrap()
            .0
            .role,
        "editor"
    );
    assert_eq!(
        flows.assign_role(other.id, user.id, "unconfigured").await,
        Err(auth_flows::Error::InvalidInput)
    );
    let audit: Vec<String> =
        sqlx::query_scalar("SELECT event FROM _brain_auth_events WHERE user_id=$1")
            .bind(user.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    for event in ["password-reset", "email-verified", "role-changed"] {
        assert!(audit.iter().any(|s| s == event))
    }

    let slow = Arc::new(Inbox {
        slow: true,
        ..Inbox::default()
    });
    let slow_user = auth.signup(credentials(&email(), password)).await.unwrap();
    let uncertain = Flows::new(
        pool.clone(),
        auth.clone(),
        roles,
        "https://app.example.test",
        Some(slow.clone()),
    )
    .unwrap();
    let start = tokio::time::Instant::now();
    uncertain.request_reset(&slow_user.email).await.unwrap();
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(
        !uncertain
            .complete_reset(&slow.token(), replacement.into())
            .await
            .unwrap(),
        "An unconfirmed delivery must never activate its hash"
    );
    // Three sends per 15-minute digest window; known and unknown requests have the same acceptance result.
    let count = inbox.messages.lock().unwrap().len();
    flows.request_reset(&address).await.unwrap();
    assert_eq!(inbox.messages.lock().unwrap().len(), count);
    pool.close().await;
}
