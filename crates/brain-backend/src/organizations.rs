//! Organizations and memberships (multi-tenant isolation).
//!
//! A resource becomes tenant data by declaring exactly one required `org` field. From then on its policy grants are
//! `org` (any member of the row's organization), `org.<role>` (members holding that role) and `org.own` (a member who
//! also owns the row). They compile into the row scope (`ownership.rs`), so lists, single reads, counts, aggregates,
//! includes, sockets and related files apply them through one predicate, and global roles cannot cross organizations.
use crate::{Resource, types::Ty};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::BTreeMap;
use uuid::Uuid;

pub const CONTRACT: &str = "server/auth/organizations.jsbrain";
pub const MAX_ROLES: usize = 16;
pub const INVITE_DAYS: i32 = 7;

/// Membership roles in order, most privileged first. The first role founds an organization.
#[derive(Clone, Debug, PartialEq)]
pub struct Organizations {
    pub roles: Vec<String>,
    /// Roles that may manage memberships and invitations.
    pub managers: Vec<String>,
}
impl Default for Organizations {
    fn default() -> Self {
        Self { roles: ["owner", "admin", "member"].map(String::from).to_vec(), managers: ["owner", "admin"].map(String::from).to_vec() }
    }
}
impl Organizations {
    /// `{ roles: [...], managers: [...] }`; `managers` defaults to every role but the last.
    pub fn from_contract(value: &Value) -> Result<Self, String> {
        let map = value.as_object().ok_or_else(|| format!("{CONTRACT}: expected an object; fix: export {{ roles: ['owner', 'admin', 'member'] }}"))?;
        if let Some(key) = map.keys().find(|k| !matches!(k.as_str(), "roles" | "managers")) {
            return Err(format!("{CONTRACT}: unknown key `{key}`; fix: use roles and managers"));
        }
        let list = |key: &str| -> Result<Option<Vec<String>>, String> {
            let Some(value) = map.get(key) else { return Ok(None) };
            let items = value.as_array().ok_or_else(|| format!("{CONTRACT}: {key} must be an array of role names"))?;
            items.iter().map(|v| v.as_str().map(String::from).ok_or_else(|| format!("{CONTRACT}: {key} must be an array of role names"))).collect::<Result<Vec<_>, _>>().map(Some)
        };
        let roles = list("roles")?.ok_or_else(|| format!("{CONTRACT}: roles is required; fix: list membership roles, most privileged first"))?;
        let managers = list("managers")?.unwrap_or_else(|| roles[..roles.len().saturating_sub(1).max(1).min(roles.len())].to_vec());
        let out = Self { roles, managers };
        out.validate()?;
        Ok(out)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.roles.is_empty() || self.roles.len() > MAX_ROLES {
            return Err(format!("{CONTRACT}: declare 1..{MAX_ROLES} membership roles"));
        }
        if let Some(role) = self.roles.iter().find(|r| !usable_role(r)) {
            return Err(format!("{CONTRACT}: role `{role}` is not usable; fix: a lowercase identifier of at most 48 characters, other than own"));
        }
        if self.roles.iter().collect::<std::collections::BTreeSet<_>>().len() != self.roles.len() {
            return Err(format!("{CONTRACT}: roles repeat a name; fix: list each role once"));
        }
        if self.managers.is_empty() || self.managers.iter().any(|m| !self.roles.contains(m)) {
            return Err(format!("{CONTRACT}: managers must name declared roles; fix: list at least one role from roles"));
        }
        if !self.managers.contains(&self.roles[0]) {
            return Err(format!("{CONTRACT}: `{}` founds every organization and must be a manager; fix: add it to managers", self.roles[0]));
        }
        Ok(())
    }
    pub fn rank(&self, role: &str) -> Option<usize> {
        self.roles.iter().position(|r| r == role)
    }
    pub fn founder(&self) -> &str {
        &self.roles[0]
    }
    pub fn manages(&self, role: &str) -> bool {
        self.managers.iter().any(|m| m == role)
    }
    /// An actor may touch a role at or below their own rank.
    pub fn outranks_or_equals(&self, actor: &str, target: &str) -> bool {
        matches!((self.rank(actor), self.rank(target)), (Some(a), Some(t)) if a <= t)
    }
}
fn usable_role(role: &str) -> bool {
    let mut bytes = role.bytes();
    role.len() <= 48 && role != "own" && bytes.next().is_some_and(|c| c.is_ascii_lowercase()) && bytes.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// A policy grant over memberships.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant<'a> {
    Member,
    Own,
    Role(&'a str),
}
pub fn grant(name: &str) -> Option<Grant<'_>> {
    match name {
        "org" => Some(Grant::Member),
        "org.own" => Some(Grant::Own),
        _ => name.strip_prefix("org.").filter(|role| usable_role(role)).map(Grant::Role),
    }
}
/// The field that names a row's organization, if the resource is tenant data.
pub fn tenant(r: &Resource) -> Option<&str> {
    r.fields.iter().find(|(_, f)| matches!(f.ty(), Some(Ty::Org))).map(|(name, _)| name.as_str())
}
/// True when membership rules decide this request. A system key answers to no organization.
pub fn enforced(r: &Resource) -> bool {
    tenant(r).is_some() && !crate::api_keys::system_active()
}

fn all_grants(r: &Resource) -> [(&'static str, &Vec<String>); 4] {
    [("read", &r.policy.read), ("create", &r.policy.create), ("update", &r.policy.update), ("delete", &r.policy.delete)]
}
/// Rules of one resource: `org` fields, and grants that could cross an organization.
pub fn validate_resource(r: &Resource) -> Result<(), String> {
    let fields: Vec<_> = r.fields.iter().filter(|(_, f)| matches!(f.ty(), Some(Ty::Org))).collect();
    if fields.len() > 1 {
        return Err(format!("{}: declare one org field; a row belongs to exactly one organization", r.name));
    }
    let Some((name, field)) = fields.first() else {
        for (op, grants) in all_grants(r) {
            if let Some(g) = grants.iter().find(|g| g.starts_with("org.")) {
                return Err(format!("{}: {op} grant `{g}` needs tenant data; fix: add a required field of type org to the resource", r.name));
            }
        }
        return Ok(());
    };
    if !field.required || field.default.is_some() || field.unique || field.visible.is_some() || field.access.is_some() {
        return Err(format!("{}.{name}: an org field must be required and public to its readers; fix: {{ type: 'org!' }} with no default, unique, visible or access", r.name));
    }
    if crate::slugs::field(r).is_some() {
        return Err(format!("{}: slug history is global, so it would reveal another organization's records; fix: remove slug: true and use a unique field", r.name));
    }
    for (op, grants) in all_grants(r) {
        for g in grants {
            let allowed = grant(g).is_some() || g == "public" && op == "read";
            if !allowed {
                return Err(format!("{}: {op} grant `{g}` would cross organizations; fix: use org, org.<membership role> or org.own (a system API key serves support tooling)", r.name));
            }
            if op == "create" && g == "org.own" {
                return Err(format!("{}: create grant `org.own` has no row to own yet; fix: use org or org.<membership role>", r.name));
            }
        }
    }
    if r.policy.read.iter().any(|g| g == "public") && !r.policy.read_where.contains_key("public") && !r.policy.expose_all_rows {
        return Err(format!("{}: public read of tenant data exposes every organization; fix: add a row condition or declare expose_all_rows", r.name));
    }
    Ok(())
}
/// A resource shared between organizations must not point into one: its references would prove a tenant row exists.
pub fn validate_graph(resources: &[Resource]) -> Result<(), String> {
    for r in resources.iter().filter(|r| tenant(r).is_none()) {
        for reference in crate::relations::refs(r) {
            if resources.iter().any(|t| t.name == reference.target && tenant(t).is_some()) {
                return Err(format!("{}: field `{}` references tenant data in `{}` from a resource without organizations; fix: add an org field to {} or remove the reference", r.name, reference.field, reference.target, r.name));
            }
        }
    }
    Ok(())
}
/// `org.<role>` grants must name a declared membership role, and no global role may read like a grant.
pub fn validate_roles(resources: &[&Resource], config: &Organizations, global: &[String]) -> Result<(), String> {
    config.validate()?;
    let any_tenant = resources.iter().any(|r| tenant(r).is_some());
    if any_tenant && global.iter().any(|role| role == "org") {
        return Err("server/auth/roles.jsbrain: role `org` collides with the membership grant of tenant resources; fix: rename the role".into());
    }
    for r in resources {
        for (op, grants) in all_grants(r) {
            for g in grants {
                if let Some(Grant::Role(role)) = grant(g)
                    && config.rank(role).is_none()
                {
                    return Err(format!("{}: {op} grant `{g}` names no membership role; fix: declare `{role}` in {CONTRACT} or use one of {}", r.name, config.roles.join(", ")));
                }
            }
        }
    }
    Ok(())
}
fn trigger_args(r: &Resource, field: &str, resources: &BTreeMap<&str, &Resource>) -> Vec<String> {
    let mut args = vec![field.to_string()];
    for reference in crate::relations::refs(r) {
        if let Some(target_field) = resources.get(reference.target.as_str()).and_then(|t| tenant(t)) {
            args.extend([reference.field.to_string(), crate::relations::table(&reference.target), target_field.to_string(), crate::relations::fk_name(r, &reference)]);
        }
    }
    args
}
/// Installs the row trigger: the organization never changes and references stay inside it.
pub(crate) async fn install(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, r: &Resource, all: &[Resource]) -> Result<(), String> {
    let table = crate::relations::table(&r.name);
    let Some(field) = tenant(r) else {
        // Audited SQL: the table name is a validated contract identifier.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TRIGGER IF EXISTS _brain_org ON {table}"))).execute(&mut **tx).await.map_err(crate::db::safe)?;
        return Ok(());
    };
    let index: BTreeMap<&str, &Resource> = all.iter().map(|x| (x.name.as_str(), x)).collect();
    let args = trigger_args(r, field, &index).iter().map(|a| format!("'{a}'")).collect::<Vec<_>>().join(",");
    // Audited SQL: every argument is a validated identifier or a generated object name; no request value is interpolated.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER IF EXISTS _brain_org ON {table}; CREATE TRIGGER _brain_org BEFORE INSERT OR UPDATE ON {table} FOR EACH ROW EXECUTE FUNCTION public._brain_org_row({args});"
    )))
    .execute(&mut **tx)
    .await
    .map_err(crate::db::safe)?;
    Ok(())
}
/// Serving refuses a tenant table whose isolation trigger is missing, disabled or different.
pub(crate) async fn verify(conn: &mut sqlx::PgConnection, r: &Resource, all: &[Resource]) -> Result<(), String> {
    let Some(field) = tenant(r) else { return Ok(()) };
    let table = crate::relations::table(&r.name);
    let index: BTreeMap<&str, &Resource> = all.iter().map(|x| (x.name.as_str(), x)).collect();
    let args: Vec<u8> = trigger_args(r, field, &index).iter().flat_map(|a| a.as_bytes().iter().copied().chain(std::iter::once(0))).collect();
    let present: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_trigger WHERE tgrelid=to_regclass($1) AND tgname='_brain_org' AND tgenabled='O' AND tgfoid='public._brain_org_row()'::regprocedure AND tgargs=$2)")
        .bind(format!("public.{table}"))
        .bind(args)
        .fetch_one(&mut *conn)
        .await
        .map_err(crate::db::safe)?;
    if !present {
        return Err(format!("BDB_SCHEMA_DRIFT: {table} organization trigger; review schema and run the explicit backend migration; no rows were removed"));
    }
    Ok(())
}
/// Whether this user may create a row in `org`: any create grant they hold through a membership there.
pub async fn may_create(pool: &PgPool, r: &Resource, user: Uuid, org: Uuid) -> Result<bool, sqlx::Error> {
    let mut any = false;
    let mut roles = Vec::new();
    for g in &r.policy.create {
        match grant(g) {
            Some(Grant::Member) => any = true,
            Some(Grant::Role(role)) => roles.push(role.to_string()),
            _ => {}
        }
    }
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _brain_org_members WHERE org_id=$1 AND user_id=$2 AND ($3 OR role=ANY($4)))")
        .bind(org)
        .bind(user)
        .bind(any)
        .bind(roles)
        .fetch_one(pool)
        .await
}
/// The organization named by a validated record.
pub fn org_of(r: &Resource, data: &Value) -> Option<Uuid> {
    data.get(tenant(r)?)?.as_str().and_then(|s| Uuid::parse_str(s).ok())
}
/// A record keeps its organization: a replacement or merge naming another is a client error.
pub fn unchanged(r: &Resource, previous: &Value, data: &Value) -> Result<(), BTreeMap<String, String>> {
    let Some(field) = tenant(r) else { return Ok(()) };
    if previous.get(field) == data.get(field) {
        return Ok(());
    }
    Err(BTreeMap::from([(field.to_string(), "A record cannot move to another organization; create it there instead".to_string())]))
}
