//! Report one exact project-token reuse opportunity; dynamic cascade changes require an author-reviewed source edit.
use crate::Ctx;
use brain_parse::{Diag, ast::Component};
use brain_css::CssItem;

pub fn check(component:&Component,ctx:&Ctx)->Vec<Diag> {
    if !ctx.design_system {return Vec::new();}
    let (Some(sheet),Some(tokens))=(&component.style,&ctx.tokens) else {return Vec::new();};
    fn category(property:&str)->Option<&'static str> {
        if matches!(property,"color"|"background-color"|"border-color"|"outline-color"|"fill"|"stroke") {Some("color")}
        else if property.starts_with("padding") || property.starts_with("margin") || matches!(property,"gap"|"row-gap"|"column-gap") {Some("spacing")}
        else if property.contains("radius") {Some("radius")}
        else if property=="font-family" {Some("font")}
        else {None}
    }
    fn token_category(name:&str)->Option<&'static str> {
        if name.starts_with("--space") || name.starts_with("--spacing") || name.contains("gutter") {Some("spacing")}
        else if name.contains("radius") {Some("radius")}
        else if name.contains("font") {Some("font")}
        else {Some("color")}
    }
    fn walk(items:&[CssItem],component:&Component,tokens:&[(String,String)],out:&mut Vec<Diag>) {
        for item in items {
            match item {
                CssItem::Rule{items,..}|CssItem::At{block:Some(items),..}=>walk(items,component,tokens,out),
                CssItem::Decl{name,value,span}=>{
                    let Some(role)=category(name) else {continue;};
                    let literal=value.trim();
                    if literal.contains("var(") || matches!(literal,"0"|"0px"|"auto"|"none"|"inherit"|"initial"|"unset"|"revert"|"currentColor"|"transparent") {continue;}
                    let matches:Vec<_>=tokens.iter().filter(|(name,value)| token_category(name)==Some(role) && value.trim()==literal).collect();
                    if matches.len()!=1 {continue;}
                    let token=&matches[0].0;
                    let mut diag=Diag::new("B2201",&component.file,*span,"This design value already has one exact project token",format!("Use var({token}) to keep the component aligned with design.brain.json"));
                    diag.level=brain_parse::Severity::Warning;
                    if out.len()<128 {out.push(diag);}
                },
                _=>{}
            }
        }
    }
    let mut out=Vec::new(); walk(&sheet.items,component,tokens,&mut out);out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn findings(source:&str, tokens:Vec<(String,String)>,enabled:bool)->Vec<Diag> {
        let component=brain_parse::parse(source,Path::new("comp/card.brain")).unwrap();
        check(&component,&Ctx { tokens:Some(tokens),design_system:enabled,..Ctx::default() })
    }
    #[test]
    fn exact_unique_same_role_token_is_an_author_reviewed_warning() {
        let ds=vec![("--space-4".into(),"16px".into()),("--ink".into(),"#222".into())];
        let found=findings("<p>Card</p><style>p { padding:16px; color:#222 }</style>",ds.clone(),true);
        assert_eq!(found.len(),2);
        assert!(found.iter().all(|d|d.code=="B2201" && d.level==brain_parse::Severity::Warning && d.edits.is_empty()));
        assert!(found.iter().any(|d|d.fix.contains("var(--space-4)")));
        assert!(findings("<p>Card</p><style>p { padding:16px }</style>",ds,false).is_empty());
    }
    #[test]
    fn ambiguous_tokens_roles_and_existing_variables_are_preserved() {
        let ds=vec![("--space-4".into(),"16px".into()),("--spacing-card".into(),"16px".into()),("--ink".into(),"#222".into())];
        assert!(findings("<p>Card</p><style>p { padding:16px; width:#222; color:var(--ink) }</style>",ds,true).is_empty());
    }
}
