//! Literal platform contracts for the native backend: file buckets (`server/files/<name>.jsbrain`)
//! and HTTP edge settings (`server/platform.jsbrain`).
use brain_backend::{Resource, auth_flows::RolesConfig, cluster::Cluster, files::Bucket, platform::{Edge, Platform}};
use serde_json::{Value, json};
use std::{fs, path::Path};

const FILES: &str = "server/files";
const EDGE: &str = "server/platform.jsbrain";

fn edge(root: &Path) -> Result<(Edge, Cluster), String> {
    let path = root.join(EDGE);
    if !path.exists() {
        return Ok((Edge::default(), Cluster::default()));
    }
    if !path.is_file() {
        return Err(format!("{EDGE}: must be a regular file"));
    }
    let value = crate::backend::read_contract(root, &path).map_err(|e| format!("{EDGE}: {e}"))?;
    let edge = Edge::from_contract(&value).map_err(|e| format!("{EDGE}: {e}"))?;
    let cluster = Cluster::from_contract(value.get("cluster")).map_err(|e| format!("{EDGE}: {e}"))?;
    Ok((edge, cluster))
}

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
    let (edge, cluster) = edge(root)?;
    Ok(Platform { buckets: buckets(root, resources, roles)?, edge, cluster, ..Default::default() })
}

/// Projection for `brain backend schema --json`; contains no environment values.
pub fn metadata(root: &Path, resources: &[Resource], roles: &RolesConfig) -> Result<Value, String> {
    let platform = load(root, resources, roles)?;
    let edge = &platform.edge;
    Ok(json!({
        "files": platform.buckets.iter().map(Bucket::schema).collect::<Vec<_>>(),
        "edge": {
            "trustProxy": edge.trust_proxy.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "corsOrigins": edge.cors_origins,
            "corsMaxAge": edge.cors_max_age.map(|d| d.as_secs()),
            "maxInFlight": edge.max_in_flight,
            "readyDelayMs": edge.ready_delay.as_millis() as u64,
            "drainMs": edge.drain.as_millis() as u64,
            "requestLog": format!("{:?}", edge.request_log).to_lowercase(),
            "slowMs": edge.slow.map(|d| d.as_millis() as u64),
        },
        "cluster": platform.cluster.schema(),
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
        fs::remove_file(root.join(FILES).join("bad.jsbrain")).unwrap();
        assert_eq!(metadata(&root, &[], &roles).unwrap()["edge"]["maxInFlight"], 512);
        fs::write(root.join(EDGE), "<meta server>\nexport default { trustProxy: ['127.0.0.1'], cors: { maxAge: 600 }, shutdown: { readyDelay: '5s' } }\n").unwrap();
        let meta = metadata(&root, &[], &roles).unwrap();
        assert_eq!(meta["edge"]["trustProxy"][0], "127.0.0.1/32");
        assert_eq!(meta["edge"]["corsMaxAge"], 600);
        assert_eq!(meta["edge"]["readyDelayMs"], 5000);
        assert_eq!(meta["edge"]["requestLog"], "errors");
        fs::write(root.join(EDGE), "<meta server>\nexport default { trustProxy: ['0.0.0.0/0'] }\n").unwrap();
        let error = metadata(&root, &[], &roles).unwrap_err();
        assert!(error.contains("server/platform.jsbrain") && error.contains("every address"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }
}
