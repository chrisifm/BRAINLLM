//! Runtime export liveness is a graph fixed point, not a HashMap iteration side effect.
use crate::{Node, Optimization, feedback};
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
    // Read once per source revision; a shake that changes a module refreshes only its summary. A module that does not
    // lex keeps its error: it pins dynamic liveness and stops named liveness, as reading it directly would.
    let mut summaries: HashMap<Node, Result<brain_min::ModuleImports, String>> = sources.iter().map(|(node, source)| (node.clone(), brain_min::module_imports(source))).collect();
    // A pruning already proven to change nothing is not repeated: (module, its source, sorted live exports).
    let mut stable: HashSet<(Node, String, Vec<String>)> = HashSet::new();
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
                "brain-links.js" => ["default".to_string()].into(),
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
            if summaries.iter().any(|(other, imports)| {
                *other != node && imports.as_ref().map_or(true, |imports| imports.dynamic.iter().any(|spec| matches(spec)))
            }) {
                continue;
            }
            for (other, imports) in &summaries {
                if *other != node {
                    keep.extend(imports.as_ref().map_err(Clone::clone)?.names_from(&matches));
                }
            }
            changed |= shake(sources, &mut summaries, &mut stable, node, keep, reports)?;
        }
        // Recompute core roots AFTER sibling pruning: removed functions must not
        // permanently pin their former imports. Mount/hydrate are generated inline.
        if sources.contains_key(&core) {
            let mut keep: HashSet<String> = ["m".to_string(), "h".to_string()].into();
            for (other, imports) in &summaries {
                if *other != core {
                    keep.extend(imports.as_ref().map_err(Clone::clone)?.names_from(&brain_min::is_runtime_spec));
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
                changed |= shake(sources, &mut summaries, &mut stable, core.clone(), keep, reports)?;
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
    summaries: &mut HashMap<Node, Result<brain_min::ModuleImports, String>>,
    stable: &mut HashSet<(Node, String, Vec<String>)>,
    node: Node,
    keep: HashSet<String>,
    reports: &mut Vec<Optimization>,
) -> Result<bool, String> {
    let original = &sources[&node];
    let mut live: Vec<String> = keep.iter().cloned().collect();
    live.sort();
    let proof = (node.clone(), original.clone(), live);
    if stable.contains(&proof) {
        return Ok(false);
    }
    let (next, detail) = feedback::shake(original, &keep)?;
    if next == *original {
        stable.insert(proof);
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
    summaries.insert(node.clone(), brain_min::module_imports(&next));
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
    fn a_wide_graph_is_read_once_per_source_revision() {
        let mut sources = fixture(false);
        for n in 0..200 {
            sources.insert(Node::File(format!("views/v{n}.brain")), format!("import{{formStatic}}from'/@brain/brain-form.js';export default()=>formStatic({n});"));
        }
        let modules = sources.len() as u64;
        let build = brain_metrics::Build::new();
        let observed = brain_metrics::attach(&build);
        apply(&mut sources, &HashMap::new(), &mut Vec::new()).unwrap();
        drop(observed);
        let lexes = build.get(brain_metrics::Counter::JsLexes);
        // one summary per module, plus what pruning itself reads for the few runtime modules
        assert!(lexes < modules + 40, "{lexes} lexes for {modules} modules");
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
