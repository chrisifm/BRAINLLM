//! Leased, revision-checked changes. Files are atomically replaced individually;
//! a batch is a recoverable logical transaction, never a filesystem transaction.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;
type Snapshot = BTreeMap<String, Vec<u8>>;
const FILE_BYTES: usize = 2 * 1024 * 1024;
const TOTAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILES: usize = 2048;
const MAX_OPERATIONS: usize = 32;
const MAX_PLANS: usize = 16;
const MAX_LEASES: usize = 128;
const DEADLINE: Duration = Duration::from_secs(120);

fn now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "system clock precedes Unix epoch".into())
}
fn revision(bytes: &[u8]) -> String {
    brain_pack::sha::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn random_id() -> Result<String> {
    let mut bytes = [0u8; 32];
    #[cfg(unix)]
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| "OS randomness unavailable".to_string())?;
    #[cfg(windows)]
    {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(
                provider: *mut std::ffi::c_void,
                buffer: *mut u8,
                size: u32,
                flags: u32,
            ) -> i32;
        }
        // BCRYPT_USE_SYSTEM_PREFERRED_RNG; no provider handle is required.
        if unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                2,
            )
        } < 0
        {
            return Err("OS randomness unavailable".into());
        }
    }
    #[cfg(not(any(unix, windows)))]
    return Err("change leases require an OS cryptographic random generator".into());
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string"))
}
fn owner(v: &Value) -> Result<&str> {
    let value = text(v, "owner")?;
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:@".contains(&b))
    {
        return Err("owner must contain 1..80 ASCII letters, digits, - _ . : or @".into());
    }
    Ok(value)
}
fn canonical(root: &Path) -> Result<PathBuf> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("app root must be a directory".into());
    }
    Ok(root)
}
fn safe(root: &Path, path: &str, internal: bool) -> Result<PathBuf> {
    if path.is_empty()
        || path.len() > 240
        || path.contains('\\')
        || path.contains('\0')
        || path.contains(':')
        || path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err("path must be a normalized app-relative path without traversal".into());
    }
    let parts: Vec<_> = Path::new(path).components().collect();
    if parts.len() > 32 || parts.iter().any(|p| !matches!(p, Component::Normal(_))) {
        return Err("invalid path or path depth exceeds 32".into());
    }
    let mut out = root.to_path_buf();
    for part in parts {
        let name = part.as_os_str().to_str().ok_or("paths must be UTF-8")?;
        let lower = name.to_ascii_lowercase();
        if !internal
            && (name.starts_with('.')
                || ["node_modules", "dist", "target", "credentials", "secrets"]
                    .contains(&lower.as_str())
                || lower.ends_with(".pem")
                || lower.ends_with(".key")
                || lower.contains(".env"))
        {
            return Err("changes only accept public app source paths; environment, credentials and generated files are excluded".into());
        }
        out.push(part);
        match fs::symlink_metadata(&out) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("symlink refused: {path}"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(out)
}
/// Shared path boundary for legacy MCP mutations; no lease or state is created.
pub(crate) fn source_path(root: &Path, path: &str) -> Result<PathBuf> {
    safe(&canonical(root)?, path, false)
}
/// Install one new source file atomically while the caller owns writer_lock.
pub(crate) fn create_source(root: &Path, path: &str, content: &str) -> Result<()> {
    let target=source_path(root,path)?;
    fs::create_dir_all(target.parent().ok_or("source file has no parent")?).map_err(|e|e.to_string())?;
    atomic(&target,content.as_bytes(),true)
}
fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn state(root: &Path) -> Result<PathBuf> {
    let base = safe(root, ".brain/changes", true)?;
    private_dir(&base)?;
    Ok(base)
}
fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn atomic(path: &Path, bytes: &[u8], exclusive: bool) -> Result<()> {
    atomic_guarded(path, bytes, exclusive, || Ok(()))
}
fn atomic_guarded(
    path: &Path,
    bytes: &[u8],
    exclusive: bool,
    verify: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let parent = path.parent().ok_or("file has no parent")?;
    let temp = parent.join(format!(".brain-change-{}.tmp", random_id()?));
    let result = (|| {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&temp).map_err(|e| e.to_string())?;
        f.write_all(bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        if let Ok(meta) = fs::metadata(path) {
            fs::set_permissions(&temp, meta.permissions()).map_err(|e| e.to_string())?;
        }
        verify()?;
        if exclusive {
            fs::hard_link(&temp, path).map_err(|e| e.to_string())?;
            fs::remove_file(&temp).map_err(|e| e.to_string())?;
        } else {
            fs::rename(&temp, path).map_err(|e| e.to_string())?;
        }
        sync_dir(parent)
    })();
    let _ = fs::remove_file(temp);
    result
}
fn save(path: &Path, value: &Value) -> Result<()> {
    atomic(
        path,
        &serde_json::to_vec(value).map_err(|e| e.to_string())?,
        false,
    )
}
fn load(path: &Path) -> Result<Value> {
    let bytes = bounded_read(path)?.ok_or("change record is missing")?;
    serde_json::from_slice(&bytes).map_err(|_| "change record is malformed".into())
}
fn bounded_read(path: &Path) -> Result<Option<Vec<u8>>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > FILE_BYTES as u64 {
        return Err("expected a regular file of at most 2 MiB".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(FILE_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > FILE_BYTES {
        return Err("file exceeds 2 MiB".into());
    }
    Ok(Some(bytes))
}
fn current(root: &Path, path: &str) -> Result<Option<String>> {
    Ok(bounded_read(&safe(root, path, false)?)?.map(|b| revision(&b)))
}
fn state_lock(base: &Path) -> Result<File> {
    let path = base.join("state.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("state lock symlink refused".into());
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.try_lock()
        .map_err(|_| "change metadata is busy; retry shortly".to_string())?;
    Ok(file)
}
// Cross-process admission, released by the kernel even after an interrupted CLI.
fn preview_slot(base: &Path) -> Result<File> {
    for index in 0..4 {
        let path = base.join(format!("preview-{index}.lock"));
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("preview lock symlink refused".into());
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| e.to_string())?;
        if file.try_lock().is_ok() {
            return Ok(file);
        }
    }
    Err("four change preparations are already active; retry after one finishes".into())
}
fn leases(base: &Path) -> Result<Vec<Value>> {
    let path = base.join("leases.json");
    let mut records = if path.exists() {
        load(&path)?
            .as_array()
            .cloned()
            .ok_or("malformed lease store")?
    } else {
        vec![]
    };
    if records.len() > MAX_LEASES {
        return Err("lease store exceeds its bound".into());
    }
    for record in &records {
        owner(record).map_err(|_| "malformed owner in lease store")?;
        let path = text(record, "path").map_err(|_| "malformed path in lease store")?;
        let token = text(record, "tokenHash").map_err(|_| "malformed token hash in lease store")?;
        if path.is_empty()
            || path.len() > 240
            || token.len() != 64
            || !token
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || record["expiresAt"].as_u64().is_none()
        {
            return Err(
                "malformed lease store; source writers remain blocked until repaired".into(),
            );
        }
    }
    let time = now()?;
    records.retain(|l| l["expiresAt"].as_u64().unwrap_or(0) > time);
    Ok(records)
}
/// Called only while edit::writer_lock holds the app writer lock. A legacy
/// writer cannot prove a token/path scope, so any active reservation blocks it.
/// Lease acquisition takes the same writer lock before state.lock, closing the
/// race in which a reservation could otherwise appear after this check.
pub(crate) fn guard_legacy_writer(root: &Path) -> Result<()> {
    let root = canonical(root)?;
    let base = state(&root)?;
    let _state = state_lock(&base)?;
    if !leases(&base)?.is_empty() {
        return Err("active file leases protect this app; use the leased changes protocol, release the leases or wait for their expiry".into());
    }
    Ok(())
}
fn requested_paths(root: &Path, args: &Value) -> Result<Vec<String>> {
    let paths = args
        .get("paths")
        .and_then(Value::as_array)
        .ok_or("paths must be an array")?;
    if paths.is_empty() || paths.len() > MAX_OPERATIONS {
        return Err("a lease requires 1..32 paths".into());
    }
    let mut result = BTreeSet::new();
    for path in paths {
        let path = path.as_str().ok_or("paths must contain strings")?;
        safe(root, path, false)?;
        if !result.insert(path.to_owned()) {
            return Err("duplicate lease path".into());
        }
    }
    Ok(result.into_iter().collect())
}
fn ttl(args: &Value) -> Result<u64> {
    let ttl = args
        .get("ttlSeconds")
        .map(|v| v.as_u64().ok_or("ttlSeconds must be an integer"))
        .transpose()?
        .unwrap_or(300);
    if !(5..=900).contains(&ttl) {
        return Err("ttlSeconds must be 5..900".into());
    }
    Ok(ttl)
}
fn authorize(records: &[Value], who: &str, token: &str, paths: &[String]) -> Result<()> {
    if token.len() != 64 {
        return Err("invalid lease token".into());
    }
    let hash = revision(token.as_bytes());
    if paths.iter().all(|path| {
        records
            .iter()
            .any(|l| l["owner"] == who && l["tokenHash"] == hash && l["path"] == path.as_str())
    }) {
        Ok(())
    } else {
        Err("lease expired or does not authorize every changed file; acquire or renew the file leases".into())
    }
}
/// Acquire/renew/release/list bounded file leases. Tokens are returned only on acquire.
pub fn lease(root: &Path, args: &Value) -> Result<Value> {
    let root = canonical(root)?;
    let base = state(&root)?;
    // Match the commit lock order: app writer first, then lease metadata.
    // Without this lock a new lease could appear while a legacy writer operates.
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("acquire");
    let _writer = if action == "acquire" {
        Some(crate::edit::writer_lock_unchecked(&root)?)
    } else {
        None
    };
    let _lock = state_lock(&base)?;
    let mut records = leases(&base)?;
    match args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("acquire")
    {
        "list" => Ok(
            json!({"leases":records.iter().map(|l| json!({"path":l["path"],"owner":l["owner"],"expiresAt":l["expiresAt"]})).collect::<Vec<_>>(),"maxTtlSeconds":900}),
        ),
        "acquire" => {
            let who = owner(args)?;
            let paths = requested_paths(&root, args)?;
            let expires = now()? + ttl(args)?;
            for path in &paths {
                if let Some(existing) = records.iter().find(|l| l["path"] == path.as_str()) {
                    return Err(format!(
                        "file lease conflict: {path} is reserved by {} until {}",
                        existing["owner"], existing["expiresAt"]
                    ));
                }
            }
            if records.len() + paths.len() > MAX_LEASES {
                return Err("at most 128 active file leases are allowed".into());
            }
            let token = random_id()?;
            let hash = revision(token.as_bytes());
            for path in &paths {
                records.push(json!({"owner":who,"path":path,"tokenHash":hash,"expiresAt":expires}));
            }
            save(&base.join("leases.json"), &json!(records))?;
            let revisions: Vec<_> = paths
                .iter()
                .map(|p| Ok(json!({"path":p,"revision":current(&root,p)?})))
                .collect::<Result<_>>()?;
            Ok(json!({"owner":who,"token":token,"expiresAt":expires,"files":revisions}))
        }
        action @ ("renew" | "release") => {
            let who = owner(args)?;
            let token = text(args, "token")?;
            let paths = requested_paths(&root, args)?;
            authorize(&records, who, token, &paths)?;
            let expires = now()? + ttl(args)?;
            for l in &mut records {
                if paths.iter().any(|p| l["path"] == p.as_str()) {
                    l["expiresAt"] = json!(if action == "release" { 0 } else { expires });
                }
            }
            records.retain(|l| l["expiresAt"] != 0);
            save(&base.join("leases.json"), &json!(records))?;
            Ok(
                json!({"action":action,"paths":paths,"expiresAt":if action == "release" {0} else {expires}}),
            )
        }
        _ => Err("lease action must be acquire, renew, release or list".into()),
    }
}
/// Read an editable source file and its exact SHA-256 revision. Environment files are excluded.
pub fn read(root: &Path, path: &str) -> Result<Value> {
    let root = canonical(root)?;
    let bytes = bounded_read(&safe(&root, path, false)?)?;
    match bytes {
        None => Ok(json!({"path":path,"revision":null,"content":null})),
        Some(bytes) => {
            clean(&bytes)?;
            let content =
                std::str::from_utf8(&bytes).map_err(|_| "changes require UTF-8 source")?;
            Ok(json!({"path":path,"revision":revision(&bytes),"content":content}))
        }
    }
}
fn clean(bytes: &[u8]) -> Result<()> {
    if let Ok(src) = std::str::from_utf8(bytes) {
        if !brain_check::scan_secrets(src).is_empty() {
            return Err(
                "source contains a detected secret; remove it before creating source snapshots"
                    .into(),
            );
        }
    }
    Ok(())
}
fn deadline(start: Instant) -> Result<()> {
    if start.elapsed() > DEADLINE {
        Err("change preparation exceeded its 120 second deadline".into())
    } else {
        Ok(())
    }
}
fn snapshot(root: &Path, start: Instant) -> Result<Snapshot> {
    if crate::env::check(root).iter().any(|d| !d.is_warning()) {
        return Err(
            "environment declarations are invalid; run brain check before preparing changes".into(),
        );
    }
    fn walk(
        root: &Path,
        dir: &Path,
        depth: usize,
        total: &mut usize,
        result: &mut Snapshot,
        start: Instant,
    ) -> Result<()> {
        deadline(start)?;
        if depth > 32 {
            return Err("snapshot exceeds 32 directory levels".into());
        }
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "source paths must be UTF-8")?;
            if ["node_modules", "dist", "target"].contains(&name.as_str())
                || (name.starts_with('.') && name != ".brain_env")
            {
                continue;
            }
            let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                return Err("snapshot refuses symlinks".into());
            }
            if meta.is_dir() {
                walk(root, &entry.path(), depth + 1, total, result, start)?;
            } else {
                if result.len() >= MAX_FILES {
                    return Err("snapshot exceeds 2048 files".into());
                }
                let mut bytes =
                    bounded_read(&entry.path())?.ok_or("source disappeared during snapshot")?;
                // Preserve declared environment keys, never values, in compiler previews.
                if name == ".brain_env" {
                    let src = std::str::from_utf8(&bytes)
                        .map_err(|_| "environment declarations must be UTF-8")?;
                    bytes = crate::env::example_of(src).into_bytes();
                } else {
                    clean(&bytes)?;
                }
                *total += bytes.len();
                if *total > TOTAL_BYTES {
                    return Err("snapshot exceeds 32 MiB".into());
                }
                let path = entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|_| "invalid snapshot root")?
                    .to_str()
                    .ok_or("source paths must be UTF-8")?
                    .replace('\\', "/");
                result.insert(path, bytes);
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    walk(root, root, 0, &mut 0, &mut result, start)?;
    Ok(result)
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn check_snapshot(_base: &Path, files: &Snapshot, start: Instant) -> Result<()> {
    let temp = std::env::temp_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if crate::scaffold::outer_root(&temp).is_some() {
        return Err("temporary directory must be outside every BRAINLLM app".into());
    }
    let dir = temp.join(format!("brain-changes-{}", random_id()?));
    private_dir(&dir)?;
    let scratch = Scratch(dir);
    for (path, bytes) in files {
        deadline(start)?;
        let dest = safe(&scratch.0, path, true)?;
        private_dir(dest.parent().ok_or("snapshot file has no parent")?)?;
        atomic(&dest, bytes, true)?;
    }
    let (_, diagnostics) = crate::run_check(Some(scratch.0.clone()))?;
    deadline(start)?;
    let errors: Vec<_> = diagnostics.iter().filter(|d| !d.is_warning()).collect();
    if let Some(first) = errors.first() {
        return Err(format!(
            "change check failed: {} errors; first {} in {}; app files unchanged",
            errors.len(),
            first.code,
            first
                .file
                .replace(&scratch.0.to_string_lossy().to_string(), "<preview>")
        ));
    }
    Ok(())
}
fn operations(root: &Path, args: &Value) -> Result<Vec<Value>> {
    let input = args
        .get("operations")
        .and_then(Value::as_array)
        .ok_or("operations must be an array")?;
    if input.is_empty() || input.len() > MAX_OPERATIONS {
        return Err("a change plan requires 1..32 operations".into());
    }
    let mut seen = BTreeSet::new();
    let mut output = vec![];
    let mut total = 0;
    for op in input {
        let path = text(op, "path")?;
        safe(root, path, false)?;
        if !seen.insert(path.to_owned())
            || seen.iter().any(|p| {
                p != path
                    && (p.starts_with(&format!("{path}/")) || path.starts_with(&format!("{p}/")))
            })
        {
            return Err("operation paths must be unique and cannot contain each other".into());
        }
        let expected = op
            .get("expectedRevision")
            .ok_or("expectedRevision is required (use null for a new file)")?;
        if !expected.is_null()
            && !expected.as_str().is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            })
        {
            return Err("expectedRevision must be a lowercase SHA-256 hash or null".into());
        }
        let content = op
            .get("content")
            .ok_or("content is required (use null to delete)")?;
        if !content.is_null() && !content.is_string() {
            return Err("content must be a string or null".into());
        }
        if expected.is_null() && content.is_null() {
            return Err("cannot delete a file expected to be absent".into());
        }
        let bytes = content.as_str().unwrap_or("").as_bytes();
        clean(bytes)?;
        total += bytes.len();
        if bytes.len() > FILE_BYTES || total > 4 * 1024 * 1024 {
            return Err("changes exceed 2 MiB per file or 4 MiB per batch".into());
        }
        output.push(json!({"path":path,"expectedRevision":expected,"content":content}));
    }
    Ok(output)
}
fn paths(ops: &[Value]) -> Vec<String> {
    ops.iter()
        .map(|op| op["path"].as_str().unwrap().to_owned())
        .collect()
}
fn guard(root: &Path, ops: &[Value]) -> Result<()> {
    for op in ops {
        let path = text(op, "path")?;
        if json!(current(root, path)?) != op["expectedRevision"] {
            return Err(format!(
                "revision conflict: {path} changed; read it again and prepare a new plan"
            ));
        }
    }
    Ok(())
}
fn overlay(files: &mut Snapshot, ops: &[Value]) {
    for op in ops {
        let path = op["path"].as_str().unwrap();
        if let Some(content) = op["content"].as_str() {
            files.insert(path.to_owned(), content.as_bytes().to_vec());
        } else {
            files.remove(path);
        }
    }
}
fn impact(ops: &[Value]) -> Value {
    json!({"files":ops.iter().map(|op| {
        let path=op["path"].as_str().unwrap();
        let kind=if op["content"].is_null(){"delete"}else if op["expectedRevision"].is_null(){"create"}else{"update"};
        let area=if path.starts_with("views/") || path.starts_with("pages/"){"route"}else if path.starts_with("server/"){"backend"}else if path.ends_with(".cssbrain"){"styles"}else if path.ends_with(".brain"){"component"}else if path.ends_with(".jsbrain"){"logic"}else{"source"};
        json!({"path":path,"operation":kind,"area":area,"beforeRevision":op["expectedRevision"],"afterRevision":op["content"].as_str().map(|s|revision(s.as_bytes())),"bytes":op["content"].as_str().map(str::len).unwrap_or(0)})
    }).collect::<Vec<_>>(),"validation":"whole-app brain check","atomicity":"per-file atomic replacement; recoverable multi-file logical transaction","externalEditorRace":"cooperative writers are serialized; unrelated editors are checked optimistically, not through a filesystem compare-and-swap"})
}
fn plan_dir(base: &Path, id: &str) -> Result<PathBuf> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("invalid plan ID".into());
    }
    let dir = base.join(id);
    if fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("plan symlink refused".into());
    }
    Ok(dir)
}
fn load_ops(dir: &Path, record: &Value) -> Result<Vec<Value>> {
    let entries = record["files"]
        .as_array()
        .ok_or("malformed change manifest")?;
    if entries.is_empty() || entries.len() > MAX_OPERATIONS {
        return Err("invalid operation count in manifest".into());
    }
    entries.iter().enumerate().map(|(i,file)| {
        let content=if file["afterRevision"].is_null(){Value::Null}else{
            let bytes=bounded_read(&dir.join(format!("{i}.after")))?.ok_or("missing prepared content")?;
            if json!(revision(&bytes)) != file["afterRevision"] { return Err("prepared content integrity check failed".into()); }
            Value::String(String::from_utf8(bytes).map_err(|_|"prepared source is not UTF-8")?)
        };
        Ok(json!({"path":file["path"],"expectedRevision":file["beforeRevision"],"content":content}))
    }).collect()
}
/// Validate a bounded isolated snapshot; save only changed source payloads privately.
/// The journal/response contains hashes and metadata, never source or environment values.
pub fn plan(root: &Path, args: &Value) -> Result<Value> {
    let start = Instant::now();
    let root = canonical(root)?;
    let base = state(&root)?;
    let who = owner(args)?;
    let token = text(args, "token")?;
    let ops = operations(&root, args)?;
    let _slot = preview_slot(&base)?;
    let path_list = paths(&ops);
    {
        let _lock = state_lock(&base)?;
        authorize(&leases(&base)?, who, token, &path_list)?;
    }
    guard(&root, &ops)?;
    let before = snapshot(&root, start)?;
    let mut after = before.clone();
    overlay(&mut after, &ops);
    check_snapshot(&base, &after, start)?;
    guard(&root, &ops)?;
    let _lock = state_lock(&base)?;
    authorize(&leases(&base)?, who, token, &path_list)?;
    let count = fs::read_dir(&base)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.len() == 64))
        .count();
    if count >= MAX_PLANS {
        return Err(
            "16 retained plans reached; discard finished plans before preparing another".into(),
        );
    }
    let id = random_id()?;
    let dir = plan_dir(&base, &id)?;
    private_dir(&dir)?;
    let result = (|| {
        let mut files = vec![];
        let mut backup_bytes = 0;
        for (i, op) in ops.iter().enumerate() {
            let path = text(op, "path")?;
            let old = before.get(path);
            if json!(old.map(|b| revision(b))) != op["expectedRevision"] {
                return Err(format!("revision conflict while snapshotting {path}"));
            }
            if let Some(bytes) = old {
                std::str::from_utf8(bytes).map_err(|_| "changes require UTF-8 source backups")?;
                backup_bytes += bytes.len();
                if backup_bytes > 4 * 1024 * 1024 {
                    return Err("change backups exceed 4 MiB per batch".into());
                }
                atomic(&dir.join(format!("{i}.before")), bytes, true)?;
            }
            if let Some(src) = op["content"].as_str() {
                atomic(&dir.join(format!("{i}.after")), src.as_bytes(), true)?;
            }
            files.push(json!({"path":path,"beforeRevision":op["expectedRevision"],"afterRevision":op["content"].as_str().map(|s|revision(s.as_bytes()))}));
        }
        let record = json!({"schemaVersion":1,"id":id,"owner":who,"createdAt":now()?,"state":"prepared","files":files,"applied":0});
        save(&dir.join("manifest.json"), &record)?;
        Ok(
            json!({"schemaVersion":1,"planId":id,"state":"prepared","dryRun":true,"impact":impact(&ops)}),
        )
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    result
}
fn write_one(root: &Path, op: &Value) -> Result<()> {
    let path = text(op, "path")?;
    let target = safe(root, path, false)?;
    if let Some(src) = op["content"].as_str() {
        // Ancestors are deliberately not created in the app until commit.
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        safe(root, path, false)?;
        if json!(current(root, path)?) != op["expectedRevision"] {
            return Err(format!("revision conflict before writing {path}"));
        }
        atomic_guarded(
            &target,
            src.as_bytes(),
            op["expectedRevision"].is_null(),
            || {
                safe(root, path, false)?;
                if json!(current(root, path)?) != op["expectedRevision"] {
                    return Err(format!(
                        "revision conflict immediately before replacing {path}"
                    ));
                }
                Ok(())
            },
        )
    } else {
        if json!(current(root, path)?) != op["expectedRevision"] {
            return Err(format!("revision conflict before deleting {path}"));
        }
        fs::remove_file(&target).map_err(|e| e.to_string())?;
        sync_dir(target.parent().ok_or("file has no parent")?)
    }
}
fn restore(root: &Path, dir: &Path, record: &mut Value) -> Result<Value> {
    let files = record["files"]
        .as_array()
        .ok_or("malformed recovery manifest")?
        .clone();
    let mut conflicts = vec![];
    let mut restored = vec![];
    // Inspect every entry, including a write completed just before a crash while
    // the persisted applied counter still describes the previous entry.
    for (i, file) in files.iter().enumerate().rev() {
        let path = text(file, "path")?;
        let actual = json!(current(root, path)?);
        if actual == file["beforeRevision"] {
            continue;
        }
        if actual != file["afterRevision"] {
            conflicts.push(path.to_owned());
            continue;
        }
        let content = if file["beforeRevision"].is_null() {
            Value::Null
        } else {
            let bytes = bounded_read(&dir.join(format!("{i}.before")))?
                .ok_or("recovery backup is missing")?;
            if json!(revision(&bytes)) != file["beforeRevision"] {
                return Err("recovery backup integrity check failed".into());
            }
            Value::String(String::from_utf8(bytes).map_err(|_| "recovery backup is not UTF-8")?)
        };
        let undo = json!({"path":path,"expectedRevision":file["afterRevision"],"content":content});
        match write_one(root, &undo) {
            Ok(()) => restored.push(path.to_owned()),
            Err(_) => conflicts.push(path.to_owned()),
        }
    }
    record["state"] = json!(if conflicts.is_empty() {
        "rolledBack"
    } else {
        "recoveryConflict"
    });
    record["recoveryConflicts"] = json!(conflicts);
    save(&dir.join("manifest.json"), record)?;
    Ok(
        json!({"planId":record["id"],"state":record["state"],"restored":restored,"preservedExternalChanges":conflicts}),
    )
}
/// Apply after fresh validation. Earlier previews do not block independent plans:
/// this check incorporates every intervening nonconflicting source change.
pub fn apply(root: &Path, id: &str, who: &str, token: &str) -> Result<Value> {
    apply_with(root, id, who, token, |_| Ok(()))
}
fn apply_with(
    root: &Path,
    id: &str,
    who: &str,
    token: &str,
    mut hook: impl FnMut(usize) -> Result<()>,
) -> Result<Value> {
    let start = Instant::now();
    let root = canonical(root)?;
    let base = state(&root)?;
    let dir = plan_dir(&base, id)?;
    let _slot = preview_slot(&base)?;
    let mut record = load(&dir.join("manifest.json"))?;
    if record["owner"] != who || record["state"] != "prepared" {
        return Err("only the owner of a prepared plan may apply it".into());
    }
    let ops = operations(&root, &json!({"operations":load_ops(&dir,&record)?}))?;
    let path_list = paths(&ops);
    for attempt in 0..2 {
        {
            let _lock = state_lock(&base)?;
            authorize(&leases(&base)?, who, token, &path_list)?;
        }
        guard(&root, &ops)?;
        let before = snapshot(&root, start)?;
        let mut after = before.clone();
        overlay(&mut after, &ops);
        check_snapshot(&base, &after, start)?;
        let _writer = crate::edit::writer_lock_unchecked(&root)?;
        let _state = state_lock(&base)?;
        authorize(&leases(&base)?, who, token, &path_list)?;
        // A second process may have applied this plan while validation ran.
        record = load(&dir.join("manifest.json"))?;
        if record["state"] != "prepared" {
            return Err("plan is no longer prepared".into());
        }
        guard(&root, &ops)?;
        if snapshot(&root, start)? != before {
            if attempt == 0 {
                continue;
            }
            return Err(
                "snapshot changed during validation; retry apply (target revisions still match)"
                    .into(),
            );
        }
        record["state"] = json!("applying");
        save(&dir.join("manifest.json"), &record)?;
        for (i, op) in ops.iter().enumerate() {
            let result = hook(i).and_then(|_| write_one(&root, op));
            if let Err(error) = result {
                let recovery = restore(&root, &dir, &mut record)?;
                return Err(format!("commit stopped: {error}; recovery: {recovery}"));
            }
            record["applied"] = json!(i + 1);
            if let Err(error) = save(&dir.join("manifest.json"), &record) {
                let recovery = restore(&root, &dir, &mut record)?;
                return Err(format!(
                    "journal update failed: {error}; recovery: {recovery}"
                ));
            }
        }
        record["state"] = json!("committed");
        record["committedAt"] = json!(now()?);
        save(&dir.join("manifest.json"), &record)?;
        return Ok(json!({"planId":id,"state":"committed","dryRun":false,"impact":impact(&ops)}));
    }
    Err("snapshot changed; retry apply".into())
}
/// Recover an interrupted batch or explicitly roll back a committed one. Files
/// changed by another editor are preserved and reported, never blindly restored.
pub fn recover(root: &Path, id: &str, who: &str, token: &str) -> Result<Value> {
    let root = canonical(root)?;
    let base = state(&root)?;
    let dir = plan_dir(&base, id)?;
    let _writer = crate::edit::writer_lock_unchecked(&root)?;
    let _state = state_lock(&base)?;
    let mut record = load(&dir.join("manifest.json"))?;
    if record["owner"] != who
        || !["applying", "committed", "recoveryConflict"]
            .iter()
            .any(|s| record["state"] == *s)
    {
        return Err("only the owner may recover an applying, committed or conflicted plan".into());
    }
    let ops = operations(&root, &json!({"operations":load_ops(&dir,&record)?}))?;
    authorize(&leases(&base)?, who, token, &paths(&ops))?;
    restore(&root, &dir, &mut record)
}
/// CLI/MCP adapter. Discard removes private payloads only for finished/prepared plans.
pub fn dispatch(root: &Path, op: &str, args: &Value) -> Result<Value> {
    match op {
        "read" => read(root, text(args, "path")?),
        "lease" => lease(root, args),
        "plan" | "preview" => plan(root, args),
        "apply" => apply(
            root,
            text(args, "planId")?,
            owner(args)?,
            text(args, "token")?,
        ),
        "recover" | "rollback" => recover(
            root,
            text(args, "planId")?,
            owner(args)?,
            text(args, "token")?,
        ),
        "status" | "discard" => {
            let root = canonical(root)?;
            let base = state(&root)?;
            let _lock = state_lock(&base)?;
            let dir = plan_dir(&base, text(args, "planId")?)?;
            let record = load(&dir.join("manifest.json"))?;
            if record["owner"] != owner(args)? {
                return Err("plan owner does not match".into());
            }
            let ops = operations(&root, &json!({"operations":load_ops(&dir,&record)?}))?;
            authorize(
                &leases(&base)?,
                owner(args)?,
                text(args, "token")?,
                &paths(&ops),
            )?;
            if op == "discard" {
                if !["prepared", "committed", "rolledBack"]
                    .iter()
                    .any(|s| record["state"] == *s)
                {
                    return Err(
                        "recover an interrupted or conflicted plan before discarding it".into(),
                    );
                }
                fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
                Ok(json!({"planId":record["id"],"state":"discarded"}))
            } else {
                Ok(
                    json!({"planId":record["id"],"state":record["state"],"owner":record["owner"],"createdAt":record["createdAt"],"impact":impact(&ops),"recoveryConflicts":record["recoveryConflicts"]}),
                )
            }
        }
        _ => Err(
            "change operation must be read, lease, plan, apply, recover, status or discard".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct App(PathBuf);
    impl App {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("brain-changes-test-{}", random_id().unwrap()));
            fs::create_dir(&root).unwrap();
            fs::write(root.join("brain.json"), "{\"name\":\"LeaseTest\"}").unwrap();
            fs::write(root.join("tokens.cssbrain"), ":root { --ink: #111; }\n").unwrap();
            fs::write(root.join("a.brain"), "<p>Alpha</p>\n").unwrap();
            fs::write(root.join("b.brain"), "<p>Beta</p>\n").unwrap();
            Self(root)
        }
        fn lease(&self, who: &str, paths: &[&str]) -> String {
            lease(&self.0, &json!({"owner":who,"paths":paths})).unwrap()["token"]
                .as_str()
                .unwrap()
                .into()
        }
        fn op(&self, path: &str, content: Value) -> Value {
            json!({"path":path,"expectedRevision":current(&self.0,path).unwrap(),"content":content})
        }
        fn plan(&self, who: &str, token: &str, ops: Vec<Value>) -> String {
            plan(
                &self.0,
                &json!({"owner":who,"token":token,"operations":ops}),
            )
            .unwrap()["planId"]
                .as_str()
                .unwrap()
                .into()
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn independently_prepared_file_plans_compose_without_rejecting_other_files() {
        let app = App::new();
        let a = app.lease("agent-a", &["a.brain"]);
        let b = app.lease("agent-b", &["b.brain"]);
        let pa = app.plan(
            "agent-a",
            &a,
            vec![app.op("a.brain", json!("<p>Changed alpha</p>\n"))],
        );
        let pb = app.plan(
            "agent-b",
            &b,
            vec![app.op("b.brain", json!("<p>Changed beta</p>\n"))],
        );
        apply(&app.0, &pa, "agent-a", &a).unwrap();
        apply(&app.0, &pb, "agent-b", &b).unwrap();
        assert_eq!(
            fs::read_to_string(app.0.join("a.brain")).unwrap(),
            "<p>Changed alpha</p>\n"
        );
        assert_eq!(
            fs::read_to_string(app.0.join("b.brain")).unwrap(),
            "<p>Changed beta</p>\n"
        );
    }
    #[test]
    fn same_file_lease_and_stale_revision_conflict_preserve_editor_work() {
        let app = App::new();
        let token = app.lease("agent-a", &["a.brain"]);
        assert!(
            lease(&app.0, &json!({"owner":"agent-b","paths":["a.brain"]}))
                .unwrap_err()
                .contains("lease conflict")
        );
        let id = app.plan(
            "agent-a",
            &token,
            vec![app.op("a.brain", json!("<p>Plan</p>\n"))],
        );
        fs::write(app.0.join("a.brain"), "<p>External editor</p>\n").unwrap();
        assert!(apply(&app.0, &id, "agent-a", &token)
            .unwrap_err()
            .contains("revision conflict"));
        assert_eq!(
            fs::read_to_string(app.0.join("a.brain")).unwrap(),
            "<p>External editor</p>\n"
        );
    }
    #[test]
    fn expired_and_wrong_tokens_cannot_commit_and_tokens_are_not_journaled() {
        let app = App::new();
        let token = app.lease("agent-a", &["a.brain"]);
        let id = app.plan(
            "agent-a",
            &token,
            vec![app.op("a.brain", json!("<p>Plan</p>\n"))],
        );
        assert!(apply(&app.0, &id, "agent-a", &"a".repeat(64)).is_err());
        let base = state(&app.0).unwrap();
        let file = base.join("leases.json");
        let encoded = fs::read_to_string(&file).unwrap();
        assert!(!encoded.contains(&token));
        let mut records: Value = serde_json::from_str(&encoded).unwrap();
        records[0]["expiresAt"] = json!(now().unwrap() - 1);
        save(&file, &records).unwrap();
        assert!(apply(&app.0, &id, "agent-a", &token)
            .unwrap_err()
            .contains("expired"));
        let replacement = app.lease("agent-b", &["a.brain"]);
        assert_ne!(token, replacement);
        assert!(
            !fs::read_to_string(plan_dir(&base, &id).unwrap().join("manifest.json"))
                .unwrap()
                .contains("<p>Plan")
        );
    }
    #[test]
    fn malformed_paths_operations_and_environment_access_are_rejected() {
        let app = App::new();
        for path in [
            "../escape.brain",
            "/tmp/escape.brain",
            "a/../b.brain",
            "a//b.brain",
            "a\\b.brain",
            ".brain_env",
            ".env",
            "a/./b.brain",
            "node_modules/a.brain",
            "secrets/password.json",
        ] {
            assert!(read(&app.0, path).is_err(), "{path}");
            assert!(
                lease(&app.0, &json!({"owner":"agent","paths":[path]})).is_err(),
                "{path}"
            );
        }
        assert!(operations(
            &app.0,
            &json!({"operations":[{"path":"a.brain","content":"hello"}]})
        )
        .is_err());
        assert!(operations(
            &app.0,
            &json!({"operations":[{"path":"a.brain","expectedRevision":null,"content":null}]})
        )
        .is_err());
        assert!(operations(
            &app.0,
            &json!({"operations":[{"path":"a.brain","expectedRevision":"wrong","content":"hello"}]})
        )
        .is_err());
        assert!(lease(
            &app.0,
            &json!({"owner":"agent","paths":["a.brain"],"ttlSeconds":901})
        )
        .is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(app.0.join("a.brain"), app.0.join("alias.brain")).unwrap();
            assert!(read(&app.0, "alias.brain").is_err());
        }
    }
    #[test]
    fn failed_check_never_changes_app_and_commit_failure_rolls_back() {
        let app = App::new();
        let token = app.lease("agent", &["a.brain", "b.brain"]);
        let original = fs::read(app.0.join("a.brain")).unwrap();
        let invalid = app.op("a.brain", json!("<p>{missingVariable}</p>\n"));
        assert!(plan(
            &app.0,
            &json!({"owner":"agent","token":token,"operations":[invalid]})
        )
        .unwrap_err()
        .contains("check failed"));
        assert_eq!(fs::read(app.0.join("a.brain")).unwrap(), original);
        let id = app.plan(
            "agent",
            &token,
            vec![
                app.op("a.brain", json!("<p>Changed</p>\n")),
                app.op("b.brain", json!("<p>Changed</p>\n")),
            ],
        );
        assert!(apply_with(&app.0, &id, "agent", &token, |i| if i == 1 {
            Err("injected I/O failure".into())
        } else {
            Ok(())
        })
        .unwrap_err()
        .contains("rolledBack"));
        assert_eq!(fs::read(app.0.join("a.brain")).unwrap(), original);
        assert_eq!(
            fs::read_to_string(app.0.join("b.brain")).unwrap(),
            "<p>Beta</p>\n"
        );
    }
    #[test]
    fn rollback_preserves_external_changes_and_can_be_resolved_later() {
        let app = App::new();
        let token = app.lease("agent", &["a.brain", "b.brain"]);
        let id = app.plan(
            "agent",
            &token,
            vec![
                app.op("a.brain", json!("<p>Plan</p>\n")),
                app.op("b.brain", json!("<p>Plan</p>\n")),
            ],
        );
        let error = apply_with(&app.0, &id, "agent", &token, |i| {
            if i == 1 {
                fs::write(app.0.join("a.brain"), "<p>External</p>\n").unwrap();
                Err("injected failure".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(error.contains("recoveryConflict"), "{error}");
        assert_eq!(
            fs::read_to_string(app.0.join("a.brain")).unwrap(),
            "<p>External</p>\n"
        );
        fs::write(app.0.join("a.brain"), "<p>Plan</p>\n").unwrap();
        assert_eq!(
            recover(&app.0, &id, "agent", &token).unwrap()["state"],
            "rolledBack"
        );
        assert_eq!(
            fs::read_to_string(app.0.join("a.brain")).unwrap(),
            "<p>Alpha</p>\n"
        );
    }
    #[test]
    fn durable_recovery_handles_write_before_progress_journal_and_environment_redaction() {
        let app = App::new();
        fs::write(
            app.0.join(".brain_env"),
            "PUBLIC_TITLE=Private title\nSECRET_VALUE=never-copy-this-value\n",
        )
        .unwrap();
        let files = snapshot(&app.0, Instant::now()).unwrap();
        let env = String::from_utf8(files[".brain_env"].clone()).unwrap();
        assert!(env.contains("SECRET_VALUE=\n"));
        assert!(!env.contains("never-copy"));
        assert!(!env.contains("Private title"));
        let token = app.lease("agent", &["a.brain"]);
        let id = app.plan(
            "agent",
            &token,
            vec![app.op("a.brain", json!("<p>Plan</p>\n"))],
        );
        let dir = plan_dir(&state(&app.0).unwrap(), &id).unwrap();
        let manifest = dir.join("manifest.json");
        let mut record = load(&manifest).unwrap();
        record["state"] = json!("applying");
        save(&manifest, &record).unwrap();
        fs::write(app.0.join("a.brain"), "<p>Plan</p>\n").unwrap();
        assert_eq!(record["applied"], 0);
        assert_eq!(
            recover(&app.0, &id, "agent", &token).unwrap()["state"],
            "rolledBack"
        );
        assert_eq!(
            fs::read_to_string(app.0.join("a.brain")).unwrap(),
            "<p>Alpha</p>\n"
        );
    }
    #[test]
    fn new_files_are_exclusive_and_committed_batches_can_be_rolled_back() {
        let app = App::new();
        let token = app.lease("agent", &["comp/new.brain", "b.brain"]);
        let id = app.plan(
            "agent",
            &token,
            vec![
                app.op("comp/new.brain", json!("<p>New</p>\n")),
                app.op("b.brain", Value::Null),
            ],
        );
        apply(&app.0, &id, "agent", &token).unwrap();
        assert!(!app.0.join("b.brain").exists());
        assert!(app.0.join("comp/new.brain").exists());
        recover(&app.0, &id, "agent", &token).unwrap();
        assert!(!app.0.join("comp/new.brain").exists());
        assert!(app.0.join("b.brain").exists());
    }
    #[test]
    fn legacy_writer_respects_leases_until_they_expire() {
        let app = App::new();
        let token = app.lease("agent", &["a.brain"]);
        let error = crate::edit::writer_lock(&app.0).unwrap_err();
        assert!(error.contains("active file leases"), "{error}");
        assert!(!error.contains(&token));
        let base = state(&app.0).unwrap();
        let mut records = load(&base.join("leases.json")).unwrap();
        records[0]["expiresAt"] = json!(now().unwrap() - 1);
        save(&base.join("leases.json"), &records).unwrap();
        let writer = crate::edit::writer_lock(&app.0).unwrap();
        // Reservation cannot sneak in after the legacy guard has passed.
        assert!(lease(&app.0, &json!({"owner":"other","paths":["b.brain"]}))
            .unwrap_err()
            .contains("another edit"));
        drop(writer);
        app.lease("other", &["b.brain"]);
        assert!(crate::edit::writer_lock(&app.0).is_err());
    }
    #[test]
    fn corrupt_lease_metadata_blocks_legacy_writes_instead_of_expiring_silently() {
        let app = App::new();
        let base = state(&app.0).unwrap();
        save(&base.join("leases.json"), &json!([{"owner":"agent","path":"a.brain","tokenHash":"invalid","expiresAt":"invalid"}])).unwrap();
        assert!(crate::edit::writer_lock(&app.0)
            .unwrap_err()
            .contains("malformed"));
    }
}
