//! Checked AST edits. `rename` renames a component file, never arbitrary JavaScript bindings.
use brain_mcp::Json;
use brain_parse::ast::{Component, Stmt};
use std::collections::BTreeMap;
use std::path::{Component as Part, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub enum Operation {
    AddProp { declaration: String },
    RemoveProp { name: String },
    Rename { name: String },
    Move { to: String },
    Extract { selector: String, name: String },
}
static NEXT: AtomicU64 = AtomicU64::new(0);
type Snapshot = BTreeMap<String, Vec<u8>>;
struct Workspace { path: PathBuf, _lock: std::fs::File }
impl Drop for Workspace {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.path); }
}
fn safe(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let path = Path::new(rel);
    if rel.is_empty() || rel.contains('\\') || path.components().any(|p| !matches!(p, Part::Normal(_))) {
        return Err("edit paths must be relative, without traversal or symlinks".into());
    }
    let mut out = root.to_path_buf();
    for part in path.components() {
        out.push(part);
        if std::fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink()) { return Err(format!("edit refuses symlink: {rel}")); }
    }
    Ok(out)
}
fn collect(root: &Path) -> Result<Snapshot, String> {
    fn walk(root: &Path, dir: &Path, depth: usize, total: &mut usize, files: &mut Snapshot) -> Result<(), String> {
        if depth > 32 { return Err("edit snapshot exceeds 32 directories of depth".into()); }
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if ["node_modules", "dist", "target"].contains(&name.as_str()) || (name.starts_with('.') && name != ".brain_env") { continue; }
            let meta = std::fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() { return Err("edit snapshot refuses symlinks; use regular project source files".into()); }
            if meta.is_dir() { walk(root, &entry.path(), depth + 1, total, files)?; }
            else if meta.is_file() {
                if meta.len() > 2 * 1024 * 1024 || files.len() >= 2048 { return Err("edit snapshot exceeds 2 MiB per file or 2,048 files".into()); }
                let bytes = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
                *total += bytes.len();
                if *total > 32 * 1024 * 1024 { return Err("edit snapshot exceeds 32 MiB; isolate the app before editing".into()); }
                let rel = entry.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                files.insert(rel, bytes);
            } else { return Err("edit snapshot requires regular files".into()); }
        }
        Ok(())
    }
    let mut files = BTreeMap::new(); walk(root, root, 0, &mut 0, &mut files)?; Ok(files)
}
fn parse(rel: &str, source: &str) -> Result<Component, String> {
    brain_parse::parse(source, Path::new(rel)).map_err(|_| format!("{rel} does not parse; fix: run brain check before editing"))
}
fn checked(root: &Path) -> Result<(), String> {
    let (_, diags) = crate::run_check(Some(root.to_path_buf()))?;
    let errors: Vec<_> = diags.iter().filter(|d| !d.is_warning()).collect();
    if errors.is_empty() { Ok(()) } else {
        Err(format!("edit refused: the project does not pass brain check ({} errors; first {} in {}); original files unchanged", errors.len(), errors[0].code, errors[0].file))
    }
}
fn component_name(name: &str) -> Result<String, String> {
    if !name.starts_with(|c: char| c.is_ascii_uppercase()) || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("component name must be PascalCase ASCII letters and digits".into());
    }
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() { if i > 0 && c.is_ascii_uppercase() { out.push('-'); } out.push(c.to_ascii_lowercase()); }
    Ok(out)
}
fn rel_import(from: &str, to: &Path) -> String {
    let parent: Vec<_> = Path::new(from).parent().unwrap().components().collect();
    let target: Vec<_> = to.components().collect();
    let common = parent.iter().zip(&target).take_while(|(a,b)| a == b).count();
    let tail = target[common..].iter().map(|p| p.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    if common == parent.len() { format!("./{tail}") } else { format!("{}{tail}", "../".repeat(parent.len() - common)) }
}
fn resolved_import(from: &str, source: &str) -> Result<PathBuf, String> {
    let mut parts = Path::new(from).parent().unwrap().to_path_buf();
    for part in Path::new(source).components() {
        match part {
            Part::Normal(p) => parts.push(p), Part::CurDir => {},
            Part::ParentDir => if !parts.pop() { return Err("edit refuses imports outside the app".into()); },
            _ => return Err("edit move only supports relative imports".into()),
        }
    }
    Ok(parts)
}
fn replace_import(c: &Component, import: &brain_parse::ast::Import, value: &str) -> Result<(usize, usize, String), String> {
    let idx = brain_lex::LineIndex::new(&c.src);
    let mut lexer = brain_lex::Lexer::new(&c.src, &c.file, &idx, import.span.start as usize, import.span.end as usize);
    loop {
        let token = lexer.next_token().map_err(|_| "could not locate the parsed import source")?;
        match token.tok {
            brain_lex::Tok::Str { value: s, .. } if s == import.source => return Ok((token.span.start as usize, token.span.end as usize, Json::str(value).stringify())),
            brain_lex::Tok::Eof => return Err("parsed import source has no string token".into()),
            _ => {}
        }
    }
}
fn rewrite_imports(rel: &str, source: &str, from: &str, to: &str) -> Result<String, String> {
    let c = parse(rel, source)?;
    if rel == from && c.style.is_some() && (source.contains("url(") || source.contains("@import")) {
        return Err("moving components with CSS URL/import references requires explicit migration".into());
    }
    // Dynamic imports cannot be safely redirected by changing static import declarations.
    let idx = brain_lex::LineIndex::new(source);
    for stmt in &c.script {
        if matches!(stmt, Stmt::Import(_)) { continue; }
        let span = stmt.span();
        let mut lexer = brain_lex::Lexer::new(source, rel, &idx, span.start as usize, span.end as usize);
        loop {
            match lexer.next_token().map_err(|_| "cannot inspect script references safely")?.tok {
                brain_lex::Tok::Ident(word) if word == "import" => return Err("move/rename refuses dynamic imports; migrate those references explicitly".into()),
                brain_lex::Tok::Str { value, .. } | brain_lex::Tok::TplFull(value) if value.starts_with('.') => {
                    if rel == from || resolved_import(rel, &value).is_ok_and(|p| p == Path::new(from)) {
                        return Err("move/rename refuses relative source strings outside static imports; review those references explicitly".into());
                    }
                }
                brain_lex::Tok::Eof => break,
                _ => {}
            }
        }
    }
    let new_rel = if rel == from { to } else { rel };
    let mut edits = Vec::new();
    for statement in &c.script {
        if let Stmt::Import(import) = statement {
            if !import.source.starts_with('.') { if rel == from { return Err("moving components with non-relative imports is unsupported".into()); } continue; }
            let target = resolved_import(rel, &import.source)?;
            if rel != from && target != Path::new(from) { continue; }
            let target = if target == Path::new(from) { PathBuf::from(to) } else { target };
            let replacement = rel_import(new_rel, &target);
            if replacement != import.source { edits.push(replace_import(&c, import, &replacement)?); }
        }
    }
    let mut out = source.to_string();
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (start, end, replacement) in edits { out.replace_range(start..end, &replacement); }
    Ok(out)
}

/// Shared by AST and MCP text/file edits. The kernel releases this lock on process death.
/// Cooperating writers serialize; external editors still require conflict detection.
pub fn writer_lock(root:&Path)->Result<std::fs::File,String>{
    let file=writer_lock_unchecked(root)?;
    crate::changes::guard_legacy_writer(root)?;
    Ok(file)
}

/// Internal kernel lock for the leased changes protocol. Callers must authorize
/// every target lease under changes/state.lock before committing or recovering.
/// Other source writers must use writer_lock, which also respects active leases.
pub(crate) fn writer_lock_unchecked(root:&Path)->Result<std::fs::File,String>{
    let state=safe(root,".brain")?;
    std::fs::create_dir_all(&state).map_err(|e|e.to_string())?;
    let lock=safe(root,".brain/edit.lock")?;
    let file=std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(lock).map_err(|e|e.to_string())?;
    file.try_lock().map_err(|_|"another edit owns .brain/edit.lock; finish it before retrying")?;
    Ok(file)
}

/// Plan and validate inside a private bounded copy; only then commit checked source changes.
/// Callback receives exact previous contents after the transaction succeeds (for MCP undo).
pub fn apply(root: &Path, path: &str, op: Operation, before_write: &mut dyn FnMut(&Path, Option<&str>)) -> Result<Json, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let original = safe(&root, path)?;
    if !path.ends_with(".brain") || !original.is_file() { return Err("edit requires an existing .brain component path".into()); }
    let lock_file=writer_lock(&root)?;
    let state=root.join(".brain");
    let scratch = state.join(format!("edit-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let work = Workspace { path: scratch.clone(), _lock: lock_file };
    std::fs::create_dir(&scratch).map_err(|e| e.to_string())?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&scratch, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?; }
    let before = collect(&root)?;
    for (rel, bytes) in &before {
        let file = scratch.join(rel); std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(file, bytes).map_err(|e| e.to_string())?;
    }
    checked(&scratch)?;
    let source = std::fs::read_to_string(scratch.join(path)).map_err(|e| e.to_string())?;
    let c = parse(path, &source)?;
    let op = match op {
        Operation::Rename { name } => Operation::Move { to: Path::new(path).parent().unwrap().join(format!("{}.brain", component_name(&name)?)).to_string_lossy().replace('\\', "/") },
        op => op,
    };
    match op {
        Operation::AddProp { declaration } => {
            if declaration.contains(['"', '\n', '\r', '<', '>']) { return Err("prop declaration must be one line; use single quotes for string defaults".into()); }
            let probe = parse("comp/prop.brain", &format!("<meta props=\"{declaration}\">\n<span />"))?;
            if probe.props.len() != 1 { return Err("add-prop expects exactly one typed prop declaration".into()); }
            if c.props.iter().any(|p| p.name == probe.props[0].name) { return Err("the prop already exists".into()); }
            let mut out = source;
            if let Some(last) = c.props.last() { out.insert_str(last.span.end as usize, &format!(" {declaration}")); }
            else { out.insert_str(0, &format!("<meta props=\"{declaration}\">\n")); }
            std::fs::write(scratch.join(path), out).map_err(|e| e.to_string())?;
        }
        Operation::RemoveProp { name } => {
            let prop = c.props.iter().find(|p| p.name == name).ok_or("prop does not exist")?;
            let mut start = prop.span.start as usize;
            // Consume only the separator before this declaration, preserving neighboring props.
            while start > 0 && source.as_bytes()[start - 1].is_ascii_whitespace() { start -= 1; }
            let mut out = source; out.replace_range(start..prop.span.end as usize, "");
            std::fs::write(scratch.join(path), out).map_err(|e| e.to_string())?;
        }
        Operation::Move { to } => {
            if !to.ends_with(".brain") { return Err("move destination must end in .brain".into()); }
            safe(&root, &to)?;
            if !["comp/", "ui/"].iter().any(|prefix| path.starts_with(prefix)) || !["comp/", "ui/"].iter().any(|prefix| to.starts_with(prefix)) {
                return Err("move/rename supports comp/ and ui/ components; route and module moves require explicit migration".into());
            }
            if root.join(&to).exists() { return Err("destination exists; edit never overwrites another file".into()); }
            for (rel, bytes) in &before {
                if rel.ends_with(".js") && String::from_utf8_lossy(bytes).contains(Path::new(path).file_name().unwrap().to_str().unwrap()) {
                    return Err("move/rename found a raw JavaScript reference to the component; migrate it explicitly".into());
                }
                if !rel.ends_with(".brain") && !rel.ends_with(".jsbrain") { continue; }
                let text = std::str::from_utf8(bytes).map_err(|_| "source is not UTF-8")?;
                let out = rewrite_imports(rel, text, path, &to)?;
                let dest = scratch.join(if rel == path { &to } else { rel });
                std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
                std::fs::write(dest, out).map_err(|e| e.to_string())?;
            }
            std::fs::remove_file(scratch.join(path)).map_err(|e| e.to_string())?;
        }
        Operation::Rename { .. } => unreachable!(),
        Operation::Extract { selector, name } => {
            component_name(&name)?;
            crate::card::split(&scratch, path, &selector, &name, &mut |_, _| {})?;
        }
    }
    checked(&scratch)?;
    let after = collect(&scratch)?;
    if collect(&root)? != before { return Err("project changed during edit validation; retry from the current source".into()); }
    let mut changed: Vec<String> = before.keys().chain(after.keys()).filter(|k| before.get(*k) != after.get(*k)).cloned().collect();
    changed.sort(); changed.dedup();
    for rel in &changed { safe(&root, rel)?; }
    commit(&root,&before,&after,&changed,&mut |_,_|{})?;
    for rel in &changed { before_write(&root.join(rel), before.get(rel).and_then(|s| std::str::from_utf8(s).ok())); }
    drop(work);
    Ok(Json::obj([("changed", Json::Arr(changed.iter().map(Json::str).collect())), ("checked", Json::Bool(true))]))
}

fn matches_source(path:&Path,expected:Option<&Vec<u8>>)->Result<bool,String>{
    use std::io::Read;
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind()==std::io::ErrorKind::NotFound => return Ok(expected.is_none()),
        Err(e)=>return Err(e.to_string()),
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() || meta.len()>2*1024*1024=>return Ok(false),
        _=>{}
    }
    let Some(expected)=expected else{return Ok(false)};
    let mut actual=Vec::new();
    std::fs::File::open(path).map_err(|e|e.to_string())?.take(2*1024*1024+1).read_to_end(&mut actual).map_err(|e|e.to_string())?;
    Ok(&actual==expected)
}

fn commit(root:&Path,before:&Snapshot,after:&Snapshot,changed:&[String],hook:&mut dyn FnMut(&str,&Path))->Result<(),String>{
    let mut written = Vec::new();
    let result = (|| -> Result<(), String> {
        for rel in changed {
            let dest = safe(root,rel)?;
            hook("before-write",&dest);
            if !matches_source(&dest,before.get(rel))? { return Err(format!("concurrent edit conflict: {rel}")); }
            match after.get(rel) {
                Some(bytes) => {
                    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
                    let temp = dest.with_extension(format!("brain-edit-{}.tmp", std::process::id()));
                    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| e.to_string())?;
                    use std::io::Write;
                    let res = file.write_all(bytes).and_then(|_| file.sync_all()).and_then(|_| {
                        if before.contains_key(rel) {
                            std::fs::set_permissions(&temp, std::fs::metadata(&dest)?.permissions())?;
                            if !matches_source(&dest,before.get(rel)).map_err(std::io::Error::other)? { return Err(std::io::Error::other(format!("concurrent edit conflict: {rel}"))); }
                            std::fs::rename(&temp, &dest)
                        } else {
                            // Link fails atomically if another writer created the destination after preflight.
                            std::fs::hard_link(&temp, &dest)?;
                            let _ = std::fs::remove_file(&temp);
                            Ok(())
                        }
                    });
                    if res.is_err() { let _ = std::fs::remove_file(&temp); }
                    res.map_err(|e| e.to_string())?;
                }
                None => {
                    if !matches_source(&dest,before.get(rel))? { return Err(format!("concurrent edit conflict: {rel}")); }
                    std::fs::remove_file(dest).map_err(|e| e.to_string())?;
                },
            }
            written.push(rel.clone());
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for rel in written.iter().rev() {
            let dest=root.join(rel);
            hook("before-rollback",&dest);
            if safe(root,rel).is_err() || !matches_source(&dest,after.get(rel)).unwrap_or(false) {
                failures.push(format!("{rel} (concurrent edit preserved)")); continue;
            }
            let restored = match before.get(rel) {
                Some(bytes) if !after.contains_key(rel) => {
                    use std::io::Write;
                    std::fs::OpenOptions::new().write(true).create_new(true).open(&dest).and_then(|mut f|f.write_all(bytes))
                }
                Some(bytes) => std::fs::write(&dest,bytes),
                None => std::fs::remove_file(&dest).or_else(|e|if e.kind()==std::io::ErrorKind::NotFound{Ok(())}else{Err(e)}),
            };
            if restored.is_err() { failures.push(rel.clone()); }
        }
        return Err(format!("edit write failed: {error}; rollback failures: {}", failures.join(", ")));
    }
    Ok(())
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    #[test]
    fn concurrent_changes_before_commit_and_rollback_are_preserved() {
        let root=std::env::temp_dir().join(format!("brain-edit-conflict-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir_all(&root).unwrap();
        struct Cleanup(PathBuf);impl Drop for Cleanup {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
        let _cleanup=Cleanup(root.clone());
        let before=Snapshot::from([("a.brain".into(),b"original a".to_vec()),("b.brain".into(),b"original b".to_vec())]);
        let after=Snapshot::from([("a.brain".into(),b"planned a".to_vec()),("b.brain".into(),b"planned b".to_vec())]);
        for(name,bytes)in &before{std::fs::write(root.join(name),bytes).unwrap();}
        let error=commit(&root,&before,&after,&["a.brain".into(),"b.brain".into()],&mut |phase,path|{
            if phase=="before-write" && path.ends_with("b.brain"){std::fs::write(path,b"other editor b").unwrap();}
            if phase=="before-rollback" && path.ends_with("a.brain"){std::fs::write(path,b"other editor a").unwrap();}
        }).unwrap_err();
        assert!(error.contains("concurrent edit conflict")&&error.contains("concurrent edit preserved"),"{error}");
        assert_eq!(std::fs::read(root.join("a.brain")).unwrap(),b"other editor a");
        assert_eq!(std::fs::read(root.join("b.brain")).unwrap(),b"other editor b");
    }
}
