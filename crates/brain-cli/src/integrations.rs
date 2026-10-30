//! Official SDK discovery and revision-checked source previews; never installs or calls a provider.
use serde_json::{Value, json};
use std::path::Path;
const MAX_PACKAGE: usize = 1024 * 1024;
fn template(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "openai" => Some((
            "server/integrations/openai.jsbrain",
            include_str!("templates/integrations/openai.jsbrain"),
        )),
        "claude" => Some((
            "server/integrations/claude.jsbrain",
            include_str!("templates/integrations/claude.jsbrain"),
        )),
        "gemini" => Some((
            "server/integrations/gemini.jsbrain",
            include_str!("templates/integrations/gemini.jsbrain"),
        )),
        "maps" => Some((
            "integrations/maps.jsbrain",
            include_str!("templates/integrations/maps.jsbrain"),
        )),
        _ => None,
    }
}
pub fn catalog() -> Value {
    let mut catalog: Value =
        serde_json::from_str(include_str!("templates/integrations/catalog.json"))
            .expect("embedded verified SDK catalog");
    catalog["adapterContract"] = json!({"version":1,"serverImports":"official SDK bare imports resolved by Node from app node_modules; not bundled into client assets","browserImports":"official ESM named exports; dependency pipeline must resolve pinned package imports","updates":"select one dependency; inspect its official release/API/engines and lockfile; rerun adapter regression tests","models":"required server environment configuration; no model default","secrets":"server-only; no SDK prompts, response bodies or credentials logged","optimization":"compiler-owned glue may be optimized; SDK public members, side effects, external keys and API contracts must remain intact"});
    catalog["existingNative"] = json!([{"id":"google-sign-in","implementation":"native auth Google Sign-In configuration; use the existing auth guide","duplicateSdk":false},{"id":"google-cloud-storage","implementation":"optional storage adapter; ADC/private access and explicit runtime setup","package":"@google-cloud/storage","automaticCloudWrites":false}]);
    catalog
}
fn version(s: &str) -> bool {
    let parts: Vec<_> = s.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 10
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}
/// `options`: optional exact `version`, `expectedRevision` for package.json, and `includeTemplate` (default true).
pub fn preview(root: &Path, id: &str, options: &Value) -> Result<Value, String> {
    if !options.is_object()
        || options
            .as_object()
            .unwrap()
            .keys()
            .any(|k| !["version", "expectedRevision", "includeTemplate"].contains(&k.as_str()))
    {
        return Err(
            "integration options accept only version, expectedRevision and includeTemplate".into(),
        );
    }
    let catalog = catalog();
    let entry = catalog["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .ok_or("unknown integration; inspect the official SDK catalog")?;
    let package = entry["package"].as_str().unwrap();
    let pin = options
        .get("version")
        .map_or(entry["version"].as_str(), Value::as_str)
        .filter(|s| version(s))
        .ok_or("integration version must be an exact stable major.minor.patch pin")?;
    let read = crate::changes::read(root, "package.json")?;
    if options
        .get("expectedRevision")
        .is_some_and(|v| v != &read["revision"])
    {
        return Err("package.json changed; prepare a new integration preview".into());
    }
    let source = read["content"].as_str().unwrap_or("{}");
    if source.len() > MAX_PACKAGE {
        return Err("package.json exceeds 1 MiB integration budget".into());
    }
    let mut value: Value =
        serde_json::from_str(source).map_err(|_| "package.json must contain valid JSON")?;
    if !value.is_object() {
        return Err("package.json must contain an object".into());
    }
    let mut found = vec![];
    for section in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(entries) = value.get(section) {
            let entries = entries
                .as_object()
                .ok_or("package dependency sections must contain objects")?;
            if let Some(old) = entries.get(package) {
                let old = old
                    .as_str()
                    .ok_or("existing SDK dependency must be a version string")?;
                found.push(json!({"section":section,"from":old,"to":pin}));
            }
        }
    }
    let mut warnings=vec!["Review the generated source, official SDK compatibility and Node engines before applying; preview does not install dependencies or update a lockfile.".to_string()];
    let declared = found
        .first()
        .map(|p| p["section"].as_str().unwrap())
        .unwrap_or("dependencies")
        .to_owned();
    if found.len() > 1 {
        return Err("SDK appears in multiple dependency sections; resolve ownership explicitly before previewing an update".into());
    }
    if declared != "dependencies" {
        warnings.push("Existing dependency section is preserved. Ensure the official server SDK is installed in the production deployment; dev/optional dependencies can be omitted.".into());
    }
    if let Some(peer) = value.get("peerDependencies").and_then(|p| p.get(package)) {
        if !peer.is_string() {
            return Err("SDK peer dependency must be a string".into());
        }
        warnings.push(
            "Existing peer dependency range is preserved; review its compatibility separately."
                .into(),
        );
    }
    if value.get(&declared).is_none() {
        value[&declared] = json!({});
    }
    value[&declared][package] = json!(pin);
    let updated = format!(
        "{}\n",
        serde_json::to_string_pretty(&value).map_err(|_| "cannot serialize package preview")?
    );
    if updated.len() > MAX_PACKAGE {
        return Err("formatted package.json exceeds 1 MiB integration budget".into());
    }
    let original: Value = serde_json::from_str(source).unwrap();
    let mut operations = vec![];
    if value != original {
        operations.push(
            json!({"path":"package.json","expectedRevision":read["revision"],"content":updated}),
        );
    }
    let include = options
        .get("includeTemplate")
        .map_or(Some(true), Value::as_bool)
        .ok_or("includeTemplate must be boolean")?;
    let mut template_status = "not-requested";
    if include {
        if let Some((path, content)) = template(id) {
            let existing = crate::changes::read(root, path)?;
            if existing["revision"].is_null() {
                operations.push(json!({"path":path,"expectedRevision":null,"content":content}));
                template_status = "create-preview";
            } else {
                template_status = "existing-preserved";
            }
        } else {
            template_status = "existing-native-adapter";
        }
    }
    Ok(
        json!({"schemaVersion":1,"dryRun":true,"integration":id,"adapterContractVersion":1,"package":package,"pin":pin,"verifiedRegistrySnapshot":pin==entry["version"].as_str().unwrap(),"verifiedAt":catalog["verifiedAt"],"integrity":if pin==entry["version"].as_str().unwrap(){entry["integrity"].clone()}else{Value::Null},"officialDocs":entry["officialDocs"],"engines":entry["engines"],"previous":found,"operations":operations,"templateStatus":template_status,"warnings":warnings,"envKeys":entry["envKeys"],"installation":{"executed":false,"lockfileUpdated":false,"next":"Apply reviewed CAS operations through the change planner; explicitly install the pinned SDK with the app package manager, review its lockfile and deploy production dependencies."},"browserDependencies":if id=="maps"{"resolve official pinned ESM package with the compiler dependency pipeline before browser use"}else{"not required; SDK stays server-only"},"verification":{"remoteCalls":false,"credentialsRead":false,"compatibility":"official example shape; offline regression tests do not prove live provider/model availability"}}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct App(std::path::PathBuf);
    impl App {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "brain-integrations-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn catalog_versions_and_selective_cas_preserve_unknown_configuration() {
        let app = App::new();
        std::fs::write(
            app.0.join("package.json"),
            r#"{"custom":{"keep":true},"dependencies":{"other":"2.0.0"}}"#,
        )
        .unwrap();
        let result = preview(&app.0, "openai", &json!({})).unwrap();
        assert_eq!(result["verifiedRegistrySnapshot"], true);
        assert_eq!(result["operations"].as_array().unwrap().len(), 2);
        let package: Value =
            serde_json::from_str(result["operations"][0]["content"].as_str().unwrap()).unwrap();
        assert_eq!(package["custom"]["keep"], true);
        assert_eq!(package["dependencies"]["other"], "2.0.0");
        assert!(package["dependencies"].get("@google/genai").is_none());
        assert!(!app.0.join("server").exists());
        assert!(preview(&app.0, "openai", &json!({"expectedRevision":"stale"})).is_err());
        assert!(preview(&app.0, "openai", &json!({"version":"latest"})).is_err());
    }
    #[test]
    fn existing_templates_are_preserved_and_non_snapshot_versions_are_unverified() {
        let app = App::new();
        let (path, _) = template("claude").unwrap();
        std::fs::create_dir_all(app.0.join(path).parent().unwrap()).unwrap();
        std::fs::write(
            app.0.join(path),
            "<meta server>\nexport const custom = true",
        )
        .unwrap();
        let p = preview(&app.0, "claude", &json!({"version":"1.0.0"})).unwrap();
        assert_eq!(p["templateStatus"], "existing-preserved");
        assert_eq!(p["verifiedRegistrySnapshot"], false);
        assert_eq!(p["integrity"], Value::Null);
        assert_eq!(p["operations"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn server_templates_compile_to_server_only_official_imports() {
        let app = App::new();
        let compiled = app.0.join("compiled");
        std::fs::create_dir(&compiled).unwrap();
        for id in ["openai", "claude", "gemini"] {
            let (path, source) = template(id).unwrap();
            let out = crate::compiler::compile(source, Path::new(path), &app.0, true)
                .unwrap_or_else(|d| panic!("{id}: {d:?}"));
            assert!(out.js.is_empty());
            let catalog = catalog();
            let e = catalog["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == id)
                .unwrap();
            assert!(out.server_js.contains(e["package"].as_str().unwrap()));
            assert!(!out.server_js.contains("gpt-"));
            let runtime = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../runtime")
                .canonicalize()
                .unwrap();
            let linked = brain_pack::rewrite_imports(&out.server_js, &mut |specifier| {
                if let Some(file) = specifier.strip_prefix("/@brain/") {
                    assert!(!file.contains("..") && !file.contains('\\'));
                    Ok(Some(format!("file://{}", runtime.join(file).display())))
                } else {
                    Ok(None)
                }
            })
            .unwrap();
            std::fs::write(compiled.join(format!("{id}.mjs")), linked).unwrap();
        }
        let check = std::process::Command::new("node")
            .args(["--test", "--test-timeout=15000"])
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src/templates/integrations/adapters.test.mjs"),
            )
            .env("BRAIN_SDK_COMPILED_DIR", &compiled)
            .output()
            .expect("Node required for compiled SDK adapter regression");
        assert!(
            check.status.success(),
            "compiled SDK adapters failed: {} {}",
            String::from_utf8_lossy(&check.stdout),
            String::from_utf8_lossy(&check.stderr)
        );
    }
}
