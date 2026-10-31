//! Package-pin upgrade previews. No registry requests, installations or invented
//! source migrations. The caller applies returned operations through `changes`.
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::fs;
use std::path::Path;

const PACKAGE: &str = "@christianirack/brainllm";
const MAX_BYTES: u64 = 2 * 1024 * 1024;

type Result<T> = std::result::Result<T, String>;
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Version {
    numbers: [u64; 3],
    pre: Vec<String>,
}
fn version(input: &str) -> Result<Version> {
    if input.is_empty() || input.len() > 128 || !input.is_ascii() {
        return Err("target must be an explicit semantic version of 1..128 ASCII bytes".into());
    }
    fn identifiers(value: &str, prerelease: bool) -> Result<Vec<String>> {
        value
            .split('.')
            .map(|id| {
                if id.is_empty()
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || (prerelease
                        && id.len() > 1
                        && id.bytes().all(|b| b.is_ascii_digit())
                        && id.starts_with('0'))
                {
                    Err("invalid semantic version identifier".into())
                } else {
                    Ok(id.to_owned())
                }
            })
            .collect()
    }
    let (base, build) = input
        .split_once('+')
        .map_or((input, None), |(a, b)| (a, Some(b)));
    if let Some(build) = build {
        identifiers(build, false)?;
    }
    let (core, pre) = base
        .split_once('-')
        .map_or((base, None), |(a, b)| (a, Some(b)));
    let pieces: Vec<_> = core.split('.').collect();
    if pieces.len() != 3 {
        return Err("target must be an exact major.minor.patch version, not a tag or range".into());
    }
    let mut numbers = [0; 3];
    for (index, part) in pieces.iter().enumerate() {
        if part.is_empty()
            || !part.bytes().all(|b| b.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return Err("semantic version numbers must be canonical nonnegative integers".into());
        }
        numbers[index] = part
            .parse()
            .map_err(|_| "semantic version number exceeds u64")?;
    }
    Ok(Version {
        numbers,
        pre: pre
            .map(|p| identifiers(p, true))
            .transpose()?
            .unwrap_or_default(),
    })
}
pub(crate) fn parse_version(input: &str) -> Result<Version> {
    version(input)
}
pub(crate) fn compare_versions(a: &Version, b: &Version) -> Ordering {
    compare(a, b)
}
fn compare(a: &Version, b: &Version) -> Ordering {
    let core = a.numbers.cmp(&b.numbers);
    if core != Ordering::Equal {
        return core;
    }
    match (a.pre.is_empty(), b.pre.is_empty()) {
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    for (x, y) in a.pre.iter().zip(&b.pre) {
        let xn = x.bytes().all(|b| b.is_ascii_digit());
        let yn = y.bytes().all(|b| b.is_ascii_digit());
        let cmp = match (xn, yn) {
            (true, true) => x.len().cmp(&y.len()).then_with(|| x.cmp(y)),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => x.cmp(y),
        };
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    a.pre.len().cmp(&b.pre.len())
}
fn hash(bytes: &[u8]) -> String {
    brain_pack::sha::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Metadata-only inventory; adoption is optional and never changes the package-pin operations.
fn adoption(root: &Path) -> Value {
    let files = [
        "brain.json",
        "app.brain",
        "tokens.cssbrain",
        "tokens.brain.css",
        "design.brain.json",
    ];
    let inventory: Vec<_> = files
        .iter()
        .map(|file| {
            let path = root.join(file);
            let state = match fs::symlink_metadata(path) {
                Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => "unsafe",
                Ok(_) => "present",
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => "missing",
                Err(_) => "unavailable",
            };
            json!({"file":file,"state":state})
        })
        .collect();
    let preview = match crate::design::preview_init(root, &json!({})) {
        Ok(plan) => json!({"status":"preview-ready","plan":plan,"executed":false}),
        Err(_) => {
            json!({"status":"needs-inspection","executed":false,"operations":[],"reason":"No safe supported token foundation could be adopted. Inspect token/baseline files; preserve existing source and resolve the design diagnostic separately."})
        }
    };
    json!({"inventory":inventory,"designAdoption":preview,"preserved":["tokens and fonts","unknown design fields","application behavior","application data","base path","component source"],"transformations":[],"instructions":["Inspect existing native and legacy token stylesheets before changing the design foundation.","Preview optional foundation adoption separately; acquire its source-file lease and preserve expectedRevision.","Recheck the token source revision from the adoption plan before applying it; a concurrent token edit needs a new preview.","Do not rewrite components or data without a separately reviewed behavior-preserving migration slice."]})
}

/// Preview an exact package pin update. Success is not evidence that a source
/// migration exists, the target is published, or an install/build will succeed.
pub fn preview(root: &Path, target: &str) -> Result<Value> {
    let target_version = version(target)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("upgrade root must be an app directory".into());
    }
    let path = root.join("package.json");
    let meta =
        fs::symlink_metadata(&path).map_err(|_| "upgrade requires an existing package.json")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_BYTES {
        return Err("package.json must be a regular, nonsymlink file of at most 2 MiB".into());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("package.json exceeds 2 MiB".into());
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| "package.json must be UTF-8")?;
    if !brain_check::scan_secrets(source).is_empty() {
        return Err(
            "package.json contains a detected secret; remove it before creating an upgrade preview"
                .into(),
        );
    }
    let mut package: Value =
        serde_json::from_str(source).map_err(|_| "package.json is invalid JSON")?;
    if !package.is_object() {
        return Err("package.json must contain an object".into());
    }
    let mut pins = vec![];
    let mut decisions = vec![];
    let mut warnings = vec![];
    for section in ["dependencies", "devDependencies", "optionalDependencies"] {
        let Some(entries) = package.get_mut(section) else {
            continue;
        };
        let entries = entries
            .as_object_mut()
            .ok_or_else(|| format!("package.json {section} must be an object"))?;
        let Some(pin) = entries.get_mut(PACKAGE) else {
            continue;
        };
        let old = pin
            .as_str()
            .ok_or_else(|| format!("{section}.{PACKAGE} must be a version string"))?
            .to_owned();
        match version(&old) {
            Ok(current)=>{
                if compare(&target_version,&current)==Ordering::Less {return Err(format!("target {target} is older than existing requirement {old}; downgrade needs a separate explicit migration plan"));}
                let breaking=if current.numbers[0]==0 {target_version.numbers[0]>0 || target_version.numbers[1]>current.numbers[1]} else {target_version.numbers[0]>current.numbers[0]};
                if breaking {decisions.push(format!("Review breaking changes from {old} to {target}; no versioned source migration was selected."));}
            },
            Err(_)=>decisions.push(format!("Resolve the actual current version behind {section} requirement {old} before assessing migration compatibility.")),
        }
        pins.push(json!({"section":section,"package":PACKAGE,"from":old,"to":target,"changed":old!=target}));
        *pin = json!(target);
    }
    if pins.is_empty() {
        return Err(format!(
            "package.json does not declare {PACKAGE} in dependencies, devDependencies or optionalDependencies; an upgrade does not add a missing framework dependency"
        ));
    }
    if pins.len() > 1 {
        warnings.push("The framework appears in multiple dependency sections; this preview updates every installable declaration consistently.".to_owned());
    }
    if let Some(peer) = package.get("peerDependencies").and_then(|p| p.get(PACKAGE)) {
        decisions.push(format!("Review the existing peer dependency requirement {peer}; it is preserved and not automatically widened."));
    }
    if !target_version.pre.is_empty() {
        decisions.push("Confirm that the requested prerelease is appropriate for this app.".into());
    }
    let changed = pins.iter().any(|p| p["changed"] == true);
    let content = if changed {
        format!(
            "{}\n",
            serde_json::to_string_pretty(&package).map_err(|e| e.to_string())?
        )
    } else {
        source.to_owned()
    };
    if content.len() as u64 > MAX_BYTES {
        return Err("formatted package.json would exceed 2 MiB".into());
    }
    if changed {
        warnings.push("package.json is formatted as two-space JSON; all other effective JSON values are preserved. Duplicate keys, if any, use standard JSON last-value semantics.".into());
    }
    decisions.push("Review release notes and supply an actual released compatibility baseline; this preview has not verified source compatibility or registry availability.".into());
    let operations = if changed {
        vec![json!({"path":"package.json","expectedRevision":hash(&bytes),"content":content})]
    } else {
        vec![]
    };
    Ok(json!({
        "schemaVersion":1,"dryRun":true,"scope":"package pin only","targetVersion":target,
        "package":PACKAGE,"pins":pins,"operations":operations,"changesRequired":changed,
        "adoption":adoption(&root),
        "sourceMigrations":[],"sourceMigrationStatus":"No versioned source migrations were selected or executed.",
        "registryVerified":false,"installation":{"required":changed,"executed":false,"manager":"not selected"},
        "requiresHumanDecisions":decisions,"warnings":warnings,
        "applyProtocol":"Acquire a package.json lease, then pass operations to brain changes plan and brain changes apply; do not bypass expectedRevision.",
        "requiredGates":["Review an explicit released contract with scripts/compatibility-check.mjs", "Install the pinned version with the app's reviewed package manager and review its lockfile", "brain check", "brain test", "brain build", "Run app tests and migration slices"],
        "frameworkContributionGates":["cargo test --workspace", "npm test --prefix runtime", "node bench/fixtures-check.mjs", "scripts/check-english.sh", "node bench/llm/gate.mjs when author contracts change"]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct App(PathBuf);
    impl App {
        fn new(content: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "brain-upgrade-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, AtomicOrdering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::write(path.join("package.json"), content).unwrap();
            Self(path)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn exact_semver_validation_rejects_tags_ranges_and_malformed_identifiers() {
        for invalid in [
            "latest",
            "^1.2.3",
            "~1.2.3",
            "v1.2.3",
            "1.2",
            "1.2.3 ",
            "01.2.3",
            "1.2.3-01",
            "1.2.3+",
            "1.2.3-alpha..1",
            "1.2.3+abc+def",
            "1.2.3-β",
        ] {
            assert!(version(invalid).is_err(), "{invalid}");
        }
        for valid in [
            "0.3.2",
            "1.2.3-rc.1",
            "1.2.3+build.01",
            "1.2.3-alpha-1+sha.123",
        ] {
            assert!(version(valid).is_ok(), "{valid}");
        }
        assert!(version(&"1".repeat(129)).is_err());
        assert_eq!(
            compare(
                &version("1.0.0-rc.10").unwrap(),
                &version("1.0.0-rc.2").unwrap()
            ),
            Ordering::Greater
        );
        assert_eq!(
            compare(&version("1.0.0").unwrap(), &version("1.0.0-rc.99").unwrap()),
            Ordering::Greater
        );
    }
    #[test]
    fn preview_preserves_other_json_values_and_never_writes_installs_or_claims_migrations() {
        let original = r#"{"name":"My App","scripts":{"build":"brain build"},"private":true,"custom":[1,null,{"text":"a\\b\n世界"}],"devDependencies":{"@christianirack/brainllm":"0.3.1","other":"^2.0.0"}}"#;
        let app = App::new(original);
        let result = preview(&app.0, "0.4.0").unwrap();
        assert_eq!(
            fs::read_to_string(app.0.join("package.json")).unwrap(),
            original
        );
        assert_eq!(result["dryRun"], true);
        assert_eq!(result["registryVerified"], false);
        assert_eq!(result["installation"]["executed"], false);
        assert_eq!(result["sourceMigrations"], json!([]));
        assert_eq!(
            result["operations"][0]["expectedRevision"],
            hash(original.as_bytes())
        );
        let mut expected: Value = serde_json::from_str(original).unwrap();
        expected["devDependencies"][PACKAGE] = json!("0.4.0");
        let actual: Value =
            serde_json::from_str(result["operations"][0]["content"].as_str().unwrap()).unwrap();
        assert_eq!(actual, expected);
        assert!(
            result["requiresHumanDecisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str().unwrap().contains("breaking"))
        );
    }
    #[test]
    fn unchanged_pin_is_noop_and_missing_or_invalid_dependency_is_not_invented() {
        let app = App::new(r#"{"dependencies":{"@christianirack/brainllm":"1.2.3"}}"#);
        let result = preview(&app.0, "1.2.3").unwrap();
        assert_eq!(result["operations"], json!([]));
        assert_eq!(result["changesRequired"], false);
        assert!(preview(&app.0, "1.2.2").unwrap_err().contains("older"));
        for source in [
            "[]",
            "not json",
            r#"{"dependencies":[]}"#,
            r#"{"dependencies":{"@christianirack/brainllm":42}}"#,
            r#"{"dependencies":{"other":"1.0.0"}}"#,
        ] {
            let app = App::new(source);
            assert!(preview(&app.0, "1.2.3").is_err(), "{source}");
        }
    }
    #[test]
    fn ranges_duplicates_and_peer_dependencies_require_explicit_review() {
        let app = App::new(
            r#"{"dependencies":{"@christianirack/brainllm":"^1.0.0"},"devDependencies":{"@christianirack/brainllm":"1.0.0"},"peerDependencies":{"@christianirack/brainllm":">=1"}}"#,
        );
        let result = preview(&app.0, "1.1.0-rc.1").unwrap();
        assert_eq!(result["pins"].as_array().unwrap().len(), 2);
        let content: Value =
            serde_json::from_str(result["operations"][0]["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["dependencies"][PACKAGE], "1.1.0-rc.1");
        assert_eq!(content["devDependencies"][PACKAGE], "1.1.0-rc.1");
        assert_eq!(content["peerDependencies"][PACKAGE], ">=1");
        assert!(result["requiresHumanDecisions"].as_array().unwrap().len() >= 3);
    }
    #[test]
    fn design_adoption_is_separate_reversible_and_preserves_legacy_values() {
        let app = App::new(r#"{"dependencies":{"@christianirack/brainllm":"1.2.3"}}"#);
        let tokens = ":root { --accent: #123456; --font: 'Local Font'; }";
        fs::write(app.0.join("tokens.brain.css"), tokens).unwrap();
        fs::write(
            app.0.join("design.brain.json"),
            r#"{"custom":{"keep":true}}"#,
        )
        .unwrap();
        let result = preview(&app.0, "1.2.4").unwrap();
        assert_eq!(result["operations"].as_array().unwrap().len(), 1);
        let adoption = &result["adoption"]["designAdoption"];
        assert_eq!(adoption["status"], "preview-ready");
        let operation = &adoption["plan"]["operations"][0];
        assert_eq!(operation["path"], "design.brain.json");
        let proposed: Value = serde_json::from_str(operation["content"].as_str().unwrap()).unwrap();
        assert_eq!(proposed["custom"]["keep"], true);
        assert_eq!(proposed["source"]["tokensFile"], "tokens.brain.css");
        assert_eq!(
            fs::read_to_string(app.0.join("tokens.brain.css")).unwrap(),
            tokens
        );
        assert_eq!(result["sourceMigrations"], json!([]));
    }
    #[test]
    fn bounds_and_symlinks_are_rejected() {
        let app = App::new(&" ".repeat(MAX_BYTES as usize + 1));
        assert!(preview(&app.0, "1.0.0").unwrap_err().contains("2 MiB"));
        #[cfg(unix)]
        {
            fs::remove_file(app.0.join("package.json")).unwrap();
            fs::write(app.0.join("actual.json"), "{}").unwrap();
            std::os::unix::fs::symlink(app.0.join("actual.json"), app.0.join("package.json"))
                .unwrap();
            assert!(preview(&app.0, "1.0.0").unwrap_err().contains("nonsymlink"));
        }
    }
}
