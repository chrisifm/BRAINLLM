//! Offline, fail-closed permission for exact locally pinned CDN imports.
use brain_parse::{
    Json,
    ast::{Component, Stmt},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const LOCK: &str = "brain.dependencies.json";
const FILE_LIMIT: usize = 1_048_576;

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn remote(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://") || value.starts_with("//")
}
fn unique(value: &Json) -> bool {
    match value {
        Json::Obj(entries) => {
            let mut names = HashSet::new();
            entries
                .iter()
                .all(|(key, value)| names.insert(key) && unique(value))
        }
        Json::Arr(entries) => entries.iter().all(unique),
        _ => true,
    }
}
// Keep the immutable URL policy identical to brain-pack::dependencies::pinned_cdn_url.
fn pinned(url: &str, package: &str, version: &str) -> bool {
    let parts: Vec<_> = package.split('/').collect();
    let valid_name = !package.is_empty()
        && package.len() <= 240
        && package
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@/._-".contains(&b))
        && parts
            .iter()
            .all(|p| !p.is_empty() && *p != "." && *p != "..")
        && !package.starts_with('.')
        && !package.contains("@@")
        && if package.starts_with('@') {
            parts.len() == 2
        } else {
            parts.len() == 1
        };
    let core: Vec<_> = version
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .collect();
    let valid_version = !version.is_empty()
        && version.len() <= 80
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
        && core.len() == 3
        && core.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        });
    if !valid_name || !valid_version || url.len() > 1024 {
        return false;
    }
    let Some((host, path)) = url.strip_prefix("https://").and_then(|r| r.split_once('/')) else {
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
            .all(|p| !p.is_empty() && p != "." && p != "..")
        && file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-@+~".contains(&b))
}
fn artifact(digest: &str) -> Option<String> {
    (digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
    .then(|| format!("vendor/brain/{digest}.mjs"))
}
fn safe_relative(relative: &str) -> bool {
    !relative.is_empty()
        && relative.len() <= 2048
        && !relative.contains(['\\', ':', '?', '#'])
        && relative
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && part != "..")
}
/// Guarded bytes, never values in diagnostics. Root must already be canonical.
/// Callers still apply their own source-extension and secret-disclosure policies.
pub(crate) fn read(root: &Path, relative: &str, limit: usize) -> Option<Vec<u8>> {
    if !safe_relative(relative) {
        return None;
    }
    let limit = limit.min(8 * 1024 * 1024);
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        let meta = fs::symlink_metadata(&path).ok()?;
        if meta.file_type().is_symlink() {
            return None;
        }
    }
    let before = fs::symlink_metadata(&path).ok()?;
    if !before.is_file() || before.len() > limit as u64 {
        return None;
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(&path).ok()?;
    let opened = file.metadata().ok()?;
    if !opened.is_file() || opened.len() > limit as u64 {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return None;
        }
    }
    // Resolve again to reject parent-directory replacement/escape before reading.
    let actual = path.canonicalize().ok()?;
    if actual != root.join(relative) {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

// Watch a safe containing directory instead of allowing load_ctx's canonicalization
// to turn a rejected symlink into an external dependency read.
pub(crate) fn watched(root: &Path, relative: &str) -> PathBuf {
    if !safe_relative(relative) {
        return root.to_path_buf();
    }
    let mut parent = root.to_path_buf();
    for part in relative.split('/') {
        let next = parent.join(part);
        if fs::symlink_metadata(&next).is_ok_and(|m| m.file_type().is_symlink()) {
            return parent;
        }
        parent = next;
    }
    parent
}

pub(crate) fn load(root: &Path, c: &Component) -> (HashSet<String>, Vec<PathBuf>) {
    // Node server imports remain external; its loader does not rewrite CDN URLs.
    if c.meta.iter().any(|m| m.name == "server") {
        return (HashSet::new(), Vec::new());
    }
    let wanted: HashSet<_> = c
        .script
        .iter()
        .filter_map(|s| match s {
            Stmt::Import(i) if remote(&i.source) => Some(i.source.as_str()),
            _ => None,
        })
        .collect();
    let mut approved = HashSet::new();
    if wanted.is_empty() {
        return (approved, Vec::new());
    }
    let Some(root) = root.canonicalize().ok() else {
        return (approved, Vec::new());
    };
    let mut deps = vec![watched(&root, LOCK)];
    let Some(bytes) = read(&root, LOCK, 262_144) else {
        return (approved, deps);
    };
    let Some(lock) = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|s| Json::parse(s).ok())
    else {
        return (approved, deps);
    };
    if !unique(&lock) || lock.get("schemaVersion") != Some(&Json::Num(1.0)) {
        return (approved, deps);
    }
    let Some(Json::Arr(entries)) = lock.get("artifacts") else {
        return (approved, deps);
    };
    if entries.len() > 64 {
        return (approved, deps);
    }
    let mut urls = HashSet::new();
    let mut selected = Vec::new();
    // Validate metadata globally like pack; read/hash only URLs requested by this component.
    for entry in entries {
        let field = |key| entry.get(key).and_then(Json::as_str);
        let (Some(url), Some(package), Some(version), Some(digest), Some(file)) = (
            field("url"),
            field("package"),
            field("version"),
            field("sha256"),
            field("file"),
        ) else {
            return (approved, deps);
        };
        if !pinned(url, package, version)
            || artifact(digest).as_deref() != Some(file)
            || !urls.insert(url)
        {
            return (approved, deps);
        }
        if wanted.contains(url) {
            deps.push(watched(&root, file));
            selected.push((url, digest, file));
        }
    }
    let mut checked = std::collections::HashMap::new();
    let mut budget = 8 * FILE_LIMIT;
    for (url, digest, file) in selected {
        let valid = *checked.entry(file).or_insert_with(|| {
            if budget == 0 {
                return false;
            }
            let Some(bytes) = read(&root, file, FILE_LIMIT.min(budget)) else {
                return false;
            };
            budget -= bytes.len();
            std::str::from_utf8(&bytes).is_ok() && digest_hex(&bytes) == digest
        });
        if valid {
            approved.insert(url.to_owned());
        }
    }
    (approved, deps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (Fixture, Component, String) {
        let root = std::env::temp_dir().join(format!(
            "brain-cdn-check-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("vendor/brain")).unwrap();
        fs::write(root.join("brain.json"), "{}").unwrap();
        let body = "export const value=1;";
        let digest = digest_hex(body.as_bytes());
        let file = artifact(&digest).unwrap();
        fs::write(root.join(&file), body).unwrap();
        fs::write(root.join(LOCK),format!(r#"{{"schemaVersion":1,"artifacts":[{{"url":"https://unpkg.com/demo@1.0.0/index.mjs","package":"demo","version":"1.0.0","sha256":"{digest}","file":"{file}"}}]}}"#)).unwrap();
        let c = brain_parse::parse(
            "import {value} from 'https://unpkg.com/demo@1.0.0/index.mjs';",
            &root.join("test.jsbrain"),
        )
        .unwrap();
        (Fixture(root.canonicalize().unwrap()), c, file)
    }
    #[test]
    fn exact_local_hash_allows_only_the_requested_url_and_watches_files() {
        let (f, c, file) = fixture();
        let (ctx, deps) = crate::load_ctx(&f.0.join("test.jsbrain"), &c);
        assert_eq!(ctx.approved_remote_imports.len(), 1);
        assert!(deps.contains(&f.0.join(LOCK)) && deps.contains(&f.0.join(&file)));
        assert!(!crate::check(&c, &ctx).iter().any(|d| d.code == "B0122"));
        fs::write(f.0.join(file), "export const value=2;").unwrap();
        let (ctx, _) = crate::load_ctx(&f.0.join("test.jsbrain"), &c);
        assert!(crate::check(&c, &ctx).iter().any(|d| d.code == "B0122"));
    }
    #[test]
    fn approval_never_extends_to_another_url_or_reads_unused_lock() {
        let (f, _, _) = fixture();
        let c = brain_parse::parse(
            "import {value} from 'https://unpkg.com/demo@1.0.0/other.mjs';",
            &f.0.join("test.jsbrain"),
        )
        .unwrap();
        assert!(load(&f.0, &c).0.is_empty());
        let local = brain_parse::parse("export const value=1;", &f.0.join("test.jsbrain")).unwrap();
        let (approved, deps) = load(&f.0, &local);
        assert!(approved.is_empty() && deps.is_empty());
    }
    #[test]
    fn shared_reader_and_watch_guard_reject_private_or_escaping_segments() {
        let (f, _, _) = fixture();
        for path in [
            "../outside",
            "/outside",
            ".brain_env",
            "dir/../outside",
            "dir/./file",
            "dir\\outside",
            "dir/.private/file",
        ] {
            assert!(read(&f.0, path, 1024).is_none());
            assert_eq!(watched(&f.0, path), f.0);
        }
    }
    #[test]
    fn missing_artifact_and_invalid_lock_fail_closed() {
        let (f, c, file) = fixture();
        fs::remove_file(f.0.join(&file)).unwrap();
        assert!(load(&f.0, &c).0.is_empty());
        fs::write(
            f.0.join(LOCK),
            "{\"schemaVersion\":1,\"schemaVersion\":1,\"artifacts\":[]}",
        )
        .unwrap();
        assert!(load(&f.0, &c).0.is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_artifact_is_never_approved() {
        let (f, c, file) = fixture();
        let original = f.0.join("original.mjs");
        fs::rename(f.0.join(&file), &original).unwrap();
        std::os::unix::fs::symlink(original, f.0.join(&file)).unwrap();
        let (approved, deps) = load(&f.0, &c);
        assert!(approved.is_empty());
        assert!(!deps.contains(&f.0.join(&file)));
        assert!(deps.contains(&f.0.join("vendor/brain")));
    }
    #[test]
    fn url_policy_matches_pack_and_diagnostics_do_not_echo_private_url_values() {
        assert!(pinned(
            "https://cdn.jsdelivr.net/npm/@googlemaps/js-api-loader@2.1.3/dist/index.js",
            "@googlemaps/js-api-loader",
            "2.1.3"
        ));
        for url in [
            "https://unpkg.com/demo@latest/index.mjs",
            "https://unpkg.com/demo@1.0.0/index.mjs?token=private",
            "https://unpkg.com/demo@1.0.0/../private.mjs",
            "https://example.test/demo@1.0.0/index.mjs",
        ] {
            assert!(!pinned(url, "demo", "1.0.0"));
        }
        let c = brain_parse::parse(
            "import x from 'https://example.test/mod.js?token=private';",
            Path::new("test.jsbrain"),
        )
        .unwrap();
        let diags = crate::check(&c, &crate::Ctx::default());
        let d = diags.iter().find(|d| d.code == "B0122").unwrap();
        assert!(!d.msg.contains("private"));
    }
}
