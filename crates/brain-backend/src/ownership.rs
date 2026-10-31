//! Row scopes for reads and relational ownership (B-09/B-21).
//!
//! A policy grant `owner.<ref>` (or `owner.<ref>.<ref>`, at most two hops) means: the reader owns the record that
//! this row references. Example: `comments.post -> posts.board -> boards.owner`, so the grant
//! `owner.post.board` lets a board owner read the comments on its posts. Scopes compile to one bound SQL
//! predicate (nested `IN (SELECT id ...)` over indexed foreign-key columns), applied to lists, counts, aggregates,
//! includes and single-record checks alike.
use crate::{Resource, relations, visibility::Viewer};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Which rows of a resource the current reader may see or change.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    All,
    /// Rows owned by `user` directly (`owner`) and/or through declared relation paths (`owner.a.b`).
    Owned { user: Uuid, direct: bool, paths: Vec<Vec<String>> },
    /// OR of read grants of which at least one carries a row condition (`{ public: { where: {...} } }`).
    Rows(Vec<Branch>),
}
/// One held read grant: who it covers, and the row conditions it adds.
#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    pub who: Who,
    pub conditions: Vec<(String, serde_json::Value)>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Who {
    Anyone,
    Owner(Uuid),
    Through(Uuid, Vec<String>),
}

/// Parses `owner.<ref>[.<ref>]`; None for every other grant.
pub fn path(grant: &str) -> Option<Vec<String>> {
    let rest = grant.strip_prefix("owner.")?;
    let parts: Vec<String> = rest.split('.').map(str::to_string).collect();
    (!parts.is_empty() && parts.len() <= 2 && parts.iter().all(|p| crate::contract::identifier(p))).then_some(parts)
}

fn grants<'a>(r: &'a Resource, operation: &str) -> &'a [String] {
    match operation {
        "read" => &r.policy.read,
        "create" => &r.policy.create,
        "update" => &r.policy.update,
        _ => &r.policy.delete,
    }
}

/// The scope a viewer has for an operation, or None when the policy denies it entirely.
pub fn scope(r: &Resource, operation: &str, viewer: Viewer<'_>) -> Option<Scope> {
    if crate::api_keys::system_active() {
        return Some(Scope::All);
    }
    if operation == "read" && r.policy.conditioned() {
        return conditioned_scope(r, viewer);
    }
    if r.policy.allows(operation, viewer.role, false) {
        return Some(Scope::All);
    }
    let user = viewer.id?;
    let grants = grants(r, operation);
    let direct = r.policy.allows(operation, viewer.role, true);
    let paths: Vec<Vec<String>> = grants.iter().filter_map(|g| path(g)).collect();
    (direct || !paths.is_empty()).then_some(Scope::Owned { user, direct, paths })
}
/// The grants a viewer holds on a resource with conditioned reads. Any unconditioned general grant wins outright.
fn conditioned_scope(r: &Resource, viewer: Viewer<'_>) -> Option<Scope> {
    let mut branches = Vec::new();
    for grant in &r.policy.read {
        let who = match (grant.as_str(), path(grant)) {
            ("public", _) => Who::Anyone,
            ("owner", _) => match (viewer.id, viewer.role) {
                (Some(user), Some(_)) => Who::Owner(user),
                _ => continue,
            },
            (_, Some(hops)) => match viewer.id {
                Some(user) => Who::Through(user, hops),
                None => continue,
            },
            (role, None) if viewer.role == Some(role) => Who::Anyone,
            _ => continue,
        };
        let conditions: Vec<_> = r.policy.read_where.get(grant).map(|c| c.iter().map(|(f, v)| (f.clone(), v.clone())).collect()).unwrap_or_default();
        if who == Who::Anyone && conditions.is_empty() {
            return Some(Scope::All);
        }
        branches.push(Branch { who, conditions });
    }
    (!branches.is_empty()).then_some(Scope::Rows(branches))
}
/// The single evaluator of "may this viewer do `operation` on this existing row": role, `owner`, `owner.<ref>` and
/// row conditions, exactly as lists, get, sockets and related files apply them.
pub async fn row_allowed(pool: &sqlx::PgPool, r: &Resource, resources: &BTreeMap<String, Resource>, operation: &str, viewer: Viewer<'_>, id: Uuid) -> Result<bool, sqlx::Error> {
    let Some(scope) = scope(r, operation, viewer) else { return Ok(false) };
    exists(pool, r, resources, &scope, id).await
}
async fn exists(pool: &sqlx::PgPool, r: &Resource, resources: &BTreeMap<String, Resource>, scope: &Scope, id: Uuid) -> Result<bool, sqlx::Error> {
    let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT EXISTS(SELECT 1 FROM _brain_res_");
    q.push(&r.name).push(" WHERE id=").push_bind(id).push(" AND ");
    scope.push(&mut q, r, resources);
    q.push(")");
    q.build_query_scalar::<bool>().fetch_one(pool).await
}
/// The relational (`owner.<ref>`) part of a viewer's read scope, used only for `owner` field visibility.
pub fn relational_read(r: &Resource, viewer: Viewer<'_>) -> Option<Scope> {
    let user = viewer.id?;
    if r.policy.conditioned() {
        let Some(Scope::Rows(branches)) = scope(r, "read", viewer) else { return None };
        let branches: Vec<Branch> = branches.into_iter().filter(|b| matches!(b.who, Who::Through(..))).collect();
        return (!branches.is_empty()).then_some(Scope::Rows(branches));
    }
    let paths: Vec<Vec<String>> = r.policy.read.iter().filter_map(|g| path(g)).collect();
    (!paths.is_empty()).then_some(Scope::Owned { user, direct: false, paths })
}
pub fn read_scope(r: &Resource, _resources: &BTreeMap<String, Resource>, viewer: Viewer<'_>) -> Option<Scope> {
    if !crate::api_keys::scopes_read(&r.name, r.policy.api) {
        return None;
    }
    scope(r, "read", viewer)
}

/// Validates relational grants against the resource graph: every hop must be a `ref:` field.
pub fn validate_graph(r: &Resource, resources: &[Resource]) -> Result<(), String> {
    for (operation, list) in [("read", &r.policy.read), ("create", &r.policy.create), ("update", &r.policy.update), ("delete", &r.policy.delete)] {
        for grant in list.iter().filter(|g| g.starts_with("owner.")) {
            let hops = path(grant).ok_or_else(|| format!("{}: invalid {operation} grant `{grant}`; fix: use owner.<ref field> or owner.<ref field>.<ref field>", r.name))?;
            let mut current = r;
            for hop in &hops {
                let target = relations::target(current, hop).ok_or_else(|| {
                    format!("{}: {operation} grant `{grant}`: `{hop}` is not a ref:<resource> field of {}", r.name, current.name)
                })?;
                current = resources.iter().find(|x| x.name == target).ok_or_else(|| format!("{}: grant `{grant}` reaches unknown resource `{target}`", r.name))?;
            }
        }
    }
    Ok(())
}

/// Checks one `owner.<ref>[.<ref>]` grant of an action or transition against the resource graph.
pub fn check_path(r: &Resource, grant: &str, resources: &[Resource]) -> Result<(), String> {
    let hops = path(grant).ok_or_else(|| format!("invalid grant `{grant}`; fix: use owner.<ref field> or owner.<ref field>.<ref field>"))?;
    let mut current = r;
    for hop in &hops {
        let target = relations::target(current, hop).ok_or_else(|| format!("grant `{grant}`: `{hop}` is not a ref:<resource> field of {}", current.name))?;
        current = resources.iter().find(|x| x.name == target).ok_or_else(|| format!("grant `{grant}` reaches unknown resource `{target}`"))?;
    }
    Ok(())
}

impl Scope {
    /// Pushes a boolean SQL expression over the current table's unqualified columns.
    pub fn push(&self, q: &mut sqlx::QueryBuilder<sqlx::Postgres>, r: &Resource, resources: &BTreeMap<String, Resource>) {
        match self {
            Scope::All => {
                q.push("true");
            }
            Scope::Owned { user, direct, paths } => {
                q.push("(false");
                if *direct {
                    q.push(" OR owner=").push_bind(*user);
                }
                for hops in paths {
                    q.push(" OR ");
                    push_path(q, r, resources, hops, *user);
                }
                q.push(")");
            }
            Scope::Rows(branches) => {
                q.push("(false");
                for branch in branches {
                    q.push(" OR (");
                    match &branch.who {
                        Who::Anyone => {
                            q.push("true");
                        }
                        Who::Owner(user) => {
                            q.push("owner=").push_bind(*user);
                        }
                        Who::Through(user, hops) => push_path(q, r, resources, hops, *user),
                    }
                    for (field, value) in &branch.conditions {
                        q.push(" AND ");
                        crate::query::push_row_condition(q, r, field, value);
                    }
                    q.push(")");
                }
                q.push(")");
            }
        }
    }
}
fn push_path(
    q: &mut sqlx::QueryBuilder<sqlx::Postgres>,
    r: &Resource,
    resources: &BTreeMap<String, Resource>,
    hops: &[String],
    user: Uuid,
) {
    let Some((hop, rest)) = hops.split_first() else {
        q.push("owner=").push_bind(user);
        return;
    };
    let Some(target) = relations::target(r, hop).and_then(|t| resources.get(&t)) else {
        // Unreachable after validation; fail closed.
        q.push("false");
        return;
    };
    // Identifiers are validated contract names; the user id is bound.
    q.push(relations::column(hop))
        .push(" IN (SELECT id FROM _brain_res_")
        .push(&target.name)
        .push(" WHERE ");
    push_path(q, target, resources, rest, user);
    q.push(")");
}

/// Whether the viewer owns this existing row through a relation path of `operation`'s grants.
pub async fn owns_through(
    pool: &sqlx::PgPool,
    r: &Resource,
    resources: &BTreeMap<String, Resource>,
    operation: &str,
    viewer: Viewer<'_>,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    if operation == "read" && r.policy.conditioned() {
        return match relational_read(r, viewer) {
            Some(relational) => exists(pool, r, resources, &relational, id).await,
            None => Ok(false),
        };
    }
    let Some(Scope::Owned { user, paths, .. }) = scope(r, operation, viewer) else {
        return Ok(false);
    };
    if paths.is_empty() {
        return Ok(false);
    }
    exists(pool, r, resources, &Scope::Owned { user, direct: false, paths }, id).await
}

/// Whether a new or replacement record value keeps the viewer's relational ownership (prevents re-pointing a row
/// to a parent the viewer does not own). Evaluated on the submitted reference ids before the write.
pub async fn data_owned_through(
    pool: &sqlx::PgPool,
    r: &Resource,
    resources: &BTreeMap<String, Resource>,
    operation: &str,
    viewer: Viewer<'_>,
    data: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let Some(user) = viewer.id else { return Ok(false) };
    let paths: Vec<Vec<String>> = grants(r, operation).iter().filter_map(|g| path(g)).collect();
    for hops in paths {
        let (first, rest) = hops.split_first().expect("validated non-empty path");
        let (Some(id), Some(target)) = (
            data[first.as_str()].as_str().and_then(|s| Uuid::parse_str(s).ok()),
            relations::target(r, first).and_then(|t| resources.get(&t)),
        ) else {
            continue;
        };
        let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT EXISTS(SELECT 1 FROM _brain_res_");
        q.push(&target.name).push(" WHERE id=").push_bind(id).push(" AND ");
        push_path(&mut q, target, resources, rest, user);
        q.push(")");
        if q.build_query_scalar::<bool>().fetch_one(pool).await? {
            return Ok(true);
        }
    }
    Ok(false)
}
