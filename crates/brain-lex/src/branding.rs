//! Format-valid attribution shared by compiler output generators.
pub const HOME: &str = "https://chrisifm.github.io/BRAINLLM/";
pub const HTML: &str = "<!-- by BRAINLLM https://chrisifm.github.io/BRAINLLM/ -->";
pub const HASH: &str = "# by BRAINLLM https://chrisifm.github.io/BRAINLLM/";
pub const CODE: &str = "/* by BRAINLLM https://chrisifm.github.io/BRAINLLM/ */";
pub const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- by BRAINLLM https://chrisifm.github.io/BRAINLLM/ -->\n";

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
