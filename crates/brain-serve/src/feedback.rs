//! Bounded durable compiler feedback before browser publication. No arbitrary diagnostic prose or source bodies.
use brain_lex::Json;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
const PRODUCER: &str = "brain-compiler-feedback";
const SLOTS: u64 = 64;
const EVENT_BYTES: usize = 16384;
const STATE_BYTES: usize = 65536;
static TEMP: AtomicU64 = AtomicU64::new(0);
/// Close releases a flock only after all inherited descriptors close. Explicitly unlock the owned lease first.
struct WriterLease(std::fs::File);
impl Drop for WriterLease {
    fn drop(&mut self) { let _ = self.0.unlock(); }
}
fn writer_lease(file: std::fs::File) -> Result<WriterLease, String> {
    for attempt in 0..3 {
        match file.try_lock() {
            Ok(()) => return Ok(WriterLease(file)),
            Err(std::fs::TryLockError::WouldBlock) => return Err("feedback writer is busy; an event may require refresh".into()),
            Err(std::fs::TryLockError::Error(error)) if error.kind() == std::io::ErrorKind::Interrupted && attempt < 2 => continue,
            Err(std::fs::TryLockError::Error(error)) => return Err(format!("feedback writer lock is unavailable ({:?}); inspect local filesystem locking support", error.kind())),
        }
    }
    unreachable!("the final lock attempt returns its result")
}
fn hash(bytes: &[u8]) -> String {
    brain_pack::sha::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn text<'a>(value: &'a Json, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}
fn num(value: &Json, key: &str) -> Option<u64> {
    match value.get(key)? {
        Json::Num(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n < 9e15 => {
            Some(*n as u64)
        }
        _ => None,
    }
}
fn field(value: &Json, key: &str) -> Json {
    value.get(key).cloned().unwrap_or(Json::Null)
}
fn epoch() -> String {
    hash(
        format!(
            "{}:{}:{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            TEMP.fetch_add(1, Ordering::Relaxed)
        )
        .as_bytes(),
    )
}
fn path_name(root: &Path, path: &Path) -> Option<String> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root).ok()?
    } else {
        path
    };
    if relative
        .components()
        .any(|p| !matches!(p, Component::Normal(_)))
    {
        return None;
    }
    let name = relative.to_str()?.replace('\\', "/");
    if name.is_empty()
        || name.len() > 240
        || name.chars().any(char::is_control)
        || name
            .split('/')
            .any(|p| p.starts_with('.') && p != ".brain_env")
        || !brain_check::scan_secrets(&name).is_empty()
    {
        return None;
    }
    Some(name)
}
fn read(path: &Path, limit: usize) -> Result<Option<Json>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("feedback metadata is unavailable".into()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit as u64 {
        return Err("feedback metadata is unsafe or exceeds its budget".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| "cannot read feedback metadata")?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err("feedback metadata changed type".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read feedback metadata")?;
    if bytes.len() > limit {
        return Err("feedback metadata exceeds its read budget".into());
    }
    let value =
        Json::parse(std::str::from_utf8(&bytes).map_err(|_| "feedback metadata is not UTF-8")?)
            .map_err(|_| "feedback metadata JSON is invalid")?;
    if text(&value, "producer") != Some(PRODUCER) || num(&value, "schemaVersion") != Some(1) {
        return Err("unknown feedback producer/schema; preserve the metadata".into());
    }
    Ok(Some(value))
}
fn directory(root: &Path, create: bool) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|_| "app root is unavailable")?;
    let mut dir = root.clone();
    for part in [".brain", "feedback", "events"] {
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
                return Err("feedback directory is unsafe".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !create => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&dir).map_err(|_| "cannot create feedback directory")?;
            }
            Err(_) => return Err("cannot inspect feedback directory".into()),
        }
        #[cfg(unix)]
        if create {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
                .map_err(|_| "cannot protect feedback directory")?;
        }
    }
    if !dir
        .canonicalize()
        .map_err(|_| "feedback directory is unavailable")?
        .starts_with(root)
    {
        return Err("feedback directory escaped app root".into());
    }
    Ok(dir)
}
fn atomic(path: &Path, value: &Json, limit: usize) -> Result<(), String> {
    let bytes = value.stringify();
    if bytes.len() > limit {
        return Err("feedback write exceeds its budget".into());
    }
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return Err("feedback destination is unsafe".into());
    }
    let tmp = path.with_file_name(format!(
        ".feedback-{}-{}.tmp",
        std::process::id(),
        TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|_| "cannot create owned feedback temporary file")?;
    let result = (|| {
        file.write_all(bytes.as_bytes())
            .and_then(|_| file.sync_data())
            .map_err(|_| "cannot persist compiler feedback")?;
        if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
            return Err("feedback destination changed unexpectedly".into());
        }
        fs::rename(&tmp, path).map_err(|_| "cannot atomically publish compiler feedback")?;
        Ok(())
    })();
    let _ = fs::remove_file(tmp);
    result
}
fn empty() -> Json {
    Json::obj([
        ("schemaVersion", Json::Num(1.0)),
        ("producer", Json::str(PRODUCER)),
        ("frameworkVersion", Json::str(env!("CARGO_PKG_VERSION"))),
        ("epoch", Json::str(epoch())),
        ("sequence", Json::Num(0.0)),
        ("active", Json::Obj(Vec::new())),
        ("activeTrackingIncomplete", Json::Bool(false)),
    ])
}
fn state(dir: &Path) -> Result<Json, String> {
    let Some(value) = read(&dir.join("state.json"), STATE_BYTES)? else {
        return Ok(empty());
    };
    if text(&value, "frameworkVersion") != Some(env!("CARGO_PKG_VERSION")) {
        return Ok(empty());
    }
    if !text(&value, "epoch")
        .is_some_and(|e| e.len() == 64 && e.bytes().all(|b| b.is_ascii_hexdigit()))
        || num(&value, "sequence").is_none()
        || !matches!(value.get("active"),Some(Json::Obj(rows))if rows.len()<=128)
    {
        return Err("feedback state is invalid; preserve it for review".into());
    }
    Ok(value)
}
fn set(value: &mut Json, key: &str, field: Json) {
    if let Json::Obj(rows) = value {
        if let Some((_, v)) = rows.iter_mut().find(|(k, _)| k == key) {
            *v = field
        } else {
            rows.push((key.into(), field));
        }
    }
}
fn diagnostics(root: &Path, file: &str, source_hash: Option<&str>, raw: &[String]) -> Vec<Json> {
    raw.iter().take(16).map(|raw|{
        let parsed=if raw.len()<=EVENT_BYTES{Json::parse(raw).ok()}else{None};let parsed=parsed.as_ref();
        let code=parsed.and_then(|d|text(d,"code")).and_then(brain_check::explain);
        let (code,title,fix)=code.map(|(code,title,why,_)|(code,title,why)).unwrap_or(("B0002","Compiler diagnostic requires inspection","Inspect brain check with the complete project context; repair the reported native source error."));
        let location=parsed.and_then(|d|text(d,"file")).and_then(|name|path_name(root,Path::new(name))).unwrap_or_else(||file.into());
        Json::obj([("code",Json::str(code)),("title",Json::str(title)),("file",Json::str(&location)),("line",parsed.and_then(|d|num(d,"line")).filter(|n|*n>0&&*n<=u32::MAX as u64).map(|n|Json::Num(n as f64)).unwrap_or(Json::Null)),("column",parsed.and_then(|d|num(d,"col")).filter(|n|*n>0&&*n<=u32::MAX as u64).map(|n|Json::Num(n as f64)).unwrap_or(Json::Null)),("sourceRevision",if location==file{source_hash.map(Json::str).unwrap_or(Json::Null)}else{Json::Null}),("fix",Json::str(fix)),("fixPrecision",Json::str("catalog guidance; raw values and automatic edits omitted"))])
    }).collect()
}
#[derive(Clone)]
pub struct Spool {
    root: PathBuf,
    dir: PathBuf,
}
impl Spool {
    pub fn new(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|_| "feedback root is unavailable")?;
        let dir = directory(&root, true)?;
        Ok(Self { root, dir })
    }
    /// Returns the persisted event for immediate agent notification, or None for an unchanged diagnostic state.
    pub fn record(
        &self,
        file: &Path,
        source_hash: Option<&str>,
        raw: Option<&[String]>,
    ) -> Result<Option<String>, String> {
        if directory(&self.root, false)? != self.dir {
            return Err("feedback directory changed".into());
        }
        let lock_path = self.dir.join("writer.lock");
        if fs::symlink_metadata(&lock_path)
            .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
        {
            return Err("feedback lock is unsafe".into());
        }
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let lock = options
            .open(lock_path)
            .map_err(|_| "cannot open feedback writer lock")?;
        let _lease = writer_lease(lock)?;
        let file =
            path_name(&self.root, file).ok_or("feedback source is outside the app or unsafe")?;
        let source_hash =
            source_hash.filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()));
        let mut state = state(&self.dir)?;
        // Recover one event whose atomic slot publish completed before its state-index write.
        let old_sequence = num(&state, "sequence").unwrap_or(0);
        let old_epoch = text(&state, "epoch").unwrap_or("").to_string();
        if old_sequence >= 8_999_999_999_999_998 {
            state = empty();
            set(&mut state, "activeTrackingIncomplete", Json::Bool(true));
        } else if let Some(pending) = read(
            &self
                .dir
                .join(format!("{:03}.json", (old_sequence + 1) % SLOTS)),
            EVENT_BYTES,
        )? {
            if text(&pending, "epoch") == Some(old_epoch.as_str())
                && num(&pending, "sequence") == Some(old_sequence + 1)
                && verify_event(&pending)
            {
                let mut active = match field(&state, "active") {
                    Json::Obj(rows) => rows.into_iter().collect::<BTreeMap<_, _>>(),
                    _ => BTreeMap::new(),
                };
                if let Some(name) = text(&pending, "file") {
                    if text(&pending, "type") == Some("compile-resolved") {
                        active.remove(name);
                    } else if active.len() < 128 {
                        let pending_hash = text(&pending, "sourceRevision");
                        let pending_diags = match field(&pending, "diagnostics") {
                            Json::Arr(rows) => rows,
                            _ => Vec::new(),
                        };
                        active.insert(
                            name.into(),
                            Json::str(hash(
                                format!("{:?}|{:?}", pending_hash, pending_diags).as_bytes(),
                            )),
                        );
                    } else {
                        set(&mut state, "activeTrackingIncomplete", Json::Bool(true));
                    }
                }
                set(
                    &mut state,
                    "active",
                    Json::Obj(active.into_iter().collect()),
                );
                set(&mut state, "sequence", Json::Num((old_sequence + 1) as f64));
                set(&mut state, "lastHash", field(&pending, "recordHash"));
                atomic(&self.dir.join("state.json"), &state, STATE_BYTES)?;
            }
        }
        let mut active = match field(&state, "active") {
            Json::Obj(rows) => rows.into_iter().collect::<BTreeMap<_, _>>(),
            _ => return Err("invalid active feedback state".into()),
        };
        let mut diags = raw
            .map(|raw| diagnostics(&self.root, &file, source_hash, raw))
            .unwrap_or_default();
        while Json::Arr(diags.clone()).stringify().len() > 12 * 1024 {
            diags.pop();
        }
        let diagnostics_truncated = raw.is_some_and(|all| all.len() > diags.len());
        let fingerprint = hash(format!("{:?}|{:?}", source_hash, diags).as_bytes());
        if raw.is_none() {
            if active.remove(&file).is_none() {
                return Ok(None);
            }
        } else if active.get(&file).and_then(Json::as_str) == Some(&fingerprint) {
            return Ok(None);
        } else {
            if active.len() >= 128 && !active.contains_key(&file) {
                if let Some(first) = active.keys().next().cloned() {
                    active.remove(&first);
                }
                set(&mut state, "activeTrackingIncomplete", Json::Bool(true));
            }
            active.insert(file.clone(), Json::str(fingerprint));
        }
        let sequence = num(&state, "sequence").unwrap_or(0) + 1;
        let epoch = text(&state, "epoch")
            .ok_or("feedback epoch missing")?
            .to_string();
        let mut event = Json::obj([
            ("schemaVersion", Json::Num(1.0)),
            ("producer", Json::str(PRODUCER)),
            ("epoch", Json::str(&epoch)),
            ("sequence", Json::Num(sequence as f64)),
            (
                "type",
                Json::str(if raw.is_some() {
                    "compile-error"
                } else {
                    "compile-resolved"
                }),
            ),
            ("file", Json::str(&file)),
            (
                "sourceRevision",
                source_hash.map(Json::str).unwrap_or(Json::Null),
            ),
            ("diagnostics", Json::Arr(diags)),
            (
                "diagnosticCount",
                Json::Num(raw.map_or(0, |d| d.len()) as f64),
            ),
            ("diagnosticsTruncated", Json::Bool(diagnostics_truncated)),
            (
                "evidence",
                Json::str("compiler-reported; no application execution"),
            ),
        ]);
        let revision = hash(event.stringify().as_bytes());
        set(&mut event, "recordHash", Json::str(&revision));
        let slot = self.dir.join(format!("{:03}.json", sequence % SLOTS));
        let _ = read(&slot, EVENT_BYTES)?;
        atomic(&slot, &event, EVENT_BYTES)?;
        set(&mut state, "sequence", Json::Num(sequence as f64));
        set(&mut state, "lastHash", Json::str(revision));
        set(
            &mut state,
            "active",
            Json::Obj(active.into_iter().collect()),
        );
        atomic(&self.dir.join("state.json"), &state, STATE_BYTES)?;
        Ok(Some(event.stringify()))
    }
}
fn verify_event(value: &Json) -> bool {
    let expected = text(value, "recordHash");
    let body = match value {
        Json::Obj(rows) => Json::Obj(
            rows.iter()
                .filter(|(k, _)| k != "recordHash")
                .cloned()
                .collect(),
        ),
        _ => return false,
    };
    expected == Some(hash(body.stringify().as_bytes()).as_str())
}
/// Read-only bounded polling. Epoch/sequence cursors detect overwritten slots, resets and a partial publication.
pub fn poll(
    root: &Path,
    after_epoch: Option<&str>,
    after: u64,
    limit: usize,
) -> Result<Json, String> {
    if after >= 9_000_000_000_000_000
        || !(1..=128).contains(&limit)
        || after_epoch.is_some_and(|e| e.len() != 64 || !e.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("invalid feedback epoch/sequence/limit cursor".into());
    }
    let dir = directory(root, false)?;
    if !dir.is_dir() {
        return Ok(Json::obj([
            ("schemaVersion", Json::Num(1.0)),
            ("available", Json::Bool(false)),
            ("events", Json::Arr(Vec::new())),
            ("reset", Json::Bool(false)),
            ("gap", Json::Bool(false)),
        ]));
    }
    let Some(state) = read(&dir.join("state.json"), STATE_BYTES)? else {
        return Ok(Json::obj([
            ("schemaVersion", Json::Num(1.0)),
            ("available", Json::Bool(false)),
            ("events", Json::Arr(Vec::new())),
            ("gap", Json::Bool(true)),
            (
                "reason",
                Json::str(
                    "state missing; an interrupted publication may require a compiler refresh",
                ),
            ),
        ]));
    };
    let current_epoch = text(&state, "epoch").ok_or("feedback epoch missing")?;
    let latest = num(&state, "sequence").ok_or("feedback sequence missing")?;
    let reset = after_epoch.is_some_and(|e| e != current_epoch) || after > latest;
    let after = if reset { 0 } else { after };
    let earliest = latest.saturating_sub(SLOTS - 1).max(1);
    let mut gaps = Vec::new();
    let mut events = Vec::new();
    let mut cursor = after;
    let start = (after + 1).max(earliest);
    for sequence in start..=latest {
        if events.len() >= limit {
            break;
        }
        let slot = dir.join(format!("{:03}.json", sequence % SLOTS));
        match read(&slot, EVENT_BYTES) {
            Ok(Some(record))
                if text(&record, "epoch") == Some(current_epoch)
                    && num(&record, "sequence") == Some(sequence)
                    && verify_event(&record) =>
            {
                events.push(record)
            }
            _ => gaps.push(Json::Num(sequence as f64)),
        }
        cursor = sequence;
    }
    let next_slot = read(
        &dir.join(format!("{:03}.json", (latest + 1) % SLOTS)),
        EVENT_BYTES,
    )
    .ok()
    .flatten();
    let pending = next_slot.as_ref().is_some_and(|record| {
        text(record, "epoch") == Some(current_epoch)
            && num(record, "sequence") == Some(latest + 1)
            && verify_event(record)
    });
    let advanced = read(&dir.join("state.json"), STATE_BYTES)?
        .as_ref()
        .is_some_and(|s| {
            text(s, "epoch") != Some(current_epoch) || num(s, "sequence") != Some(latest)
        });
    Ok(Json::obj([
        ("schemaVersion", Json::Num(1.0)),
        ("available", Json::Bool(true)),
        ("epoch", Json::str(current_epoch)),
        ("latestSequence", Json::Num(latest as f64)),
        ("nextSequence", Json::Num(cursor as f64)),
        ("earliestRetained", Json::Num(earliest as f64)),
        ("events", Json::Arr(events)),
        ("reset", Json::Bool(reset)),
        (
            "gap",
            Json::Bool(reset || after + 1 < earliest || !gaps.is_empty() || pending),
        ),
        ("missingSequences", Json::Arr(gaps)),
        ("pendingPublication", Json::Bool(pending)),
        ("snapshotAdvanced", Json::Bool(advanced)),
        ("hasMore", Json::Bool(cursor < latest)),
        (
            "activeTrackingIncomplete",
            field(&state, "activeTrackingIncomplete"),
        ),
        (
            "limits",
            Json::obj([
                ("slots", Json::Num(SLOTS as f64)),
                ("eventBytes", Json::Num(EVENT_BYTES as f64)),
                ("stateBytes", Json::Num(STATE_BYTES as f64)),
            ]),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct App(PathBuf);
    impl App {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "brain-compiler-feed-{}-{}",
                std::process::id(),
                TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("comp")).unwrap();
            Self(root)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn errors() -> Vec<String> {
        vec!["{\"code\":\"B0201\",\"file\":\"comp/example.brain\",\"line\":3,\"col\":2,\"msg\":\"private-error-value\",\"fix\":\"private-source-value\",\"body\":\"never-copy\"}".into()]
    }
    #[test]
    fn completed_writer_releases_its_lease_even_if_a_child_retains_the_open_file_description() {
        let app = App::new();
        let path = app.0.join("lease.lock");
        let file = OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap();
        let lease = writer_lease(file).unwrap();
        let inherited_description = lease.0.try_clone().unwrap();
        let competitor = OpenOptions::new().read(true).write(true).open(&path).unwrap();
        assert!(matches!(competitor.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        drop(lease);
        let next = writer_lease(competitor).expect("the completed writer must release its lease independently from an inherited descriptor");
        drop(next);
        drop(inherited_description);
    }
    #[test]
    fn durable_redacted_error_then_resolution_are_deduplicated_and_survive_reopen() {
        let app = App::new();
        let spool = Spool::new(&app.0).unwrap();
        let revision = hash(b"synthetic source");
        let raw = errors();
        let event = spool
            .record(Path::new("comp/example.brain"), Some(&revision), Some(&raw))
            .unwrap()
            .unwrap();
        assert!(!event.contains("private-error-value"));
        assert!(!event.contains("private-source-value"));
        assert!(!event.contains("never-copy"));
        assert!(event.contains("B0201"));
        assert!(
            spool
                .record(Path::new("comp/example.brain"), Some(&revision), Some(&raw))
                .unwrap()
                .is_none()
        );
        let spool = Spool::new(&app.0).unwrap();
        let resolved = spool
            .record(Path::new("comp/example.brain"), Some(&hash(b"fixed")), None)
            .unwrap()
            .unwrap();
        assert_eq!(
            text(&Json::parse(&resolved).unwrap(), "type"),
            Some("compile-resolved")
        );
        let result = poll(&app.0, None, 0, 128).unwrap();
        assert_eq!(num(&result, "latestSequence"), Some(2));
        assert_eq!(result.get("gap"), Some(&Json::Bool(false)));
        assert!(matches!(result.get("events"),Some(Json::Arr(rows))if rows.len()==2));
    }
    #[test]
    fn bounded_ring_detects_overwrites_and_epoch_resets_instead_of_claiming_complete_delivery() {
        let app = App::new();
        let spool = Spool::new(&app.0).unwrap();
        for n in 0..70 {
            spool
                .record(
                    Path::new("comp/example.brain"),
                    Some(&hash(n.to_string().as_bytes())),
                    Some(&errors()),
                )
                .unwrap();
        }
        let result = poll(&app.0, None, 0, 128).unwrap();
        assert_eq!(result.get("gap"), Some(&Json::Bool(true)));
        assert_eq!(num(&result, "earliestRetained"), Some(7));
        assert!(matches!(result.get("events"),Some(Json::Arr(rows))if rows.len()==64));
        let reset = poll(&app.0, Some(&"0".repeat(64)), 69, 1).unwrap();
        assert_eq!(reset.get("reset"), Some(&Json::Bool(true)));
        assert_eq!(num(&reset, "nextSequence"), Some(7));
    }
    #[test]
    fn interrupted_slot_publication_is_recovered_without_duplicate_sequences() {
        let app = App::new();
        let spool = Spool::new(&app.0).unwrap();
        spool
            .record(
                Path::new("comp/example.brain"),
                Some(&hash(b"one")),
                Some(&errors()),
            )
            .unwrap();
        let state_path = spool.dir.join("state.json");
        let original = fs::read(&state_path).unwrap();
        let revision = hash(b"two");
        spool
            .record(
                Path::new("comp/example.brain"),
                Some(&revision),
                Some(&errors()),
            )
            .unwrap();
        fs::write(state_path, original).unwrap();
        let partial = poll(&app.0, None, 0, 128).unwrap();
        assert_eq!(partial.get("pendingPublication"), Some(&Json::Bool(true)));
        let reopened = Spool::new(&app.0).unwrap();
        assert!(
            reopened
                .record(
                    Path::new("comp/example.brain"),
                    Some(&revision),
                    Some(&errors())
                )
                .unwrap()
                .is_none()
        );
        let healed = poll(&app.0, None, 0, 128).unwrap();
        assert_eq!(num(&healed, "latestSequence"), Some(2));
        assert_eq!(healed.get("pendingPublication"), Some(&Json::Bool(false)));
    }
    #[test]
    fn corrupted_slots_and_oversized_diagnostics_create_explicit_gaps_or_truncation() {
        let app = App::new();
        let spool = Spool::new(&app.0).unwrap();
        let many = vec![errors()[0].clone(); 30];
        spool
            .record(Path::new("comp/example.brain"), None, Some(&many))
            .unwrap();
        let slot = spool.dir.join("001.json");
        let mut record = read(&slot, EVENT_BYTES).unwrap().unwrap();
        assert_eq!(num(&record, "diagnosticCount"), Some(30));
        assert_eq!(record.get("diagnosticsTruncated"), Some(&Json::Bool(true)));
        set(&mut record, "file", Json::str("comp/edited.brain"));
        atomic(&slot, &record, EVENT_BYTES).unwrap();
        let result = poll(&app.0, None, 0, 64).unwrap();
        assert_eq!(result.get("gap"), Some(&Json::Bool(true)));
        assert!(matches!(result.get("missingSequences"),Some(Json::Arr(rows))if rows.len()==1));
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_store_and_event_slots_cannot_read_or_replace_outside_files() {
        use std::os::unix::fs::symlink;
        let app = App::new();
        let outside = App::new();
        fs::create_dir(app.0.join(".brain")).unwrap();
        symlink(&outside.0, app.0.join(".brain/feedback")).unwrap();
        assert!(Spool::new(&app.0).is_err());
        assert!(!outside.0.join("events").exists());
        let app = App::new();
        let spool = Spool::new(&app.0).unwrap();
        let target = outside.0.join("untouched.json");
        fs::write(&target, "outside-content").unwrap();
        symlink(&target, spool.dir.join("001.json")).unwrap();
        assert!(
            spool
                .record(Path::new("comp/example.brain"), None, Some(&errors()))
                .is_err()
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "outside-content");
    }
}
