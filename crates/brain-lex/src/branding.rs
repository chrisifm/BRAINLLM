//! Format-valid attribution shared by compiler output generators.
pub const HOME: &str = "https://chrisifm.github.io/BRAINLLM/";
pub const DESCRIPTION: &str = "Ultrafast frontend and full-stack web compiler written in Rust";
pub const HTML: &str = "<!-- by BRAINLLM https://chrisifm.github.io/BRAINLLM/ -->";
pub const HASH: &str = "# by BRAINLLM https://chrisifm.github.io/BRAINLLM/";
pub const CODE: &str = "/* by BRAINLLM https://chrisifm.github.io/BRAINLLM/ */";
pub const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- by BRAINLLM https://chrisifm.github.io/BRAINLLM/ -->\n";

/// Attribution is on unless an author explicitly sets the project boolean to false.
pub fn enabled(config:&crate::Json)->Result<bool,String> {
    let crate::Json::Obj(fields)=config else {return Err("brain.json must be an object".into())};
    let values:Vec<_>=fields.iter().filter(|(key,_)|key=="compilerBranding").collect();
    if values.len()>1 {return Err("compilerBranding must be declared once".into())}
    match values.first().map(|(_,value)|value) {
        None|Some(crate::Json::Bool(true))=>Ok(true),Some(crate::Json::Bool(false))=>Ok(false),
        _=>Err("compilerBranding must be true or false; attribution stays enabled unless explicitly disabled".into()),
    }
}
pub fn code_with(source:&str,enabled:bool)->String {if enabled {code(source)} else {source.into()}}

/// Add a valid JS/CSS comment after minification, retaining shebang/@charset placement.
pub fn code(source: &str) -> String {
    if source.trim().is_empty() || source.trim_end().ends_with(CODE) { return source.into(); }
    format!("{source}\n{CODE}\n")
}
/// XML attribution follows an existing declaration; it never creates a second declaration.
pub fn xml(source: &str) -> String {
    if source.contains(HTML) { return source.into(); }
    if source.starts_with("<?xml") && let Some(end) = source.find("?>") {
        let end=end+2; return format!("{}\n{HTML}{}",&source[..end],&source[end..]);
    }
    format!("{HTML}\n{source}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_explicit_boolean_disables_project_attribution() {
        for (source,wanted) in [("{}",true),(r#"{"compilerBranding":true}"#,true),(r#"{"compilerBranding":false}"#,false)] {assert_eq!(enabled(&crate::Json::parse(source).unwrap()).unwrap(),wanted);}
        for source in [r#"{"compilerBranding":"false"}"#,r#"{"compilerBranding":null}"#,r#"{"compilerBranding":0}"#,r#"{"compilerBranding":false,"compilerBranding":true}"#] {assert!(enabled(&crate::Json::parse(source).unwrap()).is_err());}
        assert_eq!(code_with("export const value=1",false),"export const value=1");
        let authored=format!("const example={:?}",CODE);assert_eq!(code_with(&authored,false),authored);
    }

    #[test]
    fn attribution_keeps_special_prefixes_and_is_idempotent_without_reviving_empty_modules() {
        assert_eq!(code(""), "");
        for source in ["#!/usr/bin/env node\nconsole.log('Ready')", "@charset \"UTF-8\";body{color:blue}"] {
            let branded=code(source); assert!(branded.starts_with(source));
            assert!(branded.ends_with(&format!("{CODE}\n")));
            assert_eq!(code(&branded),branded);
        }
        let source="<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset/>";
        let branded=xml(source);
        assert!(branded.starts_with(XML));
        assert_eq!(branded.matches("<?xml").count(),1);
        assert_eq!(xml(&branded),branded);
    }
}
