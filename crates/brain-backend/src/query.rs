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
/// Mean Earth radius (IUGG), in kilometers.
const EARTH_KM: f64 = 6371.0088;
/// Half the Earth's circumference: a `near` radius cannot usefully go past it.
const MAX_NEAR_KM: f64 = 20040.0;
/// Degrees added to an index box so rounding cannot drop a point that the exact distance check would keep.
const GEO_MARGIN: f64 = 1e-6;
/// Text comparisons and text indexes use the first 256 characters; equality is rechecked on the full value.
const TEXT_KEY: usize = 256;
/// Inline copy of `data` that filters, sorts, facets and their indexes read: every key, with each string cut at
/// [`TEXT_KEY`] characters. A large `data` lives out of line and compressed, so a predicate over it decompressed the whole
/// document of every row it visited (a cold list read the entire TOAST table); the copy stays small and in the row.
/// Text comparisons already use `left(...,TEXT_KEY)`, so they answer the same; a value that fills the key is rechecked
/// against `data` for that row only.
const KEYS: &str = "query_keys";
/// Builds the copy from `data` (a stored generated column). The version is part of the name: a different body is a new
/// function and a new column, never a silent change of stored values.
const KEYS_FUNCTION: &str = "public._brain_query_keys_v1";
pub const OPS: &[&str] = &["eq", "ne", "gt", "gte", "lt", "lte", "between", "in", "prefix", "iprefix", "exists", "has", "near", "bbox"];
const RESERVED: &[&str] = &["search", "limit", "offset", "sort", "include", "cursor", "after", "aggregate", "count", "fields", "owner"];
const TIMESTAMPS: &[&str] = &["created_at", "updated_at"];
/// Typo matches kept for one search; reaching it marks `total` approximate.
const FUZZY_CANDIDATES: i64 = 500;
/// Budget of the typo branch: past it the search answers without typo matches and says `degraded`.
const FUZZY_BUDGET: &str = "400ms";
/// Deadline of every other statement of a search request.
const SEARCH_DEADLINE: &str = "1s";

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
    /// Named alternatives: `?<name>.<field>[op]=value` for each branch sent; at least one must hold (`docs/backend/filters.md`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub or: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// `ref:<resource>` fields whose records may be attached with `?include=`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    /// Named aggregates over the filtered set (`?aggregate=<name>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aggregate: BTreeMap<String, Aggregate>,
    /// Fields whose values can be counted over the listed set (`?facets=<field>,...`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<String>,
    /// Named per-record counts of related records (`?count=<name>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub count: BTreeMap<String, Count>,
    /// Text-search language and field weights (`docs/backend/search.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<crate::search_config::TextSearch>,
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
    Geo,
    Stamp,
}
/// Filter operators of a parsed field type, as the query parser enforces them (`None`: it cannot be filtered).
pub fn filter_operators(ty: &Ty) -> Option<&'static [&'static str]> {
    Kind::of(ty).map(Kind::ops)
}
/// Filter and sort abilities of `created_at` and `updated_at`.
pub fn timestamp_operators() -> &'static [&'static str] {
    Kind::Stamp.ops()
}
/// Whether lists can sort by a parsed field type.
pub fn sortable(ty: &Ty) -> bool {
    Kind::of(ty).is_some_and(Kind::sortable)
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
            Ty::Ref(_) | Ty::Org => Kind::Ref,
            Ty::ListStr { .. } => Kind::List,
            Ty::Geo => Kind::Geo,
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
            Kind::Geo => &["near", "bbox", "exists"],
        }
    }
    fn sortable(self) -> bool {
        !matches!(self, Kind::Ref | Kind::List | Kind::Geo)
    }
    fn numeric(self) -> bool {
        matches!(self, Kind::Int | Kind::Decimal)
    }
}
/// SQL expression over the unqualified columns of the resource table. `field` is a validated identifier.
fn expr(kind: Kind, field: &str) -> String {
    match kind {
        Kind::Int => format!("({KEYS}->'{field}')::numeric"),
        Kind::Decimal => format!("({KEYS}->>'{field}')::numeric"),
        Kind::Date | Kind::DateTime => format!("({KEYS}->>'{field}') COLLATE \"C\""),
        Kind::Text => format!("left({KEYS}->>'{field}',{TEXT_KEY}) COLLATE \"C\""),
        Kind::Bool => format!("({KEYS}->>'{field}')"),
        Kind::Ref => crate::relations::column(field),
        Kind::List => format!("({KEYS}->'{field}')"),
        // `point` is built in and immutable, so the index needs no database extension.
        Kind::Geo => format!("point(({KEYS}#>>'{{{field},lng}}')::float8,({KEYS}#>>'{{{field},lat}}')::float8)"),
        Kind::Stamp => field.to_string(),
    }
}
/// A text group key: the inline copy unless it was cut at [`TEXT_KEY`]; only such a row reads its full `data`.
fn full_text(field: &str) -> String {
    format!("(CASE WHEN length({KEYS}->>'{field}')<{TEXT_KEY} THEN {KEYS}->>'{field}' ELSE data->>'{field}' END)")
}
/// One `ORDER BY` term. Timestamps are `NOT NULL`, so naming a null position changes no order, yet Postgres only matches an
/// index whose null position is the same: `created_at DESC NULLS LAST` sorted the whole table on every plain list.
fn order_term(k: &SortKey) -> String {
    let direction = if k.desc { "DESC" } else { "ASC" };
    if k.kind == Kind::Stamp {
        return format!("{} {direction}", expr(k.kind, &k.field));
    }
    format!("{} {direction} NULLS {}", expr(k.kind, &k.field), if k.nulls_first { "FIRST" } else { "LAST" })
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
    crate::search_config::validate(r)?;
    let q = &r.query;
    let at = |what: String| format!("{}: query {what}", r.name);
    if q.filter.len() > 32 || q.sort.len() > 16 || q.default_sort.len() > 4 || q.include.len() > 8 || q.aggregate.len() > 8 || q.count.len() > 8 {
        return Err(at("declares too many entries; fix: at most 32 filters, 16 sort fields, 4 default sort keys, 8 includes, 8 aggregates and 8 counts".into()));
    }
    if q.or.len() > 4 {
        return Err(at("declares too many or groups; fix: at most 4".into()));
    }
    for (group, branches) in &q.or {
        if !crate::contract::identifier(group) || RESERVED.contains(&group.as_str()) || group == "owner" || q.filter.contains_key(group) || r.fields.contains_key(group) {
            return Err(at(format!("or group `{group}` needs its own name; fix: use an identifier that is not a field, a filter or a reserved parameter")));
        }
        if !(2..=4).contains(&branches.len()) {
            return Err(at(format!("or group `{group}` needs 2..4 fields; fix: list the alternatives, or use a plain filter for one")));
        }
        for (field, ops) in branches {
            let Some(kind) = kind(r, field) else {
                return Err(at(format!("or group `{group}`: `{field}` is not a declared field of this resource of a filterable type; fix: use a local field (related fields cannot be alternatives)")));
            };
            if restricted(r, field) {
                return Err(at(format!("or group `{group}` names `{field}`, which has restricted visibility; fix: remove it or make the field public")));
            }
            let unique: BTreeSet<_> = ops.iter().collect();
            if ops.is_empty() || unique.len() != ops.len() {
                return Err(at(format!("or group `{group}`: `{field}` needs distinct operators; allowed: {}", kind.ops().join(", "))));
            }
            if let Some(op) = ops.iter().find(|op| !kind.ops().contains(&op.as_str())) {
                return Err(at(format!("or group `{group}`: `{field}` does not support `{op}`; allowed: {}", kind.ops().join(", "))));
            }
        }
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
        // A field visible to named roles only may be filtered by those roles (`parse` gates each request); one that
        // differs per row (`owner`) or that nobody reads would let a filter reveal what the reader cannot see.
        if restricted(r, local) && !r.fields.get(local).is_some_and(crate::visibility::role_gated) {
            return Err(at(format!("filter `{key}` names a field whose visibility includes `owner` or that no reader receives; fix: remove the filter, or list only role names in `visible` (e.g. ['admin']) so those roles can filter by it")));
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
    if q.facets.len() > 8 {
        return Err(at("declares too many facets; fix: at most 8".to_string()));
    }
    let mut seen_facets = BTreeSet::new();
    for field in &q.facets {
        if !seen_facets.insert(field) {
            return Err(at(format!("facet `{field}` is declared twice")));
        }
        if !matches!(kind(r, field), Some(Kind::Text | Kind::Bool | Kind::Ref | Kind::Date | Kind::Int | Kind::List)) || restricted(r, field) {
            return Err(at(format!("facet `{field}` must be a public str, enum, bool, ref, date, int or list<str> field")));
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
    if r.policy.api {
        let reached = r.query.include.iter().map(String::as_str)
            .chain(r.query.filter.keys().filter_map(|k| k.split_once('.').map(|(local, _)| local)))
            .filter_map(|local| crate::relations::target(r, local))
            .chain(r.query.count.values().map(|c| c.resource.clone()));
        for target in reached {
            if find(&target).is_some_and(|t| !t.policy.api) {
                return Err(format!("{}: query reaches server-only resource `{target}` (api: false); fix: remove that include, filter or count, or drop api: false", r.name));
            }
        }
    }
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
    // A plain index read backwards puts nulls first, so it cannot serve the other direction of a declared null position.
    let mut opposite = BTreeMap::new();
    for entry in &r.query.sort {
        let field = entry.field();
        if let Some(k) = kind(r, field).filter(|k| !matches!(k, Kind::Stamp | Kind::Ref | Kind::List)) {
            let order = if entry.nulls_first() { "ASC NULLS FIRST" } else { "DESC NULLS LAST" };
            let expression = expr(k, field);
            opposite.insert(r.object_name("_brain_ix_", &format!("btree:{expression}:{order}")), format!("USING btree (({expression}) {order})"));
        }
    }
    let mut fields: Vec<(&str, bool, bool)> = r
        .query
        .filter
        .iter()
        .filter(|(k, _)| !k.contains('.'))
        .map(|(k, ops)| (k.as_str(), ops.iter().any(|o| o == "iprefix"), ops.iter().any(|o| o == "exists")))
        .collect();
    fields.extend(r.query.sort.iter().map(|e| (e.field(), false, false)));
    // Each alternative of an `or` group has its own index, so the planner can combine them instead of scanning.
    fields.extend(r.query.or.values().flatten().map(|(k, ops)| (k.as_str(), ops.iter().any(|o| o == "iprefix"), ops.iter().any(|o| o == "exists"))));
    for (field, iprefix, _) in fields {
        match kind(r, field) {
            Some(Kind::Stamp | Kind::Ref) | None => {}
            Some(Kind::List) => add("gin", expr(Kind::List, field)),
            Some(Kind::Geo) => add("gist", expr(Kind::Geo, field)),
            Some(k) => {
                add("btree", expr(k, field));
                if iprefix {
                    add("btree", format!("lower(left({KEYS}->>'{field}',{TEXT_KEY})) COLLATE \"C\""));
                }
            }
        }
    }
    if r.query.sort.iter().any(|e| e.field() == "updated_at") || r.query.filter.contains_key("updated_at") || r.query.or.values().any(|b| b.contains_key("updated_at")) {
        add("btree", "updated_at".into());
    }
    out.extend(opposite);
    out.into_iter().collect()
}
/// The scalar expressions declared filters compare, spelled as their indexes and predicates spell them, so the planner
/// matches extended statistics to them.
pub(crate) fn statistics_expressions(r: &Resource) -> Vec<String> {
    let mut out = BTreeSet::new();
    for field in r.query.filter.keys().filter(|k| !k.contains('.')) {
        if let Some(k) = kind(r, field).filter(|k| !matches!(k, Kind::Stamp | Kind::Ref | Kind::List | Kind::Geo)) {
            out.insert(expr(k, field));
        }
    }
    out.into_iter().collect()
}
pub(crate) async fn install_indexes(tx: &mut sqlx::Transaction<'_, Postgres>, r: &Resource) -> Result<(), String> {
    install_keys(tx, r).await?;
    // A filter or sort that is gone leaves an index every write still pays for; only generated names are touched.
    let mut declared: Vec<String> = indexes(r).into_iter().map(|(name, _)| name).collect();
    declared.extend(crate::relations::refs(r).iter().map(|x| crate::relations::index_name(r, x.field)));
    let stored: Vec<String> = sqlx::query_scalar("SELECT indexname::text FROM pg_indexes WHERE schemaname='public' AND tablename=$1 AND indexname LIKE '\\_brain\\_ix\\_%'")
        .bind(format!("_brain_res_{}", r.name))
        .fetch_all(&mut **tx)
        .await
        .map_err(crate::db::safe)?;
    for name in stored.iter().filter(|name| !declared.contains(name)) {
        // Audited SQL: the name was read from the catalog.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP INDEX public.{name}"))).execute(&mut **tx).await.map_err(crate::db::safe)?;
    }
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

/// The inline [`KEYS`] column every query reads. Adding it rewrites the table once (under the migration lock, without
/// the statement deadline, like the search rebuild); after that every write keeps it current, with no trigger.
async fn keys_present(conn: &mut sqlx::PgConnection, table: &str) -> Result<bool, String> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid=to_regclass($1) AND attname=$2 AND NOT attisdropped)")
        .bind(format!("public.{table}"))
        .bind(KEYS)
        .fetch_one(conn)
        .await
        .map_err(crate::db::safe)
}
/// Serving reads every list through the key copy, so a database migrated by an older brain fails before HTTP opens.
pub(crate) async fn verify_keys(conn: &mut sqlx::PgConnection, r: &Resource) -> Result<(), String> {
    let table = format!("_brain_res_{}", r.name);
    if keys_present(conn, &table).await? {
        return Ok(());
    }
    Err(format!("BDB_SCHEMA_DRIFT: {table}.{KEYS}; review schema and run the explicit backend migration; no rows were removed"))
}
async fn install_keys(tx: &mut sqlx::Transaction<'_, Postgres>, r: &Resource) -> Result<(), String> {
    let table = format!("_brain_res_{}", r.name);
    if keys_present(&mut **tx, &table).await? {
        return Ok(());
    }
    // Audited SQL: constant names and a validated table identifier. The function is immutable: same document, same copy.
    let ddl = format!(
        "CREATE OR REPLACE FUNCTION {KEYS_FUNCTION}(d jsonb) RETURNS jsonb LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$ SELECT COALESCE(jsonb_object_agg(k,CASE WHEN jsonb_typeof(v)='string' AND length(v#>>'{{}}')>{TEXT_KEY} THEN to_jsonb(left(v#>>'{{}}',{TEXT_KEY})) ELSE v END),'{{}}'::jsonb) FROM jsonb_each(d) AS e(k,v) $$; SET LOCAL statement_timeout=0; ALTER TABLE {table} ADD COLUMN {KEYS} jsonb GENERATED ALWAYS AS ({KEYS_FUNCTION}(data)) STORED, ALTER COLUMN {KEYS} SET STORAGE MAIN; SET LOCAL statement_timeout='60s';"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl)).execute(&mut **tx).await.map_err(crate::db::safe)?;
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
    facets: Vec<String>,
    /// `?fields=`: the `data` keys each item keeps. Id, timestamps, `included` and `counts` always stay.
    fields: Option<BTreeSet<String>>,
    /// Sent branches of each declared `or` group.
    alternatives: BTreeMap<String, Vec<Filter>>,
    /// `?owner=me`: limit to the reader's own rows. The id always comes from the authenticated session.
    owner_me: bool,
    /// `?owner=<id>`, from a system key only.
    owner_id: Option<Uuid>,
    /// `?total=false`: skip the count; a walk over every page does not need it on each one.
    no_total: bool,
}
impl ListRequest {
    /// The `data` keys the page must read: `?fields=` plus the references includes follow. `None`: the whole document.
    fn stored_keys(&self) -> Option<Vec<String>> {
        let fields = self.fields.as_ref()?;
        let mut keys: BTreeSet<String> = fields.clone();
        keys.extend(self.include.iter().map(|path| path.split('.').next().unwrap_or(path).to_string()));
        Some(keys.into_iter().collect())
    }
}
impl ListRequest {
    /// Narrows items to `?fields=`. SQL already left out the rest of the document; this runs after projection and
    /// includes, so it can only remove what the reader already received, and includes may still follow a reference it drops.
    pub fn keep_fields(&self, items: &mut [Value]) {
        let Some(fields) = &self.fields else { return };
        for data in items.iter_mut().filter_map(|item| item.get_mut("data").and_then(Value::as_object_mut)) {
            data.retain(|name, _| fields.contains(name));
        }
    }
}
/// Whether `?fields=` may name a field. Fields no reader receives answer like undeclared ones, so a request learns nothing.
fn selectable(r: &Resource, field: &str) -> bool {
    r.fields.get(field).is_some_and(|f| match f.access_mode() {
        Some("writeonly") => false,
        Some("private") => crate::api_keys::system_active(),
        _ => true,
    })
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
        (Kind::Geo, "near") => near_operand(raw).map(|_| raw.to_string())?,
        (Kind::Geo, "bbox") => box_operand(raw).map(|_| raw.to_string())?,
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

/// Production answers name the bad parameter without listing what the contract declares: the catalog
/// of filters and sorts is admin-only there (`/_brain/api`), so a probe must not recover it from a 422.
pub fn without_declared_names(problems: Problems) -> Problems {
    problems
        .into_iter()
        .map(|(key, message)| {
            let kept = ["; declared:", "; filterable:"].iter().find_map(|cut| message.find(cut)).map_or(message.as_str(), |at| &message[..at]);
            (key, kept.to_string())
        })
        .collect()
}

/// Why a list request may not filter `field` with `op`, or `None` when the contract declares it. The one rule `parse`
/// answers 422 with; `brain check` applies it to the literal options of a native `list()` call (B0815).
pub fn filter_problem(r: &Resource, field: &str, op: &str) -> Option<String> {
    if let Some((group, name)) = field.split_once('.').filter(|(group, _)| r.query.or.contains_key(*group)) {
        let declared = r.query.or[group].get(name).is_some_and(|ops| ops.iter().any(|o| o == op)) && kind(r, name).is_some();
        return (!declared).then(|| format!("`{name}` with `{op}` is not an alternative of `{group}`; declared: {}", r.query.or[group].keys().cloned().collect::<Vec<_>>().join(", ")));
    }
    if r.query.filter.get(field).is_some_and(|ops| ops.iter().any(|o| o == op)) {
        return None;
    }
    Some(match r.query.filter.get(field).map(|ops| ops.join(", ")) {
        Some(ops) => format!("Operator `{op}` is not declared for `{field}`; declared: {ops}"),
        None if RESERVED.contains(&field) => "Malformed parameter".to_string(),
        None => format!("Unknown query parameter; filterable: {}", r.query.filter.keys().cloned().collect::<Vec<_>>().join(", ")),
    })
}

/// Why a list request may not sort by `field`, or `None` when it may (`created_at` always may).
pub fn sort_problem(r: &Resource, field: &str) -> Option<String> {
    let declared = r.query.sort.iter().any(|e| e.field() == field) && kind(r, field).is_some();
    (!declared && field != "created_at").then(|| format!("`{field}` is not sortable; declared: {}", sort_names(r)))
}

/// Whether the reader with `role` may use the declared filter `key`: a filter on a field visible to named roles only
/// (on this resource, or on the referenced one for `ref.field`) exists for those roles alone.
fn filter_open(r: &Resource, resources: &BTreeMap<String, Resource>, key: &str, role: Option<&str>) -> bool {
    let open = |res: &Resource, name: &str| res.fields.get(name).is_none_or(|f| crate::visibility::filterable_by(f, role));
    match key.split_once('.') {
        Some((local, inner)) => open(r, local) && crate::relations::target(r, local).and_then(|t| resources.get(&t)).is_none_or(|t| open(t, inner)),
        None => open(r, key),
    }
}

/// Parses raw query pairs against the contract. Every undeclared key, operator or malformed value is a problem.
/// `role` is the reader's: a filter on a role-gated field answers, for anyone else, exactly like an undeclared one.
pub fn parse(r: &Resource, resources: &BTreeMap<String, Resource>, pairs: &[(String, String)], role: Option<&str>) -> Result<ListRequest, Problems> {
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
        if raw.contains('\0') {
            problems.insert(key.clone(), "Remove the NUL character from this value".into());
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
                            if let Some(message) = sort_problem(r, field) {
                                bad(&message);
                            }
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
                if split(raw).len() > crate::relations::MAX_INCLUDE_PATHS {
                    bad(&format!("include takes at most {} paths", crate::relations::MAX_INCLUDE_PATHS));
                }
                for name in split(raw) {
                    if !crate::relations::include_path_declared(r, resources, name) {
                        bad(&format!("`{name}` is not includable (at most {} levels, each declared by its resource); declared: {}", crate::relations::MAX_INCLUDE_DEPTH, r.query.include.join(", ")));
                    }
                }
                req.include = split(raw).into_iter().map(str::to_string).collect();
            }
            // A filter declared on a field of this name keeps working as before.
            "fields" if !r.query.filter.contains_key("fields") => {
                let names: BTreeSet<String> = split(raw).into_iter().map(str::to_string).collect();
                if names.len() > 64 {
                    bad("fields takes at most 64 names");
                }
                if let Some(name) = names.iter().find(|name| !selectable(r, name)) {
                    let declared: Vec<&str> = r.fields.keys().map(String::as_str).filter(|f| selectable(r, f)).collect();
                    bad(&format!("`{name}` cannot be selected; declared: {}", declared.join(", ")));
                }
                req.fields = Some(names);
            }
            // A filter declared on a field of this name keeps working as before.
            "owner" if !r.query.filter.contains_key("owner") => {
                // Only a system key may name an account: for anyone else this would reveal who owns a readable row.
                match Uuid::parse_str(raw) {
                    _ if raw == "me" => req.owner_me = true,
                    Ok(id) if crate::api_keys::system_active() => req.owner_id = Some(id),
                    _ => bad("owner accepts only `me`; the server resolves it from the session"),
                }
            }
            // A filter declared on a field of this name keeps working as before.
            "total" if !r.query.filter.contains_key("total") => {
                if raw == "false" { req.no_total = true } else { bad("total accepts only `false`; leave it out to get the count") }
            }
            // A filter declared on a field of this name keeps working as before.
            "facets" if !r.query.filter.contains_key("facets") => {
                let names: Vec<String> = split(raw).into_iter().map(str::to_string).collect();
                if let Some(name) = names.iter().find(|n| !r.query.facets.contains(n)) {
                    bad(&format!("`{name}` is not a declared facet"));
                }
                req.facets = names;
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
                let open_keys = || r.query.filter.keys().filter(|key| filter_open(r, resources, key, role)).cloned().collect::<Vec<_>>().join(", ");
                let gated = r.query.filter.contains_key(field) && !filter_open(r, resources, field, role);
                let problem = if gated { Some(format!("Unknown query parameter; filterable: {}", open_keys())) } else {
                    filter_problem(r, field, op).map(|m| if m.starts_with("Unknown query parameter") { format!("Unknown query parameter; filterable: {}", open_keys()) } else { m })
                };
                if let Some(message) = problem {
                    bad(&message);
                    continue;
                }
                if let Some((group, name)) = field.split_once('.').filter(|(group, _)| r.query.or.contains_key(*group)) {
                    let Some(k) = kind(r, name) else { continue };
                    let raws = if matches!(op, "in" | "between") { split(raw) } else { vec![raw.as_str()] };
                    if op == "in" && !(1..=MAX_IN).contains(&raws.len()) || op == "between" && raws.len() != 2 {
                        bad(if op == "in" { "in takes 1..50 comma-separated values" } else { "between takes two comma-separated values: low,high" });
                        continue;
                    }
                    let values: Option<Vec<Operand>> = raws.iter().map(|v| operand(r, name, k, op, v)).collect();
                    match values {
                        Some(values) => req.alternatives.entry(group.to_string()).or_default().push(Filter { via: None, field: name.to_string(), kind: k, op: op.to_string(), values }),
                        None => bad(&format!("Invalid value for `{field}` ({op}); expected {}", expected(r, name, k))),
                    }
                    continue;
                }
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
    if req.filters.len() + req.alternatives.values().map(Vec::len).sum::<usize>() > MAX_FILTERS {
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
        Kind::Geo => "lat,lng,km for near (km above 0, at most 20040) or minLat,minLng,maxLat,maxLng for bbox".into(),
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

/// What the search text selects, decided once per request so page, total and aggregates agree.
#[derive(Debug, Default)]
enum Matches {
    /// No search text.
    #[default]
    All,
    /// Nothing can match: stop words only, punctuation only, or no searchable field.
    Nothing,
    /// One or two characters: prefix of the stored text.
    Prefix,
    /// Every full-text match; `total` is exact.
    FullText,
    /// Full text found nothing: these typo matches, bounded by [`FUZZY_CANDIDATES`].
    Typos(Vec<Uuid>),
}
/// Why a list could not be produced.
#[derive(Debug)]
pub enum ListError {
    Invalid(Problems),
    Denied(String),
    /// `owner=me` from a request without a session or key.
    SignInRequired,
    Database { error: sqlx::Error, at: &'static std::panic::Location<'static>, branch: &'static str },
}
impl From<sqlx::Error> for ListError {
    #[track_caller]
    fn from(error: sqlx::Error) -> Self {
        ListError::Database { error, at: std::panic::Location::caller(), branch: "page" }
    }
}
impl ListError {
    fn in_branch(self, name: &'static str) -> Self {
        match self {
            ListError::Database { error, at, .. } => ListError::Database { error, at, branch: name },
            other => other,
        }
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
fn operand_text(value: &Operand) -> Option<&str> {
    match value {
        Operand::Value(text) => Some(text),
        Operand::Now(_) => None,
    }
}
/// `lat,lng,km` of a `near` filter.
fn near_operand(raw: &str) -> Option<[f64; 3]> {
    let [lat, lng, km] = geo_numbers::<3>(raw)?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lng) && km > 0.0 && km <= MAX_NEAR_KM).then_some([lat, lng, km])
}
/// `minLat,minLng,maxLat,maxLng` of a `bbox` filter. `minLng` above `maxLng` means the box crosses the antimeridian.
fn box_operand(raw: &str) -> Option<[f64; 4]> {
    let [south, west, north, east] = geo_numbers::<4>(raw)?;
    let lat = -90.0..=90.0;
    let lng = -180.0..=180.0;
    (lat.contains(&south) && lat.contains(&north) && south <= north && lng.contains(&west) && lng.contains(&east)).then_some([south, west, north, east])
}
fn geo_numbers<const N: usize>(raw: &str) -> Option<[f64; N]> {
    let numbers: Vec<f64> = raw.split(',').map(|n| n.parse::<f64>().ok().filter(|n| n.is_finite())).collect::<Option<_>>()?;
    numbers.try_into().ok()
}
/// Boxes as `[west, south, east, north]`, split at the antimeridian so each one is ordinary.
fn lng_spans(south: f64, west: f64, north: f64, east: f64) -> Vec<[f64; 4]> {
    if west <= east {
        vec![[west, south, east, north]]
    } else {
        vec![[west, south, 180.0, north], [-180.0, south, east, north]]
    }
}
/// Boxes that contain every point within `km` of the center, for the index; the exact distance is checked afterwards.
fn near_boxes(lat: f64, lng: f64, km: f64) -> Vec<[f64; 4]> {
    let angle = km / EARTH_KM;
    let reach = angle.to_degrees() + GEO_MARGIN;
    let (south, north) = (lat - reach, lat + reach);
    // A circle that reaches a pole contains every longitude there.
    if south <= -90.0 || north >= 90.0 {
        return vec![[-180.0, south.max(-90.0), 180.0, north.min(90.0)]];
    }
    let span = (angle.sin() / lat.to_radians().cos()).clamp(-1.0, 1.0).asin().to_degrees() + GEO_MARGIN;
    if span >= 180.0 {
        return vec![[-180.0, south, 180.0, north]];
    }
    let (west, east) = (lng - span, lng + span);
    if west < -180.0 {
        lng_spans(south, west + 360.0, north, east)
    } else if east > 180.0 {
        lng_spans(south, west, north, east - 360.0)
    } else {
        vec![[west, south, east, north]]
    }
}
fn push_boxes(q: &mut QueryBuilder<Postgres>, e: &str, boxes: &[[f64; 4]]) {
    q.push("(");
    for (i, [west, south, east, north]) in boxes.iter().enumerate() {
        if i > 0 {
            q.push(" OR ");
        }
        q.push(format!("{e} <@ box(point(")).push_bind(*west).push(",").push_bind(*south).push("),point(").push_bind(*east).push(",").push_bind(*north).push("))");
    }
    q.push(")");
}
fn push_predicate(q: &mut QueryBuilder<Postgres>, f: &Filter) {
    let e = expr(f.kind, &f.field);
    let field = &f.field;
    let first = &f.values[0];
    // A value shorter than the text key equals the stored text exactly when their keys match: only a value that fills
    // the key could match a longer text. The recheck reads `data` again, once more per row.
    let full = |q: &mut QueryBuilder<Postgres>, v: &Operand| {
        if let (Kind::Text, Operand::Value(v)) = (f.kind, v)
            && v.chars().count() >= TEXT_KEY
        {
            q.push(format!(" AND (data->>'{field}')=")).push_bind(v.clone());
        }
    };
    match f.op.as_str() {
        "near" => {
            if let Some([lat, lng, km]) = operand_text(first).and_then(near_operand) {
                q.push("(");
                push_boxes(q, &e, &near_boxes(lat, lng, km));
                let (y, x) = (format!("({KEYS}#>>'{{{field},lat}}')::float8"), format!("({KEYS}#>>'{{{field},lng}}')::float8"));
                q.push(format!(" AND 2*{EARTH_KM}::float8*asin(least(1::float8,sqrt(power(sin(radians({y}-"))
                    .push_bind(lat)
                    .push(")/2),2)+cos(radians(")
                    .push_bind(lat)
                    .push(format!("))*cos(radians({y}))*power(sin(radians({x}-"))
                    .push_bind(lng)
                    .push(")/2),2))))<=")
                    .push_bind(km)
                    .push(")");
            }
        }
        "bbox" => {
            if let Some([south, west, north, east]) = operand_text(first).and_then(box_operand) {
                push_boxes(q, &e, &lng_spans(south, west, north, east));
            }
        }
        "exists" => {
            let yes = first == &Operand::Value("true".into());
            q.push(if yes { "(" } else { "NOT (" });
            if f.kind == Kind::Ref {
                q.push(&e).push(" IS NOT NULL)");
            } else {
                q.push(format!("{KEYS} ? '{field}')"));
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
                    q.push(format!("(lower(left({KEYS}->>'{field}',{TEXT_KEY})) COLLATE \"C\" LIKE "))
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
                "ne" => " IS DISTINCT FROM ",
                "gt" => ">",
                "gte" => ">=",
                "lt" => "<",
                _ => "<=",
            };
            if op == "ne" && f.kind == Kind::Text {
                q.push(format!("({} IS DISTINCT FROM ", full_text(field)));
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

/// The operands of one static row condition, or None when the value does not fit the field.
fn condition_operands(r: &Resource, field: &str, value: &Value) -> Option<(Kind, &'static str, Vec<Operand>)> {
    let f = r.fields.get(field)?;
    let ty = f.ty()?;
    let kind = Kind::of(&ty).filter(|k| !matches!(k, Kind::List | Kind::Geo))?;
    let one = |v: &Value| {
        if v.is_array() || v.is_object() || v.is_null() {
            return None;
        }
        let v = ty.normalize(v, true)?;
        Some(Operand::Value(v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
    };
    match value {
        Value::Null => (!f.required).then(|| (kind, "exists", vec![Operand::Value("false".into())])),
        Value::Array(list) => {
            if list.is_empty() || list.len() > 16 {
                return None;
            }
            Some((kind, "in", list.iter().map(one).collect::<Option<Vec<_>>>()?))
        }
        v => Some((kind, "eq", vec![one(v)?])),
    }
}
/// Pushes one validated row condition of a read grant; an unusable value fails closed.
pub(crate) fn push_row_condition(q: &mut QueryBuilder<Postgres>, r: &Resource, field: &str, value: &Value) {
    match condition_operands(r, field, value) {
        Some((kind, op, values)) => push_predicate(q, &Filter { via: None, field: field.to_string(), kind, op: op.into(), values }),
        None => {
            q.push("false");
        }
    }
}
/// Static validation of the `where` conditions of read grants.
pub(crate) fn validate_read_where(r: &Resource) -> Result<(), String> {
    for (grant, conditions) in &r.policy.read_where {
        let at = format!("{}: read grant `{grant}` where", r.name);
        if conditions.is_empty() || conditions.len() > 8 {
            return Err(format!("{at} needs 1..8 fields; fix: keep the row condition short and combine values with an array"));
        }
        for (field, value) in conditions {
            let Some(f) = r.fields.get(field) else {
                let known = r.fields.keys().map(String::as_str).collect::<Vec<_>>().join(", ");
                return Err(format!("{at}: `{field}` is not a declared field; fix: use one of {known}"));
            };
            if f.ty().as_ref().and_then(Kind::of).is_none_or(|k| k == Kind::List) {
                return Err(format!("{at}: `{field}` is a {} field; fix: condition on a scalar field (str, enum, bool, int, decimal, date, ref)", f.kind));
            }
            if condition_operands(r, field, value).is_none() {
                let expected = f.ty().map(|t| t.expected()).unwrap_or_default();
                return Err(format!("{at}: `{field}` value {value} does not fit the field; fix: use {expected}, an array of 1..16 such values, or null for an optional field"));
            }
        }
    }
    Ok(())
}

struct Ctx<'a> {
    r: &'a Resource,
    resources: &'a BTreeMap<String, Resource>,
    scope: &'a Scope,
    related: BTreeMap<String, Scope>,
    req: &'a ListRequest,
    matches: Matches,
    /// The session's user when the request says `owner=me`.
    owner: Option<Uuid>,
    /// Field whose own filters do not apply, so a facet still offers its other values. The scope always applies.
    facet: Option<&'a str>,
    /// Counting statements read a large `data` once per row instead of once per predicate ([`Ctx::decide_read_once`]).
    read_once: bool,
}
impl Ctx<'_> {
    /// Authorization scope and declared filters over the alias-free columns of the current table.
    fn push_scope(&self, q: &mut QueryBuilder<Postgres>) {
        self.scope.push(q, self.r, self.resources);
        if let Some(owner) = self.owner {
            q.push(" AND owner=").push_bind(owner);
        }
        for branches in self.req.alternatives.values() {
            q.push(" AND (");
            for (i, f) in branches.iter().enumerate() {
                if i > 0 {
                    q.push(" OR ");
                }
                push_predicate(q, f);
            }
            q.push(")");
        }
        for f in &self.req.filters {
            if f.via.is_none() && self.facet == Some(f.field.as_str()) {
                continue;
            }
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
    }
    /// WHERE body: scope, filters and the search decision.
    fn push_where(&self, q: &mut QueryBuilder<Postgres>) {
        self.push_scope(q);
        let text = self.req.search.trim();
        match &self.matches {
            Matches::All => {}
            Matches::Nothing => {
                q.push(" AND false");
            }
            Matches::Prefix => {
                q.push(" AND search_text LIKE public.unaccent('public.unaccent'::regdictionary,lower(")
                    .push_bind(escape_like(&text.to_lowercase()))
                    .push(")) ESCAPE '\\'");
            }
            Matches::FullText => {
                q.push(format!(" AND search_vector @@ {}", self.tsquery())).push_bind(text.to_string()).push("))");
            }
            Matches::Typos(ids) => {
                q.push(" AND id=ANY(").push_bind(ids.clone()).push(")");
            }
        }
    }
    /// WHERE body of a statement that visits every matching row. Postgres decompresses `data` again for each predicate
    /// that reads it; inside the subquery `data` resolves to one already decompressed copy of the row's document.
    /// `||` with an empty object returns its detoasted operand without rebuilding it.
    fn push_counted(&self, q: &mut QueryBuilder<Postgres>) {
        if self.read_once {
            q.push("EXISTS (SELECT 1 FROM (SELECT t.data||'{}'::jsonb AS data OFFSET 0) _once WHERE ");
        }
        self.push_where(q);
        if self.read_once {
            q.push(")");
        }
    }
    /// Whether the counting statements of this request should read `data` once. The rewritten predicates cannot use an
    /// index and the rewritten plan runs without parallel workers, so it applies only to what it pays for: several
    /// reads per row, of a table whose documents live out of line, that the planner would scan whole anyway.
    async fn decide_read_once(&self, tx: &mut sqlx::Transaction<'_, Postgres>) -> Result<bool, sqlx::Error> {
        let table = format!("_brain_res_{}", self.r.name);
        let mut q = QueryBuilder::<Postgres>::new(format!("EXPLAIN SELECT count(*) FROM {table} t WHERE "));
        self.push_where(&mut q);
        let sql = q.sql();
        if ["data->", "data#>", "data ?"].iter().map(|read| sql.as_str().matches(read).count()).sum::<usize>() < 2 {
            return Ok(false);
        }
        let out_of_line: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT pg_relation_size(reltoastrelid)>pg_relation_size(oid) FROM pg_class WHERE oid='{table}'::regclass"
        )))
        .fetch_one(&mut **tx)
        .await?;
        if !out_of_line {
            return Ok(false);
        }
        let plan: Vec<String> = q.build_query_scalar().fetch_all(&mut **tx).await?;
        let scan = format!("Seq Scan on {table} t");
        Ok(plan.iter().any(|line| line.contains(&scan)))
    }
    /// Opening of the text-search query expression; the caller binds the text and closes two parentheses.
    fn tsquery(&self) -> String {
        format!("websearch_to_tsquery('{}',public.unaccent('public.unaccent'::regdictionary,", crate::search_config::regconfig(self.r))
    }
    /// Whether the text reduces to no search term at all (only stop words of the configured language).
    async fn only_stop_words(&self, tx: &mut sqlx::Transaction<'_, Postgres>, text: &str) -> Result<bool, sqlx::Error> {
        let mut q = QueryBuilder::<Postgres>::new(format!("SELECT numnode({}", self.tsquery()));
        q.push_bind(text.to_string()).push(")))=0");
        q.build_query_scalar().fetch_one(&mut **tx).await
    }
    /// Typo fallback needs only existence, under the same authorization and filters as the eventual page and total.
    fn match_probe(&self) -> QueryBuilder<Postgres> {
        let mut q = QueryBuilder::<Postgres>::new(format!("SELECT EXISTS(SELECT 1 FROM _brain_res_{} t WHERE ", self.r.name));
        self.push_where(&mut q);
        q.push(")");
        q
    }
    async fn has_matches(&self, tx: &mut sqlx::Transaction<'_, Postgres>) -> Result<bool, sqlx::Error> {
        self.match_probe().build_query_scalar().fetch_one(&mut **tx).await
    }
    /// Typo candidates within [`FUZZY_BUDGET`]; `None` when the budget ran out, which leaves the transaction usable.
    async fn fuzzy(&self, tx: &mut sqlx::Transaction<'_, Postgres>, text: &str) -> Result<Option<Vec<Uuid>>, sqlx::Error> {
        use sqlx::Acquire;
        let mut savepoint = tx.begin().await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("SET LOCAL statement_timeout='{FUZZY_BUDGET}'"))).execute(&mut *savepoint).await?;
        let mut q = QueryBuilder::<Postgres>::new(format!("SELECT id FROM _brain_res_{} WHERE ", self.r.name));
        self.push_scope(&mut q);
        q.push(" AND public.unaccent('public.unaccent'::regdictionary,").push_bind(text.to_lowercase()).push(format!(") OPERATOR(public.<%) search_text ORDER BY id LIMIT {FUZZY_CANDIDATES}"));
        match q.build_query_scalar().fetch_all(&mut *savepoint).await {
            Ok(ids) => {
                savepoint.commit().await?;
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("SET LOCAL statement_timeout='{SEARCH_DEADLINE}'"))).execute(&mut **tx).await?;
                Ok(Some(ids))
            }
            Err(error) if crate::db::sqlstate(&error).as_deref() == Some("57014") => {
                savepoint.rollback().await?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}

/// How one search resolves. Full text decides: typo matches are a fallback for when it finds nothing, so they never
/// dilute real results or `total`. Returns the decision, whether typo matching ran out of budget, and whether `total`
/// is only a lower bound.
async fn resolve_search(tx: &mut sqlx::Transaction<'_, Postgres>, ctx: &mut Ctx<'_>, text: &str) -> Result<(bool, bool), ListError> {
    let in_branch = |name: &'static str| move |e: sqlx::Error| ListError::from(e).in_branch(name);
    let has_term = text.chars().any(char::is_alphanumeric);
    if ctx.r.search.is_empty() {
        ctx.matches = Matches::Nothing;
        return Ok((false, false));
    }
    if has_term && ctx.only_stop_words(tx, text).await.map_err(in_branch("search.terms"))? {
        ctx.matches = Matches::Nothing;
        return Ok((false, false));
    }
    // Punctuation has no indexable term or trigram: a prefix match could only scan every row, twice, to find next to nothing.
    if !has_term && text.chars().count() < 3 {
        ctx.matches = Matches::Nothing;
        return Ok((false, false));
    }
    if text.chars().count() < 3 {
        ctx.matches = Matches::Prefix;
        return Ok((false, false));
    }
    ctx.matches = Matches::FullText;
    if ctx.has_matches(tx).await.map_err(in_branch("search.full_text"))? {
        return Ok((false, false));
    }
    // pg_trgm extracts nothing from punctuation: the scan could only find no rows.
    if !has_term {
        ctx.matches = Matches::Nothing;
        return Ok((false, false));
    }
    match ctx.fuzzy(tx, text).await.map_err(in_branch("search.fuzzy"))? {
        Some(ids) => {
            let capped = ids.len() as i64 >= FUZZY_CANDIDATES;
            ctx.matches = Matches::Typos(ids);
            Ok((false, capped))
        }
        None => {
            crate::db::diagnostic(&json!({"schemaVersion":1,"event":"search-degraded","rid":crate::http::request_reference(),"resource":ctx.r.name,"operation":"list.search","branch":"search.fuzzy","budget":FUZZY_BUDGET,"reason":"query-canceled","sqlstate":"57014","fix":"Inspect the search query plan, trigram index and current database load; the bounded fuzzy branch was canceled, not proven to have no matches","private":true,"valuesCaptured":false}));
            ctx.matches = Matches::Nothing;
            Ok((true, true))
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
    if req.owner_me && viewer.id.is_none() {
        return Err(ListError::SignInRequired);
    }
    let owner = if req.owner_me { viewer.id } else { req.owner_id };
    let mut ctx = Ctx { r, resources, scope: &scope, related, req, matches: Matches::All, owner, facet: None, read_once: false };
    let keys = keys(r, req);
    let sig = signature(&keys);
    let ranked = req.sort.is_none() && search_ranked(r, req);
    let text = req.search.trim().to_string();
    let mut tx = crate::db::acquiring(pool.begin()).await?;
    let timeout = if text.is_empty() { "SET LOCAL statement_timeout='5s';" } else { "SET LOCAL statement_timeout='1s';" };
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{timeout} SET LOCAL lock_timeout='500ms'; SET LOCAL pg_trgm.word_similarity_threshold='0.45';"))).execute(&mut *tx).await?;
    let (mut degraded, mut approximate) = (false, false);
    if !text.is_empty() {
        (degraded, approximate) = resolve_search(&mut tx, &mut ctx, &text).await?;
    }
    let mut ctx = ctx;

    // Page.
    // `?fields=` reads the document once and sends only what the answer uses: the page is what crosses the wire.
    let mut q = match req.stored_keys() {
        None => QueryBuilder::<Postgres>::new(concat!("SELECT ", crate::record_columns!())),
        Some(stored) => {
            let mut q = QueryBuilder::<Postgres>::new("SELECT id,owner,COALESCE((SELECT jsonb_object_agg(_key,_value) FROM jsonb_each(t.data) AS _e(_key,_value) WHERE _key=ANY(");
            q.push_bind(stored).push(")),'{}'::jsonb) AS data,").push(crate::record_times!());
            q
        }
    };
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
        q.push(format!("ts_rank_cd(search_vector,websearch_to_tsquery('{cfg}',public.unaccent('public.unaccent'::regdictionary,", cfg = crate::search_config::regconfig(r)))
            .push_bind(text.clone())
            .push("))) DESC,");
        // Similarity only orders typo matches: over full-text matches it would rescan every long text for a tie-break.
        if !matches!(ctx.matches, Matches::FullText) {
            q.push("public.word_similarity(public.unaccent('public.unaccent'::regdictionary,")
                .push_bind(text.to_lowercase())
                .push("),search_text) DESC,");
        }
        q.push("id");
    } else {
        for k in &keys {
            q.push(format!("{},", order_term(k)));
        }
        q.push(if keys.last().is_some_and(|k| k.desc) { "id DESC" } else { "id ASC" });
    }
    q.push(" LIMIT ").push_bind(req.limit);
    if req.cursor.is_none() {
        q.push(" OFFSET ").push_bind(req.offset);
    }
    let rows = match q.build().fetch_all(&mut *tx).await {
        // A cursor is client text: a key its sort cannot cast (data exception) means it was edited, not that we failed.
        Err(error) if req.cursor.is_some() && crate::db::sqlstate(&error).is_some_and(|state| state.starts_with("22")) => {
            return Err(ListError::Invalid(Problems::from([("cursor".into(), "Invalid cursor; use next_cursor from the previous page unchanged".into())])));
        }
        other => other?,
    };
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

    if !req.no_total || !req.aggregate.is_empty() || !req.facets.is_empty() {
        ctx.read_once = ctx.decide_read_once(&mut tx).await?;
    }
    // Total over the same WHERE (without the cursor).
    let total: Option<i64> = if req.no_total {
        None
    } else {
        let mut q = QueryBuilder::<Postgres>::new(format!("SELECT count(*) FROM _brain_res_{} t WHERE ", r.name));
        ctx.push_counted(&mut q);
        Some(q.build_query_scalar().fetch_one(&mut *tx).await?)
    };

    let mut aggregates = Map::new();
    let mut truncated = Vec::new();
    for name in &req.aggregate {
        let a = &r.query.aggregate[name];
        let (value, cut) = aggregate(&mut tx, &ctx, a).await?;
        aggregates.insert(name.clone(), value);
        if cut {
            truncated.push(name.clone());
        }
    }
    let mut facets = Map::new();
    let mut facets_cut = Vec::new();
    for field in &req.facets {
        ctx.facet = Some(field);
        let (value, cut) = facet(&mut tx, &ctx, field).await?;
        facets.insert(field.clone(), value);
        if cut {
            facets_cut.push(field.clone());
        }
    }
    ctx.facet = None;
    tx.commit().await?;

    let mut out = json!({"items": items, "limit": req.limit, "offset": req.offset});
    if let Some(total) = total {
        out["total"] = json!(total);
    }
    if !ranked
        && items_len(&out) as i64 == req.limit
        && let Some((values, Some(id))) = last_keys
    {
        out["next_cursor"] = json!(encode_cursor(&sig, &values, id));
    }
    if !aggregates.is_empty() {
        out["aggregates"] = Value::Object(aggregates);
    }
    if !truncated.is_empty() {
        // Named aggregates that hit GROUP_LIMIT: their largest groups are present, the rest are not.
        out["aggregates_truncated"] = json!(truncated);
    }
    if !facets.is_empty() {
        out["facets"] = Value::Object(facets);
    }
    if !facets_cut.is_empty() {
        // Facets that hit GROUP_LIMIT: their most frequent values are present, the rest are not.
        out["facets_truncated"] = json!(facets_cut);
    }
    if degraded {
        out["degraded"] = json!(true);
    }
    if approximate && total.is_some() {
        out["total_approximate"] = json!(true);
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
                    if !k.nulls_first && k.kind != Kind::Stamp {
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
    owner_visible.then(|| crate::ownership::relational_read(r, viewer)).flatten()
}

fn group_key(k: Kind, key: Option<String>) -> Value {
    match (k, key) {
        (_, None) => Value::Null,
        (Kind::Int, Some(t)) => t.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(t)),
        (Kind::Bool, Some(t)) => json!(t == "true"),
        (_, Some(t)) => json!(t),
    }
}
/// Counts of one field's values over the rows the reader may list, and whether they were cut at [`GROUP_LIMIT`].
/// Rows without a value count under a null key; a `list<str>` counts each row once per distinct element.
async fn facet(tx: &mut sqlx::Transaction<'_, Postgres>, ctx: &Ctx<'_>, field: &str) -> Result<(Value, bool), sqlx::Error> {
    let r = ctx.r;
    let k = kind(r, field).unwrap_or(Kind::Text);
    let mut q = match k {
        Kind::List => QueryBuilder::<Postgres>::new(format!(
            "SELECT _element AS _g,count(DISTINCT id) AS _n FROM _brain_res_{} t CROSS JOIN LATERAL jsonb_array_elements_text(CASE WHEN jsonb_typeof({KEYS}->'{field}')='array' THEN {KEYS}->'{field}' ELSE '[]'::jsonb END) AS _e(_element) WHERE ",
            r.name
        )),
        _ => {
            let e = match k {
                Kind::Ref => format!("{}::text", crate::relations::column(field)),
                Kind::Text => full_text(field),
                _ => format!("({})::text", expr(k, field)),
            };
            QueryBuilder::<Postgres>::new(format!("SELECT {e} AS _g,count(*) AS _n FROM _brain_res_{} t WHERE ", r.name))
        }
    };
    ctx.push_counted(&mut q);
    q.push(format!(" GROUP BY _g ORDER BY _n DESC,_g NULLS LAST LIMIT {}", GROUP_LIMIT + 1));
    let mut rows = q.build().fetch_all(&mut **tx).await?;
    let cut = rows.len() as i64 > GROUP_LIMIT;
    rows.truncate(GROUP_LIMIT as usize);
    let values = rows.iter().map(|row| json!({"key": group_key(k, row.get("_g")), "count": row.get::<i64, _>("_n")})).collect();
    Ok((Value::Array(values), cut))
}
/// One aggregate and whether its groups were cut at [`GROUP_LIMIT`].
async fn aggregate(tx: &mut sqlx::Transaction<'_, Postgres>, ctx: &Ctx<'_>, a: &Aggregate) -> Result<(Value, bool), sqlx::Error> {
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
            Kind::Text => full_text(g),
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
    ctx.push_counted(&mut q);
    if let Some((_, e)) = &group {
        q.push(format!(" GROUP BY {e} ORDER BY count(*) DESC,{e} NULLS LAST LIMIT {}", GROUP_LIMIT + 1));
    }
    let mut rows = q.build().fetch_all(&mut **tx).await?;
    let cut = group.is_some() && rows.len() as i64 > GROUP_LIMIT;
    rows.truncate(GROUP_LIMIT as usize);
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
        None => Ok((rows.first().map(|row| Value::Object(render(row))).unwrap_or(json!({"count": 0})), false)),
        Some((k, _)) => Ok((Value::Array(
            rows.iter()
                .map(|row| {
                    let mut out = render(row);
                    out.insert("key".into(), group_key(k, row.get("_g")));
                    Value::Object(out)
                })
                .collect(),
        ), cut)),
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
        json!({"name":"total","in":"query","description":"`false` skips the count and leaves `total` out of the response","schema":{"type":"string","enum":["false"]}}),
        json!({"name":"sort","in":"query","description":format!("Comma-separated, '-' for descending: {}", sort_names(r)),"schema":{"type":"string"}}),
        json!({"name":"fields","in":"query","description":"Comma-separated data fields each item keeps; id and timestamps always stay","schema":{"type":"string"}}),
    ];
    for (field, ops) in &r.query.filter {
        for op in ops {
            let name = if op == "eq" { field.clone() } else { format!("{field}[{op}]") };
            params.push(json!({"name": name, "in":"query", "schema":{"type":"string"}}));
        }
    }
    params.push(json!({"name":"owner","in":"query","description":"`me` limits the list to the reader's own rows; requires a session or key","schema":{"type":"string","enum":["me"]}}));
    for (group, branches) in &r.query.or {
        for (field, ops) in branches {
            for op in ops {
                let name = if op == "eq" { format!("{group}.{field}") } else { format!("{group}.{field}[{op}]") };
                params.push(json!({"name": name, "in":"query", "description": format!("Alternative of `{group}`: at least one sent branch must hold"), "schema":{"type":"string"}}));
            }
        }
    }
    for (key, list) in [("include", r.query.include.clone()), ("aggregate", r.query.aggregate.keys().cloned().collect()), ("count", r.query.count.keys().cloned().collect())] {
        if !list.is_empty() {
            params.push(json!({"name": key, "in":"query", "description": if key == "include" { format!("Comma-separated: {}. Dotted paths follow references up to {} levels", list.join(", "), crate::relations::MAX_INCLUDE_DEPTH) } else { format!("Comma-separated: {}", list.join(", ")) }, "schema":{"type":"string"}}));
        }
    }
    if !r.query.facets.is_empty() {
        params.push(json!({"name":"facets","in":"query","description":format!("Comma-separated: {}. Adds `facets` with the count of each value over the rows you may list, ignoring this field's own filters", r.query.facets.join(", ")),"schema":{"type":"string"}}));
    }
    params
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
    async fn full_text_probe_stops_at_one_visible_filtered_match() {
        use sqlx::ConnectOptions;
        let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Owned disposable Postgres harness required");
        let options = url.parse::<sqlx::postgres::PgConnectOptions>().unwrap().disable_statement_logging();
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        sqlx::raw_sql("CREATE EXTENSION IF NOT EXISTS unaccent;
            CREATE TEMP TABLE _brain_res_probe(owner uuid, query_keys jsonb, search_vector tsvector);
            INSERT INTO _brain_res_probe SELECT '00000000-0000-0000-0000-000000000001'::uuid,
                '{\"category\":\"visible\"}'::jsonb, to_tsvector('simple','needle') FROM generate_series(1,2048);
            ANALYZE _brain_res_probe;").execute(&mut *tx).await.unwrap();
        let r = Resource {
            name: "probe".into(), search: vec!["title".into()], fields: Default::default(),
            policy: Default::default(), hooks: Default::default(), constraints: Default::default(), query: Default::default(),
        };
        let resources = BTreeMap::from([(r.name.clone(), r.clone())]);
        let scope = Scope::Owned { user: Uuid::from_u128(1), direct: true, paths: vec![] };
        let request = ListRequest { search: "needle".into(), filters: vec![Filter {
            via: None, field: "category".into(), kind: Kind::Text, op: "eq".into(),
            values: vec![Operand::Value("visible".into())],
        }], ..Default::default() };
        let mut ctx = Ctx { r: &r, resources: &resources, scope: &scope, related: BTreeMap::new(),
            req: &request, matches: Matches::FullText, owner: None, facet: None, read_once: false };
        use sqlx::Execute;
        let mut probe = ctx.match_probe();
        let sql = format!("EXPLAIN (ANALYZE, FORMAT JSON) {}", probe.sql().as_str());
        let mut query = probe.build();
        let args = query.take_arguments().unwrap().unwrap();
        // Prefix only the contract-built query; request values remain its original bound arguments.
        let plan: Value = sqlx::query_scalar_with(sqlx::AssertSqlSafe(sql), args)
            .fetch_one(&mut *tx).await.unwrap();
        fn scanned_rows(plan: &Value) -> f64 {
            let own = if plan["Node Type"].as_str().is_some_and(|kind| kind.ends_with("Scan")) {
                plan["Actual Rows"].as_f64().expect("EXPLAIN scan row count")
            } else { 0.0 };
            own + plan["Plans"].as_array().map_or(0.0, |children| children.iter().map(scanned_rows).sum())
        }
        assert_eq!(scanned_rows(&plan[0]["Plan"]), 1.0, "the search decision must not count every match: {plan}");
        assert!(ctx.has_matches(&mut tx).await.unwrap());
        let hidden = Scope::Owned { user: Uuid::from_u128(2), direct: true, paths: vec![] };
        ctx.scope = &hidden;
        assert!(!ctx.has_matches(&mut tx).await.unwrap(), "another owner's full-text matches must not suppress typo fallback");
        ctx.scope = &scope;
        let mut filtered = request.clone();
        filtered.filters[0].values = vec![Operand::Value("other".into())];
        ctx.req = &filtered;
        assert!(!ctx.has_matches(&mut tx).await.unwrap(), "the probe must retain declared filters");
        tx.rollback().await.unwrap();
        pool.close().await;
    }
    #[test]
    fn a_short_text_value_is_compared_by_its_key_alone() {
        let filter = |value: String| Filter { via: None, field: "title".into(), kind: Kind::Text, op: "eq".into(), values: vec![Operand::Value(value)] };
        let sql = |value: String| {
            let mut q = QueryBuilder::<Postgres>::new("");
            push_predicate(&mut q, &filter(value));
            q.sql().as_str().to_string()
        };
        assert!(!sql("short".into()).contains("(data->>'title')="), "{}", sql("short".into()));
        // Only a value that fills the key can match a longer text, so only it needs the stored text read again.
        assert!(sql("x".repeat(TEXT_KEY)).contains("(data->>'title')="));
    }
    #[test]
    fn fields_read_only_what_the_answer_and_its_includes_use() {
        let request = ListRequest { fields: Some(BTreeSet::from(["title".to_string()])), include: vec!["shop.region".into()], ..Default::default() };
        assert_eq!(request.stored_keys(), Some(vec!["shop".to_string(), "title".to_string()]));
        assert_eq!(ListRequest::default().stored_keys(), None);
    }
    #[test]
    fn near_boxes_split_at_the_antimeridian_and_open_at_the_poles() {
        use super::near_boxes;
        assert_eq!(near_boxes(0.0, 0.0, 100.0).len(), 1);
        let split = near_boxes(10.0, 179.9, 100.0);
        assert_eq!(split.len(), 2);
        assert!(split[0][0] > 170.0 && split[0][2] == 180.0 && split[1][0] == -180.0 && split[1][2] < -170.0, "{split:?}");
        let split = near_boxes(10.0, -179.9, 100.0);
        assert!(split.len() == 2 && split[0][2] == 180.0 && split[1][0] == -180.0, "{split:?}");
        let pole = near_boxes(89.9, 0.0, 50.0);
        assert_eq!((pole.len(), pole[0][0], pole[0][2], pole[0][3]), (1, -180.0, 180.0, 90.0));
        let wide = near_boxes(0.0, 0.0, 20040.0);
        assert_eq!((wide[0][0], wide[0][1], wide[0][2], wide[0][3]), (-180.0, -90.0, 180.0, 90.0));
    }
    #[test]
    fn timestamps_order_without_a_null_position_so_their_index_serves_the_list() {
        let key = |kind, field: &str, nulls_first| SortKey { field: field.into(), kind, desc: true, nulls_first };
        assert_eq!(order_term(&key(Kind::Stamp, "created_at", false)), "created_at DESC");
        assert_eq!(order_term(&key(Kind::Stamp, "updated_at", true)), "updated_at DESC");
        assert_eq!(order_term(&key(Kind::Int, "stock", false)), "(query_keys->'stock')::numeric DESC NULLS LAST");
    }
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
    fn filters_and_sorts_read_the_inline_query_keys_not_the_stored_document() {
        // `data` lives out of line once a document is large: a predicate or sort key that reads it decompresses the whole
        // document of every row it visits. Only a long text value's exact recheck may read it, and only for that row.
        let mut kinds = vec![Kind::Int, Kind::Decimal, Kind::Date, Kind::DateTime, Kind::Text, Kind::Bool, Kind::List, Kind::Geo];
        for k in kinds.drain(..) {
            assert!(!expr(k, "f").contains("data"), "{k:?}: {}", expr(k, "f"));
            assert!(!order_term(&SortKey { field: "f".into(), kind: k, desc: true, nulls_first: false }).contains("data"));
        }
        let sql = |kind: Kind, op: &str, value: &str| {
            let mut q = QueryBuilder::<Postgres>::new("");
            push_predicate(&mut q, &Filter { via: None, field: "f".into(), kind, op: op.into(), values: vec![Operand::Value(value.into())] });
            q.sql().as_str().to_string()
        };
        for (kind, op, value) in [(Kind::Text, "eq", "a"), (Kind::Text, "in", "a"), (Kind::Text, "prefix", "a"), (Kind::Text, "iprefix", "a"), (Kind::Text, "exists", "false"), (Kind::Int, "gte", "1"), (Kind::DateTime, "gte", "2026-01-01"), (Kind::List, "has", "a"), (Kind::Geo, "near", "1,2,3")] {
            let sql = sql(kind, op, value);
            assert!(!sql.contains("data"), "{kind:?} {op}: {sql}");
        }
    }
    #[test]
    fn like_prefixes_escape_wildcards() {
        assert_eq!(escape_like("a_%\\"), "a\\_\\%\\\\%");
    }
}
