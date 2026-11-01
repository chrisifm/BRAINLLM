//! Credits: an atomic balance and the append-only ledger that explains it. Every change locks the subject's balance row,
//! so concurrent debits serialize and none can take the balance below zero; a repeated idempotency key returns the
//! recorded result instead of charging again.
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub const MAX_DELTA: i64 = 9_007_199_254_740_991;
pub const MAX_PAGE: i64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub delta: i64,
    pub balance: i64,
    /// The key was already used: nothing changed and this is the original result.
    pub replayed: bool,
}
#[derive(Debug)]
pub enum Refusal {
    Insufficient,
    /// The balance would pass the largest amount a JSON client can read exactly.
    TooLarge,
    /// This key already recorded a different change.
    KeyReused,
    UnknownSubject,
    Invalid(&'static str),
    Database(sqlx::Error),
}
impl From<sqlx::Error> for Refusal {
    fn from(error: sqlx::Error) -> Self {
        Refusal::Database(error)
    }
}
/// Applies one signed change: a positive delta grants, a negative one debits.
pub async fn apply(pool: &PgPool, subject: Uuid, delta: i64, reason: &str, key: &str, actor: Option<Uuid>) -> Result<Entry, Refusal> {
    let mut tx = pool.begin().await?;
    let entry = apply_in(&mut tx, subject, delta, reason, key, actor).await?;
    tx.commit().await?;
    Ok(entry)
}
/// The same change inside a caller's transaction: it commits, or rolls back, with the caller's other writes.
pub async fn apply_in(tx: &mut sqlx::PgConnection, subject: Uuid, delta: i64, reason: &str, key: &str, actor: Option<Uuid>) -> Result<Entry, Refusal> {
    if delta == 0 || !(-MAX_DELTA..=MAX_DELTA).contains(&delta) {
        return Err(Refusal::Invalid("delta must be a non-zero integer within the safe range"));
    }
    if reason.trim().is_empty() || reason.len() > 128 {
        return Err(Refusal::Invalid("reason must be 1 to 128 bytes"));
    }
    if key.is_empty() || key.len() > 128 {
        return Err(Refusal::Invalid("idempotency_key must be 1 to 128 bytes"));
    }
    let known: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _brain_users WHERE id=$1) OR EXISTS(SELECT 1 FROM _brain_orgs WHERE id=$1)").bind(subject).fetch_one(&mut *tx).await?;
    if !known {
        return Err(Refusal::UnknownSubject);
    }
    sqlx::query("INSERT INTO _brain_credit_balances(subject,balance) VALUES($1,0) ON CONFLICT (subject) DO NOTHING").bind(subject).execute(&mut *tx).await?;
    // The row lock orders every change of this subject, including two requests with the same key.
    let held: i64 = sqlx::query_scalar("SELECT balance FROM _brain_credit_balances WHERE subject=$1 FOR UPDATE").bind(subject).fetch_one(&mut *tx).await?;
    let earlier = sqlx::query("SELECT id,delta,balance_after,reason FROM _brain_credit_ledger WHERE subject=$1 AND idempotency_key=$2").bind(subject).bind(key).fetch_optional(&mut *tx).await?;
    if let Some(row) = earlier {
        if row.get::<i64, _>("delta") != delta || row.get::<String, _>("reason") != reason {
            return Err(Refusal::KeyReused);
        }
        return Ok(Entry { id: row.get("id"), delta, balance: row.get("balance_after"), replayed: true });
    }
    let balance = held.checked_add(delta).ok_or(Refusal::TooLarge)?;
    if balance < 0 {
        return Err(Refusal::Insufficient);
    }
    if balance > MAX_DELTA {
        return Err(Refusal::TooLarge);
    }
    sqlx::query("UPDATE _brain_credit_balances SET balance=$2,updated_at=now() WHERE subject=$1").bind(subject).bind(balance).execute(&mut *tx).await?;
    let id: i64 = sqlx::query_scalar("INSERT INTO _brain_credit_ledger(subject,delta,balance_after,reason,idempotency_key,actor,request) VALUES($1,$2,$3,$4,$5,$6,$7) RETURNING id")
        .bind(subject).bind(delta).bind(balance).bind(reason).bind(key).bind(actor).bind(crate::http::request_reference())
        .fetch_one(&mut *tx).await?;
    Ok(Entry { id, delta, balance, replayed: false })
}
/// The balance and one newest-first page of its ledger. A subject that never had credits holds zero.
pub async fn statement(pool: &PgPool, subject: Uuid, before: Option<i64>, limit: i64) -> Result<Value, sqlx::Error> {
    let limit = limit.clamp(1, MAX_PAGE);
    let balance: Option<i64> = sqlx::query_scalar("SELECT balance FROM _brain_credit_balances WHERE subject=$1").bind(subject).fetch_optional(pool).await?;
    let rows = sqlx::query("SELECT id,delta,balance_after,reason,actor,to_jsonb(created_at)#>>'{}' AS at FROM _brain_credit_ledger WHERE subject=$1 AND ($2::bigint IS NULL OR id<$2) ORDER BY id DESC LIMIT $3")
        .bind(subject).bind(before).bind(limit + 1).fetch_all(pool).await?;
    let more = rows.len() as i64 > limit;
    let items: Vec<Value> = rows.iter().take(limit as usize).map(|r| json!({
        "id": r.get::<i64, _>("id"), "delta": r.get::<i64, _>("delta"), "balance": r.get::<i64, _>("balance_after"),
        "reason": r.get::<String, _>("reason"), "actor": r.get::<Option<Uuid>, _>("actor"), "at": r.get::<String, _>("at"),
    })).collect();
    let next = if more { items.last().map(|i| i["id"].clone()) } else { None };
    Ok(json!({"subject": subject, "balance": balance.unwrap_or(0), "items": items, "next": next}))
}
