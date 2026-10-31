//! Measured edit risk (INTERNALS §47): a deterministic 0..100 score with named factors and the exact checks that
//! prove an edit safe. Factors come from static declarations (`intelligence`), checker findings and, on request,
//! a production byte measurement. A score is a review priority, never proof that an edit is safe or unsafe.
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

/// What the agent named, resolved to the graph target and its source file (when it has one).
#[derive(Debug)]
pub(crate) struct Target {
    pub graph: String,
    pub file: Option<String>,
}

const PREFIXES: [&str; 8] = ["resource:", "route:", "field:", "state:", "api:", "file:", "job:", "policy:"];

pub(crate) fn resolve(root: &Path, target: &str) -> Result<Target, String> {
    if target.is_empty() || target.len() > 240 {
        return Err("target must be 1..240 bytes; fix: pass a file, component, route:/path or resource:name".into());
    }
    if let Some(p) = PREFIXES.iter().find(|p| target.starts_with(**p)) {
        let file = target.strip_prefix("file:").filter(|_| *p == "file:").map(str::to_owned);
        return Ok(Target { graph: target.into(), file });
    }
    let rel = target.strip_prefix("./").unwrap_or(target);
    if rel.contains('/') || rel.ends_with(".brain") || rel.ends_with(".jsbrain") || rel.ends_with(".cssbrain") {
        let path = crate::changes::source_path(root, rel)?;
        if !path.is_file() {
            return Err(format!("{rel} does not exist; fix: run `brain map` and pass a listed file"));
        }
        return Ok(Target { graph: rel.into(), file: Some(rel.into()) });
    }
    let file = crate::card::resolve(root, target)?;
    Ok(Target { graph: file.clone(), file: Some(file) })
}

fn bucket(n: usize, steps: &[(usize, u64)]) -> u64 {
    steps.iter().rev().find(|(min, _)| n >= *min).map(|(_, p)| *p).unwrap_or(0)
}

fn locales(root: &Path) -> usize {
    std::fs::read_to_string(root.join("brain.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|j| j["i18n"]["locales"].as_array().map(Vec::len))
        .unwrap_or(1)
}

fn view_budget(root: &Path) -> Option<u64> {
    std::fs::read_to_string(root.join("brain.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|j| j["budget"]["viewKB"].as_f64())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| (v * 1024.0) as u64)
}

/// Static risk: blast radius, contracts, backend, tests, surfaces and (optionally) measured budget headroom.
/// `errors` is the number of checker errors already in the target's scope (computed by the caller).
pub(crate) fn score(root: &Path, target: &Target, errors: usize, measure: bool) -> Result<Value, String> {
    let report = crate::intelligence::impact(root, &target.graph, None)?;
    if report["matched"].as_array().is_none_or(|m| m.is_empty()) {
        return Err(format!(
            "`{}` is not in the static graph; fix: run `brain graph --json` and pass a listed node or file",
            target.graph
        ));
    }
    let affected: Vec<String> = report["affected"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let own: BTreeSet<String> = report["matched"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let files: Vec<&str> = affected.iter().filter_map(|id| id.strip_prefix("file:")).collect();
    let own_file = target.file.clone().or_else(|| own.iter().find_map(|id| id.strip_prefix("file:").map(str::to_owned)));
    let mut tests: BTreeSet<String> = files.iter().filter(|f| f.ends_with(".test.brain")).map(|f| f.to_string()).collect();
    if let Some(f) = own_file.as_deref().and_then(|f| f.strip_suffix(".brain")) {
        let sibling = format!("{f}.test.brain");
        if root.join(&sibling).is_file() {
            tests.insert(sibling);
        }
    }
    let dependents: Vec<&str> = files
        .iter()
        .copied()
        .filter(|f| Some(*f) != own_file.as_deref() && !f.ends_with(".test.brain") && !own.contains(&format!("file:{f}")))
        .collect();
    let routes: Vec<&str> = affected.iter().filter_map(|id| id.strip_prefix("route:")).collect();
    let views: Vec<&str> = files.iter().copied().filter(|f| f.starts_with("views/") && f.ends_with(".brain") && !f.ends_with(".test.brain")).collect();
    let source = own_file.as_deref().and_then(|f| crate::map::read_source(root, f).ok()).unwrap_or_default();
    let backend = target.graph.starts_with("resource:")
        || target.graph.starts_with("field:")
        || target.graph.starts_with("job:")
        || target.graph.starts_with("policy:")
        || own_file.as_deref().is_some_and(|f| f.starts_with("server/"));
    let schema = target.graph.starts_with("field:")
        || target.graph.starts_with("resource:")
        || own_file.as_deref().is_some_and(|f| f.starts_with("server/resources/") && f.ends_with("/schema.jsbrain"));
    let contract = target.graph.starts_with("api:")
        || backend
        || source.contains("<meta props")
        || source.contains("emit(")
        || (own_file.as_deref().is_some_and(|f| f.ends_with(".jsbrain")) && source.contains("export"));
    let markup = own_file.as_deref().is_some_and(|f| f.ends_with(".brain"));
    let seo: Vec<&str> = views
        .iter()
        .copied()
        .filter(|v| crate::map::read_source(root, v).is_ok_and(|s| s.contains("<meta seo")))
        .collect();
    let i18n = markup && locales(root) > 1;
    let styles = own_file.as_deref().is_some_and(|f| f.ends_with(".cssbrain"));

    let mut factors = vec![];
    let mut push = |name: &str, points: u64, value: Value, why: &str| {
        factors.push(json!({"factor":name,"points":points,"value":value,"reason":why}));
    };
    push("dependents", bucket(dependents.len(), &[(1, 5), (3, 10), (6, 18), (16, 25)]), json!(dependents.len()),
        "files that import or render the target (static reverse closure)");
    push("routes", bucket(routes.len(), &[(1, 5), (2, 10), (5, 15)]), json!(routes), "routes whose output can change");
    push("publicContract", if contract { if dependents.is_empty() { 5 } else { 15 } } else { 0 }, json!(contract),
        "props, emits, exports, API or backend contract consumed by other files");
    push("backendSchema", if schema { 20 } else if backend { 10 } else { 0 }, json!({"backend":backend,"schemaOrMigration":schema}),
        "server contracts can require a migration and affect stored data");
    push("tests", if tests.is_empty() { 15 } else { 0 }, json!(tests), "test files that exercise the target or a dependent");
    push("checkerErrors", if errors > 0 { 10 } else { 0 }, json!(errors), "compiler errors already present in the affected scope");
    push("seo", if seo.is_empty() { 0 } else { 4 }, json!(seo), "affected views that publish SEO metadata");
    push("i18n", if i18n { 3 } else { 0 }, json!(i18n), "visible text in a multilingual app");
    push("a11y", if markup { 3 } else { 0 }, json!(markup), "markup changes re-run accessibility rules B04xx");
    push("styles", if styles { 5 } else { 0 }, json!(styles), "design tokens and global styles reach every component that uses them");
    let limit = view_budget(root);
    let mut budget = json!({"limitBytes":limit,"measured":false});
    if measure && limit.is_some() && !routes.is_empty() {
        let m = crate::impact::measure(root)?;
        let limit = limit.unwrap_or(0);
        let rows: Vec<Value> = m
            .values()
            .filter(|v| routes.contains(&v.route.as_str()))
            .map(|v| json!({"route":v.route,"bytes":v.total(),"headroomBytes":limit as i64 - v.total() as i64}))
            .collect();
        let min = rows.iter().filter_map(|r| r["headroomBytes"].as_i64()).min();
        let points = match min {
            Some(h) if h < (limit as i64) / 20 => 10,
            Some(h) if h < (limit as i64) * 3 / 20 => 5,
            _ => 0,
        };
        budget = json!({"limitBytes":limit,"measured":true,"unit":"gz bytes of JS+CSS+HTML per view (brain impact measurement)","views":rows,"minHeadroomBytes":min});
        push("budgetHeadroom", points, budget.clone(), "views close to budget.viewKB fail the build after a small addition");
    } else {
        push("budgetHeadroom", 0, budget.clone(), "not measured; pass --measure (or measure:true) to build and compare with budget.viewKB");
    }
    let total: u64 = factors.iter().filter_map(|f| f["points"].as_u64()).sum::<u64>().min(100);
    let level = match total {
        0..=24 => "low",
        25..=49 => "medium",
        50..=74 => "high",
        _ => "critical",
    };
    let mut checks = vec![json!({"command":"brain check","proves":"no compiler, accessibility, security or i18n error in the app"})];
    if !tests.is_empty() {
        for t in &tests {
            checks.push(json!({"command":format!("brain test {t}"),"proves":"existing behaviour of the target or a dependent still passes"}));
        }
    } else if let Some(f) = own_file.as_deref().filter(|f| f.ends_with(".brain")) {
        checks.push(json!({"command":format!("add {}.test.brain then brain test", f.trim_end_matches(".brain")),"proves":"no test covers this target yet; one test turns the score's largest unknown into evidence"}));
    }
    if !routes.is_empty() && limit.is_some() {
        checks.push(json!({"command":"brain build","proves":"every affected view stays within budget.viewKB"}));
    }
    if backend {
        checks.push(json!({"command":"brain backend schema --json","proves":"the resource contract and the migration it needs are explicit"}));
    }
    if !seo.is_empty() {
        checks.push(json!({"command":"brain build && brain seo audit --json","proves":"SEO score of the affected views stays above seo.minScore"}));
    }
    Ok(json!({
        "target": target.graph,
        "file": own_file,
        "score": total,
        "level": level,
        "factors": factors,
        "requiredChecks": checks,
        "blastRadius": {"files": dependents, "routes": routes, "tests": tests},
        "analysis": "static-declarations plus checker findings; budget only when measured",
        "runtimeProof": false,
        "truncated": report["truncated"],
        "qualification": "A score orders review effort. Dynamic imports, runtime data and handler side effects are reported as unknown by brain graph, not scored."
    }))
}

/// `brain risk <target> [--measure]` and MCP `brain_risk`.
pub fn assess(root: &Path, target: &str, measure: bool) -> Result<Value, String> {
    let t = resolve(root, target)?;
    let pre = crate::intelligence::preflight(root, Some(&t.graph))?;
    let errors = pre["diagnostics"].as_array().map(|d| d.iter().filter(|d| d["severity"] == "error").count()).unwrap_or(0);
    let mut value = score(root, &t, errors, measure)?;
    value["firstBlockingDiagnostic"] = pre["firstBlockingDiagnostic"].clone();
    Ok(value)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) struct App(pub std::path::PathBuf);
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    pub(crate) fn app(tag: &str) -> App {
        let root = std::env::temp_dir().join(format!(
            "brain-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        crate::scaffold::new_app(&root).unwrap();
        App(root.canonicalize().unwrap())
    }
    fn factor(v: &Value, name: &str) -> Value {
        v["factors"].as_array().unwrap().iter().find(|f| f["factor"] == name).cloned().unwrap()
    }

    #[test]
    fn shared_component_scores_higher_than_a_leaf_and_names_its_checks() {
        let a = app("risk");
        let root = &a.0;
        std::fs::create_dir_all(root.join("comp")).unwrap();
        std::fs::write(root.join("comp/badge.brain"), "<meta props=\"label:str!\">\n<span>{props.label}</span>\n").unwrap();
        std::fs::write(root.join("comp/leaf.brain"), "<p>Leaf</p>\n").unwrap();
        for (i, view) in ["views/one.brain", "views/two.brain"].iter().enumerate() {
            std::fs::write(
                root.join(view),
                format!("<meta seo title=\"Page {i}\" description=\"A page that shows a badge for the example shop.\">\n<script>import Badge from '../comp/badge.brain'</script>\n<main><Badge label=\"x\" /></main>\n"),
            )
            .unwrap();
        }
        let shared = score(root, &resolve(root, "comp/badge.brain").unwrap(), 0, false).unwrap();
        let leaf = score(root, &resolve(root, "comp/leaf.brain").unwrap(), 0, false).unwrap();
        assert!(shared["score"].as_u64() > leaf["score"].as_u64(), "{shared}\n{leaf}");
        assert_eq!(factor(&shared, "dependents")["value"], 2);
        assert_eq!(factor(&shared, "routes")["value"].as_array().unwrap().len(), 2);
        assert_eq!(factor(&shared, "publicContract")["points"], 15);
        assert_eq!(factor(&shared, "tests")["points"], 15);
        assert_eq!(factor(&shared, "seo")["value"].as_array().unwrap().len(), 2);
        let checks = shared["requiredChecks"].to_string();
        assert!(checks.contains("brain build") && checks.contains("seo audit") && checks.contains("comp/badge.test.brain"), "{checks}");
        let pre = crate::intelligence::preflight(root, Some("comp/badge.brain")).unwrap();
        assert_eq!(pre["risks"][0]["target"], "comp/badge.brain", "{pre}");
        let errors = pre["diagnostics"].as_array().unwrap().iter().filter(|d| d["severity"] == "error").count();
        assert_eq!(factor(&pre["risks"][0], "checkerErrors")["value"], errors);
        assert_eq!(factor(&pre["risks"][0], "dependents"), factor(&shared, "dependents"));
        // Deterministic: same input, same report.
        assert_eq!(shared, score(root, &resolve(root, "comp/badge.brain").unwrap(), 0, false).unwrap());
    }

    #[test]
    fn covering_tests_remove_the_test_factor_and_become_required_checks() {
        let a = app("risk-tests");
        let root = &a.0;
        let r = score(root, &resolve(root, "Counter").unwrap(), 0, false).unwrap();
        assert_eq!(factor(&r, "tests")["points"], 0, "{r}");
        assert!(r["requiredChecks"].to_string().contains("brain test comp/counter.test.brain"), "{r}");
    }

    #[test]
    fn unknown_targets_fail_with_a_fix() {
        let a = app("risk-unknown");
        assert!(resolve(&a.0, "NoSuchThing").unwrap_err().contains("fix:"));
        assert!(score(&a.0, &Target { graph: "resource:none".into(), file: None }, 0, false).unwrap_err().contains("fix:"));
    }
}
