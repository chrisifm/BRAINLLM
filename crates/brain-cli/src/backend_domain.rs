//! Loads declarative domain contracts: `server/resources/<r>/{actions,transitions,batch}.jsbrain` and
//! `server/operations/<name>.jsbrain`. Literal `export default { ... }` files only; nothing is evaluated.
use brain_backend::{Resource, domain::Domain};
use serde_json::Value;
use std::{fs, path::Path};

const MAX_OPERATIONS: usize = 64;

fn parse<T: serde::de::DeserializeOwned>(root: &Path, path: &Path, shape: &str) -> Result<T, String> {
    let value: Value = super::read_contract(root, path)?;
    let relative = path.strip_prefix(root).unwrap_or(path).display().to_string();
    serde_json::from_value(value).map_err(|error| format!("{relative}: {error}; fix: export {shape}"))
}

/// Read every domain file of the app. Absent files yield an empty domain (unchanged behavior).
pub fn load(root: &Path, resources: &[Resource]) -> Result<Domain, String> {
    let mut domain = Domain::default();
    for r in resources {
        let dir = root.join("server/resources").join(&r.name);
        let mut entry = brain_backend::domain::ResourceDomain::default();
        let file = |name: &str| Some(dir.join(name)).filter(|p| p.is_file());
        if let Some(path) = file("actions.jsbrain") {
            entry.actions = parse(root, &path, "{ <action>: { roles, input?, when?, set?, increment?, decrement? } }")?;
        }
        if let Some(path) = file("transitions.jsbrain") {
            entry.transitions = Some(parse(root, &path, "{ field, initial: [..], transitions: { <name>: { from: [..], to, roles, input?, when?, set? } } }")?);
        }
        if let Some(path) = file("batch.jsbrain") {
            entry.batch = Some(parse(root, &path, "{ max: 1..1000 }")?);
        }
        if entry != Default::default() {
            domain.resources.insert(r.name.clone(), entry);
        }
    }
    let dir = root.join("server/operations");
    if dir.is_dir() {
        let mut files: Vec<_> = fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        files.sort_by_key(|e| e.file_name());
        for e in files {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".jsbrain") else { continue };
            if domain.operations.len() >= MAX_OPERATIONS {
                return Err(format!("server/operations: at most {MAX_OPERATIONS} operations"));
            }
            let op = parse(root, &e.path(), "{ roles, input?, steps: [{ as, create|read|update|upsert|delete|action|transition, id?, data?, key?, owned?, input? }], result? }")?;
            domain.operations.insert(stem.to_string(), op);
        }
    }
    Ok(domain)
}

/// `server/jobs/<name>/queue.jsbrain`: priority, attempts, backoff, timeout, lease, drain and concurrency. Absent → defaults.
pub fn queue_policy(root: &Path, name: &str) -> Result<brain_backend::jobs::QueuePolicy, String> {
    let path = root.join("server/jobs").join(name).join("queue.jsbrain");
    if !path.is_file() {
        return Ok(Default::default());
    }
    let policy: brain_backend::jobs::QueuePolicy = parse(root, &path, "{ priority?, attempts?, backoff?: { type: 'fixed'|'exponential', delay, max }, timeout?, lease?, drain?, concurrency? }")?;
    policy.validate().map_err(|e| format!("server/jobs/{name}/queue.jsbrain: {e}"))?;
    Ok(policy)
}

/// Load and validate against resources and configured roles; the same checks run again before serving.
pub fn checked(root: &Path, resources: &[Resource], roles: &brain_backend::auth_flows::RolesConfig) -> Result<Domain, String> {
    let domain = load(root, resources)?;
    domain.validate(resources)?;
    let declared = |role: &String| role == "owner" || roles.roles.contains(role);
    for (name, d) in &domain.resources {
        let grants = d.actions.values().flat_map(|a| a.roles.iter())
            .chain(d.transitions.iter().flat_map(|t| t.transitions.values().flat_map(|x| x.roles.iter())));
        for role in grants {
            if !declared(role) {
                return Err(format!("server/resources/{name}: role {role} is not declared in server/auth/roles.jsbrain"));
            }
        }
    }
    for (name, op) in &domain.operations {
        if let Some(role) = op.roles.iter().find(|r| !declared(r)) {
            return Err(format!("server/operations/{name}.jsbrain: role {role} is not declared in server/auth/roles.jsbrain"));
        }
    }
    Ok(domain)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    fn example() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/backend/examples/shop")
    }
    const FIXTURE: &str = "../brain-backend/tests/shop_contracts.json";

    /// The documented example loads into exactly the contracts the backend Postgres tests execute.
    #[test]
    fn documented_shop_example_is_the_executed_fixture() {
        let root = example().canonicalize().unwrap();
        let snapshot = super::super::inspect(&root).unwrap();
        let loaded = serde_json::json!({"resources": snapshot["resources"], "domain": snapshot["domain"]});
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        if std::env::var_os("BRAIN_UPDATE_FIXTURES").is_some() {
            std::fs::write(&path, serde_json::to_string_pretty(&loaded).unwrap() + "\n").unwrap();
        }
        let expected: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded, expected, "docs/backend/examples/shop changed: rerun with BRAIN_UPDATE_FIXTURES=1 and review");
        let paths = &snapshot["openapi"]["paths"];
        assert!(paths["/_brain/operations/place_order"]["post"].is_object());
        assert!(paths["/api/shop_orders/{id}/transitions/pay"]["post"].is_object());
        assert_eq!(paths["/api/task_items/batch"]["post"]["x-brain-batch-max"], 5);
    }

    /// `queue.jsbrain` is projected in `brain backend schema --json`; invalid values fail with the file name.
    #[test]
    fn queue_policies_are_projected_and_validated() {
        let root = example().canonicalize().unwrap();
        let snapshot = super::super::inspect(&root).unwrap();
        let receipts = &snapshot["queues"]["order_receipts"];
        assert_eq!((receipts["priority"].as_i64(), receipts["timeout"].as_i64(), receipts["backoff"]["type"].as_str()), (Some(10), Some(900), Some("exponential")));
        let dir = std::env::temp_dir().join(format!("brain-queue-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let job = dir.join("server/jobs/mail");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(job.join("schema.jsbrain"), "export default { to: 'str!' }\n").unwrap();
        let root = dir.canonicalize().unwrap();
        assert_eq!(super::super::inspect(&root).unwrap()["queues"]["mail"]["attempts"], 3, "absent file uses defaults");
        std::fs::write(job.join("queue.jsbrain"), "export default { lease: 5 }\n").unwrap();
        let error = super::super::inspect(&root).unwrap_err();
        assert!(error.contains("server/jobs/mail/queue.jsbrain") && error.contains("lease: 10..600"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_domain_files_fail_with_the_file_name() {
        let dir = std::env::temp_dir().join(format!("brain-domain-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let r = dir.join("server/resources/notes");
        std::fs::create_dir_all(&r).unwrap();
        std::fs::write(r.join("schema.jsbrain"), "export default { title: 'str!', votes: 'int' }\n").unwrap();
        std::fs::write(r.join("policy.jsbrain"), "export default { read: ['admin'], create: ['admin'], update: ['admin'], delete: ['admin'] }\n").unwrap();
        std::fs::write(r.join("actions.jsbrain"), "export default { vote: { roles: ['admin'], increment: { votes: 1 }, extra: true } }\n").unwrap();
        let root = dir.canonicalize().unwrap();
        let error = super::super::inspect(&root).unwrap_err();
        assert!(error.contains("server/resources/notes/actions.jsbrain") && error.contains("extra"), "{error}");
        std::fs::write(r.join("actions.jsbrain"), "export default { vote: { roles: ['editor'], increment: { votes: 1 } } }\n").unwrap();
        assert!(super::super::inspect(&root).unwrap_err().contains("role editor"));
        std::fs::write(r.join("actions.jsbrain"), "export default { vote: { roles: ['admin'], increment: { votes: 1 } } }\n").unwrap();
        assert!(super::super::inspect(&root).unwrap()["domain"]["resources"]["notes"]["actions"]["vote"].is_object());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
