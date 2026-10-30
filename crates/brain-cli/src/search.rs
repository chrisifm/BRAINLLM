//! Bounded native source navigation. Stores identifier metadata only, keyed by exact source hashes.
use brain_parse::ast::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component as Part, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
#[cfg(unix)]
use std::time::Duration;
use std::time::Instant;

const MAX_FILES: usize = 2048;
const MAX_ENTRIES: usize = 8192;
const MAX_SOURCE: usize = 262144;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_ROWS: usize = 16384;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_FILES: usize = 128;
#[derive(Clone)]
struct Indexed {
    symbols: Vec<Value>,
    references: Vec<Value>,
    truncated: bool,
}
struct Cached {
    root: PathBuf,
    file: String,
    revision: String,
    index: Indexed,
    bytes: usize,
}
static CACHE: OnceLock<Mutex<VecDeque<Cached>>> = OnceLock::new();
#[derive(Default)]
struct Files {
    paths: Vec<String>,
    entries: usize,
    skipped: usize,
    truncated: bool,
}
fn hash(bytes: &[u8]) -> String {
    brain_pack::sha::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Files) {
    if depth > 32 || out.paths.len() >= MAX_FILES || out.entries >= MAX_ENTRIES {
        out.truncated = true;
        return;
    }
    if dir != root && crate::scaffold::is_app_dir(dir) {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        out.skipped += 1;
        return;
    };
    let mut entries: Vec<_> = rd
        .take(MAX_ENTRIES.saturating_sub(out.entries) + 1)
        .collect();
    if entries.len() > MAX_ENTRIES.saturating_sub(out.entries) {
        out.truncated = true;
        entries.truncate(MAX_ENTRIES - out.entries);
    }
    entries.sort_by_key(|e| e.as_ref().ok().map(|e| e.file_name()));
    for entry in entries {
        if out.paths.len() >= MAX_FILES || out.entries >= MAX_ENTRIES {
            out.truncated = true;
            break;
        }
        out.entries += 1;
        let Ok(entry) = entry else {
            out.skipped += 1;
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            || ["node_modules", "dist", "target", "secrets", "credentials"]
                .contains(&name.to_ascii_lowercase().as_str())
        {
            continue;
        }
        let Ok(ty) = entry.file_type() else {
            out.skipped += 1;
            continue;
        };
        if ty.is_symlink() {
            out.skipped += 1;
            continue;
        }
        if ty.is_dir() {
            walk(root, &entry.path(), depth + 1, out);
        } else if [".brain", ".jsbrain", ".js", ".mjs"]
            .iter()
            .any(|ext| name.ends_with(ext))
        {
            if let Ok(rel) = entry.path().strip_prefix(root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if crate::changes::source_path(root, &rel).is_ok() {
                    out.paths.push(rel);
                } else {
                    out.skipped += 1;
                }
            }
        }
    }
}
fn files(root: &Path) -> Files {
    let mut out = Files::default();
    walk(root, root, 0, &mut out);
    out.paths.sort();
    out
}
fn import_target(file: &str, source: &str) -> Option<String> {
    if !source.starts_with('.') || source.contains(['\\', '?', '#', ':']) {
        return None;
    }
    let mut path = PathBuf::new();
    for part in Path::new(file).parent()?.join(source).components() {
        match part {
            Part::Normal(p) => path.push(p),
            Part::CurDir => {}
            Part::ParentDir => {
                if !path.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let rel = path.to_str()?.replace('\\', "/");
    crate::optimization_feedback::public_path(&rel).then_some(rel)
}
fn record(file: &str, name: &str, kind: &str, span: brain_parse::Span, revision: &str) -> Value {
    json!({"id":format!("{file}#{kind}:{}:{}",span.start,span.end),"name":name,"kind":kind,"file":file,"line":span.line,"column":span.col,"parent":format!("{file}#file"),"parentKind":"source-file","sourceRevision":revision})
}
fn index(file: &str, source: &str, revision: &str) -> Result<Indexed, String> {
    let parse_path = if file.ends_with(".js") || file.ends_with(".mjs") {
        Path::new("source.jsbrain")
    } else {
        Path::new(file)
    };
    let c = brain_parse::parse(source, parse_path)
        .map_err(|_| "source cannot be indexed by the native parser")?;
    let (declarations, reads) = crate::optimization_feedback::source_index(&c)?;
    let owner_kind = if c.level == Level::View {
        "view"
    } else {
        "component"
    };
    let owner_id = (c.kind != FileKind::Module)
        .then(|| format!("{file}#{owner_kind}:{}:{}", c.span.start, c.span.end));
    let grouped = |row: &mut Value| {
        if let Some(id) = &owner_id {
            row["parent"] = json!(id);
            row["parentKind"] = json!(owner_kind);
            row["component"] = json!(c.name);
        }
    };
    let mut symbols = vec![];
    let mut references = vec![];
    let mut counts = BTreeMap::new();
    let mut exports = BTreeSet::new();
    for d in &declarations {
        if d.kind != "method" {
            *counts.entry(d.name.clone()).or_insert(0usize) += 1;
        }
    }
    for stmt in &c.script {
        if let Stmt::Export { decl, .. } = stmt {
            match decl.as_ref() {
                Stmt::Var(v) => {
                    for d in &v.decls {
                        let mut names = vec![];
                        d.pat.bound_names(&mut names);
                        for name in names {
                            exports.insert(name.span.start);
                        }
                    }
                }
                Stmt::Func(f) => {
                    if let Some(name) = &f.name {
                        exports.insert(name.span.start);
                    }
                }
                _ => {}
            }
        }
    }
    for stmt in &c.script {
        if let Stmt::ExportDefault { expr, span } = stmt {
            let mut row = record(
                file,
                "default",
                if matches!(expr.unparen(), Expr::Func(_)) {
                    "function"
                } else {
                    "export"
                },
                *span,
                revision,
            );
            row["exported"] = json!(true);
            row["bindingKind"] = json!("export-default");
            row["precision"] = json!("export-statement");
            symbols.push(row);
            if let Expr::Func(f) = expr.unparen() {
                if let Some(i) = &f.name {
                    exports.insert(i.span.start);
                }
            }
        }
    }
    let mut aliases: BTreeMap<String, (String, String, u32)> = BTreeMap::new();
    for stmt in &c.script {
        if let Stmt::Import(i) = stmt {
            if let Some(target) = import_target(file, &i.source) {
                for (local, imported) in i
                    .default
                    .iter()
                    .map(|i| (i, "default".to_string()))
                    .chain(i.namespace.iter().map(|i| (i, "*".to_string())))
                    .chain(
                        i.named
                            .iter()
                            .map(|(imported, local)| (local, imported.name.clone())),
                    )
                {
                    aliases.insert(
                        local.name.clone(),
                        (target.clone(), imported, local.span.start),
                    );
                }
            }
        }
    }
    if c.kind != FileKind::Module {
        let kind = if c.level == Level::View {
            "view"
        } else {
            "component"
        };
        symbols.push(record(file, &c.name, kind, c.span, revision));
    }
    for d in declarations.into_iter().take(MAX_ROWS) {
        let kind = if d.callable { "function" } else { d.kind };
        let mut row = record(file, &d.name, kind, d.span, revision);
        grouped(&mut row);
        row["bindingKind"] = json!(d.kind);
        row["exported"] = json!(exports.contains(&d.span.start));
        if let Some((target, imported, start)) = aliases
            .get(&d.name)
            .filter(|(_, _, start)| *start == d.span.start)
        {
            row["importResolution"] = json!({"file":target,"name":imported,"qualification":"literal-import-path","bindingStart":start});
        }
        let mut decl_ref = row.clone();
        decl_ref["role"] = json!("declaration");
        decl_ref["resolution"] = json!("source-declaration");
        references.push(decl_ref);
        symbols.push(row);
    }
    for read in reads
        .into_iter()
        .take(MAX_ROWS.saturating_sub(references.len()))
    {
        let mut row = record(file, &read.name, "reference", read.span, revision);
        grouped(&mut row);
        row["role"] = json!("identifier-use");
        row["resolution"] = json!("AST identifier occurrence; full binding resolution unavailable");
        if counts.get(&read.name) == Some(&1) {
            if let Some((target, imported, _)) = aliases.get(&read.name) {
                row["importResolution"] = json!({"file":target,"name":imported,"qualification":"unique-import-binding-and-literal-path"});
            }
        }
        references.push(row);
    }
    symbols.sort_by_key(|v| {
        (
            v["line"].as_u64().unwrap_or(0),
            v["column"].as_u64().unwrap_or(0),
            v["kind"].as_str().unwrap_or("").to_owned(),
        )
    });
    references.sort_by_key(|v| {
        (
            v["line"].as_u64().unwrap_or(0),
            v["column"].as_u64().unwrap_or(0),
            v["role"].as_str().unwrap_or("").to_owned(),
        )
    });
    let truncated = symbols.len() >= MAX_ROWS || references.len() >= MAX_ROWS;
    Ok(Indexed {
        symbols,
        references,
        truncated,
    })
}
fn cached(
    root: &Path,
    file: &str,
    source: &str,
    revision: &str,
) -> Result<(Indexed, bool), String> {
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Ok(mut entries) = cache.lock() {
        if let Some(i) = entries
            .iter()
            .position(|e| e.root == root && e.file == file && e.revision == revision)
        {
            let entry = entries.remove(i).unwrap();
            let result = entry.index.clone();
            entries.push_back(entry);
            return Ok((result, true));
        }
    }
    let indexed = index(file, source, revision)?;
    let bytes =
        serde_json::to_vec(&json!({"symbols":indexed.symbols,"references":indexed.references}))
            .map_err(|_| "index metadata serialization failed")?
            .len();
    if bytes <= MAX_CACHE_BYTES {
        if let Ok(mut entries) = cache.lock() {
            entries.retain(|e| e.root != root || e.file != file);
            while entries.len() >= MAX_CACHE_FILES
                || entries.iter().map(|e| e.bytes).sum::<usize>() + bytes > MAX_CACHE_BYTES
            {
                if entries.pop_front().is_none() {
                    break;
                }
            }
            entries.push_back(Cached {
                root: root.into(),
                file: file.into(),
                revision: revision.into(),
                index: indexed.clone(),
                bytes,
            });
        }
    }
    Ok((indexed, false))
}
fn integer(args: &Value, key: &str, default: usize, max: usize) -> Result<usize, String> {
    match args.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| *n <= max as u64)
            .map(|n| n as usize)
            .ok_or_else(|| format!("{key} must be an integer in 0..{max}")),
    }
}
/// Snapshot queries read current guarded bytes; only metadata, never source bodies, is retained.
pub fn query(root: &Path, op: &str, args: &Value) -> Result<Value, String> {
    if !args.is_object() {
        return Err("search arguments must be an object".into());
    }
    if !matches!(op, "find" | "symbols" | "refs") {
        return Err("search operation must be find, symbols or refs".into());
    }
    let root = root
        .canonicalize()
        .map_err(|_| "project root is unavailable")?;
    if !root.is_dir() {
        return Err("project root must be a directory".into());
    }
    if args.get("kind").is_some_and(|v| !v.is_string()) {
        return Err("kind must be a string".into());
    }
    if args.get("metrics").is_some_and(|v| !v.is_boolean()) {
        return Err("metrics must be a boolean".into());
    }
    let limit = integer(args, "limit", 50, 200)?;
    if limit == 0 {
        return Err("limit must be 1..200".into());
    }
    let cursor = integer(args, "cursor", 0, MAX_ROWS)?;
    let key = match op {
        "find" => "query",
        "symbols" => "file",
        _ => "name",
    };
    let term = args
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 240)
        .ok_or_else(|| format!("{key} must be nonempty and at most 240 bytes"))?;
    if !brain_check::scan_secrets(term).is_empty() {
        return Err("search query contains a secret-shaped token".into());
    }
    let kind = args.get("kind").and_then(Value::as_str);
    if kind.is_some_and(|k| {
        ![
            "component",
            "view",
            "function",
            "variable",
            "import",
            "export",
            "prop",
            "reference",
        ]
        .contains(&k)
    }) {
        return Err("unknown search kind".into());
    }
    let began = Instant::now();
    let mut scan = if op == "symbols" {
        crate::changes::source_path(&root, term)?;
        Files {
            paths: vec![term.into()],
            ..Files::default()
        }
    } else {
        files(&root)
    };
    let mut rows = Vec::new();
    let mut bytes = 0usize;
    let mut parsed = 0usize;
    let mut hits = 0usize;
    let mut skipped = vec![];
    let folded = term.to_lowercase();
    for file in &scan.paths {
        let remaining = MAX_BYTES.saturating_sub(bytes);
        if remaining == 0 || rows.len() >= MAX_ROWS {
            scan.truncated = true;
            break;
        }
        let source = match crate::map::read_source_limit(&root, file, remaining.min(MAX_SOURCE)) {
            Ok(s) => s,
            Err(_) => {
                scan.skipped += 1;
                if skipped.len() < 64 {
                    skipped.push(json!({"file":file,"status":"unknown","reason":"guarded source read rejected or exceeded its budget"}));
                }
                continue;
            }
        };
        bytes += source.len();
        let revision = hash(source.as_bytes());
        let (mut indexed, hit) = match cached(&root, file, &source, &revision) {
            Ok(indexed) => indexed,
            Err(_) => {
                scan.skipped += 1;
                if skipped.len() < 64 {
                    skipped.push(json!({"file":file,"status":"unknown","reason":"native parser or AST metadata budget cannot index this file"}));
                }
                continue;
            }
        };
        if hit {
            hits += 1;
        } else {
            parsed += 1;
        }
        scan.truncated |= indexed.truncated;
        let candidates = if op == "refs" {
            &mut indexed.references
        } else {
            &mut indexed.symbols
        };
        for mut row in candidates.drain(..) {
            if let Some(target) = row["importResolution"]["file"].as_str().map(str::to_owned) {
                match crate::changes::source_path(&root, &target) {
                    Ok(path) => row["importResolution"]["targetAvailable"] = json!(path.is_file()),
                    Err(_) => {
                        row.as_object_mut().unwrap().remove("importResolution");
                        row["importResolutionUnavailable"] = json!(true);
                    }
                }
            }
            let matches = match op {
                "symbols" => true,
                "refs" => row["name"].as_str() == Some(term),
                _ => {
                    row["name"]
                        .as_str()
                        .is_some_and(|n| n.to_lowercase().contains(&folded))
                        || file.to_lowercase().contains(&folded)
                }
            };
            if matches
                && kind.is_none_or(|k| {
                    if k == "export" {
                        row["exported"] == true
                    } else {
                        row["kind"].as_str() == Some(k)
                    }
                })
            {
                if rows.len() >= MAX_ROWS {
                    scan.truncated = true;
                    break;
                }
                rows.push(row);
            }
        }
    }
    rows.sort_by_key(|v| {
        (
            v["file"].as_str().unwrap_or("").to_owned(),
            v["line"].as_u64().unwrap_or(0),
            v["column"].as_u64().unwrap_or(0),
            v["kind"].as_str().unwrap_or("").to_owned(),
        )
    });
    let total = rows.len();
    let records: Vec<_> = rows.into_iter().skip(cursor).take(limit).collect();
    let mut result = json!({"schemaVersion":1,"operation":op,"records":records,"total":total,"cursor":cursor,"limit":limit,"nextCursor":(cursor.saturating_add(limit)<total).then_some(cursor+limit),"coverage":{"complete":!scan.truncated&&scan.skipped==0,"truncated":scan.truncated,"skippedCount":scan.skipped,"skipped":skipped,"files":scan.paths.len(),"readBytes":bytes},"qualification":"Source metadata snapshot; references are AST identifier occurrences, not a complete binding/call graph. Import targets are attached only when their literal local path and unique binding are proven.","limits":{"files":MAX_FILES,"directoryEntries":MAX_ENTRIES,"depth":32,"sourceBytes":MAX_SOURCE,"readBytes":MAX_BYTES,"records":MAX_ROWS,"page":200,"cacheFiles":MAX_CACHE_FILES,"cacheMetadataBytes":MAX_CACHE_BYTES}});
    if args["metrics"] == true {
        result["metrics"] = json!({"parsedFiles":parsed,"cacheHits":hits,"elapsedMicros":began.elapsed().as_micros().min(u64::MAX as u128)as u64});
    }
    Ok(result)
}
fn print(result: &Value, json_output: bool) {
    if json_output {
        println!("{result}");
        return;
    }
    for row in result["records"].as_array().into_iter().flatten() {
        println!(
            "{}:{}:{}\t{}\t{}",
            row["file"].as_str().unwrap_or("?"),
            row["line"].as_u64().unwrap_or(0),
            row["column"].as_u64().unwrap_or(0),
            row["kind"].as_str().unwrap_or("?"),
            row["name"].as_str().unwrap_or("?")
        );
    }
    if result["coverage"]["complete"] == false {
        eprintln!(
            "Search coverage is incomplete; inspect --json coverage for bounds and skipped sources."
        );
    }
}
#[cfg(unix)]
fn watch(root: &Path, op: &str, args: &Value) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STOP: AtomicBool = AtomicBool::new(false);
    extern "C" fn stop(_: libc::c_int) {
        STOP.store(true, Ordering::Relaxed);
    }
    fn output(value: &Value, began: Instant) -> Result<bool, String> {
        let mut bytes =
            serde_json::to_vec(value).map_err(|_| "cannot serialize search snapshot")?;
        bytes.push(b'\n');
        let mut at = 0;
        while at < bytes.len()
            && began.elapsed() < Duration::from_secs(30)
            && !STOP.load(Ordering::Relaxed)
        {
            let mut fd = libc::pollfd {
                fd: libc::STDOUT_FILENO,
                events: libc::POLLOUT,
                revents: 0,
            };
            if unsafe { libc::poll(&mut fd, 1, 100) } <= 0 {
                continue;
            }
            if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                return Ok(false);
            }
            if fd.revents & libc::POLLOUT != 0 {
                // Small writes after readiness avoid retaining a blocked output queue or mutating shared fd flags.
                let end = (at + 512).min(bytes.len());
                let n = unsafe {
                    libc::write(
                        libc::STDOUT_FILENO,
                        bytes[at..end].as_ptr().cast(),
                        end - at,
                    )
                };
                if n > 0 {
                    at += n as usize;
                } else if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                {
                    return Ok(false);
                }
            }
        }
        Ok(at == bytes.len())
    }
    struct Restore(libc::sighandler_t);
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe {
                libc::signal(libc::SIGINT, self.0);
            }
        }
    }
    STOP.store(false, Ordering::Relaxed);
    let old = unsafe { libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t) };
    if old == libc::SIG_ERR {
        return Err("cannot own the watch interrupt handler".into());
    }
    let _restore = Restore(old);
    let began = Instant::now();
    let mut emitted = 0usize;
    let mut last = String::new();
    let mut pending = None;
    while began.elapsed() < Duration::from_secs(30) && emitted < 20 && !STOP.load(Ordering::Relaxed)
    {
        let scan = files(root);
        let mut stamp = String::new();
        for file in &scan.paths {
            if let Ok(meta) = std::fs::metadata(root.join(file)) {
                stamp.push_str(&format!(
                    "{file}:{}:{:?}\n",
                    meta.len(),
                    meta.modified().ok()
                ));
            }
        }
        let current = hash(stamp.as_bytes());
        if emitted == 0 || current != last {
            if emitted == 0
                || pending
                    .as_ref()
                    .is_some_and(|(old, at): &(String, Instant)| {
                        *old == current && at.elapsed() >= Duration::from_millis(150)
                    })
            {
                let mut result = query(root, op, args)?;
                result["event"] = json!("source-snapshot");
                result["sequence"] = json!(emitted + 1);
                if !output(&result, began)? {
                    break;
                }
                emitted += 1;
                last = current;
                pending = None;
            } else if pending
                .as_ref()
                .is_none_or(|(old, _): &(String, Instant)| *old != current)
            {
                pending = Some((current, Instant::now()));
            }
        }
        let mut poll = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, 0) } > 0 {
            if poll.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                break;
            }
            if poll.revents & libc::POLLIN != 0 {
                let mut input = [0u8; 1024];
                if unsafe { libc::read(libc::STDIN_FILENO, input.as_mut_ptr().cast(), input.len()) }
                    == 0
                {
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
#[cfg(not(unix))]
fn watch(_root: &Path, _op: &str, _args: &Value) -> Result<(), String> {
    Err("bounded search watch currently requires Unix signal/stdin polling; snapshot queries remain available".into())
}
/// `args` starts with find/symbols/refs; no compilation or browser process is started.
pub fn run(args: &[String]) -> Result<(), String> {
    let op = args
        .first()
        .map(String::as_str)
        .ok_or("search operation required")?;
    let key = match op {
        "find" => "query",
        "symbols" => "file",
        "refs" => "name",
        _ => return Err("search operation must be find, symbols or refs".into()),
    };
    let term = args
        .get(1)
        .filter(|v| !v.starts_with("--"))
        .ok_or("search term or source file required")?;
    let mut request = json!({key:term});
    let mut json_output = false;
    let mut watching = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json_output = true,
            "--watch" => watching = true,
            "--kind" | "--limit" | "--cursor" => {
                let flag = args[index].trim_start_matches("--");
                index += 1;
                let value = args.get(index).ok_or("search flag needs a value")?;
                request[flag] = if flag == "kind" {
                    json!(value)
                } else {
                    json!(
                        value
                            .parse::<u64>()
                            .map_err(|_| "search limit/cursor must be integers")?
                    )
                };
            }
            _ => return Err("unknown search option".into()),
        }
        index += 1;
    }
    let root = crate::app_root()?;
    if watching {
        watch(&root, op, &request)
    } else {
        let result = query(&root, op, &request)?;
        print(&result, json_output);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "brain-search-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(root.join("comp")).unwrap();
            std::fs::write(root.join("comp/card.brain"),"<meta props=\"title:str\"><script>import { format as render } from '../lib.jsbrain'; const click = () => { const inner = 1; return render(inner) }</script><p>{title}</p>").unwrap();
            std::fs::write(
                root.join("lib.jsbrain"),
                "export function format(value) { return value }",
            )
            .unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn finds_nested_callable_props_and_static_import_alias() {
        let f = Fixture::new();
        let r = query(&f.0, "symbols", &json!({"file":"comp/card.brain"})).unwrap();
        let rows = r["records"].as_array().unwrap();
        assert!(
            rows.iter()
                .any(|r| r["name"] == "click" && r["kind"] == "function")
        );
        assert!(
            rows.iter()
                .any(|r| r["name"] == "inner" && r["kind"] == "variable")
        );
        assert!(
            rows.iter()
                .any(|r| r["name"] == "title" && r["kind"] == "prop")
        );
        let r = query(&f.0, "refs", &json!({"name":"render"})).unwrap();
        assert!(r["records"].as_array().unwrap().iter().any(|r|r["role"]=="identifier-use"&&r["importResolution"]["name"]=="format"));
    }
    #[test]
    fn changed_content_invalidates_metadata_cache_without_returning_bodies() {
        let f = Fixture::new();
        let first = query(
            &f.0,
            "symbols",
            &json!({"file":"lib.jsbrain","metrics":true}),
        )
        .unwrap();
        let warm = query(
            &f.0,
            "symbols",
            &json!({"file":"lib.jsbrain","metrics":true}),
        )
        .unwrap();
        assert_eq!(warm["metrics"]["cacheHits"], 1);
        assert_eq!(warm["metrics"]["parsedFiles"], 0);
        std::fs::write(
            f.0.join("lib.jsbrain"),
            "export function replaced(value) { return value }",
        )
        .unwrap();
        let changed = query(
            &f.0,
            "symbols",
            &json!({"file":"lib.jsbrain","metrics":true}),
        )
        .unwrap();
        assert_eq!(changed["metrics"]["parsedFiles"], 1);
        assert_ne!(
            first["records"][0]["sourceRevision"],
            changed["records"][0]["sourceRevision"]
        );
        assert!(!changed.to_string().contains("return value"));
        eprintln!(
            "native source fixture timing: cold={}us warm={}us changed={}us",
            first["metrics"]["elapsedMicros"],
            warm["metrics"]["elapsedMicros"],
            changed["metrics"]["elapsedMicros"]
        );
    }
    #[test]
    fn rejects_private_paths_and_pages_deterministically() {
        let f = Fixture::new();
        assert!(query(&f.0, "symbols", &json!({"file":"../.env"})).is_err());
        let a = query(&f.0, "find", &json!({"query":"card","limit":1})).unwrap();
        assert_eq!(a["records"].as_array().unwrap().len(), 1);
        assert_eq!(a["nextCursor"], 1);
        let b = query(&f.0, "find", &json!({"query":"card","limit":1,"cursor":1})).unwrap();
        assert_ne!(a["records"][0]["id"], b["records"][0]["id"]);
    }
    #[test]
    fn local_shadowing_prevents_proven_alias_resolution() {
        let f = Fixture::new();
        std::fs::write(f.0.join("lib.jsbrain"),"import { format as value } from './other.jsbrain'; function local(value) { return value } export const output = value").unwrap();
        std::fs::write(f.0.join("other.jsbrain"), "export const format = 1").unwrap();
        let r = query(&f.0, "refs", &json!({"name":"value"})).unwrap();
        assert!(
            r["records"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["role"] == "identifier-use")
                .all(|r| r.get("importResolution").is_none())
        );
    }
    #[test]
    fn alias_availability_is_checked_after_metadata_cache_hits() {
        let f = Fixture::new();
        let args = json!({"name":"render"});
        let before = query(&f.0, "refs", &args).unwrap();
        assert!(
            before["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["importResolution"]["targetAvailable"] == true)
        );
        std::fs::remove_file(f.0.join("lib.jsbrain")).unwrap();
        let after = query(&f.0, "refs", &args).unwrap();
        assert!(
            after["records"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r.get("importResolution").is_some())
                .all(|r| r["importResolution"]["targetAvailable"] == false)
        );
    }
    #[test]
    fn secret_and_oversized_sources_have_explicit_incomplete_coverage() {
        let f = Fixture::new();
        let source = format!(
            "export const visible = '{}{}{}'",
            "sk_",
            "live_",
            "b".repeat(32)
        );
        std::fs::write(f.0.join("lib.jsbrain"), source).unwrap();
        let r = query(&f.0, "find", &json!({"query":"visible"})).unwrap();
        assert_eq!(r["coverage"]["complete"], false);
        assert_eq!(r["records"].as_array().unwrap().len(), 0);
        std::fs::write(f.0.join("lib.jsbrain"), "a".repeat(MAX_SOURCE + 1)).unwrap();
        let r = query(&f.0, "symbols", &json!({"file":"lib.jsbrain"})).unwrap();
        assert_eq!(r["coverage"]["complete"], false);
    }
    #[test]
    fn default_exports_and_object_methods_are_discoverable_without_guessing_bindings() {
        let f = Fixture::new();
        std::fs::write(
            f.0.join("lib.jsbrain"),
            "export default { action(value) { return value } }",
        )
        .unwrap();
        let r = query(&f.0, "symbols", &json!({"file":"lib.jsbrain"})).unwrap();
        assert!(
            r["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["name"] == "default" && r["exported"] == true)
        );
        assert!(
            r["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["name"] == "action" && r["kind"] == "function")
        );
        let r = query(&f.0, "find", &json!({"query":"default","kind":"export"})).unwrap();
        assert_eq!(r["records"].as_array().unwrap().len(), 1);
    }
}
