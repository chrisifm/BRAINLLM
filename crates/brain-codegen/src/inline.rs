//! INTERNALS §13: stateless `ui/` components are inlined into the parent — no function call, no props object,
//! no instance. Their markup is generated in the parent with each prop read replaced by the parent's expression,
//! and their root elements keep the child's `data-b`, so the child's scoped CSS (kept through a side-effect import)
//! applies unchanged. Anything that could change behaviour keeps the regular call.

use brain_parse::ast::*;
use std::collections::HashMap;
use std::path::{Component as PathPart, Path, PathBuf};

pub(crate) struct Inline {
    pub comp: Component,
    /// Scope hash of the child (same input as when the child itself is compiled).
    pub hash: String,
    pub source: String,
    pub has_style: bool,
}

/// One inlined use being generated: prop name → (JS of the parent's value, reads reactive state).
pub(crate) struct Frame {
    pub props: HashMap<String, (String, bool)>,
    /// Default-slot content, generated in the parent's context.
    pub children: Vec<Node>,
    pub parent_hash: String,
    pub parent_ids: std::collections::HashSet<String>,
}

/// Default imports of `ui/*.brain` files that can be inlined, by local name. `disk` is the parent's file on disk.
pub(crate) fn candidates(c: &Component, disk: &Path, hash_len: usize) -> HashMap<String, Inline> {
    let mut out = HashMap::new();
    let dir = disk.parent().unwrap_or(Path::new("."));
    let rel_dir = Path::new(&c.file).parent().unwrap_or(Path::new(""));
    for s in &c.script {
        let Stmt::Import(i) = s else { continue };
        let Some(local) = &i.default else { continue };
        if !i.source.ends_with(".brain") || !(i.source.starts_with("./") || i.source.starts_with("../")) || i.named.len() + i.namespace.iter().count() > 0 {
            continue;
        }
        let file = normalize(&rel_dir.join(&i.source));
        if brain_parse::level_of(&file) != Level::Micro {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(dir.join(&i.source)) else { continue };
        let Ok(comp) = brain_parse::parse(&src, &file) else { continue };
        if !eligible(&comp) {
            continue;
        }
        let file = file.to_string_lossy().replace('\\', "/");
        out.insert(
            local.name.clone(),
            Inline { hash: crate::component_hash_len(&file, hash_len), source: i.source.clone(), has_style: comp.style.is_some(), comp },
        );
    }
    out
}

/// Stateless and self-contained: only `props` metadata, no script, no ids, no handlers or bindings, no nested
/// components, no named slots, no lists or awaits (their item names could shadow a prop).
fn eligible(c: &Component) -> bool {
    c.kind == FileKind::Component && c.script.is_empty() && c.meta.iter().all(|m| m.name == "props") && c.props.iter().all(|p| !p.bindable) && nodes_ok(&c.template)
}

fn nodes_ok(nodes: &[Node]) -> bool {
    nodes.iter().all(|n| match n {
        Node::Text(_) => true,
        Node::Interp(i) => !i.html,
        Node::Slot(s) => s.name.is_none() && s.attrs.is_empty(),
        Node::Element(e) => {
            e.attrs.iter().all(|a| matches!(a.kind, AttrKind::Plain | AttrKind::Class | AttrKind::Style) && !(a.name == "id" || a.name == "slot"))
                && nodes_ok(&e.children)
        }
        Node::If(b) => b.branches.iter().all(|br| nodes_ok(&br.body)) && b.otherwise.as_deref().is_none_or(nodes_ok),
        Node::Each(_) | Node::Await(_) | Node::ComponentRef(_) | Node::SlotScope(_) => false,
    })
}

/// A use can be inlined when it only passes plain props the child declares (no events, bindings or named slots).
pub(crate) fn use_ok(r: &ComponentRef, child: &Component) -> bool {
    r.attrs.iter().all(|a| a.kind == AttrKind::Plain && child.props.iter().any(|p| p.name == a.name))
        && r.children.iter().all(|n| !matches!(n, Node::Element(e) if e.attrs.iter().any(|a| a.name == "slot")) && !matches!(n, Node::SlotScope(_)))
        && (r.children.iter().all(|n| matches!(n, Node::Text(t) if t.value.trim().is_empty())) || has_default_slot(&child.template))
}

fn has_default_slot(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Slot(s) => s.name.is_none(),
        Node::Element(e) => has_default_slot(&e.children),
        Node::If(b) => b.branches.iter().any(|br| has_default_slot(&br.body)) || b.otherwise.as_deref().is_some_and(has_default_slot),
        _ => false,
    })
}

/// Lexical `a/./b/../c` → `a/c`.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in p.components() {
        match part {
            PathPart::CurDir => {}
            PathPart::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
