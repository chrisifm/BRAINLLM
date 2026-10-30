//! Fixed-algorithm short access tokens backed by revocable database sessions and rotating opaque refresh tokens.
use crate::auth::{self, User};
use axum::http::{HeaderMap, header};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

pub const ACCESS_COOKIE: &str = "__Host-brain-access";
pub const REFRESH_COOKIE: &str = "__Host-brain-refresh";
pub const ACCESS_SECONDS: u32 = 300;
pub const REFRESH_SECONDS: u32 = 604800;
#[derive(Clone)]
pub struct Config {
    secret: Arc<str>,
    pub issuer: String,
    pub audience: String,
}
impl Config {
    pub fn new(secret: String, issuer: String, audience: String) -> Result<Self, String> {
        if secret.len() < 32
            || secret.len() > 4096
            || issuer.is_empty()
            || audience.is_empty()
            || issuer.len() > 255
            || audience.len() > 255
            || issuer.chars().any(char::is_control)
            || audience.chars().any(char::is_control)
        {
            return Err("JWT needs a private secret of 32..4096 bytes and explicit issuer/audience of 1..255 characters".into());
        }
        Ok(Self {
            secret: secret.into(),
            issuer,
            audience,
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Claims {
    sub: Uuid,
    sid: Uuid,
    ver: i64,
    iss: String,
    aud: String,
    iat: u64,
    nbf: u64,
    exp: u64,
    jti: Uuid,
}
#[derive(Serialize)]
pub struct Tokens {
    pub user: User,
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u32,
    #[serde(skip)]
    pub refresh_token: String,
}
#[derive(Clone)]
pub struct Jwt {
    pool: PgPool,
    config: Config,
    encoding: EncodingKey,
    decoding: DecodingKey,
}
pub fn now() -> u64 {
    jsonwebtoken::get_current_timestamp()
}
pub(crate) fn opaque() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
pub fn valid_opaque(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for part in header.to_str().ok()?.split(';') {
            if let Some((key, value)) = part.trim().split_once('=') {
                if key == name {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(value)
                }
            }
        }
    }
    found
}
pub fn access<'a>(headers: &'a HeaderMap) -> Option<&'a str> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        return value
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")
            .filter(|value| !value.is_empty());
    }
    cookie(headers, ACCESS_COOKIE)
}
pub fn cookie_header(name: &str, value: &str, age: u32) -> String {
    format!("{name}={value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={age}")
}
impl Jwt {
    pub fn new(pool: PgPool, config: Config) -> Self {
        let _ = jsonwebtoken::crypto::rust_crypto::DEFAULT_PROVIDER.install_default();
        let encoding = EncodingKey::from_secret(config.secret.as_bytes());
        let decoding = DecodingKey::from_secret(config.secret.as_bytes());
        Self {
            pool,
            config,
            encoding,
            decoding,
        }
    }
    fn sign(&self, user: Uuid, sid: Uuid, version: i64) -> Result<String, String> {
        let time = now();
        let claims = Claims {
            sub: user,
            sid,
            ver: version,
            iss: self.config.issuer.clone(),
            aud: self.config.audience.clone(),
            iat: time,
            nbf: time,
            exp: time + u64::from(ACCESS_SECONDS),
            jti: Uuid::new_v4(),
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(|_| "Access token signing unavailable".into())
    }
    fn decode(&self, token: &str) -> Option<Claims> {
        if token.len() > 8192 {
            return None;
        }
        let header = decode_header(token).ok()?;
        if header.alg != Algorithm::HS256
            || header.crit.is_some()
            || header.jku.is_some()
            || header.jwk.is_some()
            || header.x5u.is_some()
            || header.enc.is_some()
            || header.zip.is_some()
        {
            return None;
        }
        let mut validation = Validation::new(Algorithm::HS256);
        validation.leeway = 0;
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["exp", "nbf", "iss", "aud", "sub"]);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[&self.config.audience]);
        let claims = decode::<Claims>(token, &self.decoding, &validation)
            .ok()?
            .claims;
        if claims.iat > now()
            || claims.exp <= claims.iat
            || claims.exp - claims.iat > u64::from(ACCESS_SECONDS)
            || claims.nbf != claims.iat
            || claims.ver < 0
        {
            return None;
        }
        Some(claims)
    }
    /// Every request rechecks authoritative role, disabled state, version and session revocation.
    pub async fn user(&self, token: Option<&str>) -> Result<Option<User>, String> {
        let Some(claims) = token.and_then(|token| self.decode(token)) else {
            return Ok(None);
        };
        let row=sqlx::query("SELECT u.id,u.email,u.role,u.verified FROM _brain_users u JOIN _brain_sessions s ON s.user_id=u.id WHERE u.id=$1 AND s.session_id=$2 AND u.auth_version=$3 AND u.disabled_at IS NULL AND s.revoked_at IS NULL AND s.expires_at>now()")
            .bind(claims.sub).bind(claims.sid).bind(claims.ver).fetch_optional(&self.pool).await.map_err(crate::db::safe)?;
        Ok(row.map(|row| User {
            id: row.get("id"),
            email: row.get("email"),
            role: row.get("role"),
            verified: row.get("verified"),
        }))
    }
    /// Upgrade the already authenticated opaque session; no second session is created and the existing cap is preserved.
    pub async fn issue(&self, user: User, session_token: &str) -> Result<Tokens, String> {
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        sqlx::query("SELECT id FROM _brain_users WHERE id=$1 FOR UPDATE")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(crate::db::safe)?;
        let row=sqlx::query("SELECT s.session_id,u.auth_version FROM _brain_sessions s JOIN _brain_users u ON u.id=s.user_id WHERE s.token_hash=$1 AND u.id=$2 AND u.disabled_at IS NULL AND s.revoked_at IS NULL AND s.expires_at>now() FOR UPDATE OF s")
            .bind(auth::digest(session_token)).bind(user.id).fetch_optional(&mut *tx).await.map_err(crate::db::safe)?.ok_or("Session unavailable")?;
        let sid: Uuid = row.get("session_id");
        let version: i64 = row.get("auth_version");
        let access_token = self.sign(user.id, sid, version)?;
        // Retain replay evidence within explicit per-account and per-family budgets.
        sqlx::query("DELETE FROM _brain_refresh_families WHERE id IN (SELECT id FROM _brain_refresh_families WHERE user_id=$1 AND expires_at<=now() ORDER BY expires_at LIMIT 1)").bind(user.id).execute(&mut *tx).await.map_err(crate::db::safe)?;
        let retained:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM (SELECT 1 FROM _brain_refresh_families WHERE user_id=$1 LIMIT 1000) retained").bind(user.id).fetch_one(&mut *tx).await.map_err(crate::db::safe)?;
        if retained >= 1000 {
            return Err(
                "Session retention budget reached; retry after expired sessions are cleaned".into(),
            );
        }
        let refresh_token = opaque();
        let family = Uuid::new_v4();
        sqlx::query("INSERT INTO _brain_refresh_families(id,session_id,user_id,auth_version,expires_at) SELECT $1,session_id,user_id,$3,LEAST(expires_at,now()+interval '7 days') FROM _brain_sessions WHERE session_id=$2").bind(family).bind(sid).bind(version).execute(&mut *tx).await.map_err(crate::db::safe)?;
        sqlx::query("INSERT INTO _brain_refresh_tokens(token_hash,family_id,expires_at) SELECT $1,id,expires_at FROM _brain_refresh_families WHERE id=$2").bind(auth::digest(&refresh_token)).bind(family).execute(&mut *tx).await.map_err(crate::db::safe)?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(Tokens {
            user,
            access_token,
            refresh_token,
            token_type: "Bearer",
            expires_in: ACCESS_SECONDS,
        })
    }
    /// Single-use rotation. Reuse revokes the family AND its session, immediately invalidating access tokens.
    pub async fn refresh(&self, token: Option<&str>) -> Result<Option<Tokens>, String> {
        let Some(token) = token.filter(|token| valid_opaque(token)) else {
            return Ok(None);
        };
        let hash = auth::digest(token);
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        let user:Option<Uuid>=sqlx::query_scalar("SELECT s.user_id FROM _brain_refresh_tokens t JOIN _brain_refresh_families f ON f.id=t.family_id JOIN _brain_sessions s ON s.session_id=f.session_id WHERE t.token_hash=$1").bind(&hash).fetch_optional(&mut *tx).await.map_err(crate::db::safe)?;
        let Some(user) = user else { return Ok(None) };
        sqlx::query("SELECT id FROM _brain_users WHERE id=$1 FOR UPDATE")
            .bind(user)
            .fetch_one(&mut *tx)
            .await
            .map_err(crate::db::safe)?;
        let row=sqlx::query("SELECT u.id,u.email,u.role,u.verified,u.auth_version,s.session_id,f.id AS family_id,t.used_at IS NOT NULL AS used, f.revoked_at IS NULL AND f.expires_at>now() AND t.expires_at>now() AND s.expires_at>now() AND s.revoked_at IS NULL AND u.disabled_at IS NULL AND f.auth_version=u.auth_version AS live FROM _brain_refresh_tokens t JOIN _brain_refresh_families f ON f.id=t.family_id JOIN _brain_sessions s ON s.session_id=f.session_id JOIN _brain_users u ON u.id=s.user_id WHERE t.token_hash=$1 FOR UPDATE OF t,f,s")
            .bind(&hash).fetch_optional(&mut *tx).await.map_err(crate::db::safe)?;
        let Some(row) = row else { return Ok(None) };
        let family: Uuid = row.get("family_id");
        let sid: Uuid = row.get("session_id");
        if row.get::<bool, _>("used") {
            sqlx::query("UPDATE _brain_refresh_families SET revoked_at=now() WHERE id=$1")
                .bind(family)
                .execute(&mut *tx)
                .await
                .map_err(crate::db::safe)?;
            sqlx::query("UPDATE _brain_sessions SET revoked_at=now() WHERE session_id=$1")
                .bind(sid)
                .execute(&mut *tx)
                .await
                .map_err(crate::db::safe)?;
            sqlx::query("INSERT INTO _brain_auth_events(user_id,event,outcome) VALUES ($1,'refresh-reuse','denied')").bind(user).execute(&mut *tx).await.map_err(crate::db::safe)?;
            tx.commit().await.map_err(crate::db::safe)?;
            return Ok(None);
        }
        if !row.get::<bool, _>("live") {
            return Ok(None);
        }
        let retained:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM (SELECT 1 FROM _brain_refresh_tokens WHERE family_id=$1 LIMIT 4096) retained").bind(family).fetch_one(&mut *tx).await.map_err(crate::db::safe)?;
        if retained >= 4096 {
            sqlx::query("UPDATE _brain_refresh_families SET revoked_at=now() WHERE id=$1")
                .bind(family)
                .execute(&mut *tx)
                .await
                .map_err(crate::db::safe)?;
            sqlx::query("UPDATE _brain_sessions SET revoked_at=now() WHERE session_id=$1")
                .bind(sid)
                .execute(&mut *tx)
                .await
                .map_err(crate::db::safe)?;
            tx.commit().await.map_err(crate::db::safe)?;
            return Ok(None);
        }
        let user = User {
            id: row.get("id"),
            email: row.get("email"),
            role: row.get("role"),
            verified: row.get("verified"),
        };
        let access_token = self.sign(user.id, sid, row.get("auth_version"))?;
        let refresh_token = opaque();
        let next_hash = auth::digest(&refresh_token);
        sqlx::query("INSERT INTO _brain_refresh_tokens(token_hash,family_id,parent_hash,expires_at) SELECT $1,id,$2,expires_at FROM _brain_refresh_families WHERE id=$3").bind(&next_hash).bind(&hash).bind(family).execute(&mut *tx).await.map_err(crate::db::safe)?;
        sqlx::query("UPDATE _brain_refresh_tokens SET used_at=now(),replacement_hash=$1 WHERE token_hash=$2").bind(&next_hash).bind(&hash).execute(&mut *tx).await.map_err(crate::db::safe)?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(Some(Tokens {
            user,
            access_token,
            refresh_token,
            token_type: "Bearer",
            expires_in: ACCESS_SECONDS,
        }))
    }
    pub async fn logout(&self, headers: &HeaderMap) -> Result<(), String> {
        let sid = if let Some(claims) = access(headers).and_then(|token| self.decode(token)) {
            Some(claims.sid)
        } else if let Some(token) =
            cookie(headers, REFRESH_COOKIE).filter(|token| valid_opaque(token))
        {
            sqlx::query_scalar("SELECT f.session_id FROM _brain_refresh_families f JOIN _brain_refresh_tokens t ON t.family_id=f.id WHERE t.token_hash=$1 AND f.session_id IS NOT NULL").bind(auth::digest(token)).fetch_optional(&self.pool).await.map_err(crate::db::safe)?
        } else {
            None
        };
        if let Some(sid) = sid {
            let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
            let user: Option<Uuid> =
                sqlx::query_scalar("SELECT user_id FROM _brain_sessions WHERE session_id=$1")
                    .bind(sid)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(crate::db::safe)?;
            if let Some(user) = user {
                sqlx::query("SELECT id FROM _brain_users WHERE id=$1 FOR UPDATE")
                    .bind(user)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(crate::db::safe)?;
                sqlx::query(
                    "UPDATE _brain_refresh_families SET revoked_at=now() WHERE session_id=$1",
                )
                .bind(sid)
                .execute(&mut *tx)
                .await
                .map_err(crate::db::safe)?;
                sqlx::query("UPDATE _brain_sessions SET revoked_at=now() WHERE session_id=$1")
                    .bind(sid)
                    .execute(&mut *tx)
                    .await
                    .map_err(crate::db::safe)?;
            }
            tx.commit().await.map_err(crate::db::safe)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn service() -> Jwt {
        Jwt::new(
            sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
                .unwrap(),
            Config::new(
                "synthetic-test-secret-32-bytes-long".into(),
                "issuer".into(),
                "audience".into(),
            )
            .unwrap(),
        )
    }
    #[tokio::test]
    async fn fixed_algorithm_claims_signature_and_lifetime_are_enforced_without_database() {
        let service = service();
        let uid = Uuid::new_v4();
        let sid = Uuid::new_v4();
        let token = service.sign(uid, sid, 2).unwrap();
        let claims = service.decode(&token).unwrap();
        assert_eq!(claims.sub, uid);
        assert_eq!(claims.sid, sid);
        assert_eq!(claims.ver, 2);
        let mut bad = claims.clone();
        bad.aud = "other".into();
        assert!(
            service
                .decode(&encode(&Header::default(), &bad, &service.encoding).unwrap())
                .is_none()
        );
        bad = claims.clone();
        bad.exp = now() - 1;
        assert!(
            service
                .decode(&encode(&Header::default(), &bad, &service.encoding).unwrap())
                .is_none()
        );
        bad = claims.clone();
        bad.nbf = now() + 60;
        assert!(
            service
                .decode(&encode(&Header::default(), &bad, &service.encoding).unwrap())
                .is_none()
        );
        bad = claims;
        bad.exp = now() + 3600;
        assert!(
            service
                .decode(&encode(&Header::default(), &bad, &service.encoding).unwrap())
                .is_none()
        );
        assert!(
            service
                .decode(&encode(&Header::new(Algorithm::HS384), &bad, &service.encoding).unwrap())
                .is_none()
        );
        let valid = service.decode(&token).unwrap();
        let wrong = EncodingKey::from_secret(b"different-synthetic-private-secret");
        assert!(
            service
                .decode(&encode(&Header::default(), &valid, &wrong).unwrap())
                .is_none()
        );
    }
    #[test]
    fn config_cookies_and_refresh_shapes_are_bounded_and_private() {
        assert!(Config::new("short".into(), "i".into(), "a".into()).is_err());
        assert!(valid_opaque(&opaque()));
        assert!(!valid_opaque("x"));
        let cookie = cookie_header(REFRESH_COOKIE, "test", REFRESH_SECONDS);
        assert!(cookie.contains("HttpOnly; Secure; SameSite=Lax"));
    }
}
