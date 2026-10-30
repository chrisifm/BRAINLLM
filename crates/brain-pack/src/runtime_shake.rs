//! Runtime export liveness is a graph fixed point, not a HashMap iteration side effect.
use crate::{Node, Optimization, dynamic_import_of, feedback};
use std::collections::{HashMap, HashSet};

pub(super) fn apply(
    sources: &mut HashMap<Node, String>,
    server_roots: &HashMap<String, HashSet<String>>,
    reports: &mut Vec<Optimization>,
) -> Result<(), String> {
    let core = Node::Runtime("brain.js".into());
    let mut siblings: Vec<String> = sources
        .keys()
        .filter_map(|node| match node {
            Node::Runtime(name) if name != "brain.js" && !name.ends_with(".brain") => {
                Some(name.clone())
            }
            _ => None,
        })
        .collect();
    siblings.sort();
    // Each pass only removes proven unused exports/bindings. A dependency chain can
    // propagate liveness changes across at most N runtime modules; one pass proves stability.
    let bound = siblings.len() + usize::from(sources.contains_key(&core)) + 1;
    for _ in 0..bound {
        let mut changed = false;
        for name in &siblings {
            let node = Node::Runtime(name.clone());
            let mut keep: HashSet<String> = match name.as_str() {
                "brain-router.js" => ["router".to_string()].into(),
                "brain-i18n.js" => ["add".to_string()].into(),
                _ => HashSet::new(),
            };
            if let Some(names) = server_roots.get(name) {
                if names.contains("*") {
                    continue;
                }
                keep.extend(names.iter().cloned());
            }
            let matches =
                |spec: &str| spec.ends_with(&format!("/{name}")) || spec == format!("./{name}");
            // Namespace properties reached through dynamic import are not inferred.
            if sources
                .iter()
                .any(|(other, source)| *other != node && dynamic_import_of(source, &matches))
            {
                continue;
            }
            for (other, source) in sources.iter() {
                if *other != node {
                    keep.extend(brain_min::imported_names(source, &matches)?);
                }
            }
            changed |= shake(sources, node, keep, reports)?;
        }
        // Recompute core roots AFTER sibling pruning: removed functions must not
        // permanently pin their former imports. Mount/hydrate are generated inline.
        if sources.contains_key(&core) {
            let mut keep: HashSet<String> = ["m".to_string(), "h".to_string()].into();
            for (other, source) in sources.iter() {
                if *other != core {
                    keep.extend(brain_min::imported_names(
                        source,
                        &brain_min::is_runtime_spec,
                    )?);
                }
            }
            if let Some(names) = server_roots.get("brain.js") {
                keep.extend(names.iter().map(|name| {
                    brain_min::RUNTIME_MAP
                        .iter()
                        .find(|(long, _)| long == name)
                        .map_or(name.as_str(), |(_, short)| *short)
                        .to_string()
                }));
            }
            if !keep.contains("*") {
                changed |= shake(sources, core.clone(), keep, reports)?;
            }
        }
        if !changed {
            return Ok(());
        }
    }
    Err("Runtime export optimization exceeded its bounded convergence passes".into())
}
fn shake(
    sources: &mut HashMap<Node, String>,
    node: Node,
    keep: HashSet<String>,
    reports: &mut Vec<Optimization>,
) -> Result<bool, String> {
    let original = &sources[&node];
    let (next, detail) = feedback::shake(original, &keep)?;
    if next == *original {
        return Ok(false);
    }
    let Node::Runtime(name) = &node else {
        return Err("Runtime optimization received a non-runtime module".into());
    };
    reports.push(Optimization {
        source: format!("runtime/{name}"),
        original_source_hash: None,
        stage: "runtime-tree-shake",
        detail,
    });
    sources.insert(node, next);
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(reverse: bool) -> HashMap<Node, String> {
        let mut entries = vec![
            (
                Node::File("views/login.brain".into()),
                "import{formStatic}from'/@brain/brain-form.js';export default()=>formStatic({name:['str',true]});",
            ),
            (
                Node::Runtime("brain.js".into()),
                "export function m(){}export function h(){}export function s(){return 1}export function x(){return 2}",
            ),
            (
                Node::Runtime("brain-form.js".into()),
                "import{rule,checkText}from'./brain-schema.js';export function form(s){return rule(s)}export function formStatic(s){return checkText(s)}",
            ),
            (
                Node::Runtime("brain-schema.js".into()),
                "import{x}from'./brain.js';export function rule(s){return x()+'generic parser'+s}export function checkText(s){return s}",
            ),
        ];
        if reverse {
            entries.reverse();
        }
        entries
            .into_iter()
            .map(|(node, source)| (node, source.into()))
            .collect()
    }
    #[test]
    fn fixed_point_removes_transitive_parser_and_core_imports_for_every_map_order() {
        let mut first = fixture(false);
        apply(&mut first, &HashMap::new(), &mut Vec::new()).unwrap();
        let schema = &first[&Node::Runtime("brain-schema.js".into())];
        assert!(
            !schema.contains("generic parser") && !schema.contains("{x}"),
            "{schema}"
        );
        assert!(schema.contains("./brain.js"), "module evaluation remains observable");
        assert!(!first[&Node::Runtime("brain.js".into())].contains("function x"));
        for index in 0..12 {
            let mut fresh = fixture(index % 2 == 0);
            apply(&mut fresh, &HashMap::new(), &mut Vec::new()).unwrap();
            assert_eq!(fresh, first);
        }
    }
    #[test]
    fn server_roots_dynamic_namespaces_and_module_evaluation_are_preserved() {
        let mut sources = fixture(false);
        sources.insert(
            Node::Runtime("brain-effects.js".into()),
            "import{external}from'foreign';external;globalThis.effect=1;export function unused(){}"
                .into(),
        );
        sources.insert(
            Node::File("lib/lazy.js".into()),
            "export const lazy=()=>import('./brain-dynamic.js')".into(),
        );
        sources.insert(
            Node::Runtime("brain-dynamic.js".into()),
            "export function available(){return 42}".into(),
        );
        let roots = [("brain-schema.js".into(), ["rule".into()].into())].into();
        apply(&mut sources, &roots, &mut Vec::new()).unwrap();
        assert!(sources[&Node::Runtime("brain-schema.js".into())].contains("generic parser"));
        let effect = &sources[&Node::Runtime("brain-effects.js".into())];
        assert!(effect.contains("external") && effect.contains("globalThis.effect=1"));
        assert!(sources[&Node::Runtime("brain-dynamic.js".into())].contains("available"));
    }
}
