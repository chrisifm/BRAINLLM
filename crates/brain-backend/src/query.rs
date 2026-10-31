//! Declared list queries (B-11 filters, B-12 sort, B-13 totals and aggregates, B-15 keyset cursors).
//!
//! A resource declares in `query.jsbrain` which fields can be filtered (and with which operators), sorted, included,
//! aggregated and counted. Undeclared query parameters are rejected. Every predicate is built with
//! `sqlx::QueryBuilder`: only contract-validated identifiers enter SQL text, every request value is a bound parameter.
//! Each declared filter and sort expression gets a btree (or GIN) expression index created by the explicit migration.
use crate::{Resource, ownership::Scope, types::Ty, visibility::Viewer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{Postgres, QueryBuilder, Row};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const MAX_FILTERS: usize = 16;
pub const MAX_IN: usize = 50;
pub const GROUP_LIMIT: i64 = 100;
/// Text comparisons and text indexes use the first 256 characters; equality is rechecked on the full value.
const TEXT_KEY: usize = 256;
const OPS: &[&str] = &["eq", "ne", "gt", "gte", "lt", "lte", "between", "in", "prefix", "iprefix", "exists", "has"];
const RESERVED: &[&str] = &["search", "limit", "offset", "sort", "include", "cursor", "after", "aggregate", "count"];
const TIMESTAMPS: &[&str] = &["created_at", "updated_at"];

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct QueryContract {
    /// Serialize the record `owner` id to every reader (default false: owners, relational owners and roles named
    /// in the read policy only).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expose_owner: bool,
    /// Field (or `created_at`/`updated_at`, or `<ref>.<field>`) to the operators it accepts.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub filter: BTreeMap<String, Vec<String>>,
    /// Sortable fields; `{field, nulls: 'first'|'last'}` overrides the default `last`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<SortEntry>,
    /// Order applied when the request has no `sort` (JSON:API style, `-` = descending). Default `-created_at`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_sort: Vec<String>,
    /// `ref:<resource>` fields whose records may be attached with `?include=`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    /// Named aggregates over the filtered set (`?aggregate=<name>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aggregate: BTreeMap<String, Aggregate>,
    /// Named per-record counts of related records (`?count=<name>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub count: BTreeMap<String, Count>,
}
impl QueryContract {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum SortEntry {
    Name(String),
    Spec(SortSpec),
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SortSpec {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nulls: Option<String>,
}
impl SortEntry {
    fn field(&self) -> &str {
        match self {
            SortEntry::Name(f) => f,
            SortEntry::Spec(s) => &s.field,
        }
    }
    fn nulls_first(&self) -> bool {
        matches!(self, SortEntry::Spec(SortSpec { nulls: Some(n), .. }) if n == "first")
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Aggregate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sum: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub avg: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub min: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub max: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Count {
    pub resource: String,
    pub field: String,
    #[serde(default, rename = "where", skip_serializing_if = "BTreeMap::is_empty")]
    pub filter: BTreeMap<String, Value>,
}

/// SQL shape of a filterable or sortable value.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Int,
    Decimal,
    Date,
    DateTime,
    Text,
    Bool,
    Ref,
    List,
    Stamp,
}
impl Kind {
    fn of(ty: &Ty) -> Option<Kind> {
        Some(match ty {
            Ty::Int => Kind::Int,
            Ty::Decimal { .. } => Kind::Decimal,
            Ty::Date => Kind::Date,
            Ty::DateTime => Kind::DateTime,
            Ty::Str | Ty::Email | Ty::Enum(_) => Kind::Text,
            Ty::Bool => Kind::Bool,
            Ty::Ref(_) => Kind::Ref,
            Ty::ListStr { .. } => Kind::List,
            Ty::Num | Ty::Obj => return None,
        })
    }
    fn ops(self) -> &'static [&'static str] {
        match self {
            Kind::Int | Kind::Decimal | Kind::Date | Kind::DateTime => {
                &["eq", "ne", "gt", "gte", "lt", "lte", "between", "in", "exists"]
            }
            Kind::Stamp => &["eq", "gt", "gte", "lt", "lte", "between"],
            Kind::Text => &["eq", "ne", "in", "prefix", "iprefix", "exists"],
            Kind::Bool => &["eq", "exists"],
            Kind::Ref => &["eq", "ne", "in", "exists"],
            Kind::List => &["has", "exists"],
        }
    }
    fn sortable(self) -> bool {
        !matches!(self, Kind::Ref | Kind::List)
    }
    fn numeric(self) -> bool {
        matches!(self, Kind::Int | Kind::Decimal)
    }
}
/// SQL expression over the unqualified columns of the resource table. `field` is a validated identifier.
fn expr(kind: Kind, field: &str) -> String {
    match kind {
        Kind::Int => format!("(data->'{field}')::numeric"),
        Kind::Decimal => format!("(data->>'{field}')::numeric"),
        Kind::Date | Kind::DateTime => format!("(data->>'{field}') COLLATE \"C\""),
        Kind::Text => format!("left(data->>'{field}',{TEXT_KEY}) COLLATE \"C\""),
        Kind::Bool => format!("(data->>'{field}')"),
        Kind::Ref => crate::relations::column(field),
        Kind::List => format!("(data->'{field}')"),
        Kind::Stamp => field.to_string(),
    }
}
/// Canonical text of a sort key (cursor material). Timestamps keep microseconds so keyset comparison is exact.
fn key_text(kind: Kind, field: &str) -> String {
    match kind {
        Kind::Stamp => format!("to_char({field} AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')"),
        Kind::Int | Kind::Decimal => format!("({})::text", expr(kind, field)),
        _ => expr(kind, field),
    }
}

/// The value kind of a declared field name of `r` (fields or the record timestamps).
fn kind(r: &Resource, field: &str) -> Option<Kind> {
    if TIMESTAMPS.contains(&field) {
        return Some(Kind::Stamp);
    }
    Kind::of(&r.fields.get(field)?.ty()?)
}
fn restricted(r: &Resource, field: &str) -> bool {
    r.fields.get(field).is_some_and(|f| !crate::visibility::unrestricted(f))
}

/// Static validation of one resource's query contract.
pub fn validate(r: &Resource) -> Result<(), String> {
    let q = &r.query;
    let at = |what: String| format!("{}: query {what}", r.name);
    if q.filter.len() > 32 || q.sort.len() > 16 || q.default_sort.len() > 4 || q.include.len() > 8 || q.aggregate.len() > 8 || q.count.len() > 8 {
        return Err(at("declares too many entries; fix: at most 32 filters, 16 sort fields, 4 default sort keys, 8 includes, 8 aggregates and 8 counts".into()));
    }
    for (key, ops) in &q.filter {
        let unique: BTreeSet<_> = ops.iter().collect();
        if ops.is_empty() || unique.len() != ops.len() || ops.iter().any(|op| !OPS.contains(&op.as_str())) {
            return Err(at(format!("filter `{key}` needs distinct operators from {}", OPS.join(", "))));
        }
        let (local, nested) = match key.split_once('.') {
            Some((reference, inner)) => (reference, Some(inner)),
            None => (key.as_str(), None),
        };
        if let Some(inner) = nested {
            if crate::relations::target(r, local).is_none() || !crate::contract::identifier(inner) {
                return Err(at(format!("filter `{key}`: `{local}` must be a ref:<resource> field and `{inner}` a field of the referenced resource")));
            }
            continue;
        }
        let kind = kind(r, local).ok_or_else(|| at(format!("filter `{key}` is not a declared field of a filterable type (num and obj cannot be filtered)")))?;
        if restricted(r, local) {
            return Err(at(format!("filter `{key}` names a field with restricted visibility; fix: remove the filter or make the field public")));
        }
        if let Some(op) = ops.iter().find(|op| !kind.ops().contains(&op.as_str())) {
            return Err(at(format!("filter `{key}` does not support `{op}`; allowed: {}", kind.ops().join(", "))));
        }
    }
    let mut sortable = BTreeSet::new();
    for entry in &q.sort {
        let field = entry.field();
        if !kind(r, field).is_some_and(Kind::sortable) || restricted(r, field) {
            return Err(at(format!("sort `{field}` must be a public int, decimal, date, datetime, str, email, enum or bool field, or created_at/updated_at")));
        }
        if let SortEntry::Spec(SortSpec { nulls: Some(n), .. }) = entry
            && !matches!(n.as_str(), "first" | "last")
        {
            return Err(at(format!("sort `{field}` nulls must be first or last")));
        }
        if !sortable.insert(field) {
            return Err(at(format!("sort `{field}` is declared twice")));
        }
    }
    for key in &q.default_sort {
        let field = key.strip_prefix('-').unwrap_or(key);
        if field != "created_at" && !sortable.contains(field) {
            return Err(at(format!("default_sort `{key}` must name a declared sort field or created_at")));
        }
    }
    for name in &q.include {
        if crate::relations::target(r, name).is_none() {
            return Err(at(format!("include `{name}` must be a ref:<resource> field")));
        }
    }
    for (name, aggregate) in &q.aggregate {
        if !crate::contract::identifier(name) {
            return Err(at(format!("aggregate name `{name}` must be an identifier")));
        }
        if let Some(group) = &aggregate.group
            && (!matches!(kind(r, group), Some(Kind::Text | Kind::Bool | Kind::Ref | Kind::Date | Kind::Int)) || restricted(r, group))
        {
            return Err(at(format!("aggregate `{name}` group `{group}` must be a public str, enum, bool, ref, date or int field")));
        }
        for field in aggregate.sum.iter().chain(&aggregate.avg) {
            if !kind(r, field).is_some_and(Kind::numeric) || restricted(r, field) {
                return Err(at(format!("aggregate `{name}`: sum/avg need a public int or decimal field, got `{field}`")));
            }
        }
        for field in aggregate.min.iter().chain(&aggregate.max) {
            if !matches!(kind(r, field), Some(Kind::Int | Kind::Decimal | Kind::Date | Kind::DateTime)) || restricted(r, field) {
                return Err(at(format!("aggregate `{name}`: min/max need a public int, decimal, date or datetime field, got `{field}`")));
            }
        }
    }
    for name in q.count.keys() {
        if !crate::contract::identifier(name) {
            return Err(at(format!("count name `{name}` must be an identifier")));
        }
    }
    for field in &r.search {
        if restricted(r, field) {
            return Err(format!("{}: search field `{field}` has restricted visibility; fix: remove it from search", r.name));
        }
    }
    Ok(())
}

/// Cross-resource validation: related filters, counts and relational ownership grants.
pub fn validate_graph(r: &Resource, resources: &[Resource]) -> Result<(), String> {
    let find = |name: &str| resources.iter().find(|x| x.name == name);
    for (key, ops) in &r.query.filter {
        let Some((local, inner)) = key.split_once('.') else { continue };
        let target = crate::relations::target(r, local).and_then(|t| find(&t)).ok_or_else(|| format!("{}: query filter `{key}` reaches an unknown resource", r.name))?;
        let declared = target.query.filter.get(inner);
        if !declared.is_some_and(|d| ops.iter().all(|op| d.contains(op))) {
            return Err(format!(
                "{}: query filter `{key}` needs `{inner}` declared with the same operators in {}'s query filter (that declaration creates its index)",
                r.name, target.name
            ));
        }
    }
    for (name, count) in &r.query.count {
        let target = find(&count.resource).ok_or_else(|| format!("{}: count `{name}` names unknown resource `{}`", r.name, count.resource))?;
        if crate::relations::target(target, &count.field).as_deref() != Some(r.name.as_str()) {
            return Err(format!("{}: count `{name}`: `{}.{}` must be a ref:{} field", r.name, count.resource, count.field, r.name));
        }
        for (field, value) in &count.filter {
            let ty = target.fields.get(field).and_then(|f| f.ty());
            if !ty.as_ref().and_then(Kind::of).is_some_and(|k| !matches!(k, Kind::List)) || restricted(target, field) {
                return Err(format!("{}: count `{name}` where `{field}` must be a public scalar field of {}", r.name, target.name));
            }
            if ty.and_then(|t| t.normalize(value, true)).is_none() {
                return Err(format!("{}: count `{name}` where `{field}` value does not match the field type", r.name));
            }
        }
    }
    crate::ownership::validate_graph(r, resources)
}

/// Expression indexes for declared filters and sorts (foreign-key columns are indexed by `relations::install`).
pub(crate) fn indexes(r: &Resource) -> Vec<(String, String)> {
    let mut out = BTreeMap::new();
    let mut add = |method: &str, expression: String| {
        let definition = format!("{method}:{expression}");
        out.insert(r.object_name("_brain_ix_", &definition), format!("USING {method} (({expression}))"));
    };
    let mut fields: Vec<(&str, bool, bool)> = r
        .query
        .filter
        .iter()
        .filter(|(k, _)| !k.contains('.'))
        .map(|(k, ops)| (k.as_str(), ops.iter().any(|o| o == "iprefix"), ops.iter().any(|o| o == "exists")))
        .collect();
    fields.extend(r.query.sort.iter().map(|e| (e.field(), false, false)));
    for (field, iprefix, _) in fields {
        match kind(r, field) {
            Some(Kind::Stamp | Kind::Ref) | None => {}
            Some(Kind::List) => add("gin", expr(Kind::List, field)),
            Some(k) => {
                add("btree", expr(k, field));
                if iprefix {
                    add("btree", format!("lower(left(data->>'{field}',{TEXT_KEY})) COLLATE \"C\""));
                }
            }
        }
    }
    if r.query.sort.iter().any(|e| e.field() == "updated_at") || r.query.filter.contains_key("updated_at") {
        add("btree", "updated_at".into());
    }
    out.into_iter().collect()
}
pub(crate) async fn install_indexes(tx: &mut sqlx::Transaction<'_, Postgres>, r: &Resource) -> Result<(), String> {
    for (name, definition) in indexes(r) {
        // Audited SQL: the name is a hash and the expression is built from validated identifiers only.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE INDEX IF NOT EXISTS {name} ON _brain_res_{} {definition}", r.name)))
            .execute(&mut **tx)
            .await
            .map_err(|error| {
                crate::db::safe(error);
                format!("{}: a declared query index could not be built; existing values may not match their field types; nothing was changed", r.name)
            })?;
    }
    Ok(())
}

/// One bound operand: a value, or a time relative to the database clock (`now`, `now-7d`).
#[derive(Clone, Debug, PartialEq)]
enum Operand {
    Value(String),
    Now(String),
}
#[derive(Clone, Debug)]
struct Filter {
    /// `Some(ref field)` for a related filter evaluated on the referenced resource.
    via: Option<(String, String)>,
    field: String,
    kind: Kind,
    op: String,
    values: Vec<Operand>,
}
#[derive(Clone, Debug)]
struct SortKey {
    field: String,
    kind: Kind,
    desc: bool,
    nulls_first: bool,
}
#[derive(Clone, Debug, Default)]
pub struct ListRequest {
    filters: Vec<Filter>,
    sort: Option<Vec<SortKey>>,
    pub search: String,
    pub limit: i64,
    pub offset: i64,
    cursor: Option<(Vec<Option<String>>, Uuid)>,
    pub include: Vec<String>,
    aggregate: Vec<String>,
    count: Vec<String>,
}
pub type Problems = BTreeMap<String, String>;

fn relative(s: &str) -> Option<String> {
    let rest = s.strip_prefix("now")?;
    if rest.is_empty() {
        return Some("0 seconds".into());
    }
    let (sign, rest) = match rest.as_bytes()[0] {
        b'-' => ("-", &rest[1..]),
        b'+' => ("", &rest[1..]),
        _ => return None,
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let n: u32 = rest[..digits].parse().ok().filter(|n| (1..=36500).contains(n))?;
    let unit = match &rest[digits..] {
        "m" => "minutes",
        "h" => "hours",
        "d" => "days",
        "w" => "weeks",
        _ => return None,
    };
    Some(format!("{sign}{n} {unit}"))
}
fn operand(r: &Resource, field: &str, kind: Kind, op: &str, raw: &str) -> Option<Operand> {
    if raw.len() > TEXT_KEY {
        return None;
    }
    if matches!(kind, Kind::Date | Kind::DateTime | Kind::Stamp) && raw.starts_with("now") {
        return relative(raw).map(Operand::Now);
    }
    let ty = r.fields.get(field).and_then(|f| f.ty());
    let value = match (kind, op) {
        (_, "exists") => matches!(raw, "true" | "false").then(|| raw.to_string())?,
        (Kind::Text, "prefix" | "iprefix") | (Kind::List, _) => (!raw.is_empty()).then(|| raw.to_string())?,
        (Kind::Int, _) => ty?.normalize(&json!(raw.parse::<i64>().ok()?), true)?.to_string(),
        (Kind::Bool, _) => matches!(raw, "true" | "false").then(|| raw.to_string())?,
        (Kind::Ref, _) => Uuid::parse_str(raw).ok()?.hyphenated().to_string(),
        (Kind::Stamp, _) => crate::types::datetime(raw)?,
        _ => ty?.normalize(&json!(raw), true)?.as_str()?.to_string(),
    };
    Some(Operand::Value(value))
}
fn split(raw: &str) -> Vec<&str> {
    raw.split(',').collect()
}

/// Parses raw query pairs against the contract. Every undeclared key, operator or malformed value is a problem.
pub fn parse(r: &Resource, resources: &BTreeMap<String, Resource>, pairs: &[(String, String)]) -> Result<ListRequest, Problems> {
    let mut problems = Problems::new();
    let mut req = ListRequest { limit: 25, ..Default::default() };
    let mut seen = BTreeSet::new();
    for (key, raw) in pairs {
        if !seen.insert(key.as_str()) {
            problems.insert(key.clone(), "Repeated parameter".into());
            continue;
        }
        if raw.len() > 4096 {
            problems.insert(key.clone(), "Value too long".into());
            continue;
        }
        let mut bad = |m: &str| {
            problems.insert(key.clone(), m.to_string());
        };
        match key.as_str() {
            "search" => {
                if raw.len() > 256 || raw.split_whitespace().count() > 16 {
                    bad("search must be at most 256 bytes and 16 words");
                }
                req.search = raw.clone();
            }
            "limit" => match raw.parse() {
                Ok(n) if (1..=100).contains(&n) => req.limit = n,
                _ => bad("limit must be 1..100"),
            },
            "offset" => match raw.parse() {
                Ok(n) if (0..=10000).contains(&n) => req.offset = n,
                _ => bad("offset must be 0..10000; use cursor for deeper pages"),
            },
            "sort" => {
                let mut keys = Vec::new();
                for item in split(raw) {
                    let (desc, field) = item.strip_prefix('-').map_or((false, item), |f| (true, f));
                    let declared = r.query.sort.iter().find(|e| e.field() == field);
                    match (declared, kind(r, field)) {
                        (Some(entry), Some(k)) => keys.push(SortKey { field: field.into(), kind: k, desc, nulls_first: entry.nulls_first() }),
                        _ if field == "created_at" => keys.push(SortKey { field: field.into(), kind: Kind::Stamp, desc, nulls_first: false }),
                        _ => {
                            bad(&format!("`{field}` is not sortable; declared: {}", sort_names(r)));
                        }
                    }
                }
                let unique: BTreeSet<_> = keys.iter().map(|k| k.field.clone()).collect();
                if keys.is_empty() || keys.len() > 4 || unique.len() != keys.len() {
                    bad("sort takes 1..4 distinct fields, e.g. sort=-price,title");
                }
                req.sort = Some(keys);
            }
            // `after` is an alias of `cursor` (keyset pagination by the previous page's next_cursor).
            "cursor" | "after" if req.cursor.is_some() => bad("Use either cursor or after, not both"),
            "cursor" | "after" => match decode_cursor(raw) {
                Some(c) => req.cursor = Some(c),
                None => bad("Invalid cursor; use next_cursor from the previous page unchanged"),
            },
            "include" => {
                for name in split(raw) {
                    if !r.query.include.iter().any(|x| x == name) {
                        bad(&format!("`{name}` is not includable; declared: {}", r.query.include.join(", ")));
                    }
                }
                req.include = split(raw).into_iter().map(str::to_string).collect();
            }
            "aggregate" | "count" => {
                let names: Vec<String> = split(raw).into_iter().map(str::to_string).collect();
                let declared: Vec<&String> = if key == "aggregate" { r.query.aggregate.keys().collect() } else { r.query.count.keys().collect() };
                if let Some(name) = names.iter().find(|n| !declared.contains(n)) {
                    bad(&format!("`{name}` is not a declared {key}"));
                }
                if key == "aggregate" { req.aggregate = names } else { req.count = names }
            }
            _ => {
                let (field, op) = match key.split_once('[') {
                    Some((f, rest)) => match rest.strip_suffix(']') {
                        Some(op) => (f, op),
                        None => {
                            bad("Malformed parameter; use field=value or field[op]=value");
                            continue;
                        }
                    },
                    None => (key.as_str(), "eq"),
                };
                let Some(ops) = r.query.filter.get(field).filter(|ops| ops.iter().any(|o| o == op)) else {
                    let declared = r.query.filter.get(field).map(|ops| ops.join(", "));
                    bad(&match declared {
                        Some(ops) => format!("Operator `{op}` is not declared for `{field}`; declared: {ops}"),
                        None if RESERVED.contains(&field) => "Malformed parameter".to_string(),
                        None => format!("Unknown query parameter; filterable: {}", r.query.filter.keys().cloned().collect::<Vec<_>>().join(", ")),
                    });
                    continue;
                };
                let _ = ops;
                let (via, target, name) = match field.split_once('.') {
                    Some((local, inner)) => {
                        let Some(t) = crate::relations::target(r, local).and_then(|t| resources.get(&t)) else { continue };
                        (Some((local.to_string(), t.name.clone())), t, inner)
                    }
                    None => (None, r, field),
                };
                let Some(k) = kind(target, name) else { continue };
                let raws = match op {
                    "in" => split(raw),
                    "between" => split(raw),
                    _ => vec![raw.as_str()],
                };
                if op == "in" && !(1..=MAX_IN).contains(&raws.len()) || op == "between" && raws.len() != 2 {
                    bad(if op == "in" { "in takes 1..50 comma-separated values" } else { "between takes two comma-separated values: low,high" });
                    continue;
                }
                let values: Option<Vec<Operand>> = raws.iter().map(|v| operand(target, name, k, op, v)).collect();
                match values {
                    Some(values) => req.filters.push(Filter { via, field: name.to_string(), kind: k, op: op.to_string(), values }),
                    None => bad(&format!("Invalid value for `{field}` ({op}); expected {}", expected(target, name, k))),
                }
            }
        }
    }
    if req.filters.len() > MAX_FILTERS {
        problems.insert("query".into(), format!("At most {MAX_FILTERS} filters per request"));
    }
    if req.cursor.is_some() && req.offset != 0 {
        problems.insert("cursor".into(), "cursor and offset cannot be combined".into());
    }
    if req.cursor.is_some() && req.sort.is_none() && search_ranked(r, &req) {
        problems.insert("cursor".into(), "Ranked search results are offset-paged; add sort=... to page with a cursor".into());
    }
    if problems.is_empty() { Ok(req) } else { Err(problems) }
}
fn sort_names(r: &Resource) -> String {
    let mut names: Vec<&str> = r.query.sort.iter().map(SortEntry::field).collect();
    names.push("created_at");
    names.join(", ")
}
fn expected(r: &Resource, field: &str, kind: Kind) -> String {
    match kind {
        Kind::Stamp => "an RFC 3339 date-time or now, now-7d, now+1h".into(),
        Kind::Date | Kind::DateTime => format!("{} or now, now-7d, now+1h", r.fields[field].ty().map(|t| t.expected()).unwrap_or_default()),
        Kind::Bool => "true or false".into(),
        Kind::Ref => "a record UUID".into(),
        _ => r.fields.get(field).and_then(|f| f.ty()).map(|t| t.expected()).unwrap_or_default(),
    }
}
fn search_ranked(r: &Resource, req: &ListRequest) -> bool {
    !r.search.is_empty() && req.search.trim().chars().count() >= 3
}

fn keys(r: &Resource, req: &ListRequest) -> Vec<SortKey> {
    if let Some(keys) = &req.sort {
        return keys.clone();
    }
    let default: Vec<String> = if r.query.default_sort.is_empty() { vec!["-created_at".into()] } else { r.query.default_sort.clone() };
    default
        .iter()
        .filter_map(|k| {
            let (desc, field) = k.strip_prefix('-').map_or((false, k.as_str()), |f| (true, f));
            let entry = r.query.sort.iter().find(|e| e.field() == field);
            Some(SortKey { field: field.into(), kind: kind(r, field)?, desc, nulls_first: entry.is_some_and(SortEntry::nulls_first) })
        })
        .collect()
}
fn signature(keys: &[SortKey]) -> String {
    keys.iter()
        .map(|k| format!("{}{}:{}", if k.desc { "-" } else { "" }, k.field, if k.nulls_first { "first" } else { "last" }))
        .collect::<Vec<_>>()
        .join(",")
}
fn encode_cursor(signature: &str, values: &[Option<String>], id: Uuid) -> String {
    json!({"s": signature, "k": values, "id": id}).to_string().bytes().map(|b| format!("{b:02x}")).collect()
}
fn decode_cursor(raw: &str) -> Option<(Vec<Option<String>>, Uuid)> {
    if raw.len() > 4096 || !raw.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..raw.len()).step_by(2).map(|i| u8::from_str_radix(raw.get(i..i + 2)?, 16).ok()).collect();
    let value: Value = serde_json::from_slice(&bytes?).ok()?;
    let mut keys: Vec<Option<String>> = value["k"].as_array()?.iter().map(|v| v.as_str().map(str::to_string)).collect();
    // The signature travels as the last key slot so a cursor from another ordering is rejected on use.
    keys.push(Some(value["s"].as_str()?.to_string()));
    Some((keys, Uuid::parse_str(value["id"].as_str()?).ok()?))
}

/// Why a list could not be produced.
#[derive(Debug)]
pub enum ListError {
    Invalid(Problems),
    Denied(String),
    Database(sqlx::Error),
}
impl From<sqlx::Error> for ListError {
    fn from(e: sqlx::Error) -> Self {
        ListError::Database(e)
    }
}

fn push_value(q: &mut QueryBuilder<Postgres>, kind: Kind, value: &Operand) {
    match value {
        Operand::Now(interval) => {
            let interval = interval.clone();
            match kind {
                Kind::Stamp => {
                    q.push("(now()+CAST(").push_bind(interval).push(" AS interval))");
                }
                Kind::Date => {
                    q.push("(to_char((now() AT TIME ZONE 'UTC')+CAST(").push_bind(interval).push(" AS interval),'YYYY-MM-DD') COLLATE \"C\")");
                }
                _ => {
                    q.push("(to_char((now() AT TIME ZONE 'UTC')+CAST(")
                        .push_bind(interval)
                        .push(" AS interval),'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') COLLATE \"C\")");
                }
            }
        }
        Operand::Value(v) => {
            let v = v.clone();
            match kind {
                Kind::Int | Kind::Decimal => {
                    q.push("CAST(").push_bind(v).push(" AS numeric)");
                }
                Kind::Stamp => {
                    q.push("CAST(").push_bind(v).push(" AS timestamptz)");
                }
                Kind::Ref => {
                    q.push_bind(Uuid::parse_str(&v).unwrap_or_default());
                }
                Kind::Text => {
                    q.push("left(").push_bind(v).push(format!(",{TEXT_KEY})"));
                }
                _ => {
                    q.push_bind(v);
                }
            }
        }
    }
}
fn escape_like(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}
fn push_predicate(q: &mut QueryBuilder<Postgres>, f: &Filter) {
    let e = expr(f.kind, &f.field);
    let field = &f.field;
    let first = &f.values[0];
    let full = |q: &mut QueryBuilder<Postgres>, v: &Operand| {
        if let (Kind::Text, Operand::Value(v)) = (f.kind, v) {
            q.push(format!(" AND (data->>'{field}')=")).push_bind(v.clone());
        }
    };
    match f.op.as_str() {
        "exists" => {
            let yes = first == &Operand::Value("true".into());
            q.push(if yes { "(" } else { "NOT (" });
            if f.kind == Kind::Ref {
                q.push(&e).push(" IS NOT NULL)");
            } else {
                q.push(format!("data ? '{field}')"));
            }
        }
        "has" => {
            if let Operand::Value(v) = first {
                q.push(format!("({e} ? ")).push_bind(v.clone()).push(")");
            }
        }
        "prefix" | "iprefix" => {
            if let Operand::Value(v) = first {
                if f.op == "prefix" {
                    q.push(format!("({e} LIKE ")).push_bind(escape_like(v)).push(" ESCAPE '\\')");
                } else {
                    q.push(format!("(lower(left(data->>'{field}',{TEXT_KEY})) COLLATE \"C\" LIKE "))
                        .push_bind(escape_like(&v.to_lowercase()))
                        .push(" ESCAPE '\\')");
                }
            }
        }
        "in" => {
            q.push("(");
            for (i, v) in f.values.iter().enumerate() {
                if i > 0 {
                    q.push(" OR ");
                }
                q.push(format!("({e}="));
                push_value(q, f.kind, v);
                full(q, v);
                q.push(")");
            }
            q.push(")");
        }
        "between" => {
            q.push(format!("({e} BETWEEN "));
            push_value(q, f.kind, &f.values[0]);
            q.push(" AND ");
            push_value(q, f.kind, &f.values[1]);
            q.push(")");
        }
        op => {
            let sql = match op {
                "eq" => "=",
                "ne" => "<>",
                "gt" => ">",
                "gte" => ">=",
                "lt" => "<",
                _ => "<=",
            };
            if op == "ne" && f.kind == Kind::Text {
                q.push(format!("((data->>'{field}')<>"));
                if let Operand::Value(v) = first {
                    q.push_bind(v.clone());
                }
                q.push(")");
                return;
            }
            q.push(format!("({e}{sql}"));
            push_value(q, f.kind, first);
            if op == "eq" {
                full(q, first);
            }
            q.push(")");
        }
    }
}

struct Ctx<'a> {
    r: &'a Resource,
    resources: &'a BTreeMap<String, Resource>,
    scope: &'a Scope,
    related: BTreeMap<String, Scope>,
    req: &'a ListRequest,
}
impl Ctx<'_> {
    /// WHERE body over the alias-free columns of the main table: scope, filters, related filters and search.
    fn push_where(&self, q: &mut QueryBuilder<Postgres>) {
        self.scope.push(q, self.r, self.resources);
        for f in &self.req.filters {
            q.push(" AND ");
            match &f.via {
                None => push_predicate(q, f),
                Some((local, target)) => {
                    let t = &self.resources[target];
                    q.push(format!("{} IN (SELECT id FROM _brain_res_{} WHERE ", crate::relations::column(local), t.name));
                    self.related[target].push(q, t, self.resources);
                    q.push(" AND ");
                    push_predicate(q, f);
                    q.push(")");
                }
            }
        }
        let text = self.req.search.trim();
        if text.is_empty() {
            return;
        }
        let table = format!("_brain_res_{}", self.r.name);
        if self.r.search.is_empty() {
            q.push(" AND false");
        } else if text.chars().count() < 3 {
            q.push(" AND search_text LIKE public.unaccent('public.unaccent'::regdictionary,lower(")
                .push_bind(escape_like(&text.to_lowercase()))
                .push(")) ESCAPE '\\'");
        } else {
            // Bounded candidate branches, each carrying the same authorization scope (mirrors search.rs).
            q.push(format!(" AND id IN (SELECT id FROM (SELECT id FROM {table} WHERE "));
            self.scope.push(q, self.r, self.resources);
            q.push(" AND search_vector @@ websearch_to_tsquery('simple',public.unaccent('public.unaccent'::regdictionary,")
                .push_bind(text.to_string())
                .push(")) ORDER BY ts_rank_cd(search_vector,websearch_to_tsquery('simple',public.unaccent('public.unaccent'::regdictionary,")
                .push_bind(text.to_string())
                .push(format!("))) DESC,id LIMIT 500) f UNION SELECT id FROM (SELECT id FROM {table} WHERE "));
            self.scope.push(q, self.r, self.resources);
            q.push(" AND public.unaccent('public.unaccent'::regdictionary,")
                .push_bind(text.to_lowercase())
                .push(") OPERATOR(public.<%) search_text ORDER BY id LIMIT 500) s)");
        }
    }
}

/// Runs a declared list request: page items, `total`, optional `next_cursor`, aggregates and counts.
pub async fn list(
    pool: &sqlx::PgPool,
    resources: &BTreeMap<String, Resource>,
    r: &Resource,
    req: &ListRequest,
    viewer: Viewer<'_>,
) -> Result<Value, ListError> {
    let scope = crate::ownership::read_scope(r, resources, viewer).ok_or_else(|| ListError::Denied(r.name.clone()))?;
    let mut related = BTreeMap::new();
    for f in &req.filters {
        if let Some((local, target)) = &f.via {
            let t = &resources[target];
            let s = crate::ownership::read_scope(t, resources, viewer).ok_or_else(|| ListError::Denied(local.clone()))?;
            related.insert(target.clone(), s);
        }
    }
    let ctx = Ctx { r, resources, scope: &scope, related, req };
    let keys = keys(r, req);
    let sig = signature(&keys);
    let ranked = req.sort.is_none() && search_ranked(r, req);
    let text = req.search.trim().to_string();
    let mut tx = pool.begin().await?;
    let timeout = if text.is_empty() { "SET LOCAL statement_timeout='5s';" } else { "SET LOCAL statement_timeout='1s';" };
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{timeout} SET LOCAL lock_timeout='500ms'; SET LOCAL pg_trgm.word_similarity_threshold='0.45';"))).execute(&mut *tx).await?;

    // Page.
    let mut q = QueryBuilder::<Postgres>::new(concat!("SELECT ", crate::record_columns!()));
    for (i, k) in keys.iter().enumerate() {
        q.push(format!(",{} AS _k{i}", key_text(k.kind, &k.field)));
    }
    let counts: Vec<(&String, &Count)> = req.count.iter().filter_map(|n| r.query.count.get_key_value(n)).collect();
    for (i, (name, count)) in counts.iter().enumerate() {
        let target = &resources[&count.resource];
        let s = crate::ownership::read_scope(target, resources, viewer).ok_or_else(|| ListError::Denied((*name).clone()))?;
        q.push(format!(",(SELECT count(*) FROM _brain_res_{} WHERE {}=t.id AND ", target.name, crate::relations::column(&count.field)));
        s.push(&mut q, target, resources);
        for (field, value) in &count.filter {
            let ty = target.fields[field].ty();
            let k = ty.as_ref().and_then(Kind::of).unwrap_or(Kind::Text);
            let v = ty.and_then(|t| t.normalize(value, true)).unwrap_or(Value::Null);
            let v = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            q.push(" AND ");
            push_predicate(&mut q, &Filter { via: None, field: field.clone(), kind: k, op: "eq".into(), values: vec![Operand::Value(v)] });
        }
        q.push(format!(") AS _c{i}"));
    }
    let relational = relational_read(r, viewer);
    if let Some(rel) = &relational {
        q.push(",(");
        rel.push(&mut q, r, resources);
        q.push(") AS _rel");
    }
    q.push(format!(" FROM _brain_res_{} t WHERE ", r.name));
    ctx.push_where(&mut q);
    if let Some((values, id)) = &req.cursor {
        if values.last().and_then(|s| s.as_deref()) != Some(sig.as_str()) || values.len() != keys.len() + 1 {
            return Err(ListError::Invalid(Problems::from([("cursor".into(), "The cursor belongs to a different sort; restart from the first page".into())])));
        }
        q.push(" AND ");
        push_keyset(&mut q, &keys, &values[..keys.len()], *id);
    }
    q.push(" ORDER BY ");
    if ranked {
        q.push("ts_rank_cd(search_vector,websearch_to_tsquery('simple',public.unaccent('public.unaccent'::regdictionary,")
            .push_bind(text.clone())
            .push("))) DESC, public.word_similarity(public.unaccent('public.unaccent'::regdictionary,")
            .push_bind(text.to_lowercase())
            .push("),search_text) DESC,id");
    } else {
        for k in &keys {
            q.push(format!("{} {} NULLS {},", expr(k.kind, &k.field), if k.desc { "DESC" } else { "ASC" }, if k.nulls_first { "FIRST" } else { "LAST" }));
        }
        q.push(if keys.last().is_some_and(|k| k.desc) { "id DESC" } else { "id ASC" });
    }
    q.push(" LIMIT ").push_bind(req.limit);
    if req.cursor.is_none() {
        q.push(" OFFSET ").push_bind(req.offset);
    }
    let rows = q.build().fetch_all(&mut *tx).await?;
    let mut items = Vec::with_capacity(rows.len());
    let mut last_keys = None;
    for row in rows {
        let key_values: Vec<Option<String>> = (0..keys.len()).map(|i| row.get::<Option<String>, _>(format!("_k{i}").as_str())).collect();
        let row_counts: Vec<i64> = (0..counts.len()).map(|i| row.get::<i64, _>(format!("_c{i}").as_str())).collect();
        let rel = relational.is_some() && row.get::<Option<bool>, _>("_rel").unwrap_or(false);
        let mut record = crate::db::record(row);
        last_keys = Some((key_values, record["id"].as_str().and_then(|s| Uuid::parse_str(s).ok())));
        crate::visibility::project(r, &mut record, viewer, rel);
        if !counts.is_empty() {
            record["counts"] = Value::Object(counts.iter().zip(row_counts).map(|((n, _), c)| ((*n).clone(), json!(c))).collect());
        }
        items.push(record);
    }

    // Total over the same WHERE (without the cursor).
    let mut q = QueryBuilder::<Postgres>::new(format!("SELECT count(*) FROM _brain_res_{} t WHERE ", r.name));
    ctx.push_where(&mut q);
    let total: i64 = q.build_query_scalar().fetch_one(&mut *tx).await?;

    let mut aggregates = Map::new();
    for name in &req.aggregate {
        let a = &r.query.aggregate[name];
        aggregates.insert(name.clone(), aggregate(&mut tx, &ctx, a).await?);
    }
    tx.commit().await?;

    let mut out = json!({"items": items, "limit": req.limit, "offset": req.offset, "total": total});
    if !ranked
        && items_len(&out) as i64 == req.limit
        && let Some((values, Some(id))) = last_keys
    {
        out["next_cursor"] = json!(encode_cursor(&sig, &values, id));
    }
    if !aggregates.is_empty() {
        out["aggregates"] = Value::Object(aggregates);
    }
    Ok(out)
}
fn items_len(v: &Value) -> usize {
    v["items"].as_array().map_or(0, Vec::len)
}
/// Rows strictly after `values`/`id` in the declared order (null-aware keyset predicate).
fn push_keyset(q: &mut QueryBuilder<Postgres>, keys: &[SortKey], values: &[Option<String>], id: Uuid) {
    q.push("(false");
    for i in 0..=keys.len() {
        q.push(" OR (true");
        for j in 0..i {
            let e = expr(keys[j].kind, &keys[j].field);
            match &values[j] {
                None => {
                    q.push(format!(" AND {e} IS NULL"));
                }
                Some(v) => {
                    q.push(format!(" AND {e}="));
                    push_key(q, keys[j].kind, v);
                }
            }
        }
        if i == keys.len() {
            let desc = keys.last().is_some_and(|k| k.desc);
            q.push(if desc { " AND id<" } else { " AND id>" }).push_bind(id);
        } else {
            let k = &keys[i];
            let e = expr(k.kind, &k.field);
            match &values[i] {
                None if k.nulls_first => {
                    q.push(format!(" AND {e} IS NOT NULL"));
                }
                None => {
                    q.push(" AND false");
                }
                Some(v) => {
                    q.push(format!(" AND ({e}{}", if k.desc { "<" } else { ">" }));
                    push_key(q, k.kind, v);
                    if !k.nulls_first {
                        q.push(format!(" OR {e} IS NULL"));
                    }
                    q.push(")");
                }
            }
        }
        q.push(")");
    }
    q.push(")");
}
fn push_key(q: &mut QueryBuilder<Postgres>, kind: Kind, v: &str) {
    match kind {
        Kind::Int | Kind::Decimal => {
            q.push("CAST(").push_bind(v.to_string()).push(" AS numeric)");
        }
        Kind::Stamp => {
            q.push("CAST(").push_bind(v.to_string()).push(" AS timestamptz)");
        }
        _ => {
            q.push_bind(v.to_string());
        }
    }
}
/// Relational read ownership of each row, used only for `owner` field visibility.
fn relational_read(r: &Resource, viewer: Viewer<'_>) -> Option<Scope> {
    let owner_visible = r.fields.values().any(|f| f.visible.as_ref().is_some_and(|v| v.iter().any(|g| g == "owner")));
    let user = viewer.id?;
    let paths: Vec<Vec<String>> = r.policy.read.iter().filter_map(|g| crate::ownership::path(g)).collect();
    (owner_visible && !paths.is_empty()).then_some(Scope::Owned { user, direct: false, paths })
}

async fn aggregate(tx: &mut sqlx::Transaction<'_, Postgres>, ctx: &Ctx<'_>, a: &Aggregate) -> Result<Value, sqlx::Error> {
    let r = ctx.r;
    let mut columns: Vec<(String, String, String)> = Vec::new();
    for f in &a.sum {
        columns.push(("sum".into(), f.clone(), format!("sum({})::text", expr(kind(r, f).unwrap_or(Kind::Int), f))));
    }
    for f in &a.avg {
        let scale = match r.fields[f].ty() {
            Some(Ty::Decimal { scale, .. }) => scale as i32 + 2,
            _ => 2,
        };
        columns.push(("avg".into(), f.clone(), format!("round(avg({}),{scale})::text", expr(kind(r, f).unwrap_or(Kind::Int), f))));
    }
    for (op, list) in [("min", &a.min), ("max", &a.max)] {
        for f in list {
            let k = kind(r, f).unwrap_or(Kind::Int);
            columns.push((op.into(), f.clone(), format!("{op}({})::text", expr(k, f))));
        }
    }
    let select: String = columns.iter().enumerate().map(|(i, (_, _, sql))| format!(",{sql} AS _a{i}")).collect();
    let group = a.group.as_ref().map(|g| {
        let k = kind(r, g).unwrap_or(Kind::Text);
        let e = match k {
            Kind::Ref => format!("{}::text", crate::relations::column(g)),
            Kind::Text => format!("(data->>'{g}')"),
            _ => format!("({})::text", expr(k, g)),
        };
        (k, e)
    });
    let mut q = QueryBuilder::<Postgres>::new("SELECT count(*) AS _n");
    q.push(&select);
    if let Some((_, e)) = &group {
        q.push(format!(",{e} AS _g"));
    }
    q.push(format!(" FROM _brain_res_{} t WHERE ", r.name));
    ctx.push_where(&mut q);
    if let Some((_, e)) = &group {
        q.push(format!(" GROUP BY {e} ORDER BY count(*) DESC,{e} NULLS LAST LIMIT {GROUP_LIMIT}"));
    }
    let rows = q.build().fetch_all(&mut **tx).await?;
    let render = |row: &sqlx::postgres::PgRow| {
        let mut out = Map::new();
        out.insert("count".into(), json!(row.get::<i64, _>("_n")));
        for (i, (op, field, _)) in columns.iter().enumerate() {
            let text: Option<String> = row.get(format!("_a{i}").as_str());
            let value = match (op.as_str(), kind(r, field), text) {
                (_, _, None) => Value::Null,
                ("min" | "max", Some(Kind::Int), Some(t)) => t.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(t)),
                (_, _, Some(t)) => json!(t),
            };
            out.entry(op.clone()).or_insert_with(|| json!({}))[field.as_str()] = value;
        }
        out
    };
    match group {
        None => Ok(rows.first().map(|row| Value::Object(render(row))).unwrap_or(json!({"count": 0}))),
        Some((k, _)) => Ok(Value::Array(
            rows.iter()
                .map(|row| {
                    let mut out = render(row);
                    let key: Option<String> = row.get("_g");
                    let key = match (k, key) {
                        (_, None) => Value::Null,
                        (Kind::Int, Some(t)) => t.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(t)),
                        (Kind::Bool, Some(t)) => json!(t == "true"),
                        (_, Some(t)) => json!(t),
                    };
                    out.insert("key".into(), key);
                    Value::Object(out)
                })
                .collect(),
        )),
    }
}

/// OpenAPI query parameters for the list route.
pub fn openapi_parameters(r: &Resource) -> Vec<Value> {
    let mut params = vec![
        json!({"name":"search","in":"query","schema":{"type":"string","maxLength":256}}),
        json!({"name":"limit","in":"query","schema":{"type":"integer","minimum":1,"maximum":100,"default":25}}),
        json!({"name":"offset","in":"query","schema":{"type":"integer","minimum":0,"maximum":10000,"default":0}}),
        json!({"name":"cursor","in":"query","description":"next_cursor of the previous page (keyset pagination)","schema":{"type":"string"}}),
        json!({"name":"after","in":"query","description":"Alias of cursor","schema":{"type":"string"}}),
        json!({"name":"sort","in":"query","description":format!("Comma-separated, '-' for descending: {}", sort_names(r)),"schema":{"type":"string"}}),
    ];
    for (field, ops) in &r.query.filter {
        for op in ops {
            let name = if op == "eq" { field.clone() } else { format!("{field}[{op}]") };
            params.push(json!({"name": name, "in":"query", "schema":{"type":"string"}}));
        }
    }
    for (key, list) in [("include", r.query.include.clone()), ("aggregate", r.query.aggregate.keys().cloned().collect()), ("count", r.query.count.keys().cloned().collect())] {
        if !list.is_empty() {
            params.push(json!({"name": key, "in":"query", "description": format!("Comma-separated: {}", list.join(", ")), "schema":{"type":"string"}}));
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_times_bind_fixed_intervals() {
        assert_eq!(relative("now").as_deref(), Some("0 seconds"));
        assert_eq!(relative("now-7d").as_deref(), Some("-7 days"));
        assert_eq!(relative("now+12h").as_deref(), Some("12 hours"));
        assert_eq!(relative("now-0d"), None);
        assert_eq!(relative("now-7y"), None);
        assert_eq!(relative("now-7d;drop"), None);
    }
    #[test]
    fn cursors_round_trip_with_their_sort_signature() {
        let id = Uuid::nil();
        let raw = encode_cursor("-price:last", &[Some("10.50".into())], id);
        let (keys, decoded) = decode_cursor(&raw).unwrap();
        assert_eq!(decoded, id);
        assert_eq!(keys, vec![Some("10.50".into()), Some("-price:last".into())]);
        assert!(decode_cursor("zz").is_none());
    }
    #[test]
    fn like_prefixes_escape_wildcards() {
        assert_eq!(escape_like("a_%\\"), "a\\_\\%\\\\%");
    }
}
