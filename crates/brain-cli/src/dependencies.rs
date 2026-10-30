//! Bounded metadata analysis and deterministic per-file proposal checks. No provider/network/build execution.
mod vendor;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};
fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<String>,
    entries: &mut usize,
    truncated: &mut bool,
) {
    if depth > 24 || out.len() >= 128 || *entries >= 2048 {
        *truncated = true;
        return;
    }
    if dir
        .strip_prefix(root)
        .is_ok_and(|p| p == Path::new("vendor/brain"))
    {
        return;
    }
    if dir != root && crate::scaffold::is_app_dir(dir) {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        *truncated = true;
        return;
    };
    let mut paths = vec![];
    for item in read {
        if *entries >= 2048 {
            *truncated = true;
            break;
        }
        *entries += 1;
        if let Ok(item) = item {
            paths.push(item.path());
        }
    }
    paths.sort();
    for path in paths {
        if out.len() >= 128 {
            *truncated = true;
            break;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if name.starts_with('.')
            || [
                "node_modules",
                "dist",
                "target",
                "credentials",
                "secrets",
                ".git",
            ]
            .contains(&lower.as_str())
            || lower.contains(".env")
        {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            *truncated = true;
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk(root, &path, depth + 1, out, entries, truncated);
        } else if ["brain", "jsbrain", "js", "mjs"]
            .iter()
            .any(|ext| path.extension().is_some_and(|e| e == *ext))
            && !name.ends_with(".test.brain")
        {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}
fn script(source: &str, file: &str) -> Result<Option<String>, String> {
    if file.ends_with(".js") || file.ends_with(".mjs") {
        return Ok(Some(source.to_owned()));
    }
    let c = brain_parse::parse(source, Path::new(file))
        .map_err(|_| "source parse failed; run brain check for precise diagnostics")?;
    if c.meta
        .iter()
        .any(|m| m.name == "server" || m.name == "protect")
    {
        return Ok(None);
    }
    let Some(first) = c.script.first() else {
        return Ok(Some(String::new()));
    };
    let end = c.script.last().unwrap().span().end as usize;
    Ok(Some(source[first.span().start as usize..end].to_owned()))
}
/// analyze: file?; preview: file, expectedRevision, content. Proposals never modify installed SDK source.
pub fn query(root: &Path, op: &str, args: &Value) -> Result<Value, String> {
    if !args.is_object() {
        return Err("dependency arguments must be an object".into());
    }
    if op == "fetch" {
        return vendor::fetch(root, args);
    }
    if op == "preview" {
        return preview(root, args);
    }
    if op != "analyze" || args.as_object().unwrap().keys().any(|k| k != "file") {
        return Err("dependency analyze accepts only an optional source file".into());
    }
    let root = root.canonicalize().map_err(|_| "app root unavailable")?;
    let mut files = vec![];
    let mut entries = 0;
    let mut truncated = false;
    if let Some(file) = args.get("file") {
        let file = file
            .as_str()
            .ok_or("file must be an app-relative source path")?;
        crate::changes::source_path(&root, file)?;
        files.push(file.to_owned());
    } else {
        walk(&root, &root, 0, &mut files, &mut entries, &mut truncated);
    }
    let mut resolver = brain_pack::dependencies::Resolver::new(&root)?;
    let mut sources = BTreeMap::new();
    let mut edges = vec![];
    let mut skipped = vec![];
    let mut bytes = 0usize;
    let mut queue = vec![];
    let mut input_revisions = BTreeMap::new();
    let start = Instant::now();
    for file in files {
        if start.elapsed() > Duration::from_secs(15) {
            truncated = true;
            break;
        }
        let source = match crate::map::read_source_limit(&root, &file, 262144) {
            Ok(source) => source,
            Err(_) => {
                skipped.push(json!({"file":file,"reason":"guarded source unavailable"}));
                continue;
            }
        };
        bytes += source.len();
        if bytes > 2 * 1024 * 1024 {
            truncated = true;
            break;
        }
        input_revisions.insert(file.clone(), crate::manifests::hash(source.as_bytes()));
        let code = match script(&source, &file) {
            Ok(Some(code)) => code,
            Ok(None) => continue,
            Err(_) => {
                skipped.push(json!({"file":file,"reason":"parse failed; inspect brain check"}));
                continue;
            }
        };
        let specs = match brain_pack::specifiers(&code) {
            Ok(specs) => specs,
            Err(_) => {
                skipped.push(json!({"file":file,"reason":"script tokenization failed"}));
                continue;
            }
        };
        for (_, _, spec) in specs {
            if spec.starts_with('.') || spec.starts_with("/@brain/") {
                continue;
            }
            match resolver.resolve(&file, &spec) {
                Ok(Some(target)) => {
                    if edges.len() >= 2048 {
                        truncated = true;
                        break;
                    }
                    edges.push((file.clone(), spec, target.clone()));
                    queue.push(target);
                }
                Ok(None) => {}
                Err(error) => {
                    skipped.push(json!({"file":file,"specifier":public_spec(&spec),"reason":error}))
                }
            }
        }
        sources.insert(file, code);
    }
    let mut foreign = BTreeSet::new();
    while let Some(file) = queue.pop() {
        if !foreign.insert(file.clone()) {
            continue;
        }
        if foreign.len() > 512 {
            truncated = true;
            break;
        }
        let code = match resolver.read(&file) {
            Ok(code) => code,
            Err(error) => {
                skipped.push(json!({"file":file,"reason":error}));
                continue;
            }
        };
        if edges.len() >= 2048 {
            truncated = true;
            break;
        }
        for (_, _, spec) in brain_pack::specifiers(&code)? {
            match resolver.resolve(&file, &spec) {
                Ok(Some(target)) => {
                    if edges.len() >= 2048 {
                        truncated = true;
                        break;
                    }
                    edges.push((file.clone(), spec, target.clone()));
                    queue.push(target);
                }
                Ok(None) => {
                    skipped.push(json!({"file":file,"reason":"foreign module unexpectedly referenced a framework runtime"}));
                }
                Err(error) => {
                    skipped.push(json!({"file":file,"specifier":public_spec(&spec),"reason":error}))
                }
            }
        }
        for (_, _, spec) in brain_pack::url_refs(&code)? {
            if edges.len() >= 2048 {
                truncated = true;
                break;
            }
            match resolver.resolve_url(&file, &spec) {
                Ok(Some(target)) => {
                    let javascript = target.ends_with(".js") || target.ends_with(".mjs");
                    if javascript {
                        queue.push(target.clone());
                    } else if let Err(error) = resolver.read_asset(&target) {
                        skipped.push(json!({"file":file,"reason":error}));
                        continue;
                    }
                    edges.push((file.clone(), spec, target));
                }
                Ok(None) => {}
                Err(error) => skipped.push(json!({"file":file,"reason":error})),
            }
        }
        sources.insert(file, code);
    }
    resolver.verify()?;
    let mut modules = vec![];
    if foreign.len() > 128 {
        truncated = true;
    }
    let optimization_start = Instant::now();
    for file in foreign.into_iter().take(128) {
        if optimization_start.elapsed() > Duration::from_secs(15) {
            truncated = true;
            break;
        }
        let Some(source) = sources.get(&file) else {
            continue;
        };
        let mut keep = HashSet::new();
        let mut unknown = false;
        for (from, spec, target) in &edges {
            if target != &file {
                continue;
            }
            match sources
                .get(from)
                .map(|s| brain_pack::dependencies::used_exports(s, spec))
                .transpose()?
            {
                Some(Some(names)) => keep.extend(names),
                _ => {
                    unknown = true;
                    break;
                }
            }
        }
        let (retained, report) = if !unknown {
            match brain_min::tree_shake_with_report(source, &keep) {
                Ok(mut result) => {
                    if result.report.facts.len() > 16 {
                        result.report.omitted_facts += result.report.facts.len() - 16;
                        result.report.facts.truncate(16);
                        result.report.facts_truncated = true;
                    }
                    (
                        result.code,
                        serde_json::from_str::<Value>(&result.report.to_json()).ok(),
                    )
                }
                Err(_) => {
                    unknown = true;
                    (source.clone(), None)
                }
            }
        } else {
            (source.clone(), None)
        };
        let compact = format!(
            "{}{}",
            brain_pack::dependencies::notices(source)?,
            brain_pack::dependencies::compact(&retained)?
        );
        let mut names: Vec<_> = keep.into_iter().collect();
        names.sort();
        modules.push(json!({"file":file,"staticallyImportedExports":if unknown{Value::Null}else{json!(names)},"unknownUsage":unknown,"inputBytes":source.len(),"retainedBytes":compact.len(),"removedBytes":source.len().saturating_sub(compact.len()),"qualification":"actual module text/own-printer size; not final URL-rewritten page bytes or runtime measurements","provenCleanup":report}));
    }
    let summary: Value = serde_json::from_str(&resolver.summary().stringify())
        .map_err(|_| "invalid dependency metadata")?;
    let edge_meta:Vec<_>=edges.iter().take(1024).map(|(from,spec,to)|json!({"from":from,"specifier":public_spec(spec),"to":to,"confidence":"literal graph edge"})).collect();
    if edges.len() > 1024 {
        truncated = true;
    }
    let value = json!({"schemaVersion":1,"offline":true,"dependencySnapshot":summary,"inputRevisions":input_revisions,"edges":edge_meta,"modules":modules,"coverage":{"complete":!truncated&&skipped.is_empty(),"truncated":truncated,"skipped":skipped.into_iter().take(64).collect::<Vec<_>>()},"llmProposals":{"automaticNetwork":false,"automaticChanges":false,"next":"Supply one reviewed source file/revision/content to deps preview; only real checker diagnostics can establish preventive failures. SDK/public-property renames require separate proof and are not automatically applied."},"unknowns":["dynamic and namespace consumers retain all exports","sideEffects metadata is a declaration, not proof of effect freedom","CJS/TS/JSX, remote URLs, aliases without package imports maps and cyclic emitted chunk graphs need explicit supported adapters"]});
    if serde_json::to_vec(&value)
        .map_err(|_| "cannot serialize dependency analysis")?
        .len()
        > 2 * 1024 * 1024
    {
        return Err("dependency metadata exceeds 2 MiB; analyze one source file".into());
    }
    Ok(value)
}
fn public_spec(spec: &str) -> Value {
    if spec.len() <= 240
        && !spec.contains(['?', '#', '\\', ':'])
        && spec
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@/._-".contains(&b))
    {
        json!(spec)
    } else {
        json!({"redacted":true,"revision":crate::manifests::hash(spec.as_bytes())})
    }
}
fn preview(root: &Path, args: &Value) -> Result<Value, String> {
    if args
        .as_object()
        .unwrap()
        .keys()
        .any(|k| !["file", "expectedRevision", "content"].contains(&k.as_str()))
    {
        return Err("dependency proposal accepts only file, expectedRevision and content".into());
    }
    let file = args["file"]
        .as_str()
        .filter(|s| s.ends_with(".brain") || s.ends_with(".jsbrain"))
        .ok_or("proposals require one project .brain/.jsbrain source file")?;
    if brain_pack::dependencies::is_foreign(file) {
        return Err("LLM proposals must not rewrite installed SDK source".into());
    }
    let content = args["content"]
        .as_str()
        .filter(|s| s.len() <= 262144)
        .ok_or("proposal content must contain at most 256 KiB")?;
    if !brain_check::scan_secrets(content).is_empty() {
        return Err("proposal contains a detected secret; provide public source only".into());
    }
    let existing = crate::changes::read(root, file)?;
    if args
        .get("expectedRevision")
        .is_none_or(|r| r != &existing["revision"])
    {
        return Err("proposal requires the current exact source revision; reread the file before preparing a plan".into());
    }
    let (diags, _) = crate::compiler::check(content, Path::new(file), &root.join(file));
    let accepted = !diags.iter().any(|d| !d.is_warning() && !d.is_info());
    let diagnostics:Vec<_>=diags.iter().take(64).map(|d|json!({"file":file,"code":d.code,"line":d.line,"column":d.col,"severity":if d.is_warning(){"warning"}else if d.is_info(){"info"}else{"error"},"confidence":"observed checker diagnostic"})).collect();
    let operations = if accepted && existing["content"].as_str() != Some(content) {
        vec![json!({"path":file,"expectedRevision":existing["revision"],"content":content})]
    } else {
        vec![]
    };
    Ok(
        json!({"schemaVersion":1,"dryRun":true,"accepted":accepted,"operations":operations,"diagnostics":diagnostics,"diagnosticsTruncated":diags.len()>64,"scope":"one project source file; CAS apply through the atomic change planner","executed":false,"llmNetwork":false,"qualification":"Static checking is not semantic equivalence, performance improvement or runtime proof; measure actual final emitted bytes and behavior before accepting the optimization."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct App(std::path::PathBuf);
    impl App {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "brain-deps-cli-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(p.join("views")).unwrap();
            Self(p)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn analysis_reports_actual_bytes_and_never_returns_module_bodies() {
        let app = App::new();
        std::fs::create_dir_all(app.0.join("node_modules/demo")).unwrap();
        std::fs::write(app.0.join("node_modules/demo/package.json"),r#"{"name":"demo","version":"1.0.0","type":"module","exports":"./index.mjs","sideEffects":false}"#).unwrap();
        std::fs::write(app.0.join("node_modules/demo/index.mjs"),"export function used(){return 1} export function omitted(){return 'private-literal-body'} globalThis.sdkEffect = true;").unwrap();
        std::fs::write(
            app.0.join("views/home.brain"),
            "<script>import { used } from 'demo'</script><p>{used()}</p>",
        )
        .unwrap();
        let report = query(&app.0, "analyze", &json!({})).unwrap();
        assert_eq!(report["modules"].as_array().unwrap().len(), 1);
        assert!(report["modules"][0]["removedBytes"].as_u64().unwrap() > 0);
        assert!(!report.to_string().contains("private-literal-body"));
        assert_eq!(report["coverage"]["complete"], true);
    }
    #[test]
    fn proposals_require_revision_check_and_never_edit_sdk_or_execute() {
        let app = App::new();
        std::fs::create_dir(app.0.join("comp")).unwrap();
        std::fs::write(app.0.join("comp/card.brain"), "<p>Before</p>").unwrap();
        let rev = crate::changes::read(&app.0, "comp/card.brain").unwrap()["revision"].clone();
        let preview = query(
            &app.0,
            "preview",
            &json!({"file":"comp/card.brain","expectedRevision":rev,"content":"<p>After</p>"}),
        )
        .unwrap();
        assert_eq!(preview["accepted"], true);
        assert_eq!(
            std::fs::read_to_string(app.0.join("comp/card.brain")).unwrap(),
            "<p>Before</p>"
        );
        assert!(query(&app.0,"preview",&json!({"file":"comp/card.brain","expectedRevision":"stale","content":"<p>After</p>"})).is_err());
        assert!(query(&app.0,"preview",&json!({"file":"node_modules/demo/index.jsbrain","expectedRevision":null,"content":"export default 1"})).is_err());
    }
}
