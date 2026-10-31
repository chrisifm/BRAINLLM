//! Persistent bounded agent feedback. No builds, network, app execution or source-value logging.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
const PRODUCER: &str = "brain-project-quality";
const MAX_BYTES: usize = 1024 * 1024;
const MAX_ITEMS: usize = 128;
static TEMP: AtomicU64 = AtomicU64::new(0);
type Result<T> = std::result::Result<T, String>;
fn bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("cannot inspect quality metadata".into()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_BYTES as u64 {
        return Err("quality metadata is unsafe or exceeds 1 MiB".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| "cannot read quality metadata")?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err("quality metadata is not regular".into());
    }
    let mut out = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| "cannot read quality metadata")?;
    if out.len() > MAX_BYTES {
        return Err("quality metadata exceeds its read budget".into());
    }
    Ok(Some(out))
}
fn revision(path: &Path) -> Result<Value> {
    Ok(bytes(path)?
        .map(|b| json!(crate::manifests::hash(&b)))
        .unwrap_or(Value::Null))
}
fn decode(path: &Path) -> Result<Option<Value>> {
    bytes(path)?
        .map(|b| serde_json::from_slice(&b).map_err(|_| "quality metadata JSON is invalid".into()))
        .transpose()
}
fn encode(value: &Value) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "cannot encode quality metadata")?;
    if bytes.len() > MAX_BYTES {
        return Err("quality output exceeds 1 MiB".into());
    }
    Ok(bytes)
}
fn owned(value: &Value) -> Result<()> {
    if value["producer"] != PRODUCER || value["schemaVersion"] != 1 {
        return Err(
            "quality metadata has an unknown producer/schema; preserve it for review".into(),
        );
    }
    Ok(())
}
fn safe_text(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(char::is_control) && brain_check::scan_secrets(s).is_empty()
}
fn file(root: &Path, value: &Value) -> Option<String> {
    let path = value.as_str()?;
    if !safe_text(path, 240) {
        return None;
    }
    crate::changes::source_path(root, path).ok()?;
    Some(path.into())
}
fn valid_hash(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn cas(path: &Path, content: &[u8], expected: &Value) -> Result<()> {
    if content.len() > MAX_BYTES || revision(path)? != *expected {
        return Err("quality file changed; preserve it and refresh from a new snapshot".into());
    }
    let parent = path.parent().ok_or("quality parent is missing")?;
    let temp = parent.join(format!(
        ".quality-{}-{}.tmp",
        std::process::id(),
        TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut output = options
        .open(&temp)
        .map_err(|_| "cannot create owned quality temporary file")?;
    let result = (|| {
        output
            .write_all(content)
            .and_then(|_| output.sync_all())
            .map_err(|_| "cannot persist quality temporary file")?;
        if revision(path)? != *expected {
            return Err("quality file changed during write; preserve it and retry".into());
        }
        if expected.is_null() {
            fs::hard_link(&temp, path)
                .map_err(|_| "quality destination was created concurrently")?;
        } else {
            fs::rename(&temp, path).map_err(|_| "cannot atomically replace quality output")?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}
fn recover(dir: &Path) -> Result<()> {
    let journal = dir.join("pending.json");
    let Some(plan) = decode(&journal)? else {
        return Ok(());
    };
    owned(&plan)?;
    let expected_journal = revision(&journal)?;
    let operations = plan["operations"]
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or("invalid quality recovery plan")?;
    let mut seen = std::collections::BTreeSet::new();
    for operation in operations {
        let name = operation["name"]
            .as_str()
            .filter(|s| ["IMPROVEMENTS.md", "panel.html", "improvements.json"].contains(s))
            .ok_or("invalid quality recovery destination")?;
        if !seen.insert(name) {
            return Err("duplicate quality recovery destination".into());
        }
        let text = operation["content"]
            .as_str()
            .filter(|s| s.len() <= MAX_BYTES)
            .ok_or("invalid quality recovery content")?;
        if operation["desired"] != crate::manifests::hash(text.as_bytes()) {
            return Err("quality recovery checksum differs".into());
        }
        let path = dir.join(name);
        if revision(&path)? != operation["desired"] {
            cas(&path, text.as_bytes(), &operation["expected"])?;
        }
    }
    if revision(&journal)? != expected_journal {
        return Err("quality recovery journal changed; preserve it".into());
    }
    fs::remove_file(journal).map_err(|_| "cannot finish quality recovery")?;
    Ok(())
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn render(snapshot: &Value) -> (String, String) {
    let mut md = String::from(
        "<!-- brain-project-quality schemaVersion=1 -->\n# Project improvements\n\nGenerated static evidence, not a complete audit or a 100% score. JSON is the canonical agent interface.\n\n",
    );
    let mut rows = String::new();
    for item in snapshot["items"].as_array().into_iter().flatten() {
        let title = item["title"].as_str().unwrap_or("Review evidence");
        let path = item["location"]["file"].as_str().unwrap_or("unknown");
        let line = item["location"]["line"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unknown".into());
        let fix = item["fix"].as_str().unwrap_or("");
        let priority = item["priority"].as_str().unwrap_or("P3");
        md.push_str(&format!(
            "- {priority}: {} ({}:{line}; {})\n  Fix: {}\n",
            title.replace(['\n', '\r'], " "),
            path.replace(['\n', '\r'], " "),
            item["evidence"].as_str().unwrap_or("unknown"),
            fix.replace(['\n', '\r'], " ")
        ));
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}:{}</td><td>{}</td><td>{}</td></tr>",
            escape(priority),
            escape(title),
            escape(path),
            escape(&line),
            escape(item["evidence"].as_str().unwrap_or("unknown")),
            escape(fix)
        ));
    }
    md.push_str("\nCoverage and omitted findings are explicit in improvements.json. Runtime speed, heap usage, accessibility behavior, network bytes and all-bug completeness are unmeasured unless a separate verified build/test supplies evidence.\n");
    let panel = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; connect-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'none'\"><meta name=\"viewport\" content=\"width=device-width\"><title>Project improvements</title><style>body{{font:15px system-ui;margin:32px;background:#f5f5f7;color:#202124}}main{{background:#ffffffd9;border:1px solid #ddd;border-radius:20px;padding:24px;overflow:auto}}table{{border-collapse:collapse;width:100%}}td,th{{padding:12px;text-align:left;border-bottom:1px solid #ddd}}p{{color:#555}}</style><main><h1>Project improvements</h1><p>Static evidence and explicit unknowns. No complete-audit score. JSON is the primary agent interface.</p><table><thead><tr><th>Priority</th><th>Finding</th><th>Location</th><th>Evidence</th><th>Fix</th></tr></thead><tbody>{rows}</tbody></table><p>Coverage and omitted findings: improvements.json. This panel has no scripts, network, application data or external assets.</p></main></html>"
    );
    (md, panel)
}
fn push(
    items: &mut Vec<Value>,
    total: &mut usize,
    code: &str,
    title: &str,
    location: Value,
    evidence: &str,
    severity: &str,
    fix: &str,
    acceptance: &str,
) {
    *total += 1;
    let id = crate::manifests::hash(
        format!(
            "{code}|{}|{}|{}",
            location["file"], location["line"], location["column"]
        )
        .as_bytes(),
    );
    if let Some(existing) = items.iter_mut().find(|i| i["id"] == id) {
        existing["occurrences"] = json!(existing["occurrences"].as_u64().unwrap_or(1) + 1);
        return;
    }
    let item = json!({"id":id,"code":code,"title":title,"location":location,"evidence":evidence,"severity":severity,"priority":if severity=="error"{"P1"}else if severity=="warning"{"P2"}else{"P3"},"fix":fix,"acceptance":acceptance,"occurrences":1,"status":if evidence=="unmeasured"{"needs-validation"}else{"open"}});
    if items.len() < MAX_ITEMS {
        items.push(item);
    } else if let Some((index, worst)) = items
        .iter()
        .enumerate()
        .max_by_key(|(_, i)| i["priority"].as_str().unwrap_or("P3"))
    {
        if item["priority"].as_str() < worst["priority"].as_str() {
            items[index] = item;
        }
    }
}
/// Persist selected verified build facts only; never forwards warnings, raw reports, source bodies or values.
pub fn record_build(root: &Path, build: &Value) -> Result<Value> {
    if build["schemaVersion"] != 1 {
        return Err("build feedback requires schemaVersion 1 from a verified build".into());
    }
    let dir = crate::manifests::private_dir(root, "quality")?;
    let _lock = crate::manifests::lock(&dir)?;
    let path = dir.join("build.json");
    if let Some(old) = decode(&path)? {
        owned(&old)?;
    }
    let mut metrics = BTreeMap::<String, u64>::new();
    let mut facts = Vec::new();
    let mut count = 0usize;
    for row in build["report"].as_array().into_iter().flatten().take(1000) {
        for key in ["jsRaw", "jsMin", "jsGz", "css", "cssGz", "html", "htmlGz"] {
            if let Some(n) = row[key].as_u64() {
                let total = metrics.entry(format!("perPageSum.{key}")).or_default();
                *total = total.saturating_add(n);
            }
        }
    }
    let mut stages = BTreeMap::<String, u64>::new();
    let mut truncated = false;
    let mut hashes = BTreeMap::<String, Option<(String, usize)>>::new();
    let mut read_bytes = 0usize;
    let mut visited = 0usize;
    for report in build["optimizations"]
        .as_array()
        .into_iter()
        .flatten()
        .take(256)
    {
        let Some(source) = file(root, &report["sourceFile"]) else {
            continue;
        };
        let stage = report["stage"]
            .as_str()
            .filter(|s| ["module-minify", "runtime-tree-shake", "generated-css"].contains(s))
            .unwrap_or("unknown");
        for key in [
            "removedBindings",
            "removedImportBindings",
            "retainedEffects",
            "omittedFacts",
        ] {
            if let Some(n) = report[key].as_u64() {
                let total = metrics.entry(key.into()).or_default();
                *total = total.saturating_add(n);
            }
        }
        *stages.entry(stage.into()).or_default() += 1;
        truncated |= report["factsTruncated"] == true;
        for fact in report["facts"].as_array().into_iter().flatten().take(4096) {
            let action = fact["action"].as_str().filter(|s| {
                [
                    "removed",
                    "initializer-retained",
                    "side-effect-import-retained",
                    "retained",
                ]
                .contains(s)
            });
            let Some(action) = action else { continue };
            count += 1;
            visited += 1;
            if visited > 8192 {
                truncated = true;
                break;
            }
            if facts.len() >= MAX_ITEMS {
                continue;
            }
            let original = if valid_hash(&report["originalSourceHash"]) {
                report["originalSourceHash"].clone()
            } else {
                Value::Null
            };
            let current = hashes.entry(source.clone()).or_insert_with(|| {
                if read_bytes >= 8 * 1024 * 1024 {
                    return None;
                }
                crate::map::read_source_limit(
                    root,
                    &source,
                    262144.min(8 * 1024 * 1024 - read_bytes),
                )
                .ok()
                .map(|s| {
                    read_bytes += s.len();
                    (
                        crate::manifests::hash(s.as_bytes()),
                        s.lines().count().max(1),
                    )
                })
            });
            let mapping = original.as_str() == current.as_ref().map(|(hash, _)| hash.as_str())
                && original.is_string();
            let line = if mapping {
                fact["originalLine"].as_u64().filter(|n| {
                    *n > 0
                        && *n
                            <= current
                                .as_ref()
                                .map(|(_, lines)| *lines as u64)
                                .unwrap_or(0)
                })
            } else {
                None
            };
            facts.push(json!({"sourceFile":source,"sourceRevision":original,"line":line,"column":if line.is_some(){fact["originalColumn"].as_u64().filter(|n|*n>0&&*n<=262144).map(|n|json!(n)).unwrap_or(Value::Null)}else{Value::Null},"generatedLine":fact["line"].as_u64(),"precision":if line.is_some(){"unique-source-declaration"}else{"generated-or-unknown"},"stage":stage,"action":action,"originalLocationVerified":line.is_some()}));
        }
    }
    let signature = build["build"]
        .as_str()
        .filter(|s| s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .map(str::to_string);
    let value = json!({"schemaVersion":1,"producer":PRODUCER,"kind":"verified-build-summary","buildSignature":signature,"metrics":metrics,"stages":stages,"facts":facts,"totalFacts":count,"omittedFacts":count.saturating_sub(MAX_ITEMS),"factsTruncated":truncated||count>MAX_ITEMS,"omittedReports":build["optimizations"].as_array().map(|a|a.len().saturating_sub(256)).unwrap_or(0),"qualification":"Reported emitted-byte totals sum pages and may repeat shared assets; not measured network, CPU or heap. Original source locations are retained only when their supplied revision matches a guarded current source read."});
    cas(&path, &encode(&value)?, &revision(&path)?)?;
    Ok(
        json!({"stored":true,"path":".brain/quality/build.json","facts":facts.len(),"omittedFacts":count.saturating_sub(MAX_ITEMS)}),
    )
}
/// Refresh local evidence without compiling/building, invoking app handlers or contacting services.
pub fn refresh(root: &Path) -> Result<Value> {
    let dir = crate::manifests::private_dir(root, "quality")?;
    let _lock = crate::manifests::lock(&dir)?;
    recover(&dir)?;
    let path = dir.join("improvements.json");
    let previous = decode(&path)?;
    if let Some(old) = &previous {
        owned(old)?;
    }
    let names = ["IMPROVEMENTS.md", "panel.html", "improvements.json"];
    let expected: Vec<Value> = names
        .iter()
        .map(|name| revision(&dir.join(name)))
        .collect::<Result<_>>()?;
    if previous.is_none() && expected[..2].iter().any(|v| !v.is_null()) {
        return Err("unowned quality sidecar exists; preserve it before refresh".into());
    }
    if let Some(old) = &previous {
        for (i, name) in names[..2].iter().enumerate() {
            if old["artifacts"][*name] != expected[i] {
                return Err(
                    "quality sidecar was edited; preserve edits before regeneration".into(),
                );
            }
        }
    }
    let started = Instant::now();
    let (index, documents) = crate::manifests::documents(root)?;
    let mut items = Vec::new();
    let mut total = 0usize;
    let mut checked = 0usize;
    let mut source_bytes = 0usize;
    let mut skipped = 0usize;
    // Read only bounded root foundation data; do not invoke arbitrary cross-file resolution here.
    let design_system=bytes(&root.join("design.brain.json"))?.is_some();
    let design_tokens=bytes(&root.join("tokens.cssbrain"))?.and_then(|raw|String::from_utf8(raw).ok()).and_then(|source|brain_css::parse_str(&source,"tokens.cssbrain").ok()).map(|sheet|brain_css::custom_props(&sheet));
    for document in &documents {
        if checked >= 128 || source_bytes >= 8 * 1024 * 1024 || started.elapsed().as_secs() >= 30 {
            skipped += 1;
            continue;
        }
        let Some(name) = file(root, &document["file"]) else {
            skipped += 1;
            continue;
        };
        let source = match crate::map::read_source_limit(root, &name, 262144) {
            Ok(s) => s,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        source_bytes += source.len();
        let revision = crate::manifests::hash(source.as_bytes());
        if document["revision"] != revision {
            return Err("source changed during quality preparation; refresh again".into());
        }
        let location = json!({"file":name,"line":Value::Null,"column":Value::Null,"sourceRevision":revision,"precision":"file"});
        // Conservative delimiter/markup budget before the native parser. Skipping is an explicit coverage gap.
        let mut depth = 0usize;
        let mut too_deep = false;
        for c in source.chars() {
            if matches!(c, '(' | '[' | '{') {
                depth += 1;
                too_deep |= depth > 128;
            } else if matches!(c, ')' | ']' | '}') {
                depth = depth.saturating_sub(1);
            }
        }
        if too_deep
            || source.matches('<').count() > 4096
            || !brain_check::scan_secrets(&source).is_empty()
        {
            skipped += 1;
            continue;
        }
        match brain_parse::parse(&source, Path::new(&name)) {
            Ok(component) => {
                let mut context = brain_check::Ctx {design_system,tokens:design_tokens.clone(),..brain_check::Ctx::default()};
                for stmt in &component.script {
                    if let brain_parse::ast::Stmt::Import(import) = stmt {
                        context
                            .imports
                            .insert(import.source.clone(), brain_check::Imported::Other);
                    }
                }
                for diagnostic in brain_check::check(&component, &context) {
                    // Only rules with local evidence; no invented cross-file import/type/config resolution.
                    if !(diagnostic.code.starts_with("B09")
                        || diagnostic.code.starts_with("B10")
                        || matches!(diagnostic.code, "B0201" | "B2002" | "B2004" | "B2201"))
                    {
                        continue;
                    }
                    if let Some((_, title, why, _)) = brain_check::explain(diagnostic.code) {
                        let mut at = location.clone();
                        at["line"] = json!(diagnostic.line);
                        at["column"] = json!(diagnostic.col);
                        at["precision"] = json!("native-diagnostic");
                        push(
                            &mut items,
                            &mut total,
                            diagnostic.code,
                            title,
                            at,
                            "static",
                            diagnostic.level.as_str(),
                            why,
                            "Run brain check with the complete project context, apply the source fix, then refresh this file revision.",
                        );
                    }
                }
            }
            Err(diagnostics) => {
                for diagnostic in diagnostics.into_iter().take(32) {
                    if let Some((_, title, why, _)) = brain_check::explain(diagnostic.code) {
                        let mut at = location.clone();
                        at["line"] = json!(diagnostic.line);
                        at["column"] = json!(diagnostic.col);
                        push(
                            &mut items,
                            &mut total,
                            diagnostic.code,
                            title,
                            at,
                            "proven",
                            "error",
                            why,
                            "The native parser accepts the updated source, then refresh records its new revision.",
                        );
                    }
                }
            }
        }
        checked += 1;
        if !document["states"]["unknown"]
            .as_array()
            .is_none_or(|v| v.is_empty())
        {
            push(
                &mut items,
                &mut total,
                "QSTATE",
                "Interaction states need validation",
                location,
                "unmeasured",
                "info",
                "Verify relevant permission, loading, error, empty, success and disabled states with rendered interactions; structural absence does not prove a defect.",
                "Attach scenario evidence for each relevant state; mark inapplicable states explicitly in project history.",
            );
        }
    }
    let design = crate::design::query(root, None).ok();
    let history = crate::history::query(root, "status", &json!({})).ok();
    if design.as_ref().is_some_and(|d| {
        d["check"]["warnings"]
            .as_array()
            .is_some_and(|w| !w.is_empty())
    }) {
        push(
            &mut items,
            &mut total,
            "QDESIGN",
            "Design baseline needs review",
            json!({"file":Value::Null,"line":Value::Null,"column":Value::Null,"sourceRevision":Value::Null,"precision":"unknown"}),
            "static",
            "warning",
            "Inspect brain design check and preview non-destructive baseline adoption; retain project tokens and branding.",
            "Design check records the reviewed current token revision; rendered contrast and layout are separately verified.",
        );
    }
    let build = decode(&dir.join("build.json"))?;
    if let Some(build) = &build {
        owned(build)?;
        if build["kind"] != "verified-build-summary"
            || !build["facts"]
                .as_array()
                .is_some_and(|a| a.len() <= MAX_ITEMS)
            || build.as_object().is_none_or(|o| {
                o.keys().any(|k| {
                    ![
                        "schemaVersion",
                        "producer",
                        "kind",
                        "buildSignature",
                        "metrics",
                        "stages",
                        "facts",
                        "totalFacts",
                        "omittedFacts",
                        "factsTruncated",
                        "omittedReports",
                        "qualification",
                    ]
                    .contains(&k.as_str())
                })
            })
            || build["metrics"].as_object().is_none_or(|o| {
                o.len() > 32 || o.iter().any(|(k, v)| !safe_text(k, 64) || !v.is_u64())
            })
            || !brain_check::scan_secrets(&build.to_string()).is_empty()
        {
            return Err(
                "persisted build summary is unrecognized or unsafe; preserve it for review".into(),
            );
        }
        for fact in build["facts"]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_ITEMS)
        {
            if ![
                "removed",
                "initializer-retained",
                "side-effect-import-retained",
            ]
            .contains(&fact["action"].as_str().unwrap_or(""))
            {
                continue;
            }
            let current = fact["sourceFile"]
                .as_str()
                .and_then(|name| index["sourceRevisions"].get(name))
                .and_then(Value::as_str);
            let current_match =
                fact["sourceRevision"].as_str() == current && valid_hash(&fact["sourceRevision"]);
            let mut at = json!({"file":fact["sourceFile"],"line":fact["line"],"column":fact["column"],"sourceRevision":fact["sourceRevision"],"precision":fact["precision"]});
            if !current_match {
                at["line"] = Value::Null;
                at["column"] = Value::Null;
                at["precision"] = json!("stale-build-location");
            }
            push(
                &mut items,
                &mut total,
                "QCLEANUP",
                "Production cleanup should be applied to source",
                at,
                if current_match {
                    "proven"
                } else {
                    "unmeasured"
                },
                "warning",
                "Inspect the build optimization fact and remove the unused binding in source while preserving initializers and module side effects; generated-only locations require manual source mapping.",
                "Refresh the verified build report and confirm the same cleanup fact no longer applies to the new source revision.",
            );
        }
    }
    let latest = crate::manifests::query(root, None)?;
    if latest["inputFingerprint"] != index["inputFingerprint"] {
        return Err("project changed while preparing quality feedback; retry".into());
    }
    items.sort_by(|a, b| {
        a["priority"]
            .as_str()
            .cmp(&b["priority"].as_str())
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    let represented: usize = items
        .iter()
        .map(|i| i["occurrences"].as_u64().unwrap_or(1) as usize)
        .sum();
    let mut snapshot = json!({"schemaVersion":1,"producer":PRODUCER,"frameworkVersion":env!("CARGO_PKG_VERSION"),"inputFingerprint":index["inputFingerprint"],"sourceRevisions":index["sourceRevisions"],"coverage":{"manifest":index["coverage"],"locallyCheckedFiles":checked,"locallyReadBytes":source_bytes,"skippedLocalFiles":skipped,"completeAudit":false,"crossFileSemanticContext":"not-loaded","runtimeMeasurements":"not-run","allBugs":"unknown"},"score":Value::Null,"items":items,"totalFindings":total,"omittedFindings":total.saturating_sub(represented),"truncated":total>represented||skipped>0,"design":{"available":design.is_some(),"baselinePresent":design.as_ref().map(|d|d["baselinePresent"].clone())},"history":{"available":history.is_some(),"counts":history.as_ref().map(|h|h["counts"].clone())},"build":build,"artifacts":{}});
    let (markdown, panel) = render(&snapshot);
    snapshot["artifacts"] = json!({"IMPROVEMENTS.md":crate::manifests::hash(markdown.as_bytes()),"panel.html":crate::manifests::hash(panel.as_bytes())});
    let json = String::from_utf8(encode(&snapshot)?).map_err(|_| "quality JSON encoding failed")?;
    let contents = [markdown, panel, json];
    let operations:Vec<Value>=names.iter().enumerate().map(|(i,name)|json!({"name":name,"content":contents[i],"expected":expected[i],"desired":crate::manifests::hash(contents[i].as_bytes())})).collect();
    let journal = json!({"schemaVersion":1,"producer":PRODUCER,"operations":operations});
    cas(&dir.join("pending.json"), &encode(&journal)?, &Value::Null)?;
    recover(&dir)?;
    query(root, &json!({"limit":25}))
}
/// JSON is the primary interface; pagination never claims omitted findings were inspected.
pub fn query(root: &Path, options: &Value) -> Result<Value> {
    if !options.is_object() {
        return Err("quality options must be an object".into());
    }
    let limit = options
        .get("limit")
        .map_or(Some(25), Value::as_u64)
        .filter(|n| *n > 0 && *n <= 100)
        .ok_or("quality limit must be 1..100")? as usize;
    let cursor = options
        .get("cursor")
        .map_or(Some(0), Value::as_u64)
        .filter(|n| *n <= MAX_ITEMS as u64)
        .ok_or("invalid quality cursor")? as usize;
    let dir = crate::manifests::private_dir(root, "quality")?;
    let Some(mut snapshot) = decode(&dir.join("improvements.json"))? else {
        return Ok(
            json!({"schemaVersion":1,"available":false,"next":"brain quality refresh","score":Value::Null,"coverage":{"completeAudit":false}}),
        );
    };
    owned(&snapshot)?;
    let revision = revision(&dir.join("improvements.json"))?;
    let rows = snapshot["items"]
        .as_array()
        .filter(|a| a.len() <= MAX_ITEMS)
        .ok_or("quality snapshot item budget is invalid")?;
    let selected: Vec<Value> = rows.iter().skip(cursor).take(limit).cloned().collect();
    let next = if cursor + selected.len() < rows.len() {
        Some(cursor + selected.len())
    } else {
        None
    };
    let mut stale = 0usize;
    let mut unknown = 0usize;
    let mut read_bytes = 0usize;
    for (file, hash) in snapshot["sourceRevisions"]
        .as_object()
        .into_iter()
        .flatten()
        .take(500)
    {
        if read_bytes >= 8 * 1024 * 1024 {
            unknown += 1;
            continue;
        }
        match crate::map::read_source_limit(root, file, 262144.min(8 * 1024 * 1024 - read_bytes)) {
            Ok(source) => {
                read_bytes += source.len();
                if json!(crate::manifests::hash(source.as_bytes())) != *hash {
                    stale += 1;
                }
            }
            Err(_) => unknown += 1,
        }
    }
    snapshot["items"] = json!(selected);
    snapshot["revision"] = revision;
    snapshot["nextCursor"] = json!(next);
    // A snapshot written by another compiler version reflects that version's rules; say so instead of
    // presenting it as current (`frameworkVersion` keeps the version that produced the evidence).
    let installed = env!("CARGO_PKG_VERSION");
    let framework_stale = snapshot["frameworkVersion"].as_str() != Some(installed);
    snapshot["freshness"] = json!({"staleSourceFiles":stale,"unknownSourceFiles":unknown,"pendingRecovery":dir.join("pending.json").try_exists().unwrap_or(true),"frameworkStale":framework_stale,"installedFrameworkVersion":installed,"checked":"recorded source revisions only; newly added files require refresh"});
    if framework_stale || stale > 0 {
        snapshot["next"] = json!("brain quality refresh");
    }
    snapshot["paths"] = json!({"json":".brain/quality/improvements.json","markdown":".brain/quality/IMPROVEMENTS.md","panel":".brain/quality/panel.html","private":true});
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct App(PathBuf);
    impl App {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "brain-quality-{}-{}",
                std::process::id(),
                TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("comp")).unwrap();
            fs::write(root.join("brain.json"), "{\"name\":\"quality-fixture\"}").unwrap();
            fs::write(
                root.join("app.brain"),
                "<html lang=\"en\"><head><title>Fixture</title></head><body><slot /></body></html>",
            )
            .unwrap();
            fs::write(
                root.join("comp/example.brain"),
                "<script>const example = document.title</script><p>{example}</p>",
            )
            .unwrap();
            Self(root)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn snapshots_are_real_persistent_paginated_and_do_not_copy_source_values() {
        let app = App::new();
        fs::write(
            app.0.join("comp/text.brain"),
            "<p>private-source-value-not-feedback</p>",
        )
        .unwrap();
        let result = refresh(&app.0).unwrap();
        assert_eq!(result["schemaVersion"], 1);
        assert!(result["score"].is_null());
        assert_eq!(result["coverage"]["completeAudit"], false);
        let data = fs::read_to_string(app.0.join(".brain/quality/improvements.json")).unwrap();
        assert!(!data.contains("private-source-value-not-feedback"));
        let all: Value = serde_json::from_str(&data).unwrap();
        assert!(
            all["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["code"] == "B0201" && i["location"]["line"] == 1)
        );
        let one = query(&app.0, &json!({"limit":1})).unwrap();
        assert_eq!(one["items"].as_array().unwrap().len(), 1);
        assert!(one["nextCursor"].is_number());
        let html = fs::read_to_string(app.0.join(".brain/quality/panel.html")).unwrap();
        assert!(html.contains("default-src 'none'"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("private-source-value-not-feedback"));
        assert!(query(&app.0, &json!({"limit":0})).is_err());
        assert!(query(&app.0, &json!({"cursor":10000})).is_err());
    }
    #[test]
    fn stale_sources_and_unrelated_editor_changes_are_explicit_and_preserved() {
        let app = App::new();
        refresh(&app.0).unwrap();
        fs::write(app.0.join("comp/example.brain"), "<p>Updated</p>").unwrap();
        let result = query(&app.0, &json!({})).unwrap();
        assert_eq!(result["freshness"]["staleSourceFiles"], 1);
        assert_eq!(result["freshness"]["frameworkStale"], false);
        assert_eq!(result["next"], "brain quality refresh");
        let markdown = app.0.join(".brain/quality/IMPROVEMENTS.md");
        fs::write(&markdown, "editor-owned-note").unwrap();
        assert!(refresh(&app.0).unwrap_err().contains("edited"));
        assert_eq!(fs::read_to_string(markdown).unwrap(), "editor-owned-note");
    }
    #[test]
    fn snapshot_from_another_framework_version_is_reported_stale() {
        let app = App::new();
        refresh(&app.0).unwrap();
        let path = app.0.join(".brain/quality/improvements.json");
        let mut old: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        old["frameworkVersion"] = json!("0.0.1");
        fs::write(&path, serde_json::to_string(&old).unwrap()).unwrap();
        let result = query(&app.0, &json!({})).unwrap();
        assert_eq!(result["freshness"]["frameworkStale"], true);
        assert_eq!(result["freshness"]["installedFrameworkVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(result["next"], "brain quality refresh");
        refresh(&app.0).unwrap();
        let fresh = query(&app.0, &json!({})).unwrap();
        assert_eq!(fresh["frameworkVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(fresh["freshness"]["frameworkStale"], false);
    }
    #[test]
    fn build_feedback_keeps_selected_counts_locations_and_no_arbitrary_body_or_warning() {
        let app = App::new();
        let source = fs::read(app.0.join("comp/example.brain")).unwrap();
        let hash = crate::manifests::hash(&source);
        record_build(&app.0,&json!({"schemaVersion":1,"build":"abc123","warnings":["private-warning-value"],"payload":"private-payload-value","report":[{"jsMin":20,"css":5}],"optimizations":[{"sourceFile":"comp/example.brain","originalSourceHash":hash,"stage":"module-minify","removedBindings":1,"facts":[{"action":"removed","originalLine":1,"originalColumn":1,"line":17,"body":"private-literal-value"}]}]})).unwrap();
        let stored = fs::read_to_string(app.0.join(".brain/quality/build.json")).unwrap();
        for value in [
            "private-warning-value",
            "private-payload-value",
            "private-literal-value",
        ] {
            assert!(!stored.contains(value));
        }
        let result = refresh(&app.0).unwrap();
        assert_eq!(result["build"]["metrics"]["perPageSum.jsMin"], 20);
        assert!(
            result["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["code"] == "QCLEANUP" && i["location"]["line"] == 1)
        );
        fs::write(app.0.join("comp/example.brain"), "<p>Changed</p>").unwrap();
        let result = refresh(&app.0).unwrap();
        assert!(
            result["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["code"] == "QCLEANUP"
                    && i["evidence"] == "unmeasured"
                    && i["location"]["line"].is_null())
        );
    }
    #[test]
    fn recovery_finishes_partial_owned_outputs_but_rejects_a_conflicting_editor() {
        let app = App::new();
        refresh(&app.0).unwrap();
        let dir = app.0.join(".brain/quality");
        let names = ["IMPROVEMENTS.md", "panel.html", "improvements.json"];
        let contents = [
            "<!-- brain-project-quality -->\nrecovered".to_string(),
            "<!doctype html><title>Recovered</title>".to_string(),
            fs::read_to_string(dir.join("improvements.json")).unwrap(),
        ];
        let operations:Vec<Value>=names.iter().enumerate().map(|(i,name)|json!({"name":name,"expected":revision(&dir.join(name)).unwrap(),"desired":crate::manifests::hash(contents[i].as_bytes()),"content":contents[i]})).collect();
        fs::write(
            dir.join("pending.json"),
            encode(&json!({"schemaVersion":1,"producer":PRODUCER,"operations":operations}))
                .unwrap(),
        )
        .unwrap();
        cas(
            &dir.join(names[0]),
            contents[0].as_bytes(),
            &operations[0]["expected"],
        )
        .unwrap();
        recover(&dir).unwrap();
        assert!(!dir.join("pending.json").exists());
        assert_eq!(
            fs::read_to_string(dir.join("panel.html")).unwrap(),
            contents[1]
        );
        fs::write(
            dir.join("pending.json"),
            encode(&json!({"schemaVersion":1,"producer":PRODUCER,"operations":operations}))
                .unwrap(),
        )
        .unwrap();
        fs::write(dir.join("panel.html"), "editor-change").unwrap();
        assert!(recover(&dir).is_err());
        assert_eq!(
            fs::read_to_string(dir.join("panel.html")).unwrap(),
            "editor-change"
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_private_namespace_and_files_are_refused() {
        use std::os::unix::fs::symlink;
        let app = App::new();
        let outside = App::new();
        fs::create_dir(app.0.join(".brain")).unwrap();
        symlink(&outside.0, app.0.join(".brain/quality")).unwrap();
        assert!(refresh(&app.0).is_err());
        assert!(!outside.0.join("improvements.json").exists());
    }
}
