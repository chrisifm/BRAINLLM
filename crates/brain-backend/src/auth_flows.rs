//! Single-use email-bound account challenges. No plaintext tokens in HTTP replies, jobs or logs.
use crate::auth::{Auth, digest};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RolesConfig {
    pub roles: Vec<String>,
    pub signup_role: String,
}
impl Default for RolesConfig {
    fn default() -> Self {
        Self {
            roles: ["admin", "company", "candidate", "user"]
                .map(String::from)
                .to_vec(),
            signup_role: "candidate".into(),
        }
    }
}
pub fn role_name(role: &str) -> bool {
    let mut bytes = role.bytes();
    role.len() <= 48
        && bytes.next().is_some_and(|c| c.is_ascii_lowercase())
        && bytes.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        && !matches!(role, "public" | "owner")
}
impl RolesConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.roles.len() > 32
            || self.roles.iter().any(|r| !role_name(r))
            || self
                .roles
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != self.roles.len()
            || ["admin", "company", "candidate", "user"]
                .iter()
                .any(|r| !self.roles.iter().any(|s| s.as_str() == *r))
            || self.signup_role == "admin"
            || !self.roles.contains(&self.signup_role)
        {
            return Err("Auth roles require 4..32 distinct safe names including admin/company/candidate/user, and a configured non-admin signup role".into());
        }
        Ok(())
    }
    /// Explicit server configuration only; never bind this operation to public signup input.
    pub async fn register(&self, pool: &PgPool) -> Result<(), String> {
        self.validate()?;
        let mut tx = pool.begin().await.map_err(crate::db::safe)?;
        for role in &self.roles {
            sqlx::query(
                "INSERT INTO public._brain_auth_roles(role) VALUES($1) ON CONFLICT DO NOTHING",
            )
            .bind(role)
            .execute(&mut *tx)
            .await
            .map_err(crate::db::safe)?;
        }
        tx.commit().await.map_err(crate::db::safe)
    }
}

#[derive(Clone, Copy)]
pub enum Purpose {
    Reset,
    Verify,
}
impl Purpose {
    fn name(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Verify => "verify",
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Reset => "/reset-password",
            Self::Verify => "/verify-email",
        }
    }
    fn subject(self) -> &'static str {
        match self {
            Self::Reset => "Reset your password",
            Self::Verify => "Verify your email address",
        }
    }
    fn seconds(self) -> i32 {
        match self {
            Self::Reset => 1800,
            Self::Verify => 86400,
        }
    }
}
// Deliberately no Debug/Serialize: a delivery adapter receives private values only in memory.
pub struct Message {
    pub recipient: String,
    pub link: String,
    pub purpose: Purpose,
}
pub trait Delivery: Send + Sync {
    fn deliver(
        &self,
        message: Message,
    ) -> Pin<Box<dyn Future<Output = Result<(), ()>> + Send + '_>>;
}

#[derive(Clone)]
pub struct MailConfig {
    key: Arc<str>,
    from: String,
    pub public_base_url: String,
}
fn email(input: &str) -> Option<String> {
    let input = input.trim();
    if !input.bytes().all(|c| c.is_ascii_graphic())
        || input.bytes().any(|c| b"<>(),;:\\\"[]".contains(&c))
    {
        return None;
    }
    Auth::email(input).ok()
}
fn base_url(input: &str) -> Result<String, String> {
    if input.len() > 512 || !input.bytes().all(|c| c.is_ascii_graphic()) || input.contains('\\') {
        return Err("Auth public base URL exceeds its limit or contains unsafe characters".into());
    }
    let url = url::Url::parse(input).map_err(|_| "Auth public base URL must be explicit HTTPS")?;
    let authority_path = input
        .split_once("://")
        .ok_or("Auth public base URL requires an explicit scheme")?
        .1;
    if authority_path
        .split('/')
        .next()
        .is_none_or(|a| a.is_empty())
    {
        return Err("Auth public base URL requires an explicit authority".into());
    }
    let raw_path = authority_path
        .find('/')
        .map(|i| &authority_path[i..])
        .unwrap_or("");
    if authority_path
        .split('/')
        .next()
        .is_some_and(|a| a.contains('@'))
    {
        return Err("Auth public base URL cannot contain user information".into());
    }
    // Validate the original path before URL parsing can normalize literal/encoded dot segments.
    let raw_path = raw_path.trim_end_matches('/');
    for part in raw_path.strip_prefix('/').unwrap_or(raw_path).split('/') {
        if part.is_empty() {
            if raw_path.is_empty() {
                continue;
            } else {
                return Err("Auth public base path has an empty segment".into());
            }
        }
        let mut decoded = Vec::with_capacity(part.len());
        let bytes = part.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = if bytes[i] == b'%' {
                let hex = part
                    .get(i + 1..i + 3)
                    .ok_or("Auth public base path has an invalid escape")?;
                i += 2;
                u8::from_str_radix(hex, 16)
                    .map_err(|_| "Auth public base path has an invalid escape")?
            } else {
                bytes[i]
            };
            if !c.is_ascii_graphic() || b"/\\?#%".contains(&c) {
                return Err("Auth public base path has unsafe encoded characters".into());
            }
            decoded.push(c);
            i += 1;
        }
        if decoded == b"." || decoded == b".." {
            return Err("Auth public base path cannot contain dot segments".into());
        }
    }
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Auth public base URL must be HTTPS (HTTP loopback only for development), without credentials, query or fragment".into());
    }
    Ok(format!(
        "{}{}",
        url.origin().ascii_serialization(),
        url.path().trim_end_matches('/')
    ))
}
impl MailConfig {
    pub fn resend(key: String, from: String, public_base_url: String) -> Result<Self, String> {
        if !(16..=4096).contains(&key.len())
            || !key.bytes().all(|c| c.is_ascii_graphic())
            || email(&from).is_none()
        {
            return Err(
                "Resend requires a private API key and a verified plain sender email address"
                    .into(),
            );
        }
        Ok(Self {
            key: key.into(),
            from: email(&from).ok_or("Auth mail sender is invalid")?,
            public_base_url: base_url(&public_base_url)?,
        })
    }
    pub fn delivery(&self) -> Result<Arc<dyn Delivery>, String> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(2))
            .pool_max_idle_per_host(1)
            .build()
            .map_err(|_| "Auth mail HTTPS client unavailable")?;
        Ok(Arc::new(Resend {
            config: self.clone(),
            client,
        }))
    }
}
struct Resend {
    config: MailConfig,
    client: reqwest::Client,
}
impl Delivery for Resend {
    fn deliver(
        &self,
        message: Message,
    ) -> Pin<Box<dyn Future<Output = Result<(), ()>> + Send + '_>> {
        Box::pin(async move {
            if email(&message.recipient).is_none() || message.link.len() > 1024 {
                return Err(());
            }
            let text = format!(
                "{}\n\n{}\n\nIf you did not request this email, ignore it. The link expires and can be used once.",
                message.purpose.subject(),
                message.link
            );
            let mut response=self.client.post("https://api.resend.com/emails").bearer_auth(self.config.key.as_ref())
                .header("Idempotency-Key",Uuid::new_v4().to_string())
                .json(&json!({"from":self.config.from,"to":[message.recipient],"subject":message.purpose.subject(),"text":text}))
                .send().await.map_err(|_|())?;
            if !response.status().is_success()
                || response.content_length().is_some_and(|n| n > 16384)
            {
                return Err(());
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
                if body.len() + chunk.len() > 16384 {
                    return Err(());
                }
                body.extend_from_slice(&chunk);
            }
            let result: serde_json::Value = serde_json::from_slice(&body).map_err(|_| ())?;
            if result
                .get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| !id.is_empty() && id.len() <= 128)
            {
                Ok(())
            } else {
                Err(())
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Busy,
    InvalidInput,
    Database,
}
impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "AUTH_MAIL_NOT_CONFIGURED",
            Self::Busy => "AUTH_FLOW_BUSY",
            Self::InvalidInput => "AUTH_FLOW_INPUT",
            Self::Database => "AUTH_FLOW_DATABASE",
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
fn database(_: String) -> Error {
    diagnostic(
        "AUTH_FLOW_DATABASE",
        "Inspect the private database and migration health; no source values were captured",
    );
    Error::Database
}
fn diagnostic(code: &'static str, fix: &'static str) {
    eprintln!(
        "{}",
        json!({"schemaVersion":1,"event":"auth-flow-diagnostic","private":true,"code":code,"rid":crate::http::request_reference(),"fix":fix,"valuesCaptured":false})
    );
}

#[derive(Clone)]
pub struct Flows {
    pool: PgPool,
    auth: Auth,
    roles: RolesConfig,
    base_url: String,
    delivery: Option<Arc<dyn Delivery>>,
    slots: Arc<Semaphore>,
}
impl Flows {
    pub fn new(
        pool: PgPool,
        auth: Auth,
        roles: RolesConfig,
        public_base_url: &str,
        delivery: Option<Arc<dyn Delivery>>,
    ) -> Result<Self, String> {
        roles.validate()?;
        if auth.signup_role() != roles.signup_role {
            return Err("Auth signup role and flow role configuration must match".into());
        }
        Ok(Self {
            pool,
            auth,
            roles,
            base_url: base_url(public_base_url)?,
            delivery,
            slots: Arc::new(Semaphore::new(4)),
        })
    }
    pub fn configured(&self) -> bool {
        self.delivery.is_some()
    }
    pub fn roles(&self) -> &RolesConfig {
        &self.roles
    }
    pub async fn request_reset(&self, address: &str) -> Result<(), Error> {
        self.request(Purpose::Reset, address, None).await
    }
    pub async fn request_verification(&self, user: Uuid) -> Result<(), Error> {
        self.request(Purpose::Verify, "", Some(user)).await
    }
    async fn request(
        &self,
        purpose: Purpose,
        address: &str,
        bound: Option<Uuid>,
    ) -> Result<(), Error> {
        let Some(delivery) = &self.delivery else {
            diagnostic(
                "AUTH_MAIL_NOT_CONFIGURED",
                "Configure a private sending-only Resend key, verified sender and explicit public base URL; no email was simulated",
            );
            return Err(Error::Unavailable);
        };
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let finish = tokio::time::Instant::now() + Duration::from_millis(2100);
        // Always use the same response floor for known, unknown, throttled and failed delivery cases.
        let result=tokio::time::timeout(Duration::from_secs(2),async {
            if let Some((token_hash,message))=self.prepare(purpose,address,bound).await? {
                if delivery.deliver(message).await.is_err() {return Err(Error::Unavailable)}
                // Pending hashes authorize nothing until delivery is confirmed. Never retry an uncertain send.
                sqlx::query("UPDATE public._brain_auth_tokens t SET activated_at=now() FROM public._brain_users u WHERE t.token_hash=$1 AND u.id=t.user_id AND u.email=t.email_bound AND u.auth_version=t.auth_version AND u.disabled_at IS NULL AND t.consumed_at IS NULL AND t.expires_at>now()")
                    .bind(token_hash).execute(&self.pool).await.map_err(|e|database(crate::db::safe(e)))?;
            } Ok::<_,Error>(())
        }).await;
        if !matches!(result, Ok(Ok(()))) {
            diagnostic(
                "AUTH_MAIL_DELIVERY_UNCONFIRMED",
                "Inspect private mail and database health and request a new link; no uncertain send was retried or reported as delivered",
            )
        }
        tokio::time::sleep_until(finish).await;
        Ok(())
    }
    async fn prepare(
        &self,
        purpose: Purpose,
        address: &str,
        bound: Option<Uuid>,
    ) -> Result<Option<(String, Message)>, Error> {
        let address = match (email(address), bound) {
            (Some(address), _) => address,
            (None, Some(_)) => String::new(),
            _ => return Ok(None),
        };
        let key = digest(&format!(
            "auth-flow:{}:{}",
            purpose.name(),
            bound
                .map(|u| u.to_string())
                .unwrap_or_else(|| address.clone())
        ));
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('brain:auth:flow-budget'))")
            .execute(&mut *tx)
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        sqlx::query("DELETE FROM public._brain_login_limits WHERE key IN(SELECT key FROM public._brain_login_limits WHERE expires_at<=now() LIMIT 100)").execute(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        let retained:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM(SELECT 1 FROM public._brain_login_limits LIMIT 10000) bounded").fetch_one(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        if retained >= 10000 {
            return Ok(None);
        }
        let count:i32=sqlx::query_scalar("INSERT INTO public._brain_login_limits(key,attempts,expires_at) VALUES($1,1,now()+interval '15 minutes') ON CONFLICT(key) DO UPDATE SET attempts=least(_brain_login_limits.attempts+1,4) RETURNING attempts").bind(key).fetch_one(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        if count > 3 {
            tx.commit()
                .await
                .map_err(|e| database(crate::db::safe(e)))?;
            return Ok(None);
        }
        let row=sqlx::query("SELECT id,email,auth_version,verified FROM public._brain_users WHERE disabled_at IS NULL AND (($2::uuid IS NULL AND email=$1) OR id=$2) FOR UPDATE").bind(&address).bind(bound).fetch_optional(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        let Some(row) = row else {
            tx.commit()
                .await
                .map_err(|e| database(crate::db::safe(e)))?;
            return Ok(None);
        };
        if matches!(purpose, Purpose::Verify) && row.get::<bool, _>("verified") {
            tx.commit()
                .await
                .map_err(|e| database(crate::db::safe(e)))?;
            return Ok(None);
        }
        sqlx::query("DELETE FROM public._brain_auth_tokens WHERE token_hash IN(SELECT token_hash FROM public._brain_auth_tokens WHERE expires_at<=now() ORDER BY expires_at LIMIT 100)").execute(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        let tokens:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM(SELECT 1 FROM public._brain_auth_tokens LIMIT 10000) bounded").fetch_one(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        if tokens >= 10000 {
            tx.commit()
                .await
                .map_err(|e| database(crate::db::safe(e)))?;
            return Ok(None);
        }
        let user: Uuid = row.get("id");
        let address: String = row.get("email");
        sqlx::query("UPDATE public._brain_auth_tokens SET consumed_at=now() WHERE user_id=$1 AND purpose=$2 AND consumed_at IS NULL").bind(user).bind(purpose.name()).execute(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        let opaque = crate::jwt::opaque();
        let hashed = digest(&opaque);
        sqlx::query("INSERT INTO public._brain_auth_tokens(token_hash,user_id,purpose,expires_at,email_bound,auth_version) VALUES($1,$2,$3,now()+$4::integer*interval '1 second',$5,$6)")
            .bind(&hashed).bind(user).bind(purpose.name()).bind(purpose.seconds()).bind(&address).bind(row.get::<i64,_>("auth_version")).execute(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        tx.commit()
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        Ok(Some((
            hashed,
            Message {
                recipient: address,
                link: format!("{}{}#token={opaque}", self.base_url, purpose.path()),
                purpose,
            },
        )))
    }
    pub async fn complete_reset(&self, token: &str, password: String) -> Result<bool, Error> {
        if !crate::jwt::valid_opaque(token) {
            return Ok(false);
        }
        if !(12..=1024).contains(&password.len()) {
            return Err(Error::InvalidInput);
        }
        let hash = digest(token);
        let user:Option<Uuid>=sqlx::query_scalar("SELECT user_id FROM public._brain_auth_tokens WHERE token_hash=$1 AND purpose='reset' AND activated_at IS NOT NULL AND consumed_at IS NULL AND expires_at>now() AND email_bound IS NOT NULL AND auth_version IS NOT NULL").bind(&hash).fetch_optional(&self.pool).await.map_err(|e|database(crate::db::safe(e)))?;
        let Some(user) = user else { return Ok(false) };
        let password_hash = self.auth.hash(password).await.map_err(|_| Error::Busy)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        let live=sqlx::query("SELECT email,auth_version FROM public._brain_users WHERE id=$1 AND disabled_at IS NULL FOR UPDATE").bind(user).fetch_optional(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        let Some(live) = live else { return Ok(false) };
        let changed=sqlx::query("UPDATE public._brain_auth_tokens SET consumed_at=now() WHERE token_hash=$1 AND user_id=$2 AND purpose='reset' AND activated_at IS NOT NULL AND consumed_at IS NULL AND expires_at>now() AND email_bound=$3 AND auth_version=$4")
            .bind(&hash).bind(user).bind(live.get::<String,_>("email")).bind(live.get::<i64,_>("auth_version")).execute(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?.rows_affected();
        if changed != 1 {
            return Ok(false);
        }
        sqlx::query("UPDATE public._brain_users SET password_hash=$2,auth_version=auth_version+1,updated_at=now() WHERE id=$1").bind(user).bind(password_hash).execute(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        revoke(&mut tx, user).await?;
        audit(&mut tx, user, "password-reset").await?;
        tx.commit()
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        Ok(true)
    }
    pub async fn verify_email(&self, user: Uuid, token: &str) -> Result<bool, Error> {
        if !crate::jwt::valid_opaque(token) {
            return Ok(false);
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        let live=sqlx::query("SELECT email,auth_version FROM public._brain_users WHERE id=$1 AND disabled_at IS NULL AND NOT verified FOR UPDATE").bind(user).fetch_optional(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        let Some(live) = live else { return Ok(false) };
        let changed=sqlx::query("UPDATE public._brain_auth_tokens SET consumed_at=now() WHERE token_hash=$1 AND user_id=$2 AND purpose='verify' AND activated_at IS NOT NULL AND consumed_at IS NULL AND expires_at>now() AND email_bound=$3 AND auth_version=$4")
            .bind(digest(token)).bind(user).bind(live.get::<String,_>("email")).bind(live.get::<i64,_>("auth_version")).execute(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?.rows_affected();
        if changed != 1 {
            return Ok(false);
        }
        sqlx::query("UPDATE public._brain_users SET verified=true,updated_at=now() WHERE id=$1")
            .bind(user)
            .execute(&mut *tx)
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        audit(&mut tx, user, "email-verified").await?;
        tx.commit()
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        Ok(true)
    }
    /// Caller supplies authenticated actor identity; authorization always reloads its current database role.
    pub async fn assign_role(&self, actor: Uuid, target: Uuid, role: &str) -> Result<bool, Error> {
        if !self.roles.roles.iter().any(|r| r == role) {
            return Err(Error::InvalidInput);
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('brain:auth:role-change'))")
            .execute(&mut *tx)
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        let rows=sqlx::query("SELECT id,role FROM public._brain_users WHERE id=ANY($1) AND disabled_at IS NULL ORDER BY id FOR UPDATE").bind(vec![actor,target]).fetch_all(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        if !rows
            .iter()
            .any(|r| r.get::<Uuid, _>("id") == actor && r.get::<String, _>("role") == "admin")
        {
            return Ok(false);
        }
        let Some(old) = rows
            .iter()
            .find(|r| r.get::<Uuid, _>("id") == target)
            .map(|r| r.get::<String, _>("role"))
        else {
            return Ok(false);
        };
        if old == role {
            return Ok(true);
        }
        if old == "admin" {
            let admins:i64=sqlx::query_scalar("SELECT count(*) FROM public._brain_users WHERE role='admin' AND disabled_at IS NULL").fetch_one(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
            if admins <= 1 {
                return Ok(false);
            }
        }
        sqlx::query("UPDATE public._brain_users SET role=$2,auth_version=auth_version+1,updated_at=now() WHERE id=$1").bind(target).bind(role).execute(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
        revoke(&mut tx, target).await?;
        audit(&mut tx, target, "role-changed").await?;
        tx.commit()
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
        Ok(true)
    }
}
async fn revoke(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, user: Uuid) -> Result<(), Error> {
    for sql in [
        "UPDATE public._brain_sessions SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL",
        "UPDATE public._brain_refresh_families SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL",
        "UPDATE public._brain_auth_tokens SET consumed_at=now() WHERE user_id=$1 AND consumed_at IS NULL",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(user)
            .execute(&mut **tx)
            .await
            .map_err(|e| database(crate::db::safe(e)))?;
    }
    Ok(())
}
async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: Uuid,
    event: &'static str,
) -> Result<(), Error> {
    sqlx::query(
        "INSERT INTO public._brain_auth_events(user_id,event,outcome) VALUES($1,$2,'success')",
    )
    .bind(user)
    .bind(event)
    .execute(&mut **tx)
    .await
    .map_err(|e| database(crate::db::safe(e)))?;
    Ok(())
}

/// Private operator entry point only. Never expose this bootstrap as a public HTTP route.
/// Promotes an explicitly selected existing account only while no enabled administrator exists.
pub async fn bootstrap_first_admin(pool: &PgPool, user: Uuid) -> Result<bool, Error> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| database(crate::db::safe(e)))?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('brain:auth:role-change'))")
        .execute(&mut *tx)
        .await
        .map_err(|e| database(crate::db::safe(e)))?;
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public._brain_users WHERE role='admin' AND disabled_at IS NULL)").fetch_one(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?;
    if exists {
        return Ok(false);
    }
    let changed=sqlx::query("UPDATE public._brain_users SET role='admin',auth_version=auth_version+1,updated_at=now() WHERE id=$1 AND disabled_at IS NULL").bind(user).execute(&mut *tx).await.map_err(|e|database(crate::db::safe(e)))?.rows_affected();
    if changed != 1 {
        return Ok(false);
    }
    revoke(&mut tx, user).await?;
    audit(&mut tx, user, "admin-bootstrapped").await?;
    tx.commit()
        .await
        .map_err(|e| database(crate::db::safe(e)))?;
    Ok(true)
}
