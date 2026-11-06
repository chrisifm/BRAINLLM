//! A lazy mount cannot remove a component already statically imported for an eager mount.
use crate::{Ctx, Imported, component_source, scope::{self, Kind, Scopes}};
use brain_parse::{Diag, ast::*};
use std::collections::BTreeMap;

pub fn check(component: &Component, ctx: &Ctx) -> Vec<Diag> {
    let mut uses = BTreeMap::<String, (bool, Vec<&ComponentRef>)>::new();
    let mut scopes = Scopes::new(scope::top_bindings(component));
    visit(&component.template, &mut scopes, &mut |reference, scopes| {
        if scopes.lookup(&reference.name) != Some(Kind::Import) { return; }
        let Some(source) = component_source(component, &reference.name) else { return; };
        if !matches!(ctx.imports.get(source), Some(Imported::Brain(_))) { return; }
        // Stateless ui/ uses may be inlined in production, leaving no eager component module.
        // This checker does not duplicate codegen eligibility: keep that case unknown.
        let mut path = std::path::PathBuf::new();
        for part in std::path::Path::new(&component.file).parent().unwrap_or(std::path::Path::new("")).join(source).components() {
            match part {
                std::path::Component::CurDir => {},
                std::path::Component::ParentDir => { path.pop(); },
                other => path.push(other.as_os_str()),
            }
        }
        if brain_parse::level_of(&path) == Level::Micro { return; }
        let entry = uses.entry(path.to_string_lossy().into_owned()).or_default();
        if reference.attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == "lazy") {
            entry.1.push(reference);
        } else { entry.0 = true; }
    });
    uses.values().filter(|(eager, _)| *eager).flat_map(|(_, lazy)| lazy).map(|reference| {
        Diag::new("B0225", &component.file, reference.span,
            format!("`<{} lazy>` also has an eager use in this file; its module stays in the eager JavaScript graph", reference.name),
            "keep every use of this component lazy to defer its JavaScript, or keep the eager use intentionally; lazy still delays this instance's mount and does not reduce the total route budget")
    }).collect()
}

fn visit<'a>(nodes: &'a [Node], scopes: &mut Scopes, f: &mut impl FnMut(&'a ComponentRef, &Scopes)) {
    for node in nodes {
        match node {
            Node::Element(element) => visit(&element.children, scopes, f),
            Node::ComponentRef(reference) => { f(reference, scopes); visit(&reference.children, scopes, f); }
            Node::If(block) => {
                for branch in &block.branches { visit(&branch.body, scopes, f); }
                if let Some(body) = &block.otherwise { visit(body, scopes, f); }
            }
            Node::Each(block) => {
                scopes.push(); scopes.declare_pat(&block.item, Kind::EachItem);
                if let Some(index) = &block.index { scopes.declare(&index.name, Kind::EachItem); }
                visit(&block.body, scopes, f); scopes.pop();
                if let Some(body) = &block.empty { visit(body, scopes, f); }
            }
            Node::Await(block) => {
                visit(&block.pending, scopes, f);
                for branch in [&block.then, &block.catch].into_iter().flatten() {
                    scopes.push();
                    if let Some(binding) = &branch.binding { scopes.declare_pat(binding, Kind::EachItem); }
                    visit(&branch.body, scopes, f); scopes.pop();
                }
            }
            Node::SlotScope(block) => {
                scopes.push(); for name in &block.names { scopes.declare(&name.name, Kind::EachItem); }
                visit(&block.body, scopes, f); scopes.pop();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn warnings(markup: &str) -> Vec<Diag> {
        let source = format!("<script>import Card from './card.brain'\nimport Alias from './card.brain'</script>{markup}");
        let component = brain_parse::parse(&source, std::path::Path::new("views/example.brain")).unwrap();
        let mut ctx = Ctx::default(); ctx.imports.insert("./card.brain".into(), Imported::Brain(vec![]));
        check(&component, &ctx)
    }
    #[test]
    fn mixed_modes_warn_at_each_lazy_use_including_aliases_and_nested_branches() {
        for markup in ["<Card/><Card lazy/>", "<Alias/><section><Card lazy=\"idle\"/></section>",
            "{#if true}<Card/>{:else}<Alias lazy=\"interaction\"/>{/if}"] {
            let warnings = warnings(markup); assert_eq!(warnings.len(), 1, "{markup}");
            assert_eq!(warnings[0].level, brain_lex::Severity::Warning);
            assert!(!warnings[0].fix.is_empty()); assert!(warnings[0].edits.is_empty());
        }
    }
    #[test]
    fn potentially_inlined_ui_is_not_reported_as_an_eager_module() {
        let component = brain_parse::parse("<script>import Card from '../ui/card.brain'</script><Card/><Card lazy/>", std::path::Path::new("views/example.brain")).unwrap();
        let mut ctx = Ctx::default(); ctx.imports.insert("../ui/card.brain".into(), Imported::Brain(vec![]));
        assert!(check(&component, &ctx).is_empty());
    }
    #[test]
    fn lazy_only_eager_only_and_shadowed_names_do_not_warn() {
        for markup in ["<Card lazy/><Alias lazy=\"idle\"/>", "<Card/><Alias/>",
            "<Card lazy/>{#each [] as Card (Card)}<Card/>{/each}",
            "<Card lazy/>{#await Promise.resolve()}{:then Card}<Card/>{/await}",
            "<Card lazy/>{#slot Card}<Card/>{/slot}"] { assert!(warnings(markup).is_empty(), "{markup}"); }
    }
}
