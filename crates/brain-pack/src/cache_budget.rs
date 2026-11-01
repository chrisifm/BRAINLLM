//! Amortized cache maintenance: a directory is scanned when half of the room proven by the last scan has been used, so
//! N writes cost O(log N) scans instead of N. Halving also leaves room for one concurrent writer we cannot see.
//! Eviction removes the least recently used entries down to a low-water mark, and an entry used by the running build is
//! never evicted: a build larger than the cache keeps a stable subset instead of evicting its own earlier results.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

#[derive(Default)]
struct Room {
    writes: usize,
    bytes: u64,
    full_for: Option<u64>,
}
static ROOM: OnceLock<Mutex<HashMap<PathBuf, Room>>> = OnceLock::new();

/// Everything written or used at or after this instant belongs to the running build. Outside a build nothing is protected.
fn protected_since() -> Option<SystemTime> {
    brain_metrics::current().map(|build| build.started_at())
}

/// Marks an entry as used by this build, so eviction order follows use and not the first write.
pub fn touch(path: &Path, modified: Option<SystemTime>) {
    let Some(since) = protected_since() else { return };
    if modified.is_some_and(|m| m >= since) {
        return;
    }
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Whether one more entry of `size` bytes fits. `scan` runs only when the known room is used up; it returns the
/// (count, bytes) left in the directory after evicting, or `None` when the directory could not be inspected.
pub fn reserve(dir: &Path, size: u64, max_entries: usize, max_bytes: u64, scan: impl FnOnce() -> Option<(usize, u64)>) -> bool {
    let mut all = ROOM.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    let room = all.entry(dir.to_path_buf()).or_default();
    let build = brain_metrics::current().map(|b| b.id());
    if build.is_some() && room.full_for == build {
        return false;
    }
    if room.writes > 0 && room.bytes >= size {
        room.writes -= 1;
        room.bytes -= size;
        return true;
    }
    let Some((count, used)) = scan() else { return false };
    if count >= max_entries || used.saturating_add(size) > max_bytes {
        room.writes = 0;
        room.bytes = 0;
        room.full_for = build;
        return false;
    }
    room.writes = (max_entries - count - 1) / 2;
    room.bytes = (max_bytes - used - size) / 2;
    room.full_for = None;
    true
}

/// Evicts the oldest evictable entries until `slots` more entries of `size` total bytes fit under the low-water mark.
/// Returns what is left. Small limits stay exact; a protected entry is skipped, even if that leaves no room.
pub fn evict(mut files: Vec<(Option<SystemTime>, PathBuf, u64)>, mut used: u64, slots: usize, size: u64, max_entries: usize, max_bytes: u64) -> (usize, u64) {
    let mut count = files.len();
    if count + slots <= max_entries && used.saturating_add(size) <= max_bytes {
        return (count, used);
    }
    let (low_entries, low_bytes) = if max_entries >= 16 { (max_entries - max_entries / 8, max_bytes - max_bytes / 8) } else { (max_entries, max_bytes) };
    let since = protected_since();
    files.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    for (modified, path, len) in files {
        if count + slots <= low_entries && used.saturating_add(size) <= low_bytes {
            break;
        }
        if since.is_some_and(|s| modified.is_some_and(|m| m >= s)) {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            brain_metrics::count(brain_metrics::Counter::CacheEvictions, 1);
            count -= 1;
            used = used.saturating_sub(len);
        }
    }
    (count, used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brain-budget-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scans_grow_logarithmically_and_the_limit_holds() {
        let dir = scratch("log");
        let (scans, entries) = (Cell::new(0), Cell::new(0usize));
        for _ in 0..1024 {
            let fits = reserve(&dir, 1, 1024, u64::MAX / 2, || {
                scans.set(scans.get() + 1);
                Some((entries.get(), entries.get() as u64))
            });
            if fits {
                entries.set(entries.get() + 1);
            }
        }
        assert_eq!(entries.get(), 1024, "fills the cache exactly");
        assert!(scans.get() <= 12, "{} scans for 1024 writes", scans.get());
        assert!(!reserve(&dir, 1, 1024, u64::MAX / 2, || Some((1024, 1024))), "a full directory refuses");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_byte_limit_is_never_exceeded() {
        let dir = scratch("bytes");
        let used = Cell::new(0u64);
        for _ in 0..100 {
            if reserve(&dir, 10, 1000, 95, || Some((0, used.get()))) {
                used.set(used.get() + 10);
            }
        }
        assert!(used.get() <= 95, "{}", used.get());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn entries_of_the_running_build_survive_eviction() {
        let dir = scratch("protected");
        let mut files = Vec::new();
        for n in 0..4 {
            let p = dir.join(format!("{n}"));
            std::fs::write(&p, b"x").unwrap();
            files.push((Some(SystemTime::UNIX_EPOCH), p, 1));
        }
        let build = brain_metrics::Build::new();
        let _observed = brain_metrics::attach(&build);
        let fresh = dir.join("fresh");
        std::fs::write(&fresh, b"x").unwrap();
        files.push((std::fs::metadata(&fresh).unwrap().modified().ok(), fresh.clone(), 1));
        let (count, _) = evict(files, 5, 1, 1, 2, 100);
        assert!(fresh.exists(), "written by this build");
        assert_eq!(count, 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_full_directory_is_not_rescanned_during_one_build() {
        let dir = scratch("full");
        let build = brain_metrics::Build::new();
        let _observed = brain_metrics::attach(&build);
        let scans = Cell::new(0);
        for _ in 0..50 {
            assert!(!reserve(&dir, 1, 4, 100, || {
                scans.set(scans.get() + 1);
                Some((4, 4))
            }));
        }
        assert_eq!(scans.get(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
