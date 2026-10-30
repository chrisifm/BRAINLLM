//! Owner-managed short links. Destinations must match an explicit application origin; identifiers are not authorization.
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub fn slug(title: &str, limit: usize) -> Result<String, String> {
    if title.len() > 4096 || !(8..=128).contains(&limit) {
        return Err("Slug input exceeds 4 KiB or limit is outside 8..128".into());
    }
    let value = slug::slugify(title);
    let value = value[..value.len().min(limit)].trim_end_matches('-');
    if value.is_empty() {
        return Err("Slug has no usable characters; provide a public descriptive title".into());
    }
    Ok(value.into())
}
fn code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
pub fn origin(raw: &str) -> Option<String> {
    let value = url::Url::parse(raw).ok()?;
    (matches!(value.scheme(), "http" | "https")
        && value.username().is_empty()
        && value.password().is_none())
    .then(|| value.origin().ascii_serialization())
}
pub fn target(raw: &str, origins: &[String]) -> Result<String, String> {
    if raw.len() > 2048 || raw.chars().any(char::is_control) {
        return Err("Short-link destination must be a bounded HTTP URL".into());
    }
    let url =
        url::Url::parse(raw).map_err(|_| "Short-link destination must be an absolute HTTP URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || !origins
            .iter()
            .any(|origin| origin == &url.origin().ascii_serialization())
    {
        return Err("Short-link destination must use an explicitly configured application origin without credentials".into());
    }
    if url.query_pairs().any(|(name, _)| {
        [
            "token",
            "password",
            "secret",
            "authorization",
            "session",
            "jwt",
            "api_key",
            "apikey",
        ]
        .iter()
        .any(|key| name.to_ascii_lowercase().contains(key))
    }) {
        return Err("Do not store credential-bearing destinations in public short links".into());
    }
    Ok(url.to_string())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub target: String,
    pub code: Option<String>,
    pub expires_in: Option<i64>,
}
#[derive(Serialize)]
pub struct Link {
    pub id: Uuid,
    pub code: String,
    pub path: String,
}
pub async fn create(
    pool: &PgPool,
    owner: Uuid,
    request: Create,
    origins: &[String],
) -> Result<Link, String> {
    let destination = target(&request.target, origins)?;
    if request.code.as_ref().is_some_and(|value| !code(value)) {
        return Err(
            "Short-link code must contain 1..80 lowercase ASCII letters, digits or hyphens".into(),
        );
    }
    let ttl = request.expires_in.unwrap_or(2592000);
    if !(60..=31536000).contains(&ttl) {
        return Err("Short-link expiry must be between one minute and one year".into());
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|_| "Cannot open short-link transaction")?;
    let exists: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM _brain_users WHERE id=$1 AND disabled_at IS NULL FOR UPDATE",
    )
    .bind(owner)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| "Cannot validate short-link owner")?;
    if exists.is_none() {
        return Err("Short-link owner is unavailable".into());
    }
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_short_links WHERE owner=$1")
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "Cannot inspect short-link capacity")?;
    if total >= 10000 {
        return Err(
            "Short-link owner capacity reached; remove expired links before creating more".into(),
        );
    }
    for _ in 0..5 {
        let id = Uuid::new_v4();
        let value = request
            .code
            .clone()
            .unwrap_or_else(|| id.simple().to_string()[..16].into());
        let inserted=sqlx::query("INSERT INTO _brain_short_links(id,owner,code,target,expires_at) VALUES($1,$2,$3,$4,now()+make_interval(secs=>$5)) ON CONFLICT(code) DO NOTHING")
            .bind(id).bind(owner).bind(&value).bind(&destination).bind(ttl as f64).execute(&mut *tx).await.map_err(|_|"Cannot create short link")?.rows_affected();
        if inserted == 1 {
            tx.commit().await.map_err(|_| "Cannot commit short link")?;
            return Ok(Link {
                id,
                code: value.clone(),
                path: format!("/s/{value}"),
            });
        }
        if request.code.is_some() {
            return Err("Short-link code already exists; choose a distinct public code".into());
        }
    }
    Err("Short-link collision budget exceeded; retry with a new code".into())
}
pub async fn resolve(
    pool: &PgPool,
    value: &str,
    origins: &[String],
) -> Result<Option<String>, String> {
    if !code(value) {
        return Ok(None);
    }
    let row = sqlx::query(
        "SELECT target FROM _brain_short_links WHERE code=$1 AND NOT disabled AND expires_at>now()",
    )
    .bind(value)
    .fetch_optional(pool)
    .await
    .map_err(|_| "Cannot resolve short link")?;
    row.map(|row| target(row.get::<String, _>("target").as_str(), origins))
        .transpose()
}
pub async fn revoke(pool: &PgPool, id: Uuid, owner: Uuid) -> Result<bool, String> {
    Ok(
        sqlx::query("UPDATE _brain_short_links SET disabled=true WHERE id=$1 AND owner=$2")
            .bind(id)
            .bind(owner)
            .execute(pool)
            .await
            .map_err(|_| "Cannot revoke short link")?
            .rows_affected()
            == 1,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_slugs_and_redirects_have_bounded_explicit_contracts() {
        assert_eq!(
            slug("A fast native project", 80).unwrap(),
            "a-fast-native-project"
        );
        assert_eq!(slug("An extremely detailed title", 8).unwrap(), "an-extre");
        assert!(slug("", 80).is_err());
        let origins = vec!["https://example.test".into()];
        assert_eq!(
            target("https://example.test/docs?q=public", &origins).unwrap(),
            "https://example.test/docs?q=public"
        );
        for url in [
            "//example.test/path",
            "https://other.test/",
            "javascript:alert(1)",
            "https://user:password@example.test/",
            "https://example.test/?access_token=private",
            "https://example.test/\r\nLocation: x",
        ] {
            assert!(target(url, &origins).is_err());
        }
    }
}
