//! Declarative domain writes: partial updates, atomic actions, state transitions, batches, upserts and
//! multi-resource operations. Every write runs inside one Postgres transaction; no application code is evaluated.
use crate::Resource;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Merge a partial body into the stored record data. Only sent keys change; `null` removes an optional field
/// (or restores its default during revalidation). Unknown keys are rejected before any application code runs.
pub fn merge_patch(
    r: &Resource,
    existing: &Value,
    patch: &Value,
) -> Result<Value, BTreeMap<String, String>> {
    let Some(patch) = patch.as_object() else {
        return Err(BTreeMap::from([("body".into(), "Expected an object".into())]));
    };
    if patch.is_empty() {
        return Err(BTreeMap::from([(
            "body".into(),
            "Send at least one declared field to change".into(),
        )]));
    }
    let unknown: BTreeMap<String, String> = patch
        .keys()
        .filter(|key| !r.fields.contains_key(*key))
        .map(|key| (key.clone(), "Unknown field".into()))
        .collect();
    if !unknown.is_empty() {
        return Err(unknown);
    }
    let mut merged: Map<String, Value> = existing.as_object().cloned().unwrap_or_default();
    for (key, value) in patch {
        if value.is_null() {
            merged.remove(key);
        } else {
            merged.insert(key.clone(), value.clone());
        }
    }
    Ok(Value::Object(merged))
}

/// JSON Schema of a PATCH body: every declared field is optional, `null` clears an optional field.
pub fn patch_schema(r: &Resource) -> Value {
    let mut schema = r.schema();
    if let Some(props) = schema["properties"].as_object_mut() {
        for (name, prop) in props.iter_mut() {
            if r.fields.get(name).is_some_and(|f| !f.required || f.default.is_some()) {
                *prop = serde_json::json!({"anyOf":[prop.clone(),{"type":"null"}]});
            }
        }
    }
    schema["required"] = serde_json::json!([]);
    schema["minProperties"] = serde_json::json!(1);
    schema
}
/// OpenAPI operation for PATCH, derived from the resource's update operation.
pub fn patch_operation(r: &Resource, mut update: Value) -> Value {
    update["operationId"] = Value::String(format!("{}_patch", r.name));
    update["description"] = Value::String("Partial update: only sent fields change; null clears an optional field. The merged record is revalidated against the full contract and written with the same conflict check as PUT.".into());
    update["requestBody"] = serde_json::json!({"required":true,"content":{"application/json":{"schema":patch_schema(r)}}});
    update
}

// ---------------------------------------------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------------------------------------------
use crate::{auth::User, contract::Field, types::Ty};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

/// Default and hard maximum number of items in one batch request.
pub const DEFAULT_BATCH_MAX: usize = 100;
pub const BATCH_LIMIT: usize = 1000;
/// Maximum steps in one declared operation.
pub const MAX_STEPS: usize = 32;

/// Domain contracts of an application. Absent files produce an empty value; behavior is then identical to before.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Domain {
    /// Per-resource `actions.jsbrain`, `transitions.jsbrain` and `batch.jsbrain`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resources: BTreeMap<String, ResourceDomain>,
    /// `server/operations/<name>.jsbrain`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operations: BTreeMap<String, Operation>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceDomain {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub actions: BTreeMap<String, Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transitions: Option<Transitions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<Batch>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub max: usize,
}
/// Atomic single-record action: guards (`when`), assignments (`set`) and numeric deltas (`increment`), evaluated
/// on the locked current row inside one transaction.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub when: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub increment: BTreeMap<String, Value>,
    /// Subtract the resolved amount (`$input.quantity` stays positive in the request).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub decrement: BTreeMap<String, Value>,
}
/// State machine on one `enum` field. The field changes only through these transitions.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Transitions {
    pub field: String,
    pub initial: Vec<String>,
    pub transitions: BTreeMap<String, Transition>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub from: Vec<String>,
    pub to: String,
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub when: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub increment: BTreeMap<String, Value>,
    /// Subtract the resolved amount (`$input.quantity` stays positive in the request).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub decrement: BTreeMap<String, Value>,
}
impl Transition {
    fn action(&self, field: &str) -> Action {
        let mut action = Action {
            roles: self.roles.clone(),
            input: self.input.clone(),
            when: self.when.clone(),
            set: self.set.clone(),
            increment: self.increment.clone(),
            decrement: self.decrement.clone(),
        };
        action.when.insert(field.into(), json!({"in": self.from}));
        action.set.insert(field.into(), Value::String(self.to.clone()));
        action
    }
}
/// Server-side domain operation: declared steps that run in ONE transaction and roll back together.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input: BTreeMap<String, String>,
    pub steps: Vec<Step>,
    /// Step names returned in `result`; empty returns every step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result: Vec<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(rename = "as")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upsert: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub data: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owned: bool,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub input: Map<String, Value>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Read,
    Create,
    Update,
    Upsert,
    Delete,
    Action,
    Transition,
}
impl Step {
    fn kind(&self) -> Result<(Kind, &str), String> {
        let forms = [
            (Kind::Read, &self.read),
            (Kind::Create, &self.create),
            (Kind::Update, &self.update),
            (Kind::Upsert, &self.upsert),
            (Kind::Delete, &self.delete),
            (Kind::Action, &self.action),
            (Kind::Transition, &self.transition),
        ];
        let mut set = forms.iter().filter_map(|(k, v)| v.as_deref().map(|v| (*k, v)));
        match (set.next(), set.next()) {
            (Some(one), None) => Ok(one),
            _ => Err(format!("step `{}` needs exactly one of read, create, update, upsert, delete, action or transition", self.name)),
        }
    }
}

/// Field shapes such as `int!` used by action and operation inputs.
pub fn shape_fields(shapes: &BTreeMap<String, String>) -> Result<BTreeMap<String, Field>, String> {
    if shapes.len() > 32 {
        return Err("inputs declare at most 32 fields".into());
    }
    shapes
        .iter()
        .map(|(name, shape)| {
            let field = Field {
                kind: shape.trim_end_matches('!').into(),
                required: shape.ends_with('!'),
                ..Default::default()
            };
            if !crate::contract::identifier(name) || field.ty().is_none() {
                return Err(format!("invalid input `{name}: {shape}`; fix: use a declared field type such as int! or str"));
            }
            Ok((name.clone(), field))
        })
        .collect()
}
fn input_validator(name: &str, shapes: &BTreeMap<String, String>) -> Result<Option<Resource>, String> {
    if shapes.is_empty() {
        return Ok(None);
    }
    Ok(Some(Resource {
        name: name.into(),
        fields: shape_fields(shapes)?,
        search: Vec::new(),
        policy: Default::default(),
        hooks: Default::default(),
        constraints: Default::default(),
        query: Default::default(),
    }))
}

/// A `$` reference inside a declared value.
#[derive(Debug, PartialEq)]
enum Reference<'a> {
    Input(&'a str),
    User(&'a str),
    Now(i64),
    Step(&'a str, &'a str),
    Literal(&'a str),
}
fn reference(s: &str) -> Option<Result<Reference<'_>, String>> {
    let rest = s.strip_prefix('$')?;
    if rest.starts_with('$') {
        return Some(Ok(Reference::Literal(rest)));
    }
    let bad = || Err(format!("invalid reference `{s}`; fix: use $input.<field>, $user.id, $now, $now+7d or $steps.<name>.<id|owner|data.field> ($$ escapes a literal $)"));
    Some(if let Some(field) = rest.strip_prefix("input.") {
        if crate::contract::identifier(field) { Ok(Reference::Input(field)) } else { bad() }
    } else if let Some(field) = rest.strip_prefix("user.") {
        if matches!(field, "id" | "role" | "email") { Ok(Reference::User(field)) } else { bad() }
    } else if rest == "now" {
        Ok(Reference::Now(0))
    } else if let Some(offset) = rest.strip_prefix("now") {
        let (sign, amount) = match offset.as_bytes().first() {
            Some(b'+') => (1, &offset[1..]),
            Some(b'-') => (-1, &offset[1..]),
            _ => return Some(bad()),
        };
        let unit = match amount.chars().last() {
            Some('s') => 1,
            Some('m') => 60,
            Some('h') => 3600,
            Some('d') => 86400,
            _ => return Some(bad()),
        };
        match amount[..amount.len() - 1].parse::<i64>() {
            Ok(n) if (0..=36600).contains(&n) && !amount.starts_with('+') => Ok(Reference::Now(sign * n * unit)),
            _ => bad(),
        }
    } else if let Some(path) = rest.strip_prefix("steps.") {
        match path.split_once('.') {
            Some((step, field))
                if crate::contract::identifier(step)
                    && (matches!(field, "id" | "owner")
                        || field.strip_prefix("data.").is_some_and(crate::contract::identifier)) =>
            {
                Ok(Reference::Step(step, field))
            }
            _ => bad(),
        }
    } else {
        bad()
    })
}
/// References used by a value tree, for load-time validation.
fn references(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) if s.starts_with('$') => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| references(v, out)),
        Value::Object(map) => map.values().for_each(|v| references(v, out)),
        _ => {}
    }
}

const GUARD_OPS: &[&str] = &["eq", "ne", "in", "not_in", "gt", "gte", "lt", "lte", "exists"];
fn guard_ops(value: &Value) -> Vec<(&str, &Value)> {
    match value.as_object() {
        Some(map) if !map.is_empty() && map.keys().all(|k| GUARD_OPS.contains(&k.as_str())) => {
            map.iter().map(|(k, v)| (k.as_str(), v)).collect()
        }
        _ => vec![("eq", value)],
    }
}

fn grants_valid(roles: &[String], allow_owner: bool) -> bool {
    !roles.is_empty()
        && roles.len() <= 16
        && roles.iter().all(|role| {
            role == "owner" && allow_owner || role != "owner" && role != "public" && crate::auth_flows::role_name(role)
        })
}

impl Action {
    fn validate(&self, r: &Resource, label: &str, known_steps: Option<&[String]>) -> Result<(), String> {
        let fail = |m: String| Err(format!("{label}: {m}"));
        if !grants_valid(&self.roles, true) {
            return fail("roles must list 1..16 configured role names or owner; public cannot write".into());
        }
        let inputs = input_validator("input", &self.input).map_err(|m| format!("{label}: {m}"))?;
        let input_names: Vec<String> = inputs.map(|i| i.fields.keys().cloned().collect()).unwrap_or_default();
        if self.set.is_empty() && self.increment.is_empty() && self.decrement.is_empty() {
            return fail("declare set, increment and/or decrement".into());
        }
        for (field, guard) in &self.when {
            let Some(ty) = r.fields.get(field).and_then(Field::ty) else {
                return fail(format!("when references undeclared field `{field}`"));
            };
            for (op, operand) in guard_ops(guard) {
                if matches!(op, "gt" | "gte" | "lt" | "lte") && !(ty.ordered() || ty == Ty::Num) {
                    return fail(format!("`{op}` on `{field}` needs an int, num, decimal, date or datetime field"));
                }
                if matches!(op, "in" | "not_in") && !operand.as_array().is_some_and(|a| !a.is_empty() && a.len() <= 64) {
                    return fail(format!("`{op}` on `{field}` needs a list of 1..64 values"));
                }
                if op == "exists" && !operand.is_boolean() {
                    return fail(format!("`exists` on `{field}` needs true or false"));
                }
            }
        }
        for field in self.set.keys().chain(self.increment.keys()).chain(self.decrement.keys()) {
            if !r.fields.contains_key(field) {
                return fail(format!("assignment to undeclared field `{field}`"));
            }
        }
        for field in self.increment.keys().chain(self.decrement.keys()) {
            let twice = usize::from(self.set.contains_key(field))
                + usize::from(self.increment.contains_key(field))
                + usize::from(self.decrement.contains_key(field));
            if twice > 1 {
                return fail(format!("`{field}` can appear in only one of set, increment and decrement"));
            }
            let ty = r.fields[field].ty();
            if !matches!(ty, Some(Ty::Int | Ty::Num | Ty::Decimal { .. })) {
                return fail(format!("increment/decrement on `{field}` needs an int, num or decimal field"));
            }
        }
        let mut refs = Vec::new();
        for value in self.when.values().chain(self.set.values()).chain(self.increment.values()).chain(self.decrement.values()) {
            references(value, &mut refs);
        }
        for text in refs {
            match reference(&text).unwrap() {
                Err(message) => return fail(message),
                Ok(Reference::Input(name)) if !input_names.iter().any(|n| n == name) => {
                    return fail(format!("`{text}` names an undeclared input; declare it under input"));
                }
                Ok(Reference::Step(step, _)) if !known_steps.is_some_and(|s| s.iter().any(|n| n == step)) => {
                    return fail(format!("`{text}` is only valid inside an operation after step `{step}`"));
                }
                Ok(_) => {}
            }
        }
        Ok(())
    }
}

impl Domain {
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty() && self.operations.is_empty()
    }
    pub fn resource(&self, name: &str) -> Option<&ResourceDomain> {
        self.resources.get(name)
    }
    pub fn batch_max(&self, resource: &str) -> usize {
        self.resource(resource).and_then(|d| d.batch.as_ref()).map_or(DEFAULT_BATCH_MAX, |b| b.max)
    }
    /// Validate every domain contract against the resource contracts. Fails closed before serving.
    pub fn validate(&self, resources: &[Resource]) -> Result<(), String> {
        let find = |name: &str| resources.iter().find(|r| r.name == name);
        for (name, domain) in &self.resources {
            let r = find(name).ok_or_else(|| format!("server/resources/{name}: domain files name an undeclared resource"))?;
            let hooked = ["create", "update", "delete"].iter().any(|op| r.hooks.before(op).is_some());
            if hooked && (!domain.actions.is_empty() || domain.transitions.is_some()) {
                return Err(format!("server/resources/{name}: actions and transitions cannot target a resource with a mandatory before-hook; express the rule as a guard, a constraint or move it into the hook"));
            }
            if domain.actions.len() > 32 {
                return Err(format!("server/resources/{name}/actions.jsbrain: at most 32 actions"));
            }
            for (action_name, action) in &domain.actions {
                if !crate::contract::identifier(action_name) {
                    return Err(format!("server/resources/{name}/actions.jsbrain: invalid action name `{action_name}`"));
                }
                action.validate(r, &format!("server/resources/{name}/actions.jsbrain {action_name}"), None)?;
                if let Some(t) = &domain.transitions
                    && (action.set.contains_key(&t.field) || action.increment.contains_key(&t.field) || action.decrement.contains_key(&t.field))
                {
                    return Err(format!("server/resources/{name}/actions.jsbrain {action_name}: `{}` is a state field; change it with a transition", t.field));
                }
            }
            if let Some(t) = &domain.transitions {
                let label = format!("server/resources/{name}/transitions.jsbrain");
                let Some(Ty::Enum(values)) = r.fields.get(&t.field).and_then(Field::ty) else {
                    return Err(format!("{label}: field must name a declared enum field"));
                };
                let known = |v: &String| values.contains(v);
                if t.initial.is_empty() || !t.initial.iter().all(known) {
                    return Err(format!("{label}: initial must list enum values of `{}`", t.field));
                }
                if t.transitions.is_empty() || t.transitions.len() > 64 {
                    return Err(format!("{label}: declare 1..64 transitions"));
                }
                for (tname, transition) in &t.transitions {
                    if !crate::contract::identifier(tname) {
                        return Err(format!("{label}: invalid transition name `{tname}`"));
                    }
                    if transition.from.is_empty() || !transition.from.iter().all(known) || !known(&transition.to) {
                        return Err(format!("{label} {tname}: from and to must be values of `{}`", t.field));
                    }
                    if transition.set.contains_key(&t.field) || transition.increment.contains_key(&t.field) || transition.decrement.contains_key(&t.field) {
                        return Err(format!("{label} {tname}: `to` already assigns `{}`", t.field));
                    }
                    transition.action(&t.field).validate(r, &format!("{label} {tname}"), None)?;
                }
            }
            if let Some(batch) = &domain.batch
                && !(1..=BATCH_LIMIT).contains(&batch.max)
            {
                return Err(format!("server/resources/{name}/batch.jsbrain: max must be 1..{BATCH_LIMIT}"));
            }
        }
        for (op_name, op) in &self.operations {
            let label = format!("server/operations/{op_name}.jsbrain");
            if !crate::contract::identifier(op_name) {
                return Err(format!("{label}: invalid operation name"));
            }
            if !grants_valid(&op.roles, false) {
                return Err(format!("{label}: roles must list 1..16 configured role names (owner and public are not operation grants)"));
            }
            let inputs = input_validator("input", &op.input).map_err(|m| format!("{label}: {m}"))?;
            let input_names: Vec<String> = inputs.map(|i| i.fields.keys().cloned().collect()).unwrap_or_default();
            if op.steps.is_empty() || op.steps.len() > MAX_STEPS {
                return Err(format!("{label}: declare 1..{MAX_STEPS} steps"));
            }
            let mut seen: Vec<String> = Vec::new();
            for step in &op.steps {
                let slabel = format!("{label} step `{}`", step.name);
                if !crate::contract::identifier(&step.name) || seen.contains(&step.name) {
                    return Err(format!("{slabel}: `as` must be a unique identifier"));
                }
                let (kind, target) = step.kind()?;
                let (resource, member) = match kind {
                    Kind::Action | Kind::Transition => {
                        let (res, member) = target.split_once('.').ok_or_else(|| format!("{slabel}: use \"<resource>.<name>\""))?;
                        (res, Some(member))
                    }
                    _ => (target, None),
                };
                let r = find(resource).ok_or_else(|| format!("{slabel}: unknown resource `{resource}`"))?;
                let op_kind = match kind {
                    Kind::Create => Some("create"),
                    Kind::Update | Kind::Action | Kind::Transition => Some("update"),
                    Kind::Upsert => Some("create"),
                    Kind::Delete => Some("delete"),
                    Kind::Read => None,
                };
                if op_kind.is_some_and(|o| r.hooks.before(o).is_some()) || kind == Kind::Upsert && r.hooks.before("update").is_some() {
                    return Err(format!("{slabel}: `{resource}` declares a mandatory before-hook; operations cannot bypass it"));
                }
                let needs_id = !matches!(kind, Kind::Create | Kind::Upsert);
                if needs_id != step.id.is_some() {
                    return Err(format!("{slabel}: {}", if needs_id { "id is required (for example $input.post_id)" } else { "id is not used by create/upsert" }));
                }
                let data_kind = matches!(kind, Kind::Create | Kind::Update | Kind::Upsert);
                if data_kind == step.data.is_empty() {
                    return Err(format!("{slabel}: {}", if data_kind { "data is required" } else { "data is only used by create, update and upsert" }));
                }
                for key in step.data.keys() {
                    if !r.fields.contains_key(key) {
                        return Err(format!("{slabel}: data names undeclared field `{key}`"));
                    }
                }
                if kind == Kind::Upsert {
                    let mut wanted = step.key.clone();
                    wanted.sort();
                    if !r.unique_sets().iter().any(|(_, set)| {
                        let mut set = set.clone();
                        set.sort();
                        set == wanted
                    }) {
                        return Err(format!("{slabel}: key must match a declared unique field or composite unique set of `{resource}`"));
                    }
                } else if !step.key.is_empty() {
                    return Err(format!("{slabel}: key is only used by upsert"));
                }
                if !matches!(kind, Kind::Action | Kind::Transition) && !step.input.is_empty() {
                    return Err(format!("{slabel}: input is only used by action and transition steps"));
                }
                let domain = self.resources.get(resource);
                match (kind, member) {
                    (Kind::Action, Some(m)) if domain.is_some_and(|d| d.actions.contains_key(m)) => {}
                    (Kind::Transition, Some(m)) if domain.and_then(|d| d.transitions.as_ref()).is_some_and(|t| t.transitions.contains_key(m)) => {}
                    (Kind::Action | Kind::Transition, _) => return Err(format!("{slabel}: `{target}` is not declared")),
                    _ => {}
                }
                if let Some(t) = domain.and_then(|d| d.transitions.as_ref())
                    && matches!(kind, Kind::Update | Kind::Upsert)
                    && step.data.contains_key(&t.field)
                {
                    return Err(format!("{slabel}: `{}` is a state field; use a transition step", t.field));
                }
                let mut refs = Vec::new();
                for value in step.data.values().chain(step.input.values()) {
                    references(value, &mut refs);
                }
                if let Some(id) = &step.id {
                    if !id.starts_with('$') {
                        return Err(format!("{slabel}: id must be a reference such as $input.post_id or $steps.<name>.id"));
                    }
                    refs.push(id.clone());
                }
                for text in refs {
                    match reference(&text).unwrap() {
                        Err(message) => return Err(format!("{slabel}: {message}")),
                        Ok(Reference::Input(name)) if !input_names.iter().any(|n| n == name) => {
                            return Err(format!("{slabel}: `{text}` names an undeclared operation input"));
                        }
                        Ok(Reference::Step(name, _)) if !seen.iter().any(|n| n == name) => {
                            return Err(format!("{slabel}: `{text}` must reference an earlier step"));
                        }
                        Ok(_) => {}
                    }
                }
                seen.push(step.name.clone());
            }
            for name in &op.result {
                if !seen.contains(name) {
                    return Err(format!("{label}: result names unknown step `{name}`"));
                }
            }
        }
        Ok(())
    }
    /// The state field of a resource with a declared state machine changes only through its transitions.
    pub fn protect(&self, r: &Resource, before: Option<&Value>, after: &Value) -> Result<(), BTreeMap<String, String>> {
        let Some(t) = self.resource(&r.name).and_then(|d| d.transitions.as_ref()) else {
            return Ok(());
        };
        let value = after.get(&t.field);
        let ok = match before {
            None => value.and_then(Value::as_str).is_some_and(|v| t.initial.iter().any(|i| i == v)),
            Some(before) => before.get(&t.field) == value,
        };
        if ok {
            Ok(())
        } else {
            let names: Vec<&str> = t.transitions.keys().map(String::as_str).collect();
            Err(BTreeMap::from([(
                t.field.clone(),
                if before.is_none() {
                    format!("Must start as one of: {}", t.initial.join(", "))
                } else {
                    format!("Change this field only through a declared transition: {}", names.join(", "))
                },
            )]))
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Execution: every function below runs inside the caller's transaction. Nothing commits here.
// ---------------------------------------------------------------------------------------------------------------
pub type Tx = sqlx::Transaction<'static, sqlx::Postgres>;

/// Typed failure; the HTTP layer maps it to a status and stable code. Any failure rolls back the transaction.
#[derive(Debug)]
pub enum Failure {
    /// 403 POLICY_DENIED
    Forbidden,
    /// 404 NOT_FOUND; the label names the step or batch item.
    Missing(String),
    /// 422 INVALID_INPUT with field errors.
    Invalid(BTreeMap<String, String>),
    /// 409 GUARD_FAILED: the locked current record does not satisfy a declared condition.
    Guard(BTreeMap<String, String>),
    /// Database failure on `resource`; declared unique/check names are reported with the label prefix.
    Database { resource: String, label: Option<String>, error: sqlx::Error },
}
impl From<sqlx::Error> for Failure {
    fn from(error: sqlx::Error) -> Self {
        Failure::Database { resource: String::new(), label: None, error }
    }
}
fn prefixed(label: &str, errors: BTreeMap<String, String>) -> BTreeMap<String, String> {
    errors.into_iter().map(|(k, v)| (format!("{label}.{k}"), v)).collect()
}
fn located(resource: &str, label: Option<&str>) -> impl FnOnce(sqlx::Error) -> Failure {
    let (resource, label) = (resource.to_string(), label.map(str::to_string));
    move |error| Failure::Database { resource, label, error }
}

/// One committed change, reported to live subscribers after commit.
#[derive(Clone, Debug)]
pub struct Change {
    pub resource: String,
    pub operation: &'static str,
    pub record: Value,
}

fn table(r: &Resource) -> String {
    assert!(crate::contract::identifier(&r.name));
    format!("_brain_res_{}", r.name)
}
fn columns() -> &'static str {
    crate::record_columns!()
}
async fn lock(tx: &mut Tx, r: &Resource, id: Uuid) -> Result<Option<Value>, sqlx::Error> {
    // Audited SQL: the table identifier is contract-validated; the id is bound.
    let sql = format!("SELECT {} FROM {} WHERE id=$1 FOR UPDATE", columns(), table(r));
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map(|row| row.map(crate::db::record))
}
async fn lock_many(tx: &mut Tx, r: &Resource, ids: &[Uuid]) -> Result<Vec<Value>, sqlx::Error> {
    // Ordered locking prevents deadlocks between concurrent batches touching the same rows.
    let sql = format!("SELECT {} FROM {} WHERE id=ANY($1) ORDER BY id FOR UPDATE", columns(), table(r));
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(ids)
        .fetch_all(&mut **tx)
        .await
        .map(|rows| rows.into_iter().map(crate::db::record).collect())
}
async fn insert(tx: &mut Tx, r: &Resource, owner: Uuid, data: Value) -> Result<Value, sqlx::Error> {
    let sql = format!("INSERT INTO {}(id,owner,data) VALUES($1,$2,$3) RETURNING {}", table(r), columns());
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(Uuid::new_v4())
        .bind(owner)
        .bind(data)
        .fetch_one(&mut **tx)
        .await
        .map(crate::db::record)?;
    crate::db::hook(tx, r, "create", &row).await?;
    Ok(row)
}
async fn insert_absent(tx: &mut Tx, r: &Resource, owner: Uuid, data: &Value, key: &[String]) -> Result<Option<Value>, sqlx::Error> {
    // ON CONFLICT infers the declared unique expression index; key fields are contract-validated identifiers.
    let target = key.iter().map(|f| format!("(data->>'{f}')")).collect::<Vec<_>>().join(",");
    let sql = format!("INSERT INTO {}(id,owner,data) VALUES($1,$2,$3) ON CONFLICT ({target}) DO NOTHING RETURNING {}", table(r), columns());
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(Uuid::new_v4())
        .bind(owner)
        .bind(data)
        .fetch_optional(&mut **tx)
        .await?
        .map(crate::db::record);
    if let Some(row) = &row {
        crate::db::hook(tx, r, "create", row).await?;
    }
    Ok(row)
}
async fn lock_by_key(tx: &mut Tx, r: &Resource, data: &Value, key: &[String]) -> Result<Option<Value>, sqlx::Error> {
    let filter = key
        .iter()
        .enumerate()
        .map(|(i, f)| format!("data->>'{f}' IS NOT DISTINCT FROM ${}", i + 1))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!("SELECT {} FROM {} WHERE {filter} FOR UPDATE", columns(), table(r));
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for field in key {
        let text = data.get(field).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
        query = query.bind(text);
    }
    query.fetch_optional(&mut **tx).await.map(|row| row.map(crate::db::record))
}
async fn replace(tx: &mut Tx, r: &Resource, id: Uuid, data: Value) -> Result<Value, sqlx::Error> {
    let sql = format!("UPDATE {} SET data=$2,updated_at=now() WHERE id=$1 RETURNING {}", table(r), columns());
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .bind(data)
        .fetch_one(&mut **tx)
        .await
        .map(crate::db::record)?;
    crate::db::hook(tx, r, "update", &row).await?;
    Ok(row)
}
async fn remove(tx: &mut Tx, r: &Resource, id: Uuid) -> Result<Value, sqlx::Error> {
    let sql = format!("DELETE FROM {} WHERE id=$1 RETURNING {}", table(r), columns());
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map(crate::db::record)?;
    crate::db::hook(tx, r, "delete", &row).await?;
    Ok(row)
}

/// Transaction timestamp (`now()` is constant inside one transaction), formatted without a date library.
#[derive(Clone, Copy, Debug)]
struct Clock(i64);
impl Clock {
    async fn read(tx: &mut Tx) -> Result<Self, sqlx::Error> {
        sqlx::query_scalar::<_, i64>("SELECT (extract(epoch FROM now())*1000)::bigint")
            .fetch_one(&mut **tx)
            .await
            .map(Clock)
    }
    fn value(self, offset_seconds: i64, ty: Option<&Ty>) -> Value {
        let ms = self.0 + offset_seconds * 1000;
        let days = ms.div_euclid(86_400_000);
        let rem = ms.rem_euclid(86_400_000);
        // Howard Hinnant's civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        let date = format!("{y:04}-{m:02}-{d:02}");
        Value::String(if ty == Some(&Ty::Date) {
            date
        } else {
            format!("{date}T{:02}:{:02}:{:02}.{:03}Z", rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000)
        })
    }
}

/// Values available to `$` references while one action, step or batch item runs.
struct Scope<'a> {
    input: &'a Map<String, Value>,
    user: &'a User,
    steps: &'a BTreeMap<String, Value>,
    clock: Clock,
}
impl Scope<'_> {
    fn resolve(&self, value: &Value, ty: Option<&Ty>) -> Value {
        match value {
            Value::String(text) => match reference(text) {
                None => value.clone(),
                Some(Err(_)) => Value::Null, // contracts are validated at load; unreachable for served contracts
                Some(Ok(reference)) => match reference {
                    Reference::Literal(rest) => Value::String(rest.to_string()),
                    Reference::Input(name) => self.input.get(name).cloned().unwrap_or(Value::Null),
                    Reference::User(field) => match field {
                        "id" => json!(self.user.id),
                        "role" => json!(self.user.role),
                        _ => json!(self.user.email),
                    },
                    Reference::Now(offset) => self.clock.value(offset, ty),
                    Reference::Step(step, field) => {
                        let record = self.steps.get(step).cloned().unwrap_or(Value::Null);
                        match field.strip_prefix("data.") {
                            Some(name) => record["data"].get(name).cloned().unwrap_or(Value::Null),
                            None => record.get(field).cloned().unwrap_or(Value::Null),
                        }
                    }
                },
            },
            Value::Array(items) => Value::Array(items.iter().map(|v| self.resolve(v, None)).collect()),
            Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), self.resolve(v, None))).collect()),
            other => other.clone(),
        }
    }
    fn resolve_data(&self, r: &Resource, data: &Map<String, Value>) -> Value {
        Value::Object(
            data.iter()
                .map(|(k, v)| (k.clone(), self.resolve(v, r.fields.get(k).and_then(Field::ty).as_ref())))
                .collect(),
        )
    }
}

fn guard_holds(ty: &Ty, current: Option<&Value>, op: &str, operand: &Value) -> bool {
    let normalized = |v: &Value| ty.normalize(v, false);
    let equal = |v: &Value| current.is_some_and(|c| normalized(v).is_some_and(|n| &n == c));
    let order = |v: &Value| -> Option<std::cmp::Ordering> {
        let (c, n) = (current?, normalized(v)?);
        match ty {
            Ty::Num => c.as_f64()?.partial_cmp(&n.as_f64()?),
            _ => ty.compare(c, &n),
        }
    };
    match op {
        "eq" => equal(operand),
        "ne" => !equal(operand),
        "in" => operand.as_array().is_some_and(|a| a.iter().any(equal)),
        "not_in" => !operand.as_array().is_some_and(|a| a.iter().any(equal)),
        "gt" => order(operand).is_some_and(|o| o.is_gt()),
        "gte" => order(operand).is_some_and(|o| o.is_ge()),
        "lt" => order(operand).is_some_and(|o| o.is_lt()),
        "lte" => order(operand).is_some_and(|o| o.is_le()),
        "exists" => current.is_some() == (operand == &Value::Bool(true)),
        _ => false,
    }
}
fn describe(op: &str, operand: &Value) -> String {
    let text = operand.as_str().map(str::to_string).unwrap_or_else(|| operand.to_string());
    match op {
        "eq" => format!("Must equal {text}"),
        "ne" => format!("Must not equal {text}"),
        "in" => format!("Must be one of {text}"),
        "not_in" => format!("Must not be one of {text}"),
        "gt" => format!("Must be > {text}"),
        "gte" => format!("Must be >= {text}"),
        "lt" => format!("Must be < {text}"),
        "lte" => format!("Must be <= {text}"),
        _ => if operand == &Value::Bool(true) { "Must be present".into() } else { "Must be absent".into() },
    }
}
/// Exact addition on canonical decimal strings with `scale` fractional digits.
fn decimal_add(a: &str, b: &str, precision: u8, scale: u8) -> Option<String> {
    let units = |s: &str| -> Option<i128> {
        let canonical = crate::types::decimal(s, 38, scale)?;
        let negative = canonical.starts_with('-');
        let digits: String = canonical.chars().filter(char::is_ascii_digit).collect();
        let n: i128 = digits.parse().ok()?;
        Some(if negative { -n } else { n })
    };
    let sum = units(a)?.checked_add(units(b)?)?;
    let (sign, abs) = (if sum < 0 { "-" } else { "" }, sum.unsigned_abs().to_string());
    let scale = scale as usize;
    let padded = format!("{abs:0>width$}", width = scale + 1);
    let (int, frac) = padded.split_at(padded.len() - scale);
    let text = if scale == 0 { format!("{sign}{int}") } else { format!("{sign}{int}.{frac}") };
    crate::types::decimal(&text, precision, scale as u8)
}
/// Negated amount for `decrement`; anything that is not a number or decimal string stays invalid.
fn negate(value: &Value) -> Value {
    match value {
        Value::Number(n) if n.is_i64() => n.as_i64().and_then(i64::checked_neg).map_or(Value::Null, Value::from),
        Value::Number(n) => n.as_f64().and_then(|f| serde_json::Number::from_f64(-f)).map_or(Value::Null, Value::Number),
        Value::String(s) => Value::String(s.strip_prefix('-').map_or_else(|| format!("-{s}"), str::to_string)),
        _ => Value::Null,
    }
}
fn increment(ty: &Ty, current: Option<&Value>, delta: &Value) -> Result<Value, String> {
    match ty {
        Ty::Int => {
            let (c, d) = (current.and_then(Value::as_i64).unwrap_or(0), delta.as_i64().ok_or("Increment must be an integer")?);
            c.checked_add(d)
                .filter(|n| n.abs() <= crate::types::SAFE_INT)
                .map(Value::from)
                .ok_or_else(|| "Result is outside the safe integer range".into())
        }
        Ty::Num => {
            let (c, d) = (current.and_then(Value::as_f64).unwrap_or(0.0), delta.as_f64().ok_or("Increment must be a number")?);
            serde_json::Number::from_f64(c + d).map(Value::Number).ok_or_else(|| "Result is not a finite number".into())
        }
        Ty::Decimal { precision, scale } => {
            let delta = match delta {
                Value::String(s) => s.clone(),
                Value::Number(n) if n.is_i64() => n.to_string(),
                _ => return Err("Decimal increments are strings such as \"-1.50\"".into()),
            };
            let current = current.and_then(Value::as_str).unwrap_or("0");
            decimal_add(current, &delta, *precision, *scale)
                .map(Value::String)
                .ok_or_else(|| format!("Result does not fit decimal({precision},{scale})"))
        }
        _ => Err("Increment needs an int, num or decimal field".into()),
    }
}

/// Authority used to execute declared writes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Authority {
    /// Check the action's roles (owner = caller owns the row).
    Roles,
    /// Inside an operation: the operation's roles were checked; optionally require ownership.
    Operation { owned: bool },
}
fn owns(record: &Value, user: &User) -> bool {
    record["owner"].as_str() == Some(user.id.to_string().as_str())
}

/// Executes declared domain writes for one authenticated caller inside one transaction.
pub struct Engine<'a> {
    pub domain: &'a Domain,
    pub resources: &'a BTreeMap<String, Resource>,
    pub user: &'a User,
    pub changes: Vec<Change>,
    clock: Option<Clock>,
}
impl<'a> Engine<'a> {
    pub fn new(domain: &'a Domain, resources: &'a BTreeMap<String, Resource>, user: &'a User) -> Self {
        Self { domain, resources, user, changes: Vec::new(), clock: None }
    }
    async fn clock(&mut self, tx: &mut Tx) -> Result<Clock, sqlx::Error> {
        if self.clock.is_none() {
            self.clock = Some(Clock::read(tx).await?);
        }
        Ok(self.clock.unwrap())
    }
    fn resource(&self, name: &str) -> Result<&'a Resource, Failure> {
        self.resources.get(name).ok_or_else(|| Failure::Missing(name.into()))
    }
    fn validate_input(shapes: &BTreeMap<String, String>, input: &Value, label: &str) -> Result<Map<String, Value>, Failure> {
        let value = if input.is_null() { json!({}) } else { input.clone() };
        match input_validator("input", shapes).map_err(|m| Failure::Invalid(BTreeMap::from([(label.into(), m)])))? {
            None if value.as_object().is_some_and(Map::is_empty) => Ok(Map::new()),
            None => Err(Failure::Invalid(BTreeMap::from([(label.to_string(), "This action declares no input; send {}".into())]))),
            Some(validator) => validator
                .input(&value)
                .map(|v| v.as_object().cloned().unwrap_or_default())
                .map_err(|e| Failure::Invalid(prefixed(label, e))),
        }
    }

    /// Run one declared action (or transition when `state` is set) on a locked row.
    async fn act(
        &mut self,
        tx: &mut Tx,
        r: &'a Resource,
        action: &Action,
        id: Uuid,
        input: &Value,
        authority: Authority,
        label: Option<&str>,
        steps: &BTreeMap<String, Value>,
        state: Option<&str>,
    ) -> Result<Value, Failure> {
        let at = |m: BTreeMap<String, String>| match label {
            Some(l) => prefixed(l, m),
            None => m,
        };
        let input = Self::validate_input(&action.input, input, &label.map_or("input".into(), |l| format!("{l}.input")))?;
        let clock = self.clock(tx).await?;
        let existing = lock(tx, r, id).await.map_err(located(&r.name, label))?
            .ok_or_else(|| Failure::Missing(label.unwrap_or("id").into()))?;
        match authority {
            Authority::Roles => {
                let allowed = action.roles.iter().any(|role| role == &self.user.role || role == "owner" && owns(&existing, self.user));
                if !allowed {
                    return Err(Failure::Forbidden);
                }
            }
            Authority::Operation { owned } if owned && !owns(&existing, self.user) => {
                return Err(Failure::Missing(label.unwrap_or("id").into()));
            }
            Authority::Operation { .. } => {}
        }
        let scope = Scope { input: &input, user: self.user, steps, clock };
        let data = existing["data"].as_object().cloned().unwrap_or_default();
        let mut failed = BTreeMap::new();
        for (field, guard) in &action.when {
            let ty = r.fields[field].ty().expect("validated field");
            for (op, operand) in guard_ops(guard) {
                let operand = scope.resolve(operand, Some(&ty));
                if !guard_holds(&ty, data.get(field), op, &operand) {
                    let message = match state {
                        Some(name) if Some(field.as_str()) == self.domain.resource(&r.name).and_then(|d| d.transitions.as_ref()).map(|t| t.field.as_str()) => {
                            format!("Transition {name} is allowed from {}", operand.as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default())
                        }
                        _ => describe(op, &operand),
                    };
                    failed.entry(field.clone()).or_insert(message);
                }
            }
        }
        if !failed.is_empty() {
            return Err(Failure::Guard(at(failed)));
        }
        let mut next = data.clone();
        let mut errors = BTreeMap::new();
        for (field, value) in &action.set {
            let ty = r.fields[field].ty();
            match scope.resolve(value, ty.as_ref()) {
                Value::Null => {
                    next.remove(field);
                }
                v => {
                    next.insert(field.clone(), v);
                }
            }
        }
        let deltas = action.increment.iter().map(|(f, d)| (f, scope.resolve(d, None)))
            .chain(action.decrement.iter().map(|(f, d)| (f, negate(&scope.resolve(d, None)))));
        for (field, delta) in deltas {
            let ty = r.fields[field].ty().expect("validated field");
            match increment(&ty, data.get(field), &delta) {
                Ok(v) => {
                    next.insert(field.clone(), v);
                }
                Err(m) => {
                    errors.insert(field.clone(), m);
                }
            }
        }
        if !errors.is_empty() {
            return Err(Failure::Invalid(at(errors)));
        }
        let validated = r.input(&Value::Object(next)).map_err(|e| Failure::Invalid(at(e)))?;
        if state.is_none() {
            self.domain.protect(r, Some(&existing["data"]), &validated).map_err(|e| Failure::Invalid(at(e)))?;
        }
        let record = replace(tx, r, id, validated).await.map_err(located(&r.name, label))?;
        self.changes.push(Change { resource: r.name.clone(), operation: "update", record: record.clone() });
        Ok(record)
    }

    /// `POST /api/<resource>/<id>/actions/<action>`.
    pub async fn action(&mut self, tx: &mut Tx, resource: &str, name: &str, id: Uuid, input: &Value) -> Result<Value, Failure> {
        let r = self.resource(resource)?;
        let action = self.domain.resource(resource).and_then(|d| d.actions.get(name)).ok_or_else(|| Failure::Missing("action".into()))?;
        self.act(tx, r, action, id, input, Authority::Roles, None, &BTreeMap::new(), None).await
    }
    /// `POST /api/<resource>/<id>/transitions/<transition>`.
    pub async fn transition(&mut self, tx: &mut Tx, resource: &str, name: &str, id: Uuid, input: &Value) -> Result<Value, Failure> {
        let r = self.resource(resource)?;
        let t = self.domain.resource(resource).and_then(|d| d.transitions.as_ref()).ok_or_else(|| Failure::Missing("transition".into()))?;
        let transition = t.transitions.get(name).ok_or_else(|| Failure::Missing("transition".into()))?;
        let action = transition.action(&t.field);
        self.act(tx, r, &action, id, input, Authority::Roles, None, &BTreeMap::new(), Some(name)).await
    }

    /// Insert unless the declared unique key already exists. In `update` mode the sent fields are merged into the
    /// existing row (fields not sent keep their stored values) and the result is revalidated.
    pub async fn upsert(
        &mut self,
        tx: &mut Tx,
        r: &'a Resource,
        data: &Value,
        key: &[String],
        update: bool,
        label: Option<&str>,
        can_update: &(dyn Fn(&Value) -> bool + Sync),
    ) -> Result<(Value, bool), Failure> {
        let at = |m: BTreeMap<String, String>| match label {
            Some(l) => prefixed(l, m),
            None => m,
        };
        let full = r.input(data).map_err(|e| Failure::Invalid(at(e)))?;
        if key.iter().any(|field| full.get(field).is_none()) {
            return Err(Failure::Invalid(at(key.iter().filter(|f| full.get(*f).is_none()).map(|f| (f.clone(), "Required for the upsert key".into())).collect())));
        }
        for _ in 0..3 {
            self.domain.protect(r, None, &full).map_err(|e| Failure::Invalid(at(e)))?;
            if let Some(row) = insert_absent(tx, r, self.user.id, &full, key).await.map_err(located(&r.name, label))? {
                self.changes.push(Change { resource: r.name.clone(), operation: "create", record: row.clone() });
                return Ok((row, true));
            }
            let Some(existing) = lock_by_key(tx, r, &full, key).await.map_err(located(&r.name, label))? else {
                continue; // removed concurrently between the conflict and the lock: retry the insert
            };
            if !update {
                return Ok((existing, false));
            }
            if !can_update(&existing) {
                return Err(Failure::Forbidden);
            }
            let sent = data.as_object().cloned().unwrap_or_default();
            let merged = merge_patch(r, &existing["data"], &Value::Object(sent)).map_err(|e| Failure::Invalid(at(e)))?;
            let validated = r.input(&merged).map_err(|e| Failure::Invalid(at(e)))?;
            self.domain.protect(r, Some(&existing["data"]), &validated).map_err(|e| Failure::Invalid(at(e)))?;
            let id = Uuid::parse_str(existing["id"].as_str().unwrap_or_default()).map_err(|_| Failure::Missing("id".into()))?;
            let row = replace(tx, r, id, validated).await.map_err(located(&r.name, label))?;
            self.changes.push(Change { resource: r.name.clone(), operation: "update", record: row.clone() });
            return Ok((row, false));
        }
        Err(Failure::Guard(at(BTreeMap::from([("key".into(), "The record changed concurrently; retry".into())]))))
    }

    /// `POST /_brain/operations/<name>`: every step in one transaction; any failure rolls all of them back.
    pub async fn operation(&mut self, tx: &mut Tx, name: &str, input: &Value) -> Result<Value, Failure> {
        let op = self.domain.operations.get(name).ok_or_else(|| Failure::Missing("operation".into()))?;
        if !op.roles.iter().any(|role| role == &self.user.role) {
            return Err(Failure::Forbidden);
        }
        let input = Self::validate_input(&op.input, input, "input")?;
        let clock = self.clock(tx).await?;
        let mut steps: BTreeMap<String, Value> = BTreeMap::new();
        for step in &op.steps {
            let (kind, target) = step.kind().map_err(|m| Failure::Invalid(BTreeMap::from([(step.name.clone(), m)])))?;
            let label = step.name.as_str();
            let resource_name = target.split('.').next().unwrap_or(target);
            let r = self.resource(resource_name)?;
            let scope = Scope { input: &input, user: self.user, steps: &steps, clock };
            let id = match &step.id {
                None => None,
                Some(expr) => Some(
                    scope
                        .resolve(&Value::String(expr.clone()), None)
                        .as_str()
                        .and_then(|s| Uuid::parse_str(s).ok())
                        .ok_or_else(|| Failure::Invalid(BTreeMap::from([(format!("{label}.id"), "Expected a record id".into())])))?,
                ),
            };
            let authority = Authority::Operation { owned: step.owned };
            let record = match kind {
                Kind::Read | Kind::Update | Kind::Delete => {
                    let id = id.expect("validated id");
                    let existing = lock(tx, r, id).await.map_err(located(&r.name, Some(label)))?
                        .ok_or_else(|| Failure::Missing(label.into()))?;
                    if step.owned && !owns(&existing, self.user) {
                        return Err(Failure::Missing(label.into()));
                    }
                    match kind {
                        Kind::Read => existing,
                        Kind::Delete => {
                            let row = remove(tx, r, id).await.map_err(located(&r.name, Some(label)))?;
                            self.changes.push(Change { resource: r.name.clone(), operation: "delete", record: json!({"id":row["id"],"owner":row["owner"]}) });
                            row
                        }
                        _ => {
                            let patch = scope.resolve_data(r, &step.data);
                            let merged = merge_patch(r, &existing["data"], &patch).map_err(|e| Failure::Invalid(prefixed(label, e)))?;
                            let validated = r.input(&merged).map_err(|e| Failure::Invalid(prefixed(label, e)))?;
                            self.domain.protect(r, Some(&existing["data"]), &validated).map_err(|e| Failure::Invalid(prefixed(label, e)))?;
                            let row = replace(tx, r, id, validated).await.map_err(located(&r.name, Some(label)))?;
                            self.changes.push(Change { resource: r.name.clone(), operation: "update", record: row.clone() });
                            row
                        }
                    }
                }
                Kind::Create => {
                    let data = scope.resolve_data(r, &step.data);
                    let validated = r.input(&data).map_err(|e| Failure::Invalid(prefixed(label, e)))?;
                    self.domain.protect(r, None, &validated).map_err(|e| Failure::Invalid(prefixed(label, e)))?;
                    let row = insert(tx, r, self.user.id, validated).await.map_err(located(&r.name, Some(label)))?;
                    self.changes.push(Change { resource: r.name.clone(), operation: "create", record: row.clone() });
                    row
                }
                Kind::Upsert => {
                    let data = scope.resolve_data(r, &step.data);
                    let user = self.user;
                    let owned = step.owned;
                    let can = move |row: &Value| !owned || owns(row, user);
                    self.upsert(tx, r, &data, &step.key, true, Some(label), &can).await?.0
                }
                Kind::Action | Kind::Transition => {
                    let member = target.split_once('.').map(|(_, m)| m).unwrap_or_default();
                    let step_input = Value::Object(step.input.iter().map(|(k, v)| (k.clone(), scope.resolve(v, None))).collect());
                    let domain = self.domain.resource(&r.name).ok_or_else(|| Failure::Missing(label.into()))?;
                    let (action, state) = if kind == Kind::Action {
                        (domain.actions.get(member).cloned().ok_or_else(|| Failure::Missing(label.into()))?, None)
                    } else {
                        let t = domain.transitions.as_ref().ok_or_else(|| Failure::Missing(label.into()))?;
                        (t.transitions.get(member).ok_or_else(|| Failure::Missing(label.into()))?.action(&t.field), Some(member))
                    };
                    let steps_now = steps.clone();
                    self.act(tx, r, &action, id.expect("validated id"), &step_input, authority, Some(label), &steps_now, state).await?
                }
            };
            steps.insert(step.name.clone(), record);
        }
        let result: Map<String, Value> = steps
            .into_iter()
            .filter(|(name, _)| op.result.is_empty() || op.result.contains(name))
            .collect();
        Ok(json!({"operation": name, "result": result}))
    }
}

/// Body of `POST /api/<resource>/batch`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BatchRequest {
    pub op: String,
    pub items: Vec<Value>,
    #[serde(default)]
    pub key: Vec<String>,
    #[serde(default)]
    pub on_conflict: Option<String>,
}
/// Body of `POST /api/<resource>/upsert`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertRequest {
    pub key: Vec<String>,
    pub data: Value,
    #[serde(default)]
    pub on_conflict: Option<String>,
}
/// The key must be exactly one declared unique set (single `unique: true` field or composite set).
pub fn upsert_key(r: &Resource, key: &[String]) -> Result<Vec<String>, BTreeMap<String, String>> {
    let mut wanted = key.to_vec();
    wanted.sort();
    r.unique_sets()
        .into_iter()
        .map(|(_, set)| set)
        .find(|set| {
            let mut sorted = set.clone();
            sorted.sort();
            sorted == wanted
        })
        .ok_or_else(|| BTreeMap::from([("key".into(), "Must match a declared unique field or composite unique set".into())]))
}
fn conflict_mode(value: Option<&str>) -> Result<bool, BTreeMap<String, String>> {
    match value.unwrap_or("update") {
        "update" => Ok(true),
        "ignore" => Ok(false),
        _ => Err(BTreeMap::from([("on_conflict".into(), "Use update or ignore".into())])),
    }
}
impl<'a> Engine<'a> {
    /// Single-record upsert with the resource policy: create grant always, update grant for the existing row.
    pub async fn upsert_request(&mut self, tx: &mut Tx, r: &'a Resource, request: &UpsertRequest) -> Result<(Value, bool), Failure> {
        let role = self.user.role.clone();
        if !r.policy.allows("create", Some(&role), false) {
            return Err(Failure::Forbidden);
        }
        let key = upsert_key(r, &request.key).map_err(Failure::Invalid)?;
        let update = conflict_mode(request.on_conflict.as_deref()).map_err(Failure::Invalid)?;
        let user = self.user;
        let policy = r.policy.clone();
        let can = move |row: &Value| policy.allows("update", Some(&user.role), owns(row, user));
        self.upsert(tx, r, &request.data, &key, update, None, &can).await
    }

    /// All-or-nothing batch with per-index errors (`items.<i>.<field>`). Every item is validated before any write.
    pub async fn batch(&mut self, tx: &mut Tx, r: &'a Resource, request: &BatchRequest) -> Result<Value, Failure> {
        let max = self.domain.batch_max(&r.name);
        if request.items.is_empty() || request.items.len() > max {
            return Err(Failure::Invalid(BTreeMap::from([("items".into(), format!("Send 1..{max} items"))])));
        }
        let role = self.user.role.clone();
        let op = request.op.as_str();
        let grant = match op {
            "create" | "upsert" => "create",
            "update" | "delete" => op,
            _ => return Err(Failure::Invalid(BTreeMap::from([("op".into(), "Use create, update, upsert or delete".into())]))),
        };
        if !r.policy.allows(grant, Some(&role), grant != "create") {
            return Err(Failure::Forbidden);
        }
        if op != "upsert" && (!request.key.is_empty() || request.on_conflict.is_some()) {
            return Err(Failure::Invalid(BTreeMap::from([("key".into(), "key and on_conflict apply only to upsert".into())])));
        }
        let item = |i: usize| format!("items.{i}");
        let mut errors = BTreeMap::new();
        let mut out = Vec::with_capacity(request.items.len());
        match op {
            "create" => {
                let mut rows = Vec::new();
                for (i, value) in request.items.iter().enumerate() {
                    match r.input(value).and_then(|data| self.domain.protect(r, None, &data).map(|_| data)) {
                        Ok(data) => rows.push(data),
                        Err(e) => errors.extend(prefixed(&item(i), e)),
                    }
                }
                if !errors.is_empty() {
                    return Err(Failure::Invalid(errors));
                }
                for (i, data) in rows.into_iter().enumerate() {
                    let row = insert(tx, r, self.user.id, data).await.map_err(located(&r.name, Some(&item(i))))?;
                    self.changes.push(Change { resource: r.name.clone(), operation: "create", record: row.clone() });
                    out.push(row);
                }
                Ok(json!({"op":op,"count":out.len(),"items":out}))
            }
            "upsert" => {
                let key = upsert_key(r, &request.key).map_err(Failure::Invalid)?;
                let update = conflict_mode(request.on_conflict.as_deref()).map_err(Failure::Invalid)?;
                for (i, value) in request.items.iter().enumerate() {
                    if let Err(e) = r.input(value) {
                        errors.extend(prefixed(&item(i), e));
                    }
                }
                if !errors.is_empty() {
                    return Err(Failure::Invalid(errors));
                }
                let user = self.user;
                let policy = r.policy.clone();
                let can = move |row: &Value| policy.allows("update", Some(&user.role), owns(row, user));
                let mut created = Vec::new();
                for (i, value) in request.items.iter().enumerate() {
                    let (row, new) = self.upsert(tx, r, value, &key, update, Some(&item(i)), &can).await?;
                    created.push(new);
                    out.push(row);
                }
                Ok(json!({"op":op,"count":out.len(),"items":out,"created":created}))
            }
            _ => {
                // update: { id, data } with PATCH semantics; delete: { id }.
                let mut ids = Vec::new();
                for (i, value) in request.items.iter().enumerate() {
                    let id = value.get("id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok());
                    let shape_ok = value.as_object().is_some_and(|o| {
                        o.keys().all(|k| k == "id" || op == "update" && k == "data") && (op == "delete" || o.contains_key("data"))
                    });
                    match id {
                        Some(id) if shape_ok && !ids.contains(&id) => ids.push(id),
                        Some(_) if shape_ok => {
                            errors.insert(format!("{}.id", item(i)), "Duplicate id in this batch".into());
                        }
                        _ => {
                            errors.insert(item(i), if op == "update" { "Expected { id, data }".into() } else { "Expected { id }".into() });
                        }
                    }
                }
                if !errors.is_empty() {
                    return Err(Failure::Invalid(errors));
                }
                let locked = lock_many(tx, r, &ids).await.map_err(located(&r.name, None))?;
                let owners_only = r.policy.owners_only(op, Some(&role));
                let mut writes = Vec::new();
                for (i, id) in ids.iter().enumerate() {
                    let Some(existing) = locked.iter().find(|row| row["id"].as_str() == Some(id.to_string().as_str())) else {
                        return Err(Failure::Missing(item(i)));
                    };
                    if owners_only && !owns(existing, self.user) {
                        return Err(Failure::Missing(item(i)));
                    }
                    if op == "update" {
                        let merged = merge_patch(r, &existing["data"], &request.items[i]["data"])
                            .and_then(|m| r.input(&m))
                            .and_then(|v| self.domain.protect(r, Some(&existing["data"]), &v).map(|_| v));
                        match merged {
                            Ok(data) => writes.push((*id, Some(data))),
                            Err(e) => errors.extend(prefixed(&item(i), e)),
                        }
                    } else {
                        writes.push((*id, None));
                    }
                }
                if !errors.is_empty() {
                    return Err(Failure::Invalid(errors));
                }
                for (i, (id, data)) in writes.into_iter().enumerate() {
                    let row = match data {
                        Some(data) => {
                            let row = replace(tx, r, id, data).await.map_err(located(&r.name, Some(&item(i))))?;
                            self.changes.push(Change { resource: r.name.clone(), operation: "update", record: row.clone() });
                            row
                        }
                        None => {
                            let row = remove(tx, r, id).await.map_err(located(&r.name, Some(&item(i))))?;
                            let deleted = json!({"id":row["id"],"owner":row["owner"]});
                            self.changes.push(Change { resource: r.name.clone(), operation: "delete", record: deleted.clone() });
                            deleted
                        }
                    };
                    out.push(row);
                }
                Ok(json!({"op":op,"count":out.len(),"items":out}))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// OpenAPI projection
// ---------------------------------------------------------------------------------------------------------------
fn row_schema(r: &Resource) -> Value {
    json!({"type":"object","required":["id","owner","data","created_at","updated_at"],"properties":{"id":{"type":"string","format":"uuid"},"owner":{"type":"string","format":"uuid"},"data":r.schema(),"created_at":{"type":"string","format":"date-time"},"updated_at":{"type":"string","format":"date-time"}}})
}
fn input_schema(shapes: &BTreeMap<String, String>) -> Value {
    match input_validator("input", shapes) {
        Ok(Some(v)) => v.schema(),
        _ => json!({"type":"object","additionalProperties":false,"maxProperties":0}),
    }
}
fn errors() -> Value {
    json!({"401":{"description":"Sign in required"},"403":{"description":"Policy or role denied"},"404":{"description":"Record, step target or contract not found"},"409":{"description":"GUARD_FAILED (declared condition not met) or UNIQUE_CONFLICT; nothing was committed"},"422":{"description":"INVALID_INPUT with field errors; nothing was committed"}})
}
fn post(operation_id: String, description: &str, body: Value, output: Value, extra: Value) -> Value {
    let mut out = json!({"operationId":operation_id,"description":description,"requestBody":{"required":true,"content":{"application/json":{"schema":body}}},"responses":errors()});
    out["responses"]["200"] = json!({"description":"Committed","content":{"application/json":{"schema":output}}});
    if let Some(map) = extra.as_object() {
        for (k, v) in map {
            out[k] = v.clone();
        }
    }
    out
}
/// Add domain routes to an OpenAPI document produced by `contract::openapi_with_roles`.
pub fn openapi(domain: &Domain, resources: &[Resource], api: &mut Value) {
    let Some(paths) = api["paths"].as_object_mut() else { return };
    let id_param = json!({"name":"id","in":"path","required":true,"schema":{"type":"string","format":"uuid"}});
    for r in resources {
        let d = domain.resource(&r.name);
        let max = domain.batch_max(&r.name);
        let row = row_schema(r);
        let batch_body = json!({"type":"object","additionalProperties":false,"required":["op","items"],"properties":{
            "op":{"enum":["create","update","upsert","delete"]},
            "items":{"type":"array","minItems":1,"maxItems":max,"description":"create/upsert: record data; update: { id, data } with PATCH semantics; delete: { id }"},
            "key":{"type":"array","items":{"type":"string"},"description":"upsert only: a declared unique field set"},
            "on_conflict":{"enum":["update","ignore"],"default":"update"}}});
        paths.insert(format!("/api/{}/batch", r.name), json!({"post": post(format!("{}_batch", r.name), "All-or-nothing batch in one transaction. Every item is validated first; errors are keyed items.<index>.<field>. Not available when the resource declares a before-hook for the operation.", batch_body, json!({"type":"object","required":["op","count","items"],"properties":{"op":{"type":"string"},"count":{"type":"integer"},"items":{"type":"array","items":row},"created":{"type":"array","items":{"type":"boolean"}}}}), json!({"x-brain-batch-max":max,"x-brain-policy":serde_json::to_value(&r.policy).unwrap_or_default()}))}));
        let keys: Vec<Vec<String>> = r.unique_sets().into_iter().map(|(_, set)| set).collect();
        if !keys.is_empty() {
            paths.insert(format!("/api/{}/upsert", r.name), json!({"post": post(format!("{}_upsert", r.name), "Insert, or on a duplicate key update the sent fields (on_conflict: update) or return the existing record (ignore). Race-safe through the unique index.", json!({"type":"object","additionalProperties":false,"required":["key","data"],"properties":{"key":{"enum":keys},"data":r.schema(),"on_conflict":{"enum":["update","ignore"],"default":"update"}}}), json!({"type":"object","required":["record","created"],"properties":{"record":row,"created":{"type":"boolean"}}}), json!({}))}));
        }
        let Some(d) = d else { continue };
        for (name, action) in &d.actions {
            paths.insert(format!("/api/{}/{{id}}/actions/{name}", r.name), json!({"parameters":[id_param],"post": post(format!("{}_action_{name}", r.name), "Atomic action on the locked record: guards (when), assignments (set) and deltas (increment) in one transaction.", input_schema(&action.input), row.clone(), json!({"x-brain-action":action}))}));
        }
        if let Some(t) = &d.transitions {
            for (name, transition) in &t.transitions {
                paths.insert(format!("/api/{}/{{id}}/transitions/{name}", r.name), json!({"parameters":[id_param],"post": post(format!("{}_transition_{name}", r.name), &format!("Move `{}` from {} to {} in one transaction.", t.field, transition.from.join("|"), transition.to), input_schema(&transition.input), row.clone(), json!({"x-brain-transition":transition,"x-brain-state-field":t.field}))}));
            }
        }
    }
    for (name, op) in &domain.operations {
        let mut result = Map::new();
        for step in &op.steps {
            if !op.result.is_empty() && !op.result.contains(&step.name) {
                continue;
            }
            let target = step.kind().map(|(_, t)| t.split('.').next().unwrap_or(t).to_string()).unwrap_or_default();
            if let Some(r) = resources.iter().find(|r| r.name == target) {
                result.insert(step.name.clone(), row_schema(r));
            }
        }
        let output = json!({"type":"object","required":["operation","result"],"properties":{"operation":{"const":name},"result":{"type":"object","additionalProperties":false,"required":result.keys().collect::<Vec<_>>(),"properties":result}}});
        paths.insert(format!("/_brain/operations/{name}"), json!({"post": post(format!("operation_{name}"), "Declared domain operation: every step runs in ONE Postgres transaction and any failure rolls all of them back.", input_schema(&op.input), output, json!({"x-brain-roles":op.roles,"x-brain-steps":op.steps}))}));
    }
}
