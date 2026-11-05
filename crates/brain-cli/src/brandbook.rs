//! A living public brand reference: authored decisions plus observed project design facts, never invented claims.
use brain_parse::{Diag, Span, ast::{AttrValue, Node}};
use serde_json::{Value, json};
use std::{collections::{BTreeMap,BTreeSet}, path::{Path, PathBuf}};

pub const VIEW: &str = "views/brandbook.brain";
pub const MARKER: &str = "<section data-brain-brandbook=\"content\"></section>";
pub const TEMPLATE: &str = r#"<meta seo title="Brandbook" description="The project brand reference: strategy, voice, identity, product standards, channels, governance and research, with current tokens and components." canonical="/brandbook" robots="noindex">
<main class="brandbook">
  <h1>Brandbook</h1>
  <p>Drafts need approval. Token contrast and UI accessibility remain unverified.</p>
  <section data-brain-brandbook="content"></section>
</main>
<style>
  .brandbook { --brandbook-space: 1em; --brandbook-space-small: .5em; display: grid; gap: var(--brandbook-space); padding: var(--brandbook-space); }
  .brandbook section { display: grid; gap: var(--brandbook-space-small); }
  .brandbook nav ul { display: flex; flex-wrap: wrap; gap: var(--brandbook-space-small); list-style: none; padding: 0; }
  .brandbook table { width: 100%; border-collapse: collapse; text-align: start; }
  .brandbook th, .brandbook td { padding: var(--brandbook-space-small); vertical-align: top; overflow-wrap: anywhere; }
  .brandbook code { overflow-wrap: anywhere; white-space: pre-wrap; }
</style>
"#;

type Section = (&'static str, &'static str, &'static str);
const GROUPS: &[(&str, &[Section])] = &[
    ("Strategy", &[
        ("purpose", "Purpose and promise", "State the problem, purpose, promise, values and limits in concrete language."),
        ("audience", "Audience", "Describe the people served, their context and needs; distinguish evidence from assumptions."),
        ("evidence", "Audience evidence", "Cite approved research with dates, scope, limitations and what it changes. Never invent metrics."),
        ("personas", "User personas", "Use clearly labeled illustrative personas; avoid presenting assumptions as measured people."),
        ("segments", "Segmentation", "Connect each audience segment to its need, message and relevant experience."),
        ("personality", "Personality and mood", "Define a few observable traits and how the interface should feel."),
        ("locales", "Regions and languages", "Define supported language, terminology, formats and localization choices without stereotyping."),
    ]),
    ("Voice", &[
        ("voice", "Voice and situational tone", "Keep a stable voice; describe tone for onboarding, errors, support and sensitive moments."),
        ("writing", "Writing rules", "Use plain, specific language; list preferred terms, prohibited claims and useful examples."),
        ("verbal-platform", "Verbal platform", "Document positioning, value proposition and a short explanation of the offer."),
        ("signature-lines", "Signature phrases", "Record only approved signature language and where each phrase belongs."),
        ("connection-lines", "Audience messages", "Show messages for real audience situations rather than generic slogans."),
        ("topics", "Editorial topics", "Describe the subjects the brand can credibly address and its boundaries."),
        ("microcopy", "Product microcopy", "Give examples for actions, labels, help, validation, empty states and recovery."),
    ]),
    ("Identity and product", &[
        ("logo", "Logo", "Identify approved logo assets, variants and meaningful text alternatives."),
        ("clear-space", "Clear space and minimum size", "Specify practical clear space, minimum sizes and small-format alternatives."),
        ("misuse", "Incorrect uses", "Explain distortions, contrast failures and alterations that must be avoided."),
        ("color", "Color", "Explain semantic color roles and pairings; use the current token inventory below as the implementation reference."),
        ("typography", "Typography", "Define type roles, hierarchy, fallbacks, readability and font licensing."),
        ("surfaces", "Backgrounds and surfaces", "Describe layering, emphasis and the treatment of content and controls."),
        ("motion", "Rhythm and motion", "Describe meaningful motion, duration, interruption and reduced-motion behavior."),
        ("support-assets", "Supporting assets", "Define photography, supporting shapes, composition and reusable asset rules."),
        ("ambassador", "Brand representative", "If applicable, document an approved character or representative and consistent usage; otherwise state why it is not applicable."),
        ("illustration", "Illustration and graphics", "Define visual grammar and useful examples without introducing unrelated styles."),
        ("iconography", "Iconography", "Define icon style, sizes, labels, stroke consistency and accessible alternatives."),
        ("components", "Components and states", "Document reused controls and permission, loading, error, empty, success and disabled states."),
        ("accessibility", "Accessibility", "Set practical keyboard, focus, contrast, touch, text-size and assistive-technology requirements."),
        ("seo", "SEO and titles", "Define useful titles, descriptions, structured data and public indexing intent."),
        ("social-preview", "Sharing previews", "Describe social-card composition, text boundaries and asset reuse."),
        ("email", "Email", "Define transactional and editorial email voice, structure, fallbacks and consent requirements."),
        ("implementation", "Implementation", "Link public implementation guidance and design contracts; keep secrets and server internals private."),
    ]),
    ("Assistant", &[
        ("assistant-scope", "Assistant responsibilities", "If applicable, state the user tasks an assistant can actually complete and when it hands off."),
        ("assistant-style", "Assistant response style", "Define concise, understandable answers, uncertainty handling and helpful next steps."),
        ("assistant-boundaries", "Assistant boundaries", "State prohibited claims, sensitive-data boundaries and escalation rules; do not invent unsupported capabilities."),
    ]),
    ("Channels and governance", &[
        ("social", "Social channels", "Define channel purpose, audience, formats and approved publishing responsibilities."),
        ("video", "Video and thumbnails", "Define visual hierarchy, titles, pacing, captions and accessibility for video."),
        ("advertising", "Advertising", "Set truthful claims, approved assets, disclosure and measurable campaign intent."),
        ("requirements", "Mandatory elements", "List required elements by context without imposing irrelevant elements everywhere."),
        ("ai-policy", "AI policy", "Define permitted AI assistance, review, attribution and prohibited fabricated evidence."),
        ("brand-policy", "Brand policies", "Describe consistency, collaboration and exceptions with an explicit review owner."),
        ("legal", "Legal", "Link reviewed public policies and licensing obligations; mark unreviewed legal decisions as pending."),
        ("governance", "Governance", "Record ownership, approval process, decision history and how exceptions are resolved."),
        ("downloads", "Downloads", "List approved public assets and usage instructions; never expose private working files."),
    ]),
    ("Research", &[
        ("competition", "Competition", "Compare relevant alternatives using cited facts and explicit dates."),
        ("benchmarks", "Benchmarks", "Document useful reference patterns, criteria and provenance rather than copying a brand."),
        ("gaps", "Open gaps", "Record unresolved brand decisions with an owner and acceptance criteria."),
        ("product-standard", "Product quality standard", "Define observable quality requirements for the actual product or service."),
        ("risks", "Risks", "State plausible communication and experience risks and evidence-based mitigations."),
        ("crisis", "Crisis and reputation", "Define escalation, verified facts and who can approve a public response."),
        ("experiments", "Experiments", "Separate hypotheses, proposed tests and accepted findings."),
        ("measurement", "Measurement", "Define useful outcomes and their measurement limits; do not publish private analytics by default."),
        ("inclusion", "Inclusive language", "Describe respectful, understandable language and review for discriminatory assumptions."),
        ("emerging-tech", "Emerging technology", "Describe relevant technology effects using current evidence and clear uncertainty."),
        ("technology-limits", "Technology limitations", "Explain limits and practical safeguards without implying unsupported capability."),
        ("audience-context", "Audience-specific context", "Describe how needs change across relevant roles or situations, with evidence."),
        ("technology-language", "Technology language", "Record accurate technology terminology, approved explanations and claims to avoid."),
        ("references", "References", "Keep public sources, research scope and review dates traceable."),
    ]),
];

pub fn defaults() -> Value {
    let entries: serde_json::Map<String, Value> = GROUPS.iter().flat_map(|(_,sections)| sections.iter()).map(|(id,_,prompt)| ((*id).into(), json!({"status":"draft","text":prompt,"evidence":[]}))).collect();
    json!({"schemaVersion":1,"sections":entries})
}
fn error(file: &str, message: impl Into<String>) -> Diag {
    Diag::new("B2203", file, Span { line:1, col:1, ..Default::default() }, message, "correct the brandbook contract and keep the owned placeholder on its own line; read brain guide brandbook")
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;").replace('{', "&#123;").replace('}', "&#125;").replace('\n', "&#10;").replace('\r', "&#13;")
}
fn markers(nodes: &[Node], out: &mut Vec<Span>) {
    for node in nodes {
        match node {
            Node::Element(element) => {
                if element.attrs.iter().any(|attr| attr.name == "data-brain-brandbook" && matches!(&attr.value, AttrValue::Text(value) if value == "content")) { out.push(element.span); }
                markers(&element.children, out);
            }
            Node::If(block) => { for branch in &block.branches { markers(&branch.body,out); } if let Some(nodes)=&block.otherwise { markers(nodes,out); } }
            Node::Each(block) => { markers(&block.body,out); if let Some(nodes)=&block.empty { markers(nodes,out); } }
            Node::ComponentRef(component) => markers(&component.children,out),
            _ => {}
        }
    }
}
pub struct Prepared { pub source: String, pub deps: Vec<PathBuf>, pub line: u32, pub col: u32, pub file: String, pub diagnostics: Vec<Diag> }
impl Prepared {
    pub fn remap(&self, mut diagnostic: Diag) -> Diag {
        if self.line != 0 && diagnostic.line == self.line && diagnostic.file.replace('\\',"/") == self.file {
            diagnostic.col = self.col;
            diagnostic.edits.clear();
            diagnostic.msg = format!("generated brandbook content: {}", diagnostic.msg);
            if diagnostic.code=="B1403" {
                diagnostic.fix="run brain fix to register generated messages, then translate the locale files; keep the owned placeholder empty and rename any local t helper".into();
            }
        }
        diagnostic
    }
}
/// Only a structural, empty owned element in the actual brandbook route is expanded. Authored pages remain intact.
pub fn prepare(source: &str, rel: &Path, root: &Path) -> Result<Prepared, Diag> {
    prepare_for(source,rel,root,rel)
}
pub fn prepare_for(source: &str, rel: &Path, root: &Path, message_scope: &Path) -> Result<Prepared, Diag> {
    let mut prepared = Prepared { source:source.into(), deps:vec![], line:0, col:0, file:rel.to_string_lossy().replace('\\',"/"), diagnostics:vec![] };
    let rel = rel.to_string_lossy().replace('\\', "/");
    if ![VIEW, "views/brandbook/index.brain"].contains(&rel.as_str()) || !source.contains("data-brain-brandbook") { return Ok(prepared); }
    let Ok(component) = brain_parse::parse(source, Path::new(&rel)) else { return Ok(prepared); };
    let mut found = Vec::new(); markers(&component.template, &mut found);
    if found.is_empty() { return Ok(prepared); }
    if found.len() != 1 { return Err(error(&rel, "the brandbook route must contain one owned placeholder")); }
    let span = found[0];
    let start = span.start as usize; let end = span.end as usize;
    if source.get(start..end) != Some(MARKER) || source[..start].rsplit('\n').next().is_some_and(|s| !s.trim().is_empty()) || source[end..].split('\n').next().is_some_and(|s| !s.trim().is_empty()) {
        return Err(error(&rel, "keep the exact empty brandbook placeholder on its own line"));
    }
    let design_path = root.join(crate::design::FILE);
    let raw = crate::map::read_source_limit(root, crate::design::FILE, 1024 * 1024).map_err(|message| error(crate::design::FILE, message))?;
    let mut design: Value = serde_json::from_str(&raw).map_err(|_| error(crate::design::FILE, "invalid design JSON"))?;
    let book = design.get("brandbook").cloned().unwrap_or_else(defaults);
    let rendered = render(root, &book, &mut prepared.deps)?;
    prepared.deps.push(design_path);
    prepared.source.replace_range(start..end, &format!("<section data-brain-brandbook=\"content\">{rendered}</section>"));
    prepared.line = span.line; prepared.col = span.col;
    if let Some(i18n) = brain_check::i18n::load(root) {
        let component = brain_parse::parse(&prepared.source, message_scope).map_err(|_|error(&rel,"generated brandbook could not be parsed"))?;
        let mut aliases:BTreeMap<String,String> = match book.get("messages") {
            None => BTreeMap::new(),
            Some(value) => serde_json::from_value(value.clone()).map_err(|_|error(crate::design::FILE,"brandbook.messages must map original text to dictionary keys"))?,
        };
        if aliases.len()>4096 || aliases.iter().any(|(text,key)|text.len()>16384 || key.is_empty() || key.len()>256) { return Err(error(crate::design::FILE,"brandbook message identities exceed their bounds")); }
        let messages=brain_check::known_literal_messages(&component,&i18n,&aliases);
        let mut edits=Vec::new();
        for (edit,key,text) in messages.into_iter().filter(|message|message.0.line==span.line) {
            aliases.insert(text,key); edits.push(edit);
        }
        if aliases.len()>4096 || aliases.keys().any(|text|text.len()>16384) { return Err(error(crate::design::FILE,"brandbook message identities exceed their bounds")); }
        prepared.source = Diag::apply(&prepared.source,&edits).ok_or_else(||error(&rel,"generated brandbook locale edits overlap"))?;
        prepared.deps.extend(i18n.files.iter().map(|file|root.join(&file.file)));
        if !aliases.is_empty() && book.get("messages") != Some(&json!(aliases)) {
            let mut updated=book.clone();updated["messages"]=json!(aliases);design["brandbook"]=updated;
            let replacement=serde_json::to_string_pretty(&design).map_err(|_|error(crate::design::FILE,"cannot serialize brandbook message identities"))?+"\n";
            if replacement.len()>1024*1024 { return Err(error(crate::design::FILE,"brandbook translation identities exceed the design contract size limit")); }
            let mut diagnostic=Diag::new("B1403",crate::design::FILE,Span {line:1,col:1,..Default::default()},"record managed brandbook translation identities","run brain fix to preserve message keys when default translations change").edit(0,raw.len() as u32,replacement);
            diagnostic.locate(&raw); prepared.diagnostics.push(diagnostic);
        }
    }
    Ok(prepared)
}
fn render(root: &Path, book: &Value, deps: &mut Vec<PathBuf>) -> Result<String, Diag> {
    let fail = |message: &str| error(crate::design::FILE, message);
    if book["schemaVersion"].as_u64() != Some(1) { return Err(fail("brandbook.schemaVersion must be 1")); }
    let sections = book["sections"].as_object().ok_or_else(|| fail("brandbook.sections must be an object keyed by stable section IDs"))?;
    let known: BTreeSet<&str> = GROUPS.iter().flat_map(|(_,s)| s.iter().map(|(id,_,_)| *id)).collect();
    if sections.keys().any(|id| !known.contains(id.as_str())) { return Err(fail("unknown brandbook section ID; use the IDs from brain guide brandbook")); }
    let mut html = String::from("<nav aria-label=\"Brandbook sections\"><ul>");
    for (i,(group,_)) in GROUPS.iter().enumerate() { html.push_str(&format!("<li><a href=\"#brandbook-group-{i}\">{}</a></li>", escape(group))); }
    html.push_str("<li><a href=\"#brandbook-current-design\">Current design</a></li></ul></nav>");
    for (i,(group,entries)) in GROUPS.iter().enumerate() {
        html.push_str(&format!("<section id=\"brandbook-group-{i}\"><h2>{}</h2>",escape(group)));
        for (id,title,prompt) in *entries {
            let entry = sections.get(*id);
            let status = match entry { None => "draft", Some(value) => value["status"].as_str().ok_or_else(|| fail("each declared brandbook section needs an explicit string status"))? };
            if !["draft","proposed","approved","implemented","not-applicable"].contains(&status) { return Err(fail("brandbook status must be draft, proposed, approved, implemented or not-applicable")); }
            let text = match entry { None => *prompt, Some(value) => value["text"].as_str().ok_or_else(|| fail("brandbook section text must be a string"))? };
            if text.trim().is_empty() || text.len() > 4000 { return Err(fail("brandbook section text must contain 1..4000 bytes")); }
            if !brain_check::scan_secrets(text).is_empty() { return Err(fail("brandbook text contains a detected credential; keep secrets out of this public reference")); }
            // Default authoring instructions remain in the editable contract. Public drafts show their status,
            // not a second copy of the writing checklist. Authored decisions and evidence are always rendered.
            let untouched = status == "draft" && text == *prompt && entry.and_then(|value|value.get("evidence")).is_none_or(|value|value.as_array().is_some_and(|values|values.is_empty()));
            let displayed = if untouched { "Content and evidence pending review." } else { text };
            html.push_str(&format!("<h3 id=\"brandbook-{id}\">{}</h3><p>{}. {}</p>", escape(title),escape(status),escape(displayed)));
            if let Some(value) = entry.and_then(|e| e.get("evidence")) {
                let evidence = value.as_array().filter(|v|v.len() <= 8).ok_or_else(|| fail("brandbook evidence must be a list of at most 8 public source descriptions"))?;
                if !evidence.is_empty() { html.push_str("<ul aria-label=\"Declared evidence\">"); }
                for value in evidence {
                    let text = value.as_str().filter(|s| !s.trim().is_empty() && s.len() <= 512).ok_or_else(|| fail("each evidence description must contain 1..512 bytes"))?;
                    if !brain_check::scan_secrets(text).is_empty() { return Err(fail("brandbook evidence contains a detected credential")); }
                    html.push_str(&format!("<li>{}</li>", escape(text)));
                }
                if !evidence.is_empty() { html.push_str("</ul>"); }
            }
        }
        html.push_str("</section>");
    }
    let (token_file, token_source) = crate::design::token_source(root).map_err(|message| fail(&message))?;
    deps.push(root.join(&token_file));
    let current = crate::design::baseline(&token_source, &token_file, None);
    if current["validation"]["valid"] != true { return Err(fail("current project tokens cannot be described safely; run brain design check")); }
    html.push_str(&format!("<section id=\"brandbook-current-design\"><h2>Current design</h2><p>Declared in <code>{}</code>.</p><table><caption>Current token declarations</caption><thead><tr><th scope=\"col\">Token</th><th scope=\"col\">Role</th><th scope=\"col\">Value</th><th scope=\"col\">Context</th></tr></thead><tbody>",escape(&token_file)));
    if let Some(tokens) = current["tokens"].as_object() {
        for (name,token) in tokens {
            if let Some(definitions) = token["definitions"].as_array() {
                for definition in definitions {
                    let context = definition["context"].as_array().map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" / ")).unwrap_or_default();
                    html.push_str(&format!("<tr><th scope=\"row\"><code>{}</code></th><td><code>{}</code></td><td><code>{}</code></td><td><code>{}</code></td></tr>",escape(name),escape(token["category"].as_str().unwrap_or("other")),escape(definition["value"].as_str().unwrap_or("")),escape(&context)));
                }
            }
        }
    }
    html.push_str("</tbody></table><h3>Available UI files</h3><p>Source inventory only; no examples are executed.</p><ul>");
    inventory(root, &root.join("ui"), 0, &mut 0, &mut html, deps)?;
    html.push_str("</ul></section>");
    if html.len() > 512 * 1024 { return Err(fail("generated brandbook exceeds 512 KiB; simplify public entries before building")); }
    Ok(html)
}
fn inventory(root: &Path, dir: &Path, depth: usize, count: &mut usize, html: &mut String, deps: &mut Vec<PathBuf>) -> Result<(), Diag> {
    if depth > 8 || *count > 1024 { return Err(error(crate::design::FILE,"brandbook UI inventory exceeds its bounds")); }
    deps.push(dir.into());
    let metadata = match std::fs::symlink_metadata(dir) { Ok(m) => m, Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()), Err(_) => return Err(error(crate::design::FILE,"cannot inspect UI inventory")) };
    if metadata.file_type().is_symlink() || !metadata.is_dir() { return Err(error(crate::design::FILE,"brandbook UI inventory needs an ordinary directory")); }
    let mut entries = std::fs::read_dir(dir).map_err(|_|error(crate::design::FILE,"cannot read UI inventory"))?.collect::<Result<Vec<_>,_>>().map_err(|_|error(crate::design::FILE,"cannot read UI entry"))?;
    entries.sort_by_key(|e|e.file_name());
    for entry in entries {
        *count += 1; if *count > 1024 { return Err(error(crate::design::FILE,"brandbook UI inventory exceeds 1024 entries")); }
        let kind=entry.file_type().map_err(|_|error(crate::design::FILE,"cannot inspect UI entry"))?;
        if entry.file_name().to_string_lossy().starts_with('.') || kind.is_symlink() { continue; }
        if kind.is_dir() { inventory(root,&entry.path(),depth+1,count,html,deps)?; }
        else if kind.is_file() && entry.path().extension().is_some_and(|ext|ext=="brain") && !entry.file_name().to_string_lossy().ends_with(".test.brain") {
            let path=entry.path(); deps.push(path.clone());
            html.push_str(&format!("<li><code>{}</code></li>",escape(&path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\',"/"))));
        }
    }
    Ok(())
}
pub fn missing(root: &Path) -> Option<Diag> {
    if root.join(VIEW).is_file() || root.join("views/brandbook/index.brain").is_file() { return None; }
    let mut diagnostic = Diag::new("B2202", "views/", Span {line:1,col:1,..Default::default()}, "this site has no /brandbook reference for brand, voice and design decisions", "run brain add view brandbook, then read brain guide brandbook; record real public decisions and keep drafts explicit");
    diagnostic.level=brain_parse::Severity::Warning;
    Some(diagnostic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64,Ordering};
    struct App(PathBuf);
    impl Drop for App { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
    fn app() -> App {
        static NEXT:AtomicU64=AtomicU64::new(0);
        let app=App(std::env::temp_dir().join(format!("brain-brandbook-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed))));
        crate::scaffold::new_app(&app.0).unwrap();app
    }
    #[test]
    fn expansion_uses_current_tokens_and_inventory_without_overwriting_editorial_decisions() {
        let app=app();
        let path=app.0.join(crate::design::FILE);
        let mut design:Value=serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        design["brandbook"]["sections"]["purpose"]=json!({"status":"approved","text":"Keep {literal} text and <script> as text, not code.","evidence":["Public review, recorded by the project owner"]});
        std::fs::write(&path,serde_json::to_vec(&design).unwrap()).unwrap();
        let editorial=std::fs::read(&path).unwrap();
        let before=prepare(TEMPLATE,Path::new(VIEW),&app.0).unwrap();
        assert!(before.source.contains("Strategy") && before.source.contains("Current design"));
        assert!(before.source.contains("Content and evidence pending review."));
        assert!(!before.source.contains("Describe the people served, their context and needs"));
        assert!(before.source.contains("&#123;literal&#125;") && before.source.contains("&lt;script&gt;"));
        assert!(before.source.contains("approved."));
        let tokens=app.0.join(crate::scaffold::TOKENS_FILE);
        let mut source=std::fs::read_to_string(&tokens).unwrap();source.push_str("\n:root { --brandbook-sample: #13579b; }\n");
        std::fs::write(tokens,&source).unwrap();
        std::fs::write(app.0.join("ui/sample.brain"),"<span>Sample</span>").unwrap();
        let after=prepare(TEMPLATE,Path::new(VIEW),&app.0).unwrap();
        assert!(after.source.contains("--brandbook-sample") && after.source.contains("#13579b") && after.source.contains("ui/sample.brain"));
        assert!(after.deps.contains(&app.0.join("ui")) && after.deps.contains(&path));
        assert_eq!(std::fs::read(&path).unwrap(),editorial);
        let adopted=crate::design::baseline(&source,crate::scaffold::TOKENS_FILE,Some(&design));
        assert_eq!(adopted["brandbook"],design["brandbook"]);
    }
    #[test]
    fn authored_routes_and_script_strings_are_not_owned_markers_and_duplicate_nodes_fail() {
        let app=app();
        let authored="<main><h1>Our brand</h1></main>";
        assert_eq!(prepare(authored,Path::new(VIEW),&app.0).unwrap().source,authored);
        let string="<script>const sample = '<section data-brain-brandbook=\"content\"></section>'</script><main><h1>{sample}</h1></main>";
        assert_eq!(prepare(string,Path::new(VIEW),&app.0).unwrap().source,string);
        let duplicate=TEMPLATE.replace(MARKER,&format!("{MARKER}\n{MARKER}"));
        assert!(prepare(&duplicate,Path::new(VIEW),&app.0).is_err());
        assert_eq!(prepare(TEMPLATE,Path::new("views/other.brain"),&app.0).unwrap().source,TEMPLATE);
    }
    #[test]
    fn compiler_check_and_persistent_compile_cache_see_the_same_expanded_source() {
        let app=app();let path=app.0.join(VIEW);
        let (diags,deps)=crate::compiler::check(TEMPLATE,Path::new(VIEW),&path);
        let errors:Vec<_>=diags.iter().filter(|d|!d.is_warning()).collect();
        assert!(errors.is_empty(),"{errors:?}");
        assert!(deps.contains(&app.0.join(crate::design::FILE)));
        let first=crate::compiler::compile(TEMPLATE,Path::new(VIEW),&app.0,false).unwrap();
        let design_path=app.0.join(crate::design::FILE);
        let mut design:Value=serde_json::from_str(&std::fs::read_to_string(&design_path).unwrap()).unwrap();
        design["brandbook"]["sections"]["purpose"]["text"]=json!("A revised public promise.");
        std::fs::write(design_path,serde_json::to_vec(&design).unwrap()).unwrap();
        let second=crate::compiler::compile(TEMPLATE,Path::new(VIEW),&app.0,false).unwrap();
        assert_ne!(first.js,second.js);
        assert!(second.js.contains("A revised public promise"));
        assert!(second.deps.contains(&app.0.join("ui")));
    }
    #[test]
    fn nested_app_checks_expand_the_real_route_and_report_the_outer_relative_contract_path() {
        let parent=app(); let child=parent.0.join("child"); crate::scaffold::new_app(&child).unwrap();
        let path=child.join(crate::design::FILE);
        let mut design:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        design["brandbook"]["sections"]["voice"]["status"]=json!(true);
        std::fs::write(path,serde_json::to_vec(&design).unwrap()).unwrap();
        let (diags,_)=crate::compiler::check(TEMPLATE,Path::new("child/views/brandbook.brain"),&child.join(VIEW));
        assert!(diags.iter().any(|d|d.code=="B2203" && d.file=="child/design.brain.json"),"{diags:?}");
    }
    #[test]
    fn nested_managed_locale_keys_use_the_same_reported_scope_as_the_checker() {
        let parent=app();let child=parent.0.join("child");crate::scaffold::new_app(&child).unwrap();
        let path=child.join("brain.json");let mut config:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        config["i18n"]=json!({"default":"en","locales":["en","es"]});
        std::fs::write(path,serde_json::to_vec(&config).unwrap()).unwrap();
        std::fs::create_dir_all(child.join("i18n")).unwrap();
        let reported=Path::new("child/views/brandbook.brain");
        let key=format!("{}.strategy",brain_check::i18n::key_scope(&reported.to_string_lossy()));
        for locale in ["en","es"] { std::fs::write(child.join(format!("i18n/{locale}.json")),json!({&key:"Strategy"}).to_string()).unwrap(); }
        let prepared=prepare_for(TEMPLATE,Path::new(VIEW),&child,reported).unwrap();
        assert!(prepared.source.contains(&format!("t('{key}')")),"{}",prepared.source);
        let (diags,_)=crate::compiler::check(TEMPLATE,reported,&child.join(VIEW));
        assert!(!diags.iter().any(|d|d.code=="B1403" && d.file==reported.to_string_lossy() && d.msg.contains("\"Strategy\"")),"{diags:?}");
        assert!(diags.iter().any(|d|d.code=="B1403" && d.file=="child/design.brain.json" && !d.edits.is_empty()),"{diags:?}");
    }
    #[test]
    fn generated_line_mapping_never_changes_an_authorial_dependency_diagnostic() {
        let app=app(); let prepared=prepare(TEMPLATE,Path::new(VIEW),&app.0).unwrap();
        let mut diagnostic=Diag::new("B0406","ui/other.brain",Span {line:prepared.line,col:17,..Default::default()},"button has no text","add meaningful text");
        diagnostic.edits.push(brain_lex::Edit {text:"Example".into(),..Default::default()});
        assert_eq!(prepared.remap(diagnostic.clone()),diagnostic);
        diagnostic.file=VIEW.into();
        let generated=prepared.remap(diagnostic);
        assert!(generated.msg.starts_with("generated brandbook content:"));
        assert!(generated.edits.is_empty());
    }
}
