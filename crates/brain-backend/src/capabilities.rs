//! `brain backend capabilities`: what this version supports, read from the same code that enforces it, so an agent
//! never learns a filter or type from a doc that drifted. Operators come from the query parser; the tests below fail when
//! an operator no listed type offers appears, or when a listed type or option stops being accepted.
use crate::{domain, query, types};
use serde_json::{Value, json};

/// One example of every field type the parser accepts.
const FIELD_TYPES: &[(&str, &str)] = &[
    ("str", "str"),
    ("num", "num"),
    ("bool", "bool"),
    ("email", "email"),
    ("obj", "obj"),
    ("int", "int"),
    ("decimal(p,s)", "decimal(10,2)"),
    ("date", "date"),
    ("datetime", "datetime"),
    ("enum(a|b)", "enum(a|b)"),
    ("list<str>", "list<str>"),
    ("ref:<resource>", "ref:posts"),
];
const FIELD_OPTIONS: &[&str] = &["required", "default", "unique", "on_delete", "visible", "access", "slug", "pattern"];
const POLICY_OPERATIONS: &[&str] = &["read", "create", "update", "delete"];
const POLICY_GRANTS: &[&str] = &["public", "owner", "owner.<ref>", "<role>"];

pub fn inventory() -> Value {
    let field_types: Vec<Value> = FIELD_TYPES
        .iter()
        .map(|(name, example)| {
            let parsed = types::parse(example);
            let filters = parsed.as_ref().and_then(query::filter_operators).unwrap_or(&[]);
            let sortable = parsed.as_ref().is_some_and(query::sortable);
            json!({"type":name,"example":example,"filters":filters,"sortable":sortable})
        })
        .collect();
    json!({
        "schemaVersion": 1,
        "version": env!("CARGO_PKG_VERSION"),
        "fieldTypes": field_types,
        "fieldOptions": FIELD_OPTIONS,
        "filterOperators": query::OPS,
        "timestampFilters": query::timestamp_operators(),
        "policy": {
            "operations": POLICY_OPERATIONS,
            "grants": POLICY_GRANTS,
            "conditionedRead": "{ <grant>: { where: { <field>: <value> } } } on read only",
        },
        "limits": { "batchDefaultItems": domain::DEFAULT_BATCH_MAX },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_advertised_type_and_option_is_accepted_by_the_contract_parser() {
        for (name, example) in FIELD_TYPES {
            assert!(types::parse(example).is_some(), "{name}");
            let field: Result<crate::contract::Field, _> = serde_json::from_value(json!({"kind":example}));
            assert!(field.is_ok(), "{name}: {field:?}");
        }
        for option in FIELD_OPTIONS {
            let value = match *option {
                "required" | "unique" => json!(true),
                "visible" | "access" => json!(["public"]),
                _ => json!("x"),
            };
            let field: Result<crate::contract::Field, _> = serde_json::from_value(json!({"kind":"str",option.to_string():value}));
            let error = field.err().map(|e| e.to_string()).unwrap_or_default();
            assert!(!error.contains("unknown field"), "{option}: {error}");
        }
    }
    #[test]
    fn advertised_operators_are_real_and_every_real_one_is_advertised() {
        let inventory = inventory();
        let mut used: Vec<&str> = inventory["fieldTypes"].as_array().unwrap().iter().flat_map(|t| t["filters"].as_array().unwrap().iter().map(|o| o.as_str().unwrap())).collect();
        used.sort();
        used.dedup();
        let mut known: Vec<&str> = query::OPS.to_vec();
        known.sort();
        assert_eq!(used, known, "an operator no field type accepts, or one missing from the inventory");
    }
    #[test]
    fn the_filter_guide_table_matches_the_code() {
        let guide = include_str!("../../../docs/backend/filters.md");
        for (name, example) in FIELD_TYPES {
            let Some(ops) = types::parse(example).as_ref().and_then(query::filter_operators) else { continue };
            let row = format!("| {} |", ops.join(" "));
            let row_code = format!("`{}` |", ops.join(" "));
            assert!(guide.contains(&row) || guide.contains(&row_code), "{name}: docs/backend/filters.md lacks `{}`", ops.join(" "));
        }
    }
}
