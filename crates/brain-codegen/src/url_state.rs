//! Default public, scalar select/tab state. Arbitrary form values and repeated lexical bindings stay private.
use brain_check::scope::{self, Kind};
use brain_parse::ast::*;
use std::collections::BTreeSet;

pub fn setup(component: &Component) -> String {
    let top = scope::top_bindings(component);
    let mut names = BTreeSet::new();
    fn walk(nodes: &[Node], names: &mut BTreeSet<String>) {
        for node in nodes {
            let (attrs, children, public) = match node {
                Node::Element(e) => (&e.attrs, &e.children, e.tag == "select"),
                Node::ComponentRef(c) => (&c.attrs, &c.children, matches!(c.name.as_str(), "Select" | "Tabs")),
                Node::If(b) => {
                    for branch in &b.branches { walk(&branch.body, names); }
                    if let Some(other) = &b.otherwise { walk(other, names); }
                    continue;
                }
                // A repeated instance needs an explicit stable key; never infer an index or serialize a form field.
                _ => continue,
            };
            if public && !attrs.iter().any(|a| a.name == "data-url-state" && matches!(&a.value, AttrValue::Text(v) if v == "off")) {
                for attr in attrs {
                    if attr.kind == AttrKind::Bind && attr.name == "value" {
                        if let AttrValue::Expr(Expr::Ident(id)) = &attr.value { names.insert(id.name.clone()); }
                    }
                }
            }
            walk(children, names);
        }
    }
    walk(&component.template, &mut names);
    let mut output = String::new();
    for name in names {
        if top.get(&name) != Some(&Kind::Sig) { continue; }
        // Explicit urlState() already owns its public key.
        let explicit = component.script.iter().any(|s| match s {
            Stmt::Var(v) => v.decls.iter().any(|d| matches!((&d.pat, d.init.as_ref()),
                (Pat::Ident(i), Some(Expr::Call { callee, .. })) if i.name == name && matches!(callee.unparen(), Expr::Ident(f) if f.name == "urlState"))),
            _ => false,
        });
        if explicit { continue; }
        let key = format!("ui.{}.{}", crate::component_hash_len(&component.file.replace('\\', "/"),12), name);
        output.push_str(&format!("  $urlBind({name}, {});\n", crate::js_str(&key)));
    }
    output
}
