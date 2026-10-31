//! Relations between resources (B-01): `ref:<resource>` fields become real foreign keys.
//!
//! The value stays in the JSONB record (`data.<field>` = canonical UUID string) and is mirrored by the
//! `_brain_ref_sync` trigger into a typed column `ref_<field> uuid` that carries a Postgres FOREIGN KEY with an
//! explicit `ON DELETE` action and a btree index. The trigger covers every write path (CRUD, hooks, batch writes),
//! so the column can never drift from the record. Only validated identifiers enter SQL text; values are bound.
use crate::{Field, Resource, types::Ty};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

pub const ON_DELETE: &[&str] = &["restrict", "cascade", "set_null"];

/// Postgres column holding the mirrored reference.
pub fn column(field: &str) -> String {
    debug_assert!(crate::contract::identifier(field));
    format!("ref_{field}")
}
fn table(name: &str) -> String {
    assert!(crate::contract::identifier(name));
    format!("_brain_res_{name}")
}

/// One declared reference of a resource.
#[derive(Clone, Debug, PartialEq)]
pub struct Ref<'a> {
    pub field: &'a str,
    pub target: String,
    pub action: &'a str,
    pub required: bool,
}
pub fn refs(r: &Resource) -> Vec<Ref<'_>> {
    r.fields
        .iter()
        .filter_map(|(name, f)| match f.ty() {
            Some(Ty::Ref(target)) => Some(Ref {
                field: name,
                target,
                action: f.on_delete.as_deref().unwrap_or("restrict"),
                required: f.required,
            }),
            _ => None,
        })
        .collect()
}
pub fn target(r: &Resource, field: &str) -> Option<String> {
    match r.fields.get(field)?.ty()? {
        Ty::Ref(target) => Some(target),
        _ => None,
    }
}

pub fn validate_field(name: &str, field: &Field, ty: &Ty) -> Result<(), String> {
    match (&field.on_delete, ty) {
        (None, _) => Ok(()),
        (Some(_), ty) if !matches!(ty, Ty::Ref(_)) => Err(format!(
            "field `{name}`: on_delete applies only to ref:<resource> fields"
        )),
        (Some(action), _) if !ON_DELETE.contains(&action.as_str()) => Err(format!(
            "field `{name}`: on_delete must be restrict, cascade or set_null"
        )),
        (Some(action), _) if action == "set_null" && field.required => Err(format!(
            "field `{name}`: a required reference cannot use set_null; fix: use restrict or cascade, or make the field optional"
        )),
        (Some(_), _) if field.default.is_some() => Err(format!(
            "field `{name}`: a reference cannot have a default"
        )),
        _ => Ok(()),
    }
}

/// Every reference target must be a declared resource of the same project.
pub fn validate_graph(resources: &[Resource]) -> Result<(), String> {
    let names: std::collections::BTreeSet<&str> = resources.iter().map(|r| r.name.as_str()).collect();
    for r in resources {
        for reference in refs(r) {
            if !names.contains(reference.target.as_str()) {
                return Err(format!(
                    "{}: field `{}` references unknown resource `{}`; fix: create server/resources/{}/ or correct ref:<resource>",
                    r.name, reference.field, reference.target, reference.target
                ));
            }
        }
        crate::query::validate_graph(r, resources)?;
    }
    Ok(())
}

pub fn fk_name(r: &Resource, reference: &Ref<'_>) -> String {
    r.object_name("_brain_fk_", &format!("{}>{}:{}", reference.field, reference.target, reference.action))
}
pub fn index_name(r: &Resource, field: &str) -> String {
    r.object_name("_brain_ix_", &format!("ref:{field}"))
}
fn action_sql(action: &str) -> (&'static str, &'static str) {
    match action {
        "cascade" => ("CASCADE", "c"),
        "set_null" => ("SET NULL", "n"),
        _ => ("RESTRICT", "r"),
    }
}
fn trigger_args(r: &Resource) -> Vec<u8> {
    refs(r)
        .iter()
        .flat_map(|x| x.field.as_bytes().iter().copied().chain(std::iter::once(0)))
        .collect()
}

/// Installs columns, the sync trigger, foreign keys and indexes. Runs after every resource table exists, so
/// self references and cycles work. Existing rows are only backfilled when the column is new and empty.
pub(crate) async fn install(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    r: &Resource,
) -> Result<(), String> {
    let refs = refs(r);
    if refs.is_empty() {
        return Ok(());
    }
    let table = table(&r.name);
    let args = refs.iter().map(|x| format!("'{}'", x.field)).collect::<Vec<_>>().join(",");
    let mut ddl = String::new();
    for reference in &refs {
        ddl.push_str(&format!("ALTER TABLE {table} ADD COLUMN IF NOT EXISTS {} uuid;", column(reference.field)));
    }
    ddl.push_str(&format!("DROP TRIGGER IF EXISTS _brain_refs ON {table}; CREATE TRIGGER _brain_refs BEFORE INSERT OR UPDATE ON {table} FOR EACH ROW EXECUTE FUNCTION public._brain_ref_sync({args});"));
    // Audited SQL: table/field identifiers pass Resource::validate; no request value is interpolated.
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl)).execute(&mut **tx).await.map_err(crate::db::safe)?;
    for reference in &refs {
        let col = column(reference.field);
        let field = reference.field;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET data=data WHERE {col} IS NULL AND data ? '{field}'"
        )))
        .execute(&mut **tx)
        .await
        .map_err(|error| {
            crate::db::safe(error);
            format!("{}: existing `{field}` values are not record ids; review the data before migrating; nothing was changed", r.name)
        })?;
        let name = fk_name(r, reference);
        if !fk_exists(tx, &table, &name, &col, &reference.target, reference.action).await? {
            let (action, _) = action_sql(reference.action);
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "ALTER TABLE {table} ADD CONSTRAINT {name} FOREIGN KEY ({col}) REFERENCES {}(id) ON DELETE {action}",
                self::table(&reference.target)
            )))
            .execute(&mut **tx)
            .await
            .map_err(|error| {
                crate::db::safe(error);
                format!("{}: existing `{field}` values reference missing {} records; review the data before migrating; nothing was changed", r.name, reference.target)
            })?;
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE INDEX IF NOT EXISTS {} ON {table}({col})",
            index_name(r, field)
        )))
        .execute(&mut **tx)
        .await
        .map_err(crate::db::safe)?;
    }
    Ok(())
}

async fn fk_exists(
    conn: &mut sqlx::PgConnection,
    table: &str,
    name: &str,
    column: &str,
    target: &str,
    action: &str,
) -> Result<bool, String> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_constraint c JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=c.conkey[1] WHERE c.conrelid=to_regclass($1) AND c.conname=$2 AND c.contype='f' AND cardinality(c.conkey)=1 AND a.attname=$3 AND c.confrelid=to_regclass($4) AND c.confdeltype::text=$5 AND c.convalidated)")
        .bind(format!("public.{table}"))
        .bind(name)
        .bind(column)
        .bind(format!("public.{}", self::table(target)))
        .bind(action_sql(action).1)
        .fetch_one(conn)
        .await
        .map_err(crate::db::safe)
}

/// Serving refuses a database whose reference column, trigger, key or index is missing or different.
pub(crate) async fn verify(conn: &mut sqlx::PgConnection, r: &Resource) -> Result<(), String> {
    let refs = refs(r);
    if refs.is_empty() {
        return Ok(());
    }
    let table = table(&r.name);
    let drift = |what: String| format!("BDB_SCHEMA_DRIFT: {table} {what}; review schema and run the explicit backend migration; no rows were removed");
    let trigger: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_trigger WHERE tgrelid=to_regclass($1) AND tgname='_brain_refs' AND tgenabled='O' AND tgfoid='public._brain_ref_sync()'::regprocedure AND tgargs=$2)")
        .bind(format!("public.{table}"))
        .bind(trigger_args(r))
        .fetch_one(&mut *conn)
        .await
        .map_err(crate::db::safe)?;
    if !trigger {
        return Err(drift("reference sync trigger".into()));
    }
    for reference in &refs {
        let col = column(reference.field);
        if !fk_exists(conn, &table, &fk_name(r, reference), &col, &reference.target, reference.action).await? {
            return Err(drift(format!("foreign key for `{}`", reference.field)));
        }
        let indexed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid WHERE i.indrelid=to_regclass($1) AND c.relname=$2 AND i.indisvalid)")
            .bind(format!("public.{table}"))
            .bind(index_name(r, reference.field))
            .fetch_one(&mut *conn)
            .await
            .map_err(crate::db::safe)?;
        if !indexed {
            return Err(drift(format!("reference index for `{}`", reference.field)));
        }
    }
    Ok(())
}

/// A write referencing a missing record: the declared ref field that failed, from the foreign key name.
pub fn missing_reference(r: &Resource, error: &sqlx::Error) -> Option<(String, String)> {
    let db = error.as_database_error()?;
    if db.code()?.as_ref() != "23503" {
        return None;
    }
    let name = db.constraint()?;
    refs(r)
        .into_iter()
        .find(|reference| fk_name(r, reference) == name)
        .map(|reference| (reference.field.to_string(), reference.target))
}
/// A delete refused because other records still reference this one: `on_delete: restrict` raises 23001,
/// a key reached through a cascade raises 23503.
pub fn referenced(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .is_some_and(|code| code == "23001" || code == "23503")
}

/// Why related records could not be attached.
#[derive(Debug, PartialEq)]
pub enum IncludeError {
    Unknown(String),
    Denied(String),
    Database,
}

/// Attaches `included.<field>` (the referenced record or null) to each row. One query per included field,
/// bounded by the page size; the target's read policy applies (owner-only readers see only their own rows,
/// other rows render as null) and so does the target's field visibility.
pub async fn include(
    pool: &sqlx::PgPool,
    resources: &BTreeMap<String, Resource>,
    r: &Resource,
    names: &[String],
    rows: &mut [Value],
    viewer: crate::visibility::Viewer<'_>,
) -> Result<(), IncludeError> {
    for name in names {
        let Some(target_name) = target(r, name).filter(|_| r.query.include.contains(name)) else {
            return Err(IncludeError::Unknown(name.clone()));
        };
        let target = resources.get(&target_name).ok_or_else(|| IncludeError::Unknown(name.clone()))?;
        let scope = crate::ownership::read_scope(target, resources, viewer).ok_or_else(|| IncludeError::Denied(name.clone()))?;
        let ids: Vec<Uuid> = rows
            .iter()
            .filter_map(|row| row["data"][name.as_str()].as_str().and_then(|s| Uuid::parse_str(s).ok()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut found: BTreeMap<String, Value> = BTreeMap::new();
        if !ids.is_empty() {
            let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new(concat!("SELECT ", crate::record_columns!(), " FROM "));
            q.push(table(&target.name)).push(" WHERE id=ANY(").push_bind(ids).push(") AND ");
            scope.push(&mut q, target, resources);
            let fetched = q.build().fetch_all(pool).await.map_err(|error| {
                crate::db::safe(error);
                IncludeError::Database
            })?;
            for row in fetched {
                let mut record = crate::db::record(row);
                crate::visibility::project(target, &mut record, viewer, false);
                found.insert(record["id"].as_str().unwrap_or_default().to_string(), record);
            }
        }
        for row in rows.iter_mut() {
            let related = row["data"][name.as_str()]
                .as_str()
                .and_then(|id| found.get(id).cloned())
                .unwrap_or(Value::Null);
            if !row["included"].is_object() {
                row["included"] = serde_json::json!({});
            }
            row["included"][name.as_str()] = related;
        }
    }
    Ok(())
}
