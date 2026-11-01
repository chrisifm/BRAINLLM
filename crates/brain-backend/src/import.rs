//! Reviewed one-time import (B-28, B-39): accounts and records keep their ids, owners and references. One file is one
//! transaction; a preview runs every insert and rolls back, so what it reports is what `apply` would do. Imports fire no
//! hook, webhook or socket event: history is being restored, nothing has just happened.
use crate::{Resource, auth::Auth, contract::identifier};
use serde_json::Value;
use sqlx::{Acquire, PgPool, Postgres, Transaction};
use uuid::Uuid;

/// A stored hash this backend can verify at login; the first successful login replaces a scrypt one with Argon2.
pub fn supported_hash(hash: &str) -> bool {
    crate::auth::legacy_scrypt(hash).is_some() || (hash.starts_with("$argon2") && argon2::password_hash::phc::PasswordHash::new(hash).is_ok())
}

#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub rows: usize,
    pub applied: bool,
    /// Line number and reason. Reasons never quote a value from the file.
    pub problems: Vec<(usize, String)>,
    /// Rows past the reported problems that also failed.
    pub more_problems: usize,
}
impl Outcome {
    fn problem(&mut self, line: usize, reason: impl Into<String>) {
        if self.problems.len() < 20 {
            self.problems.push((line, reason.into()));
        } else {
            self.more_problems += 1;
        }
    }
}

fn rows(text: &str) -> impl Iterator<Item = (usize, Result<Value, serde_json::Error>)> + '_ {
    text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()).map(|(i, l)| (i + 1, serde_json::from_str(l)))
}
fn id_of(row: &Value, key: &str) -> Option<Uuid> {
    row.get(key)?.as_str()?.parse().ok()
}
fn timestamp(row: &Value, key: &str) -> Result<Option<String>, String> {
    match row.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if chrono::DateTime::parse_from_rfc3339(s).is_ok() => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{key}` must be an RFC 3339 timestamp with its offset")),
    }
}
fn unknown_keys(row: &Value, allowed: &[&str]) -> Option<String> {
    row.as_object()?.keys().find(|k| !allowed.contains(&k.as_str())).map(|k| format!("unknown key `{}`", if identifier(k) { k.as_str() } else { "?" }))
}
async fn finish(tx: Transaction<'_, Postgres>, mut outcome: Outcome, apply: bool) -> Result<Outcome, String> {
    if apply && outcome.problems.is_empty() {
        tx.commit().await.map_err(crate::db::safe)?;
        outcome.applied = true;
    } else {
        tx.rollback().await.map_err(crate::db::safe)?;
    }
    Ok(outcome)
}
/// The database's own refusal, reduced to its kind: messages can quote the row.
fn refusal(error: &sqlx::Error) -> String {
    match error.as_database_error().and_then(|e| e.code()).as_deref() {
        Some("23505") => "this id or a unique value already exists".into(),
        Some("23503") => "its owner or a reference does not exist yet; fix: import the target first, in dependency order".into(),
        Some("23514") | Some("23502") => "a declared constraint refuses this row".into(),
        _ => "the database refused this row".into(),
    }
}

/// `{"id","email","password_hash","role","verified","created_at"}` per line; `id`, `email` and `role` are required.
/// Without a hash the account exists with no usable password, and recovery or a linked provider signs it in.
pub async fn users(pool: &PgPool, roles: &[String], unusable_hash: &str, text: &str, apply: bool) -> Result<Outcome, String> {
    let mut outcome = Outcome::default();
    let mut tx = pool.begin().await.map_err(crate::db::safe)?;
    for (line, row) in rows(text) {
        outcome.rows += 1;
        let Ok(row) = row else {
            outcome.problem(line, "not valid JSON");
            continue;
        };
        if let Some(reason) = unknown_keys(&row, &["id", "email", "password_hash", "role", "verified", "created_at"]) {
            outcome.problem(line, reason);
            continue;
        }
        let Some(id) = id_of(&row, "id") else {
            outcome.problem(line, "`id` must be a UUID");
            continue;
        };
        let Some(Ok(email)) = row.get("email").and_then(Value::as_str).map(Auth::email) else {
            outcome.problem(line, "`email` is missing or invalid");
            continue;
        };
        let Some(role) = row.get("role").and_then(Value::as_str).filter(|r| roles.iter().any(|d| d == r)) else {
            outcome.problem(line, "`role` is not declared in server/auth/roles.jsbrain");
            continue;
        };
        let hash = match row.get("password_hash") {
            None | Some(Value::Null) => unusable_hash,
            Some(Value::String(h)) if supported_hash(h) => h.as_str(),
            Some(_) => {
                outcome.problem(line, "`password_hash` is not a PHC string for scrypt or Argon2; fix: convert it to `$scrypt$ln=..,r=..,p=..$salt$hash` (unpadded base64), or leave it out and send the account through recovery");
                continue;
            }
        };
        let created = match timestamp(&row, "created_at") {
            Ok(created) => created,
            Err(reason) => {
                outcome.problem(line, reason);
                continue;
            }
        };
        let verified = row.get("verified").and_then(Value::as_bool).unwrap_or(false);
        let mut one = tx.begin().await.map_err(crate::db::safe)?;
        let done = sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role,verified,created_at) VALUES($1,$2,$3,$4,$5,COALESCE($6::timestamptz,now()))")
            .bind(id).bind(&email).bind(hash).bind(role).bind(verified).bind(created)
            .execute(&mut *one).await;
        match done {
            Ok(_) => one.commit().await.map_err(crate::db::safe)?,
            Err(error) => {
                outcome.problem(line, refusal(&error));
                one.rollback().await.map_err(crate::db::safe)?;
            }
        }
    }
    finish(tx, outcome, apply).await
}

/// `{"id","owner","data","created_at","updated_at"}` per line; `id`, `owner` and `data` are required. `data` passes the
/// same field validation as a create; state-machine and hook rules do not run, because history already happened.
pub async fn records(pool: &PgPool, r: &Resource, text: &str, apply: bool) -> Result<Outcome, String> {
    let mut outcome = Outcome::default();
    let insert = format!("INSERT INTO _brain_res_{}(id,owner,data,created_at,updated_at) VALUES($1,$2,$3,COALESCE($4::timestamptz,now()),COALESCE($5::timestamptz,$4::timestamptz,now()))", r.name);
    let mut tx = pool.begin().await.map_err(crate::db::safe)?;
    for (line, row) in rows(text) {
        outcome.rows += 1;
        let Ok(row) = row else {
            outcome.problem(line, "not valid JSON");
            continue;
        };
        if let Some(reason) = unknown_keys(&row, &["id", "owner", "data", "created_at", "updated_at"]) {
            outcome.problem(line, reason);
            continue;
        }
        let (Some(id), Some(owner)) = (id_of(&row, "id"), id_of(&row, "owner")) else {
            outcome.problem(line, "`id` and `owner` must be UUIDs");
            continue;
        };
        let data = match row.get("data").map(|d| r.input(d)) {
            Some(Ok(data)) => data,
            Some(Err(fields)) => {
                outcome.problem(line, format!("`data` is invalid in: {}", fields.keys().map(String::as_str).collect::<Vec<_>>().join(", ")));
                continue;
            }
            None => {
                outcome.problem(line, "`data` is missing");
                continue;
            }
        };
        let (created, updated) = match (timestamp(&row, "created_at"), timestamp(&row, "updated_at")) {
            (Ok(c), Ok(u)) => (c, u),
            (Err(reason), _) | (_, Err(reason)) => {
                outcome.problem(line, reason);
                continue;
            }
        };
        let mut one = tx.begin().await.map_err(crate::db::safe)?;
        let done = sqlx::query(sqlx::AssertSqlSafe(insert.clone())).bind(id).bind(owner).bind(data).bind(created).bind(updated).execute(&mut *one).await;
        match done {
            Ok(_) => one.commit().await.map_err(crate::db::safe)?,
            Err(error) => {
                outcome.problem(line, refusal(&error));
                one.rollback().await.map_err(crate::db::safe)?;
            }
        }
    }
    finish(tx, outcome, apply).await
}
