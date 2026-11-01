//! What an account does with its own sessions: list them, end one or all the others, prove the password again, and the
//! one-use grant between a verified signup and its login. Revocation is by `session_id`, always scoped to the caller.
use crate::auth::digest;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::sync::OnceLock;
use uuid::Uuid;

const GRANT_SECONDS: f64 = 300.0;
static IDLE_SECONDS: OnceLock<u32> = OnceLock::new();
/// `BRAIN_AUTH_IDLE_SECONDS`: a session unused for this long stops working. Unset, sessions end only at their expiry.
pub fn configure_idle_seconds(seconds: u32) -> Result<(), String> {
    if !(300..=31_536_000).contains(&seconds) {
        return Err("BRAIN_AUTH_IDLE_SECONDS must be 300..31536000".into());
    }
    IDLE_SECONDS.set(seconds).map_err(|_| "Idle timeout is already configured".to_string())
}
pub(crate) fn idle_seconds() -> Option<f64> {
    IDLE_SECONDS.get().map(|seconds| f64::from(*seconds))
}
/// The last use is written at most once a minute, so a busy session costs no write per request. Losing one only makes
/// the idle clock earlier, and the idle check fails closed.
pub(crate) async fn touch(pool: &PgPool, session: Uuid) {
    let _ = sqlx::query("UPDATE public._brain_sessions SET last_used_at=now() WHERE session_id=$1").bind(session).execute(pool).await;
}
const STAMP: &str = "YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"";
pub async fn list(pool: &PgPool, user: Uuid, current: Uuid) -> Result<Vec<Value>, String> {
    let rows = sqlx::query("SELECT session_id,to_char(created_at AT TIME ZONE 'UTC',$3) AS created,to_char(last_used_at AT TIME ZONE 'UTC',$3) AS used,to_char(auth_time AT TIME ZONE 'UTC',$3) AS proved,to_char(expires_at AT TIME ZONE 'UTC',$3) AS expires FROM public._brain_sessions WHERE user_id=$1 AND revoked_at IS NULL AND expires_at>now() AND ($2::float8 IS NULL OR last_used_at>now()-make_interval(secs=>$2)) ORDER BY last_used_at DESC,session_id")
        .bind(user)
        .bind(idle_seconds())
        .bind(STAMP)
        .fetch_all(pool)
        .await
        .map_err(crate::db::safe)?;
    Ok(rows
        .iter()
        .map(|row| {
            let id: Uuid = row.get("session_id");
            json!({"id":id,"current":id==current,"created_at":row.get::<String,_>("created"),"last_used_at":row.get::<String,_>("used"),"auth_time":row.get::<String,_>("proved"),"expires_at":row.get::<String,_>("expires")})
        })
        .collect())
}
/// Ends sessions of `user` chosen by `only` or by everything but `except`, with their refresh families. The owner is
/// always part of the match, so another account's id behaves exactly like an unknown one. Returns how many ended.
async fn end(pool: &PgPool, user: Uuid, only: Option<Uuid>, except: Option<Uuid>) -> Result<u64, String> {
    let mut tx = pool.begin().await.map_err(crate::db::safe)?;
    sqlx::query("SELECT id FROM public._brain_users WHERE id=$1 FOR UPDATE").bind(user).fetch_one(&mut *tx).await.map_err(crate::db::safe)?;
    let ended: Vec<Uuid> = sqlx::query_scalar("UPDATE public._brain_sessions SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL AND ($2::uuid IS NULL OR session_id=$2) AND ($3::uuid IS NULL OR session_id<>$3) RETURNING session_id")
        .bind(user)
        .bind(only)
        .bind(except)
        .fetch_all(&mut *tx)
        .await
        .map_err(crate::db::safe)?;
    sqlx::query("UPDATE public._brain_refresh_families SET revoked_at=now() WHERE user_id=$1 AND session_id=ANY($2) AND revoked_at IS NULL").bind(user).bind(&ended).execute(&mut *tx).await.map_err(crate::db::safe)?;
    if !ended.is_empty() {
        sqlx::query("INSERT INTO public._brain_auth_events(user_id,event,outcome) VALUES ($1,'sessions-ended','success')").bind(user).execute(&mut *tx).await.map_err(crate::db::safe)?;
    }
    tx.commit().await.map_err(crate::db::safe)?;
    Ok(ended.len() as u64)
}
pub async fn revoke_one(pool: &PgPool, user: Uuid, session: Uuid) -> Result<bool, String> {
    end(pool, user, Some(session), None).await.map(|count| count == 1)
}
pub async fn revoke_others(pool: &PgPool, user: Uuid, current: Uuid) -> Result<u64, String> {
    end(pool, user, None, Some(current)).await
}
/// `None` when the session is gone; the age is computed by the database clock that wrote it.
pub async fn auth_age(pool: &PgPool, user: Uuid, session: Uuid) -> Result<Option<f64>, String> {
    sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM now()-auth_time)::float8 FROM public._brain_sessions WHERE session_id=$1 AND user_id=$2 AND revoked_at IS NULL AND expires_at>now()")
        .bind(session)
        .bind(user)
        .fetch_optional(pool)
        .await
        .map_err(crate::db::safe)
}
/// Created only after the provider confirmed the signup's token; good for one login of that account for five minutes.
pub async fn signup_grant(pool: &PgPool, user: Uuid) -> Result<String, String> {
    let grant = crate::jwt::opaque();
    sqlx::query("DELETE FROM public._brain_auth_challenges WHERE token_hash IN (SELECT token_hash FROM public._brain_auth_challenges WHERE purpose='signup_login' AND expires_at<=now() ORDER BY expires_at LIMIT 100)").execute(pool).await.map_err(crate::db::safe)?;
    sqlx::query("INSERT INTO public._brain_auth_challenges(token_hash,purpose,user_id,expires_at) VALUES ($1,'signup_login',$2,now()+make_interval(secs=>$3))")
        .bind(digest(&grant))
        .bind(user)
        .bind(GRANT_SECONDS)
        .execute(pool)
        .await
        .map_err(crate::db::safe)?;
    Ok(grant)
}
/// Spends the grant whatever happens to the password next; it is bound to the account of `email`.
pub async fn consume_signup_grant(pool: &PgPool, grant: &str, email: &str) -> Result<bool, String> {
    if !crate::jwt::valid_opaque(grant) {
        return Ok(false);
    }
    let Ok(email) = crate::auth::Auth::email(email) else { return Ok(false) };
    sqlx::query("UPDATE public._brain_auth_challenges SET consumed_at=now() WHERE token_hash=$1 AND purpose='signup_login' AND consumed_at IS NULL AND expires_at>now() AND user_id=(SELECT id FROM public._brain_users WHERE email=$2 AND disabled_at IS NULL)")
        .bind(digest(grant))
        .bind(email)
        .execute(pool)
        .await
        .map(|done| done.rows_affected() == 1)
        .map_err(crate::db::safe)
}
