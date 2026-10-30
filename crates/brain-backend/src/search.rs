//! Explicit searchable text fields only. Every candidate branch carries the same authorization scope.
use crate::Resource;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;
#[derive(Clone, Debug)]
pub struct Cursor {
    pub created_at: String,
    pub id: Uuid,
}
/// Owner None requires the caller to have already authorized a broad/public read policy.
/// A cursor uses the record's chronological timestamp/id, never offsets derived from another owner.
pub async fn list(
    pool: &PgPool,
    r: &Resource,
    owner: Option<Uuid>,
    text: &str,
    limit: i64,
    offset: i64,
    cursor: Option<&Cursor>,
) -> Result<Vec<Value>, sqlx::Error> {
    if r.validate().is_err()
        || text.len() > 256
        || text.split_whitespace().count() > 16
        || !(1..=100).contains(&limit)
        || !(0..=10000).contains(&offset)
        || cursor.is_some_and(|c| c.created_at.len() > 40 || !text.trim().is_empty())
    {
        return Err(sqlx::Error::Protocol(
            "Invalid bounded resource search request".into(),
        ));
    }
    let table = format!("_brain_res_{}", r.name);
    let mut tx = pool.begin().await?;
    sqlx::raw_sql("SET LOCAL statement_timeout='1s'; SET LOCAL lock_timeout='500ms'; SET LOCAL pg_trgm.word_similarity_threshold='0.45';").execute(&mut *tx).await?;
    let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT id,owner,data FROM ");
    q.push(&table);
    let owner_predicate = |q: &mut sqlx::QueryBuilder<sqlx::Postgres>| {
        if let Some(owner) = owner {
            q.push(" owner=").push_bind(owner);
        } else {
            q.push(" true");
        }
    };
    let text = text.trim();
    q.push(" WHERE ");
    owner_predicate(&mut q);
    if text.is_empty() {
        if let Some(c) = cursor {
            q.push(" AND (created_at,id)<(")
                .push_bind(c.created_at.clone())
                .push("::timestamptz,")
                .push_bind(c.id)
                .push(")");
        }
        q.push(" ORDER BY created_at DESC,id DESC");
    } else if r.search.is_empty() {
        q.push(" AND false ORDER BY created_at DESC,id DESC");
    } else if text.chars().count() < 3 {
        // Fixed prefix behavior, no unrestricted typo scan for one/two-character input.
        let normalized = normalize_prefix(text);
        q.push(" AND search_text LIKE public.unaccent('public.unaccent'::regdictionary,lower(")
            .push_bind(normalized)
            .push(")) ESCAPE '\\' ORDER BY created_at DESC,id DESC");
    } else {
        // Search branches are distinct and each bounds authorized candidates before ranking.
        // No global dictionary/count/suggestion vocabulary is constructed.
        let mut candidates = String::new();
        // The validated table identifier is the only interpolated data.
        candidates.push_str(&format!(
            " AND id IN (SELECT id FROM (SELECT id FROM {table} WHERE "
        ));
        q.push(candidates);
        owner_predicate(&mut q);
        q.push(" AND search_vector @@ websearch_to_tsquery('simple',public.unaccent('public.unaccent'::regdictionary,").push_bind(text.to_string()).push(")) ORDER BY ts_rank_cd(search_vector,websearch_to_tsquery('simple',public.unaccent('public.unaccent'::regdictionary,").push_bind(text.to_string()).push("))) DESC,id LIMIT 500) f UNION SELECT id FROM (SELECT id FROM ").push(&table).push(" WHERE ");
        owner_predicate(&mut q);
        // text <% indexed_column (word similarity), not long-document whole-string similarity.
        q.push(" AND public.unaccent('public.unaccent'::regdictionary,").push_bind(text.to_lowercase()).push(") OPERATOR(public.<%) search_text ORDER BY id LIMIT 500) t) ORDER BY ts_rank_cd(search_vector,websearch_to_tsquery('simple',public.unaccent('public.unaccent'::regdictionary,").push_bind(text.to_string()).push("))) DESC, public.word_similarity(public.unaccent('public.unaccent'::regdictionary,").push_bind(text.to_lowercase()).push("),search_text) DESC,id");
    }
    q.push(" LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let rows = q.build().fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(rows.into_iter().map(|row|json!({"id":row.get::<Uuid,_>("id"),"owner":row.get::<Uuid,_>("owner"),"data":row.get::<Value,_>("data")})).collect())
}
fn normalize_prefix(text: &str) -> String {
    let mut s = String::new();
    for c in text.to_lowercase().chars() {
        if matches!(c, '%' | '_' | '\\') {
            s.push('\\');
        }
        s.push(c);
    }
    s.push('%');
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_query_patterns_escape_sql_wildcards() {
        assert_eq!(normalize_prefix("A_%\\"), "a\\_\\%\\\\%");
    }
}
