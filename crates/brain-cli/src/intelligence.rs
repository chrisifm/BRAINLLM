//! Bounded, evidence-based static context. Edges are declarations, never runtime trace proof.
use brain_parse::ast::{Expr, Stmt};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path, PathBuf};

const MAX_FILES: usize = 2048;
const MAX_BYTES: u64 = 262_144;
const MAX_TOTAL: u64 = 16_777_216;
const MAX_RECORDS: usize = 16_384;

#[derive(Default)]
struct Graph {
    nodes: BTreeMap<String, Value>,
    edges: Vec<Value>,
    unknown: Vec<Value>,
    diagnostics: Vec<Value>,
    truncated: bool,
}
impl Graph {
    fn node(&mut self, id: &str, kind: &str, file: Option<&str>) {
        if self.nodes.len() >= MAX_RECORDS {
            self.truncated = true;
            return;
        }
        self.nodes
            .entry(id.into())
            .or_insert_with(|| json!({"id":id,"kind":kind,"file":file}));
    }
    fn edge(
        &mut self,
        from: &str,
        to: &str,
        relation: &str,
        file: &str,
        span: Option<brain_parse::Span>,
    ) {
        if self.edges.len() >= MAX_RECORDS {
            self.truncated = true;
            return;
        }
        self.edges.push(json!({"from":from,"to":to,"relation":relation,"evidence":{"kind":"static-declaration","file":file,"line":span.map(|s|s.line),"column":span.map(|s|s.col)},"runtimeProof":false}));
    }
    fn unresolved(&mut self, file: &str, reason: &str) {
        if self.unknown.len() < MAX_FILES {
            self.unknown
                .push(json!({"file":file,"status":"unknown","reason":reason}));
        } else {
            self.truncated = true;
        }
    }
}
fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}
fn walk(root: &Path, dir: &Path, depth: usize, files: &mut Vec<PathBuf>, truncated: &mut bool) {
    if depth > 32 || files.len() >= MAX_FILES {
        *truncated = true;
        return;
    }
    if dir != root && crate::scaffold::is_app_dir(dir) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if files.len() >= MAX_FILES {
            *truncated = true;
            break;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            || matches!(name.as_str(), "target" | "node_modules" | "dist" | "vendor")
        {
            continue;
        }
        let Ok(ty) = e.file_type() else {
            continue;
        };
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            walk(root, &e.path(), depth + 1, files, truncated);
        } else if name.ends_with(".brain") || name.ends_with(".jsbrain") {
            files.push(e.path());
        }
    }
}
fn resolve_import(root: &Path, file: &str, source: &str) -> Option<String> {
    if !source.starts_with('.') || source.contains(['?', '#', '\\']) {
        return None;
    }
    let joined = Path::new(file).parent()?.join(source);
    let mut clean = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::Normal(s) => clean.push(s),
            Component::CurDir => {}
            Component::ParentDir => {
                if !clean.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let canonical = root.join(&clean).canonicalize().ok()?;
    if !canonical.starts_with(root.canonicalize().ok()?) {
        return None;
    }
    Some(clean.to_string_lossy().replace('\\', "/"))
}
fn safe_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_:.".contains(&b))
}
fn api_expr(stmt: &Stmt) -> Vec<&Expr> {
    match stmt {
        Stmt::ExportDefault { expr, .. } => vec![expr],
        Stmt::Export { decl, .. } => api_expr(decl),
        Stmt::Var(v) => v.decls.iter().filter_map(|d| d.init.as_ref()).collect(),
        _ => vec![],
    }
}
fn build(root: &Path) -> Result<Graph, String> {
    if !root.is_dir() {
        return Err("project root must be an existing directory".into());
    }
    let mut g = Graph::default();
    let mut files = vec![];
    walk(root, root, 0, &mut files, &mut g.truncated);
    let mut total = 0;
    let resources = crate::backend::contracts(root, "resources").unwrap_or_else(|_| {
        g.unresolved(
            "server/resources/",
            "invalid resource contract; run brain check for B0735",
        );
        vec![]
    });
    let jobs = crate::backend::contracts(root, "jobs").unwrap_or_else(|_| {
        g.unresolved(
            "server/jobs/",
            "invalid job contract; run brain check for B0735",
        );
        vec![]
    });
    let resource_names: BTreeSet<_> = resources.iter().map(|r| r.name.clone()).collect();
    for r in resources {
        let id = format!("resource:{}", r.name);
        let base = format!("server/resources/{}", r.name);
        g.node(&id, "resource", Some(&format!("{base}/schema.jsbrain")));
        let policy = format!("policy:{}", r.name);
        g.node(&policy, "policy", Some(&format!("{base}/policy.jsbrain")));
        g.edge(
            &id,
            &policy,
            "authorized-by",
            &format!("{base}/policy.jsbrain"),
            None,
        );
        let cache = format!("cache:resource:{}", r.name);
        g.node(&cache, "cache-invalidation-target", None);
        g.edge(
            &cache,
            &id,
            "depends-on",
            &format!("{base}/schema.jsbrain"),
            None,
        );
        for field in r.fields.keys() {
            let f = format!("field:{}.{}", r.name, field);
            g.node(
                &f,
                "resource-field",
                Some(&format!("{base}/schema.jsbrain")),
            );
            g.edge(&f, &id, "field-of", &format!("{base}/schema.jsbrain"), None);
        }
        for op in ["create", "update", "delete"] {
            if let Some(queue) = r.hooks.queue(op) {
                let hook = format!("hook:{}:{op}", r.name);
                let job = format!("job:{queue}");
                g.node(&hook, "hook", Some(&format!("{base}/hooks.jsbrain")));
                g.node(
                    &job,
                    "job",
                    Some(&format!("server/jobs/{queue}/handler.jsbrain")),
                );
                g.edge(
                    &id,
                    &hook,
                    &format!("after-{op}"),
                    &format!("{base}/hooks.jsbrain"),
                    None,
                );
                g.edge(
                    &hook,
                    &job,
                    "enqueues",
                    &format!("{base}/hooks.jsbrain"),
                    None,
                );
            }
        }
    }
    for job in jobs {
        let file = format!("server/jobs/{}/handler.jsbrain", job.name);
        let id = format!("job:{}", job.name);
        g.node(&id, "job", Some(&file));
        g.node(&format!("file:{file}"), "server-module", Some(&file));
        g.edge(&id, &format!("file:{file}"), "handler", &file, None);
    }
    for path in files {
        let file = relative(root, &path);
        let size = std::fs::metadata(&path)
            .map(|m| m.len())
            .unwrap_or(MAX_BYTES + 1);
        if size > MAX_BYTES || total + size > MAX_TOTAL {
            g.truncated = true;
            g.unresolved(&file, "source byte limit exceeded");
            continue;
        }
        total += size;
        let Ok(src) = std::fs::read_to_string(&path) else {
            g.unresolved(&file, "source is unreadable or not UTF-8");
            continue;
        };
        let id = format!("file:{file}");
        let kind = if file.starts_with("server/") {
            "server-module"
        } else if file.ends_with(".jsbrain") {
            "module"
        } else {
            "component"
        };
        g.node(&id, kind, Some(&file));
        if let Some(route) = crate::map::route_of(&file) {
            if file.starts_with("views/") {
                let route_id = format!("route:{route}");
                g.node(&route_id, "view-route", Some(&file));
                g.edge(&route_id, &id, "renders", &file, None);
            }
        }
        let c = match brain_parse::parse(&src, Path::new(&file)) {
            Ok(c) => c,
            Err(ds) => {
                for d in ds.into_iter().take(20) {
                    g.diagnostics.push(json!({"code":d.code,"file":file,"line":d.line,"column":d.col,"severity":d.level.as_str(),"action":"Run brain check for the diagnostic message and exact fix."}));
                }
                g.unresolved(&file, "parse failure prevents dependency extraction");
                continue;
            }
        };
        for stmt in &c.script {
            if let Stmt::Import(i) = stmt {
                if let Some(target) = resolve_import(root, &file, &i.source) {
                    let dest = format!("file:{target}");
                    g.node(&dest, "imported-file", Some(&target));
                    g.edge(&id, &dest, "imports", &file, Some(i.span));
                } else {
                    g.unresolved(
                        &file,
                        "external, unresolved, or nonlocal import; no effects inferred",
                    );
                }
            }
            for expr in api_expr(stmt) {
                let Expr::Call {
                    callee, args, span, ..
                } = expr.unparen()
                else {
                    continue;
                };
                if !matches!(callee.unparen(),Expr::Ident(i) if i.name == "api") {
                    continue;
                }
                let Some(Expr::Str { value, .. }) = args.first().map(Expr::unparen) else {
                    g.unresolved(&file, "dynamic API contract path");
                    continue;
                };
                // Only the backend's exact local resource routes establish a cross-layer edge.
                let Some(name) = value
                    .strip_prefix("/api/")
                    .filter(|n| resource_names.contains(*n))
                else {
                    g.unresolved(
                        &file,
                        "API path has no exact declared backend resource match",
                    );
                    continue;
                };
                let api = format!("api:{name}");
                g.node(&api, "client-api-contract", Some(&file));
                g.edge(&id, &api, "declares-api", &file, Some(*span));
                g.edge(
                    &api,
                    &format!("resource:{name}"),
                    "resource-endpoint",
                    &file,
                    Some(*span),
                );
            }
        }
        let deps = brain_check::contract::deps(&c);
        for (keys, relation) in [(deps.reads, "reads"), (deps.writes, "writes")] {
            for key in keys {
                if !safe_key(&key) {
                    g.unresolved(
                        &file,
                        "computed dependency key omitted; inspect the component card locally",
                    );
                    continue;
                }
                let shared = format!("state:{key}");
                g.node(&shared, "shared-state", None);
                g.edge(&id, &shared, relation, &file, None);
            }
        }
    }
    g.edges.sort_by_key(Value::to_string);
    g.edges.dedup();
    Ok(g)
}

/// Cursor independently indexes each deterministic collection. Totals disclose omitted records.
pub fn graph_page(root: &Path, cursor: usize, limit: usize) -> Result<Value, String> {
    let g = build(root)?;
    let limit = limit.clamp(1, 500);
    let nodes: Vec<_> = g.nodes.values().skip(cursor).take(limit).cloned().collect();
    let edges: Vec<_> = g.edges.iter().skip(cursor).take(limit).cloned().collect();
    let total = g
        .nodes
        .len()
        .max(g.edges.len())
        .max(g.unknown.len())
        .max(g.diagnostics.len());
    Ok(
        json!({"schemaVersion":1,"analysis":"static-declarations","runtimeProof":false,"nodes":nodes,"edges":edges,"unknown":g.unknown.iter().skip(cursor).take(limit).collect::<Vec<_>>(),"diagnostics":g.diagnostics.iter().skip(cursor).take(limit).collect::<Vec<_>>(),"totals":{"nodes":g.nodes.len(),"edges":g.edges.len(),"unknown":g.unknown.len(),"diagnostics":g.diagnostics.len()},"cursor":cursor,"nextCursor":(cursor.saturating_add(limit)<total).then_some(cursor.saturating_add(limit)),"truncated":g.truncated,"limits":{"files":MAX_FILES,"fileBytes":MAX_BYTES,"totalBytes":MAX_TOTAL,"nodes":MAX_RECORDS,"edges":MAX_RECORDS},"integration":"Backend contract matching does not prove attachment to the frontend dev or SSR server."}),
    )
}
fn matches(g: &Graph, target: &str) -> BTreeSet<String> {
    g.nodes
        .iter()
        .filter(|(id, n)| {
            id.as_str() == target
                || n["file"].as_str() == Some(target)
                || id.strip_prefix("file:") == Some(target)
                || id.strip_prefix("resource:") == Some(target)
                || id.strip_prefix("field:") == Some(target)
                || id.strip_prefix("state:") == Some(target)
        })
        .map(|(id, _)| id.clone())
        .collect()
}
fn closure(g: &Graph, initial: &BTreeSet<String>) -> BTreeSet<String> {
    let mut found = initial.clone();
    let mut queue: VecDeque<_> = initial.iter().cloned().collect();
    while let Some(next) = queue.pop_front() {
        for e in &g.edges {
            if matches!(
                e["relation"].as_str(),
                Some(
                    "imports"
                        | "renders"
                        | "reads"
                        | "writes"
                        | "declares-api"
                        | "resource-endpoint"
                        | "depends-on"
                        | "field-of"
                )
            ) && e["to"].as_str() == Some(next.as_str())
            {
                if let Some(from) = e["from"].as_str() {
                    if found.insert(from.into()) {
                        queue.push_back(from.into());
                    }
                }
            }
        }
    }
    found
}
/// Conservative reverse dependency closure; this reports a plan, it does not invalidate runtime caches.
pub fn impact(root: &Path, target: &str, operation: Option<&str>) -> Result<Value, String> {
    if operation.is_some_and(|o| !matches!(o, "create" | "update" | "delete")) {
        return Err("operation must be create, update, or delete".into());
    }
    let g = build(root)?;
    let initial = matches(&g, target);
    let mut seeds = initial.clone();
    for id in &initial {
        if id.starts_with("field:") {
            for e in &g.edges {
                if e["from"].as_str() == Some(id) && e["relation"] == "field-of" {
                    if let Some(to) = e["to"].as_str() {
                        seeds.insert(to.into());
                    }
                }
            }
        }
    }
    let affected = closure(&g, &seeds);
    let mut queues = BTreeSet::new();
    for e in &g.edges {
        if seeds.contains(e["from"].as_str().unwrap_or(""))
            && e["relation"].as_str().is_some_and(|r| {
                r.starts_with("after-") && operation.is_none_or(|op| r == format!("after-{op}"))
            })
        {
            for q in &g.edges {
                if q["from"] == e["to"] && q["relation"] == "enqueues" {
                    if let Some(to) = q["to"].as_str() {
                        queues.insert(to.to_string());
                    }
                }
            }
        }
    }
    let edges: Vec<_> = g
        .edges
        .iter()
        .filter(|e| {
            affected.contains(e["from"].as_str().unwrap_or(""))
                && affected.contains(e["to"].as_str().unwrap_or(""))
        })
        .take(500)
        .collect();
    Ok(
        json!({"schemaVersion":1,"analysis":"static-declarations","runtimeProof":false,"target":target,"matched":initial,"operation":operation,"affected":affected.iter().take(500).collect::<Vec<_>>(),"paths":edges,"invalidationPlan":{"performed":false,"views":affected.iter().filter(|id|id.starts_with("route:")).take(100).collect::<Vec<_>>(),"resources":seeds.iter().filter(|id|id.starts_with("resource:")).collect::<Vec<_>>(),"cacheKeys":affected.iter().filter(|id|id.starts_with("cache:")).take(100).collect::<Vec<_>>(),"queues":queues,"qualification":"Potential import consumers; execution, authorization, cache residency and handler side effects are not observed."},"unknown":g.unknown.iter().take(100).collect::<Vec<_>>(),"truncated":g.truncated || affected.len()>500 || g.unknown.len()>100}),
    )
}

/// Relevant structural context; byte-count budgeting is conservative and explicitly not a model tokenizer.
pub fn context(root: &Path, task: &str, budget: usize) -> Result<Value, String> {
    let g = build(root)?;
    let ceiling = budget.clamp(512, 32_768);
    let words: Vec<_> = task
        .split(|c: char| !c.is_alphanumeric() && !"_./:".contains(c))
        .filter(|w| w.len() > 1)
        .map(str::to_lowercase)
        .collect();
    let mut ranked: Vec<_> = g
        .nodes
        .values()
        .map(|n| {
            let text = n.to_string().to_lowercase();
            (
                words.iter().filter(|w| text.contains(w.as_str())).count(),
                n,
            )
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1["id"].as_str().cmp(&b.1["id"].as_str()))
    });
    let mut out = json!({"schemaVersion":1,"budget":{"requested":budget,"maximumBytes":ceiling,"unit":"UTF-8 bytes; conservative token upper estimate, not tokenizer tokens"},"analysis":"static-declarations","runtimeProof":false,"selection":"lexical task relevance over contract identifiers; no source bodies","nodes":[],"edges":[],"unknownCount":g.unknown.len(),"truncated":true});
    for (_, node) in ranked.iter().take(100) {
        out["nodes"].as_array_mut().unwrap().push((*node).clone());
        if out.to_string().len() > ceiling.saturating_sub(32) {
            out["nodes"].as_array_mut().unwrap().pop();
            break;
        }
    }
    let chosen: BTreeSet<_> = out["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["id"].as_str().map(str::to_owned))
        .collect();
    for e in g
        .edges
        .iter()
        .filter(|e| {
            chosen.contains(e["from"].as_str().unwrap_or(""))
                && chosen.contains(e["to"].as_str().unwrap_or(""))
        })
        .take(100)
    {
        out["edges"].as_array_mut().unwrap().push(e.clone());
        if out.to_string().len() > ceiling.saturating_sub(32) {
            out["edges"].as_array_mut().unwrap().pop();
            break;
        }
    }
    out["truncated"] = json!(g.truncated || chosen.len() < ranked.len());
    Ok(out)
}

/// Redact literal payloads from checker prose while retaining instructions and diagnostic identity.
fn safe_diagnostic_text(input: &str, root: &Path) -> String {
    let input = input.replace(root.to_string_lossy().as_ref(), ".");
    if input.to_ascii_lowercase().contains("password")
        || input.to_ascii_lowercase().contains("secret")
        || input.to_ascii_lowercase().contains("token=")
    {
        return "Sensitive diagnostic detail omitted; run brain check locally for the exact fix."
            .into();
    }
    let mut out = String::new();
    let mut quote = None;
    let mut escaped = false;
    for c in input.chars().take(2000) {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                out.push(c);
                quote = None;
            }
        } else if matches!(c, '\'' | '"' | '`') {
            out.push(c);
            out.push_str("[literal omitted]");
            quote = Some(c);
        } else {
            out.push(c);
        }
    }
    out
}
fn findings(
    root: &Path,
    target: Option<&str>,
    affected: &BTreeSet<String>,
) -> Result<(Vec<Value>, usize), String> {
    let (_, ds, base) = crate::run_check_base(Some(root.to_path_buf()))?;
    let mut result = vec![];
    let mut total = 0;
    for d in ds {
        let absolute = if Path::new(&d.file).is_absolute() {
            PathBuf::from(&d.file)
        } else {
            base.join(&d.file)
        };
        let file = relative(root, &absolute);
        // A checker base can be outside a nested app; never leak the absolute ancestor.
        if Path::new(&file).is_absolute() {
            continue;
        }
        if target.is_some_and(|t| file != t && !affected.contains(&format!("file:{file}"))) {
            continue;
        }
        total += 1;
        if result.len() >= 100 {
            continue;
        }
        result.push(json!({"code":d.code,"file":file,"line":d.line,"column":d.col,"severity":d.level.as_str(),"message":safe_diagnostic_text(&d.msg,root),"fix":safe_diagnostic_text(&d.fix,root),"source":"compiler-checker","staticEvidence":true,"observedRuntimeFailure":false,"nextCommand":format!("brain explain {}",d.code)}));
    }
    result.sort_by_key(|d| {
        (
            d["severity"] != "error",
            d["file"].as_str().unwrap_or("").to_owned(),
            d["line"].as_u64().unwrap_or(0),
            d["column"].as_u64().unwrap_or(0),
        )
    });
    Ok((result, total))
}

/// Preventive findings are compiler observations, not predictions of a runtime root cause.
pub fn preflight(root: &Path, target: Option<&str>) -> Result<Value, String> {
    let g = build(root)?;
    let matched = target.map(|t| matches(&g, t)).unwrap_or_default();
    let affected = closure(&g, &matched);
    let (diagnostics, total) = findings(root, target, &affected)?;
    let first = diagnostics
        .iter()
        .find(|d| d["severity"] == "error")
        .cloned();
    Ok(
        json!({"schemaVersion":1,"analysis":"compiler-checker-and-static-declarations","staticEvidence":true,"runtimeProof":false,"target":target,"firstBlockingDiagnostic":first,"diagnostics":diagnostics,"diagnosticTotal":total,"affected":affected.iter().take(500).collect::<Vec<_>>(),"risks":[],"unknown":g.unknown.iter().take(100).collect::<Vec<_>>(),"truncated":g.truncated || total>100 || affected.len()>500 || g.unknown.len()>100,"qualification":"Compiler findings identify source defects. Runtime execution, private request data and handler side effects are not observed."}),
    )
}

/// Actual compiler findings plus dependency evidence. Dependencies are not proven runtime causes.
pub fn why(root: &Path, target: &str) -> Result<Value, String> {
    let report = impact(root, target, None)?;
    let affected: BTreeSet<String> = report["affected"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let (diagnostics, total) = findings(root, Some(target), &affected)?;
    let first = diagnostics
        .iter()
        .find(|d| d["severity"] == "error")
        .cloned();
    Ok(
        json!({"schemaVersion":1,"firstBlockingDiagnostic":first,"rootCause":"Runtime cause unproven; compiler defects retain their observed source location.","staticEvidence":true,"runtimeProof":false,"diagnostics":diagnostics,"diagnosticTotal":total,"causalEvidence":report,"nextSteps":["Resolve reported compiler diagnostics at their source locations.","Inspect the affected component and backend contract cards.","Reproduce the runtime failure and collect request or reactive trace evidence before assigning a runtime cause."],"redaction":"Quoted source literals are omitted from messages and fixes; run brain check locally for exact text."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closure_keeps_hook_effects_separate() {
        let mut g = Graph::default();
        g.edge(
            "file:views/home.brain",
            "file:api/posts.jsbrain",
            "imports",
            "views/home.brain",
            None,
        );
        g.edge(
            "file:api/posts.jsbrain",
            "resource:posts",
            "resource-endpoint",
            "api/posts.jsbrain",
            None,
        );
        g.edge(
            "resource:posts",
            "job:email",
            "after-create",
            "server/resources/posts/hooks.jsbrain",
            None,
        );
        let affected = closure(&g, &BTreeSet::from(["resource:posts".into()]));
        assert!(affected.contains("file:views/home.brain"));
        assert!(!affected.contains("job:email"));
    }
    #[test]
    fn private_literals_are_not_dependency_keys() {
        assert!(safe_key("cart.total"));
        assert!(!safe_key("resource:fetch('secret')"));
        assert!(!safe_key("a\nPASSWORD=value"));
    }
    #[test]
    fn quoted_checker_values_are_redacted() {
        let result = safe_diagnostic_text(
            "unexpected 'private-value'; use `safe()`",
            Path::new("/project"),
        );
        assert!(!result.contains("private-value"));
        assert!(result.contains("unexpected"));
    }
    #[test]
    fn literal_api_contract_links_view_resource_and_queue() {
        let root = std::env::temp_dir().join(format!(
            "brain-intelligence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("views")).unwrap();
        std::fs::create_dir_all(root.join("api")).unwrap();
        std::fs::create_dir_all(root.join("server/resources/posts")).unwrap();
        std::fs::create_dir_all(root.join("server/jobs/notify")).unwrap();
        std::fs::write(
            root.join("views/home.brain"),
            "<script>import posts from '../api/posts.jsbrain'</script><main>Posts</main>",
        )
        .unwrap();
        std::fs::write(
            root.join("api/posts.jsbrain"),
            "export default api('/api/posts', { get: { out: '[Post]' } })",
        )
        .unwrap();
        std::fs::write(
            root.join("server/resources/posts/schema.jsbrain"),
            "export default { title: 'str!' }",
        )
        .unwrap();
        std::fs::write(root.join("server/resources/posts/policy.jsbrain"), "export default { read: ['public'], create: ['admin'], update: ['admin'], delete: ['admin'] }").unwrap();
        std::fs::write(
            root.join("server/resources/posts/hooks.jsbrain"),
            "export default { after_create: 'notify' }",
        )
        .unwrap();
        std::fs::write(
            root.join("server/jobs/notify/schema.jsbrain"),
            "export default { record: 'obj!' }",
        )
        .unwrap();
        let report = impact(&root, "posts", Some("create")).unwrap();
        assert!(
            report["affected"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "file:views/home.brain")
        );
        assert!(
            report["invalidationPlan"]["queues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "job:notify")
        );
        let context = context(&root, "posts", 512).unwrap();
        assert!(context.to_string().len() <= 512);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn parent_import_cannot_escape_project() {
        assert!(
            resolve_import(
                Path::new("."),
                "views/home.brain",
                "../../../secrets.jsbrain"
            )
            .is_none()
        );
    }
}
