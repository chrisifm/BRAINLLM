//! `/_brain/credits`: administrators change a balance; its user, an organization's managers and administrators read it.
use super::*;
use crate::credits::{self, Refusal};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    delta: i64,
    reason: String,
    idempotency_key: String,
}
pub(super) fn routes() -> Router<App> {
    Router::new().route("/_brain/credits/{subject}", get(read).post(change))
}
async fn change(State(app): State<App>, Path(subject): Path<Uuid>, h: HeaderMap, Json(input): Json<Change>) -> Result<Json<Value>> {
    let admin = app.admin(&h).await?;
    match credits::apply(&app.pool, subject, input.delta, &input.reason, &input.idempotency_key, Some(admin.id)).await {
        Ok(entry) => Ok(Json(json!({"id": entry.id, "delta": entry.delta, "balance": entry.balance, "replayed": entry.replayed}))),
        Err(Refusal::Insufficient) => Err(Error::new(StatusCode::CONFLICT, "CREDITS_INSUFFICIENT", "The balance is lower than this debit; nothing was charged; read the balance or grant credits first")),
        Err(Refusal::TooLarge) => Err(Error::validation(json!({"delta": "The balance would pass 9007199254740991"}))),
        Err(Refusal::KeyReused) => Err(Error::new(StatusCode::CONFLICT, "CREDITS_KEY_REUSED", "This idempotency_key recorded a different change; nothing was charged; use a new key for a new change")),
        Err(Refusal::UnknownSubject) => Err(Error::missing()),
        Err(Refusal::Invalid(message)) => Err(Error::validation(json!({"body": message}))),
        Err(Refusal::Database(error)) => Err(Error::database(error)),
    }
}
async fn read(State(app): State<App>, Path(subject): Path<Uuid>, h: HeaderMap, Query(pairs): Query<Vec<(String, String)>>) -> Result<Json<Value>> {
    let u = app.required(&h).await?;
    if subject != u.id && u.role != "admin" {
        organizations_http::manager(&app, subject, &u).await?;
    }
    let (mut before, mut limit) = (None, 50);
    for (key, value) in &pairs {
        let bad = || Error::validation(json!({key.as_str(): "Use before (an entry id) or limit from 1 to 100"}));
        match key.as_str() {
            "before" => before = Some(value.parse::<i64>().map_err(|_| bad())?),
            "limit" => {
                limit = value.parse::<i64>().map_err(|_| bad())?;
                if !(1..=credits::MAX_PAGE).contains(&limit) {
                    return Err(bad());
                }
            }
            _ => return Err(bad()),
        }
    }
    credits::statement(&app.pool, subject, before, limit).await.map(Json).map_err(|e| Error::database(e))
}
