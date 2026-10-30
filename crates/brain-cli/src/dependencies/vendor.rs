//! Explicit HTTPS fetch only. No source is executed, and builds never enter this module.
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
const MAX_BYTES: usize = 1024 * 1024;
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
fn slot() -> Result<Slot, String> {
    ACTIVE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 4).then_some(n + 1)
        })
        .map(|_| Slot)
        .map_err(|_| "four dependency fetches are active; retry after one settles".into())
}
fn field<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))
}
fn request(args: &Value) -> Result<(String, String, String, String), String> {
    if !args.is_object()
        || args.as_object().unwrap().keys().any(|k| {
            !["url", "package", "version", "sha256", "expectedRevision"].contains(&k.as_str())
        })
        || args.get("expectedRevision").is_none()
    {
        return Err("deps fetch requires url, package, exact version, sha256 and explicit expectedRevision for brain.dependencies.json".into());
    }
    let url = field(args, "url")?;
    let package = field(args, "package")?;
    let version = field(args, "version")?;
    if !brain_pack::dependencies::pinned_cdn_url(url, package, version) {
        return Err("CDN fetch only accepts pinned immutable HTTPS files on cdn.jsdelivr.net, unpkg.com or esm.sh without redirects, query values or credentials".into());
    }
    let digest = field(args, "sha256")?.to_ascii_lowercase();
    if brain_pack::dependencies::artifact_file(&digest).is_none() {
        return Err("sha256 must contain exactly 64 hexadecimal digits".into());
    }
    Ok((url.into(), package.into(), version.into(), digest))
}
fn lock(root: &Path) -> Result<(Value, Value), String> {
    let read = crate::changes::read(root, brain_pack::dependencies::VENDOR_LOCK)?;
    let Some(source) = read["content"].as_str() else {
        return Ok((json!({"schemaVersion":1,"artifacts":[]}), Value::Null));
    };
    if source.len() > 262144 {
        return Err("vendor lock exceeds 256 KiB".into());
    }
    let value: Value = serde_json::from_str(source)
        .map_err(|_| "vendor lock is invalid JSON; preserve it and inspect locally")?;
    if !value.is_object() || value["schemaVersion"] != 1 {
        return Err("unsupported vendor lock schema; automatic replacement refused".into());
    }
    let entries = value["artifacts"]
        .as_array()
        .filter(|a| a.len() <= 64)
        .ok_or("vendor lock must have at most 64 artifacts")?;
    let mut urls = std::collections::BTreeSet::new();
    for entry in entries {
        let get = |key| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .ok_or("vendor artifact metadata is incomplete")
        };
        let url = get("url")?;
        let package = get("package")?;
        let version = get("version")?;
        let digest = get("sha256")?;
        let file = get("file")?;
        if !brain_pack::dependencies::pinned_cdn_url(url, package, version)
            || brain_pack::dependencies::artifact_file(digest).as_deref() != Some(file)
            || !urls.insert(url)
        {
            return Err("vendor lock artifact metadata is invalid or duplicated".into());
        }
    }
    Ok((value, read["revision"].clone()))
}
fn verify_body(bytes: &[u8], digest: &str) -> Result<(), String> {
    if bytes.len() > MAX_BYTES || crate::manifests::hash(bytes) != digest {
        return Err(
            "CDN artifact exceeds 1 MiB or differs from the requested SHA-256; nothing written"
                .into(),
        );
    }
    let source = std::str::from_utf8(bytes).map_err(|_| "CDN ESM artifact must be UTF-8")?;
    if !brain_check::scan_secrets(source).is_empty() {
        return Err("CDN source contains detected credential-looking literals; use a reviewed public adapter".into());
    }
    brain_pack::dependencies::standalone_esm(source)?;
    Ok(())
}
fn install(root: &Path, args: &Value, bytes: &[u8], cached: bool) -> Result<Value, String> {
    let (url, package, version, digest) = request(args)?;
    verify_body(bytes, &digest)?;
    let _writer = crate::edit::writer_lock(root)?;
    let (mut metadata, revision) = lock(root)?;
    if revision != args["expectedRevision"] {
        return Err(
            "vendor lock changed during fetch; inspect its current revision and retry".into(),
        );
    }
    let file = brain_pack::dependencies::artifact_file(&digest).unwrap();
    let existing = crate::changes::read(root, &file)?;
    if !existing["revision"].is_null() && existing["revision"] != digest {
        return Err(
            "content-addressed vendor path was modified; preserve and inspect it before retrying"
                .into(),
        );
    }
    let entries = metadata["artifacts"].as_array_mut().unwrap();
    let position = entries.iter().position(|entry| entry["url"] == url);
    if let Some(index) = position {
        if entries[index]["sha256"] != digest {
            return Err("immutable CDN URL already has a different digest; select a new explicit version rather than replacing its identity".into());
        }
    } else if entries.len() >= 64 {
        return Err("vendor lock has 64 artifacts; explicitly review and archive unused entries before fetching another".into());
    }
    let mut record = position
        .map(|i| entries[i].clone())
        .unwrap_or_else(|| json!({}));
    for (key, value) in [
        ("url", json!(url)),
        ("package", json!(package)),
        ("version", json!(version)),
        ("sha256", json!(digest)),
        ("file", json!(file)),
        ("bytes", json!(bytes.len())),
        ("format", json!("esm")),
        ("source", json!("explicit-pinned-cdn-fetch")),
    ] {
        record[key] = value;
    }
    if let Some(index) = position {
        entries[index] = record;
    } else {
        entries.push(record);
    }
    let encoded = format!(
        "{}\n",
        serde_json::to_string_pretty(&metadata).map_err(|_| "cannot serialize vendor lock")?
    );
    if encoded.len() > 262144 {
        return Err("vendor lock exceeds 256 KiB; no changes written".into());
    }
    let created = existing["revision"].is_null();
    if created {
        crate::changes::create_source(root, &file, std::str::from_utf8(bytes).unwrap())?;
    }
    let destination = crate::changes::source_path(root, brain_pack::dependencies::VENDOR_LOCK)?;
    let commit = crate::manifests::atomic(&destination, encoded.as_bytes());
    if commit.is_err() && created {
        if crate::changes::read(root, &file).is_ok_and(|read| read["revision"] == digest) {
            if let Ok(path) = crate::changes::source_path(root, &file) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    commit?;
    Ok(
        json!({"schemaVersion":1,"file":file,"url":url,"package":package,"version":version,"sha256":digest,"bytes":bytes.len(),"revision":crate::manifests::hash(encoded.as_bytes()),"cached":cached,"sameOriginOutput":true,"executedCode":false,"installedPackages":false,"published":false,"buildNetwork":false,"next":"Import the exact pinned URL in app source or its content-addressed local path. The compiler verifies locked bytes offline and emits only reachable code; review actual final build bytes and runtime behavior."}),
    )
}
/// One bounded read-only HTTP request, then cooperative writer/CAS installation of the reviewed artifact.
pub(super) fn fetch(root: &Path, args: &Value) -> Result<Value, String> {
    let _slot = slot()?;
    let (url, _, _, digest) = request(args)?;
    let (metadata, revision) = lock(root)?;
    if revision != args["expectedRevision"] {
        return Err("vendor lock revision is stale; inspect brain.dependencies.json first".into());
    }
    let file = brain_pack::dependencies::artifact_file(&digest).unwrap();
    let existing = crate::changes::read(root, &file)?;
    let known = metadata["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["url"] == url && entry["sha256"] == digest);
    if known && existing["revision"] == digest {
        return install(
            root,
            args,
            existing["content"]
                .as_str()
                .ok_or("cached artifact is unavailable")?
                .as_bytes(),
            true,
        );
    }
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err("explicit dependency fetch requires a blocking tool executor; use the CLI fetch operation".into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "cannot create bounded dependency fetch runtime")?;
    let body = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .user_agent(concat!("BRAINLLM/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| "cannot configure HTTPS dependency fetch")?;
        let mut response = client
            .get(&url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| "CDN HTTPS request failed; no artifact written")?;
        if !response.status().is_success() {
            return Err("CDN request was unsuccessful or redirected; no artifact written");
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BYTES as u64)
        {
            return Err("CDN artifact exceeds 1 MiB; no artifact written");
        }
        let media = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let media = media
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if ![
            "application/javascript",
            "text/javascript",
            "application/ecmascript",
            "text/ecmascript",
        ]
        .contains(&media.as_str())
        {
            return Err("CDN response is not a declared JavaScript media type");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "CDN body read failed; no artifact written")?
        {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err("CDN artifact exceeds 1 MiB; no artifact written");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    })?;
    install(root, args, &body, false)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct App(std::path::PathBuf);
    impl App {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "brain-vendor-cli-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn offline_install_checks_digest_cas_preserves_unknowns_and_builds_without_network() {
        let app = App::new();
        let body = b"/*! MIT test notice */ export function twice(n){return n*2}";
        let digest = crate::manifests::hash(body);
        let mut args = json!({"url":"https://unpkg.com/demo@1.0.0/index.mjs","package":"demo","version":"1.0.0","sha256":digest,"expectedRevision":null});
        let report = install(&app.0, &args, body, false).unwrap();
        assert_eq!(report["executedCode"], false);
        let mut resolver = brain_pack::dependencies::Resolver::new(&app.0).unwrap();
        let file = resolver
            .resolve("app.brain", args["url"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(resolver.read(&file).unwrap().contains("twice"));
        resolver.verify().unwrap();
        assert!(install(&app.0, &args, body, false).is_err());
        let mut metadata: Value = serde_json::from_str(
            crate::changes::read(&app.0, brain_pack::dependencies::VENDOR_LOCK).unwrap()["content"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        metadata["futureOption"] = json!({"preserved":true});
        metadata["artifacts"][0]["futureValue"] = json!("keep");
        let edited = serde_json::to_string(&metadata).unwrap();
        std::fs::write(app.0.join(brain_pack::dependencies::VENDOR_LOCK), &edited).unwrap();
        args["expectedRevision"] = json!(crate::manifests::hash(edited.as_bytes()));
        let next = install(&app.0, &args, body, true).unwrap();
        let saved: Value = serde_json::from_str(
            crate::changes::read(&app.0, brain_pack::dependencies::VENDOR_LOCK).unwrap()["content"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["futureOption"]["preserved"], true);
        assert_eq!(saved["artifacts"][0]["futureValue"], "keep");
        assert!(!next.to_string().contains("MIT test notice"));
        assert_eq!(next["cached"], true);
        assert!(install(&app.0, &args, b"different", false).is_err());
    }
    #[test]
    fn private_urls_and_unmanaged_graphs_are_rejected_before_install() {
        let app = App::new();
        let body = b"export {x} from './missing.mjs'";
        let args = json!({"url":"https://unpkg.com/demo@1.0.0/index.mjs","package":"demo","version":"1.0.0","sha256":crate::manifests::hash(body),"expectedRevision":null});
        assert!(install(&app.0, &args, body, false).is_err());
        assert!(!app.0.join("vendor").exists());
        let mut private = args.clone();
        private["url"] = json!("https://localhost/demo@1.0.0/index.mjs?key=private");
        assert!(request(&private).is_err());
    }
}
