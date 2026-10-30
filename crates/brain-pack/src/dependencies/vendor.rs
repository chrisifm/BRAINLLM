//! Explicit pinned CDN artifact locks. Builds only read already fetched bytes.
use super::*;
pub const VENDOR_LOCK: &str = "brain.dependencies.json";
pub(super) fn is_vendor(file: &str) -> bool {
    file.starts_with("vendor/brain/")
}
pub fn artifact_file(digest: &str) -> Option<String> {
    (digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
    .then(|| format!("vendor/brain/{digest}.mjs"))
}
/// Canonical immutable file URLs only: no redirects, user info, ports, query values or encoded path segments.
pub fn pinned_cdn_url(url: &str, package: &str, version: &str) -> bool {
    if package_name(package).is_err()
        || package_name(package).is_ok_and(|(name, sub)| name != package || !sub.is_empty())
        || !exact_version(version)
        || url.len() > 1024
    {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    let prefix = match host {
        "cdn.jsdelivr.net" => format!("npm/{package}@{version}/"),
        "unpkg.com" | "esm.sh" => format!("{package}@{version}/"),
        _ => return false,
    };
    let Some(file) = path.strip_prefix(&prefix) else {
        return false;
    };
    !file.is_empty()
        && (file.ends_with(".js") || file.ends_with(".mjs"))
        && file
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-@+~".contains(&b))
}
/// CDN single-file artifacts must not hide a second unresolved module/resource graph.
pub fn standalone_esm(source: &str) -> Result<String, String> {
    let prepared = prepare(source)?;
    if !crate::specifiers(&prepared)?.is_empty() {
        return Err(error(
            "CDN artifact has module imports; use the installed npm graph or an explicitly declared complete vendor graph",
        ));
    }
    if crate::url_refs(&prepared)?
        .iter()
        .any(|(_, _, path)| ![".", "..", "./", "../"].contains(&path.as_str()))
    {
        return Err(error(
            "CDN artifact has URL resources; use the installed npm asset graph",
        ));
    }
    Ok(prepared)
}
impl Resolver {
    fn load_vendor(&mut self) -> Result<(), String> {
        if self.vendor.is_some() {
            return Ok(());
        }
        let bytes=self.read_bytes(VENDOR_LOCK,MANIFEST_BYTES).map_err(|_|error("CDN import has no readable vendor lock; explicitly fetch the exact file/version/SHA-256 first"))?;
        let lock = Json::parse(
            std::str::from_utf8(&bytes).map_err(|_| error("vendor lock must be UTF-8"))?,
        )
        .map_err(|_| error("vendor lock is invalid JSON"))?;
        if !unique_json(&lock) || lock.get("schemaVersion") != Some(&Json::Num(1.0)) {
            return Err(error("unsupported vendor lock schema or duplicate fields"));
        }
        let Some(Json::Arr(entries)) = lock.get("artifacts") else {
            return Err(error("vendor lock needs an artifact list"));
        };
        if entries.len() > 64 {
            return Err(error("vendor lock exceeds 64 artifacts"));
        }
        let mut urls = HashSet::new();
        for entry in entries {
            let get = |key| {
                entry
                    .get(key)
                    .and_then(Json::as_str)
                    .ok_or_else(|| error("vendor artifact metadata is incomplete"))
            };
            let url = get("url")?;
            let package = get("package")?;
            let version = get("version")?;
            let digest = get("sha256")?;
            let file = get("file")?;
            if !pinned_cdn_url(url, package, version)
                || artifact_file(digest).as_deref() != Some(file)
                || !urls.insert(url)
            {
                return Err(error(
                    "vendor artifact URL/pin/path/digest is invalid or duplicated",
                ));
            }
        }
        self.vendor_revision = Some(hash(&bytes));
        self.vendor = Some(lock);
        Ok(())
    }
    pub(super) fn resolve_vendor(&mut self, url: &str) -> Result<String, String> {
        if self.bytes == 0 {
            self.start = Instant::now();
        }
        self.load_vendor()?;
        let Some(Json::Arr(entries)) = self.vendor.as_ref().unwrap().get("artifacts") else {
            unreachable!()
        };
        entries
            .iter()
            .find(|entry| entry.get("url").and_then(Json::as_str) == Some(url))
            .and_then(|entry| entry.get("file"))
            .and_then(Json::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                error(
                    "CDN URL is not pinned in the local artifact lock; no build network is allowed",
                )
            })
    }
    pub(super) fn read_vendor(&mut self, file: &str) -> Result<String, String> {
        self.load_vendor()?;
        if self.files.len() >= MAX_FILES && !self.files.contains_key(file) {
            return Err(error("dependency graph exceeds 512 files"));
        }
        let Some(Json::Arr(entries)) = self.vendor.as_ref().unwrap().get("artifacts") else {
            unreachable!()
        };
        let digest = entries
            .iter()
            .find(|entry| entry.get("file").and_then(Json::as_str) == Some(file))
            .and_then(|entry| entry.get("sha256"))
            .and_then(Json::as_str)
            .ok_or_else(|| error("vendor source is not owned by the artifact lock"))?
            .to_owned();
        let bytes = self.read_bytes(file, FILE_BYTES)?;
        if hash(&bytes) != digest {
            return Err(error(
                "vendored CDN bytes differ from the locked SHA-256; refetch the reviewed immutable artifact",
            ));
        }
        let source =
            std::str::from_utf8(&bytes).map_err(|_| error("vendored JavaScript must be UTF-8"))?;
        let prepared = standalone_esm(source)?;
        self.files.insert(file.to_owned(), (digest, bytes.len()));
        Ok(prepared)
    }
    pub(super) fn verify_vendor(&mut self) -> Result<(), String> {
        if let Some(expected) = self.vendor_revision.clone() {
            if hash(&self.read_bytes(VENDOR_LOCK, MANIFEST_BYTES)?) != expected {
                return Err(error("vendor lock changed during the build snapshot"));
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_cdn_urls_reject_mutable_or_private_destinations() {
        assert!(pinned_cdn_url(
            "https://cdn.jsdelivr.net/npm/@googlemaps/js-api-loader@2.1.3/dist/index.js",
            "@googlemaps/js-api-loader",
            "2.1.3"
        ));
        for url in [
            "https://unpkg.com/demo@latest/index.mjs",
            "http://unpkg.com/demo@1.0.0/index.mjs",
            "https://unpkg.com@localhost/demo@1.0.0/index.mjs",
            "https://unpkg.com/demo@1.0.0/index.mjs?token=x",
            "https://unpkg.com/demo@1.0.0/../private.mjs",
            "https://example.test/demo@1.0.0/index.mjs",
        ] {
            assert!(!pinned_cdn_url(url, "demo", "1.0.0"));
        }
    }
    #[test]
    fn cdn_artifact_requires_a_declared_complete_graph() {
        assert!(standalone_esm("export const value=1").is_ok());
        assert!(standalone_esm("export {x} from './other.mjs'").is_err());
        assert!(standalone_esm("export const x=new URL('./module.wasm',import.meta.url)").is_err());
    }
}
