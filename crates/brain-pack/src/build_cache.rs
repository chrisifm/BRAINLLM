//! Owned bounded build cache. Corrupt, oversized or unsafe entries are misses, never compiler output.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
const MAX: usize = 16 * 1024 * 1024;
const ENTRIES: usize = 512;
const TOTAL: u64 = 128 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"BRAINBC2";
static NEXT: AtomicU64 = AtomicU64::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static CORRUPT: AtomicU64 = AtomicU64::new(0);
static WRITES: AtomicU64 = AtomicU64::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);
static EVICTED: AtomicU64 = AtomicU64::new(0);
static ERRORS: AtomicU64 = AtomicU64::new(0);
static UNSAFE: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub hits: u64,
    pub misses: u64,
    pub corrupt: u64,
    pub writes: u64,
    pub skipped: u64,
    pub evicted: u64,
    pub errors: u64,
    pub unsafe_paths: u64,
}
pub fn stats() -> Stats {
    Stats {
        hits: HITS.load(Ordering::Relaxed),
        misses: MISSES.load(Ordering::Relaxed),
        corrupt: CORRUPT.load(Ordering::Relaxed),
        writes: WRITES.load(Ordering::Relaxed),
        skipped: SKIPPED.load(Ordering::Relaxed),
        evicted: EVICTED.load(Ordering::Relaxed),
        errors: ERRORS.load(Ordering::Relaxed),
        unsafe_paths: UNSAFE.load(Ordering::Relaxed),
    }
}
impl Stats {
    pub fn since(self, before: Self) -> Self {
        Self {
            hits: self.hits.saturating_sub(before.hits),
            misses: self.misses.saturating_sub(before.misses),
            corrupt: self.corrupt.saturating_sub(before.corrupt),
            writes: self.writes.saturating_sub(before.writes),
            skipped: self.skipped.saturating_sub(before.skipped),
            evicted: self.evicted.saturating_sub(before.evicted),
            errors: self.errors.saturating_sub(before.errors),
            unsafe_paths: self.unsafe_paths.saturating_sub(before.unsafe_paths),
        }
    }
    pub fn to_json(self) -> String {
        format!(
            "{{\"scope\":\"process observations during build; concurrent activity may contribute\",\"hits\":{},\"misses\":{},\"corruptRecomputed\":{},\"writes\":{},\"writeSkipped\":{},\"evicted\":{},\"computeErrors\":{},\"unsafePathsBypassed\":{}}}",
            self.hits,
            self.misses,
            self.corrupt,
            self.writes,
            self.skipped,
            self.evicted,
            self.errors,
            self.unsafe_paths
        )
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn key(kind: &str, input: &[u8], fingerprint: &str) -> String {
    let mut identity = Vec::new();
    for bytes in [kind.as_bytes(), fingerprint.as_bytes()] {
        identity.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        identity.extend_from_slice(bytes);
    }
    identity.extend_from_slice(&crate::sha::sha256(input));
    format!("v2-{}", hex(&crate::sha::sha256(&identity)))
}
fn directory(root: &Path, kind: &str) -> Option<PathBuf> {
    if kind.is_empty()
        || kind.len() > 128
        || !kind
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || matches!(kind, "." | "..")
    {
        return None;
    }
    let mut path = root.canonicalize().ok()?;
    for part in [".brain", "cache", kind] {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
            Ok(_) => return None,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => return None,
                }
                let m = fs::symlink_metadata(&path).ok()?;
                if !m.is_dir() || m.file_type().is_symlink() {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
    Some(path)
}
fn read(path: &Path) -> Option<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX as u64 + 48
    {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX as u64 + 49)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() < 48 || &bytes[..8] != MAGIC {
        return None;
    }
    let length = u64::from_be_bytes(bytes[8..16].try_into().ok()?);
    if length > MAX as u64
        || bytes.len() != length as usize + 48
        || crate::sha::sha256(&bytes[48..]) != bytes[16..48]
    {
        return None;
    }
    Some(bytes.split_off(48))
}
fn owned_name(name: &str) -> bool {
    name.len() == 67
        && name.starts_with("v2-")
        && name[3..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn reserve(dir: &Path, incoming: &Path, size: u64, entries: usize, total: u64) -> bool {
    let Ok(read) = fs::read_dir(dir) else {
        return false;
    };
    let mut files = Vec::new();
    let mut used = 0u64;
    for (index, entry) in read.enumerate() {
        if index >= entries.saturating_mul(4).max(64) {
            return false;
        }
        let Ok(entry) = entry else { return false };
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !owned_name(&name) {
            continue;
        }
        let Ok(m) = fs::symlink_metadata(&path) else {
            return false;
        };
        if !m.is_file() || m.file_type().is_symlink() {
            continue;
        }
        if path == incoming {
            if fs::remove_file(&path).is_err() {
                return false;
            }
            continue;
        }
        used = used.saturating_add(m.len());
        files.push((m.modified().ok(), path, m.len()));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut count = files.len();
    for (_, path, len) in files {
        if count < entries && used.saturating_add(size) <= total {
            break;
        }
        if fs::remove_file(path).is_err() {
            return false;
        }
        EVICTED.fetch_add(1, Ordering::Relaxed);
        count -= 1;
        used = used.saturating_sub(len);
    }
    count < entries && used.saturating_add(size) <= total
}
fn store(dir: &Path, path: &Path, output: &[u8]) -> bool {
    if output.len() > MAX {
        return false;
    }
    let lock_path = dir.join(".write.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return false;
    }
    let Ok(lock) = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
    else {
        return false;
    };
    let Ok(_guard) = lock.try_lock() else {
        return false;
    };
    let size = 48 + output.len() as u64;
    if !reserve(dir, path, size, ENTRIES, TOTAL) {
        return false;
    }
    let temp = dir.join(format!(
        ".owned-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let operation = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(MAGIC)?;
        file.write_all(&(output.len() as u64).to_be_bytes())?;
        file.write_all(&crate::sha::sha256(output))?;
        file.write_all(output)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if operation.is_err() {
        let _ = fs::remove_file(&temp);
    }
    operation.is_ok()
}
pub(crate) fn get(
    root: &Path,
    kind: &str,
    input: &[u8],
    fingerprint: &str,
    compute: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    if input.len() > 32 * 1024 * 1024 {
        SKIPPED.fetch_add(1, Ordering::Relaxed);
        return compute();
    }
    let location = directory(root, kind).map(|dir| {
        let path = dir.join(key(kind, input, fingerprint));
        (dir, path)
    });
    if let Some((_, path)) = &location {
        if let Some(bytes) = read(path) {
            HITS.fetch_add(1, Ordering::Relaxed);
            return Ok(bytes);
        }
    }
    if let Some((_, path)) = &location {
        if fs::symlink_metadata(path).is_ok() {
            CORRUPT.fetch_add(1, Ordering::Relaxed);
        }
    } else {
        UNSAFE.fetch_add(1, Ordering::Relaxed);
    }
    MISSES.fetch_add(1, Ordering::Relaxed);
    let output = match compute() {
        Ok(value) => value,
        Err(error) => {
            ERRORS.fetch_add(1, Ordering::Relaxed);
            return Err(error);
        }
    };
    if let Some((dir, path)) = location {
        if store(&dir, &path, &output) {
            WRITES.fetch_add(1, Ordering::Relaxed);
        } else {
            SKIPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn root() -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "brain-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Scratch(p)
    }
    #[test]
    fn key_changes_with_input_kind_and_toolchain_and_corruption_is_a_miss() {
        let root = root();
        let p = &root.0;
        assert_eq!(
            get(p, "min", b"input", "v1", || Ok(b"result".to_vec())).unwrap(),
            b"result"
        );
        assert_eq!(
            get(p, "min", b"input", "v1", || panic!("cache should hit")).unwrap(),
            b"result"
        );
        assert_eq!(
            get(p, "min", b"input", "v2", || Ok(b"new compiler".to_vec())).unwrap(),
            b"new compiler"
        );
        assert_ne!(key("a", b"x", "v"), key("b", b"x", "v"));
        assert_ne!(key("a", b"x", "v"), key("a", b"y", "v"));
        let file = p.join(".brain/cache/min").join(key("min", b"input", "v1"));
        let mut data = fs::read(&file).unwrap();
        *data.last_mut().unwrap() ^= 1;
        fs::write(file, data).unwrap();
        assert_eq!(
            get(p, "min", b"input", "v1", || Ok(b"recomputed".to_vec())).unwrap(),
            b"recomputed"
        );
    }
    #[test]
    fn failure_is_not_cached_and_budget_pruning_preserves_foreign_files() {
        let root = root();
        assert!(get(&root.0, "min", b"x", "v", || Err("failed".into())).is_err());
        let dir = directory(&root.0, "min").unwrap();
        fs::write(dir.join("foreign.txt"), b"retain").unwrap();
        for n in 0..3 {
            fs::write(dir.join(key("min", &[n], "v")), [0u8; 8]).unwrap();
        }
        assert!(reserve(
            &dir,
            &dir.join(key("min", b"incoming", "v")),
            8,
            2,
            16
        ));
        assert!(dir.join("foreign.txt").exists());
        assert_eq!(
            fs::read_dir(dir)
                .unwrap()
                .flatten()
                .filter(|e| owned_name(&e.file_name().to_string_lossy()))
                .count(),
            1
        );
    }
    #[cfg(unix)]
    #[test]
    fn cache_symlinks_are_bypassed_without_writing_external_state() {
        let root = root();
        let outside = root.0.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::create_dir(root.0.join(".brain")).unwrap();
        std::os::unix::fs::symlink(&outside, root.0.join(".brain/cache")).unwrap();
        assert_eq!(
            get(&root.0, "min", b"x", "v", || Ok(b"safe".to_vec())).unwrap(),
            b"safe"
        );
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}
