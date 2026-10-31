//! Reviewed approval of a changed resource contract (`brain backend approve-contract`). A registered contract never
//! changes implicitly: migrate refuses it. This compares what is recorded with what the code declares, says which rows
//! each change touches, and records the new contract only on request. Data is never rewritten or removed.
use crate::{Resource, db};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

const CONSTRAINTS: &str = "$constraints";

/// What one field or setting changes. `safe` changes cannot invalidate a row already stored.
fn changes(recorded: &Value, declared: &Value, recorded_search: &Value, declared_search: &Value) -> Vec<Value> {
    let empty = serde_json::Map::new();
    let old = recorded.as_object().unwrap_or(&empty);
    let new = declared.as_object().unwrap_or(&empty);
    let mut out = Vec::new();
    let mut note = |field: &str, change: &str, safe: bool| out.push(json!({"field":field,"change":change,"safe":safe}));
    for (field, value) in new {
        match old.get(field) {
            None if field == CONSTRAINTS => note(field, "constraints-added", false),
            None if value["required"] == json!(true) => note(field, "added-required", false),
            None if value["unique"] == json!(true) => note(field, "added-unique", false),
            None => note(field, "added-optional", true),
            Some(before) if before == value => {}
            Some(_) if field == CONSTRAINTS => note(field, "constraints-changed", false),
            Some(before) => {
                let mut relaxed = before.clone();
                relaxed["required"] = json!(false);
                if before["required"] == json!(true) && value["required"] == json!(false) && relaxed == *value {
                    note(field, "required-relaxed", true);
                } else {
                    note(field, "changed", false);
                }
            }
        }
    }
    for field in old.keys().filter(|field| !new.contains_key(*field)) {
        note(field, if field == CONSTRAINTS { "constraints-removed" } else { "removed" }, false);
    }
    if recorded_search != declared_search {
        note("$search", "search-changed", false);
    }
    out
}

/// The difference between every registered resource and its declaration. Reads only; a resource not registered yet is
/// the normal work of `brain backend migrate` and does not appear.
pub async fn review(pool: &PgPool, resources: &[Resource]) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for r in resources {
        r.validate()?;
        let row = sqlx::query("SELECT fields,search_fields FROM _brain_resource_schemas WHERE name=$1")
            .bind(&r.name)
            .fetch_optional(pool)
            .await
            .map_err(|_| "Cannot read the registered contracts; fix: run `brain backend migrate` first".to_string())?;
        let Some(row) = row else { continue };
        let mut found = changes(&row.get::<Value, _>("fields"), &r.recorded_fields(), &row.get::<Value, _>("search_fields"), &json!(r.search));
        if found.is_empty() {
            continue;
        }
        let table = format!("_brain_res_{}", r.name);
        // Audited SQL: the table name comes from a validated identifier; the field is bound.
        let rows: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}"))).fetch_one(pool).await.map_err(db::safe)?;
        for change in &mut found {
            let field = change["field"].as_str().unwrap_or_default().to_string();
            if !field.starts_with('$') {
                let with: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE data ? $1"))).bind(&field).fetch_one(pool).await.map_err(db::safe)?;
                change["rowsWithValue"] = json!(with);
            }
        }
        let safe = found.iter().all(|change| change["safe"] == json!(true));
        out.push(json!({"resource":r.name,"rows":rows,"safe":safe,"changes":found}));
    }
    Ok(out)
}

/// Resources the reviewed change may record, or the ones that still need `--accept`. Nothing is written here.
pub fn approved(reviewed: &[Value], accepted: &[String]) -> Result<Vec<String>, Vec<String>> {
    let blocked: Vec<String> = reviewed
        .iter()
        .filter(|item| item["safe"] != json!(true))
        .filter_map(|item| item["resource"].as_str())
        .filter(|name| !accepted.iter().any(|accept| accept == name))
        .map(str::to_string)
        .collect();
    if blocked.is_empty() {
        Ok(reviewed.iter().filter_map(|item| item["resource"].as_str().map(str::to_string)).collect())
    } else {
        Err(blocked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_changes_that_cannot_invalidate_a_stored_row_count_as_safe() {
        let of = |old: Value, new: Value| -> Vec<(String, bool)> {
            changes(&old, &new, &json!([]), &json!([])).iter().map(|c| (c["change"].as_str().unwrap().to_string(), c["safe"].as_bool().unwrap())).collect()
        };
        let a = json!({"kind":"str","required":true});
        let optional = json!({"kind":"str","required":false});
        assert_eq!(of(json!({"a":a}), json!({"a":a,"b":optional})), [("added-optional".to_string(), true)]);
        assert_eq!(of(json!({"a":a}), json!({"a":optional})), [("required-relaxed".to_string(), true)]);
        assert_eq!(of(json!({"a":a}), json!({"a":a,"b":a})), [("added-required".to_string(), false)]);
        assert_eq!(of(json!({"a":a}), json!({"a":{"kind":"int","required":true}})), [("changed".to_string(), false)]);
        assert_eq!(of(json!({"a":a,"b":a}), json!({"a":a})), [("removed".to_string(), false)]);
        assert_eq!(of(json!({"a":a}), json!({"a":a,"b":{"kind":"str","required":false,"unique":true}})), [("added-unique".to_string(), false)]);
        assert!(of(json!({"a":a}), json!({"a":a})).is_empty());
        assert_eq!(changes(&json!({}), &json!({}), &json!(["a"]), &json!([]))[0]["safe"], json!(false));
    }
    #[test]
    fn a_change_needing_review_blocks_the_whole_apply_until_it_is_named() {
        let reviewed = [json!({"resource":"a","safe":true}), json!({"resource":"b","safe":false})];
        assert_eq!(approved(&reviewed, &[]).unwrap_err(), ["b"]);
        assert_eq!(approved(&reviewed, &["b".into()]).unwrap(), ["a", "b"]);
    }
}
