//! Discover native capabilities and prepare pure project helpers without installing packages.
use serde_json::{Value, json};
use std::path::Path;

pub fn run(args: &[String]) -> Result<(), String> {
    let positional: Vec<&str> = args
        .iter()
        .skip(1)
        .filter(|s| s.as_str() != "--json")
        .map(String::as_str)
        .collect();
    let value = match positional.as_slice() {
        [] | ["list"] => list(),
        ["new", name] => new(&crate::app_root()?, name)?,
        _ => return Err("usage: brain lib list | brain lib new <name> [--json]".into()),
    };
    println!("{}", value);
    Ok(())
}

pub fn list() -> Value {
    json!({"schemaVersion":1,"libraries":[
        {"name":"http","functions":["api","http","resource"],"module":"brain-data.js","contract":"Typed requests, cancellation, retries and policy-aware data caching"},
        {"name":"validation","functions":["form"],"module":"brain-form.js","contract":"Shared literal schemas and field errors"},
        {"name":"state","functions":["store","bus","provide","use","key"],"module":"brain-ch.js","contract":"Scoped state and explicit dependency injection"},
        {"name":"date","functions":["dateParse","dateAdd","dateDiff","dateRange","dateZone"],"module":"brain-time.js","contract":"UTC calendar arithmetic, bounded ranges and explicit timezone projection; synchronous and pure"},
        {"name":"hash","functions":["hashText","shortHash"],"module":"brain-hash.js","contract":"Async Web Crypto SHA-256/384/512; bounded text input, explicit truncation"},
        {"name":"outbound","functions":["httpCall"],"module":"brain-outbound.js","contract":"Server-side calls to external APIs: per-attempt timeout, bounded jittered retries only for idempotent requests, Idempotency-Key, response size cap, private-address refusal checked on every resolved address and redirect hop, and redacted errors","guide":"docs/backend/outbound.md"},
        {"name":"storage","functions":["database","storageInfo","persistStorage"],"module":"brain-storage.js","contract":"Namespaced transactional IndexedDB records, bounded batches, expiry/build invalidation and explicit quota failures; never authentication authority"},
        {"name":"preferences","functions":["storage","cookie"],"module":"brain-preferences.js","contract":"Small bounded public local/session preferences and Secure/SameSite cookie names; no authentication credentials"},
        {"name":"navigation","functions":["urlState","urlDialog"],"module":"brain-url.js","contract":"Public typed query state, direct links and back/forward with component disposal; private form values excluded"},
        {"name":"sqlite","functions":["sqlite"],"module":"brain-sqlite.js","optionalPackage":"@sqlite.org/sqlite-wasm","contract":"Official SDK initializer in an owned OPFS worker; exclusive lease, non-destructive opening, bounded statements/rows and atomic batches","guide":"docs/BROWSER-DATA.md"},
        {"name":"threads","functions":["threadCapabilities","loadWorklet","messageChannel","offscreen","sharedMemory"],"module":"brain-threads.js","contract":"Detected native capabilities and explicit fallbacks; worklets, transferable canvas and isolated shared memory","guide":"docs/BROWSER-DATA.md"},
        {"name":"workers","functions":["*.worker.jsbrain","sharedWorker","transfer"],"module":"brain-worker.js","contract":"Lazy dedicated worker RPC with bounded pending calls, deadlines, cancellation and optional transfer lists"},
        {"name":"format","functions":["fmt"],"module":"brain-i18n.js","contract":"Intl date, number, money and relative formatting"},
        {"name":"backend","commands":["brain backend schema","brain media setup-gcs","brain media list","brain media cleanup","brain jobs work"],"contract":"Native Postgres contracts, authentication, private media, leased queues and explicit storage provisioning","guide":"docs/BACKEND.md"},
        {"name":"providers","commands":["brain integration list","brain integration add","brain integration update"],"contract":"Discover official AI and Google SDK adapters; selective pinned package plans, no provider calls or credential return","guide":"docs/INTEGRATIONS.md"},
        {"name":"design-system","commands":["brain design blueprint","brain design seed","brain design configure","brain design check","brain design gallery"],"contract":"Mandatory project-owned foundation, editable basics, baseline drift and static component contracts"},
        {"name":"environment","functions":[".brain_env"],"contract":"Process overrides, named environments and public/private separation; never return secret values"}
    ],"inclusion":"Native functions are auto-imported by use and pass through existing module tree shaking. Build ledger measures emitted bytes; exact per-component savings require fixture evidence.","guide":"docs/NATIVE-LIBRARIES.md","new":"brain lib new <helper-name> creates one pure named-export project helper; publication remains a separate reviewed contribution"})
}
pub fn new(root: &Path, name: &str) -> Result<Value, String> {
    if name.is_empty()
        || name.len() > 64
        || !name.starts_with(|c: char| c.is_ascii_lowercase())
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
    {
        return Err("helper name must start with a lowercase letter and contain 1..64 lowercase letters, digits, hyphens or underscores".into());
    }
    let mut symbol = String::new();
    let mut upper = false;
    for c in name.chars() {
        if c == '-' || c == '_' {
            upper = true;
        } else {
            symbol.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        }
    }
    if brain_check::scope::PRIMITIVES.contains(&symbol.as_str()) {
        return Err("helper name collides with a built-in primitive".into());
    }
    let path = format!("lib/{}.jsbrain", name.replace('_', "-"));
    let _writer = crate::edit::writer_lock(root)?;
    crate::changes::source_path(root, &path)?;
    let source = format!(
        "// Pure helper: keep inputs and outputs explicit; avoid module-level effects.\nexport function {symbol}(value) {{\n  return value\n}}\n"
    );
    brain_parse::parse(&source, Path::new(&path)).map_err(|_| "helper template did not parse")?;
    crate::changes::create_source(root, &path, &source)?;
    Ok(
        json!({"schemaVersion":1,"files":[path],"exports":[symbol],"installedPackages":[],"guide":"docs/NATIVE-LIBRARIES.md","next":"Implement a pure function, import the named export, run brain check and inspect the build byte ledger"}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_is_value_free_and_helper_creation_is_exclusive() {
        let root = std::env::temp_dir().join(format!("brain-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            list()["libraries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["name"] == "hash")
        );
        let result = new(&root, "normalize-title").unwrap();
        assert_eq!(result["exports"][0], "normalizeTitle");
        assert!(new(&root, "normalize-title").is_err());
        assert!(new(&root, "../escape").is_err());
        assert!(new(&root, "date-add").is_err());
        let source = std::fs::read_to_string(root.join("lib/normalize-title.jsbrain")).unwrap();
        let component =
            brain_parse::parse(&source, Path::new("lib/normalize-title.jsbrain")).unwrap();
        assert!(
            !brain_check::check(&component, &Default::default())
                .iter()
                .any(|d| !d.is_warning())
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
