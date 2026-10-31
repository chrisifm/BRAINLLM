//! Argon2id credentials and revocable opaque sessions. Roles are loaded from Postgres on every request.
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

pub const COOKIE: &str = "__Host-brain";
const LOGIN_ATTEMPTS: i32 = 10;
const LOGIN_ATTEMPTS_ACCOUNT: i32 = 100;
const DEFAULT_SESSION_SECONDS: u32 = 604800;
static SESSION_SECONDS: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
/// `BRAIN_AUTH_SESSION_SECONDS`: the length of new sessions and refresh families, set once at startup.
pub fn configure_session_seconds(seconds: u32) -> Result<(), String> {
    if !(300..=31_536_000).contains(&seconds) {
        return Err("BRAIN_AUTH_SESSION_SECONDS must be 300..31536000".into());
    }
    SESSION_SECONDS.set(seconds).map_err(|_| "Session length is already configured".to_string())
}
pub fn session_seconds() -> u32 {
    SESSION_SECONDS.get().copied().unwrap_or(DEFAULT_SESSION_SECONDS)
}
#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub role: String,
    pub verified: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}
#[derive(Clone)]
pub struct Auth {
    pool: PgPool,
    slots: Arc<Semaphore>,
    dummy: String,
    signup_role: Arc<str>,
}
pub fn digest(s: &str) -> String {
    let digest = Sha256::digest(s.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
fn token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
pub fn cookie(token: &str, age: u32) -> String {
    format!("{COOKIE}={token}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={age}")
}
pub fn session_cookie(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|s| s.trim().strip_prefix(&format!("{COOKIE}=")))
}
impl Auth {
    pub async fn new(pool: PgPool) -> Result<Self, String> {
        Self::new_with_role(pool, "candidate").await
    }
    pub async fn new_with_role(pool: PgPool, signup_role: &str) -> Result<Self, String> {
        if !crate::auth_flows::role_name(signup_role) || signup_role == "admin" {
            return Err("Signup role must be a server-configured non-admin role".into());
        }
        let dummy = tokio::task::spawn_blocking(|| {
            Argon2::default()
                .hash_password(token().as_bytes())
                .map(|h| h.to_string())
        })
        .await
        .map_err(|_| "Password worker failed")?
        .map_err(|_| "Password hashing unavailable")?;
        Ok(Self {
            pool,
            slots: Arc::new(Semaphore::new(2)),
            dummy,
            signup_role: signup_role.into(),
        })
    }
    pub fn signup_role(&self) -> &str {
        &self.signup_role
    }
    pub fn email(email: &str) -> Result<String, &'static str> {
        let email = email.trim().to_ascii_lowercase();
        if email.len() > 254
            || email.len() < 3
            || email.chars().any(char::is_whitespace)
            || !email
                .split_once('@')
                .is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.contains('@'))
        {
            return Err("Enter a valid email address");
        }
        Ok(email)
    }
    pub(crate) async fn hash(&self, password: String) -> Result<String, String> {
        if !(12..=1024).contains(&password.len()) {
            return Err("Password must be 12..1024 bytes".into());
        }
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "Password workers busy; retry shortly")?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            Argon2::default()
                .hash_password(password.as_bytes())
                .map(|h| h.to_string())
                .map_err(|_| "Password hashing failed".into())
        })
        .await
        .map_err(|_| "Password worker failed")?
    }
    pub async fn signup(&self, credentials: Credentials) -> Result<User, String> {
        let email = Self::email(&credentials.email)?;
        let hash = self.hash(credentials.password).await?;
        let id = Uuid::new_v4();
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role) VALUES ($1,$2,$3,$4)")
            .bind(id)
            .bind(&email)
            .bind(hash)
            .bind(self.signup_role())
            .execute(&mut *tx)
            .await
            .map_err(|_| "Cannot create account with these details")?;
        sqlx::query("INSERT INTO _brain_auth_events(user_id,event) VALUES ($1,'signup')")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(crate::db::safe)?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(User {
            id,
            email,
            role: self.signup_role().into(),
            verified: false,
        })
    }
    async fn login_attempt(&self, key: &str, cap: i32) -> Result<i32, String> {
        sqlx::query_scalar("INSERT INTO _brain_login_limits(key,attempts,expires_at) VALUES ($1,1,now()+interval '15 minutes') ON CONFLICT(key) DO UPDATE SET attempts=CASE WHEN _brain_login_limits.expires_at<=now() THEN 1 WHEN _brain_login_limits.attempts>=$2 THEN $2+1 ELSE _brain_login_limits.attempts+1 END, expires_at=CASE WHEN _brain_login_limits.expires_at<=now() THEN now()+interval '15 minutes' ELSE _brain_login_limits.expires_at END RETURNING attempts").bind(key).bind(cap).fetch_one(&self.pool).await.map_err(crate::db::safe)
    }
    pub async fn login(&self, credentials: Credentials) -> Result<Option<(User, String)>, String> {
        self.login_from(credentials, None).await
    }
    /// One failure counter per email and client, so a third party cannot lock the owner out from elsewhere, plus a
    /// higher cap per email for attacks spread over many addresses. Unknown accounts count the same.
    pub async fn login_from(&self, credentials: Credentials, client: Option<std::net::IpAddr>) -> Result<Option<(User, String)>, String> {
        let email = Self::email(&credentials.email)?;
        if credentials.password.len() > 1024 {
            return Ok(None);
        }
        let account = digest(&email);
        let key = client.map_or_else(|| account.clone(), |ip| digest(&format!("{email}|{ip}")));
        // Cleanup is bounded and never logs email/key.
        sqlx::query("DELETE FROM _brain_login_limits WHERE key IN (SELECT key FROM _brain_login_limits WHERE expires_at<now() LIMIT 100)").execute(&self.pool).await.map_err(crate::db::safe)?;
        if self.login_attempt(&key, LOGIN_ATTEMPTS).await? > LOGIN_ATTEMPTS {
            return Ok(None);
        }
        if client.is_some() && self.login_attempt(&account, LOGIN_ATTEMPTS_ACCOUNT).await? > LOGIN_ATTEMPTS_ACCOUNT {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT id,email,role,verified,password_hash,auth_version FROM _brain_users WHERE email=$1 AND disabled_at IS NULL",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(crate::db::safe)?;
        let hash: String = row
            .as_ref()
            .map(|r| r.get("password_hash"))
            .unwrap_or_else(|| self.dummy.clone());
        let password = credentials.password;
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "Password workers busy; retry shortly")?;
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            PasswordHash::new(&hash).is_ok_and(|h| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &h)
                    .is_ok()
            })
        })
        .await
        .map_err(|_| "Password worker failed")?;
        if !valid {
            return Ok(None);
        }
        let Some(row) = row else { return Ok(None) };
        let user = User {
            id: row.get("id"),
            email: row.get("email"),
            role: row.get("role"),
            verified: row.get("verified"),
        };
        let token = token();
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        sqlx::query("DELETE FROM _brain_login_limits WHERE key=$1")
            .bind(&key)
            .execute(&mut *tx)
            .await
            .map_err(crate::db::safe)?;
        reserve_session(&mut tx, user.id).await?;
        let live:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _brain_users WHERE id=$1 AND auth_version=$2 AND disabled_at IS NULL)").bind(user.id).bind(row.get::<i64,_>("auth_version")).fetch_one(&mut *tx).await.map_err(crate::db::safe)?;
        if !live {
            return Ok(None);
        }
        sqlx::query("INSERT INTO _brain_sessions(token_hash,user_id,expires_at) VALUES ($1,$2,now()+make_interval(secs=>$3))").bind(digest(&token)).bind(user.id).bind(f64::from(session_seconds())).execute(&mut *tx).await.map_err(crate::db::safe)?;
        sqlx::query("INSERT INTO _brain_auth_events(user_id,event) VALUES ($1,'login')")
            .bind(user.id)
            .execute(&mut *tx)
            .await
            .map_err(crate::db::safe)?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(Some((user, token)))
    }
    /// App-origin POST creates a browser-bound one-use GIS nonce; the plaintext is never stored.
    pub async fn google_challenge(&self) -> Result<String, String> {
        let nonce = crate::jwt::opaque();
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('brain:google:challenge'))")
            .execute(&mut *tx)
            .await
            .map_err(crate::db::safe)?;
        sqlx::query("DELETE FROM _brain_auth_challenges WHERE token_hash IN (SELECT token_hash FROM _brain_auth_challenges WHERE purpose='google_nonce' AND expires_at<=now() ORDER BY expires_at LIMIT 100)").execute(&mut *tx).await.map_err(crate::db::safe)?;
        let retained:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM (SELECT 1 FROM _brain_auth_challenges WHERE purpose='google_nonce' LIMIT 10000) retained").fetch_one(&mut *tx).await.map_err(crate::db::safe)?;
        if retained >= 10000 {
            return Err("Identity challenge budget reached; retry shortly".into());
        }
        sqlx::query("INSERT INTO _brain_auth_challenges(token_hash,purpose,provider,expires_at) VALUES ($1,'google_nonce','google',now()+interval '5 minutes')").bind(digest(&nonce)).execute(&mut *tx).await.map_err(crate::db::safe)?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(nonce)
    }
    /// Identity fields come only from Google::verify. An existing email account links only as `link_google` allows.
    pub async fn google_login(
        &self,
        identity: &crate::google::Identity,
        nonce: &str,
    ) -> Result<Option<(User, String)>, String> {
        if !crate::jwt::valid_opaque(nonce) {
            return Ok(None);
        }
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        let consumed=sqlx::query("UPDATE _brain_auth_challenges SET consumed_at=now() WHERE token_hash=$1 AND purpose='google_nonce' AND provider='google' AND expires_at>now() AND consumed_at IS NULL").bind(digest(nonce)).execute(&mut *tx).await.map_err(crate::db::safe)?.rows_affected();
        if consumed != 1 {
            return Ok(None);
        }
        let existing=sqlx::query("SELECT u.id,u.email,u.role,u.verified,u.disabled_at IS NOT NULL AS disabled FROM _brain_users u JOIN _brain_identities i ON i.user_id=u.id WHERE i.provider='google' AND i.subject=$1").bind(&identity.subject).fetch_optional(&mut *tx).await.map_err(crate::db::safe)?;
        let user = if let Some(row) = existing {
            if row.get::<bool, _>("disabled") {
                tx.commit().await.map_err(crate::db::safe)?;
                return Ok(None);
            }
            User {
                id: row.get("id"),
                email: row.get("email"),
                role: row.get("role"),
                verified: row.get("verified"),
            }
        } else {
            let holder = sqlx::query("SELECT id,email,role,verified,disabled_at IS NOT NULL AS disabled FROM _brain_users WHERE email=$1 FOR UPDATE")
                .bind(&identity.email)
                .fetch_optional(&mut *tx)
                .await
                .map_err(crate::db::safe)?;
            if let Some(row) = holder {
                let Some(user) = self.link_google(&mut tx, identity, &row).await? else {
                    tx.commit().await.map_err(crate::db::safe)?;
                    return Ok(None);
                };
                return self.finish_google(tx, user).await;
            }
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role,verified) VALUES ($1,$2,$3,$4,$5)").bind(id).bind(&identity.email).bind(&self.dummy).bind(self.signup_role()).bind(identity.authoritative_email).execute(&mut *tx).await.map_err(|_|"Cannot create identity account")?;
            sqlx::query("INSERT INTO _brain_identities(provider,subject,user_id,issuer,verified_email) VALUES ('google',$1,$2,$3,$4)").bind(&identity.subject).bind(id).bind(&identity.issuer).bind(&identity.email).execute(&mut *tx).await.map_err(crate::db::safe)?;
            User {
                id,
                email: identity.email.clone(),
                role: self.signup_role().into(),
                verified: identity.authoritative_email,
            }
        };
        self.finish_google(tx, user).await
    }
    async fn finish_google(
        &self,
        mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
        user: User,
    ) -> Result<Option<(User, String)>, String> {
        reserve_session(&mut tx, user.id).await?;
        let token = token();
        sqlx::query("INSERT INTO _brain_sessions(token_hash,user_id,expires_at) VALUES ($1,$2,now()+make_interval(secs=>$3))").bind(digest(&token)).bind(user.id).bind(f64::from(session_seconds())).execute(&mut *tx).await.map_err(crate::db::safe)?;
        sqlx::query("INSERT INTO _brain_auth_events(user_id,event,outcome) VALUES ($1,'google-login','success')").bind(user.id).execute(&mut *tx).await.map_err(crate::db::safe)?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(Some((user, token)))
    }
    /// The email already has an account and this Google subject has none. Link only when Google is authoritative for
    /// the address. An account that never proved it holds nothing to preserve: its password, sessions, recovery links
    /// and API keys may belong to whoever typed the address first, so the proven owner annuls them all.
    /// A linked identity is never replaced, and a disabled account stays disabled.
    async fn link_google(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        identity: &crate::google::Identity,
        row: &sqlx::postgres::PgRow,
    ) -> Result<Option<User>, String> {
        let id: Uuid = row.get("id");
        let linked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _brain_identities WHERE user_id=$1 AND provider='google')")
            .bind(id)
            .fetch_one(&mut **tx)
            .await
            .map_err(crate::db::safe)?;
        if row.get::<bool, _>("disabled") || linked || !identity.authoritative_email {
            sqlx::query("INSERT INTO _brain_auth_events(user_id,event,outcome) VALUES ($1,'google-link','denied')").bind(id).execute(&mut **tx).await.map_err(crate::db::safe)?;
            return Ok(None);
        }
        if !row.get::<bool, _>("verified") {
            sqlx::query("UPDATE _brain_users SET password_hash=$2,verified=true,auth_version=auth_version+1,updated_at=now() WHERE id=$1")
                .bind(id)
                .bind(&self.dummy)
                .execute(&mut **tx)
                .await
                .map_err(crate::db::safe)?;
            crate::auth_flows::revoke(tx, id).await.map_err(|error| error.to_string())?;
            sqlx::query("UPDATE public._brain_api_keys SET revoked_at=now(),updated_at=now() WHERE user_id=$1 AND revoked_at IS NULL")
                .bind(id)
                .execute(&mut **tx)
                .await
                .map_err(crate::db::safe)?;
            sqlx::query("INSERT INTO _brain_auth_events(user_id,event,outcome) VALUES ($1,'unproven-credentials-annulled','success')").bind(id).execute(&mut **tx).await.map_err(crate::db::safe)?;
        }
        sqlx::query("INSERT INTO _brain_identities(provider,subject,user_id,issuer,verified_email) VALUES ('google',$1,$2,$3,$4)")
            .bind(&identity.subject)
            .bind(id)
            .bind(&identity.issuer)
            .bind(&identity.email)
            .execute(&mut **tx)
            .await
            .map_err(crate::db::safe)?;
        sqlx::query("INSERT INTO _brain_auth_events(user_id,event,outcome) VALUES ($1,'google-link','success')").bind(id).execute(&mut **tx).await.map_err(crate::db::safe)?;
        Ok(Some(User {
            id,
            email: row.get("email"),
            role: row.get("role"),
            verified: true,
        }))
    }
    pub async fn user(&self, token: Option<&str>) -> Result<Option<User>, String> {
        let Some(token) =
            token.filter(|t| t.len() == 64 && t.bytes().all(|c| c.is_ascii_hexdigit()))
        else {
            return Ok(None);
        };
        sqlx::query("SELECT u.id,u.email,u.role,u.verified FROM _brain_users u JOIN _brain_sessions s ON s.user_id=u.id WHERE s.token_hash=$1 AND s.expires_at>now() AND s.revoked_at IS NULL AND u.disabled_at IS NULL")
            .bind(digest(token)).fetch_optional(&self.pool).await.map(|r|r.map(|r|User {id:r.get("id"),email:r.get("email"),role:r.get("role"),verified:r.get("verified")})).map_err(crate::db::safe)
    }
    pub async fn logout(&self, token: Option<&str>) -> Result<(), String> {
        if let Some(token) = token {
            let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
            sqlx::query("INSERT INTO _brain_auth_events(user_id,event) SELECT user_id,'logout' FROM _brain_sessions WHERE token_hash=$1").bind(digest(token)).execute(&mut *tx).await.map_err(crate::db::safe)?;
            sqlx::query("DELETE FROM _brain_sessions WHERE token_hash=$1")
                .bind(digest(token))
                .execute(&mut *tx)
                .await
                .map_err(crate::db::safe)?;
            tx.commit().await.map_err(crate::db::safe)?;
        }
        Ok(())
    }
}

/// Call inside the SAME transaction as every opaque/JWT session insert.
/// Serialize by user, retain nine existing live sessions, then let the caller insert the tenth.
pub async fn reserve_session(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: Uuid,
) -> Result<(), String> {
    sqlx::query("SELECT id FROM _brain_users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut **tx)
        .await
        .map_err(crate::db::safe)?;
    sqlx::query("DELETE FROM _brain_sessions WHERE user_id=$1 AND (expires_at<=now() OR revoked_at IS NOT NULL OR token_hash IN (SELECT token_hash FROM _brain_sessions WHERE user_id=$1 ORDER BY expires_at DESC,token_hash OFFSET 9))").bind(user).execute(&mut **tx).await.map_err(crate::db::safe)?;
    Ok(())
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Use the disposable backend Postgres harness"]
    async fn google_nonces_are_single_use_and_existing_emails_never_link_implicitly() {
        let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only");
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool, &[]).await.unwrap();
        let auth = Auth::new(pool.clone()).await.unwrap();
        let email = format!("identity-{}@example.test", Uuid::new_v4().simple());
        let existing = auth
            .signup(Credentials {
                email: email.clone(),
                password: "synthetic-existing-account-password".into(),
            })
            .await
            .unwrap();
        let mut identity = crate::google::Identity {
            subject: Uuid::new_v4().simple().to_string(),
            issuer: "https://accounts.google.com".into(),
            email,
            authoritative_email: false,
        };
        let nonce = auth.google_challenge().await.unwrap();
        assert!(
            auth.google_login(&identity, &nonce)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            auth.google_login(&identity, &nonce)
                .await
                .unwrap()
                .is_none()
        );
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM _brain_identities WHERE user_id=$1")
                .bind(existing.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0);
        identity.email = format!("new-{}@example.test", Uuid::new_v4().simple());
        let nonce = auth.google_challenge().await.unwrap();
        let (user, token) = auth.google_login(&identity, &nonce).await.unwrap().unwrap();
        assert_eq!(user.role, "candidate");
        assert!(!user.verified);
        assert!(auth.user(Some(&token)).await.unwrap().is_some());
        assert!(
            auth.google_login(&identity, &nonce)
                .await
                .unwrap()
                .is_none()
        );
        let nonce = auth.google_challenge().await.unwrap();
        let returning = auth
            .google_login(&identity, &nonce)
            .await
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(returning.id, user.id);
        sqlx::query("UPDATE _brain_users SET disabled_at=now() WHERE id=$1")
            .bind(user.id)
            .execute(&pool)
            .await
            .unwrap();
        let nonce = auth.google_challenge().await.unwrap();
        assert!(
            auth.google_login(&identity, &nonce)
                .await
                .unwrap()
                .is_none()
        );
        sqlx::query("DELETE FROM _brain_users WHERE id=ANY($1)")
            .bind(vec![existing.id, user.id])
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }
    fn gmail(subject: &str, email: &str) -> crate::google::Identity {
        crate::google::Identity {
            subject: subject.into(),
            issuer: "https://accounts.google.com".into(),
            email: email.into(),
            authoritative_email: true,
        }
    }
    fn account(email: &str) -> Credentials {
        Credentials {
            email: email.into(),
            password: "synthetic-existing-account-password".into(),
        }
    }
    #[tokio::test]
    #[ignore = "Use the disposable backend Postgres harness"]
    async fn google_links_when_both_sides_are_verified_and_keeps_the_proven_credentials() {
        let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only");
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool, &[]).await.unwrap();
        let auth = Auth::new(pool.clone()).await.unwrap();
        let email = format!("link-{}@example.test", Uuid::new_v4().simple());
        let user = auth.signup(account(&email)).await.unwrap();
        sqlx::query("UPDATE _brain_users SET verified=true WHERE id=$1").bind(user.id).execute(&pool).await.unwrap();
        let (_, before) = auth.login(account(&email)).await.unwrap().unwrap();
        let subject = Uuid::new_v4().simple().to_string();
        // Not authoritative: Google only observed this address once, so it proves nothing about it today.
        let mut weak = gmail(&subject, &email);
        weak.authoritative_email = false;
        let nonce = auth.google_challenge().await.unwrap();
        assert!(auth.google_login(&weak, &nonce).await.unwrap().is_none());
        let nonce = auth.google_challenge().await.unwrap();
        let (linked, _) = auth.google_login(&gmail(&subject, &email), &nonce).await.unwrap().unwrap();
        assert_eq!(linked.id, user.id);
        assert!(linked.verified);
        assert!(auth.user(Some(&before)).await.unwrap().is_some(), "a proven session survives linking");
        assert!(auth.login(account(&email)).await.unwrap().is_some(), "a proven password survives linking");
        // A second Google account claiming the same address never replaces the linked one.
        let intruder = Uuid::new_v4().simple().to_string();
        let nonce = auth.google_challenge().await.unwrap();
        assert!(auth.google_login(&gmail(&intruder, &email), &nonce).await.unwrap().is_none());
        let subjects: Vec<String> = sqlx::query_scalar("SELECT subject FROM _brain_identities WHERE user_id=$1").bind(user.id).fetch_all(&pool).await.unwrap();
        assert_eq!(subjects, vec![subject.clone()]);
        let nonce = auth.google_challenge().await.unwrap();
        assert_eq!(auth.google_login(&gmail(&subject, &email), &nonce).await.unwrap().unwrap().0.id, user.id);
        pool.close().await;
    }
    #[tokio::test]
    #[ignore = "Use the disposable backend Postgres harness"]
    async fn google_owner_annuls_every_credential_of_an_unverified_account_holding_the_email() {
        let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only");
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool, &[]).await.unwrap();
        let auth = Auth::new(pool.clone()).await.unwrap();
        let jwt = crate::jwt::Jwt::new(
            pool.clone(),
            crate::jwt::Config::new("synthetic-jwt-signing-secret-32-bytes".into(), "issuer".into(), "audience".into()).unwrap(),
        );
        let email = format!("squat-{}@example.test", Uuid::new_v4().simple());
        let squatter = auth.signup(account(&email)).await.unwrap();
        let (session_user, session) = auth.login(account(&email)).await.unwrap().unwrap();
        let (token_user, opaque) = auth.login(account(&email)).await.unwrap().unwrap();
        assert_eq!(session_user.id, token_user.id);
        let tokens = jwt.issue(token_user, &opaque).await.unwrap();
        // Every door the unproven holder may have prepared for later.
        sqlx::query("INSERT INTO public._brain_auth_tokens(token_hash,user_id,purpose,expires_at,email_bound,auth_version,activated_at) VALUES($1,$2,'reset',now()+interval '1 hour',$3,0,now())")
            .bind(digest(&email)).bind(squatter.id).bind(&email).execute(&pool).await.unwrap();
        let key = Uuid::new_v4();
        sqlx::query("INSERT INTO public._brain_api_keys(id,user_id,name,prefix,secret_hash,scopes) VALUES($1,$2,'prepared',$3,$4,ARRAY['*'])")
            .bind(key).bind(squatter.id).bind(&key.simple().to_string()[..12]).bind(digest(&key.to_string())).execute(&pool).await.unwrap();
        let subject = Uuid::new_v4().simple().to_string();
        let nonce = auth.google_challenge().await.unwrap();
        let (owner, owner_session) = auth.google_login(&gmail(&subject, &email), &nonce).await.unwrap().unwrap();
        assert_eq!(owner.id, squatter.id);
        assert!(owner.verified, "the real owner proved the address");
        assert!(auth.login(account(&email)).await.unwrap().is_none(), "the unproven password no longer signs in");
        assert!(auth.user(Some(&session)).await.unwrap().is_none(), "unproven opaque sessions end");
        assert!(jwt.user(Some(&tokens.access_token)).await.unwrap().is_none(), "unproven access tokens end");
        assert!(jwt.refresh(Some(&tokens.refresh_token)).await.unwrap().is_none(), "unproven refresh tokens end");
        assert!(auth.user(Some(&owner_session)).await.unwrap().is_some());
        let open: i64 = sqlx::query_scalar("SELECT count(*) FROM public._brain_auth_tokens WHERE user_id=$1 AND consumed_at IS NULL").bind(squatter.id).fetch_one(&pool).await.unwrap();
        assert_eq!(open, 0, "prepared recovery links are consumed");
        let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM public._brain_api_keys WHERE user_id=$1 AND revoked_at IS NULL").bind(squatter.id).fetch_one(&pool).await.unwrap();
        assert_eq!(keys, 0, "prepared API keys are revoked");
        // Disabled stays disabled however the owner signs in.
        let other = format!("disabled-{}@example.test", Uuid::new_v4().simple());
        let disabled = auth.signup(account(&other)).await.unwrap();
        sqlx::query("UPDATE _brain_users SET disabled_at=now() WHERE id=$1").bind(disabled.id).execute(&pool).await.unwrap();
        let nonce = auth.google_challenge().await.unwrap();
        assert!(auth.google_login(&gmail(&Uuid::new_v4().simple().to_string(), &other), &nonce).await.unwrap().is_none());
        let identities: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_identities WHERE user_id=$1").bind(disabled.id).fetch_one(&pool).await.unwrap();
        assert_eq!(identities, 0);
        pool.close().await;
    }
}
