//! Bounded exact component discovery. Resolution is a normalization edit, never a filesystem write.
use crate::scope::{self, Kind, Scopes};
use brain_parse::ast::*;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Component as Part, Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

pub const CODE: &str = "B0222";
#[derive(Clone, Debug)]
pub enum Resolution {
    Unique { source: String },
    Ambiguous { paths: Vec<String> },
    Incomplete,
}
#[derive(Clone, Debug, Default)]
pub struct AutoComponents {
    pub resolutions: BTreeMap<String, Resolution>,
    pub insertion: Option<(u32, u32, String)>,
    pub names: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    changed: Option<(i64, i64)>,
}
#[derive(Clone)]
struct Index {
    root: PathBuf,
    dirs: Vec<Stamp>,
    names: BTreeMap<String, Vec<PathBuf>>,
    complete: bool,
}
static CACHE: OnceLock<Mutex<VecDeque<Index>>> = OnceLock::new();
const MAX_DIRS: usize = 256;
const MAX_FILES: usize = 4096;
const MAX_ENTRIES: usize = 8192;
const MAX_DEPTH: usize = 12;
fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return None;
    }
    Some(Stamp {
        path: path.into(),
        len: meta.len(),
        modified: meta.modified().ok(),
        created: meta.created().ok(),
        changed: {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                Some((meta.ctime(), meta.ctime_nsec()))
            }
            #[cfg(not(unix))]
            {
                None
            }
        },
    })
}
fn excluded(name: &str, depth: usize) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules"
                | "target"
                | "dist"
                | "secrets"
                | "credentials"
                | "coverage"
                | "tests"
                | "test"
        )
        || (depth == 0 && matches!(name, "public" | "server" | "content" | "docs" | "bench"))
}
fn scan(root: &Path) -> Index {
    let mut index = Index {
        root: root.into(),
        dirs: Vec::new(),
        names: BTreeMap::new(),
        complete: true,
    };
    let mut pending = vec![(root.to_path_buf(), 0)];
    let (mut entries, mut files) = (0, 0);
    while let Some((dir, depth)) = pending.pop() {
        if depth > MAX_DEPTH || index.dirs.len() >= MAX_DIRS {
            index.complete = false;
            break;
        }
        let Some(before) = stamp(&dir) else {
            index.complete = false;
            break;
        };
        index.dirs.push(before.clone());
        if depth > 0
            && (dir.join("app.brain").is_file()
                || dir.join("brain.json").is_file()
                || crate::TOKEN_FILES
                    .iter()
                    .any(|name| dir.join(name).is_file()))
        {
            continue;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            index.complete = false;
            break;
        };
        let mut children = Vec::new();
        for child in read {
            entries += 1;
            if entries > MAX_ENTRIES {
                index.complete = false;
                break;
            }
            let Ok(child) = child else {
                index.complete = false;
                break;
            };
            let Some(name) = child.file_name().to_str().map(str::to_string) else {
                index.complete = false;
                break;
            };
            if excluded(&name, depth) {
                continue;
            }
            let Ok(kind) = child.file_type() else {
                index.complete = false;
                break;
            };
            if kind.is_symlink() {
                index.complete = false;
                break;
            }
            if kind.is_dir() {
                children.push((child.path(), depth + 1));
            } else if kind.is_file()
                && name.ends_with(".brain")
                && !name.ends_with(".test.brain")
                && !name.starts_with('_')
                && name != "app.brain"
            {
                files += 1;
                if files > MAX_FILES {
                    index.complete = false;
                    break;
                }
                let path = child.path();
                if std::fs::canonicalize(&path).ok().as_ref() != Some(&path) {
                    index.complete = false;
                    break;
                }
                index
                    .names
                    .entry(brain_parse::component_name(&path))
                    .or_default()
                    .push(path);
            }
        }
        if !index.complete {
            break;
        }
        if stamp(&dir).as_ref() != Some(&before) {
            index.complete = false;
            break;
        }
        children.sort_by(|a, b| b.0.cmp(&a.0));
        pending.extend(children);
    }
    for paths in index.names.values_mut() {
        paths.sort();
    }
    index
}
fn index(root: &Path) -> Index {
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Ok(mut cache) = cache.lock() {
        if let Some(at) = cache.iter().position(|entry| {
            entry.root == root
                && entry.complete
                && entry
                    .dirs
                    .iter()
                    .all(|old| stamp(&old.path).as_ref() == Some(old))
        }) {
            let entry = cache.remove(at).unwrap();
            cache.push_front(entry.clone());
            return entry;
        }
    }
    let entry = scan(root);
    if entry.complete {
        if let Ok(mut cache) = cache.lock() {
            cache.retain(|old| old.root != root);
            cache.push_front(entry.clone());
            cache.truncate(8);
        }
    }
    entry
}
fn requested(nodes: &[Node], scopes: &mut Scopes, out: &mut BTreeSet<String>) {
    for node in nodes {
        match node {
            Node::ComponentRef(reference) => {
                if scopes.lookup(&reference.name).is_none()
                    && !scope::UI_KIT.contains(&reference.name.as_str())
                    && !matches!(reference.name.as_str(), "Img" | "Portal")
                {
                    out.insert(reference.name.clone());
                }
                requested(&reference.children, scopes, out);
            }
            Node::Element(element) => requested(&element.children, scopes, out),
            Node::If(block) => {
                for branch in &block.branches {
                    requested(&branch.body, scopes, out);
                }
                if let Some(body) = &block.otherwise {
                    requested(body, scopes, out);
                }
            }
            Node::Each(block) => {
                scopes.push();
                scopes.declare_pat(&block.item, Kind::Const);
                if let Some(name) = &block.index {
                    scopes.declare(&name.name, Kind::Const);
                }
                requested(&block.body, scopes, out);
                scopes.pop();
                if let Some(body) = &block.empty {
                    requested(body, scopes, out);
                }
            }
            Node::Await(block) => {
                requested(&block.pending, scopes, out);
                for branch in [&block.then, &block.catch].into_iter().flatten() {
                    scopes.push();
                    if let Some(binding) = &branch.binding {
                        scopes.declare_pat(binding, Kind::Const);
                    }
                    requested(&branch.body, scopes, out);
                    scopes.pop();
                }
            }
            Node::SlotScope(block) => {
                scopes.push();
                for name in &block.names {
                    scopes.declare(&name.name, Kind::Const);
                }
                requested(&block.body, scopes, out);
                scopes.pop();
            }
            _ => {}
        }
    }
}
fn relative(from: &Path, to: &Path) -> Option<String> {
    let left: Vec<_> = from.components().collect();
    let right: Vec<_> = to.components().collect();
    let same = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
    if same == 0 {
        return None;
    }
    let mut parts = vec!["..".to_string(); left.len() - same];
    for part in &right[same..] {
        let Part::Normal(value) = part else {
            return None;
        };
        parts.push(value.to_str()?.into());
    }
    let source = parts.join("/");
    Some(if source.starts_with("../") {
        source
    } else {
        format!("./{source}")
    })
}
/// Always retain scanned directories as dependencies, including after normalization adds explicit imports.
pub fn load(
    disk: &Path,
    component: &Component,
    root: Option<&Path>,
) -> (AutoComponents, Vec<PathBuf>) {
    let mut result = AutoComponents::default();
    if component.kind == FileKind::Module {
        return (result, vec![]);
    }
    let Some(root) = root else {
        return (result, vec![]);
    };
    let Ok(canonical) = std::fs::canonicalize(root) else {
        return (result, vec![root.into()]);
    };
    if std::fs::symlink_metadata(root).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return (result, vec![root.into()]);
    }
    let cache = index(&canonical);
    let mut deps: Vec<_> = cache.dirs.iter().map(|dir| dir.path.clone()).collect();
    deps.push(canonical.clone());
    deps.sort();
    deps.dedup();
    let mut names = BTreeSet::new();
    let mut scopes = Scopes::new(scope::top_bindings(component));
    requested(&component.template, &mut scopes, &mut names);
    requested(&component.head, &mut scopes, &mut names);
    if names.len() > 256 {
        return (result, deps);
    }
    let own = std::fs::canonicalize(disk).ok();
    let Some(parent) = disk
        .parent()
        .and_then(|path| std::fs::canonicalize(path).ok())
    else {
        return (result, deps);
    };
    for name in names {
        if name == component.name {
            continue;
        }
        if !cache.complete {
            result.resolutions.insert(name, Resolution::Incomplete);
            continue;
        }
        let paths: Vec<_> = cache
            .names
            .get(&name)
            .into_iter()
            .flatten()
            .filter(|path| own.as_ref() != Some(*path))
            .collect();
        match paths.as_slice() {
            [path] => {
                if let Some(source) = relative(&parent, path) {
                    result
                        .resolutions
                        .insert(name, Resolution::Unique { source });
                }
            }
            [] => {}
            _ => {
                result.resolutions.insert(
                    name,
                    Resolution::Ambiguous {
                        paths: paths
                            .iter()
                            .take(12)
                            .filter_map(|path| {
                                path.strip_prefix(&canonical)
                                    .ok()
                                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                            })
                            .collect(),
                    },
                );
            }
        }
    }
    let mut imports = String::new();
    for (name, resolution) in &result.resolutions {
        if let Resolution::Unique { source } = resolution {
            let mut literal = String::new();
            brain_parse::push_json_str(&mut literal, source);
            imports.push_str(&format!("import {name} from {literal};"));
            result.names.push(name.clone());
        }
    }
    if !imports.is_empty() {
        if let Some((start, end, script)) = insertion(component, &imports) {
            result.insertion = Some((start as u32, end as u32, script));
        } else {
            result
                .resolutions
                .retain(|_, resolution| !matches!(resolution, Resolution::Unique { .. }));
            result.names.clear();
        }
    }
    (result, deps)
}
// Find real script tags, skipping comments, quoted attributes and external scripts. Never search script bodies.
fn insertion(component: &Component, imports: &str) -> Option<(usize, usize, String)> {
    let source = component.src.as_str();
    if let Some(statement) = component.script.first() {
        let at = statement.span().start as usize;
        return Some((at, at, imports.to_string()));
    }
    let protected: Vec<_> = component
        .template
        .iter()
        .chain(&component.head)
        .map(Node::span)
        .collect();
    let mut at = 0;
    while let Some(relative) = source[at..].find('<') {
        let start = at + relative;
        if let Some(span) = protected
            .iter()
            .find(|span| span.start as usize <= start && start < span.end as usize)
        {
            at = span.end as usize;
            continue;
        }
        if source[start..].starts_with("<!--") {
            at = start + source[start..].find("-->")? + 3;
            continue;
        }
        let (end, src) = tag(source, start)?;
        let tag = &source[start + 1..end - 1];
        let name = tag
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '/' | '>'))
            .next()
            .unwrap_or("");
        if name == "script" {
            if !src {
                if source[start..end].trim_end().ends_with("/>") {
                    return Some((start, end, format!("<script>{imports}</script>")));
                }
                return Some((end, end, imports.to_string()));
            }
            at = end + source[end..].find("</script")?;
            continue;
        }
        if name == "style" {
            at = end + source[end..].find("</style")?;
            continue;
        }
        at = end;
    }
    let mut at = 0;
    loop {
        at += source[at..].len() - source[at..].trim_start().len();
        if source[at..].starts_with("<!--") {
            at += source[at..].find("-->")? + 3;
            continue;
        }
        if source[at..].starts_with("<meta")
            && source
                .as_bytes()
                .get(at + 5)
                .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>')
        {
            at = tag(source, at)?.0;
            continue;
        }
        break;
    }
    Some((at, at, format!("<script>{imports}</script>")))
}
fn tag(source: &str, start: usize) -> Option<(usize, bool)> {
    let bytes = source.as_bytes();
    let mut at = start + 1;
    let mut src = false;
    while at < bytes.len() {
        if bytes[at] == b'>' {
            return Some((at + 1, src));
        }
        if matches!(bytes[at], b'\'' | b'"') {
            let quote = bytes[at];
            at += 1;
            while at < bytes.len() && bytes[at] != quote {
                at += 1;
            }
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"src")
            && at > start
            && bytes[at - 1].is_ascii_whitespace()
            && bytes
                .get(at + 3)
                .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(*byte, b'=' | b'>'))
        {
            src = true;
        }
        at += 1;
    }
    None
}
