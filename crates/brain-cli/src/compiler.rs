//! Compiler facade: `check` (brain-parse + brain-check) for `brain check` and `compile`
//! (brain-codegen) for dev, test and build.

pub use brain_parse::Diag;
use std::io::Read;
use std::path::{Path, PathBuf};

const CACHE_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
const CACHE_BYTES: u64 = 128 * 1024 * 1024;
const CACHE_ENTRIES: usize = 1024;
static CACHE_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub struct Output {
    pub js: String,
    pub css: String,
    pub deps: Vec<PathBuf>,
    /// `load` + `server:` functions (INTERNALS §11.1).
    pub server_js: String,
    /// Names of the `server:` functions (RPC endpoints).
    pub server_fns: Vec<String>,
}

/// Parse + context (tokens, props of imports) + check. `rel` goes into `Diag.file`; `disk` is the real path.
/// Returns every Diag (errors and warnings) and the dependencies read, for the watcher.
pub fn check(src: &str, rel: &Path, disk: &Path) -> (Vec<Diag>, Vec<PathBuf>) {
    // A .cssbrain (tokens or a shared stylesheet) is plain CSS, not a component: only its syntax is checked here.
    if rel.extension().is_some_and(|e| e == "cssbrain") {
        let file = rel.to_string_lossy().replace('\\', "/");
        return (
            brain_css::parse_str(src, &file).err().into_iter().collect(),
            Vec::new(),
        );
    }
    match brain_parse::parse(src, rel) {
        Err(ds) => (ds, Vec::new()),
        Ok(comp) => {
            let (ctx, deps) = brain_check::load_ctx(disk, &comp);
            (brain_check::check(&comp, &ctx), deps)
        }
    }
}

/// Compiles to JS + CSS with brain-codegen (which also runs the checker). `root` = app root, to read
/// tokens and imports; `dev` = output with `sig(v,'name')` and `dev(Comp)` for HMR (SPEC §6.2).
pub fn compile(src: &str, rel: &Path, root: &Path, dev: bool) -> Result<Output, Vec<Diag>> {
    compile_hash(src, rel, root, dev, 6)
}

/// `compile` with an explicit component-hash length (production uses 4, 5 for a file whose 4 collide).
pub fn compile_hash(
    src: &str,
    rel: &Path,
    root: &Path,
    dev: bool,
    hash_len: usize,
) -> Result<Output, Vec<Diag>> {
    // brain.json "lint" (INTERNALS §20): codegen applies it to the quality families before deciding what blocks
    let lint: Vec<(String, Option<brain_parse::Severity>)> = crate::lint_rules(root)
        .unwrap_or_default()
        .into_iter()
        .map(|(c, l)| {
            let lvl = match l.as_str() {
                "off" => None,
                "info" => Some(brain_parse::Severity::Info),
                "warning" => Some(brain_parse::Severity::Warning),
                _ => Some(brain_parse::Severity::Error),
            };
            (c, lvl)
        })
        .collect();
    let opts = brain_codegen::Options {
        dev,
        hash_len,
        root: Some(root.to_path_buf()),
        public_env: crate::env::public(),
        lint,
        ..Default::default()
    };
    // Persistent, content-addressed cache (INTERNALS §28): the key covers the file's blocks, the compiler version
    // and every option; each dependency (imported file, tokens, inlined ui/) is stored with its content hash and
    // re-validated on a hit. A save without changes costs one hash.
    let key = brain_codegen::blocks::source_cache_key(src, rel, &opts);
    // `-2`: entry format with server_fns (older entries lack it and are recomputed)
    let entry = root.join(".brain/cache").join(format!("{key}-2.json"));
    if let Some(o) = read_cache(&entry) {
        return Ok(o);
    }
    let o = brain_codegen::compile_with(src, rel, &opts)?;
    let out = Output {
        js: o.js,
        css: o.css,
        deps: o.deps,
        server_js: o.server_js,
        server_fns: o.server_fns,
    };
    write_cache(&entry, &out);
    Ok(out)
}

/// FNV-1a 64 of a file's bytes (`None` = unreadable/missing).
fn file_hash(p: &Path) -> Option<String> {
    let meta = match std::fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some("missing".into()),
        Err(_) => return None,
    };
    if meta.file_type().is_symlink() {
        return None;
    }
    if meta.is_dir() {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(p).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "dist" | "target")
            {
                continue;
            }
            let kind = entry.file_type().ok()?;
            if kind.is_dir()
                || entry
                    .path()
                    .extension()
                    .is_some_and(|x| x == "brain" || x == "jsbrain")
            {
                names.push(format!("{}:{}:{}:{name}", kind.is_dir(), kind.is_file(), kind.is_symlink()));
            }
        }
        names.sort();
        return Some(brain_codegen::blocks::hash(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
    }
    if !meta.is_file() || meta.len() > 128 * 1024 * 1024 {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(p).ok()?;
    let opened = file.metadata().ok()?;
    if !opened.is_file() || opened.len() > 128 * 1024 * 1024 {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.dev() != opened.dev() || meta.ino() != opened.ino() {
            return None;
        }
    }
    let started = std::time::Instant::now();
    let mut read = 0u64;
    let mut h: u64 = 0xcbf29ce484222325;
    let mut bytes = [0u8; 64 * 1024];
    loop {
        if read > 128 * 1024 * 1024 || started.elapsed() > std::time::Duration::from_secs(2) {
            return None;
        }
        let n = file.read(&mut bytes).ok()?;
        read += n as u64;
        if n == 0 {
            break;
        }
        for &b in &bytes[..n] {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    let after = file.metadata().ok()?;
    if read != opened.len()
        || after.len() != opened.len()
        || after.modified().ok() != opened.modified().ok()
    {
        return None;
    }
    Some(format!("{h:016x}"))
}

/// The HMR fast path must validate the same context as the disk cache before reusing unchanged source blocks.
pub(crate) fn dependency_hashes(deps: &[PathBuf]) -> Option<Vec<String>> {
    deps.iter().map(|p| file_hash(p)).collect()
}

fn read_cache(entry: &Path) -> Option<Output> {
    use brain_mcp::Json;
    let file = std::fs::File::open(entry).ok()?;
    if file.metadata().ok()?.len() > CACHE_ENTRY_BYTES {
        return None;
    }
    let mut text = String::new();
    file.take(CACHE_ENTRY_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > CACHE_ENTRY_BYTES {
        return None;
    }
    let j = Json::parse(&text).ok()?;
    let s = |k: &str| j.get(k).and_then(Json::as_str).map(str::to_string);
    let mut deps = Vec::new();
    if let Some(Json::Arr(ds)) = j.get("deps") {
        for d in ds {
            let p = PathBuf::from(d.get("path")?.as_str()?);
            // a dependency changed (or disappeared): the cached output is stale
            if d.get("hash").and_then(Json::as_str).is_none()
                || file_hash(&p).as_deref() != d.get("hash").and_then(Json::as_str)
            {
                return None;
            }
            deps.push(p);
        }
    } else {
        return None;
    }
    let server_fns = s("server_fns")
        .map(|x| {
            x.split(',')
                .filter(|f| !f.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some(Output {
        js: s("js")?,
        css: s("css")?,
        server_js: s("server_js").unwrap_or_default(),
        server_fns,
        deps,
    })
}

fn write_cache(entry: &Path, o: &Output) {
    use brain_mcp::Json;
    let deps = o
        .deps
        .iter()
        .map(|p| {
            Json::obj([
                ("path", Json::str(p.to_string_lossy())),
                ("hash", file_hash(p).map_or(Json::Null, Json::Str)),
            ])
        })
        .collect();
    let j = Json::obj([
        ("js", Json::str(&o.js)),
        ("css", Json::str(&o.css)),
        ("server_js", Json::str(&o.server_js)),
        ("server_fns", Json::str(o.server_fns.join(","))),
        ("deps", Json::Arr(deps)),
    ]);
    if let Some(dir) = entry.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // write-then-rename: a reader never sees half an entry
    let nonce = CACHE_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = entry.with_extension(format!("tmp{}-{nonce}", std::process::id()));
    let bytes = j.stringify();
    if bytes.len() as u64 > CACHE_ENTRY_BYTES {
        return;
    }
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, entry);
    }
    let _ = std::fs::remove_file(&tmp);
    if let Some(dir) = entry.parent() {
        prune_cache(dir, CACHE_BYTES, CACHE_ENTRIES);
    }
}

/// Oldest-write eviction; only complete entries in this project's compiler cache are eligible.
fn prune_cache(dir: &Path, max_bytes: u64, max_entries: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files = Vec::new();
    let mut bytes = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with("-2.json"))
        {
            continue;
        }
        let Ok(meta) = entry.file_type().and_then(|kind| {
            if kind.is_file() {
                entry.metadata()
            } else {
                Err(std::io::Error::other("not a cache file"))
            }
        }) else {
            continue;
        };
        bytes = bytes.saturating_add(meta.len());
        files.push((
            meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            path,
            meta.len(),
        ));
    }
    if bytes <= max_bytes && files.len() <= max_entries {
        return;
    }
    files.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut count = files.len();
    for (_, path, size) in files {
        if bytes <= max_bytes && count <= max_entries {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            bytes = bytes.saturating_sub(size);
            count -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codegen_dev_output() {
        let src = "<script>\n  let n = sig(0)\n</script>\n<button on:click={n++}>{n}</button>\n<style>button{color:var(--accent)}</style>";
        let out = compile(
            src,
            Path::new("comp/x.brain"),
            Path::new("/nonexistent"),
            true,
        )
        .ok()
        .unwrap();
        assert!(
            out.js.contains("/@brain/brain-dev.js") && out.js.contains("\"n\""),
            "{}",
            out.js
        );
        assert!(out.css.contains("@scope"));
    }

    #[test]
    fn persistent_cache_hits_and_invalidates_on_dependency_change() {
        let root = std::env::temp_dir().join(format!("brain-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ui")).unwrap();
        std::fs::create_dir_all(root.join("comp")).unwrap();
        std::fs::write(
            root.join("tokens.cssbrain"),
            ":root{--accent:#0062c4;--space-2:8px}",
        )
        .unwrap();
        std::fs::write(
            root.join("ui/chip.brain"),
            "<meta props=\"label:str\">\n<span class=\"chip\">{label}</span>",
        )
        .unwrap();
        let src = "<script>\n  import Chip from '../ui/chip.brain'\n</script>\n<p>x <Chip label=\"a\" /></p>\n<style>p{color:var(--accent)}</style>";
        let rel = Path::new("comp/x.brain");
        let a = compile_hash(src, rel, &root, true, 6)
            .unwrap_or_else(|d| panic!("{}", Diag::list_to_json(&d)));
        let entries = || {
            std::fs::read_dir(root.join(".brain/cache"))
                .map(|d| d.count())
                .unwrap_or(0)
        };
        assert_eq!(entries(), 1);
        let b = compile_hash(src, rel, &root, true, 6).ok().unwrap();
        assert_eq!(a.js, b.js);
        assert_eq!(entries(), 1, "same content → cache hit, no new entry");
        // a dependency changes: the entry must be revalidated and recompiled (output may stay equal)
        std::fs::write(
            root.join("tokens.cssbrain"),
            ":root{--accent:#0062c4;--space-2:8px;--bad:#c42b1c}",
        )
        .unwrap();
        assert!(
            read_cache(
                &root.join(".brain/cache").join(
                    std::fs::read_dir(root.join(".brain/cache"))
                        .unwrap()
                        .next()
                        .unwrap()
                        .unwrap()
                        .file_name()
                )
            )
            .is_none()
        );
        compile_hash(src, rel, &root, true, 6).ok().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn real_checker_reports_this() {
        let src = "<script>\n  let n = sig(0)\n  this.n = 1\n</script>\n<p>{n}</p>";
        let p = Path::new("comp/x.brain");
        let (diags, _) = check(src, p, Path::new("/nonexistent/comp/x.brain"));
        assert!(
            diags.iter().any(|d| d.code == "B0102"),
            "expected B0102: {}",
            Diag::list_to_json(&diags)
        );
        assert!(compile(src, p, Path::new("/nonexistent"), true).is_err());
    }
    #[test]
    fn cache_budget_evicts_only_complete_owned_entries() {
        let dir = std::env::temp_dir().join(format!("brain-cache-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..5 {
            std::fs::write(dir.join(format!("{i}-2.json")), b"12345678").unwrap();
        }
        std::fs::write(dir.join("other.json"), b"preserve").unwrap();
        std::fs::write(dir.join("active.tmp1-1"), b"preserve").unwrap();
        prune_cache(&dir, 16, 2);
        let kept = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with("-2.json"))
            .count();
        assert_eq!(kept, 2);
        assert!(dir.join("other.json").exists() && dir.join("active.tmp1-1").exists());
        let oversized = dir.join("oversized-2.json");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(CACHE_ENTRY_BYTES + 1)
            .unwrap();
        assert!(read_cache(&oversized).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_cache_writers_leave_one_valid_entry_and_no_temporary_files() {
        let dir = std::env::temp_dir().join(format!("brain-cache-writers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("same-2.json");
        std::thread::scope(|scope| {
            for i in 0..8 {
                let entry = &entry;
                scope.spawn(move || {
                    let out = Output {
                        js: format!("// {i}\n{}", "x".repeat(65536)),
                        css: String::new(),
                        deps: vec![],
                        server_js: String::new(),
                        server_fns: vec![],
                    };
                    write_cache(entry, &out);
                });
            }
        });
        assert!(read_cache(&entry).is_some());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cache_tracks_absent_context_files_and_rejects_incomplete_metadata() {
        let root = std::env::temp_dir().join(format!("brain-cache-context-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("comp")).unwrap();
        std::fs::write(root.join("app.brain"), "<slot />").unwrap();
        let src = "<p>Hello</p>";
        compile(src, Path::new("comp/x.brain"), &root, true)
            .ok()
            .unwrap();
        let entry = std::fs::read_dir(root.join(".brain/cache"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(read_cache(&entry).is_some());
        std::fs::write(
            root.join("types.jsbrain"),
            "export const User = { id: 'num!' }",
        )
        .unwrap();
        assert!(
            read_cache(&entry).is_none(),
            "creating a previously absent shared schema invalidates cache"
        );
        compile(src, Path::new("comp/x.brain"), &root, true)
            .ok()
            .unwrap();
        assert!(read_cache(&entry).is_some());
        std::fs::write(
            root.join("comp/tokens.cssbrain"),
            ":root { --accent: #0062c4; }",
        )
        .unwrap();
        assert!(
            read_cache(&entry).is_none(),
            "a nearer token file changes resolution"
        );
        std::fs::write(&entry, r#"{"js":"x","css":""}"#).unwrap();
        assert!(
            read_cache(&entry).is_none(),
            "missing dependencies are not a valid cache entry"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, unix))]
mod hash_resource_tests {
    use super::*;
    #[test]
    fn dependency_hashing_rejects_fifos_and_symlinks_before_opening() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("brain-hash-guard-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.jsbrain");
        std::fs::write(&source, "export default 1").unwrap();
        symlink(&source, root.join("alias.jsbrain")).unwrap();
        let pipe = root.join("pipe.jsbrain");
        let raw = std::ffi::CString::new(pipe.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
        assert!(file_hash(&pipe).is_none());
        assert!(file_hash(&root.join("alias.jsbrain")).is_none());
        assert!(file_hash(&source).is_some());
        std::fs::remove_dir_all(root).unwrap();
    }
}
