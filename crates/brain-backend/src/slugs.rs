//! Slug history for permanent redirects. A resource marks one unique text field `slug: true`; when a record's slug
//! changes, the previous value keeps resolving to the record so a page can answer `301` to the current slug.
use crate::{Field, Resource, ownership::Scope};
use std::collections::BTreeMap;

pub fn field(r: &Resource) -> Option<&str> {
    r.fields.iter().find(|(_, f)| f.slug).map(|(name, _)| name.as_str())
}

pub fn validate(r: &Resource) -> Result<(), String> {
    let slugs: Vec<(&String, &Field)> = r.fields.iter().filter(|(_, f)| f.slug).collect();
    if slugs.len() > 1 {
        return Err(format!("{}: only one field can be the slug; fix: keep slug: true on one field", r.name));
    }
    if let Some((name, f)) = slugs.first()
        && (f.kind != "str" || !f.unique || f.visible.is_some() || f.access.as_deref().is_some_and(|a| a != "readonly"))
    {
        return Err(format!(
            "{}: slug field `{name}` must be a public unique str; fix: {{ type: 'str!', unique: true, slug: true }}",
            r.name
        ));
    }
    Ok(())
}

/// Installs (or removes) the tracking trigger. Runs in the migration transaction after tables exist.
pub(crate) async fn install(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, r: &Resource) -> Result<(), String> {
    let table = format!("_brain_res_{}", r.name);
    let ddl = match field(r) {
        Some(f) => format!(
            "DROP TRIGGER IF EXISTS _brain_slugs ON {table}; CREATE TRIGGER _brain_slugs AFTER INSERT OR UPDATE OF data ON {table} FOR EACH ROW EXECUTE FUNCTION public._brain_slug_track('{f}');"
        ),
        None => format!("DROP TRIGGER IF EXISTS _brain_slugs ON {table};"),
    };
    // Audited SQL: table and field identifiers pass Resource::validate; no request value is interpolated.
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl)).execute(&mut **tx).await.map_err(crate::db::safe)?;
    Ok(())
}

/// The current slug of the record an old slug pointed at, inside the reader's scope; None when the slug is live,
/// unknown, or its record is gone or not readable.
pub async fn redirect(
    pool: &sqlx::PgPool,
    resources: &BTreeMap<String, Resource>,
    r: &Resource,
    scope: &Scope,
    slug: &str,
) -> Result<Option<String>, sqlx::Error> {
    let Some(f) = field(r) else { return Ok(None) };
    let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT t.data->>'");
    // Identifiers are validated contract names; the slug is bound.
    q.push(f).push("' FROM public._brain_slug_history h JOIN _brain_res_").push(&r.name);
    q.push(" t ON t.id=h.record_id WHERE h.resource=").push_bind(&r.name).push(" AND h.slug=").push_bind(slug);
    q.push(" AND t.id IN (SELECT id FROM _brain_res_").push(&r.name).push(" WHERE ");
    scope.push(&mut q, r, resources);
    q.push(")");
    q.build_query_scalar::<Option<String>>().fetch_optional(pool).await.map(Option::flatten)
}
