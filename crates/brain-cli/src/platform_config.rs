//! Literal platform contracts for the native backend: file buckets (`server/files/<name>.jsbrain`)
//! and HTTP edge settings (`server/platform.jsbrain`).
use brain_backend::{Resource, auth_flows::RolesConfig, cluster::Cluster, files::Bucket, http::cache_http::CachePolicy, platform::{Edge, Platform}};
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

/// `platform.trustProxy` for the build manifest; a broken contract is reported by `brain check`.
pub fn trust_proxy(root: &Path) -> Vec<String> {
    edge(root).map(|(edge, _)| edge.trust_proxy.iter().map(ToString::to_string).collect()).unwrap_or_default()
}

/// B0784: what `brain doctor`, `brain check` and a production `brain backend serve` say about `platform.trustProxy`.
/// (message, fix); `refuse` marks the public range a production start stops on.
pub fn proxy_findings(root: &Path, site: Option<&str>) -> Vec<(String, String, bool)> {
    let Ok((edge, _)) = edge(root) else { return Vec::new() };
    let mut out = Vec::new();
    for net in edge.undeclared_public_proxies() {
        out.push((format!("{EDGE}: trustProxy entry `{net}` reaches public addresses, so whoever holds one can choose a client's rate-limit key; a production start refuses it"),
            "list only your own proxies; a CDN in front of the backend goes in `trustPublicProxy` with its published ranges".to_string(), true));
    }
    // A private range holds machines you do not run as a proxy: any of them could then choose a client's address.
    for net in edge.wide_private_proxies() {
        out.push((format!("{EDGE}: trustProxy entry `{net}` is wider than a proxy needs, so any machine in it can choose a client's rate-limit key"),
            "list the proxy's own address or smallest subnet".to_string(), false));
    }
    let public_site = site.is_some_and(|site| site.starts_with("https://"));
    if edge.trust_proxy.is_empty() && public_site && (root.join("server/resources").is_dir() || root.join("server/auth").is_dir()) {
        out.push((format!("{EDGE}: no trustProxy, and the native backend listens only on loopback, so behind a proxy every visitor shares one rate-limit and lockout key"),
            "declare the proxy that reaches the backend: `export default { trustProxy: ['127.0.0.1', '::1'] }`".to_string(), false));
    }
    out
}

/// How many public proxy ranges the app declares (`trustPublicProxy`).
pub fn public_proxies(root: &Path) -> usize {
    edge(root).map(|(edge, _)| edge.public_proxy.len()).unwrap_or(0)
}

const CACHE: &str = "server/cache.jsbrain";

fn cache(root: &Path, resources: &[Resource]) -> Result<CachePolicy, String> {
    let path = root.join(CACHE);
    if !path.exists() {
        return Ok(CachePolicy::default());
    }
    if !path.is_file() {
        return Err(format!("{CACHE}: must be a regular file"));
    }
    let value = crate::backend::read_contract(root, &path).map_err(|e| format!("{CACHE}: {e}"))?;
    CachePolicy::from_contract(&value, resources).map_err(|e| format!("{CACHE}: {e}"))
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

fn organizations(root: &Path) -> Result<brain_backend::organizations::Organizations, String> {
    use brain_backend::organizations::{CONTRACT, Organizations};
    let path = root.join(CONTRACT);
    if !path.exists() {
        return Ok(Organizations::default());
    }
    let value = crate::backend::read_contract(root, &path).map_err(|e| format!("{CONTRACT}: {e}"))?;
    Organizations::from_contract(&value)
}

fn captcha(root: &Path, environment: &str) -> Result<brain_backend::captcha::Captcha, String> {
    use brain_backend::captcha::{CONTRACT, Captcha};
    let path = root.join(CONTRACT);
    if !path.exists() {
        return Ok(Captcha::default());
    }
    let value = crate::backend::read_contract(root, &path).map_err(|e| format!("{CONTRACT}: {e}"))?;
    Captcha::from_contract(&value, |name| crate::env::value(root, environment, name))
}

/// B0790: the captcha secret an environment does not hold. A development server answers 503 without it; production refuses to start.
pub(crate) fn captcha_secret_problem(root: &Path, environment: &str) -> Option<(String, String, bool)> {
    use brain_backend::captcha::CONTRACT;
    let name = captcha(root, environment).ok()?.missing_secret()?.to_string();
    Some((CONTRACT.to_string(), format!("{CONTRACT}: {name} is not set, so every protected action fails while the captcha cannot be verified"), environment != "production"))
}

pub fn load(root: &Path, environment: &str, resources: &[Resource], roles: &RolesConfig) -> Result<Platform, String> {
    let (edge, cluster) = edge(root)?;
    Ok(Platform { buckets: buckets(root, resources, roles)?, edge, cluster, cache: cache(root, resources)?, organizations: organizations(root)?, captcha: captcha(root, environment)?, ..Default::default() })
}

/// Projection for `brain backend schema --json`; contains no environment values.
pub fn metadata(root: &Path, resources: &[Resource], roles: &RolesConfig) -> Result<Value, String> {
    let platform = load(root, "development", resources, roles)?;
    let edge = &platform.edge;
    Ok(json!({
        "files": platform.buckets.iter().map(Bucket::schema).collect::<Vec<_>>(),
        "captcha": platform.captcha.schema(),
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
        "cache": platform.cache.schema(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_captcha_secret_resolves_like_every_other_backend_setting() {
        let root = std::env::temp_dir().join(format!("brain-platform-captcha-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let contract = root.join(brain_backend::captcha::CONTRACT);
        fs::create_dir_all(contract.parent().unwrap()).unwrap();
        fs::write(&contract, "<meta server>\nexport default { verify_url: 'https://challenges.example.com/verify', secret_env: 'BRAIN_TEST_CAPTCHA_FILE_ONLY', protect: ['signup'] }\n").unwrap();
        let (file, message, disables) = captcha_secret_problem(&root, "development").expect("no secret anywhere");
        assert!(file.ends_with("jsbrain") && message.contains("BRAIN_TEST_CAPTCHA_FILE_ONLY") && disables, "{message}");
        assert!(!captcha_secret_problem(&root, "production").unwrap().2);
        fs::write(root.join(crate::env::FILE), "BRAIN_TEST_CAPTCHA_FILE_ONLY=from-the-file\n").unwrap();
        assert_eq!(captcha_secret_problem(&root, "development"), None);
        let _ = fs::remove_dir_all(&root);
    }
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
        for (entries, flagged) in [("trustProxy: ['127.0.0.1', '10.0.0.0/8', 'fd00::/8']", 0), ("trustProxy: ['127.0.0.1', '8.8.8.0/24']", 1), ("trustProxy: ['11.0.0.0/8']", 1), ("trustPublicProxy: ['8.8.8.0/24']", 0)] {
            fs::write(root.join(EDGE), format!("<meta server>\nexport default {{ {entries} }}\n")).unwrap();
            assert_eq!(proxy_findings(&root, None).iter().filter(|f| f.2).count(), flagged, "{entries}");
        }
        fs::write(root.join(EDGE), "<meta server>\nexport default { trustPublicProxy: ['10.0.0.0/8'] }\n").unwrap();
        assert!(metadata(&root, &[], &roles).unwrap_err().contains("private range"));
        fs::write(root.join(EDGE), "<meta server>\nexport default { trustProxy: ['0.0.0.0/0'] }\n").unwrap();
        let error = metadata(&root, &[], &roles).unwrap_err();
        assert!(error.contains("server/platform.jsbrain") && error.contains("every address"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }
}
