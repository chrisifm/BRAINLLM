//! Response projection: the declared contract decides which fields a reader receives (B-20).
//!
//! A field with `visible: [...]` is serialized only for a reader matching one of those grants: `public` (every reader
//! of the record), `owner` (the authenticated owner of that row, or a relational owner, see `ownership.rs`) or a role
//! name. There is no implicit administrator bypass: list `admin` explicitly. Restricted fields cannot be filtered,
//! sorted, searched or aggregated, so a reader cannot infer hidden values through the query surface.
use crate::{Field, Resource};
use serde_json::Value;
use uuid::Uuid;

/// The authenticated reader of a response. `id`/`role` are None for anonymous public reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct Viewer<'a> {
    pub id: Option<Uuid>,
    pub role: Option<&'a str>,
}

pub fn validate_field(name: &str, field: &Field) -> Result<(), String> {
    let Some(grants) = &field.visible else {
        return Ok(());
    };
    let unique: std::collections::BTreeSet<_> = grants.iter().collect();
    if grants.is_empty() || grants.len() > 16 || unique.len() != grants.len() {
        return Err(format!(
            "field `{name}` visible needs 1..16 distinct grants; fix: list public, owner or role names, e.g. ['owner', 'admin']"
        ));
    }
    for grant in grants {
        if !(matches!(grant.as_str(), "public" | "owner") || crate::auth_flows::role_name(grant)) {
            return Err(format!(
                "field `{name}` visible grant `{grant}` is invalid; fix: use public, owner or a configured role name"
            ));
        }
    }
    Ok(())
}

/// True when the field is readable by everyone who can read the record.
pub fn unrestricted(field: &Field) -> bool {
    field
        .visible
        .as_ref()
        .is_none_or(|grants| grants.iter().any(|g| g == "public"))
}

fn sees(field: &Field, viewer: Viewer<'_>, owns: bool) -> bool {
    match &field.visible {
        None => true,
        Some(grants) => grants.iter().any(|g| {
            g == "public" || g == "owner" && owns && viewer.id.is_some() || viewer.role == Some(g.as_str())
        }),
    }
}

/// Removes fields the viewer may not see from one rendered record (`{id, owner, data, ...}`).
/// `relational_owner` is true when the viewer owns the record through a declared relation.
pub fn project(r: &Resource, row: &mut Value, viewer: Viewer<'_>, relational_owner: bool) {
    if r.fields.values().all(|f| f.visible.is_none()) {
        return;
    }
    let owns = relational_owner
        || viewer
            .id
            .is_some_and(|id| row["owner"].as_str() == Some(id.hyphenated().to_string().as_str()));
    if let Some(data) = row.get_mut("data").and_then(Value::as_object_mut) {
        for (name, field) in &r.fields {
            if !sees(field, viewer, owns) {
                data.remove(name);
            }
        }
    }
}

/// OpenAPI hint for a restricted field.
pub fn schema_hint(field: &Field) -> Option<Value> {
    field.visible.as_ref().map(|grants| serde_json::json!(grants))
}
