//! Compact source-contract changes for agent feedback. Never treats a source save as a deployed schema update.
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
pub fn snapshot(root: &Path) -> Result<Value, String> {
    let value = crate::backend::inspect(root)?;
    Ok(json!({"resources":value["resources"],"jobs":value["jobs"],"roles":value["roles"],"webhooks":value["webhooks"]}))
}
fn revision(value: &Value) -> String {
    brain_pack::sha::sha256(value.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn names(value: &Value, key: &str) -> BTreeMap<String, Value> {
    value[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item["name"]
                .as_str()
                .map(|name| (name.to_owned(), item.clone()))
        })
        .collect()
}
pub fn diff(before: Option<&Value>, after: &Value) -> Value {
    let Some(before) = before else {
        return json!({"schemaVersion":1,"baseline":true,"sourceRevision":revision(after),"runtimeReloadRequired":false,"runtimeState":"unknown; source inventory only"});
    };
    if before == after {
        return Value::Null;
    }
    let mut changes = Vec::new();
    let mut breaking = false;
    let mut migration = false;
    let mut security = false;
    for kind in ["resources", "jobs"] {
        let old = names(before, kind);
        let current = names(after, kind);
        for (name, item) in &old {
            let Some(next) = current.get(name) else {
                breaking = true;
                changes.push(json!({"kind":kind,"name":name,"change":"removed","breaking":true}));
                continue;
            };
            let mut fields = Vec::new();
            let mut field_breaking = false;
            if let Some(prior) = item["fields"].as_object() {
                for (field, shape) in prior {
                    if next["fields"].get(field) != Some(shape) {
                        field_breaking = true;
                        fields.push(field.clone());
                    }
                }
            }
            if let Some(new) = next["fields"].as_object() {
                for (field, shape) in new {
                    if item["fields"].get(field).is_none() {
                        field_breaking |= shape["required"] == true;
                        fields.push(field.clone());
                    }
                }
            }
            let policy = item["policy"] != next["policy"];
            let search = item["search"] != next["search"];
            let hooks = item["hooks"] != next["hooks"];
            if !fields.is_empty() || policy || search || hooks {
                breaking |= field_breaking || policy;
                security |= policy;
                migration |= !fields.is_empty() || search;
                changes.push(json!({"kind":kind,"name":name,"change":"modified","fields":fields,"policyChanged":policy,"searchChanged":search,"hooksChanged":hooks,"breaking":field_breaking||policy}));
            }
        }
        for name in current.keys().filter(|name| !old.contains_key(*name)) {
            migration = true;
            changes.push(json!({"kind":kind,"name":name,"change":"added","breaking":false}));
        }
    }
    let roles = before["roles"] != after["roles"];
    let webhooks = before["webhooks"] != after["webhooks"];
    security |= roles || webhooks;
    breaking |= roles || webhooks;
    let truncated = changes.len() > 128;
    changes.truncate(128);
    json!({"schemaVersion":1,"beforeRevision":revision(before),"sourceRevision":revision(after),"breaking":breaking,"rolesChanged":roles,"webhooksChanged":webhooks,"securityReviewRequired":security,"migrationReviewRequired":migration,"runtimeReloadRequired":true,"runtimeState":"unchanged until an explicit backend restart; source contract only","changes":changes,"truncated":truncated,"fix":"Inspect brain backend schema --json, review data and policy compatibility, explicitly migrate when needed, then restart the selected development backend"})
}
#[cfg(test)]
mod tests {
    use super::*;
    fn resource() -> Value {
        json!({"resources":[{"name":"notes","fields":{"title":{"kind":"str","required":true}},"policy":{"read":["owner"]},"search":[],"hooks":{}}],"jobs":[],"roles":{"roles":["admin","candidate"],"signup_role":"candidate"}})
    }
    #[test]
    fn reports_real_breaking_fields_and_policy_changes_without_claiming_a_deployment() {
        let old = resource();
        let mut after = old.clone();
        after["resources"][0]["fields"]["title"]["kind"] = json!("num");
        let changed = diff(Some(&old), &after);
        assert_eq!(changed["breaking"], true);
        assert_eq!(changed["migrationReviewRequired"], true);
        assert_eq!(changed["changes"][0]["fields"], json!(["title"]));
        assert_eq!(changed["runtimeReloadRequired"], true);
        assert_ne!(changed["beforeRevision"], changed["sourceRevision"]);
        let mut policy = old.clone();
        policy["resources"][0]["policy"]["read"] = json!(["public"]);
        assert_eq!(diff(Some(&old), &policy)["securityReviewRequired"], true);
    }
    #[test]
    fn webhook_configuration_changes_require_security_review_without_exposing_values() {
        let old = resource();
        let mut after = old.clone();
        after["webhooks"] = json!({"configured":true,"inbound":["billing"]});
        let change = diff(Some(&old), &after);
        assert_eq!(change["webhooksChanged"], true);
        assert_eq!(change["securityReviewRequired"], true);
        assert_eq!(change["runtimeReloadRequired"], true);
        assert!(!change.to_string().contains("billing"));
    }
    #[test]
    fn optional_additions_are_not_breaking_but_removals_roles_and_required_fields_are() {
        let old = resource();
        assert!(diff(Some(&old), &old).is_null());
        let mut optional = old.clone();
        optional["resources"][0]["fields"]["note"] = json!({"kind":"str","required":false});
        assert_eq!(diff(Some(&old), &optional)["breaking"], false);
        optional["resources"][0]["fields"]["note"]["required"] = json!(true);
        assert_eq!(diff(Some(&old), &optional)["breaking"], true);
        optional["resources"] = json!([]);
        assert_eq!(diff(Some(&old), &optional)["breaking"], true);
        assert_eq!(
            diff(None, &old)["runtimeState"],
            "unknown; source inventory only"
        );
    }
}
