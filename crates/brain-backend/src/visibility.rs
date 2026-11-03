//! Response projection: the declared contract decides which fields a reader receives (B-20).
//!
//! A field with `visible: [...]` is serialized only for a reader matching one of those grants: `public` (every reader
//! of the record), `owner` (the authenticated owner of that row, or a relational owner, see `ownership.rs`) or a role
//! name. There is no implicit administrator bypass: list `admin` explicitly. Restricted fields cannot be sorted,
//! searched or aggregated, so a reader cannot infer hidden values through the query surface. A field visible to named
//! roles only (no `public`, no `owner`) may be a declared filter: only those roles can use it, and any other reader
//! gets the answer of an undeclared filter (`filterable_by`, `query::parse`).
use crate::{Field, Resource, contract::FieldAccess};
use serde_json::Value;
use uuid::Uuid;

/// The authenticated reader of a response. `id`/`role` are None for anonymous public reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct Viewer<'a> {
    pub id: Option<Uuid>,
    pub role: Option<&'a str>,
}

pub fn validate_field(name: &str, field: &Field) -> Result<(), String> {
    match &field.access {
        Some(FieldAccess::Mode(mode)) if !matches!(mode.as_str(), "private" | "readonly" | "writeonly") => {
            return Err(format!("field `{name}` access `{mode}` is invalid; fix: use private, readonly, writeonly or {{ write: ['admin'] }}"));
        }
        Some(FieldAccess::Roles(roles)) if roles.write.is_empty() || roles.write.len() > 16 || roles.write.iter().any(|role| !crate::auth_flows::role_name(role)) => {
            return Err(format!("field `{name}` access write needs 1..16 configured role names; fix: {{ write: ['admin'] }}, or readonly when only the server sets it"));
        }
        _ => {}
    }
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
    !hidden(field)
        && field
        .visible
        .as_ref()
        .is_none_or(|grants| grants.iter().any(|g| g == "public"))
}

/// Readable by named roles only (`visible: ['admin']`): the same for every row, so the query surface can be opened
/// to exactly those roles. `owner` grants differ per row and `private`/`writeonly` fields nobody reads: never.
pub fn role_gated(field: &Field) -> bool {
    !hidden(field) && field.visible.as_ref().is_some_and(|grants| !grants.is_empty() && grants.iter().all(|g| g != "public" && g != "owner"))
}

/// Whether a reader with `role` may filter by a field: every field but a role-gated one, which only its roles (and a
/// system key, which reads every readable field) may use. Anyone else gets the answer of an undeclared filter.
pub fn filterable_by(field: &Field, role: Option<&str>) -> bool {
    !role_gated(field) || crate::api_keys::system_active() || role.is_some_and(|role| field.visible.as_ref().is_some_and(|g| g.iter().any(|g| g == role)))
}

fn sees(field: &Field, viewer: Viewer<'_>, owns: bool) -> bool {
    match &field.visible {
        None => true,
        Some(grants) => grants.iter().any(|g| {
            g == "public" || g == "owner" && owns && viewer.id.is_some() || viewer.role == Some(g.as_str())
        }),
    }
}

/// Never serialized to any reader (`private` and `writeonly`).
pub fn hidden(field: &Field) -> bool {
    matches!(field.access_mode(), Some("private" | "writeonly"))
}
/// Clients may not send it (`private` and `readonly`): only defaults and before-write hooks set it.
pub fn server_set(field: &Field) -> bool {
    matches!(field.access_mode(), Some("private" | "readonly"))
}
/// Whether a client with `role` may send the field.
fn writable_by(field: &Field, role: Option<&str>) -> bool {
    match &field.access {
        Some(FieldAccess::Roles(roles)) => role.is_some_and(|role| roles.write.iter().any(|allowed| allowed == role)),
        _ => !server_set(field),
    }
}

/// Client input naming server-set fields is rejected before any hook runs.
pub fn reject_server_fields(r: &Resource, input: &Value, role: Option<&str>) -> Result<(), std::collections::BTreeMap<String, String>> {
    if crate::api_keys::system_active() {
        return Ok(());
    }
    let errors: std::collections::BTreeMap<String, String> = r
        .fields
        .iter()
        .filter(|(name, field)| !writable_by(field, role) && input.get(name.as_str()).is_some())
        .map(|(name, field)| {
            let message = match &field.access {
                Some(FieldAccess::Roles(roles)) => format!("Only {} may set this field; remove it from the request", roles.write.join(", ")),
                _ => format!("Field is {}; it is set by the server, remove it from the request", field.access_mode().unwrap_or_default()),
            };
            (name.clone(), message)
        })
        .collect();
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

/// A full replacement (PUT) keeps the stored value of fields the client cannot send or cannot read back.
pub fn keep_stored(r: &Resource, input: &mut Value, stored: &Value) {
    let Some(input) = input.as_object_mut() else { return };
    for (name, field) in &r.fields {
        if field.access.is_some()
            && !input.contains_key(name)
            && let Some(value) = stored.get(name).filter(|v| !v.is_null())
        {
            input.insert(name.clone(), value.clone());
        }
    }
}

/// Removes fields the viewer may not see from one rendered record (`{id, owner, data, ...}`).
/// `relational_owner` is true when the viewer owns the record through a declared relation.
/// The `owner` id is shown only to the owner, relational owners and roles named in the read policy, unless the
/// resource declares `expose_owner: true` in `query.jsbrain` (OWASP API3: no undeclared data on public reads).
pub fn project(r: &Resource, row: &mut Value, viewer: Viewer<'_>, relational_owner: bool) {
    // A system key reads private fields and the owner; `writeonly` stays unreadable by every identity.
    if crate::api_keys::system_active() {
        if let Some(data) = row.get_mut("data").and_then(Value::as_object_mut) {
            for (name, field) in &r.fields {
                if field.access_mode() == Some("writeonly") {
                    data.remove(name);
                }
            }
        }
        return;
    }
    let owns = relational_owner
        || viewer
            .id
            .is_some_and(|id| row["owner"].as_str() == Some(id.hyphenated().to_string().as_str()));
    if let Some(data) = row.get_mut("data").and_then(Value::as_object_mut) {
        for (name, field) in &r.fields {
            if hidden(field) || !sees(field, viewer, owns) {
                data.remove(name);
            }
        }
    }
    let named_role = viewer
        .role
        .is_some_and(|role| r.policy.read.iter().any(|g| g == role && g != "public" && g != "owner"));
    if !owns && !named_role && !r.query.expose_owner
        && let Some(row) = row.as_object_mut()
    {
        row.remove("owner");
    }
}

/// OpenAPI hint for a restricted field.
pub fn schema_hint(field: &Field) -> Option<Value> {
    field.visible.as_ref().map(|grants| serde_json::json!(grants))
}
