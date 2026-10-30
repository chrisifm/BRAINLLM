//! Private, bounded source-contract manifests. No application code, resources or environment files are executed.
use brain_parse::ast::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component as Part, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_FILES: usize = 500;
const MAX_SOURCE: usize = 262144;
const MAX_READ: usize = 16 * 1024 * 1024;
pub(crate) const MAX_METADATA: usize = 2 * 1024 * 1024;
const PRODUCER: &str = "brain-project-manifests";
static TEMP: AtomicU64 = AtomicU64::new(0);
pub(crate) fn hash(bytes: &[u8]) -> String {
    brain_pack::sha::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(crate) fn private_dir(root: &Path, leaf: &str) -> Result<PathBuf, String> {
    if !["manifests", "design", "history", "quality"].contains(&leaf) {
        return Err("unknown private metadata namespace".into());
    }
    let root = root.canonicalize().map_err(|_| "app root is unavailable")?;
    let mut dir = root.clone();
    for part in [".brain", leaf] {
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                return Err("private metadata directory is unsafe".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&dir).map_err(|_| "cannot create private metadata directory")?;
            }
            Err(_) => return Err("cannot inspect private metadata directory".into()),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
                .map_err(|_| "cannot protect private metadata directory")?;
        }
    }
    if !dir
        .canonicalize()
        .map_err(|_| "private metadata path is unavailable")?
        .starts_with(root)
    {
        return Err("private metadata escaped app root".into());
    }
    Ok(dir)
}
pub(crate) fn read_json(path: &Path, limit: usize) -> Result<Value, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "metadata file is unavailable")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit as u64 {
        return Err("metadata file is unsafe or exceeds its budget".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| "cannot read metadata")?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err("opened metadata is not a regular file".into());
    }
    let mut bytes = vec![];
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read metadata")?;
    if bytes.len() > limit {
        return Err("metadata exceeds its read budget".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "metadata JSON is invalid".into())
}
pub(crate) fn lock(dir: &Path) -> Result<File, String> {
    let path = dir.join("writer.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
        return Err("metadata lock is unsafe".into());
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
    let file = options.open(path).map_err(|_| "cannot own metadata lock")?;
    file.try_lock()
        .map_err(|_| "metadata refresh is busy; retry shortly".to_string())?;
    Ok(file)
}
pub(crate) fn atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_METADATA {
        return Err("metadata output exceeds 2 MiB".into());
    }
    let parent = path.parent().ok_or("metadata parent is missing")?;
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
        return Err("metadata destination is unsafe".into());
    }
    let temp = parent.join(format!(
        ".write-{}-{}.tmp",
        std::process::id(),
        TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut owned = false;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| "cannot create owned metadata temporary file")?;
        owned = true;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "cannot persist metadata")?;
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
            return Err("metadata destination changed unexpectedly".into());
        }
        fs::rename(&temp, path).map_err(|_| "cannot replace metadata atomically")?;
        Ok(())
    })();
    if owned {
        let _ = fs::remove_file(temp);
    }
    result
}
#[derive(Default)]
struct Inputs {
    files: BTreeMap<String, String>,
    revisions: BTreeMap<String, String>,
    skipped: usize,
    truncated: bool,
    entries: usize,
    bytes: usize,
}
fn walk(root: &Path, dir: &Path, depth: usize, paths: &mut Vec<String>, inputs: &mut Inputs) {
    if depth > 32 || paths.len() >= MAX_FILES || inputs.entries >= 4096 {
        inputs.truncated = true;
        return;
    }
    if dir != root && crate::scaffold::is_app_dir(dir) {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        inputs.skipped += 1;
        return;
    };
    let remaining = 4096usize.saturating_sub(inputs.entries);
    let mut entries: Vec<_> = rd.take(remaining + 1).collect();
    if entries.len() > remaining {
        inputs.truncated = true;
        entries.truncate(remaining);
    }
    entries.sort_by_key(|e| e.as_ref().ok().map(|e| e.file_name()));
    for entry in entries {
        if paths.len() >= MAX_FILES {
            inputs.truncated = true;
            break;
        }
        inputs.entries += 1;
        let Ok(entry) = entry else {
            inputs.skipped += 1;
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            || ["dist", "target", "node_modules", "secrets", "credentials"].contains(&name.as_str())
        {
            continue;
        }
        let Ok(ty) = entry.file_type() else {
            inputs.skipped += 1;
            continue;
        };
        if ty.is_symlink() {
            inputs.skipped += 1;
            continue;
        }
        if ty.is_dir() {
            walk(root, &entry.path(), depth + 1, paths, inputs);
        } else if [
            ".brain",
            ".jsbrain",
            ".cssbrain",
            ".brain.css",
            ".js",
            ".mjs",
        ]
        .iter()
        .any(|ext| name.ends_with(ext))
            && !name.ends_with(".test.brain")
        {
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if crate::optimization_feedback::public_path(&rel) {
                paths.push(rel);
            } else {
                inputs.skipped += 1;
            }
        }
    }
}
fn inputs(root: &Path) -> Result<Inputs, String> {
    let mut result = Inputs::default();
    let mut paths = vec![];
    walk(root, root, 0, &mut paths, &mut result);
    for name in ["design.brain.json", "brain.json"] {
        if root.join(name).is_file() {
            paths.push(name.into());
        }
    }
    paths.sort();
    paths.dedup();
    if paths.len() > MAX_FILES {
        result.truncated = true;
        paths.truncate(MAX_FILES);
    }
    let began = Instant::now();
    for file in paths {
        if result.bytes >= MAX_READ || began.elapsed() > Duration::from_secs(15) {
            result.truncated = true;
            break;
        }
        match crate::map::read_source_limit(root, &file, MAX_SOURCE.min(MAX_READ - result.bytes)) {
            Ok(source) => {
                result.bytes += source.len();
                result
                    .revisions
                    .insert(file.clone(), hash(source.as_bytes()));
                result.files.insert(file, source);
            }
            Err(_) => {
                result.skipped += 1;
            }
        }
    }
    Ok(result)
}
fn normalized_import(root: &Path, file: &str, value: &str) -> Option<String> {
    if !value.starts_with('.') || value.contains(['\\', '?', '#', ':']) {
        return None;
    }
    let mut path = PathBuf::new();
    for p in Path::new(file).parent()?.join(value).components() {
        match p {
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
    crate::optimization_feedback::public_path(&rel)
        .then(|| crate::changes::source_path(root, &rel).ok())
        .flatten()
        .map(|_| rel)
}
fn names(c: &brain_parse::ast::Component, key: &str) -> Vec<String> {
    brain_check::meta_names(c, key)
        .into_iter()
        .filter(|n| {
            n.len() <= 80
                && n.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
        .take(64)
        .collect()
}
fn tokens_from(value: &str, out: &mut BTreeSet<String>) {
    if let Ok(sheet) = brain_css::parse_str(&format!("value:{value};"), "inline.cssbrain") {
        for (name, _) in brain_css::var_refs(&sheet) {
            if name.len() <= 80 {
                out.insert(name);
            }
        }
    }
}
fn nodes(
    items: &[Node],
    slots: &mut BTreeSet<String>,
    states: &mut Vec<Value>,
    tokens: &mut BTreeSet<String>,
) {
    for n in items {
        match n {
            Node::Slot(s) => {
                slots.insert(s.name.clone().unwrap_or_else(|| "default".into()));
                for a in &s.attrs {
                    if let AttrValue::Text(t) = &a.value {
                        if a.kind == AttrKind::Style {
                            tokens_from(t, tokens);
                        }
                    }
                }
            }
            Node::Element(e) => {
                for a in &e.attrs {
                    if a.name == "disabled" {
                        states.push(json!({"state":"disabled","kind":"attribute","line":a.span.line,"column":a.span.col}));
                    }
                    if a.kind == AttrKind::Style {
                        if let AttrValue::Text(t) = &a.value {
                            tokens_from(t, tokens);
                        }
                    } else if a.kind == AttrKind::Plain && a.name == "style" {
                        if let AttrValue::Text(t) = &a.value {
                            if let Ok(sheet) = brain_css::parse_str(t, "inline.cssbrain") {
                                tokens.extend(
                                    brain_css::var_refs(&sheet)
                                        .into_iter()
                                        .map(|(name, _)| name),
                                );
                            }
                        }
                    }
                }
                nodes(&e.children, slots, states, tokens);
            }
            Node::ComponentRef(c) => nodes(&c.children, slots, states, tokens),
            Node::SlotScope(s) => nodes(&s.body, slots, states, tokens),
            Node::If(b) => {
                for branch in &b.branches {
                    nodes(&branch.body, slots, states, tokens);
                }
                if let Some(body) = &b.otherwise {
                    nodes(body, slots, states, tokens);
                }
            }
            Node::Each(b) => {
                nodes(&b.body, slots, states, tokens);
                if let Some(body) = &b.empty {
                    states.push(json!({"state":"empty","kind":"each-empty-branch","line":b.span.line,"column":b.span.col,"represented":!body.is_empty()}));
                    nodes(body, slots, states, tokens);
                }
            }
            Node::Await(b) => {
                states.push(json!({"state":"loading","kind":"await-pending-branch","line":b.span.line,"column":b.span.col,"represented":!b.pending.is_empty()}));
                nodes(&b.pending, slots, states, tokens);
                for (state, branch) in [("success", &b.then), ("error", &b.catch)] {
                    if let Some(branch) = branch {
                        states.push(json!({"state":state,"kind":"await-branch","line":branch.span.line,"column":branch.span.col,"represented":!branch.body.is_empty()}));
                        nodes(&branch.body, slots, states, tokens);
                    }
                }
            }
            _ => {}
        }
    }
}
fn component(root: &Path, file: &str, source: &str, revision: &str) -> Option<Value> {
    if !file.ends_with(".brain") {
        return None;
    }
    let c = match brain_parse::parse(source, Path::new(file)) {
        Ok(c) => c,
        Err(diags) => {
            return Some(
                json!({"schemaVersion":1,"producer":PRODUCER,"file":file,"name":Path::new(file).file_stem().unwrap_or_default().to_string_lossy(),"kind":"unknown","revision":revision,"contract":Value::Null,"diagnostics":{"confidence":"parse-failed","semanticCheck":"not-run","items":diags.iter().take(32).map(|d|json!({"code":d.code,"line":d.line,"column":d.col})).collect::<Vec<_>>()},"confidence":"unknown"}),
            );
        }
    };
    let imports:Vec<_>=c.script.iter().filter_map(|s|if let Stmt::Import(i)=s{Some(match normalized_import(root,file,&i.source){Some(path)=>json!({"file":path,"line":i.span.line,"column":i.span.col,"confidence":"literal-path"}),None=>json!({"file":Value::Null,"line":i.span.line,"column":i.span.col,"confidence":"external-or-unresolved"})})}else{None}).take(128).collect();
    let mut slots = BTreeSet::new();
    let mut states = vec![];
    let mut used = BTreeSet::new();
    if let Some(sheet) = &c.style {
        used.extend(brain_css::var_refs(sheet).into_iter().map(|(name, _)| name));
    }
    nodes(&c.template, &mut slots, &mut states, &mut used);
    let props:Vec<_>=c.props.iter().take(128).map(|p|json!({"name":p.name,"type":p.ty.as_str(),"optional":p.optional||p.default.is_some(),"bindable":p.bindable,"hasDefault":p.default.is_some(),"line":p.span.line,"column":p.span.col})).collect();
    let contract_truncated = c.props.len() > 128
        || c.script
            .iter()
            .filter(|s| matches!(s, Stmt::Import(_)))
            .count()
            > 128
        || slots.len() > 64
        || states.len() > 128
        || used.len() > 256
        || ["emits", "expose"]
            .iter()
            .any(|key| brain_check::meta_names(&c, key).len() > 64);
    let observed: BTreeSet<_> = states.iter().filter_map(|s| s["state"].as_str()).collect();
    let unknown: Vec<_> = [
        "permission",
        "loading",
        "error",
        "empty",
        "success",
        "disabled",
    ]
    .into_iter()
    .filter(|s| !observed.contains(s))
    .collect();
    Some(
        json!({"schemaVersion":1,"producer":PRODUCER,"truncated":contract_truncated,"file":file,"name":c.name,"kind":if c.kind==FileKind::App{"app"}else if c.level==Level::View{"view"}else{"component"},"revision":revision,
        "contract":{"props":props,"emits":names(&c,"emits"),"expose":names(&c,"expose"),"slots":slots.into_iter().take(64).collect::<Vec<_>>(),"imports":imports},"usedTokens":used.into_iter().take(256).collect::<Vec<_>>(),
        "states":{"hierarchy":["permission","loading","error","empty","success"],"disabledCrosscutting":true,"evidence":states.iter().take(128).collect::<Vec<_>>(),"unknown":unknown,"confidence":"static-structure; branches not executed"},
        "diagnostics":{"confidence":"syntax-parsed","semanticCheck":"not-run","items":[],"next":"brain check"},"confidence":"declarations-only; no runtime rendering or resource effects observed"}),
    )
}
fn imports(root: &Path, file: &str, source: &str) -> Vec<String> {
    let path = if file.ends_with(".js") || file.ends_with(".mjs") {
        Path::new("source.jsbrain")
    } else {
        Path::new(file)
    };
    brain_parse::parse(source, path)
        .ok()
        .map(|c| {
            c.script
                .into_iter()
                .filter_map(|s| {
                    if let Stmt::Import(i) = s {
                        normalized_import(root, file, &i.source)
                    } else {
                        None
                    }
                })
                .take(128)
                .collect()
        })
        .unwrap_or_default()
}
fn dependency_set(
    file: &str,
    graph: &BTreeMap<String, Vec<String>>,
    out: &mut BTreeSet<String>,
    depth: usize,
) {
    if depth > 32 || out.len() >= MAX_FILES || !out.insert(file.into()) {
        return;
    }
    if let Some(imports) = graph.get(file) {
        for next in imports {
            dependency_set(next, graph, out, depth + 1);
        }
    }
}
fn fingerprint(input: &Inputs) -> String {
    hash(serde_json::to_string(&json!({"revisions":input.revisions,"frameworkVersion":env!("CARGO_PKG_VERSION"),"generatorVersion":1,"complete":!input.truncated&&input.skipped==0})).unwrap().as_bytes())
}
fn refresh_with(root: &Path, input: Inputs) -> Result<Value, String> {
    let dir = private_dir(root, "manifests")?;
    let _lock = lock(&dir)?;
    let previous = read_json(&dir.join("index.json"), MAX_METADATA)
        .ok()
        .filter(|v| v["producer"] == PRODUCER && v["schemaVersion"] == 1);
    let mut writes: Vec<(PathBuf, Vec<u8>)> = vec![];
    let mut graph = BTreeMap::new();
    for (file, source) in &input.files {
        if file.ends_with(".brain")
            || file.ends_with(".jsbrain")
            || file.ends_with(".js")
            || file.ends_with(".mjs")
        {
            graph.insert(file.clone(), imports(root, file, source));
        }
    }
    let mut documents = vec![];
    let mut bytes = 0usize;
    let mut reused = 0usize;
    let mut truncated = input.truncated;
    let mut source_meta = vec![];
    for (file, source) in &input.files {
        let Some(mut manifest) = component(root, file, source, &input.revisions[file]) else {
            continue;
        };
        truncated |= manifest["truncated"].as_bool().unwrap_or(false);
        let mut deps = BTreeSet::new();
        dependency_set(file, &graph, &mut deps, 0);
        for name in [
            "tokens.cssbrain",
            "tokens.brain.css",
            "design.brain.json",
            "brain.json",
            "app.brain",
        ] {
            if input.revisions.contains_key(name) {
                deps.insert(name.into());
            }
        }
        if file.starts_with("views/") {
            for parent in Path::new(file)
                .parent()
                .into_iter()
                .flat_map(|p| p.ancestors())
            {
                for leaf in ["_layout.brain", "_guard.jsbrain"] {
                    let name = parent.join(leaf).to_string_lossy().replace('\\', "/");
                    if input.revisions.contains_key(&name) {
                        deps.insert(name);
                    }
                }
            }
        }
        let revisions: BTreeMap<_, _> = deps
            .iter()
            .map(|name| (name.clone(), input.revisions.get(name).cloned()))
            .collect();
        let dependency_hash=hash(serde_json::to_vec(&json!({"dependencies":revisions,"frameworkVersion":env!("CARGO_PKG_VERSION"),"generatorVersion":1})).unwrap().as_slice());
        manifest["dependencies"] = json!(revisions);
        manifest["dependencyFingerprint"] = json!(dependency_hash);
        manifest["frameworkVersion"] = json!(env!("CARGO_PKG_VERSION"));
        let id = hash(file.as_bytes());
        let cache = dir.join(format!("{id}.json"));
        let content =
            serde_json::to_vec(&manifest).map_err(|_| "cannot serialize component manifest")?;
        if bytes + content.len() > MAX_METADATA {
            truncated = true;
            break;
        }
        bytes += content.len();
        let revision = hash(&content);
        if read_json(&cache, MAX_METADATA).is_ok_and(|old| old == manifest) {
            reused += 1;
        } else {
            writes.push((cache, content));
        }
        source_meta.push(json!({"file":file,"name":manifest["name"],"kind":manifest["kind"],"revision":manifest["revision"],"manifestId":id,"metadataRevision":revision,"dependencyFingerprint":manifest["dependencyFingerprint"]}));
        documents.push(manifest);
    }
    let unknown_contracts = documents
        .iter()
        .filter(|d| d["confidence"] == "unknown")
        .count();
    let source_complete = !input.truncated && input.skipped == 0;
    let index = json!({"schemaVersion":1,"producer":PRODUCER,"frameworkVersion":env!("CARGO_PKG_VERSION"),"generatorVersion":1,"inputFingerprint":fingerprint(&input),"sourceRevisions":input.revisions,"documents":source_meta,"coverage":{"complete":!truncated&&input.skipped==0&&unknown_contracts==0,"sourceComplete":source_complete,"unknownContracts":unknown_contracts,"truncated":truncated,"skippedCount":input.skipped,"sourceFiles":input.files.len(),"sourceBytes":input.bytes,"metadataBytes":bytes},"limits":{"files":MAX_FILES,"sourceBytes":MAX_SOURCE,"readBytes":MAX_READ,"metadataBytes":MAX_METADATA},"confidence":"static source contracts; no app logic or resources executed"});
    let content =
        serde_json::to_vec(&index).map_err(|_| "cannot serialize project manifest index")?;
    if content.len() + bytes > MAX_METADATA {
        return Err("manifest metadata plus index exceeds 2 MiB; narrow the app or reduce component contracts".into());
    }
    let mut created: Vec<(PathBuf, String)> = vec![];
    let commit = (|| {
        for (path, data) in &writes {
            let fresh = !path.exists();
            atomic(path, data)?;
            if fresh {
                created.push((path.clone(), hash(data)));
            }
        }
        atomic(&dir.join("index.json"), &content)
    })();
    if let Err(error) = commit {
        for (path, revision) in created {
            if read_json(&path, MAX_METADATA)
                .and_then(|value| {
                    serde_json::to_vec(&value).map_err(|_| "invalid owned metadata".to_string())
                })
                .is_ok_and(|bytes| hash(&bytes) == revision)
            {
                let _ = fs::remove_file(path);
            }
        }
        return Err(error);
    }
    // Remove only unchanged, previously indexed documents owned by this generator after publishing the new TOC.
    let retained: BTreeSet<_> = index["documents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["manifestId"].as_str())
        .collect();
    if let Some(previous) = previous {
        for old in previous["documents"]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_FILES)
        {
            if let Some(id) = old["manifestId"].as_str().filter(|id| {
                id.len() == 64
                    && id.bytes().all(|b| b.is_ascii_hexdigit())
                    && !retained.contains(id)
            }) {
                let path = dir.join(format!("{id}.json"));
                if let Ok(value) = read_json(&path, MAX_METADATA) {
                    if value["producer"] == PRODUCER
                        && serde_json::to_vec(&value).is_ok_and(|bytes| {
                            old["metadataRevision"].as_str() == Some(hash(&bytes).as_str())
                        })
                    {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
    }
    Ok(
        json!({"schemaVersion":1,"private":true,"path":".brain/manifests/index.json","documents":documents.len(),"reused":reused,"coverage":index["coverage"],"index":index}),
    )
}
fn snapshot(root: &Path) -> Result<Value, String> {
    let input = inputs(root)?;
    let dir = private_dir(root, "manifests")?;
    if let Ok(index) = read_json(&dir.join("index.json"), MAX_METADATA) {
        if index["producer"] == PRODUCER
            && index["schemaVersion"] == 1
            && index["coverage"]["sourceComplete"] == true
            && index["inputFingerprint"].as_str() == Some(fingerprint(&input).as_str())
        {
            return Ok(index);
        }
    }
    Ok(refresh_with(root, input)?["index"].clone())
}
pub fn refresh(root: &Path) -> Result<Value, String> {
    refresh_with(root, inputs(root)?)
}
pub fn query(root: &Path, target: Option<&str>) -> Result<Value, String> {
    if let Some(target) = target {
        if target.contains('/') || target.starts_with('.') || target.ends_with(".brain") {
            crate::changes::source_path(root, target)?;
        }
    }
    let index = snapshot(root)?;
    let Some(target) = target else {
        return Ok(index);
    };
    let matches: Vec<_> = index["documents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["file"].as_str() == Some(target) || d["name"].as_str() == Some(target))
        .collect();
    if matches.len() != 1 {
        return Err(
            "manifest target is missing or ambiguous; use its exact app-relative file".into(),
        );
    }
    let row = matches[0];
    let id = row["manifestId"]
        .as_str()
        .ok_or("manifest index is invalid")?;
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("manifest identifier is invalid".into());
    }
    let dir = private_dir(root, "manifests")?;
    let value = read_json(&dir.join(format!("{id}.json")), MAX_METADATA)?;
    let bytes = serde_json::to_vec(&value).map_err(|_| "manifest JSON is invalid")?;
    if value["producer"] != PRODUCER
        || value["schemaVersion"] != 1
        || row["metadataRevision"].as_str() != Some(hash(&bytes).as_str())
    {
        return Err(
            "component manifest changed outside the generator; run brain manifest refresh".into(),
        );
    }
    Ok(value)
}
pub(crate) fn documents(root: &Path) -> Result<(Value, Vec<Value>), String> {
    let index = snapshot(root)?;
    let dir = private_dir(root, "manifests")?;
    let mut out = vec![];
    let mut bytes = 0usize;
    for row in index["documents"]
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX_FILES)
    {
        let Some(id) = row["manifestId"]
            .as_str()
            .filter(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            continue;
        };
        let value = read_json(&dir.join(format!("{id}.json")), MAX_METADATA - bytes)?;
        let data = serde_json::to_vec(&value).map_err(|_| "manifest JSON is invalid")?;
        bytes += data.len();
        if bytes > MAX_METADATA || row["metadataRevision"].as_str() != Some(hash(&data).as_str()) {
            return Err("cached manifest integrity failed; refresh the project manifests".into());
        }
        out.push(value);
    }
    Ok((index, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct App(PathBuf);
    impl App {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "brain-manifests-{}-{}",
                std::process::id(),
                TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("comp")).unwrap();
            fs::write(root.join("tokens.cssbrain"), ":root { --accent: #0062c4; }").unwrap();
            fs::write(root.join("comp/card.brain"),"<meta props=\"title:str\" emits=\"choose\"><script>import { data } from '../data.jsbrain'</script><button disabled>{title}</button><slot /><style>button{color:var(--accent)}</style>").unwrap();
            fs::write(root.join("data.jsbrain"), "export const data = 1").unwrap();
            Self(root)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn private_contracts_have_no_bodies_or_default_values() {
        let app = App::new();
        let report = refresh(&app.0).unwrap();
        assert_eq!(report["documents"], 1);
        let card = query(&app.0, Some("comp/card.brain")).unwrap();
        assert_eq!(card["contract"]["props"][0]["name"], "title");
        assert_eq!(card["usedTokens"][0], "--accent");
        assert!(!card.to_string().contains("export const data"));
        assert!(!app.0.join("dist").exists());
        assert!(app.0.join(".brain/manifests/index.json").is_file());
    }
    #[test]
    fn dependency_and_token_changes_invalidate_only_current_metadata() {
        let app = App::new();
        let before = query(&app.0, Some("Card")).unwrap();
        fs::write(app.0.join("data.jsbrain"), "export const data = 2").unwrap();
        let after = query(&app.0, Some("Card")).unwrap();
        assert_eq!(before["revision"], after["revision"]);
        assert_ne!(
            before["dependencyFingerprint"],
            after["dependencyFingerprint"]
        );
        fs::write(
            app.0.join("tokens.cssbrain"),
            ":root { --accent: #aa0000; }",
        )
        .unwrap();
        let next = query(&app.0, Some("Card")).unwrap();
        assert_ne!(
            after["dependencyFingerprint"],
            next["dependencyFingerprint"]
        );
    }
    #[test]
    fn private_cache_symlinks_and_public_source_traversal_are_refused() {
        let app = App::new();
        assert!(query(&app.0, Some("../private")).is_err());
        #[cfg(unix)]
        {
            fs::create_dir_all(app.0.join(".brain")).unwrap();
            let outside = app.0.join("outside");
            fs::create_dir(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, app.0.join(".brain/manifests")).unwrap();
            assert!(refresh(&app.0).is_err());
            assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
        }
    }
    #[test]
    fn deleted_documents_are_pruned_without_touching_other_private_files_or_values() {
        let app = App::new();
        fs::write(
            app.0.join(".brain_env"),
            "DATABASE_URL=private-environment-value",
        )
        .unwrap();
        let first = refresh(&app.0).unwrap();
        let id = first["index"]["documents"][0]["manifestId"]
            .as_str()
            .unwrap();
        let stale = app.0.join(format!(".brain/manifests/{id}.json"));
        let note = app.0.join(".brain/manifests/notes.json");
        fs::write(&note, "{\"userNote\":true}").unwrap();
        assert!(!first.to_string().contains("private-environment-value"));
        assert!(
            first["index"]["sourceRevisions"]
                .get(".brain_env")
                .is_none()
        );
        fs::remove_file(app.0.join("comp/card.brain")).unwrap();
        refresh(&app.0).unwrap();
        assert!(!stale.exists());
        assert!(note.exists());
    }
}
