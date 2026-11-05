//! Artifact validation for ordinary HTTP static hosting, without framework URL rewrites.
use brain_mcp::Json;
use brain_seo::html::{attr, elements, elements_with_document_scope};
use reqwest::Url;
use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};

const MAX_FILES: usize = 20_000;
const MAX_TEXT: u64 = 16 * 1024 * 1024;
const MAX_TOTAL: u64 = 256 * 1024 * 1024;
// WAI-ARIA IDREF/IDREFS and HTML document-ID associations (never navigation URLs).
const ID_REFS: &[&str] = &["aria-activedescendant", "aria-controls", "aria-describedby", "aria-details", "aria-errormessage", "aria-flowto", "aria-labelledby", "aria-owns", "for", "list", "form", "headers", "itemref", "popovertarget", "commandfor"];

#[derive(Clone)]
struct Reference { file: String, url: String, base: Url, fragment: bool, integrity: Option<String>, external: bool }

/// The declaration scan is deliberately separate from browser execution. External URLs are counted, never fetched.
pub(crate) fn audit(root: &Path) -> Result<Json, String> {
    audit_output(root, &root.join("dist"))
}

pub(crate) fn audit_output(root: &Path, dist: &Path) -> Result<Json, String> {
    let mut files = BTreeMap::new();
    collect(dist, dist, &mut files, 0)?;
    let config = brain_seo::Config::load(root)?.with_env(root, "production");
    let origin = crate::env::value(root, "production", "PUBLIC_SITE_URL").or(config.site).unwrap_or_else(|| "https://static-audit.invalid".into());
    let site = Url::parse(&origin).map_err(|_| "static audit needs a valid site origin")?;
    let prefix = brain_pack::read_base(root);
    let mut references = Vec::new();
    let mut ids = BTreeMap::new();
    let mut id_targets = BTreeMap::new();
    let mut id_references = Vec::new();
    let mut issues = Vec::new();
    let mut total = 0;
    let mut pages = 0;
    for (file, size) in &files {
        if file == "server.mjs" || file.starts_with("_server/") {
            issues.push(issue(file, "", "server artifact in a static export"));
        }
        if !matches!(file.rsplit('.').next(), Some("html" | "svg" | "css" | "js" | "mjs")) { continue; }
        total += size;
        if *size > MAX_TEXT || total > MAX_TOTAL { return Err("static artifact analysis exceeds its text-size limit; simplify the output before retrying".into()); }
        let text = fs::read_to_string(dist.join(file)).map_err(|_| format!("cannot read static text file {file}"))?;
        let base = file_url(&site, &prefix, file)?;
        match file.rsplit('.').next().unwrap_or("") {
            "html" | "svg" => {
                if file.ends_with(".html") { pages += 1; }
                let mut document_ids = BTreeSet::new();
                let mut element_ids = BTreeSet::new();
                // Standalone SVG is XML: an unqualified element called template is not an HTML template.
                let scoped = if file.ends_with(".svg") { elements(&text).into_iter().map(|(tag, attrs, body)| (tag, attrs, body, true)).collect() }
                    else { elements_with_document_scope(&text) };
                for (tag, attrs, body, in_document) in scoped {
                    for key in ["id", "name"] {
                        if !in_document { continue; }
                        if key == "name" && tag != "a" { continue; }
                        if let Some(value) = attr(&attrs, key) {
                            document_ids.insert(value.to_string());
                            if key == "id" && !element_ids.insert(value.to_string()) {
                                issues.push(issue(file, value, "duplicate document ID"));
                            }
                        }
                    }
                    for key in ID_REFS {
                        if !in_document { continue; }
                        // HTML's association names only apply to the elements that define them; an app's custom
                        // data-like attribute on another element does not establish a document relationship.
                        let applies = key.starts_with("aria-") || match *key {
                            "for" => matches!(tag.as_str(), "label" | "output"),
                            "list" => tag == "input",
                            "form" => matches!(tag.as_str(), "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"),
                            "headers" => matches!(tag.as_str(), "td" | "th"),
                            "itemref" => attr(&attrs, "itemscope").is_some(),
                            "popovertarget" => matches!(tag.as_str(), "button" | "input"),
                            "commandfor" => tag == "button",
                            _ => false,
                        };
                        if applies && let Some(value) = attr(&attrs, key) {
                            for target in value.split_ascii_whitespace() {
                                if id_references.len() + references.len() >= 200_000 { return Err("static artifact exceeds the declared-reference analysis limit".into()); }
                                id_references.push((file.clone(), *key, target.to_string()));
                            }
                        }
                    }
                    if in_document && tag == "base" && attr(&attrs, "href").is_some() {
                        issues.push(issue(file, attr(&attrs, "href").unwrap_or(""), "base elements change URL resolution and are unsupported by the static artifact audit"));
                    }
                    for key in ["src", "href", "xlink:href", "poster", "data"] {
                        if key == "data" && tag != "object" { continue; }
                        if tag == "base" { continue; }
                        if let Some(url) = attr(&attrs, key) {
                            let external = key == "href" && matches!(tag.as_str(), "a" | "area") && brain_seo::html::intentional_external(url, attr(&attrs, "rel").unwrap_or(""));
                            references.push(Reference { file: file.clone(), url: url.into(), base: base.clone(), fragment: in_document && matches!(tag.as_str(), "a" | "area" | "use"), integrity: attr(&attrs, "integrity").map(str::to_string), external });
                        }
                    }
                    for key in ["srcset", "imagesrcset"] {
                        if let Some(value) = attr(&attrs, key) {
                            for url in srcset(value) { references.push(reference(file, &base, url)); }
                        }
                    }
                    if let Some(style) = attr(&attrs, "style") { add_css(file, &base, style, &mut references)?; }
                    if let Some(body) = body {
                        if tag == "style" { add_css(file, &base, body, &mut references)?; }
                        if tag == "script" && attr(&attrs, "type") == Some("module") { add_js(file, &base, body, &mut references)?; }
                    }
                }
                ids.insert(file.clone(), document_ids);
                id_targets.insert(file.clone(), element_ids);
            }
            "css" => add_css(file, &base, &text, &mut references)?,
            _ => add_js(file, &base, &text, &mut references)?,
        }
        if references.len() + id_references.len() > 200_000 { return Err("static artifact exceeds the declared-reference analysis limit".into()); }
    }
    if pages == 0 { issues.push(issue("dist/", "", "static output contains no HTML pages")); }
    let id_reference_count = id_references.len();
    for (file, key, target) in id_references {
        if !id_targets.get(&file).is_some_and(|ids| ids.contains(&target)) {
            issues.push(issue(&file, &target, &format!("missing document ID reference from {key}")));
        }
    }
    let mut external = BTreeSet::new();
    let mut checked = 0;
    let mut digests = BTreeMap::new();
    let mut hashed_bytes = 0;
    for r in references {
        let url = match r.base.join(r.url.trim()) { Ok(url) => url, Err(_) => { issues.push(issue(&r.file, &r.url, "invalid resource URL")); continue; } };
        if !matches!(url.scheme(), "http" | "https") { continue; }
        if r.external || url.origin() != site.origin() { external.insert(url.origin().ascii_serialization()); continue; }
        checked += 1;
        // A plain directory host redirects its unslashed mount root to the directory index.
        let path = if url.path() == prefix.trim_end_matches('/') { Some("") } else { url.path().strip_prefix(&prefix) };
        let Some(path) = path else { issues.push(issue(&r.file, &r.url, "local URL escapes the configured hosting base")); continue; };
        let decoded = if path.is_empty() { String::new() } else {
            match brain_lex::url_path::decode_relative(path.trim_end_matches('/')) { Ok(path) => path, Err(_) => { issues.push(issue(&r.file, &r.url, "unsafe or invalid encoded resource path")); continue; } }
        };
        let target = if files.contains_key(&decoded) && !url.path().ends_with('/') { Some(decoded.clone()) }
            else { let index = if decoded.is_empty() { "index.html".into() } else { format!("{decoded}/index.html") }; files.contains_key(&index).then_some(index) };
        let Some(target) = target.filter(|p| !brain_pack::private::is_private(p)) else { issues.push(issue(&r.file, &r.url, "missing resource or page on ordinary static hosting")); continue; };
        if let Some(fragment) = url.fragment().filter(|f| !f.is_empty() && !f.starts_with(":~:text=")) && r.fragment {
            let fragment = percent_text(fragment)?;
            if ids.get(&target).is_some_and(|ids| !ids.contains(&fragment)) { issues.push(issue(&r.file, &r.url, "missing anchor or SVG symbol")); }
        }
        if let Some(integrity) = r.integrity {
            let expected = match digests.get(&target) {
                Some(digest) => digest,
                None => {
                    let size = files[&target]; hashed_bytes += size;
                    if size > MAX_TEXT || hashed_bytes > MAX_TOTAL { return Err(format!("static integrity check exceeds its byte limit: {target}")); }
                    let bytes = fs::read(dist.join(&target)).map_err(|_| format!("cannot read integrity target {target}"))?;
                    digests.entry(target.clone()).or_insert_with(|| brain_pack::sha::integrity(&bytes))
                }
            };
            if integrity.split_ascii_whitespace().any(|token| token.starts_with("sha512-")) {
                issues.push(issue(&r.file, &r.url, "SHA-512 integrity requires browser verification; the generated SHA-384 artifact audit is incomplete"));
            } else if !integrity.split_ascii_whitespace().any(|token| token == expected) {
                issues.push(issue(&r.file, &r.url, "integrity does not match the final emitted bytes (audit supports generated SHA-384 integrity)"));
            }
        }
    }
    Ok(Json::obj([
        ("ok", Json::Bool(issues.is_empty())), ("mode", Json::str("static-artifact")),
        ("pages", Json::Num(pages as f64)), ("files", Json::Num(files.len() as f64)),
        ("references", Json::Num((checked + id_reference_count) as f64)), ("issues", Json::Arr(issues)),
        ("externalOriginsUnchecked", Json::Arr(external.iter().map(Json::str).collect())),
        ("coverage", Json::str("All emitted HTML/SVG attributes, srcset, CSS URLs/imports, literal module imports and new URL resources; local targets, fragments, duplicate IDs, ARIA/HTML document ID references and generated SHA-384 integrity. Template contents are excluded from document ID scope; instantiated templates and shadow roots need browser tests. Does not execute JavaScript, submit forms or fetch external services. Dynamic URLs and interaction paths still require browser tests.")),
    ]))
}

fn issue(file: &str, url: &str, message: &str) -> Json {
    // Diagnostic output never repeats a resolved PUBLIC_SITE_URL value. The emitted file and path identify the source.
    let reference = Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https")).map(|u| format!("{}{}", u.path(), u.fragment().map(|f| format!("#{f}")).unwrap_or_default()));
    let url = reference.as_deref().unwrap_or(url);
    Json::obj([("file", Json::str(file)), ("reference", Json::str(url)), ("message", Json::str(message)), ("fix", Json::str("correct the source reference or emit the missing resource, rebuild, and rerun brain doctor"))])
}
fn reference(file: &str, base: &Url, url: String) -> Reference { Reference { file: file.into(), base: base.clone(), url, fragment: false, integrity: None, external: false } }
fn collect(root: &Path, at: &Path, files: &mut BTreeMap<String, u64>, depth: usize) -> Result<(), String> {
    if depth > 64 { return Err("static artifact exceeds the directory depth limit".into()); }
    let meta = fs::symlink_metadata(at).map_err(|_| "cannot inspect static output")?;
    if meta.file_type().is_symlink() { return Err("static audit refuses symlinked output; rebuild ordinary confined files".into()); }
    if meta.is_dir() {
        for entry in fs::read_dir(at).map_err(|_| "cannot read static output")? { collect(root, &entry.map_err(|_| "cannot read static output entry")?.path(), files, depth + 1)?; }
    } else if meta.is_file() {
        if files.len() >= MAX_FILES { return Err("static artifact exceeds the file-count analysis limit".into()); }
        files.insert(at.strip_prefix(root).map_err(|_| "static path escapes dist")?.to_string_lossy().replace('\\', "/"), meta.len());
    } else { return Err("static output contains a non-regular file".into()); }
    Ok(())
}
fn file_url(site: &Url, prefix: &str, file: &str) -> Result<Url, String> {
    let mut url = site.clone();
    url.set_path(prefix);
    {
        let mut segments = url.path_segments_mut().map_err(|_| "static site URL cannot be a base")?;
        segments.pop_if_empty();
        for segment in file.split('/') { segments.push(segment); }
    }
    // An index document is requested through its directory on an ordinary HTTP host.
    if file == "index.html" || file.ends_with("/index.html") { url.path_segments_mut().map_err(|_| "static site URL cannot be a base")?.pop().push(""); }
    Ok(url)
}
fn percent_text(text: &str) -> Result<String, String> {
    let hex = |b: u8| (b as char).to_digit(16).map(|v| v as u8);
    let mut bytes = Vec::new(); let input = text.as_bytes(); let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            let hi = input.get(i + 1).and_then(|b| hex(*b)).ok_or("invalid percent-encoded fragment")?;
            let lo = input.get(i + 2).and_then(|b| hex(*b)).ok_or("invalid percent-encoded fragment")?;
            bytes.push(hi * 16 + lo); i += 3;
        } else { bytes.push(input[i]); i += 1; }
    }
    String::from_utf8(bytes).map_err(|_| "invalid UTF-8 fragment".into())
}
fn add_js(file: &str, base: &Url, source: &str, refs: &mut Vec<Reference>) -> Result<(), String> {
    for (_, _, url) in brain_pack::specifiers(source)?.into_iter().chain(brain_pack::url_refs(source)?) {
        if url.contains('\\') { return Err(format!("static audit cannot resolve escaped module URL in {file}")); }
        refs.push(reference(file, base, url));
    }
    Ok(())
}
fn add_css(file: &str, base: &Url, source: &str, refs: &mut Vec<Reference>) -> Result<(), String> {
    for url in css_urls(source)? { refs.push(reference(file, base, url)); }
    Ok(())
}
fn srcset(source: &str) -> Vec<String> {
    let mut out = Vec::new(); let mut rest = source;
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        let end = rest.find(|c: char| c.is_ascii_whitespace()).unwrap_or(rest.len());
        let url = &rest[..end]; rest = &rest[end..];
        if !url.is_empty() { out.push(url.trim_end_matches(',').into()); }
        if url.ends_with(',') { continue; }
        // Descriptor commas inside parentheses are not candidate delimiters.
        let mut depth = 0;
        let end = rest.char_indices().find_map(|(i,c)| { if c == '(' { depth += 1; } if c == ')' { depth -= 1; } (c == ',' && depth == 0).then_some(i + 1) }).unwrap_or(rest.len());
        rest = &rest[end..];
    }
    out
}

/// Owned CSS lexical scan: comments and ordinary strings never introduce URL dependencies.
fn css_urls(source: &str) -> Result<Vec<String>, String> {
    let b = source.as_bytes(); let mut i = 0; let mut out = Vec::new();
    while i < b.len() {
        if b[i..].starts_with(b"/*") { i += 2; i += source[i..].find("*/").ok_or("unterminated CSS comment in static artifact")? + 2; continue; }
        let import = b[i..].get(..7).is_some_and(|s| s.eq_ignore_ascii_case(b"@import")) && b.get(i + 7).is_some_and(|c| c.is_ascii_whitespace() || matches!(c, b'\'' | b'"' | b'/'));
        let url = b[i..].get(..4).is_some_and(|s| s.eq_ignore_ascii_case(b"url(")) && (i == 0 || !(b[i-1].is_ascii_alphanumeric() || matches!(b[i-1], b'-' | b'_')));
        if import || url {
            i += if import { 7 } else { 4 };
            loop {
                while b.get(i).is_some_and(u8::is_ascii_whitespace) { i += 1; }
                if b[i..].starts_with(b"/*") { i += 2; i += source[i..].find("*/").ok_or("unterminated CSS import comment")? + 2; } else { break; }
            }
            if import && !matches!(b.get(i), Some(b'\'' | b'"')) { continue; }
            let quote = b.get(i).copied().filter(|c| matches!(c, b'\'' | b'"'));
            if quote.is_some() { i += 1; }
            let start = i;
            while i < b.len() && !quote.map_or(b[i] == b')', |q| b[i] == q) {
                if b[i] == b'\\' { return Err("escaped CSS URLs require browser verification; static analysis is incomplete".into()); }
                i += 1;
            }
            if i == b.len() { return Err("unterminated CSS URL in static artifact".into()); }
            out.push(source[start..i].trim().to_string()); i += 1;
        } else if matches!(b[i], b'\'' | b'"') {
            let quote = b[i]; i += 1;
            while i < b.len() && b[i] != quote { if b[i] == b'\\' { i += 1; } i += 1; }
            i = (i + 1).min(b.len());
        } else { i += 1; }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, sync::atomic::{AtomicU64, Ordering}};
    struct App(PathBuf);
    impl Drop for App { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    fn app() -> App {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let app = App(std::env::temp_dir().join(format!("brain-static-audit-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))));
        fs::create_dir_all(app.0.join("dist/docs")).unwrap();
        fs::create_dir_all(app.0.join("dist/js")).unwrap();
        fs::create_dir_all(app.0.join("dist/css")).unwrap();
        fs::create_dir_all(app.0.join("dist/img")).unwrap();
        fs::write(app.0.join("brain.json"), r#"{"output":"static","base":"/site/","site":"https://example.com"}"#).unwrap();
        let js = "export const x = new URL('../img/payload.bin',import.meta.url);";
        fs::write(app.0.join("dist/js/main.js"), js).unwrap();
        fs::write(app.0.join("dist/img/payload.bin"), [0,1,2,3]).unwrap();
        fs::write(app.0.join("dist/img/icons.svg"), "<svg><symbol id='ok'><path d='M0 0'/></symbol></svg>").unwrap();
        fs::write(app.0.join("dist/img/100%.svg"), "<svg/>").unwrap();
        fs::write(app.0.join("dist/css/site.css"), "/*url(/missing-comment)*/.x{background:url('../img/icons.svg')} .y::after{content:'url(/missing-string)'}").unwrap();
        fs::write(app.0.join("dist/index.html"), "<a href='/site/docs/#section'>Docs</a><a href='/site'>Home</a>").unwrap();
        fs::write(app.0.join("dist/docs/index.html"), format!("<link rel='stylesheet' href='../css/site.css'><h1 id='section'>Docs</h1><svg><use href='../img/icons.svg#ok'></use></svg><img src='../img/100%25.svg' srcset='data:image/svg+xml;base64,PHN2Zy8+ 1x, ../img/icons.svg 2x'><script type='module' src='../js/main.js' integrity='{}'></script><a href='https://other.example.com/'>External</a>", brain_pack::sha::integrity(js.as_bytes()))).unwrap();
        app
    }
    #[test]
    fn validates_directory_indexes_binary_module_resources_sprites_srcset_and_final_integrity() {
        let app = app();
        let report = audit(&app.0).unwrap();
        assert_eq!(report.get("ok"), Some(&Json::Bool(true)), "{}", report.stringify());
        assert_eq!(report.get("pages"), Some(&Json::Num(2.0)));
        assert_eq!(report.get("externalOriginsUnchecked"), Some(&Json::Arr(vec![Json::str("https://other.example.com")])));
        assert!(!report.stringify().contains("missing-comment") && !report.stringify().contains("missing-string"));
    }
    #[test]
    fn template_names_in_svg_stay_document_elements_and_html_integration_contents_stay_inert() {
        let app = app();
        fs::write(app.0.join("dist/img/foreign.svg"), "<svg xmlns='http://www.w3.org/2000/svg'><template><g id='target'/></template><g aria-labelledby='target'/></svg>").unwrap();
        fs::write(app.0.join("dist/index.html"), "<svg><template><g id='target'/></template><g aria-labelledby='target'/><foreignObject><template><p id='inert'>Inert</p></template></foreignObject></svg><p id='inert'>Live</p>").unwrap();
        let report = audit(&app.0).unwrap();
        assert_eq!(report.get("ok"), Some(&Json::Bool(true)), "{}", report.stringify());
    }
    #[test]
    fn intentional_external_navigation_does_not_exempt_relative_links_or_assets() {
        let app = app();
        fs::write(app.0.join("dist/index.html"), "<a rel='noopener ExTeRnAl' href='https://example.com/live-only/'>Live site</a>").unwrap();
        let report = audit(&app.0).unwrap();
        assert_eq!(report.get("ok"), Some(&Json::Bool(true)), "{}", report.stringify());
        assert!(report.get("externalOriginsUnchecked").unwrap().stringify().contains("https://example.com"));
        for markup in ["<a rel='external' href='/site/missing/'>Missing</a>", "<a href='https://example.com/missing/'>Missing</a>", "<img rel='external' src='https://example.com/site/missing.png'>"] {
            fs::write(app.0.join("dist/index.html"), markup).unwrap();
            let report = audit(&app.0).unwrap();
            assert_eq!(report.get("ok"), Some(&Json::Bool(false)), "{}", report.stringify());
        }
    }
    #[test]
    fn detects_broken_symbols_resources_anchors_and_postprocessing_integrity_damage() {
        let app = app();
        fs::write(app.0.join("dist/img/icons.svg"), "<svg><symbol id='changed'/></svg>").unwrap();
        fs::write(app.0.join("dist/js/main.js"), "import './missing.js';").unwrap();
        fs::write(app.0.join("dist/index.html"), "<a href='/site/docs/#absent'>Missing</a><img src='/outside.svg'>").unwrap();
        let report = audit(&app.0).unwrap();
        assert_eq!(report.get("ok"), Some(&Json::Bool(false)));
        let text = report.stringify();
        for expected in ["missing anchor or SVG symbol", "missing resource or page", "integrity does not match", "escapes the configured hosting base", "missing.js", "docs/index.html"] { assert!(text.contains(expected), "{expected}: {text}"); }
    }
    #[test]
    fn checks_document_id_references_and_duplicates_without_cross_page_or_named_anchor_fallback() {
        let app = app();
        let valid = "<label for='field'>Field</label><input id='field' list='choices' form='entry' aria-labelledby='title hint' aria-describedby='hint' aria-errormessage='hint'><h1 id='title'>Title</h1><p id='hint'>Hint</p><datalist id='choices'></datalist><form id='entry'></form>";
        fs::write(app.0.join("dist/index.html"), valid).unwrap();
        assert_eq!(audit(&app.0).unwrap().get("ok"), Some(&Json::Bool(true)));
        for markup in [
            "<h1 id='title'>Title</h1><section aria-labelledby='title missing'>Content</section>",
            "<a name='title'></a><section aria-labelledby='title'>Content</section>",
            "<p aria-describedby='section'>The ID exists only in the other page</p>",
            "<p id='same'>One</p><p id='same'>Two</p>",
            "<template><span id='title'>Inert</span></template><p aria-labelledby='title'>Missing live ID</p>",
        ] {
            fs::write(app.0.join("dist/index.html"), markup).unwrap();
            let report = audit(&app.0).unwrap();
            assert_eq!(report.get("ok"), Some(&Json::Bool(false)), "{}", report.stringify());
            assert!(report.stringify().contains("missing document ID reference") || report.stringify().contains("duplicate document ID"), "{}", report.stringify());
        }
        fs::write(app.0.join("dist/index.html"), "<h1 id='title'>Live title</h1><template><span id='title' aria-labelledby='later'>Inert</span><template><b id='title'>Nested</b></template></template><p aria-labelledby='title'>Valid reference</p>").unwrap();
        let report = audit(&app.0).unwrap();
        assert_eq!(report.get("ok"), Some(&Json::Bool(true)), "{}", report.stringify());
    }
    #[test]
    fn production_site_origin_from_the_environment_keeps_absolute_local_assets_in_scope() {
        let app = app();
        fs::write(app.0.join(".brain_env"), "[production]\nPUBLIC_SITE_URL=https://deployment.example.org\n").unwrap();
        fs::write(app.0.join("dist/index.html"), "<img src='https://deployment.example.org/site/missing.svg'>").unwrap();
        let report = audit(&app.0).unwrap();
        assert_eq!(report.get("ok"), Some(&Json::Bool(false)));
        let text = report.stringify();
        assert!(text.contains("/site/missing.svg") && !text.contains("deployment.example.org"), "{text}");
    }

    #[test]
    fn stronger_sri_and_compact_css_imports_cannot_report_a_false_success() {
        let app = app();
        let path = app.0.join("dist/docs/index.html");
        let html = fs::read_to_string(&path).unwrap();
        let digest = brain_pack::sha::integrity(&fs::read(app.0.join("dist/js/main.js")).unwrap());
        fs::write(path, html.replace(&digest, &format!("{digest} sha512-{}", brain_pack::sha::base64(&[0;64])))).unwrap();
        fs::write(app.0.join("dist/css/site.css"), "@import\"missing.css\";@import/**/\"missing-commented.css\";").unwrap();
        let report = audit(&app.0).unwrap();
        assert_eq!(report.get("ok"), Some(&Json::Bool(false)));
        let text = report.stringify();
        for expected in ["SHA-512", "missing.css", "missing-commented.css"] { assert!(text.contains(expected), "{text}"); }
    }

    #[test]
    fn refuses_server_artifacts_and_never_uses_framework_extensionless_rewrites() {
        let app = app();
        fs::write(app.0.join("dist/flat.html"), "<h1>Flat</h1>").unwrap();
        fs::write(app.0.join("dist/server.mjs"), "export const server = true;").unwrap();
        fs::write(app.0.join("dist/index.html"), "<a href='/site/flat'>Flat</a>").unwrap();
        let report = audit(&app.0).unwrap().stringify();
        assert!(report.contains("server artifact") && report.contains("missing resource or page"), "{report}");
    }
    #[test]
    fn css_and_srcset_scans_do_not_treat_comments_strings_or_data_commas_as_files() {
        assert_eq!(css_urls("@import\"compact.css\";@import/**/\"commented.css\";@import 'base.css'; .x{background:URL(\"image.png\")}/*url(no)*/.x{content:'url(no)'}").unwrap(), ["compact.css", "commented.css", "base.css", "image.png"]);
        assert_eq!(srcset("data:image/png;base64,AAAA 1x, image.png 2x, next.png 3x"), ["data:image/png;base64,AAAA", "image.png", "next.png"]);
        assert!(css_urls(".x{background:url('escaped\\20 name.png')}").is_err());
        assert!(css_urls("/*unterminated").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_output_cannot_escape_the_artifact() {
        let app = app();
        std::os::unix::fs::symlink(app.0.join("brain.json"), app.0.join("dist/outside.json")).unwrap();
        assert!(audit(&app.0).unwrap_err().contains("symlink"));
    }
}
