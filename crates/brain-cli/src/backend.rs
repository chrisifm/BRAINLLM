//! Experimental backend CLI. Static BrainScript contracts are parsed by brain-parse, never evaluated.
use brain_backend::{
    Field, Policy, Resource,
    contract::{self, Hooks},
};
use brain_parse::ast::{Expr, ObjProp, PropKey, Stmt};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

#[path = "socket_settings.rs"]
mod socket_configuration;
pub use socket_configuration::{socket_settings, SocketSettings};

fn literal(expr: &Expr) -> Result<Value, String> {
    match expr.unparen() {
        Expr::Str{value,..}=>Ok(json!(value)), Expr::Bool{value,..}=>Ok(json!(value)), Expr::Null(_)=>Ok(Value::Null),
        Expr::Num{raw,..}=>number(raw,false),
        Expr::Unary{op:"-",arg,..}=>match arg.unparen() {Expr::Num{raw,..}=>number(raw,true),_=>Err("Contract numbers must be plain decimal literals".into())},
        Expr::Object{props,..}=>{
            let mut obj=serde_json::Map::new();
            for p in props {
                let ObjProp::KeyValue{key,value,..}=p else{return Err("Contract objects allow only named literal properties".into())};
                let name=match key {PropKey::Ident(i)=>&i.name,PropKey::Str{value,..}=>value,_=>return Err("Contract property names must be identifiers or strings".into())};
                if obj.insert(name.clone(),literal(value)?).is_some(){return Err(format!("Duplicate contract property `{name}`"))}
            }
            Ok(Value::Object(obj))
        },
        Expr::Array{elems,..}=>elems.iter().map(|e|e.as_ref().ok_or_else(||"Contract arrays cannot have holes".into()).and_then(literal)).collect::<Result<Vec<_>,_>>().map(Value::Array),
        _=>Err("Contract must export one literal object; no function calls, imports or computed values".into()),
    }
}
/// Plain JSON-compatible decimal literals only; integers outside i64 are refused instead of rounded.
fn number(raw: &str, negative: bool) -> Result<Value, String> {
    let text = if negative { format!("-{raw}") } else { raw.to_string() };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Number(n)) if !(n.is_f64() && raw.bytes().all(|c| c.is_ascii_digit())) => Ok(Value::Number(n)),
        _ => Err(format!("Contract number `{text}` must be a plain decimal literal within the 64-bit integer range; write exact decimals as strings")),
    }
}
#[path = "backend_domain.rs"]
pub(crate) mod domain;
pub(crate) fn read_contract(root: &Path, path: &Path) -> Result<Value, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Contract must be inside the app and <=64 KiB")?
        .to_str()
        .ok_or("Contract path must be UTF-8")?;
    crate::changes::source_path(root, relative).map_err(|error| format!("{relative}: {error}"))?;
    let src = crate::map::read_source_limit(root, relative, 65536).map_err(|error| {
        format!("{relative}: contract must be a regular app source file <=64 KiB ({error})")
    })?;
    let c = brain_parse::parse(&src, Path::new(relative)).map_err(|d| crate::diags_json(&d))?;
    let result = match c.script.as_slice() {
        [Stmt::ExportDefault { expr, .. }] => literal(expr),
        _ => Err(
            "Contract must contain only `export default { ... }` (optional <meta server>)".into(),
        ),
    };
    result.map_err(|message| {
        format!(
            "{}: {message}",
            path.strip_prefix(root).unwrap_or(path).display()
        )
    })
}
fn fields(value: Value) -> Result<BTreeMap<String, Field>, String> {
    let obj = value
        .as_object()
        .ok_or("Schema must be an object of field:type pairs")?;
    obj.iter()
        .map(|(name, v)| {
            // `'int!'` or the object form `{ type: 'int!', default: 0, unique: true, on_delete, visible }`.
            let mut on_delete = None;
            let mut visible = None;
            let mut access = None;
            let mut pattern = None;
            let mut slug = false;
            let (kind, default, unique) = match v {
                // `'str! private'`: an optional access modifier after the type.
                Value::String(kind) => match kind.rsplit_once(' ').filter(|(_, m)| matches!(*m, "private" | "readonly" | "writeonly")) {
                    Some((kind, modifier)) => {
                        access = Some(brain_backend::contract::FieldAccess::Mode(modifier.to_string()));
                        (kind.trim_end(), None, false)
                    }
                    None => (kind.as_str(), None, false),
                },
                Value::Object(spec) => {
                    if let Some(key) = spec.keys().find(|k| !matches!(k.as_str(), "type" | "default" | "unique" | "on_delete" | "visible" | "access" | "slug" | "pattern")) {
                        return Err(format!("field `{name}`: unknown option `{key}`; fix: use type, default, unique, on_delete, visible, access, slug and pattern"));
                    }
                    let kind = spec.get("type").and_then(Value::as_str).ok_or_else(|| format!("field `{name}` needs type: 'str' (or another kind)"))?;
                    let unique = match spec.get("unique") {
                        None => false,
                        Some(Value::Bool(b)) => *b,
                        Some(_) => return Err(format!("field `{name}`: unique must be true or false")),
                    };
                    if let Some(action) = spec.get("on_delete") {
                        on_delete = Some(action.as_str().ok_or_else(|| format!("field `{name}`: on_delete must be 'restrict', 'cascade' or 'set_null'"))?.to_string());
                    }
                    match spec.get("slug") {
                        None => {}
                        Some(Value::Bool(b)) => slug = *b,
                        Some(_) => return Err(format!("field `{name}`: slug must be true or false")),
                    }
                    if let Some(text) = spec.get("pattern") {
                        pattern = Some(text.as_str().ok_or_else(|| format!("field `{name}`: pattern must be a regular expression string"))?.to_string());
                    }
                    if let Some(mode) = spec.get("access") {
                        access = Some(serde_json::from_value(mode.clone()).map_err(|_| format!("field `{name}`: access must be 'private', 'readonly', 'writeonly' or {{ write: ['admin'] }}"))?);
                    }
                    if let Some(grants) = spec.get("visible") {
                        visible = Some(serde_json::from_value(grants.clone()).map_err(|_| format!("field `{name}`: visible must be a list such as ['owner', 'admin']"))?);
                    }
                    (kind, spec.get("default").cloned(), unique)
                }
                _ => return Err("Field shape must be a string or { type, default, unique, on_delete, visible, access, slug, pattern }".into()),
            };
            Ok((
                name.clone(),
                Field {
                    kind: kind.trim_end_matches('!').into(),
                    required: kind.ends_with('!'),
                    default,
                    unique,
                    on_delete,
                    visible,
                    access,
                    slug,
                    pattern,
                },
            ))
        })
        .collect()
}
pub fn contracts(root: &Path, kind: &str) -> Result<Vec<Resource>, String> {
    let dir = root.join("server").join(kind);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());
    let mut out = Vec::new();
    for e in entries {
        if !e.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        // Legacy server subfolders without resource/job metadata are not silently reinterpreted as new contracts.
        if !["schema.jsbrain", "policy.jsbrain", "hooks.jsbrain"]
            .iter()
            .any(|file| e.path().join(file).is_file())
        {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let fields = fields(read_contract(root, &e.path().join("schema.jsbrain"))?)
            .map_err(|message| format!("server/{kind}/{name}/schema.jsbrain: {message}"))?;
        let policy = if kind == "resources" {
            serde_json::from_value(read_contract(root, &e.path().join("policy.jsbrain"))?).map_err(
                |error| format!("server/{kind}/{name}/policy.jsbrain: {error}"),
            )?
        } else {
            Policy::default()
        };
        let hooks_path = e.path().join("hooks.jsbrain");
        let hooks = if hooks_path.exists() {
            serde_json::from_value(read_contract(root, &hooks_path)?).map_err(|_| {
                format!(
                    "{name}: hooks allow only before_create/before_update/before_delete module names and after_create/after_update/after_delete queue names"
                )
            })?
        } else {
            Hooks::default()
        };
        let search_path = e.path().join("search.jsbrain");
        let search = if kind == "resources" && search_path.is_file() {
            serde_json::from_value(read_contract(root, &search_path)?)
                .map_err(|_| "search.jsbrain must export a list of public string field names")?
        } else {
            Vec::new()
        };
        let constraints_path = e.path().join("constraints.jsbrain");
        let constraints = if kind == "resources" && constraints_path.is_file() {
            serde_json::from_value(read_contract(root, &constraints_path)?).map_err(|_| {
                format!("server/{kind}/{name}/constraints.jsbrain must export {{ unique: [[field, field]], check: [{{ field, min, max }} | {{ compare: [a, op, b] }} | {{ together: [a, b] }}] }}")
            })?
        } else {
            Default::default()
        };
        let query_path = e.path().join("query.jsbrain");
        let query = if kind == "resources" && query_path.is_file() {
            serde_json::from_value(read_contract(root, &query_path)?).map_err(|_| {
                format!("server/{kind}/{name}/query.jsbrain must export {{ filter: {{ field: ['eq', ...] }}, sort: [field | {{ field, nulls }}], default_sort: ['-field'], include: [ref], or: {{ name: {{ field: ['eq'] }} }}, aggregate: {{ name: {{ group, sum, avg, min, max }} }}, count: {{ name: {{ resource, field, where }} }}, facets: [field], search: {{ language, weights: {{ field: 'A' }} }} }}")
            })?
        } else {
            Default::default()
        };
        let r = Resource {
            search,
            name,
            fields,
            policy,
            hooks,
            constraints,
            query,
        };
        if kind == "jobs" { r.validate_job() } else { r.validate() }
            .map_err(|mut message| {
                // The validator knows the schema; only this project boundary knows its declared roles.
                // Implicit legacy role discovery itself reads contracts; only explicit roles avoid that cycle.
                if message.starts_with("invalid ") && message.contains(" policy `")
                    && root.join("server/auth/roles.jsbrain").symlink_metadata().is_ok()
                    && let Ok(roles) = auth_roles(root)
                { message.push_str(&format!("; declared roles: {}", roles.roles.join(", "))); }
                format!("server/{kind}/{}: {message}", r.name)
            })?;
        out.push(r);
    }
    Ok(out)
}
/// Child stderr is untrusted: keep only the shape the job runner reports for a backend client failure.
fn job_backend_failure(raw: &Value) -> Value {
    let Some(status) = raw["status"].as_u64().filter(|status| (400..600).contains(status)) else { return Value::Null };
    let code = raw["code"].as_str().filter(|code| code.len() <= 64 && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'));
    let fields: serde_json::Map<String, Value> = raw["fields"].as_object().into_iter().flatten().take(16)
        .filter_map(|(name, text)| Some((name.chars().take(64).collect(), json!(text.as_str()?.chars().take(160).collect::<String>())))).collect();
    json!({"status":status,"code":code,"fields":fields})
}
/// A job that succeeded after its handler caught backend failures (job-runner.mjs `JOB_BACKEND`): keep only status,
/// code, resource and identifier-shaped field names, so a swallowed 422 reaches the worker log without any value.
fn job_caught_failures(raw: &Value) -> Option<Value> {
    let ident = |s: &str| (1..=48).contains(&s.len()) && s.as_bytes()[0].is_ascii_lowercase() && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_');
    let count = raw["count"].as_u64().filter(|n| *n > 0)?;
    let failures: Vec<Value> = raw["failures"].as_array()?.iter().take(8).filter_map(|failure| {
        let status = failure["status"].as_u64().filter(|status| (400..600).contains(status))?;
        let code = failure["code"].as_str().filter(|code| (1..=64).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'));
        let resource = failure["target"].as_str().filter(|target| match target.split_once('.') {
            Some((area, name)) => matches!(area, "files" | "operation") && ident(name),
            None => ident(target),
        });
        let fields: Vec<&str> = failure["fields"].as_array().into_iter().flatten().take(8)
            .filter_map(Value::as_str).filter(|field| brain_backend::http::loggable_field(field)).collect();
        let times = failure["count"].as_u64().unwrap_or(1).clamp(1, count);
        Some(json!({"status":status,"code":code,"resource":resource,"fields":fields,"count":times}))
    }).collect();
    Some(json!({"count":count,"failures":failures}))
}

pub fn auth_roles(root: &Path) -> Result<brain_backend::auth_flows::RolesConfig, String> {
    use brain_backend::auth_flows::RolesConfig;
    let path = root.join("server/auth/roles.jsbrain");
    let roles = if path.symlink_metadata().is_ok() {
        serde_json::from_value(read_contract(root, &path)?)
            .map_err(|_| "server/auth/roles.jsbrain must declare roles and signup_role; fix: export default { roles: ['admin', 'user'], signup_role: 'user' }")?
    } else if legacy_implicit_roles(root) {
        RolesConfig::legacy_implicit()
    } else {
        RolesConfig::default()
    };
    roles.validate()?;
    Ok(roles)
}
/// No roles file, and a grant names a role only the pre-0.6.2 implicit defaults declared. Such an app keeps those
/// defaults (B0739 asks it to declare them); without this it would stop compiling and its signups would change role.
fn legacy_implicit_roles(root: &Path) -> bool {
    !root.join("server/auth/roles.jsbrain").exists()
        && named_grants(root).iter().any(|(role, _)| {
            role != "admin" && role != "user" && brain_backend::auth_flows::LEGACY_IMPLICIT_ROLES.contains(&role.as_str())
        })
}
/// Every role name a contract grants, with the file that names it: resource policies and field visibility, domain
/// actions, transitions and operations, file buckets and routes. Keywords and `owner.<ref>` paths are not roles.
/// Unreadable contracts are skipped here; their own validation reports them.
fn named_grants(root: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |role: &str, at: String| {
        if !matches!(role, "public" | "owner" | "related") && brain_backend::ownership::path(role).is_none() {
            out.push((role.to_string(), at));
        }
    };
    let resources = contracts(root, "resources").unwrap_or_default();
    for r in &resources {
        for grants in [&r.policy.read, &r.policy.create, &r.policy.update, &r.policy.delete] {
            for role in grants {
                push(role, format!("server/resources/{}/policy.jsbrain", r.name));
            }
        }
        for field in r.fields.values() {
            for role in field.visible.iter().flatten() {
                push(role, format!("server/resources/{}/schema.jsbrain", r.name));
            }
        }
    }
    if let Ok(d) = domain::load(root, &resources) {
        for (name, resource) in &d.resources {
            let grants = resource.actions.values().flat_map(|a| a.roles.iter())
                .chain(resource.transitions.iter().flat_map(|t| t.transitions.values().flat_map(|x| x.roles.iter())));
            for role in grants {
                push(role, format!("server/resources/{name}"));
            }
        }
        for (name, op) in &d.operations {
            for role in &op.roles {
                push(role, format!("server/operations/{name}.jsbrain"));
            }
        }
    }
    let mut scan = |at: String, path: PathBuf, keys: &[&str]| {
        if path.is_file()
            && let Ok(contract) = read_contract(root, &path)
        {
            for key in keys {
                for role in contract[*key].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    push(role, at.clone());
                }
            }
        }
    };
    for entry in std::fs::read_dir(root.join("server/routes")).into_iter().flatten().flatten() {
        let at = format!("server/routes/{}/route.jsbrain", entry.file_name().to_string_lossy());
        scan(at, entry.path().join("route.jsbrain"), &["auth"]);
    }
    for entry in std::fs::read_dir(root.join("server/files")).into_iter().flatten().flatten() {
        scan(format!("server/files/{}", entry.file_name().to_string_lossy()), entry.path(), &["create", "read", "delete"]);
    }
    out
}
fn undeclared(role: &str, at: &str, roles: &brain_backend::auth_flows::RolesConfig) -> String {
    format!(
        "{at}: role `{role}` is not declared, so this grant would admit nobody; fix: add '{role}' to roles in server/auth/roles.jsbrain (`brain add roles` creates the file) and run `brain backend migrate`, or grant a declared role: {}",
        roles.roles.join(", ")
    )
}
fn validate_roles(root: &Path, roles: &brain_backend::auth_flows::RolesConfig) -> Result<(), String> {
    // Policies have always failed on an undeclared role; field visibility and routes warn (role_warnings) so that an
    // app that compiled before keeps compiling.
    if let Some((role, at)) = named_grants(root).into_iter().find(|(role, at)| at.ends_with("/policy.jsbrain") && !roles.roles.contains(role)) {
        return Err(undeclared(&role, &at, roles));
    }
    Ok(())
}
/// Warnings about the roles of a valid contract: `(code, file, message, fix)`. Nothing here can fail a green app.
pub fn role_warnings(root: &Path) -> Vec<(&'static str, String, String, String)> {
    let Ok(roles) = auth_roles(root) else { return Vec::new() };
    if validate_roles(root, &roles).is_err() {
        return Vec::new(); // B0735 owns it
    }
    let mut out = Vec::new();
    if legacy_implicit_roles(root) {
        let list = roles.roles.iter().map(|r| format!("'{r}'")).collect::<Vec<_>>().join(", ");
        out.push(("B0739", "server/auth/roles.jsbrain".to_string(),
            format!("no roles file: this app names a role from the implicit defaults of 0.6.1 and earlier, so it keeps them and signs people up as `{}`", roles.signup_role),
            format!("run `brain add roles` to declare them: export default {{ roles: [{list}], signup_role: '{}' }}; then remove the names you do not use", roles.signup_role)));
    }
    let grants = named_grants(root);
    for (role, at) in &grants {
        if !roles.roles.contains(role) && !at.ends_with("/policy.jsbrain") {
            let text = undeclared(role, at, &roles);
            let (message, fix) = text.split_once("; fix: ").unwrap_or((&text, ""));
            out.push(("B0738", at.clone(), message.to_string(), fix.to_string()));
        }
    }
    let routes_admit = crate::route_executor::contracts(root, "development").is_ok_and(|r| r.iter().any(|route| matches!(route.access, brain_backend::routes::Access::SignedIn)));
    let others: std::collections::BTreeSet<&str> = grants.iter().map(|(r, _)| r.as_str()).filter(|r| *r != "admin").collect();
    if !others.is_empty() && !others.contains(roles.signup_role.as_str()) && !routes_admit {
        out.push(("B0738", "server/auth/roles.jsbrain".to_string(),
            format!("accounts that sign up get the role `{}`, which no grant names; the grants name: {}", roles.signup_role, others.iter().copied().collect::<Vec<_>>().join(", ")),
            format!("if people who sign up should act here, set signup_role to one of those roles or add '{}' to the grants; existing accounts keep their role (`brain backend rename-role --from <old> --to <new>` moves them)", roles.signup_role)));
    }
    out
}
/// `brain add roles`: writes the roles this app already has, so declaring them changes nothing.
pub fn add_roles(root: &Path) -> Result<String, String> {
    let path = root.join("server/auth/roles.jsbrain");
    if path.symlink_metadata().is_ok() {
        return Err("server/auth/roles.jsbrain already exists; fix: edit it directly".into());
    }
    let roles = auth_roles(root)?;
    std::fs::create_dir_all(root.join("server/auth")).map_err(|_| "could not create server/auth")?;
    let list = roles.roles.iter().map(|r| format!("'{r}'")).collect::<Vec<_>>().join(", ");
    std::fs::write(&path, format!("<meta server>\nexport default {{ roles: [{list}], signup_role: '{}' }}\n", roles.signup_role))
        .map_err(|_| "could not write server/auth/roles.jsbrain")?;
    Ok("server/auth/roles.jsbrain".into())
}
/// B0749: resources whose unconditioned public read also serves rows a state-like field marks as not ready.
pub fn row_exposure(root: &Path) -> Vec<(String, String, String)> {
    let state_like = |name: &str| {
        let name = name.strip_prefix("is_").or_else(|| name.strip_prefix("has_")).unwrap_or(name);
        let name = name.strip_suffix("_at").unwrap_or(name);
        ["status", "state", "published", "draft", "visibility", "approved"].contains(&name)
    };
    let resources = contracts(root, "resources").unwrap_or_default();
    resources.iter()
        .filter(|r| r.policy.api && !r.policy.expose_all_rows && r.policy.read.iter().any(|g| g == "public") && !r.policy.read_where.contains_key("public"))
        .filter_map(|r| {
            let (field, _) = r.fields.iter().find(|(name, _)| state_like(name))?;
            Some((
                format!("server/resources/{}/policy.jsbrain", r.name),
                format!("read: ['public'] on `{}` serves every row to anonymous visitors, whatever its `{field}` says", r.name),
                format!("limit it with read: [{{ public: {{ where: {{ {field}: <public value> }} }} }}, 'owner', 'admin'], or declare expose_all_rows: true if every row is meant to be public"),
            ))
        })
        .collect()
}
/// True when a field name reads as internal. Whole `_`-separated words only, so `tokens` or `hashtag` stay out.
fn internal_looking(name: &str) -> bool {
    let words: Vec<&str> = name.split('_').collect();
    let pair = |a: &str, b: &str| words.windows(2).any(|w| w[0] == a && w[1] == b);
    words.iter().any(|w| ["password", "passwd", "pwd", "secret", "token", "salt", "hash", "otp", "internal", "legacy", "apikey"].contains(w))
        || pair("api", "key") || pair("private", "key") || pair("ip", "address") || pair("external", "id")
}
/// B0797: fields named like internals that every reader of the record receives, because nothing says otherwise.
pub fn public_field_warnings(root: &Path) -> Vec<(String, String, String)> {
    let resources = contracts(root, "resources").unwrap_or_default();
    let mut out = Vec::new();
    for r in resources.iter().filter(|r| r.policy.api) {
        for (name, field) in &r.fields {
            let hidden = matches!(field.access_mode(), Some("private" | "writeonly"));
            if !hidden && field.visible.is_none() && internal_looking(name) {
                out.push((
                    format!("server/resources/{}/schema.jsbrain", r.name),
                    format!("`{name}` reads as an internal field, and every reader of `{}` receives it because nothing limits it", r.name),
                    format!("write `{name}: 'str private'` (server only), 'str writeonly' (accepted, never returned) or {{ type: 'str', visible: ['owner', 'admin'] }}; if it is meant to be public, say visible: ['public']"),
                ));
            }
        }
    }
    out
}
/// B0794: unique fields any signed-in user can write, so the first writer takes the value from everybody else.
/// An explicit `access` states who may; grants limited to admins or named roles are not "any user".
pub fn claimable_unique_warnings(root: &Path) -> Vec<(String, String, String)> {
    let resources = contracts(root, "resources").unwrap_or_default();
    let mut out = Vec::new();
    for r in resources.iter().filter(|r| r.policy.api && r.policy.create.iter().chain(&r.policy.update).any(|grant| grant == "user" || grant == "public")) {
        for (name, field) in r.fields.iter().filter(|(_, f)| f.unique && f.access_mode().is_none()) {
            out.push((
                format!("server/resources/{}/schema.jsbrain", r.name),
                format!("any signed-in user can write `{name}` on `{}`, and it is unique: whoever writes a value first takes it from everybody else", r.name),
                format!("restrict it with {{ type: '{}', unique: true, access: {{ write: ['admin'] }} }}, or state that it is meant with access: {{ write: ['user'] }}", field.kind),
            ));
        }
    }
    out
}
/// B0808: a role that may `update` a resource without being able to read it: PATCH answers 200 on a row that same role
/// reads as 404. `public` and conditioned read grants count as reading; `api: false` resources have no such endpoint.
pub fn blind_update_warnings(root: &Path) -> Vec<(String, String, String)> {
    let resources = contracts(root, "resources").unwrap_or_default();
    let mut out = Vec::new();
    for r in resources.iter().filter(|r| r.policy.api) {
        for grant in r.policy.update.iter().filter(|g| !r.policy.read.iter().any(|read| read == *g || read == "public")) {
            out.push((
                format!("server/resources/{}/policy.jsbrain", r.name),
                format!("`{grant}` may update `{}` but not read it, so it writes rows it reads as missing", r.name),
                format!("add '{grant}' to `read`, or remove it from `update`"),
            ));
        }
    }
    out
}
/// B0809: an operation step that writes rows by id or key, without `owned: true`, on a resource one of its roles cannot
/// read in full. Every keyed step applies the read policy at run time (`Engine::may_read`), so such a step answers 404
/// (403 for an upsert) for a row the caller cannot read: an author expecting the operation to reach every row learns it here.
/// Returns `(file, message, fix)`.
pub fn operation_reach_warnings(root: &Path) -> Vec<(String, String, String)> {
    let resources = contracts(root, "resources").unwrap_or_default();
    let Ok(d) = domain::load(root, &resources) else { return Vec::new() };
    let mut out = Vec::new();
    for (name, op) in &d.operations {
        for step in op.steps.iter().filter(|s| !s.owned) {
            let by_id = from_input(step.id.as_deref());
            let target = if by_id { [&step.update, &step.delete, &step.action, &step.transition].into_iter().find_map(|t| t.as_deref()) } else { None };
            let (target, answer) = match (target, step.upsert.as_deref()) {
                (Some(t), _) => (t, "404, as for a missing row"),
                (None, Some(t)) => (t, "403 when the key matches one of them"),
                (None, None) => continue,
            };
            let Some(resource) = target.split('.').next().and_then(|t| resources.iter().find(|r| r.name == t)) else { continue };
            if let Some(role) = op.roles.iter().find(|role| !resource.policy.allows("read", Some(role.as_str()), false)) {
                out.push((
                    format!("server/operations/{name}.jsbrain"),
                    format!("step `{}` cannot change rows of `{}` that `{role}` reads as missing: it answers {answer}", step.name, resource.name),
                    format!("add `owned: true` when the step is meant for the caller's own rows, or grant `{role}` read on `{}` when it must reach every row", resource.name),
                ));
            }
        }
    }
    out
}
fn from_input(id: Option<&str>) -> bool {
    id.is_some_and(|id| id.starts_with("$input."))
}
/// B0813: a `debit` step (the only credit-balance step an operation has) whose `subject` is not the caller: an id from
/// the input or from an earlier step lets anyone the operation admits spend another person's balance. Returns
/// `(file, message, fix)`.
pub fn debit_subject_warnings(root: &Path) -> Vec<(String, String, String)> {
    let resources = contracts(root, "resources").unwrap_or_default();
    let Ok(d) = domain::load(root, &resources) else { return Vec::new() };
    let mut out = Vec::new();
    for (name, op) in &d.operations {
        for step in &op.steps {
            let Some(debit) = &step.debit else { continue };
            let subject = debit.subject.trim();
            if !subject.starts_with('$') || subject.starts_with("$user.") {
                continue;
            }
            let roles = op.roles.join("`, `");
            out.push((
                format!("server/operations/{name}.jsbrain"),
                format!("step `{}` debits `{subject}`, a balance the caller chooses: anyone with role `{roles}` can spend another person's credits", step.name),
                "charge the caller with `subject: '$user.id'`; when the operation must charge others, admit only a system key or an administrator role and keep this warning as the record of that decision".to_string(),
            ));
        }
    }
    out
}
/// B0790: secrets a declared route or webhook needs and the development environment does not hold: `(file, message, fix)`.
/// Absent, serve starts with that route disabled; too short, it refuses to start. Names only, never values.
pub fn secret_warnings(root: &Path) -> Vec<(String, String, String)> {
    let fix = "set it to a generated value in .brain_env or the host environment; a secret held only on the deployed host may keep this warning on other machines";
    let mut findings: Vec<_> = secret_findings(root, "development")
        .into_iter()
        .map(|(file, message, disables)| {
            let message = message.strip_prefix(&format!("{file}: ")).unwrap_or(&message).to_string();
            let effect = if disables { "`brain backend serve` starts with this disabled (503, no handler runs)" } else { "`brain backend serve` refuses to start" };
            let fix = if file == brain_backend::captcha::CONTRACT { "configure the provider-issued secret in .brain_env or the host environment; use public dummy keys only under development/testing" } else { fix };
            (file, format!("{effect}: {message}"), fix.to_string())
        })
        .collect();
    if let Some((file, message, _)) = crate::platform_config::captcha_public_test_secret_problem(root, "production") {
        let message = message.strip_prefix(&format!("{file}: ")).unwrap_or(&message).to_string();
        findings.push((file, message, "keep public test keys in development/testing only; set the real provider-issued secret under [production] or in the production host environment".into()));
    }
    findings
}
/// Every route and webhook secret problem of an environment: `(file, message, disables)`.
pub(crate) fn secret_findings(root: &Path, environment: &str) -> Vec<(String, String, bool)> {
    crate::route_executor::secret_problems(root, environment).into_iter().map(|p| (p.file, p.message, p.disables)).chain(crate::webhook_config::secret_problem(root, environment)).chain(crate::platform_config::captcha_secret_problem(root, environment)).collect()
}
/// Why a source a contract names cannot start, with its own `; fix:`. `None` when it can be read and fits its limit.
pub(crate) fn source_problem(root: &Path, relative: &str, limit: usize, fix: &str) -> Option<String> {
    if !root.join(relative).exists() {
        return Some(format!("{relative}: a contract names this file and it does not exist, so `brain backend serve` refuses to start; fix: {fix}"));
    }
    crate::map::read_source_limit(root, relative, limit).err().map(|reason| format!("{relative}: `brain backend serve` refuses this source: {reason}; fix: {fix}"))
}
/// B0735 for what only `brain backend serve` reads, kept apart from `inspect` so that schema, docs and migrate still work
/// while a handler is being written. Each message starts with its file and may end with its own `; fix:`.
pub fn startup_contract_errors(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = analytics_settings(root).err().into_iter().collect();
    out.extend(crate::route_executor::declared(root).map(|routes| crate::route_executor::handler_problems(root, &routes)).unwrap_or_default());
    out.extend(contracts(root, "resources").map(|resources| crate::hooks_executor::source_problems(root, &resources)).unwrap_or_default());
    out
}
const BUCKET_TYPES_FIX: &str = "list what it should hold, for example types: ['application/pdf', 'image/*'], or write types: '*' to keep accepting any file as before; official MIME names work and matching uses the file's content";
/// B0770: buckets that never listed their types (`check` reports them, `serve` refuses to start).
pub fn bucket_type_errors(root: &Path) -> Vec<(String, String, String)> {
    let (Ok(resources), Ok(roles)) = (contracts(root, "resources"), auth_roles(root)) else { return Vec::new() };
    let buckets = crate::platform_config::load(root, "development", &resources, &roles).map(|p| p.buckets).unwrap_or_default();
    buckets.iter().filter(|b| b.open_by_omission()).map(|b| (
        format!("server/files/{}.jsbrain", b.name),
        format!("bucket `{}` does not say which types it takes; since 0.8.0 an unstated list is refused, because it would accept every file, HTML and executables included", b.name),
        BUCKET_TYPES_FIX.to_string(),
    )).collect()
}
pub fn inspect(root: &Path) -> Result<Value, String> {
    let resources = contracts(root, "resources")?;
    let jobs = contracts(root, "jobs")?;
    contract::validate_graph(&resources, &jobs)?;
    let roles = auth_roles(root)?;
    validate_roles(root, &roles)?;
    let domain = domain::checked(root, &resources, &roles)?;
    let mut openapi = contract::openapi_with_roles(&resources, &roles);
    brain_backend::domain::openapi(&domain, &resources, &mut openapi);
    // The contract documents a route whether or not this machine holds its secret (B0790 reports that); an invalid one is B0735.
    let routes = crate::route_executor::declared(root)?;
    let buckets = crate::platform_config::load(root, "development", &resources, &roles).map(|p| p.buckets).unwrap_or_default();
    brain_backend::openapi::extend(&mut openapi, &routes, &buckets);
    let queues: serde_json::Map<String, Value> = jobs
        .iter()
        .map(|j| Ok((j.name.clone(), json!(domain::queue_policy(root, &j.name)?))))
        .collect::<Result<_, String>>()?;
    let cron: serde_json::Map<String, Value> = domain::cron(root, &jobs)?.into_iter().map(|(n, s)| (n, json!(s))).collect();
    Ok(
        json!({"schemaVersion":1,"experimental":true,"resources":resources,"jobs":jobs,"queues":queues,"cron":cron,"roles":roles,"domain":domain,"webhooks":crate::webhook_config::metadata(root)?,"apiKeys":crate::api_key_config::metadata(root)?,"platform":crate::platform_config::metadata(root,&resources,&roles)?,"openapi":openapi,"limits":{"activeJobs":10000,"workers":8,"jobSeconds":60,"leaseSeconds":90},"delivery":"at-least-once; handlers must be idempotent"}),
    )
}
pub(crate) fn database_url(root: &Path, environment: &str) -> Result<String, String> {
    database(root, environment)
}
fn database(root: &Path, environment: &str) -> Result<String, String> {
    std::env::var("DATABASE_URL")
        .ok()
        .or_else(|| crate::env::value(root, environment, "DATABASE_URL"))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| missing_database(root, environment))
}
/// B0762 when no source holds `DATABASE_URL`: which file was read, which section, and the sections that do have it.
fn missing_database(root: &Path, environment: &str) -> String {
    let file = root.join(crate::env::FILE);
    let Ok(source) = std::fs::read_to_string(&file) else {
        return format!("B0762: DATABASE_URL is not set: it is not in the process environment and {} does not exist; fix: create that file, run from the app folder, or set DATABASE_URL in the process environment", file.display());
    };
    let parsed = brain_check::env::parse(&source);
    let holding: Vec<String> = parsed.environments().into_iter().filter(|name| name != environment && crate::env::value(root, name, "DATABASE_URL").is_some_and(|v| !v.is_empty())).collect();
    match holding.first() {
        Some(section) => format!("B0762: DATABASE_URL is not set for the {environment} environment, but {} has it under [{section}]; fix: pass --env {section} on a deployed machine, or set it under [{environment}]", file.display()),
        None => format!("B0762: DATABASE_URL is not set: it is not in the process environment and {} has no value for it in the {environment} environment; fix: set it in that file or the process environment, or pass --env <name> for the section that holds it", file.display()),
    }
}
/// Where `DATABASE_URL` comes from for this environment; never its value. Named when Postgres refuses the login.
pub(crate) fn database_source(root: &Path, environment: &str) -> String {
    if std::env::var("DATABASE_URL").is_ok_and(|v| !v.is_empty()) {
        return "the process environment".into();
    }
    let file = root.join(crate::env::FILE);
    let parsed = std::fs::read_to_string(&file).map(|source| brain_check::env::parse(&source));
    let in_section = parsed.as_ref().is_ok_and(|p| p.sections.iter().any(|(name, entries)| name == environment && entries.iter().any(|e| e.key == "DATABASE_URL")));
    if in_section { format!("{} section [{environment}]", file.display()) } else { format!("{} shared block", file.display()) }
}
/// `application_name` for this app's Postgres sessions: `brain:<brain.json name>:<role>`.
pub(crate) fn pg_name(root: &Path, role: &str) -> String {
    let name = std::fs::read_to_string(root.join("brain.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["name"].as_str().map(str::to_string))
        .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    brain_backend::db::application_name(&name, role)
}
/// `brain.json` `analytics` (docs/backend/analytics.md): on by default; `false` or `{"enabled": false}` turns it off.
/// Keys: `consent` ("required"), `retentionDays` (1–3650, default 180), `countryHeader`, `eventsPerMinute`.
pub(crate) fn analytics_settings(root: &Path) -> Result<Option<brain_backend::analytics::Config>, String> {
    let manifest = std::fs::read_to_string(root.join("brain.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null);
    let a = &manifest["analytics"];
    if a == &Value::Bool(false) || a["enabled"] == Value::Bool(false) {
        return Ok(None);
    }
    if !(a.is_null() || a.is_object() || a == &Value::Bool(true)) {
        return Err("brain.json: analytics must be true, false or an object".into());
    }
    let mut cfg = brain_backend::analytics::Config::default();
    if let Some(obj) = a.as_object() {
        for key in obj.keys() {
            if !matches!(key.as_str(), "enabled" | "consent" | "retentionDays" | "countryHeader" | "eventsPerMinute") {
                return Err(format!("brain.json: analytics.{key} is unknown; fix: use enabled, consent, retentionDays, countryHeader or eventsPerMinute"));
            }
        }
        match &a["consent"] {
            Value::Null => {}
            Value::String(s) if s == "required" => cfg.consent_required = true,
            Value::String(s) if s == "none" => {}
            _ => return Err("brain.json: analytics.consent must be \"required\" or \"none\"".into()),
        }
        let num = |k: &str| match &a[k] {
            Value::Null => Ok(None),
            value => value.as_u64().map(|n| Some(u32::try_from(n).unwrap_or(u32::MAX))).ok_or_else(|| format!("brain.json: analytics.{k} must be a whole number")),
        };
        if let Some(n) = num("retentionDays")? { cfg.retention_days = n; }
        if let Some(n) = num("eventsPerMinute")? { cfg.events_per_minute = n; }
        match &a["countryHeader"] {
            Value::Null => {}
            Value::String(h) => cfg.country_header = Some(h.clone()),
            _ => return Err("brain.json: analytics.countryHeader must be a lowercase HTTP header name".into()),
        }
    }
    cfg.validate().map_err(|error| format!("brain.json: {error}"))?;
    Ok(Some(cfg))
}
fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}
/// Every setting the backend refuses at startup, as (problem, fix). Values never reach a message: only variable names.
pub fn config_problems(setting: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    let mut report = |problem: String, fix: &str| problems.push((problem, fix.to_string()));
    let placeholder = || "http://127.0.0.1".to_string();
    let secret_fix = "set BRAIN_JWT_SECRET to 32..4096 random bytes; BRAIN_JWT_ISSUER and BRAIN_JWT_AUDIENCE are optional and default to the backend origin";
    match setting("BRAIN_JWT_SECRET") {
        Some(secret) => {
            let issuer = setting("BRAIN_JWT_ISSUER").unwrap_or_else(placeholder);
            let audience = setting("BRAIN_JWT_AUDIENCE").unwrap_or_else(placeholder);
            match brain_backend::jwt::Config::new(secret, issuer, audience) {
                Err(_) => report("BRAIN_JWT_SECRET, BRAIN_JWT_ISSUER or BRAIN_JWT_AUDIENCE is out of range".into(), secret_fix),
                Ok(config) => {
                    if let Some(previous) = setting("BRAIN_JWT_SECRET_PREVIOUS")
                        && config.with_previous(previous.split(',').map(str::to_string).collect()).is_err()
                    {
                        report("BRAIN_JWT_SECRET_PREVIOUS has a key out of range".into(), "list retired keys separated by commas, each 32..4096 bytes, and remove them once their tokens expired");
                    }
                }
            }
        }
        None => {
            for key in ["GOOGLE_CLIENT_ID", "APPLE_CLIENT_ID", "BRAIN_AUTH_LINKEDIN_CLIENT_ID"] {
                if setting(key).is_some() {
                    report(format!("{key} needs native JWT sessions, and BRAIN_JWT_SECRET is not set"), secret_fix);
                }
            }
        }
    }
    if let Some(id) = setting("GOOGLE_CLIENT_ID")
        && brain_backend::google::Config::new(id).is_err()
    {
        report("GOOGLE_CLIENT_ID is not a web OAuth client id".into(), "use the web client id that ends in .apps.googleusercontent.com");
    }
    // Return URLs default to the site origin at startup; a placeholder origin keeps the shape check honest here.
    if let Some(client) = setting("APPLE_CLIENT_ID")
        && let Err(error) = brain_backend::apple::Config::new(client, setting("APPLE_APP_ID"), setting("APPLE_REDIRECT_URI").unwrap_or_else(|| "https://example.com/".into()))
    {
        report(format!("APPLE_CLIENT_ID, APPLE_APP_ID or APPLE_REDIRECT_URI is invalid: {error}"), "use the Services ID (com.example.web), the primary App ID and the https Return URL registered with Apple");
    }
    match (setting("BRAIN_AUTH_LINKEDIN_CLIENT_ID"), setting("BRAIN_AUTH_LINKEDIN_CLIENT_SECRET")) {
        (Some(client), Some(secret)) => {
            let redirect = setting("BRAIN_AUTH_LINKEDIN_REDIRECT_URI").unwrap_or_else(|| "https://example.com/_brain/auth/linkedin/callback".into());
            if let Err(error) = brain_backend::linkedin::Config::new(client, secret, redirect) {
                report(format!("LinkedIn sign-in setting is invalid: {error}"), "use the app's Client ID and Primary Client Secret, and a redirect URL registered in the LinkedIn app");
            }
        }
        (None, None) => {}
        _ => report("LinkedIn sign-in needs both BRAIN_AUTH_LINKEDIN_CLIENT_ID and BRAIN_AUTH_LINKEDIN_CLIENT_SECRET".into(), "set both in the private server environment, or remove both"),
    }
    if setting("BRAIN_AUTH_REFRESH_GRACE_SECONDS").is_some_and(|v| !v.parse::<u32>().is_ok_and(|n| n <= 60)) {
        report("BRAIN_AUTH_REFRESH_GRACE_SECONDS must be 0..60".into(), "set whole seconds (0 turns the grace off), or remove it to use 10");
    }
    if setting("BRAIN_AUTH_SESSION_SECONDS").is_some_and(|v| !v.parse::<u32>().is_ok_and(|n| (300..=31_536_000).contains(&n))) {
        report("BRAIN_AUTH_SESSION_SECONDS must be 300..31536000".into(), "set the session length in whole seconds, or remove it to use the default");
    }
    if setting("BRAIN_AUTH_IDLE_SECONDS").is_some_and(|v| !v.parse::<u32>().is_ok_and(|n| (300..=31_536_000).contains(&n))) {
        report("BRAIN_AUTH_IDLE_SECONDS must be 300..31536000".into(), "set the idle timeout in whole seconds, or remove it so sessions end only when they expire");
    }
    problems
}
/// The same findings for `brain check` and `brain doctor`, from the development section of .brain_env. Only for apps
/// that run the native backend: a key like GOOGLE_CLIENT_ID means nothing to the backend in any other app.
pub fn config_diagnostics(root: &Path) -> Vec<(String, String)> {
    let native = root.join("server/resources").is_dir() || root.join("server/jobs").is_dir() || root.join("server/routes").is_dir() || crate::env::value(root, "development", "BRAIN_BACKEND_URL").is_some();
    if !native {
        return Vec::new();
    }
    config_problems(&|key| crate::env::value(root, "development", key).filter(|v| !v.is_empty()))
}
/// Provider setup hints contain validated public origins only, never provider credentials.
pub fn google_origin_setup(origins: &[String]) -> Value {
    let mut required = origins.to_vec();
    if required.iter().any(|origin| origin.starts_with("http://localhost:")) && !required.iter().any(|origin| origin == "http://localhost") {
        required.push("http://localhost".into());
    }
    required.sort();
    required.dedup();
    json!({"authorizedJavaScriptOrigins":required,"providerVerified":false,
        "fix":"Add these exact browser origins to the Google web client's Authorized JavaScript origins; keep the frontend origin in the native HTTP allowlist. This report does not inspect Google Cloud configuration. See docs/JWT-GOOGLE.md."})
}
pub fn google_origin_advice(root: &Path) -> Vec<(String, Value)> {
    let native = root.join("server/resources").is_dir() || root.join("server/jobs").is_dir() || root.join("server/routes").is_dir();
    ["development", "production"].into_iter().filter_map(|environment| {
        if !native && crate::env::value(root, environment, "BRAIN_BACKEND_URL").is_none() { return None; }
        crate::env::value(root, environment, "GOOGLE_CLIENT_ID")?;
        let site = crate::env::value(root, environment, "PUBLIC_SITE_URL");
        let settings = socket_settings(root, site.as_deref(), None).ok()?;
        (!settings.http_origins.is_empty()).then(|| (environment.into(), google_origin_setup(&settings.http_origins)))
    }).collect()
}
fn operator(args: &[String], url: &str, production: bool) -> Result<(), String> {
    let usage = "usage: brain backend users disable|enable|revoke-sessions|disable-identity|enable-identity --user <uuid>|--email <address> [--provider google|apple|linkedin] [--env development]";
    let action = args.get(1).map(String::as_str).ok_or(usage)?;
    if !matches!(action, "disable" | "enable" | "revoke-sessions" | "disable-identity" | "enable-identity") {
        return Err(usage.into());
    }
    let provider = crate::flag(args, "--provider");
    let provider = match action {
        "disable-identity" | "enable-identity" => Some(provider.ok_or("this action requires --provider, for example --provider google")?),
        _ => None,
    };
    runtime()?.block_on(async {
        use brain_backend::auth_flows as flows;
        let pool = brain_backend::boot::connect(url, &brain_backend::boot::Boot::command(production)).await?;
        let user = match (crate::flag(args, "--user"), crate::flag(args, "--email")) {
            (Some(user), None) => user.parse().map_err(|_| "--user must be a UUID".to_string())?,
            (None, Some(address)) => flows::user_id_by_email(&pool, &address).await.map_err(|e| e.to_string())?.ok_or("No user has that email")?,
            _ => return Err(usage.to_string()),
        };
        let done = match action {
            "disable" => flows::set_user_disabled(&pool, user, true).await,
            "enable" => flows::set_user_disabled(&pool, user, false).await,
            "revoke-sessions" => flows::revoke_user_sessions(&pool, user).await,
            "disable-identity" => flows::set_identity_disabled(&pool, user, provider.as_deref().unwrap_or_default(), true).await,
            _ => flows::set_identity_disabled(&pool, user, provider.as_deref().unwrap_or_default(), false).await,
        }
        .map_err(|e| e.to_string())?;
        pool.close().await;
        if !done {
            return Err("Refused: the target does not exist, it has no such identity, or it is the last enabled administrator".into());
        }
        println!("{}", json!({"schemaVersion":1,"ok":true,"action":action,"user":user}));
        Ok(())
    })
}
/// B0785: without `--env`, `serve` runs as development: no TLS check on the database, open API description, no HSTS.
/// That default is only right on a developer's machine, so anything that looks deployed has to say what it is.
fn unstated_environment(args: &[String], database: Option<&str>, node_env: Option<&str>) -> Option<String> {
    if args.iter().any(|a| a == "--env" || a.starts_with("--env=")) {
        return None;
    }
    let why = if node_env == Some("production") {
        "NODE_ENV is production"
    } else if database.is_some_and(|url| !brain_backend::boot::database_is_local(url)) {
        "DATABASE_URL points at another machine"
    } else {
        return None;
    };
    Some(format!("B0785: {why} and no --env was given, so this would run with development defaults; fix: pass --env production (or --env development if this really is a development machine)"))
}
/// Exit status of a long-running command stopped by its configuration (`EX_CONFIG`): a restart cannot fix it, so the
/// generated PM2 file lists it in `stop_exit_codes` and `--with-workers` does not restart a worker that returns it.
pub(crate) const CONFIG_EXIT: u8 = 78;
static CONFIG_PHASE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Marks whether a failure right now is the configuration's. Off around what can recover alone: connecting to Postgres,
/// binding the port, serving.
fn config_phase(on: bool) {
    CONFIG_PHASE.store(on, std::sync::atomic::Ordering::SeqCst);
}
pub(crate) fn failed_on_configuration() -> bool {
    CONFIG_PHASE.load(std::sync::atomic::Ordering::SeqCst)
}
/// The exit that must not count as a crash: what a supervisor of workers does with it.
fn restart_worker(code: Option<i32>) -> bool {
    code != Some(CONFIG_EXIT as i32)
}
/// B0790: a secret held too short, or endpoints left without their encryption key. The environment alone reveals it, so
/// it stops before any connection, once, instead of after one on every restart. An absent secret only disables its route.
fn secret_refusal(root: &Path, environment: &str) -> Option<String> {
    let found: Vec<String> = secret_findings(root, environment).into_iter().filter(|f| !f.2).map(|(file, message, _)| format!("{file}: {message}")).collect();
    (!found.is_empty()).then(|| format!("B0790: {}; fix: set a long enough generated secret in the process environment or .brain_env (`brain check` lists them); a weak or unprotected secret is refused rather than disabled because it would let forged requests in", found.join(" | ")))
}
/// One start warning per disabled route or endpoint, as an event a log reader can match. Names only.
fn warn_disabled(root: &Path, environment: &str) {
    for (file, message, _) in secret_findings(root, environment).into_iter().filter(|f| f.2) {
        let message = message.strip_prefix(&format!("{file}: ")).unwrap_or(&message).to_string();
        brain_backend::log_line!("{}", json!({"event":"route-disabled","code":"B0790","file":file,"msg":format!("disabled, answers 503 and runs no handler: {message}"),"fix":"set the secret in the process environment or .brain_env and restart"}));
    }
}
/// The environment is checked before the lookup: without `--env` the lookup reads the development section, so a deployed
/// shell would otherwise hear B0762 about a variable it did set, under [production].
pub(crate) fn stated_database(args: &[String], root: &Path, environment: &str) -> Result<String, String> {
    let node_env = std::env::var("NODE_ENV").ok();
    if let Some(refusal) = unstated_environment(args, None, node_env.as_deref()) {
        return Err(refusal);
    }
    let url = database(root, environment)?;
    match unstated_environment(args, Some(&url), node_env.as_deref()) {
        Some(refusal) => Err(refusal),
        None => Ok(url),
    }
}
/// The backend started by `brain run --backend`; stops with the command that owns it.
pub struct DevBackend {
    child: std::process::Child,
    pub page_origin: Option<String>,
}
impl Drop for DevBackend {
    fn drop(&mut self) {
        stop_owned_child(&mut self.child);
    }
}
fn stop_owned_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn backend_ready(address: &std::net::SocketAddr) -> bool {
    use std::io::{Read, Write};
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(address, std::time::Duration::from_millis(200)) else { return false };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(1)));
    if stream.write_all(b"GET /health/ready HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    stream.read(&mut head).is_ok_and(|n| head[..n].starts_with(b"HTTP/1.1 200"))
}

/// Something listens at this backend URL: another process on this machine owns the database and the queues.
fn answers(url: &str) -> bool {
    use std::net::ToSocketAddrs;
    let host = url.split_once("://").map_or(url, |(_, rest)| rest).split('/').next().unwrap_or("");
    let target = if host.rsplit_once(':').is_some_and(|(_, port)| port.parse::<u16>().is_ok()) { host.to_string() } else { format!("{host}:{}", if url.starts_with("https:") { 443 } else { 80 }) };
    target.to_socket_addrs().ok().into_iter().flatten().any(|addr| std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok())
}

/// A backend URL that does not point at this machine.
fn is_remote_backend(url: &str) -> bool {
    let host = url.split_once("://").map_or("", |(_, rest)| rest).split('/').next().unwrap_or("");
    let host = if host.starts_with('[') { host.split(']').next().unwrap_or("").trim_start_matches('[') } else { host.split(':').next().unwrap_or("") };
    !host.is_empty() && !matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// The port of a fixed loopback backend URL.
fn loopback_port(url: &str) -> Option<u16> {
    let rest = url.strip_prefix("http://")?.trim_end_matches('/');
    let (host, port) = rest.rsplit_once(':')?;
    matches!(host, "127.0.0.1" | "localhost" | "[::1]").then_some(())?;
    port.parse().ok()
}

/// `brain run --backend`: one command for both processes. The port comes from `BRAIN_BACKEND_URL` and the allowed origin
/// from the development server's real port, so no setting is repeated. Migrations stay explicit: if they are pending, this
/// stops and names the command.
pub fn start_for_dev(root: &Path, environment: &str, dev_port: u16) -> Result<DevBackend, String> {
    let url = std::env::var("BRAIN_BACKEND_URL").ok().filter(|v| !v.is_empty()).or_else(|| crate::env::value(root, environment, "BRAIN_BACKEND_URL"))
        .ok_or("--backend needs BRAIN_BACKEND_URL; fix: run `brain add auth`, or set it in .brain_env")?;
    let port = loopback_port(&url).ok_or_else(|| format!("--backend needs a loopback BRAIN_BACKEND_URL with a port, for example {}; fix: correct it in .brain_env", crate::auth_setup::DEFAULT_BACKEND_URL))?;
    let site = crate::env::value(root, environment, "PUBLIC_SITE_URL").and_then(|s| brain_backend::links::origin(&s));
    let host = site.as_deref().filter(|s| s.starts_with("http://localhost")).map_or("127.0.0.1", |_| "localhost");
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    // Something else on that port would answer for the backend: its data and allowed origins, not this app's.
    if std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(200)).is_ok() {
        return Err(format!("B0763: port {port} from BRAIN_BACKEND_URL is already in use, so --backend would talk to another process; fix: stop it, or change the port in BRAIN_BACKEND_URL in .brain_env"));
    }
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this brain binary: {e}"))?;
    let mut command = std::process::Command::new(exe);
    command.args(["backend", "serve", "--env", environment, "--port", &port.to_string()]).stdin(std::process::Stdio::null()).current_dir(root);
    // A dev port other than the declared site (`--port`, a fallback): the backend's public URL and allowed origin follow it.
    let declared = crate::backend::socket_settings(root, site.as_deref(), None).map(|s| s.http_origins).unwrap_or_default();
    let origin = format!("http://{host}:{dev_port}");
    let bound_origin_allowed = declared.contains(&format!("http://127.0.0.1:{dev_port}"));
    if !declared.contains(&origin) && !bound_origin_allowed {
        command.env("PUBLIC_SITE_URL", &origin);
    }
    // The page has to open on an origin the backend accepts, or every mutation answers ORIGIN_DENIED.
    let page_origin = (!bound_origin_allowed).then_some(origin);
    let mut child = command.spawn().map_err(|e| format!("could not start the backend: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("the backend stopped at startup ({status}); its message is above. If tables are missing: `brain backend migrate --create-db`"));
        }
        // Ready, not merely listening: settings refused after the bind would otherwise read as a healthy start.
        if backend_ready(&address) {
            return Ok(DevBackend { child, page_origin });
        }
        if std::time::Instant::now() > deadline {
            stop_owned_child(&mut child);
            return Err(format!("the backend did not answer on port {port} within 30 s; run `brain backend serve` alone to see why"));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    if args.first().is_some_and(|a| a == "capabilities") {
        println!("{}", brain_backend::capabilities::inventory());
        return Ok(());
    }
    let root = crate::app_root()?;
    let environment = crate::env::name(args, "development");
    let operation = args.first().map(String::as_str).unwrap_or("schema");
    if operation != "approve-contract" && crate::flag(args, "--only").is_some() {
        return Err("--only applies only to brain backend approve-contract; fix: preview contract changes with that command before applying selected resources".into());
    }
    if operation == "migrate" && crate::flag(args, "--resource").is_some() {
        return Err("brain backend migrate does not select resources with --resource; fix: preview with brain backend approve-contract, then use --only <resource> --apply".into());
    }
    if operation == "logs" {
        let rid = args.get(1).filter(|a| !a.starts_with("--")).map(String::as_str);
        println!("{}", failure_logs(&root, rid));
        return Ok(());
    }
    if operation == "keys" {
        return crate::api_key_config::run(&root, &args[1..], stated_database(args, &root, &environment)?);
    }
    if operation == "data" && !matches!(args.get(1).map(String::as_str), Some("prepare" | "maintain" | "defaults")) {
        return Err("usage: brain data prepare|maintain|defaults [--apply] [--resource <name>] [--env <environment>]; defaults also accepts --limit <1..10000> and --include-null".into());
    }
    if args.iter().any(|arg| arg == "--include-null") && !(operation == "data" && args.get(1).is_some_and(|arg| arg == "defaults")) {
        return Err("--include-null applies only to brain data defaults".into());
    }
    if operation == "data" && args.get(1).is_some_and(|arg| arg != "defaults") && crate::flag(args,"--limit").is_some() {
        return Err("--limit applies only to brain data defaults".into());
    }
    let default_fill_limit = if operation == "data" && args.get(1).is_some_and(|arg| arg == "defaults") {
        if args.iter().any(|arg| arg.starts_with("--apply=") || arg.starts_with("--include-null=")) {
            return Err("default fill --apply and --include-null are switches without =values".into());
        }
        if args.iter().any(|arg|arg=="--limit") && crate::flag(args,"--limit").is_none() {
            return Err("default fill --limit needs a value from 1..10000".into());
        }
        crate::flag(args,"--limit").map(|value|value.parse::<u32>().ok().filter(|limit|(1..=10000).contains(limit))
            .ok_or("default fill --limit must be 1..10000".to_string())).transpose()?.unwrap_or(1000)
    } else { 1000 };
    if !matches!(
        operation,
        "schema" | "migrate" | "approve-contract" | "serve" | "bootstrap-admin" | "rename-role" | "users" | "import" | "data"
    ) {
        return Err("usage: brain backend capabilities|schema|migrate|approve-contract [--accept <resource>] [--only <resource>] [--apply]|import users|<resource> --file <rows.jsonl> [--apply]|serve|bootstrap-admin|rename-role --from <role> --to <role> [--apply]|logs [rid]|users|keys --user <uuid> [--env development] [--port 5182] [--origin http://127.0.0.1:5182] [--with-workers]".into());
    }
    let snapshot = inspect(&root)?;
    if operation == "schema" {
        println!("{snapshot}");
        return Ok(());
    }
    if operation == "serve" {
        config_phase(true);
        // Before any connection, so a wrong setting is never reported as a database or port problem.
        let problems = config_problems(&|key| std::env::var(key).ok().filter(|v| !v.is_empty()).or_else(|| crate::env::value(&root, &environment, key)));
        if !problems.is_empty() {
            let lines: Vec<String> = problems.iter().map(|(problem, fix)| format!("{problem}; fix: {fix}")).collect();
            return Err(format!("B0751 backend settings: {}", lines.join(" | ")));
        }
        if let Some((file, _, fix)) = bucket_type_errors(&root).into_iter().next() {
            return Err(format!("B0770: {file} does not say which types it takes; fix: {fix}"));
        }
    }
    let url = stated_database(args, &root, &environment)?;
    if operation == "users" {
        return operator(args, &url, environment == "production");
    }
    // Everything the environment alone can reveal stops before the first connection, with its code and fix.
    if operation == "serve" {
        let get = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty()).or_else(|| crate::env::value(&root, &environment, key));
        if let Some(found) = crate::startup_config::findings(&get, environment == "production").into_iter().find(|f| f.fatal) {
            return Err(found.message);
        }
        if let Some(refusal) = secret_refusal(&root, &environment) {
            return Err(refusal);
        }
        warn_disabled(&root, &environment);
    }
    if operation == "serve" && environment == "production"
        && let Some((message, fix, _)) = crate::platform_config::proxy_findings(&root, None).into_iter().find(|f| f.2)
    {
        return Err(format!("B0784: {message}; fix: {fix}"));
    }
    let roles = auth_roles(&root)?;
    let resources = contracts(&root, "resources")?;
    let jobs = contracts(&root, "jobs")?;
    let domain = domain::checked(&root, &resources, &roles)?;
    runtime()?.block_on(async{
        let production=environment=="production";
        if operation=="migrate" && args.iter().any(|a|a=="--create-db") {
            if production {return Err("--create-db is for local development; fix: create the production database with your provider".into());}
            if let Some(name)=brain_backend::boot::create_local_database(&url).await? {brain_backend::log_line!("created database {name}");}
        }
        let boot=if operation=="serve" {brain_backend::boot::Boot::server(production)} else {brain_backend::boot::Boot::command(production)}.from_source(database_source(&root,&environment));
        config_phase(false);
        let pool=brain_backend::boot::connect(&url,&boot).await?;
        config_phase(operation=="serve");
        if operation=="migrate"{
            brain_backend::db::migrate(&pool,&resources).await?;
            roles.register(&pool).await?;
            let undeclared=brain_backend::auth_flows::undeclared_accounts(&pool,&roles).await?;
            let stale=brain_backend::db::undeclared_fields(&pool,&resources).await?;
            pool.close().await;
            let mut report=json!({"schemaVersion":1,"ok":true,"resources":resources.len()});
            if !stale.is_empty() {
                report["undeclaredFields"]=json!(stale.iter().map(|(resource,field,rows)|json!({"resource":resource,"field":field,"rows":rows})).collect::<Vec<_>>());
                report["undeclaredFieldsNote"]=json!("These stored fields left the schema; each one is dropped from its row when that row is next written. Copy the data first if you still need it, or declare the field again.");
            }
            if !undeclared.is_empty() {
                // Nothing is rewritten: those accounts keep their role and owner grants, but no role grant reaches them.
                report["undeclaredRoles"]=json!(undeclared.iter().map(|(role,accounts)|json!({"role":role,"accounts":accounts})).collect::<Vec<_>>());
                report["next"]=json!("Declare those roles in server/auth/roles.jsbrain, or preview moving their accounts with `brain backend rename-role --from <role> --to <role>` (add --apply; sessions are revoked)");
            }
            println!("{report}");return Ok(())
        }
        if operation=="import" {
            let target=args.get(1).filter(|a|!a.starts_with("--")).map(String::as_str).ok_or("usage: brain backend import users|<resource> --file <rows.jsonl> [--apply]")?;
            let file=crate::flag(args,"--file").ok_or("usage: brain backend import users|<resource> --file <rows.jsonl> [--apply]")?;
            let text=std::fs::read_to_string(&file).map_err(|e|format!("{file}: {e}"))?;
            let apply=args.iter().any(|a|a=="--apply");
            let outcome=if target=="users" {
                let declared=roles.roles.clone();
                let auth=brain_backend::auth::Auth::new_with_role(pool.clone(),&roles.signup_role).await?;
                brain_backend::import::users(&pool,&declared,auth.unusable_hash(),&text,apply).await?
            } else {
                let resource=resources.iter().find(|r|r.name==target).ok_or_else(||format!("no resource is named `{target}`; fix: use `users` or a resource from server/resources"))?;
                brain_backend::import::records(&pool,resource,&text,apply).await?
            };
            pool.close().await;
            let problems:Vec<_>=outcome.problems.iter().map(|(line,reason)|json!({"line":line,"reason":reason})).collect();
            println!("{}",json!({"schemaVersion":1,"ok":problems.is_empty(),"rows":outcome.rows,"applied":outcome.applied,"problems":problems,"moreProblems":outcome.more_problems}));
            return if problems.is_empty() {Ok(())} else {Err(format!("B0798: {file} was not imported: {} rows have problems; fix: correct them and run the import again",problems.len()+outcome.more_problems))};
        }
        if operation=="data" {
            let mode=match args.get(1).map(String::as_str) {
                Some("prepare")=>brain_backend::maintenance::Mode::Prepare,
                Some("maintain")=>brain_backend::maintenance::Mode::Maintain,
                _=>brain_backend::maintenance::Mode::Maintain,
            };
            let chosen:Vec<_>=match crate::flag(args,"--resource") {
                Some(name)=>vec![resources.iter().find(|r|r.name==name).cloned().ok_or_else(||format!("no resource is named `{name}`; fix: use a resource from server/resources"))?],
                None=>resources.clone(),
            };
            let apply=args.iter().any(|arg|arg=="--apply");
            let report=if args.get(1).is_some_and(|arg|arg=="defaults") {
                brain_backend::defaults::fill(&pool,&chosen,apply,args.iter().any(|arg|arg=="--include-null"),default_fill_limit).await?
            } else { brain_backend::maintenance::run_all(&pool,&chosen,mode,apply).await? };
            pool.close().await;
            println!("{report}");
            return Ok(());
        }
        if operation=="approve-contract" {
            let reviewed=brain_backend::contract_review::review(&pool,&resources).await?;
            let accepted=crate::flag(args,"--accept").map(|list|list.split(',').map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
            let apply=args.iter().any(|a|a=="--apply");
            // `--only a,b` approves just those resources; every other changed one stays pending with its recorded contract.
            if let Some(only)=crate::flag(args,"--only").map(|list|list.split(',').map(str::to_string).collect::<Vec<_>>()) {
                let (names,pending)=brain_backend::contract_review::approved_only(&reviewed,&only,&accepted)?;
                if apply {brain_backend::db::migrate_approving_only(&pool,&resources,&names).await?;}
                pool.close().await;
                let next=if apply&&pending.is_empty(){Value::Null}else if apply{json!(format!("Review the pending resources, then: brain backend approve-contract --only {} --apply",pending.join(",")))}else{json!(format!("brain backend approve-contract --only {}{} --apply",only.join(","),if accepted.is_empty(){String::new()}else{format!(" --accept {}",accepted.join(","))}))};
                println!("{}",json!({"schemaVersion":1,"ok":true,"applied":apply&&!names.is_empty(),"approved":names,"pending":pending,"changes":reviewed,"needsAccept":[],"next":next}));return Ok(());
            }
            let decision=brain_backend::contract_review::approved(&reviewed,&accepted);
            if apply {
                let names=decision.as_ref().map_err(|blocked|format!("B0787: nothing was changed: {} has changes that can invalidate stored rows; fix: review them without --apply, then pass --accept {}, or apply only the others with --only <resource>",blocked.join(", "),blocked.join(",")))?;
                brain_backend::db::migrate_approving(&pool,&resources,names).await?;
            }
            pool.close().await;
            let needs:Vec<String>=decision.err().unwrap_or_default();
            let next=if apply||reviewed.is_empty(){Value::Null}else if needs.is_empty(){json!("brain backend approve-contract --apply")}else{json!(format!("Review the rows each change touches, then: brain backend approve-contract --accept {} --apply",needs.join(",")))};
            println!("{}",json!({"schemaVersion":1,"ok":true,"applied":apply&&!reviewed.is_empty(),"changes":reviewed,"needsAccept":needs,"next":next}));return Ok(());
        }
        if operation=="rename-role" {
            let from=crate::flag(args,"--from").ok_or("rename-role requires --from <role> --to <role>")?;
            let to=crate::flag(args,"--to").ok_or("rename-role requires --from <role> --to <role>")?;
            let apply=args.iter().any(|a|a=="--apply");
            let accounts=brain_backend::auth_flows::rename_role(&pool,&roles,&from,&to,apply).await?;
            pool.close().await;
            println!("{}",json!({"schemaVersion":1,"ok":true,"from":from,"to":to,"accounts":accounts,"applied":apply,"sessionsRevoked":apply}));return Ok(());
        }
        if operation=="bootstrap-admin" {
            brain_backend::db::verify(&pool,&resources).await?;
            let user=match (crate::flag(args,"--user"),crate::flag(args,"--email")) {
                (Some(_),Some(_))=>return Err("bootstrap-admin takes --user or --email, not both".into()),
                (None,Some(address))=>brain_backend::auth_flows::user_id_by_email(&pool,&address).await.map_err(|error|error.to_string())?.ok_or("bootstrap-admin: no account has this email; no account is created. Sign up first")?.to_string(),
                (Some(user),None)=>user,
                (None,None)=>return Err("bootstrap-admin requires --email or the explicit --user UUID of an existing enabled user; no account is created".into()),
            };
            let user=user.parse().map_err(|_|"user must be a UUID")?;
            let promoted=brain_backend::auth_flows::bootstrap_first_admin(&pool,user).await.map_err(|error|error.to_string())?;
            pool.close().await;
            if !promoted {return Err("Admin bootstrap requires an enabled existing user and no active administrator".into());}
            println!("{}",json!({"schemaVersion":1,"ok":true,"user":user,"role":"admin","sessionsRevoked":true}));return Ok(());
        }
        brain_backend::db::keep_diagnostics(failure_log(&root));
        if !production && brain_serve::trace::settings(&root).enabled {brain_backend::dev_trace::install(&root);}
        let port=crate::flag(args,"--port").unwrap_or("5182".into()).parse::<u16>().map_err(|_|"B0763: --port is not a port number; fix: pass 0 to 65535")?;
        config_phase(false);
        let listener=brain_serve::listen::bind_waiting("127.0.0.1",port,brain_serve::listen::RESTART_WAIT).and_then(|l|{l.set_nonblocking(true)?;tokio::net::TcpListener::from_std(l)}).map_err(|e|format!("B0763: cannot listen on 127.0.0.1:{port} ({e}); fix: stop what uses that port or pass another with --port"))?;
        config_phase(true);
        let addr=listener.local_addr().map_err(|e|e.to_string())?;
        let public_site=crate::env::value(&root,&environment,"PUBLIC_SITE_URL").unwrap_or_else(||format!("http://{addr}"));
        let sockets=socket_settings(&root,Some(&public_site),crate::flag(args,"--origin").as_deref())?;
        let origin=sockets.http_origins.first().cloned().ok_or("B0744: the backend has no HTTP origin; fix: set PUBLIC_SITE_URL, pass --origin or add it to brain.json backend.origins")?;
        // Never migrate on serve: database changes are an explicit, reviewable command.
        let setting=|key:&str|std::env::var(key).ok().filter(|v|!v.is_empty()).or_else(||crate::env::value(&root,&environment,key));
        let jwt=setting("BRAIN_JWT_SECRET").map(|secret|brain_backend::jwt::Config::new(secret,setting("BRAIN_JWT_ISSUER").unwrap_or_else(||origin.clone()),setting("BRAIN_JWT_AUDIENCE").unwrap_or_else(||origin.clone()))).transpose()?;
        let jwt=match (jwt,setting("BRAIN_JWT_SECRET_PREVIOUS")) {(Some(config),Some(previous))=>Some(config.with_previous(previous.split(',').map(str::to_string).collect())?),(config,_)=>config};
        let jwt=match (jwt,setting("BRAIN_AUTH_REFRESH_GRACE_SECONDS")) {(Some(config),Some(seconds))=>Some(config.with_refresh_grace(seconds.parse().map_err(|_|"BRAIN_AUTH_REFRESH_GRACE_SECONDS must be 0..60")?)?),(config,_)=>config};
        if let Some(seconds)=setting("BRAIN_AUTH_SESSION_SECONDS") {brain_backend::auth::configure_session_seconds(seconds.parse().map_err(|_|"BRAIN_AUTH_SESSION_SECONDS must be 300..31536000")?)?;}
        if let Some(seconds)=setting("BRAIN_AUTH_IDLE_SECONDS") {brain_backend::sessions::configure_idle_seconds(seconds.parse().map_err(|_|"BRAIN_AUTH_IDLE_SECONDS must be 300..31536000")?)?;}
        let google=setting("GOOGLE_CLIENT_ID").map(brain_backend::google::Config::new).transpose()?;
        if google.is_some() && jwt.is_none() {return Err("Google Sign-In requires configured native JWT sessions; set BRAIN_JWT_SECRET, issuer and audience".into());}
        // Apple and LinkedIn default their return URLs to this origin; set the variable when the registered URL differs.
        let apple=setting("APPLE_CLIENT_ID").map(|client|brain_backend::apple::Config::new(client,setting("APPLE_APP_ID"),setting("APPLE_REDIRECT_URI").unwrap_or_else(||format!("{}/",origin.trim_end_matches('/'))))).transpose()?;
        if apple.is_some() && jwt.is_none() {return Err("Sign in with Apple requires configured native JWT sessions; set BRAIN_JWT_SECRET, issuer and audience".into());}
        let linkedin=match (setting("BRAIN_AUTH_LINKEDIN_CLIENT_ID"),setting("BRAIN_AUTH_LINKEDIN_CLIENT_SECRET")) {
            (Some(client),Some(secret))=>Some(brain_backend::linkedin::Config::new(client,secret,setting("BRAIN_AUTH_LINKEDIN_REDIRECT_URI").unwrap_or_else(||format!("{}/_brain/auth/linkedin/callback",origin.trim_end_matches('/'))))?),
            (None,None)=>None,
            _=>return Err("LinkedIn sign-in needs both BRAIN_AUTH_LINKEDIN_CLIENT_ID and BRAIN_AUTH_LINKEDIN_CLIENT_SECRET".into()),
        };
        if linkedin.is_some() && jwt.is_none() {return Err("LinkedIn sign-in requires configured native JWT sessions; set BRAIN_JWT_SECRET, issuer and audience".into());}
        let gcs=match setting("BRAIN_GCS_BUCKET") {
            Some(bucket)=>Some(brain_backend::media::GcsConfig::new(bucket,setting("BRAIN_GCS_PREFIX").unwrap_or_else(||"brain-media".into()),std::path::PathBuf::from(setting("BRAIN_GCS_ADAPTER").ok_or("B0761: BRAIN_GCS_BUCKET is set without BRAIN_GCS_ADAPTER; fix: point BRAIN_GCS_ADAPTER at the absolute path of the private .mjs SDK adapter, or remove the bucket")?))?),
            None=>None,
        };
        let public_base=setting("PUBLIC_SITE_URL").unwrap_or_else(||origin.clone());
        let mail=setting("BRAIN_MAIL_RESEND_KEY").map(|key|brain_backend::auth_flows::MailConfig::resend(key,setting("BRAIN_MAIL_FROM").ok_or("B0760: BRAIN_MAIL_RESEND_KEY is set without BRAIN_MAIL_FROM; fix: set BRAIN_MAIL_FROM to a verified sender, as `sender@your-domain` or `Name <sender@your-domain>`")?,public_base.clone())).transpose()?;
        let mail=mail.map(|mut mail|{
            if let Some([verify,reset,notice])=brain_check::i18n::load(&root).as_ref().and_then(brain_check::i18n::auth_email_text){
                mail.text=brain_backend::auth_flows::EmailText{verify,reset,notice};
            }
            mail
        });
        if google.is_some() {
            let setup = google_origin_setup(&sockets.http_origins);
            brain_backend::log_line!("{}",json!({"schemaVersion":1,"event":"google-origin-setup","environment":environment,"setup":setup}));
        }
        let config=brain_backend::Config{api_keys:crate::api_key_config::load(&root)?,sockets:sockets.native_config()?,before_hooks:crate::hooks_executor::load(&root,&environment,&resources)?,webhooks:crate::webhook_config::load(&root,&environment)?,roles,mail,auth_public_base_url:Some(public_base),gcs,jwt,google,apple,linkedin,resources,jobs,uploads:root.join(".brain/uploads"),origins:sockets.http_origins,production:environment=="production"};
        brain_backend::analytics::configure(analytics_settings(&root)?)?;
        brain_backend::log_line!("{}",json!({"schemaVersion":1,"event":"backend-starting","url":format!("http://{addr}"),"experimental":true}));
        if let Some((msg,fix))=anonymous_catalog_warning(args,&environment){brain_backend::log_line!("{}",json!({"schemaVersion":1,"event":"backend-warning","code":"B0771","msg":msg,"fix":fix}));}
        let mut platform=crate::platform_config::load(&root,&environment,&config.resources,&config.roles)?;
        if let Some((_,msg,true))=crate::platform_config::captcha_secret_problem(&root,&environment){brain_backend::log_line!("{}",json!({"schemaVersion":1,"event":"backend-warning","code":"B0790","msg":msg,"fix":"set it in .brain_env or the host environment"}));}
        platform.routes=crate::route_executor::contracts(&root,&environment)?;
        platform.disabled_webhooks=crate::webhook_config::disabled_inbound(&root,&environment);
        platform.handlers=crate::route_executor::load(&root,&environment,&platform.routes)?;
        // Separate HTTP and worker pools with declared sizes, checked against the server's connection budget.
        pool.close().await;
        let size=|key:&str,default:u32|setting(key).map(|v|v.parse::<u32>().ok().filter(|n|(1..=64).contains(n)).ok_or(format!("B0763: {key} is out of range; fix: a whole number from 1 to 64, or remove it to use the default"))).transpose().map(|v|v.unwrap_or(default));
        let (http_size,worker_size)=(size("BRAIN_DB_HTTP_POOL",8)?,size("BRAIN_DB_WORKER_POOL",2)?);
        let processes=size("BRAIN_BACKEND_PROCESSES",1)?;
        // More than one process: rate windows and socket fan-out must live in Postgres, not in one process's memory.
        if processes>1 {platform.cluster.shared=true;}
        config_phase(false);
        let http_pool=brain_backend::db::connect_named(&url,brain_backend::db::Workload::Http,http_size,&pg_name(&root,"http")).await?;
        brain_backend::db::check_connection_budget(&http_pool,(http_size+worker_size)*processes).await?;
        let worker_pool=brain_backend::db::connect_named(&url,brain_backend::db::Workload::Worker,worker_size,&pg_name(&root,"worker")).await?;
        let queues:Vec<String>=config.jobs.iter().map(|job|job.name.clone()).collect();
        let schedules:Vec<String>=domain::cron(&root,&config.jobs).unwrap_or_default().into_iter().map(|(name,_)|name).collect();
        let with_workers=args.iter().any(|a|a=="--with-workers");
        if !queues.is_empty()||!schedules.is_empty() {
            tokio::spawn(monitor_queues(brain_backend::jobs::Queue::new(worker_pool.clone()),queues.clone(),schedules.clone(),with_workers));
        }
        let workers=with_workers.then(||own_workers(&root,&environment,&format!("http://{addr}"),&queues,!schedules.is_empty()));
        let served=brain_backend::http::serve_full(listener,http_pool,worker_pool,config,domain,platform).await;
        if let Some((stop,tasks))=workers {
            let _=stop.send(true);
            for task in tasks {let _=task.await;}
        }
        served
    })
}
/// Where `brain backend serve` keeps its failure events.
fn failure_log(root: &Path) -> std::path::PathBuf {
    root.join(".brain/backend/failures.jsonl")
}
/// Kept failure events, oldest first: the ones of one request id, or the latest 50.
pub fn failure_logs(root: &Path, rid: Option<&str>) -> Value {
    let file = failure_log(root);
    let older = std::fs::read_to_string(file.with_extension("jsonl.1")).unwrap_or_default();
    let newer = std::fs::read_to_string(&file).unwrap_or_default();
    let mut events: Vec<Value> = older.lines().chain(newer.lines()).filter_map(|line| serde_json::from_str(line).ok()).collect();
    match rid {
        Some(rid) => events.retain(|event| event["rid"] == rid),
        None => {
            let extra = events.len().saturating_sub(50);
            events.drain(..extra);
        }
    }
    let empty = events.is_empty();
    let mut out = json!({"schemaVersion":1,"file":".brain/backend/failures.jsonl","events":events});
    if empty {
        out["fix"] = json!("No kept failure matches. Events are kept while `brain backend serve` runs; each one also goes to its stderr.");
    }
    out
}
/// All generated files are reserved with create_new; roll back only files created by this invocation.
pub fn add(root: &Path, kind: &str, name: &str, shapes: &str) -> Result<Value, String> {
    add_with_admin(root, kind, name, shapes, false)
}
/// Opt-in project-owned administrative views committed with the generated backend contract.
pub fn add_with_admin(root: &Path, kind: &str, name: &str, shapes: &str, admin_ui: bool) -> Result<Value, String> {
    add_with_policy(root, kind, name, shapes, admin_ui, None)
}

/// The starting policies `brain add resource --policy` offers; the file stays the place to change them.
fn policy_preset(preset: &str) -> Result<Policy, String> {
    let grants = match preset {
        "admin" => json!({"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}),
        "owner" => json!({"read":["owner","admin"],"create":["user","admin"],"update":["owner","admin"],"delete":["owner","admin"]}),
        "public-read" => json!({"read":["public"],"create":["admin"],"update":["admin"],"delete":["admin"]}),
        other => return Err(format!("unknown --policy `{other}`; fix: use admin (the default), owner or public-read")),
    };
    serde_json::from_value(grants).map_err(|e| e.to_string())
}

pub fn add_with_policy(root: &Path, kind: &str, name: &str, shapes: &str, admin_ui: bool, preset: Option<&str>) -> Result<Value, String> {
    if preset.is_some() && kind != "resource" {
        return Err("--policy is only supported for resources".into());
    }
    let policy = preset.map(policy_preset).transpose()?.unwrap_or_default();
    if admin_ui && kind != "resource" {
        return Err("--admin-ui is only supported for resources".into());
    }
    let _writer = if admin_ui { None } else { Some(crate::edit::writer_lock(root)?) };
    if !contract::identifier(name) {
        return Err(
            "Use a lowercase resource/job identifier (letters, digits, underscores)".into(),
        );
    }
    let mut pairs = BTreeMap::new();
    for shape in shapes.split_whitespace() {
        let (name, shape) = shape
            .split_once(':')
            .ok_or("fields must be name:type pairs")?;
        if pairs.insert(name.to_string(), json!(shape)).is_some() {
            return Err("Duplicate field".into());
        }
    }
    let fields = fields(json!(pairs))?;
    let r = Resource {
        search: Vec::new(),
        name: name.into(),
        fields,
        policy,
        hooks: Hooks::default(),
        constraints: Default::default(),
        query: Default::default(),
    };
    if kind == "job" { r.validate_job()? } else { r.validate()? }
    let dir = if kind == "resource" {
        "resources"
    } else if kind == "job" {
        "jobs"
    } else {
        return Err("kind must be resource or job".into());
    };
    let prefix = format!("server/{dir}/{name}");
    let mut files = vec![(
        format!("{prefix}/schema.jsbrain"),
        format!(
            "<meta server>\nexport default {}\n",
            serde_json::to_string_pretty(&pairs).unwrap()
        ),
    )];
    if kind == "resource" {
        files.push((format!("{prefix}/search.jsbrain"), "<meta server>\n// Explicit public string fields only. Never index private data or full JSON bodies.\nexport default []\n".into()));
        files.push((
            format!("{prefix}/policy.jsbrain"),
            format!(
                "<meta server>\nexport default {}\n",
                serde_json::to_string_pretty(&r.policy).unwrap()
            ),
        ));
        files.push((format!("{prefix}/hooks.jsbrain"),"<meta server>\n// Set before_create/before_update/before_delete to a server/hooks module name; return ctx.payload.\n// Set after_create/after_update/after_delete to a job name; events commit with the mutation.\nexport default {}\n".into()));
    } else {
        files.push((format!("{prefix}/handler.jsbrain"),"<meta server>\n// ctx: payload, jobId, attempt, env, backend (docs/backend/job-backend.md). Delivery is at least once; make effects idempotent by jobId.\nexport default async function handle(ctx) {\n  throw new Error('Implement this job handler before starting a worker')\n}\n".into()));
    }
    if kind == "resource" {
        files.extend(resource_support_files(&r, &files[0].1)?);
    }
    if admin_ui {
        files.extend(crate::resource_admin::files(root, &r)?);
        let contract_path = format!("server/resources/{name}/client-contract.json");
        let (_, source) = files.iter_mut().find(|(path, _)| path == &contract_path)
            .ok_or("Generated resource contract is missing")?;
        let mut contract: Value = serde_json::from_str(source).map_err(|_| "Cannot read generated resource contract")?;
        contract["adminUi"] = json!({"status":"generated","routes":[format!("/admin/{name}"),format!("/admin/{name}/new"),format!("/admin/{name}/:id"),format!("/admin/{name}/:id/edit")],"guard":format!("views/admin/{name}/_guard.jsbrain"),"projectOwned":true});
        contract["requirements"] = json!(["Review native policy and run explicit backend migration","Configure fixed BRAIN_BACKEND_URL for production and development","Sign in with an existing administrator; signup never grants admin","Run build and doctor after adapting project-owned views and controls"]);
        *source = serde_json::to_string_pretty(&contract).map_err(|_| "Cannot encode administrative contract")? + "\n";
        // Publish generated status only after every route/control has been installed.
        let index = files.iter().position(|(path, _)| path == &contract_path).ok_or("Generated resource contract is missing")?;
        let contract_file = files.remove(index);
        files.push(contract_file);
    }
    for (rel, _) in &files {
        for p in root.join(rel).ancestors().take_while(|p| *p != root) {
            if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err("Generator refuses symlink paths".into());
            }
        }
        if root.join(rel).exists() {
            return Err(format!(
                "{rel} already exists; preserve it and edit its contract"
            ));
        }
    }
    if admin_ui {
        return crate::resource_admin::commit(root, &files);
    }
    let mut written = Vec::new();
    let result = (|| {
        for (rel, body) in &files {
            crate::changes::create_source(root, rel, body)?;
            written.push(root.join(rel));
        }
        Ok::<(), String>(())
    })();
    if let Err(e) = result {
        for p in written {
            let _ = fs::remove_file(p);
        }
        return Err(e);
    }
    Ok(
        json!({"schemaVersion":1,"files":files.iter().map(|(p,_)|p).collect::<Vec<_>>(),"experimental":true,"next":if kind=="resource"{"Review policy; run brain backend schema, then brain backend migrate, then brain run (it starts the backend too); its API is /api/<name>"}else{"Implement handler; inspect schema then start brain jobs work"}}),
    )
}

/// Native contracts and executable server/client helpers; routes are an explicit opt-in.
fn resource_support_files(
    r: &Resource,
    schema_source: &str,
) -> Result<Vec<(String, String)>, String> {
    let name = &r.name;
    let repository = format!("server/resources/{name}/repository.jsbrain");
    let service = format!("server/services/{name}.jsbrain");
    let client = format!("api/{name}.jsbrain");
    let record_client = format!("api/{name}-record.jsbrain");
    let mut files = vec![];
    files.push((repository.clone(),format!("<meta server>\n// Request-scoped native policy and identity; never create another database client.\nexport default function repository(ctx) {{\n  if (!ctx.backend || typeof ctx.backend.resource !== 'function') throw Error('Native backend is not configured')\n  return ctx.backend.resource('{name}')\n}}\n")));
    files.push((service.clone(),format!(r#"<meta server>
import repository from '../resources/{name}/repository.jsbrain'
import schema from '../resources/{name}/schema.jsbrain'
function admin(ctx) {{
  if (!ctx.backend) throw Error('Native backend is not configured')
  if (!ctx.user || ctx.user.role !== 'admin') throw {{status:403,message:'Administrator access required'}}
}}
// A field is `'str!'`, `'str! private'` or `{{ type: 'str!', ... }}`; `!` (or `required: true`) marks it required.
function field(key) {{
  const spec=schema[key]
  const shape=spec && typeof spec === 'object' ? spec.type : spec
  const text=String(shape == null ? '' : shape).trim().split(' ')[0]
  return {{kind:text.replace(/!$/,''),required:text.endsWith('!') || Boolean(spec && typeof spec === 'object' && spec.required === true)}}
}}
function values(input) {{
  if (!input || typeof input !== 'object' || Array.isArray(input)) throw {{errors:{{form:'Use a resource object'}}}}
  for (const key of Object.keys(input)) {{
    if (!Object.prototype.hasOwnProperty.call(schema,key)) throw {{errors:{{form:'Remove undeclared resource fields'}}}}
  }}
  for (const key of Object.keys(schema)) {{
    if (field(key).required && input[key] == null) {{
      let errors = {{}}
      errors[key] = 'Required'
      throw {{errors}}
    }}
  }}
  return {{...input}}
}}
async function list(ctx,query) {{ admin(ctx); return await repository(ctx).list(query) }}
async function get(ctx,id) {{ admin(ctx); return await repository(ctx).get(id) }}
async function save(ctx,id,input) {{
  admin(ctx)
  const payload=values(input)
  return id ? await repository(ctx).update(id,payload) : await repository(ctx).create(payload)
}}
async function remove(ctx,id) {{ admin(ctx); return await repository(ctx).remove(id) }}
async function editor(ctx,id) {{
  admin(ctx)
  const source=id ? (await repository(ctx).get(id)).data : {{}}
  let result={{}}
  for (const key of Object.keys(schema)) {{
    const kind=field(key).kind
    const value=source[key]
    result[key]=kind === 'obj' ? (value == null ? '' : JSON.stringify(value)) : kind === 'bool' ? Boolean(value) : kind === 'num' ? (value == null ? '' : value) : value == null ? '' : String(value)
  }}
  return result
}}
async function saveForm(ctx,id,input) {{
  admin(ctx)
  const payload=values(input)
  for (const key of Object.keys(payload)) {{
    const kind=field(key).kind
    const value=payload[key]
    let invalid=false
    if (kind === 'obj') {{
      try {{ payload[key]=value === '' ? {{}} : JSON.parse(value) }} catch {{ invalid=true }}
      if (!payload[key] || typeof payload[key] !== 'object' || Array.isArray(payload[key])) invalid=true
    }} else if (kind === 'bool') {{
      if (typeof value !== 'boolean') invalid=true
    }} else if (kind === 'num') {{
      if (value === '' || value == null) payload[key]=null
      else if (typeof value === 'number') {{if (!Number.isFinite(value)) invalid=true}}
      else if (typeof value !== 'string' || !/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(value) || !Number.isFinite(Number(value))) invalid=true
      else payload[key]=Number(value)
    }} else if ((kind === 'str' || kind === 'email') && value != null) {{
      if (typeof value !== 'string' || (field(key).required && !value.trim())) invalid=true
    }}
    if (invalid) {{let errors={{}};errors[key]='Use a valid field value';throw {{errors}}}}
  }}
  return await save(ctx,id,payload)
}}
export default {{list,get,save,saveForm,remove,editor}}
"#)));
    files.push((client.clone(),format!("// Native resource schema supplies the inferred list/row shape; private data is never cached.\nexport default api('/api/{name}',{{get:{{}},post:{{}}}})\n")));
    files.push((
        record_client.clone(),
        format!("export default api('/api/{name}/:id',{{get:{{}},put:{{}},delete:{{}}}})\n"),
    ));
    let contract_path = format!("server/resources/{name}/client-contract.json");
    files.push((contract_path,serde_json::to_string_pretty(&json!({"schemaVersion":1,"resource":name,"schema":format!("server/resources/{name}/schema.jsbrain"),"schemaRevision":crate::manifests::hash(schema_source.as_bytes()),"fields":r.fields,"policy":format!("server/resources/{name}/policy.jsbrain"),"client":[client,record_client],"repository":repository,"service":service,"adminUi":{"status":"planned","reason":"Administrative routes require explicit --admin-ui opt-in"},"authorization":"Native backend policy is authoritative; generated services additionally require authoritative admin context","requirements":["Review policy and run explicit backend migration","Configure fixed BRAIN_BACKEND_URL for production and development","Sign in with an existing administrator; signup never grants admin","Build application views deliberately; this command does not create routes, guards or controls"],"generatedSnapshot":true})).map_err(|_|"Cannot encode generated resource contract")?+"\n"));
    Ok(files)
}


pub(crate) struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn worker_module(root: &Path, name: &str) -> Result<(Scratch, PathBuf), String> {
    let rel = format!("server/jobs/{name}/handler.jsbrain");
    // The same refusals as a route handler, before any job is claimed: otherwise every attempt fails and retries.
    let source = crate::map::read_source_limit(root, &rel, 256 * 1024).map_err(|reason| format!("{rel}: handler source refused: {reason}; run brain check"))?;
    let exports_default = brain_parse::parse(&source, Path::new(&rel)).is_ok_and(|c| c.script.iter().any(|s| matches!(s, brain_parse::ast::Stmt::ExportDefault { .. })));
    let module = server_module(root, &rel)?;
    if !exports_default {
        return Err(format!("B0772 {rel}: this handler has no default export; fix: add `export default async function handle(ctx) {{ ... }}`"));
    }
    Ok(module)
}
/// B0771: the API catalog answers anyone unless the process runs with `--env production`. A plain development run says
/// so through `--env development` and stays quiet; a serve with no `--env`, or any other name, is usually a deployment.
fn anonymous_catalog_warning(args: &[String], environment: &str) -> Option<(String, String)> {
    let chosen = args.iter().any(|arg| arg == "--env" || arg.starts_with("--env="));
    (environment != "production" && (!chosen || environment != "development")).then(|| {
        (
            format!("`/_brain/api` lists every resource, field and route to anonymous callers because this process runs with environment `{environment}`"),
            "start deployed servers with `brain backend serve --env production`, which limits the catalog to admins and turns on the other production defaults".to_string(),
        )
    })
}

pub(crate) fn server_module(root: &Path, rel: &str) -> Result<(Scratch, PathBuf), String> {
    let mut files = BTreeMap::new();
    let mut written = BTreeMap::new();
    let compile = |src: &str, path: &Path| {
        crate::compiler::compile(src, path, root, true)
            .map(|o| brain_pack::Unit {
                js: o.js,
                css: o.css,
                server_js: o.server_js,
                server: o.server, server_fns: o.server_fns,
            })
            .map_err(|d| crate::diags_json(&d))
    };
    let native: BTreeMap<String, String> = crate::runtime_files()
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "brain.js" | "brain-budget.js" | "brain-time.js" | "brain-hash.js" | "brain-outbound.js"
            )
        })
        .collect();
    let used = std::cell::RefCell::new(std::collections::BTreeSet::new());
    let entry = brain_pack::server::emit(
        root,
        &rel,
        &compile,
        &|name| {
            native.contains_key(name).then(|| {
                used.borrow_mut().insert(name.to_string());
                format!("./_runtime/{name}.mjs")
            })
        },
        &mut written,
        &mut files,
        0,
    )?;
    let mut pending: Vec<String> = used.into_inner().into_iter().collect();
    let mut emitted = std::collections::HashSet::new();
    while let Some(name) = pending.pop() {
        if !emitted.insert(name.clone()) {
            continue;
        }
        let mut code = native
            .get(&name)
            .ok_or("Job imports an unsupported native runtime module")?
            .clone();
        if name == "brain.js" {
            let keep = ["_rt", "fx", "untrack"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            code = brain_min::tree_shake(&code, &keep)?;
        }
        let code = brain_pack::rewrite_imports(&code, &mut |specifier| {
            if let Some(dependency) = specifier.strip_prefix("./") {
                if !native.contains_key(dependency) {
                    return Err("Job runtime dependency is not supported on the backend".into());
                }
                pending.push(dependency.to_owned());
                return Ok(Some(format!("./{dependency}.mjs")));
            }
            Ok(None)
        })?;
        files.insert(format!("_server/_runtime/{name}.mjs"), code);
    }
    let trace_modules: BTreeMap<_, _> = written
        .iter()
        .map(|(source, generated)| {
            (
                generated.trim_start_matches("_server/").to_string(),
                source.clone(),
            )
        })
        .collect();
    let trace_index = json!({"schemaVersion":1,"modules":trace_modules}).to_string();
    if trace_modules.len() > 2048 || trace_index.len() > 262144 {
        return Err(
            "Job module graph exceeds the private trace-index limit; split the handler's imports"
                .into(),
        );
    }
    files.insert("_server/trace-index.json".into(), trace_index);
    let stamp = brain_pack::content_hash(&format!("{:?}", std::time::SystemTime::now()));
    let scratch = Scratch(
        root.join(".brain/jobs")
            .join(format!("{}-{stamp}", std::process::id())),
    );
    for (name, code) in files {
        let path = scratch.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(path, code).map_err(|e| e.to_string())?;
    }
    let path = fs::canonicalize(scratch.0.join(entry)).map_err(|e| e.to_string())?;
    Ok((scratch, path))
}

/// One liveness finding: code, file, message, fix.
pub(crate) type Finding = (&'static str, String, String, String);

/// Stats of the declared queues plus any other queue that still holds active jobs (`declared: false`).
async fn queue_rows(queue: &brain_backend::jobs::Queue, declared: &[String]) -> Result<Vec<Value>, String> {
    let mut names: Vec<String> = declared.to_vec();
    names.extend(queue.active_queues().await?.into_iter().filter(|name| !declared.contains(name)));
    let mut rows = Vec::with_capacity(names.len());
    for name in names {
        let mut row = queue.stats(&name).await?;
        row["declared"] = json!(declared.contains(&name));
        rows.push(row);
    }
    Ok(rows)
}

/// What the rows mean for an operator. Pure, so the wording and the conditions have a unit test.
fn liveness_findings(rows: &[Value], schedules: &[String], schedulers: i64) -> Vec<Finding> {
    let mut out = Vec::new();
    for row in rows {
        let name = row["queue"].as_str().unwrap_or("");
        let (pending, oldest) = (row["pending"].as_i64().unwrap_or(0), row["oldestWaitSeconds"].as_i64().unwrap_or(0));
        if row["declared"] != json!(true) {
            out.push(("B0768", format!("server/jobs/{name}/"),
                format!("queue `{name}` holds {pending} due job(s) but no longer has a declaration under server/jobs, so no worker can run them"),
                format!("restore server/jobs/{name}/ and run `brain jobs work {name}`, or inspect them with `brain jobs stats {name}`")));
        } else if row["workers"].as_i64().unwrap_or(0) == 0 {
            let backlog = if pending > 0 { format!("{pending} job(s) due, the oldest waiting {oldest} s") } else { "nothing due yet, but new jobs will wait".to_string() };
            out.push(("B0765", format!("server/jobs/{name}/"),
                format!("queue `{name}` has no worker reporting to this database in the last 45 s: {backlog}; a worker in another process counts once it reports here, and one on another database or environment never does; `brain backend serve` and `brain start` do not process queues"),
                format!("run `brain jobs work {name}` as its own process, or `brain backend serve --with-workers` for a small deployment")));
        }
    }
    if !schedules.is_empty() && schedulers == 0 {
        out.push(("B0767", "server/cron/".to_string(),
            format!("{} schedule(s) declared ({}) but no scheduler is running, so no slot is enqueued", schedules.len(), schedules.join(", ")),
            "run `brain jobs cron` as its own process, or `brain backend serve --with-workers`".to_string()));
    }
    out
}

/// One-shot liveness check for `brain doctor`, `brain run` and `brain start`. Never fails: when the database cannot
/// answer, that is the finding (B0766). Workers need the same database, so they are not running either.
pub(crate) fn queue_findings(root: &Path, environment: &str) -> Vec<Finding> {
    let unknown = |message: &str, fix: &str| vec![("B0766", crate::env::FILE.to_string(), message.to_string(), fix.to_string())];
    let Ok(jobs) = contracts(root, "jobs") else { return vec![] };
    let queues: Vec<String> = jobs.iter().map(|job| job.name.clone()).collect();
    let schedules: Vec<String> = domain::cron(root, &jobs).unwrap_or_default().into_iter().map(|(name, _)| name).collect();
    if queues.is_empty() && schedules.is_empty() {
        return vec![];
    }
    let Ok(url) = database(root, environment) else {
        // A front-only process: its backend, and the queues with it, run on another host.
        let backend = std::env::var("BRAIN_BACKEND_URL").ok().or_else(|| crate::env::value(root, environment, "BRAIN_BACKEND_URL"));
        if backend.as_deref().is_some_and(|url| is_remote_backend(url) || answers(url)) {
            return vec![];
        }
        return unknown("queue workers were not checked: no DATABASE_URL, so jobs can be neither stored nor run here", "set DATABASE_URL in .brain_env and run `brain backend migrate`");
    };
    let Ok(rt) = runtime() else { return vec![] };
    let checked = rt.block_on(async {
        let boot = brain_backend::boot::Boot::command(environment == "production");
        let pool = tokio::time::timeout(std::time::Duration::from_secs(5), brain_backend::boot::connect(&url, &boot)).await.map_err(|_| "timeout")?.map_err(|_| "connect")?;
        let queue = brain_backend::jobs::Queue::new(pool.clone());
        let rows = queue_rows(&queue, &queues).await.map_err(|_| "query")?;
        let schedulers = queue.schedulers().await.map_err(|_| "query")?;
        pool.close().await;
        Ok::<_, &str>(liveness_findings(&rows, &schedules, schedulers))
    });
    match checked {
        Ok(findings) => findings,
        Err("query") => unknown("queue workers were not checked: the database is reachable but has no worker table", "run `brain backend migrate`"),
        Err(_) => unknown("queue workers were not checked: the database is unreachable. Stored jobs stay safe and wait, but no worker can claim them and no new job can be enqueued until it returns", "start Postgres or correct DATABASE_URL, then run `brain doctor` again"),
    }
}

/// B0778: accounts whose role `server/auth/roles.jsbrain` does not declare. Every grant refuses them with a plain 403, and
/// nothing in the response says why. Silent when the database cannot answer: the queue check already reports that.
pub(crate) fn role_findings(root: &Path, environment: &str) -> Vec<Finding> {
    let (Ok(url), Ok(config)) = (database(root, environment), auth_roles(root)) else { return vec![] };
    let Ok(rt) = runtime() else { return vec![] };
    let rows: Vec<(String, i64)> = rt
        .block_on(async {
            let boot = brain_backend::boot::Boot::command(environment == "production");
            let pool = tokio::time::timeout(std::time::Duration::from_secs(5), brain_backend::boot::connect(&url, &boot)).await.ok()?.ok()?;
            let rows = brain_backend::auth::role_counts(&pool).await.ok();
            pool.close().await;
            rows
        })
        .unwrap_or_default();
    undeclared_roles(&rows, &config.roles)
}

fn undeclared_roles(rows: &[(String, i64)], declared: &[String]) -> Vec<Finding> {
    rows.iter()
        .filter(|(role, _)| !declared.contains(role))
        .map(|(role, count)| ("B0778", "server/auth/roles.jsbrain".to_string(), format!("{count} account(s) have the role `{role}`, which is not declared here, so every grant answers them 403 POLICY_DENIED"), format!("add '{role}' to roles, or change those accounts with POST /_brain/auth/users/{{id}}/role")))
        .collect()
}

/// Run the one-shot check beside a starting server, so a slow database never delays it.
pub(crate) fn warn_queues_in_background(root: &Path, environment: &str) {
    if !root.join("server/jobs").is_dir() && !root.join("server/cron").is_dir() {
        return;
    }
    let (root, environment) = (root.to_path_buf(), environment.to_string());
    std::thread::spawn(move || {
        let mut findings = queue_findings(&root, &environment);
        if waits_for_siblings(&findings) {
            std::thread::sleep(std::time::Duration::from_secs(WORKER_START_GRACE_SECONDS));
            findings = queue_findings(&root, &environment);
        }
        for (code, file, message, fix) in findings {
            brain_backend::log_line!("{}", json!({"schemaVersion":1,"event":"queue-warning","code":code,"file":file,"msg":message,"fix":fix,"valuesCaptured":false}));
        }
    });
}

/// A static sibling of the same failure: a job with a schema but no handler can be enqueued and can never run.
pub fn job_warnings(root: &Path) -> Vec<Finding> {
    let names = contracts(root, "jobs").unwrap_or_default();
    names.iter().filter(|job| !root.join("server/jobs").join(&job.name).join("handler.jsbrain").is_file()).map(|job| {
        ("B0769", format!("server/jobs/{}/", job.name),
            format!("job `{}` has a schema but no handler.jsbrain: it can be enqueued and no worker can ever run it", job.name),
            format!("add server/jobs/{}/handler.jsbrain with `export default async function handle(ctx) {{ … }}`", job.name))
    }).collect()
}

/// A handler that imports a browser module compiles on its own and only fails when its worker or route starts: a job
/// restarted 48 times before anyone saw it. The same bundling the process does, before anything starts.
pub fn handler_bundle_problems(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for kind in ["jobs", "routes"] {
        let Ok(entries) = fs::read_dir(root.join("server").join(kind)) else { continue };
        let mut names: Vec<String> = entries.flatten().filter(|e| e.path().join("handler.jsbrain").is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for name in names {
            let rel = format!("server/{kind}/{name}/handler.jsbrain");
            if let Err(message) = server_module(root, &rel)
                && message.contains("which the build did not emit")
            {
                out.push((rel, message));
            }
        }
    }
    out
}

/// A worker restarted with the server needs a moment to report in: no warning until it has had that moment.
const WORKER_START_GRACE_SECONDS: u64 = 10;

/// A deploy restarts the server, its workers and its scheduler together: a queue with no worker (B0765) or schedules with
/// no scheduler (B0767) on the first read may only mean the sibling has not reported yet.
fn waits_for_siblings(findings: &[Finding]) -> bool {
    findings.iter().any(|(code, ..)| matches!(*code, "B0765" | "B0767"))
}

fn first_read_waits(rows: &[Value], schedules: &[String], schedulers: i64, graced: bool, own_workers: bool) -> bool {
    !graced && !own_workers && waits_for_siblings(&liveness_findings(rows, schedules, schedulers))
}

/// Backend startup list, then a periodic `queue-stalled` line for each queue not being drained. Counts and ages only:
/// never a payload value. `BRAIN_QUEUE_CHECK_SECONDS` sets the period (5..3600, default 60).
async fn monitor_queues(queue: brain_backend::jobs::Queue, queues: Vec<String>, schedules: Vec<String>, own_workers: bool) {
    /// A due job this old means the queue is not keeping up even with workers alive.
    const BEHIND_SECONDS: i64 = 300;
    let period = std::env::var("BRAIN_QUEUE_CHECK_SECONDS").ok().and_then(|v| v.parse::<u64>().ok()).filter(|n| (5..=3600).contains(n)).unwrap_or(60);
    if own_workers {
        // Give the workers started beside this server time to report in before the first list.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    let mut first = true;
    let mut unknown = false;
    let mut graced = false;
    loop {
        let read = async { Ok::<_, String>((queue_rows(&queue, &queues).await?, queue.schedulers().await?)) };
        match read.await {
            Ok((rows, schedulers)) => {
                unknown = false;
                if first && first_read_waits(&rows, &schedules, schedulers, graced, own_workers) {
                    graced = true;
                    tokio::time::sleep(std::time::Duration::from_secs(WORKER_START_GRACE_SECONDS)).await;
                    continue;
                }
                if first {
                    let list: Vec<Value> = rows.iter().map(|row| json!({"queue":row["queue"],"pending":row["pending"],"oldestWaitSeconds":row["oldestWaitSeconds"],"workers":row["workers"]})).collect();
                    brain_backend::log_line!("{}", json!({"schemaVersion":1,"event":"queue-workers","queues":list,"schedules":schedules,"schedulers":schedulers,"valuesCaptured":false}));
                    for (code, file, message, fix) in liveness_findings(&rows, &schedules, schedulers) {
                        brain_backend::log_line!("{}", json!({"schemaVersion":1,"event":"queue-warning","code":code,"file":file,"msg":message,"fix":fix,"valuesCaptured":false}));
                    }
                    first = false;
                }
                for row in &rows {
                    let (pending, oldest, workers) = (row["pending"].as_i64().unwrap_or(0), row["oldestWaitSeconds"].as_i64().unwrap_or(0), row["workers"].as_i64().unwrap_or(0));
                    if pending > 0 && (workers == 0 || oldest >= BEHIND_SECONDS) {
                        let name = row["queue"].as_str().unwrap_or("");
                        brain_backend::log_line!("{}", json!({"schemaVersion":1,"event":"queue-stalled","code":"B0765","queue":name,"pending":pending,"oldestWaitSeconds":oldest,"workers":workers,"fix":format!("run `brain jobs work {name}`; `brain jobs stats` shows every queue"),"valuesCaptured":false}));
                    }
                }
            }
            Err(_) if !unknown => {
                unknown = true;
                brain_backend::log_line!("{}", json!({"schemaVersion":1,"event":"queue-liveness-unknown","code":"B0766","fix":"Postgres is not answering: stored jobs wait, and this check resumes when it returns","valuesCaptured":false}));
            }
            Err(_) => {}
        }
        tokio::time::sleep(std::time::Duration::from_secs(period)).await;
    }
}

/// `brain backend serve --with-workers`: one supervised child per declared queue (and one scheduler), for a deployment
/// small enough to run them together. Children are the same `brain jobs` commands, so a handler cannot take the server
/// down. A stopped server stops them and lets each drain; one that dies is restarted with a capped backoff.
fn own_workers(root: &Path, environment: &str, backend: &str, queues: &[String], schedules: bool) -> (tokio::sync::watch::Sender<bool>, Vec<tokio::task::JoinHandle<()>>) {
    let (stop, recv) = tokio::sync::watch::channel(false);
    let mut commands: Vec<Vec<String>> = queues.iter().map(|queue| vec!["work".to_string(), queue.clone()]).collect();
    if schedules {
        commands.push(vec!["cron".to_string()]);
    }
    let backend_set = std::env::var_os("BRAIN_BACKEND_URL").is_some() || crate::env::value(root, environment, "BRAIN_BACKEND_URL").is_some();
    let tasks = commands.into_iter().map(|command| {
        let (root, environment, backend, mut recv) = (root.to_path_buf(), environment.to_string(), backend.to_string(), recv.clone());
        tokio::spawn(async move {
            let mut backoff = 1u64;
            loop {
                let Ok(program) = std::env::current_exe() else { return };
                let mut cmd = tokio::process::Command::new(program);
                cmd.arg("jobs").args(&command).args(["--env", &environment]).current_dir(&root).stdin(Stdio::null()).kill_on_drop(true).env(OWNED_BY, std::process::id().to_string());
                if !backend_set {
                    cmd.env("BRAIN_BACKEND_URL", &backend);
                }
                let started = std::time::Instant::now();
                let Ok(mut child) = cmd.spawn() else { return };
                tokio::select! {
                    status = child.wait() => {
                        if started.elapsed().as_secs() > 60 { backoff = 1; }
                        let code = status.ok().and_then(|s| s.code());
                        if !restart_worker(code) {
                            brain_backend::log_line!("{}", json!({"schemaVersion":1,"event":"worker-stopped","command":command,"code":code,"reason":"its configuration stops it and a restart cannot fix that; the reason is logged above","fix":"correct the setting and restart the server"}));
                            return;
                        }
                        brain_backend::log_line!("{}", json!({"schemaVersion":1,"event":"worker-exited","command":command,"code":code,"restartSeconds":backoff,"valuesCaptured":false}));
                    }
                    _ = recv.changed() => {
                        #[cfg(unix)]
                        if let Some(pid) = child.id() {
                            // SAFETY: a signal to the child this task owns and has not reaped yet.
                            unsafe { libc::kill(pid as i32, libc::SIGTERM); }
                        }
                        // The longest drain is 600 s by policy; past that the child is dropped and killed.
                        let _ = tokio::time::timeout(std::time::Duration::from_secs(610), child.wait()).await;
                        return;
                    }
                }
                tokio::select! {
                    _ = recv.changed() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(backoff)) => {}
                }
                backoff = (backoff * 2).min(30);
            }
        })
    }).collect();
    (stop, tasks)
}
/// Set on workers a server owns: they stop if that server dies without stopping them (SIGKILL), so a restart under a
/// process manager never leaves a second set running.
const OWNED_BY: &str = "BRAIN_WORKER_PARENT";
async fn owner_gone() {
    let owner = std::env::var(OWNED_BY).ok().and_then(|v| v.parse::<i32>().ok());
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        #[cfg(unix)]
        // SAFETY: getppid has no preconditions.
        if owner.is_some_and(|pid| unsafe { libc::getppid() } != pid) {
            return;
        }
        #[cfg(not(unix))]
        let _ = owner;
    }
}

/// The validated payload of `brain jobs enqueue <queue>`: `--payload '<json>'`, or a single positional JSON argument
/// after the queue (`brain jobs enqueue mail '{"to":"a@example.com"}'`). Nothing given is `{}`, which the job's schema
/// still validates. Every refusal names `--payload` as the fix.
fn enqueue_payload(args: &[String], name: &str, job: &Resource) -> Result<Value, String> {
    let valued = crate::flags::TABLE.iter().find(|(command, ..)| *command == "jobs").map_or(&[][..], |(_, valued, _)| *valued);
    let extra: Vec<String> = crate::positional(args, valued).into_iter().skip(2).collect();
    let example = format!("brain jobs enqueue {name} --payload '{{\"field\":\"value\"}}'");
    let given = match (crate::flag(args, "--payload"), extra.as_slice()) {
        (Some(_), [first, ..]) => return Err(format!("`brain jobs enqueue` got both --payload and a positional argument `{first}`; fix: pass the JSON with --payload only, e.g. {example}")),
        (_, [_, _, ..]) => return Err(format!("`brain jobs enqueue` takes one queue and one payload; fix: pass the JSON with --payload, e.g. {example}")),
        (Some(text), []) => Some(text),
        (None, [text]) => Some(text.clone()),
        (None, []) => None,
    };
    let supplied = given.is_some();
    let text = given.unwrap_or_else(|| "{}".to_string());
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("Invalid payload JSON ({e}); fix: pass the JSON with --payload, quoted for the shell, e.g. {example}"))?;
    job.input(&value).map_err(|fields| {
        let error = json!({"code":"INVALID_JOB_PAYLOAD","fields":fields}).to_string();
        if supplied { error } else { format!("{error}; fix: pass the JSON with --payload, e.g. {example}") }
    })
}

pub fn jobs(args: &[String]) -> Result<(), String> {
    let root = crate::app_root()?;
    let environment = crate::env::name(args, "development");
    let op = args.first().map(String::as_str).unwrap_or("");
    if !matches!(op, "enqueue" | "status" | "work" | "prune" | "dead" | "retry" | "stats" | "cron") {
        return Err("usage: brain jobs enqueue <queue> [--payload '<json>'] [--key K] [--priority N] [--delay S] (example: brain jobs enqueue order_receipts --payload '{\"order_id\":\"A-17\"}') | status <id> | work <queue> [--concurrency 2] | dead <queue> | retry <queue> [--id <uuid>] | stats [queue] | cron [--once] | prune".into());
    }
    let snapshot = inspect(&root)?;
    let long_running = matches!(op, "work" | "cron");
    config_phase(long_running);
    let url = stated_database(args, &root, &environment)?;
    if long_running && let Some(refusal) = crate::webhook_config::secret_problem(&root, &environment).filter(|p| !p.2).map(|(file, message, _)| format!("B0790: {file}: {message}; fix: set the secret in the process environment or .brain_env")) {
        return Err(refusal);
    }
    let name = args.get(1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_default();
    let declared: Vec<String> = contracts(&root, "jobs")?.into_iter().map(|job| job.name).collect();
    if name.is_empty() && matches!(op, "enqueue" | "work" | "dead" | "retry") {
        let example = declared.first().map_or("order_receipts", String::as_str);
        let list = if declared.is_empty() { "none yet; add one with `brain add job`".to_string() } else { declared.join(", ") };
        return Err(format!("`brain jobs {op}` needs a queue name; declared queues: {list}; example: brain jobs {op} {example}"));
    }
    let policy = if matches!(op, "enqueue" | "work") { Some(domain::queue_policy(&root, &name)?) } else { None };
    let job_contract = if matches!(op, "enqueue" | "work") {
        Some(
            contracts(&root, "jobs")?
                .into_iter()
                .find(|j| j.name == name)
                .ok_or("Unknown job; use brain add job and brain backend schema")?,
        )
    } else {
        None
    };
    // Resolved before any database connection: a payload mistake needs no Postgres to be reported.
    let enqueued = if op == "enqueue" { Some(enqueue_payload(args, &name, job_contract.as_ref().unwrap())?) } else { None };
    let module = if op == "work" {
        Some(worker_module(&root, &name)?)
    } else {
        None
    };
    let mut env: Vec<_> = crate::env::select(&root, &environment);
    env.retain(|(name, _)| name != "BRAIN_JOB_RUN_AS");
    if policy.as_ref().is_some_and(|p| p.run_as.as_deref() == Some("system")) {
        env.push(("BRAIN_JOB_RUN_AS".to_string(), "system".to_string()));
    }
    let _ = snapshot;
    runtime()?.block_on(async {
        let production = environment == "production";
        let boot = if matches!(op, "work" | "cron") { brain_backend::boot::Boot::server(production) } else { brain_backend::boot::Boot::command(production) }.from_source(database_source(&root, &environment));
        config_phase(false);
        let pool = brain_backend::boot::connect(&url, &boot).await?;
        brain_backend::db::verify(&pool, &contracts(&root, "resources")?).await?;
        let queue = brain_backend::jobs::Queue::new(pool.clone());
        let webhooks = if op == "work" {
            crate::webhook_config::load(&root,&environment)?.map(|config|brain_backend::webhooks::Webhooks::new(pool.clone(),config,&contracts(&root,"jobs")?)).transpose()?
        } else {None};

        match op {
            "enqueue" => {
                let payload = enqueued.clone().unwrap();
                let policy = policy.as_ref().unwrap();
                let number = |flag: &str, default: i64| -> Result<i64, String> {
                    crate::flag(args, flag).map_or(Ok(default), |v| v.parse().map_err(|_| format!("{flag} must be an integer")))
                };
                let key = crate::flag(args, "--key");
                let options = brain_backend::jobs::Enqueue {
                    key: key.as_deref(),
                    attempts: Some(policy.attempts),
                    priority: i16::try_from(number("--priority", policy.priority.into())?).map_err(|_| "--priority must be -100..100")?,
                    delay_seconds: number("--delay", 0)?,
                };
                let id = queue.enqueue_with(&name, payload, &options).await?;
                println!("{}", json!({"schemaVersion":1,"id":id}));
            }
            "status" => {
                let id = name.parse().map_err(|_| "Job id must be a UUID")?;
                println!(
                    "{}",
                    json!({"schemaVersion":1,"job":queue.status(id).await?})
                );
            }
            "dead" => {
                println!("{}", json!({"schemaVersion":1,"dead":queue.dead(&name, 100).await?}));
            }
            "retry" => {
                let id = crate::flag(args, "--id").map(|v| v.parse().map_err(|_| "--id must be a UUID")).transpose()?;
                println!("{}", json!({"schemaVersion":1,"revived":queue.retry_dead(&name, id, 1000).await?}));
            }
            "cron" => {
                let schedules = domain::cron(&root, &contracts(&root, "jobs")?)?;
                if schedules.is_empty() {
                    return Err("No schedules; add server/cron/<name>.jsbrain".into());
                }
                let print = |name: &str, slot: brain_backend::schedules::Slot, id: brain_backend::schedules::JobId| println!("{}", json!({"schemaVersion":1,"event":"cron-enqueued","schedule":name,"slot":slot.format("%Y-%m-%dT%H:%M:%SZ").to_string(),"id":id}));
                if args.iter().any(|a| a == "--once") {
                    for (name, slot, id) in brain_backend::schedules::tick(&queue, &schedules, brain_backend::schedules::now()).await? {
                        print(&name, slot, id);
                    }
                } else {
                    let (send, recv) = tokio::sync::watch::channel(false);
                    brain_backend::log_line!("{}", json!({"schemaVersion":1,"event":"cron-ready","schedules":schedules.len()}));
                    let runner = brain_backend::schedules::run(queue.clone(), schedules, recv, print);
                    tokio::pin!(runner);
                    tokio::select! {
                        result = &mut runner => result?,
                        _ = brain_backend::http::wait_shutdown() => { let _ = send.send(true); runner.await?; }
                        _ = owner_gone() => { let _ = send.send(true); runner.await?; }
                    }
                }
            }
            "stats" => {
                if name.is_empty() {
                    let schedules = domain::cron(&root, &contracts(&root, "jobs")?).unwrap_or_default().len();
                    println!("{}", json!({"schemaVersion":1,"queues":queue_rows(&queue, &declared).await?,"schedules":schedules,"schedulers":queue.schedulers().await?}));
                } else {
                    let mut stats = queue.stats(&name).await?;
                    stats["declared"] = json!(declared.contains(&name));
                    println!("{}", json!({"schemaVersion":1,"stats":stats}));
                }
            }
            "prune" => {
                println!(
                    "{}",
                    json!({"schemaVersion":1,"removed":queue.prune().await?})
                );
            }
            "work" => {
                let mut policy = policy.clone().unwrap();
                let n = crate::flag(args, "--concurrency")
                    .unwrap_or(policy.concurrency.to_string())
                    .parse()
                    .map_err(|_| "Concurrency must be 1..8")?;
                if !(1..=8).contains(&n) {
                    return Err("Concurrency must be 1..8".into());
                }
                let module = module.as_ref().unwrap().1.clone();
                let contract = job_contract.clone().unwrap();
                let (send, recv) = tokio::sync::watch::channel(false);
                brain_backend::log_line!(
                    "{}",
                    json!({"schemaVersion":1,"event":"worker-ready","queue":name,"concurrency":n,"version":env!("CARGO_PKG_VERSION")})
                );
                policy.concurrency = n;
                let worker = queue.work_with(&name, &policy, recv, move |mut job| {
                    let (module, env, contract, webhooks) = (module.clone(), env.clone(), contract.clone(), webhooks.clone());
                    async move {
                        if let Some(webhooks) = webhooks {
                            if let Some(payload) = webhooks.decode_job(&job).await.map_err(|_| "Webhook work payload could not be authorized")? {job.payload=payload;}
                        }
                        contract
                            .input(&job.payload)
                            .map_err(|_| "Job payload does not match its schema")?;
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};
                        // Child stderr is untrusted input. Retain a bounded prefix, drain the remainder, and
                        // reconstruct only source/index/coordinate fields; never echo application stderr.
                        fn coordinate(value: &Value) -> Option<u64> {
                            value.as_u64().filter(|n| (1..=10_000_000).contains(n))
                        }
                        fn source_path_valid(source: &str) -> bool {
                            source.len() <= 512
                                && !source.starts_with('/')
                                && !source.contains(['\\', '?', '#'])
                                && !source.chars().any(char::is_control)
                                && source
                                    .split('/')
                                    .all(|p| !p.is_empty() && p != "." && p != "..")
                        }
                        fn clean_frame(frame: &Value, modules: &BTreeMap<String, String>) -> Option<Value> {
                            let generated = frame.get("generated")?;
                            let file = generated.get("file")?.as_str()?;
                            let key = file.strip_prefix("_server/")?;
                            if key.contains('/') || key.len() > 85 {
                                return None;
                            }
                            let source = modules.get(key)?;
                            if !source_path_valid(source) {
                                return None;
                            }
                            let generated_line = coordinate(&generated["line"])?;
                            let generated_column = coordinate(&generated["column"]);
                            let line = if frame["precision"] == "source-line"
                                && frame["file"].as_str() == Some(source.as_str())
                            {
                                coordinate(&frame["line"])
                            } else {
                                None
                            };
                            Some(json!({"file":source,"line":line,"column":Value::Null,
                            "precision":if line.is_some(){"source-line"}else{"module-only"},
                            "generated":{"file":file,"line":generated_line,"column":generated_column}}))
                        }
                        fn clean_group(
                            group: &Value,
                            modules: &BTreeMap<String, String>,
                            remaining: &mut usize,
                        ) -> Value {
                            let mut frames = Vec::new();
                            if let Some(items) = group["frames"].as_array() {
                                for frame in items.iter().take(24) {
                                    if *remaining == 0 {
                                        break;
                                    }
                                    if let Some(clean) = clean_frame(frame, modules) {
                                        frames.push(clean);
                                        *remaining -= 1;
                                    }
                                }
                            }
                            let location = frames.first().cloned().unwrap_or_else(|| {
                            group["location"]["file"].as_str()
                                .filter(|source|source_path_valid(source) && modules.values().any(|known|known.as_str()==*source))
                                .map(|source|json!({"file":source,"line":Value::Null,"column":Value::Null,"precision":"module-only","generated":Value::Null}))
                                .unwrap_or(Value::Null)
                        });
                            let supplied = group["frames"].as_array().map_or(0, Vec::len);
                            json!({"location":location,"frames":frames,"truncated":{
                            "frames":group["truncated"]["frames"].as_bool().unwrap_or(false) || supplied>frames.len(),
                            "stackUnavailable":group["truncated"]["stackUnavailable"].as_bool().unwrap_or(false)}})
                        }
                        let trace_modules: BTreeMap<String, String> = module
                            .parent()
                            .map(|dir| dir.join("trace-index.json"))
                            .filter(|p| fs::metadata(p).is_ok_and(|m| m.len() <= 262144))
                            .and_then(|p| fs::read(p).ok())
                            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                            .filter(|v| v["schemaVersion"] == 1)
                            .and_then(|v| serde_json::from_value(v["modules"].clone()).ok())
                            .filter(|m: &BTreeMap<String, String>| m.len() <= 2048)
                            .unwrap_or_default();
                        let job_id = job.id.to_string();
                        let queue_name = job.queue.clone();
                        let runner = format!("{}\n{}\n{}", brain_serve::load::llm_script(), include_str!("templates/job-runner.mjs"), include_str!("../../brain-serve/src/key-backend.js"));
                        let mut cmd = tokio::process::Command::new("node");
                        cmd.args([
                            "--enable-source-maps",
                            "--max-old-space-size=256",
                            "--input-type=module",
                            "--eval",
                            &runner,
                        ])
                        .arg(module)
                        .envs(env)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .kill_on_drop(true);
                        #[cfg(unix)]
                        cmd.process_group(0);
                        let mut child = cmd.spawn().map_err(|_| "Cannot start Node job handler")?;
                        let _group = HandlerGroup(child.id());
                        let mut stderr = child.stderr.take().ok_or("Missing handler stderr")?;
                        let bytes = json!({"jobId":job.id,"attempt":job.attempt,"payload":job.payload}).to_string();
                        child
                            .stdin
                            .take()
                            .ok_or("Missing handler stdin")?
                            .write_all(bytes.as_bytes())
                            .await
                            .map_err(|_| "Cannot send job payload")?;
                        let drain = async move {
                            let mut retained = Vec::with_capacity(65536);
                            let mut chunk = [0u8; 4096];
                            let mut total = 0u64;
                            loop {
                                match stderr.read(&mut chunk).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => {
                                        total = total.saturating_add(n as u64);
                                        let keep = n.min(65536usize.saturating_sub(retained.len()));
                                        retained.extend_from_slice(&chunk[..keep]);
                                    }
                                }
                            }
                            (retained, total)
                        };
                        // Queue timeout/cancellation drops this joined future, Child and HandlerGroup together.
                        let (status, (retained, stderr_bytes)) = tokio::join!(child.wait(), drain);
                        let success = status.map_err(|_| "Job process failed")?.success();
                        if !success {
                            let mut trace = Value::Null;
                            let mut backend_failure = Value::Null;
                            for bytes in retained.split(|b| *b == b'\n')
                                .filter_map(|line|line.strip_prefix(b"JOB_TRACE "))
                                .take(16) {
                                let Ok(raw) = serde_json::from_slice::<Value>(bytes) else {
                                    continue;
                                };
                                if raw["schemaVersion"] != 1
                                    || raw["event"] != "job-trace"
                                    || raw["jobId"].as_str() != Some(job_id.as_str())
                                {
                                    continue;
                                }
                                let mut remaining = 24usize;
                                let group = clean_group(&raw, &trace_modules, &mut remaining);
                                let mut causes = Vec::new();
                                if let Some(items) = raw["causes"].as_array() {
                                    for (index, cause) in items.iter().take(6).enumerate() {
                                        let mut group = clean_group(cause, &trace_modules, &mut remaining);
                                        group["depth"] = json!(index + 1);
                                        causes.push(group);
                                    }
                                }
                                let mut truncated = serde_json::Map::new();
                                for key in [
                                    "frames",
                                    "causes",
                                    "stackUnavailable",
                                    "indexUnavailable",
                                    "output",
                                ] {
                                    truncated.insert(
                                        key.into(),
                                        json!(raw["truncated"][key].as_bool().unwrap_or(false)),
                                    );
                                }
                                truncated.insert(
                                    "frames".into(),
                                    json!(
                                        truncated["frames"] == true
                                            || group["truncated"]["frames"] == true
                                            || causes.iter().any(|c| c["truncated"]["frames"] == true)
                                    ),
                                );
                                truncated.insert(
                                    "indexUnavailable".into(),
                                    json!(truncated["indexUnavailable"] == true || trace_modules.is_empty()),
                                );
                                backend_failure = job_backend_failure(&raw["backend"]);
                                trace = json!({"schemaVersion":1,"location":group["location"],"frames":group["frames"],"causes":causes,"truncated":truncated});
                                break;
                            }
                            brain_backend::log_line!(
                                "{}",
                                json!({"schemaVersion":1,"event":"job-failed","jobId":job_id,"queue":queue_name,"backend":backend_failure,
                            "trace":trace,"stderrBytes":stderr_bytes,"stderrTruncated":stderr_bytes>65536})
                            );
                            Err("Job handler failed".into())
                        } else {
                            // A succeeded job whose handler caught backend failures: name them, or a swallowed 422
                            // never shows up anywhere (the worker does not echo application stderr).
                            let caught = retained.split(|b| *b == b'\n')
                                .filter_map(|line| line.strip_prefix(b"JOB_BACKEND "))
                                .take(16)
                                .filter_map(|bytes| serde_json::from_slice::<Value>(bytes).ok())
                                .filter(|raw| raw["schemaVersion"] == 1 && raw["event"] == "job-backend-failures" && raw["jobId"].as_str() == Some(job_id.as_str()))
                                .last()
                                .and_then(|raw| job_caught_failures(&raw));
                            if let Some(caught) = caught {
                                brain_backend::log_line!(
                                    "{}",
                                    json!({"schemaVersion":1,"event":"job-backend-failures","jobId":job_id,"queue":queue_name,"count":caught["count"],"failures":caught["failures"]})
                                );
                            }
                            Ok(())
                        }
                    }
                });
                tokio::pin!(worker);
                tokio::select! {
                    r=&mut worker=>r?,
                    _=brain_backend::http::wait_shutdown()=>{let _=send.send(true);worker.await?;},
                    _=owner_gone()=>{let _=send.send(true);worker.await?;},
                }
            }
            _ => unreachable!(),
        }
        pool.close().await;
        Ok(())
    })
}

/// Small default discovery response; request one resource/job for its complete contract.
pub fn card(root: &Path, name: Option<&str>) -> Result<Value, String> {
    let snapshot = inspect(root)?;
    if let Some(name) = name {
        let (only, name) = if let Some(n) = name.strip_prefix("resource:") {
            (Some("resources"), n)
        } else if let Some(n) = name.strip_prefix("job:") {
            (Some("jobs"), n)
        } else {
            (None, name)
        };
        for kind in ["resources", "jobs"] {
            if only.is_some_and(|k| k != kind) {
                continue;
            }
            if let Some(c) = snapshot[kind]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"].as_str() == Some(name))
            {
                let generated = format!("server/resources/{name}/client-contract.json");
                let generated_contract = if kind == "resources"
                    && fs::symlink_metadata(root.join(&generated)).is_ok_and(|m| {
                        m.is_file() && !m.file_type().is_symlink() && m.len() <= 32768
                    }) {
                    Some(generated)
                } else {
                    None
                };
                return Ok(
                    json!({"schemaVersion":1,"kind":kind,"contract":c,"generatedContract":generated_contract,"responses":if kind=="resources"{json!({"row":{"id":"uuid","owner?":"uuid","data":"the schema's fields each reader may see","created_at":"timestamp","updated_at":"timestamp"},"list":{"items":"[row]","limit":"int","offset":"int"},"read":"row","create":"row","update":"row","delete":{"ok":"bool"},"owner":"present only for the owner, relational owners, roles named in the read policy, or everyone with expose_owner: a public list omits it while create returns it to its owner"})}else{json!(null)},"files":if kind=="resources"{vec![format!("server/resources/{name}/schema.jsbrain"),format!("server/resources/{name}/policy.jsbrain"),format!("server/resources/{name}/hooks.jsbrain")]}else{vec![format!("server/jobs/{name}/schema.jsbrain"),format!("server/jobs/{name}/handler.jsbrain")]}}),
                );
            }
        }
        return Err("Unknown backend contract; omit name to list resources/jobs".into());
    }
    let names = |kind: &str| {
        snapshot[kind]
            .as_array()
            .unwrap()
            .iter()
            .take(100)
            .map(|c| c["name"].clone())
            .collect::<Vec<_>>()
    };
    Ok(
        json!({"schemaVersion":1,"experimental":true,"resources":names("resources"),"jobs":names("jobs"),"next":"Request one name for its schema, policy, hooks and owned files. CLI: brain backend schema"}),
    )
}

// Kill only the owned process group on cancellation/timeout, including ordinary child processes of a handler.
// A deliberately detached child is outside this trusted-project runner's containment contract.
struct HandlerGroup(Option<u32>);
impl Drop for HandlerGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn google_origin_setup_preserves_native_trust_and_names_local_provider_entries() {
        let origins = vec!["http://localhost:5280".to_string(), "https://example.test".into()];
        let setup = super::google_origin_setup(&origins);
        assert_eq!(setup["authorizedJavaScriptOrigins"], serde_json::json!(["http://localhost", "http://localhost:5280", "https://example.test"]));
        assert_eq!(setup["providerVerified"], false);
        assert_eq!(origins.len(), 2, "provider-only localhost entry does not widen native HTTP trust");
        let selected = super::google_origin_setup(&["https://override.example.test".into()]);
        assert_eq!(selected["authorizedJavaScriptOrigins"], serde_json::json!(["https://override.example.test"]));
    }

    #[cfg(unix)]
    #[test]
    fn stopping_a_dev_startup_child_reaps_its_process() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id() as libc::pid_t;
        super::stop_owned_child(&mut child);
        let mut status = 0;
        // A second wait cannot reap it: the owner already consumed the exit status.
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ECHILD));
    }

    #[test]
    fn a_succeeded_job_names_the_backend_failures_it_caught_and_never_a_value() {
        let raw = json!({"count":3,"failures":[
            {"status":422,"code":null,"target":"subscriptions","fields":["status[gte]","name with spaces"],"count":2,"message":"dropped"},
            {"status":503,"code":"DATABASE_BUSY","target":"files.reports","fields":[],"count":9},
            {"status":200,"target":"orders"},
            {"status":404,"code":"lower","target":"../auth","fields":["id"]}
        ]});
        let kept = super::job_caught_failures(&raw).unwrap();
        assert_eq!(kept["count"], json!(3));
        assert_eq!(kept["failures"], json!([
            {"status":422,"code":null,"resource":"subscriptions","fields":["status[gte]"],"count":2},
            {"status":503,"code":"DATABASE_BUSY","resource":"files.reports","fields":[],"count":3},
            {"status":404,"code":null,"resource":null,"fields":["id"],"count":1}
        ]));
        assert!(super::job_caught_failures(&json!({"count":0,"failures":[]})).is_none());
        assert!(super::job_caught_failures(&json!({"count":2})).is_none());
    }
    #[test]
    fn a_job_backend_failure_keeps_only_its_validated_shape() {
        use serde_json::json;
        let kept = super::job_backend_failure(&json!({"status":422,"code":"BAD_INPUT","fields":{"limit":"limit must be 1..100","odd":7},"extra":"dropped"}));
        assert_eq!(kept, json!({"status":422,"code":"BAD_INPUT","fields":{"limit":"limit must be 1..100"}}));
        assert_eq!(super::job_backend_failure(&json!({"status":200})), json!(null));
        assert_eq!(super::job_backend_failure(&json!(null)), json!(null));
        assert_eq!(super::job_backend_failure(&json!({"status":503,"code":"lower case"}))["code"], json!(null));
    }

    #[test]
    fn enqueue_takes_the_payload_from_the_flag_or_one_positional_json_and_names_the_fix() {
        use serde_json::json;
        let job: brain_backend::Resource = serde_json::from_value(json!({"name":"mail","fields":{"to":{"kind":"str","required":true}},"policy":{"read":[],"create":[],"update":[],"delete":[]}})).unwrap();
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<String>>();
        let payload = |list: &[&str]| super::enqueue_payload(&args(list), "mail", &job);
        assert_eq!(payload(&["enqueue", "mail", "--payload", r#"{"to":"a@example.com"}"#]).unwrap(), json!({"to":"a@example.com"}));
        assert_eq!(payload(&["enqueue", "mail", r#"{"to":"a@example.com"}"#, "--key", "k1"]).unwrap(), json!({"to":"a@example.com"}));
        assert_eq!(payload(&["enqueue", "mail", "--env", "development", r#"{"to":"a@example.com"}"#]).unwrap(), json!({"to":"a@example.com"}));
        let missing = payload(&["enqueue", "mail"]).unwrap_err();
        assert!(missing.contains("INVALID_JOB_PAYLOAD") && missing.contains("; fix: pass the JSON with --payload"), "{missing}");
        let invalid = payload(&["enqueue", "mail", "{to:"]).unwrap_err();
        assert!(invalid.starts_with("Invalid payload JSON") && invalid.contains("fix: pass the JSON with --payload"), "{invalid}");
        let both = payload(&["enqueue", "mail", "{}", "--payload", "{}"]).unwrap_err();
        assert!(both.contains("both --payload and a positional"), "{both}");
        assert!(payload(&["enqueue", "mail", "{}", "{}"]).unwrap_err().contains("fix: pass the JSON with --payload"));
        let rejected = payload(&["enqueue", "mail", "--payload", "{}"]).unwrap_err();
        assert!(rejected.contains("INVALID_JOB_PAYLOAD") && !rejected.contains("fix:"), "{rejected}");
    }

    #[test]
    fn a_configuration_exit_is_the_one_a_worker_supervisor_does_not_restart() {
        assert!(!super::restart_worker(Some(i32::from(super::CONFIG_EXIT))));
        assert!(super::restart_worker(Some(1)) && super::restart_worker(Some(2)) && super::restart_worker(None));
    }
    #[test]
    fn unstated_environment_stops_what_looks_deployed() {
        let args = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        let remote = "postgres://u@db.internal/app";
        for (given, database, node_env, refused) in [
            (&["serve"][..], Some("postgres://u@127.0.0.1/app"), None, false),
            (&["serve"][..], Some("postgres://u@%2Ftmp/app"), Some("development"), false),
            (&["serve"][..], Some(remote), None, true),
            (&["serve"][..], Some("postgres://u@127.0.0.1/app"), Some("production"), true),
            (&["serve", "--env", "development"][..], Some(remote), None, false),
            (&["migrate", "--env", "production"][..], Some(remote), Some("production"), false),
        ] {
            let found = unstated_environment(&args(given), database, node_env);
            assert_eq!(found.is_some(), refused, "{given:?} {database:?} {node_env:?}");
            assert!(found.is_none_or(|text| text.starts_with("B0785") && text.contains("--env production") && !text.contains("db.internal")));
        }
    }
    fn roles_app(files: &[(&str, &str)]) -> (std::path::PathBuf, Vec<&'static str>) {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!("brain-roles-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("brain.json"), "{}").unwrap();
        for (path, source) in files {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), source).unwrap();
        }
        let codes = role_warnings(&root).iter().map(|w| w.0).collect();
        (root, codes)
    }
    const SCHEMA: &str = "<meta server>\nexport default { title: 'str!' }\n";
    fn policy(create: &str) -> String {
        format!("<meta server>\nexport default {{ read: ['owner'], create: {create}, update: ['owner'], delete: ['owner'] }}\n")
    }
    #[test]
    fn a_first_read_without_workers_waits_once_for_a_worker_started_beside_the_server() {
        let row = |workers: i64| json!({"queue":"mail","declared":true,"pending":0,"oldestWaitSeconds":0,"workers":workers});
        assert!(first_read_waits(&[row(0)], &[], 0, false, false), "the worker may be starting in this same second");
        assert!(!first_read_waits(&[row(1)], &[], 0, false, false));
        assert!(!first_read_waits(&[row(0)], &[], 0, true, false), "only one grace period");
        assert!(!first_read_waits(&[row(0)], &[], 0, false, true), "own workers already had theirs");
        // a deploy restarts the scheduler beside the server too: no B0767 before it had the same moment
        let daily = ["daily".to_string()];
        assert!(first_read_waits(&[], &daily, 0, false, false), "the scheduler may be starting in this same second");
        assert!(!first_read_waits(&[], &daily, 1, false, false));
        assert!(!first_read_waits(&[row(1)], &[], 0, false, false), "nothing to wait for");
        let finding = |code: &'static str| (code, String::new(), String::new(), String::new());
        assert!(waits_for_siblings(&[finding("B0767")]) && waits_for_siblings(&[finding("B0765")]));
        assert!(!waits_for_siblings(&[finding("B0768")]) && !waits_for_siblings(&[finding("B0766")]), "an undeclared queue or an unreachable database does not fix itself in 10 s");
    }

    #[test]
    fn liveness_findings_name_each_queue_that_nothing_runs() {
        let row = |queue: &str, pending: i64, workers: i64, declared: bool| json!({"queue":queue,"pending":pending,"oldestWaitSeconds":90,"workers":workers,"declared":declared});
        let codes = |rows: &[Value], schedules: &[String], schedulers: i64| liveness_findings(rows, schedules, schedulers).into_iter().map(|f| f.0).collect::<Vec<_>>();
        let stuck = liveness_findings(&[row("mail", 78, 0, true)], &[], 0);
        assert_eq!(stuck[0].0, "B0765");
        assert!(stuck[0].2.contains("`mail`") && stuck[0].2.contains("78 job(s)") && stuck[0].2.contains("90 s") && stuck[0].3.contains("brain jobs work mail"), "{stuck:?}");
        assert!(stuck[0].2.contains("reporting to this database") && stuck[0].2.contains("another process"), "the check reads the database, not the machine: {stuck:?}");
        assert_eq!(codes(&[row("idle", 0, 0, true)], &[], 0), ["B0765"], "a queue with nothing due still has no worker");
        assert_eq!(codes(&[row("old", 3, 0, false)], &[], 0), ["B0768"]);
        let daily = ["daily".to_string()];
        assert_eq!(codes(&[], &daily, 0), ["B0767"]);
        // Negative: a worker and a scheduler silence them.
        assert!(codes(&[row("mail", 78, 1, true)], &daily, 1).is_empty());
    }

    #[test]
    fn a_handler_importing_a_browser_module_is_found_before_it_starts() {
        let (root, _) = roles_app(&[
            ("server/lib/data.jsbrain", "import { resource } from 'brain'\nexport const rows = resource('/x')\n"),
            ("server/jobs/mail/handler.jsbrain", "<meta server>\nimport { rows } from '../../lib/data.jsbrain'\nexport default async function handle(ctx) { return rows }\n"),
        ]);
        let found = handler_bundle_problems(&root);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "server/jobs/mail/handler.jsbrain");
        assert!(found[0].1.contains("add `<meta server>` to server/lib/data.jsbrain"), "{}", found[0].1);
        std::fs::write(root.join("server/lib/data.jsbrain"), "<meta server>\nexport const rows = []\n").unwrap();
        assert!(handler_bundle_problems(&root).is_empty());
    }

    #[test]
    fn a_job_without_a_handler_is_reported() {
        let (root, _) = roles_app(&[("server/jobs/mail/schema.jsbrain", "<meta server>\nexport default { to: 'str!' }\n")]);
        assert_eq!(job_warnings(&root).into_iter().map(|w| w.0).collect::<Vec<_>>(), ["B0769"]);
        std::fs::write(root.join("server/jobs/mail/handler.jsbrain"), "<meta server>\nexport default async function handle(ctx) {}\n").unwrap();
        assert!(job_warnings(&root).is_empty());
    }

    #[test]
    fn a_new_app_has_general_roles_and_signup_is_granted_by_user() {
        let p = policy("['user']");
        let (root, warnings) = roles_app(&[("server/resources/posts/schema.jsbrain", SCHEMA), ("server/resources/posts/policy.jsbrain", &p)]);
        let roles = auth_roles(&root).unwrap();
        assert_eq!((roles.roles.join(","), roles.signup_role.as_str()), ("admin,user".to_string(), "user"));
        assert!(inspect(&root).is_ok());
        assert!(warnings.is_empty(), "{warnings:?}");
    }
    #[test]
    fn a_published_app_without_a_roles_file_keeps_its_implicit_roles() {
        for (role, _) in [("candidate", ()), ("company", ())] {
            let p = policy(&format!("['{role}']"));
            let (root, warnings) = roles_app(&[("server/resources/posts/schema.jsbrain", SCHEMA), ("server/resources/posts/policy.jsbrain", &p)]);
            assert!(inspect(&root).is_ok(), "{role} must keep compiling");
            assert_eq!(auth_roles(&root).unwrap().signup_role, brain_backend::auth_flows::LEGACY_IMPLICIT_SIGNUP_ROLE);
            assert!(warnings.contains(&"B0739"), "{warnings:?}");
            assert_eq!(add_roles(&root).unwrap(), "server/auth/roles.jsbrain");
            let declared = auth_roles(&root).unwrap();
            assert_eq!(declared.signup_role, brain_backend::auth_flows::LEGACY_IMPLICIT_SIGNUP_ROLE, "declaring changes nothing");
            assert!(!role_warnings(&root).iter().any(|w| w.0 == "B0739"));
            assert!(add_roles(&root).is_err(), "never overwrites");
        }
    }
    #[test]
    fn roles_files_may_omit_or_keep_any_name() {
        let p = policy("['member']");
        // omits the old names
        let (root, _) = roles_app(&[("server/auth/roles.jsbrain", "<meta server>\nexport default { roles: ['admin', 'member'], signup_role: 'member' }\n"), ("server/resources/posts/schema.jsbrain", SCHEMA), ("server/resources/posts/policy.jsbrain", &p)]);
        assert!(inspect(&root).is_ok());
        // declares them only to compile, signup elsewhere
        let (root, _) = roles_app(&[("server/auth/roles.jsbrain", "<meta server>\nexport default { roles: ['admin', 'company', 'candidate', 'user'], signup_role: 'user' }\n")]);
        assert!(inspect(&root).is_ok());
        for bad in ["roles: ['member'], signup_role: 'member'", "roles: ['admin', 'user'], signup_role: 'admin'", "roles: ['admin', 'user'], signup_role: 'guest'"] {
            let source = format!("<meta server>\nexport default {{ {bad} }}\n");
            let (root, _) = roles_app(&[("server/auth/roles.jsbrain", &source)]);
            let error = auth_roles(&root).err().unwrap();
            assert!(error.starts_with("server/auth/roles.jsbrain: ") && error.contains("; fix: "), "{error}");
        }
    }
    #[test]
    fn undeclared_grants_are_actionable_and_a_silent_signup_role_warns() {
        let p = policy("['editor']");
        let (root, warnings) = roles_app(&[("server/resources/posts/schema.jsbrain", SCHEMA), ("server/resources/posts/policy.jsbrain", &p)]);
        let error = inspect(&root).unwrap_err();
        assert!(error.starts_with("server/resources/posts/policy.jsbrain: role `editor`") && error.contains("brain add roles"), "{error}");
        assert!(warnings.is_empty(), "the error owns it: {warnings:?}");
        // The signup role reaches nothing while grants name another role.
        let (root, warnings) = roles_app(&[("server/auth/roles.jsbrain", "<meta server>\nexport default { roles: ['admin', 'editor', 'user'], signup_role: 'user' }\n"), ("server/resources/posts/schema.jsbrain", SCHEMA), ("server/resources/posts/policy.jsbrain", &p)]);
        assert!(inspect(&root).is_ok());
        assert_eq!(warnings, vec!["B0738"]);
        // Field visibility compiled before: it warns instead of failing.
        let open = policy("['user']");
        let (root, warnings) = roles_app(&[("server/resources/posts/schema.jsbrain", "<meta server>\nexport default { title: { type: 'str', visible: ['ghost', 'owner'] } }\n"), ("server/resources/posts/policy.jsbrain", &open)]);
        assert!(inspect(&root).is_ok());
        assert_eq!(warnings, vec!["B0738"]);
    }

    #[test]
    fn create_owner_policy_error_lists_the_projects_actual_roles_and_reference_alternatives() {
        let source=policy("['owner']");
        let (root,_)=roles_app(&[
            ("server/auth/roles.jsbrain","<meta server>\nexport default {roles:['admin','member'],signup_role:'member'}\n"),
            ("server/resources/items/schema.jsbrain","<meta server>\nexport default { title:'str!', parent:'ref:parents!' }\n"),
            ("server/resources/items/policy.jsbrain",&source),
        ]);
        let error=contracts(&root,"resources").unwrap_err();
        assert!(error.contains("; fix:") && error.contains("owner.parent"),"{error}");
        assert!(error.contains("declared roles: admin, member") && !error.contains("declared roles: admin, user"),"{error}");
        assert_eq!(fs::read_to_string(root.join("server/resources/items/policy.jsbrain")).unwrap(),source);
        fs::remove_file(root.join("server/auth/roles.jsbrain")).unwrap();
        let error=contracts(&root,"resources").unwrap_err();
        assert!(error.contains("; fix:") && error.contains("owner.parent"),"{error}");
        assert!(!error.contains("declared roles:"),"implicit legacy roles are not explicit declarations: {error}");
        assert_eq!(fs::read_to_string(root.join("server/resources/items/policy.jsbrain")).unwrap(),source);
        fs::remove_dir_all(root).unwrap();
    }
    use super::*;
    #[test]
    fn brain_json_analytics_switch_defaults_on_and_validates() {
        let root = std::env::temp_dir().join(format!("brain-analytics-switch-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let with = |v: &str| {
            fs::write(root.join("brain.json"), format!("{{\"name\":\"shop\"{v}}}")).unwrap();
            analytics_settings(&root)
        };
        assert_eq!(with("").unwrap().unwrap().retention_days, 180);
        assert!(with(",\"analytics\":false").unwrap().is_none());
        assert!(with(",\"analytics\":{\"enabled\":false}").unwrap().is_none());
        let c = with(",\"analytics\":{\"consent\":\"required\",\"retentionDays\":30,\"countryHeader\":\"cf-ipcountry\"}").unwrap().unwrap();
        assert!(c.consent_required && c.retention_days == 30 && c.country_header.as_deref() == Some("cf-ipcountry"));
        assert!(with(",\"analytics\":{\"retentionDays\":0}").is_err());
        assert!(with(",\"analytics\":{\"track\":true}").is_err());
        assert!(with(",\"analytics\":\"yes\"").is_err());
        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn generated_backend_contracts_check_compile_and_preserve_existing_files() {
        let root =
            std::env::temp_dir().join(format!("brain-backend-scaffold-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        crate::scaffold::new_app(&root).unwrap();
        add(&root, "resource", "posts", "title:str! body:str").unwrap();
        add(
            &root,
            "job",
            "post_created",
            "resource:str! operation:str! record:obj!",
        )
        .unwrap();
        fs::write(
            root.join("server/resources/posts/hooks.jsbrain"),
            "<meta server>\nexport default {after_create:'post_created'}\n",
        )
        .unwrap();
        let snapshot = inspect(&root).unwrap();
        assert_eq!(
            snapshot["resources"][0]["fields"]["title"]["required"],
            true
        );
        assert_eq!(
            snapshot["resources"][0]["policy"]["create"],
            json!(["admin"])
        );
        assert!(add(&root, "resource", "posts", "title:bool").is_err());
        let (_, diags) = crate::run_check(Some(root.clone())).unwrap();
        assert!(
            !diags.iter().any(|d| !d.is_warning()),
            "{}",
            crate::diags_json(&diags)
        );
        let (scratch, path) = worker_module(&root, "post_created").unwrap();
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .contains("Implement this job handler")
        );
        drop(scratch);
        fs::write(
            root.join("server/resources/posts/hooks.jsbrain"),
            "<meta server>\nexport default {after_create:'missing_job'}\n",
        )
        .unwrap();
        let (_, diags) = crate::run_check(Some(root.clone())).unwrap();
        assert!(diags.iter().any(|d| d.code == "B0735"));
        fs::remove_dir_all(root).unwrap();
    }
    fn fixture_root(label: &str) -> Scratch {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Scratch(
            std::env::temp_dir().join(format!("brain-{label}-{}-{stamp}", std::process::id())),
        );
        fs::create_dir_all(&root.0).unwrap();
        root
    }
    /// The generated service reads every field shape `fields()` accepts: `'str!'`, `'str! private'` and the object
    /// form `{ type: 'str!', ... }`; it used to call `schema[key].endsWith('!')` and threw on an object.
    #[test]
    fn generated_service_reads_object_form_and_modifier_fields() {
        let root = fixture_root("resource-object-fields");
        crate::scaffold::new_app(&root.0).unwrap();
        add(&root.0, "resource", "notes", "title:str! body:str").unwrap();
        let service = fs::read_to_string(root.0.join("server/services/notes.jsbrain")).unwrap();
        let body: String = service
            .lines()
            .filter(|l| !l.starts_with("<meta server>") && !l.starts_with("import "))
            .collect::<Vec<_>>()
            .join("\n")
            .replace("export default {", "const service = {");
        let harness = format!(
            r#"const schema = {{ title: {{ type: 'str!', unique: true }}, body: {{ type: 'str' }}, count: {{ type: 'num' }}, secret: 'str! private', flag: 'bool' }}
const saved = []
function repository() {{ return {{ create: async (p) => (saved.push(p), {{ id: 'n1', data: p }}), update: async (id, p) => ({{ id, data: p }}), get: async () => ({{ data: {{ title: 'Hello', count: 2 }} }}) }} }}
{body}
const ctx = {{ backend: {{}}, user: {{ role: 'admin' }} }}
const assert = (ok, what) => {{ if (!ok) throw new Error(what) }}
let missing
try {{ await service.save(ctx, null, {{ body: 'x', secret: 's' }}) }} catch (e) {{ missing = e }}
assert(missing && missing.errors && missing.errors.title === 'Required', 'object-form required field is enforced')
let modifier
try {{ await service.save(ctx, null, {{ title: 't' }}) }} catch (e) {{ modifier = e }}
assert(modifier && modifier.errors && modifier.errors.secret === 'Required', 'a field with an access modifier keeps its !')
const form = await service.editor(ctx, 'n1')
assert(form.title === 'Hello' && form.count === 2 && form.body === '' && form.flag === false, 'editor reads object-form kinds: ' + JSON.stringify(form))
await service.saveForm(ctx, null, {{ title: 'T', body: '', count: '3', secret: 's', flag: true }})
assert(saved.at(-1).count === 3, 'saveForm converts an object-form num field')
let blank
try {{ await service.saveForm(ctx, null, {{ title: '  ', secret: 's' }}) }} catch (e) {{ blank = e }}
assert(blank && blank.errors && blank.errors.title, 'a blank required object-form str is refused')
console.log('object-form service: PASS')
"#
        );
        let output = std::process::Command::new("node")
            .args(["--input-type=module", "-e", &harness])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success() && stdout.contains("object-form service: PASS"), "{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    }
    #[test]
    fn resource_support_preserves_the_default_budget_without_adding_admin_routes() {
        let parent = fixture_root("resource-default-budget");
        let root = parent.0.join("app");
        crate::scaffold::new_app(&root).unwrap();
        for (name, fields) in [
            ("posts", "title:str! body:str"),
            ("metrics", "title:str! active:bool! score:num meta:obj"),
        ] {
            let result = add(&root, "resource", name, fields).unwrap();
            assert!(
                result["files"].as_array().unwrap().iter().all(|path| !path
                    .as_str()
                    .unwrap()
                    .starts_with("views/")
                    && !path.as_str().unwrap().starts_with("ui/"))
            );
            let snapshot: Value = serde_json::from_slice(
                &fs::read(root.join(format!("server/resources/{name}/client-contract.json")))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(snapshot["adminUi"]["status"], "planned");
            assert!(snapshot.get("views").is_none() && snapshot.get("guard").is_none());
            for client in snapshot["client"].as_array().unwrap() {
                assert!(root.join(client.as_str().unwrap()).is_file());
            }
            assert!(
                root.join(snapshot["repository"].as_str().unwrap())
                    .is_file()
            );
            assert!(root.join(snapshot["service"].as_str().unwrap()).is_file());
            let config: Value =
                serde_json::from_slice(&fs::read(root.join("brain.json")).unwrap()).unwrap();
            assert_eq!(config["budget"]["viewKB"], 30);
            crate::run_build(&root, Some("https://example.test".into())).unwrap();
        }
        assert!(!root.join("views/admin").exists());
    }
    #[test]
    fn generated_admin_ui_builds_real_routes_and_rpc_without_overwrites() {
        use std::io::Read;
        let root = fixture_root("generated-crud");
        // Freeze the old rendered fixture identity before measurement: same 45-byte
        // pattern (four-digit process id + nineteen-digit nanosecond timestamp).
        // Its digit sample was captured once, not selected by a passing budget.
        const BRAND: &str = "brain-generated-crud-2580-1793395249408887000";
        assert_eq!(BRAND.len(), 45);
        crate::scaffold::new_app_named(&root.0, BRAND).unwrap();
        let original_control = fs::read(root.0.join("ui/button.brain")).unwrap();
        let added = add_with_admin(
            &root.0,
            "resource",
            "posts",
            "title:str! active:bool! score:num meta:obj",
            true,
        )
        .unwrap();
        assert_eq!(added["adminUi"]["status"], "generated");
        assert_eq!(
            fs::read(root.0.join("ui/button.brain")).unwrap(),
            original_control
        );
        let card = card(&root.0, Some("resource:posts")).unwrap();
        assert_eq!(
            card["generatedContract"],
            "server/resources/posts/client-contract.json"
        );
        let manifest: Value = serde_json::from_slice(
            &fs::read(root.0.join("server/resources/posts/client-contract.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["fields"]["active"]["required"], true);
        assert_eq!(
            manifest["schemaRevision"],
            crate::manifests::hash(
                &fs::read(root.0.join("server/resources/posts/schema.jsbrain")).unwrap()
            )
        );
        assert_eq!(manifest["adminUi"]["status"], "generated");
        assert_eq!(manifest["adminUi"]["routes"].as_array().unwrap().len(), 4);
        for path in ["index.brain", "[id].brain", "new.brain", "[id]/edit.brain"] {
            let source = fs::read_to_string(root.0.join("views/admin/posts").join(path)).unwrap();
            assert!(
                !source.contains("api(") && !source.contains("resource("),
                "Admin requests must use the server service: {path}"
            );
            assert!(
                !source.contains("from '../../../api/")
                    && !source.contains("from '../../../../api/")
            );
            if path == "index.brain" {
                assert!(source.contains("method=\"get\"") && !source.contains("on:submit="));
            }
        }
        let saved = fs::read(root.0.join("views/admin/posts/_guard.jsbrain")).unwrap();
        assert!(add(&root.0, "resource", "posts", "title:bool").is_err());
        assert_eq!(
            fs::read(root.0.join("views/admin/posts/_guard.jsbrain")).unwrap(),
            saved
        );
        let config: Value = serde_json::from_slice(&fs::read(root.0.join("brain.json")).unwrap()).unwrap();
        assert_eq!(config["budget"]["viewKB"], 30);
        let report = crate::run_build(&root.0, Some("https://example.test".into())).unwrap();
        eprintln!("CRUD default-budget fixture bytes by route: {:?}", report.report);
        // one root _loading.brain is one pattern, and an empty fullLoad is not emitted: both keep every route's HTML small
        let page = fs::read_dir(root.0.join("dist/_server/pages")).unwrap().map(|f| fs::read_to_string(f.unwrap().path()).unwrap()).find(|html| html.contains("router({")).unwrap();
        assert!(page.contains("loading:[{rx:\"^/\",view:") && !page.contains("fullLoad:[]"), "{page}");
        assert!(page.contains("href=\"#brain-view\">Skip to content</a>") && page.contains("id=\"brain-view\""), "the scaffold's skip link targets the region: {page}");
        for entry in &report.ledger {
            let mut entry: Value = serde_json::from_str(&entry.stringify()).unwrap();
            let route = entry["route"].as_str().unwrap_or("");
            if route.starts_with("/admin/posts") {
                assert!(
                    entry["assets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|asset| asset["source"] != "runtime/brain-data.js"),
                    "Server-owned admin reads must not import the generic client API runtime"
                );
            }
            if route.starts_with("/admin/posts") || route == "/login" {
                if let Some(assets) = entry["assets"].as_array_mut() {
                    assets.sort_by_key(|asset| {
                        std::cmp::Reverse(asset["bytes"]["gzip"].as_u64().unwrap_or(0))
                    });
                    assets.truncate(8);
                }
                eprintln!("CRUD emitted ledger (largest eight assets): {entry}");
            }
        }
        let mut command = std::process::Command::new("node");
        command
            .args([
                "--input-type=module",
                "-e",
                include_str!("../tests/generated-resource-rpc.mjs"),
            ])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("BRAIN_CRUD_DIST", root.0.join("dist"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().unwrap();
        let mut group = HandlerGroup(Some(child.id()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                drop(group);
                let _ = child.kill();
                let _ = child.wait();
                panic!("Generated CRUD fixture exceeded its owned deadline")
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        group.0 = None;
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .take(16385)
            .read_to_string(&mut output)
            .unwrap();
        let mut error = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .take(16385)
            .read_to_string(&mut error)
            .unwrap();
        assert!(output.len() <= 16384 && error.len() <= 16384);
        assert!(status.success(), "{output}\n{error}");
        assert!(output.contains("Generated admin CRUD: PASS"));
    }
    #[test]
    fn resource_support_conflicts_roll_back_and_preserve_unrelated_admin_source() {
        let root = fixture_root("crud-owned-guard");
        crate::scaffold::new_app(&root.0).unwrap();
        fs::create_dir_all(root.0.join("views/admin/posts")).unwrap();
        let guard = root.0.join("views/admin/posts/_guard.jsbrain");
        fs::write(&guard, "export default () => false\n").unwrap();
        fs::create_dir_all(root.0.join("api")).unwrap();
        fs::write(root.0.join("api/posts.jsbrain"), "export default {}\n").unwrap();
        assert!(add(&root.0, "resource", "posts", "title:str!").is_err());
        assert_eq!(
            fs::read_to_string(&guard).unwrap(),
            "export default () => false\n"
        );
        assert!(
            !root
                .0
                .join("server/resources/posts/schema.jsbrain")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(root.0.join("api/posts.jsbrain")).unwrap(),
            "export default {}\n"
        );
        fs::remove_file(root.0.join("api/posts.jsbrain")).unwrap();
        add(&root.0, "resource", "posts", "title:str!").unwrap();
        assert_eq!(
            fs::read_to_string(&guard).unwrap(),
            "export default () => false\n"
        );
    }
    #[test]
    fn resource_contract_reader_rejects_nonregular_symlink_and_oversize_sources() {
        let root = fixture_root("contract-bounds");
        let source = "export default {title:'str!'}";
        let file = root.0.join("schema.jsbrain");
        fs::write(
            &file,
            format!("{source}{}", " ".repeat(65536 - source.len())),
        )
        .unwrap();
        assert!(read_contract(&root.0, &file).is_ok());
        fs::write(&file, "x".repeat(65537)).unwrap();
        assert!(
            read_contract(&root.0, &file)
                .unwrap_err()
                .contains("64 KiB")
        );
        #[cfg(unix)]
        {
            use std::{
                ffi::CString,
                os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
            };
            fs::remove_file(&file).unwrap();
            let fifo = CString::new(file.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            // A bounded writer releases an accidentally blocking old reader, making a regression fail rather than hang.
            let writer_path = file.clone();
            let writer = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if let Ok(mut f) = fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(writer_path)
                {
                    use std::io::Write;
                    let _ = f.write_all(b"export default {title:'str!'}");
                }
            });
            assert!(
                read_contract(&root.0, &file)
                    .unwrap_err()
                    .contains("nonregular")
            );
            writer.join().unwrap();
            fs::remove_file(&file).unwrap();
            fs::write(root.0.join("actual.jsbrain"), source).unwrap();
            std::os::unix::fs::symlink(root.0.join("actual.jsbrain"), &file).unwrap();
            assert!(read_contract(&root.0, &file).is_err());
            fs::create_dir_all(root.0.join("actual")).unwrap();
            fs::write(root.0.join("actual/schema.jsbrain"), source).unwrap();
            std::os::unix::fs::symlink(root.0.join("actual"), root.0.join("linked")).unwrap();
            assert!(read_contract(&root.0, &root.0.join("linked/schema.jsbrain")).is_err());
        }
    }
    #[test]
    fn field_access_modifiers_parse_in_string_and_object_forms() {
        let parsed = fields(json!({"title":"str!","secret":"str writeonly","state":"enum(draft|live)! readonly","score":{"type":"int","access":"private"},"price":"decimal(10,2)"})).unwrap();
        assert_eq!((parsed["secret"].kind.as_str(), parsed["secret"].access_mode()), ("str", Some("writeonly")));
        assert_eq!((parsed["state"].kind.as_str(), parsed["state"].required, parsed["state"].access_mode()), ("enum(draft|live)", true, Some("readonly")));
        assert_eq!(parsed["score"].access_mode(), Some("private"));
        assert_eq!(parsed["title"].access, None);
        assert_eq!(parsed["price"].access, None);
        assert!(fields(json!({"x":{"type":"str","access":1}})).is_err());
    }
    #[test]
    fn exact_type_contracts_read_object_fields_numbers_and_constraints() {
        let root = fixture_root("exact-types");
        let dir = root.0.join("server/resources/listings");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("schema.jsbrain"), "<meta server>\nexport default {\n  title: 'str!',\n  status: { type: 'enum(draft|published)!', default: 'draft' },\n  slug: { type: 'str!', unique: true },\n  seats: { type: 'int', default: 1 },\n  price_min: 'decimal(10,2)',\n  price_max: 'decimal(10,2)',\n  currency: 'enum(MXN|USD)',\n  ends_on: 'date',\n  tags: 'list<str,10>'\n}\n").unwrap();
        fs::write(dir.join("policy.jsbrain"), "<meta server>\nexport default { read: ['admin'], create: ['admin'], update: ['admin'], delete: ['admin'] }\n").unwrap();
        fs::write(dir.join("constraints.jsbrain"), "<meta server>\nexport default {\n  unique: [['title', 'ends_on']],\n  check: [\n    { field: 'seats', min: 1, max: 500 },\n    { compare: ['price_min', '<=', 'price_max'] },\n    { together: ['price_min', 'price_max', 'currency'] }\n  ]\n}\n").unwrap();
        let resources = contracts(&root.0, "resources").unwrap();
        let r = &resources[0];
        assert_eq!(r.fields["status"].default, Some(json!("draft")));
        assert!(r.fields["slug"].unique && r.fields["slug"].required);
        assert_eq!(r.fields["seats"].default, Some(json!(1)));
        assert_eq!(r.constraints.check.len(), 3);
        assert_eq!(r.unique_sets().len(), 2);
        let saved = r.input(&json!({"title":"Widget","slug":"widget"})).unwrap();
        assert_eq!(saved, json!({"title":"Widget","slug":"widget","status":"draft","seats":1}));
        assert!(r.input(&json!({"title":"A","slug":"a","price_min":"10"})).unwrap_err().contains_key("price_max"));
        fs::write(dir.join("schema.jsbrain"), "export default { price: { type: 'int', default: 99999999999999999999 } }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("64-bit integer range"));
        fs::write(dir.join("schema.jsbrain"), "export default { price: { type: 'int', unknown: true } }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("unknown option"));
        fs::write(dir.join("schema.jsbrain"), "export default { price: 'money' }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("decimal(p,s)"));
        fs::write(dir.join("schema.jsbrain"), "export default { a: 'int', b: 'int' }\n").unwrap();
        fs::write(dir.join("constraints.jsbrain"), "export default { check: [{ field: 'a', min: 1, max: -2 }] }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("min greater than max"));
        fs::write(dir.join("constraints.jsbrain"), "export default { checks: [] }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("constraints.jsbrain must export"));
    }
    #[test]
    fn contracts_do_not_evaluate_application_code() {
        let root =
            std::env::temp_dir().join(format!("brain-backend-literal-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("schema.jsbrain");
        fs::write(&path, "export default fetch('/side-effect')\n").unwrap();
        assert!(read_contract(&root, &path).is_err());
        fs::write(&path, "export default { title:'str!', title:'bool' }\n").unwrap();
        assert!(read_contract(&root, &path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

/// Native media operations. Provisioning returns a guarded file plan; cloud setup never executes implicitly.
pub fn media(args: &[String]) -> Result<(), String> {
    let root = crate::app_root()?;
    let operation = args.first().map(String::as_str).unwrap_or("help");
    if operation == "setup-gcs" {
        let mut operations = Vec::new();
        let mut preserved = Vec::new();
        for (path, content) in brain_backend::media::gcs_files() {
            let current = crate::changes::read(&root, &path)?;
            if current["revision"].is_null() {
                operations.push(json!({"path":path,"expectedRevision":null,"content":content}));
            } else {
                preserved.push(json!({"path":path,"revision":current["revision"]}));
            }
        }
        println!(
            "{}",
            json!({"schemaVersion":1,"dryRun":true,"operations":operations,"preserved":preserved,"installed":false,"cloudContacted":false,"environmentKeys":["BRAIN_GCS_BUCKET","BRAIN_GCS_PREFIX","BRAIN_GCS_ADAPTER"],"next":"Apply reviewed operations through brain changes, install the private pinned SDK, and configure ADC outside public files"})
        );
        return Ok(());
    }
    if !matches!(operation, "list" | "cleanup") {
        return Err("usage: brain media setup-gcs | list --owner <uuid> [--limit 50] [--offset 0] | cleanup [--limit 8]; cleanup processes only recorded owned deletion work".into());
    }
    let environment = crate::env::name(args, "development");
    let url = stated_database(args, &root, &environment)?;
    let limit = crate::flag(args, "--limit")
        .unwrap_or_else(|| {
            if operation == "list" {
                "50".into()
            } else {
                "8".into()
            }
        })
        .parse::<i64>()
        .map_err(|_| "media limit must be an integer")?;
    let max = if operation == "list" { 100 } else { 8 };
    if !(1..=max).contains(&limit) {
        return Err(format!("media limit must be 1..{max}"));
    }
    runtime()?.block_on(async {
        let pool = brain_backend::boot::connect(&url, &brain_backend::boot::Boot::command(environment == "production")).await?;
        let result = if operation == "list" {
            let owner = crate::flag(args, "--owner")
                .ok_or("media list requires an explicit owner UUID")?
                .parse()
                .map_err(|_| "owner must be a UUID")?;
            let offset = crate::flag(args, "--offset")
                .unwrap_or_else(|| "0".into())
                .parse::<i64>()
                .map_err(|_| "media offset must be 0..10000")?;
            if !(0..=10000).contains(&offset) {
                return Err("media offset must be 0..10000".into());
            }
            brain_backend::media::library(&pool, owner, limit, offset).await
        } else {
            let setting = |key: &str| {
                std::env::var(key)
                    .ok()
                    .filter(|v| !v.is_empty())
                    .or_else(|| crate::env::value(&root, &environment, key))
            };
            let gcs =
                setting("BRAIN_GCS_BUCKET")
                    .map(|bucket| {
                        brain_backend::media::GcsConfig::new(
                            bucket,
                            setting("BRAIN_GCS_PREFIX").unwrap_or_else(|| "brain-media".into()),
                            PathBuf::from(setting("BRAIN_GCS_ADAPTER").ok_or(
                                "BRAIN_GCS_ADAPTER is required for configured GCS cleanup",
                            )?),
                        )
                    })
                    .transpose()?;
            let store = brain_backend::media::Store::new(root.join(".brain/uploads"), gcs)?;
            brain_backend::media::cleanup(&pool, &store, limit).await
        };
        pool.close().await;
        println!("{}", result?);
        Ok(())
    })
}

#[cfg(test)]
mod failure_log_tests {
    #[test]
    fn accounts_with_an_undeclared_role_are_named() {
        let found = super::undeclared_roles(&[("admin".into(), 2), ("employer".into(), 3)], &["admin".into(), "user".into()]);
        assert_eq!(found.len(), 1);
        assert!(found[0].0 == "B0778" && found[0].2.contains("3 account(s)") && found[0].2.contains("`employer`"), "{:?}", found[0]);
    }
    #[test]
    fn a_backend_on_this_machine_excuses_a_process_without_a_database_only_while_it_answers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(super::answers(&format!("http://127.0.0.1:{port}/")));
        drop(listener);
        assert!(!super::answers(&format!("http://127.0.0.1:{port}")));
        assert!(!super::answers(""));
    }
    #[test]
    fn only_a_backend_on_another_host_excuses_a_process_without_a_database() {
        assert!(is_remote_backend("https://api.example.com"));
        assert!(is_remote_backend("http://10.0.0.5:8787"));
        for here in ["http://127.0.0.1:8787", "http://localhost:8787/", "http://[::1]:8787", "http://localhost", ""] {
            assert!(!is_remote_backend(here), "{here}");
        }
    }
    #[test]
    fn anonymous_catalog_warns_unless_the_environment_is_chosen_and_local() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(super::anonymous_catalog_warning(&args(&["serve"]), "development").is_some());
        assert!(super::anonymous_catalog_warning(&args(&["serve", "--env", "staging"]), "staging").is_some());
        assert!(super::anonymous_catalog_warning(&args(&["serve", "--env", "development"]), "development").is_none());
        assert!(super::anonymous_catalog_warning(&args(&["serve", "--env", "production"]), "production").is_none());
    }
    use super::*;
    #[test]
    fn failure_logs_find_a_request_and_tolerate_a_missing_or_damaged_file() {
        let root = std::env::temp_dir().join(format!("brain-failure-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(failure_logs(&root, Some("r1"))["events"], json!([]));
        assert!(failure_logs(&root, Some("r1"))["fix"].is_string());
        std::fs::create_dir_all(root.join(".brain/backend")).unwrap();
        std::fs::write(root.join(".brain/backend/failures.jsonl.1"), "{\"rid\":\"r1\",\"n\":1}\n").unwrap();
        std::fs::write(failure_log(&root), "not json\n{\"rid\":\"r2\",\"n\":2}\n{\"rid\":\"r1\",\"n\":3}\n").unwrap();
        let found = failure_logs(&root, Some("r1"));
        assert_eq!(found["events"], json!([{"rid":"r1","n":1},{"rid":"r1","n":3}]));
        assert!(found.get("fix").is_none());
        assert_eq!(failure_logs(&root, None)["events"].as_array().unwrap().len(), 3);
        std::fs::remove_dir_all(root).unwrap();
    }
}
