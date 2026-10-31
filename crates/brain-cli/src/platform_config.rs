//! Literal platform contracts for the native backend: file buckets (`server/files/<name>.jsbrain`).
use brain_backend::{Resource, auth_flows::RolesConfig, files::Bucket, platform::Platform};
use serde_json::{Value, json};
use std::{fs, path::Path};

const FILES: &str = "server/files";

fn buckets(root: &Path, resources: &[Resource], roles: &RolesConfig) -> Result<Vec<Bucket>, String> {
    let dir = root.join(FILES);
    if !dir.is_dir() {
        return Ok(vec![]);
    }
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .map_err(|_| format!("{FILES}: directory is unreadable"))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|_| format!("{FILES}: directory is unreadable"))?;
    entries.sort_by_key(|e| e.file_name());
    let mut out = Vec::new();
    for entry in entries {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(name) = file.strip_suffix(".jsbrain") else { continue };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            return Err(format!("{FILES}/{file}: bucket contracts are regular files"));
        }
        let value = crate::backend::read_contract(root, &entry.path())
            .map_err(|e| format!("{FILES}/{file}: {e}"))?;
        let bucket = Bucket::from_contract(name, value)?;
        bucket.validate(resources, &roles.roles)?;
        out.push(bucket);
    }
    if out.len() > 64 {
        return Err(format!("{FILES}: declare at most 64 buckets"));
    }
    Ok(out)
}

pub fn load(root: &Path, resources: &[Resource], roles: &RolesConfig) -> Result<Platform, String> {
    Ok(Platform { buckets: buckets(root, resources, roles)?, ..Default::default() })
}

/// Projection for `brain backend schema --json`; contains no environment values.
pub fn metadata(root: &Path, resources: &[Resource], roles: &RolesConfig) -> Result<Value, String> {
    let platform = load(root, resources, roles)?;
    Ok(json!({
        "files": platform.buckets.iter().map(Bucket::schema).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buckets_load_from_literal_contracts_and_fail_with_the_file_name() {
        let root = std::env::temp_dir().join(format!("brain-platform-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(FILES)).unwrap();
        fs::write(root.join(FILES).join("attachments.jsbrain"), "<meta server>\nexport default { visibility: 'private', maxBytes: '10mb', types: ['application/pdf', 'image/*'], create: ['user'] }\n").unwrap();
        let roles = RolesConfig::default();
        let meta = metadata(&root, &[], &roles).unwrap();
        assert_eq!(meta["files"][0]["name"], "attachments");
        assert_eq!(meta["files"][0]["maxBytes"], 10 * 1024 * 1024);
        fs::write(root.join(FILES).join("bad.jsbrain"), "<meta server>\nexport default { create: ['ghost'] }\n").unwrap();
        let error = metadata(&root, &[], &roles).unwrap_err();
        assert!(error.contains("server/files/bad.jsbrain"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }
}
