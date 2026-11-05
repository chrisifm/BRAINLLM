//! Portable output is a project policy, shared by CLI, doctor and MCP (not process-global state).
use std::path::Path;

pub(crate) fn enabled(root: &Path, force: bool) -> Result<bool, String> {
    let path = root.join("brain.json");
    let config: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map_err(|_| "brain.json: invalid JSON; fix: correct the configuration")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(_) => return Err("brain.json: cannot read configuration; fix: check its permissions".into()),
    };
    if !config.is_object() { return Err("brain.json must be an object; fix: use a JSON configuration object".into()); }
    match config.get("output") {
        None => Ok(force),
        Some(serde_json::Value::String(mode)) if mode == "auto" || mode == "static" => Ok(force || mode == "static"),
        _ => Err("brain.json output must be auto or static; fix: set \"output\": \"static\" for portable HTML or remove output for automatic server selection".into()),
    }
}

pub(crate) fn validate(root: &Path) -> Result<(), String> {
    // Backend contracts do not necessarily enter the frontend import graph. Never silently omit them.
    fn declarations(dir: &Path, depth: usize) -> Result<bool, String> {
        if depth > 64 { return Err("server contracts exceed the directory depth limit; fix: simplify their layout".into()); }
        let meta = match std::fs::symlink_metadata(dir) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err("cannot inspect server contracts; fix: check their permissions".into()),
        };
        if meta.file_type().is_symlink() { return Err("static output cannot inspect a symlinked server contract directory; fix: use ordinary project files".into()); }
        let entries = std::fs::read_dir(dir).map_err(|_| "cannot read server contracts; fix: check their permissions")?;
        for entry in entries {
            let entry = entry.map_err(|_| "cannot read server contract entry; fix: check its permissions")?;
            let kind = entry.file_type().map_err(|_| "cannot inspect server contract entry; fix: check its permissions")?;
            if kind.is_symlink() { return Err("static output cannot inspect symlinked server contracts; fix: use ordinary project files".into()); }
            if kind.is_dir() {
                if declarations(&entry.path(), depth + 1)? { return Ok(true); }
            } else if entry.path().extension().is_some_and(|ext| ext == "jsbrain") { return Ok(true); }
        }
        Ok(false)
    }
    let config: serde_json::Value = std::fs::read_to_string(root.join("brain.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if config.get("backend").is_some_and(|v| !v.is_null()) {
        return Err("brain.json backend: native backend declarations require a server; fix: remove backend for a static site or use automatic output mode".into());
    }
    for folder in ["resources", "jobs", "routes", "auth", "files", "mail", "webhooks", "sockets", "hooks"] {
        if declarations(&root.join("server").join(folder), 0)? {
            return Err(format!("server/{folder}: backend contracts require a server; fix: remove the contract for a static site, or remove static output mode"));
        }
    }
    for file in ["server/platform.jsbrain", "server/cache.jsbrain", "server/analytics.jsbrain", "server/api-keys.jsbrain"] {
        if root.join(file).exists() { return Err(format!("{file}: native server configuration requires a server; fix: remove it for a static site or use automatic output mode")); }
    }
    Ok(())
}


/// Reserve only empty placeholders in an owned, short-lived directory on the destination filesystem.
/// This catches native Unicode normalization/case aliasing before touching the previous published artifact.
pub(crate) fn check_destinations<'a>(dist: &Path, paths: impl Iterator<Item = &'a String>) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Probe(std::path::PathBuf);
    impl Drop for Probe { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let mut owned = None;
    for _ in 0..64 {
        let path = dist.join(format!(".brain-output-check-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::create_dir(&path) {
            Ok(()) => { owned = Some(Probe(path)); break; }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("cannot validate static output destinations; fix: check dist/ permissions".into()),
        }
    }
    let probe = owned.ok_or("cannot reserve a static output check directory; fix: remove obsolete check directories owned by stopped builds")?;
    let mut directories = std::collections::HashSet::new();
    for path in paths {
        let relative = Path::new(path);
        if relative.components().any(|part| !matches!(part, std::path::Component::Normal(_))) || path.contains('\\') {
            return Err("static output destination is not a confined relative path".into());
        }
        for parent in relative.ancestors().skip(1).filter(|parent| !parent.as_os_str().is_empty()) {
            let identity = parent.to_string_lossy().to_string();
            if !directories.insert(identity.clone()) { continue; }
            let directory = probe.0.join(parent);
            std::fs::create_dir_all(&directory).map_err(|_| "static output has a file/directory collision")?;
            let marker = directory.join(".brain-output-directory");
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&marker) {
                Ok(mut file) => std::io::Write::write_all(&mut file, identity.as_bytes()).map_err(|_| "cannot record static directory identity")?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if std::fs::read_to_string(&marker).ok().as_deref() != Some(identity.as_str()) {
                        return Err(format!("static output directory alias at `{identity}`; fix: use one spelling under the filesystem's case and Unicode normalization rules"));
                    }
                }
                Err(_) => return Err("cannot validate static directory identity".into()),
            }
        }
        let file = probe.0.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|_| "static output has a file/directory collision; fix: use distinct page and asset names")?;
        std::fs::OpenOptions::new().write(true).create_new(true).open(&file).map_err(|_| format!("static output destination collision or unwritable file `{path}`; fix: use names distinct under the host filesystem's case and Unicode normalization rules"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_reservations_detect_native_aliases_and_clean_only_owned_scratch() {
        let root = std::env::temp_dir().join(format!("brain-static-destinations-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("keep.txt"), "previous artifact").unwrap();
        let composed = "caf\u{00e9}";
        let decomposed = "cafe\u{0301}";
        std::fs::write(root.join(composed), "native normalization probe").unwrap();
        let normalizes = std::fs::read_to_string(root.join(decomposed)).ok().as_deref() == Some("native normalization probe");
        std::fs::remove_file(root.join(composed)).unwrap();
        let paths = [format!("{composed}/index.html"), format!("{decomposed}/index.html")];
        let result = check_destinations(&root, paths.iter());
        assert_eq!(result.is_err(), normalizes, "must follow the actual destination filesystem");
        let distinct_files = [format!("{composed}/a.html"), format!("{decomposed}/b.html")];
        assert_eq!(check_destinations(&root, distinct_files.iter()).is_err(), normalizes, "directory aliases also corrupt otherwise distinct exported paths");
        let duplicate = ["a/index.html".to_string(), "a/index.html".to_string()];
        assert!(check_destinations(&root, duplicate.iter()).is_err());
        assert_eq!(std::fs::read_to_string(root.join("keep.txt")).unwrap(), "previous artifact");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "no scratch survives success or failure");
        std::fs::remove_dir_all(root).unwrap();
    }
}
