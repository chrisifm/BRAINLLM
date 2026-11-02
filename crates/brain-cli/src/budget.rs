//! Build budgets use measured emitted bytes, never source size or estimated browser timings.
use std::path::Path;
use brain_mcp::Json;

/// A route within this share of `budget.viewKB` (or NEAR_BYTES, whichever is larger) passes with a warning: its HTML
/// carries the environment's values (PUBLIC_* constants, the site URL), so the same commit can measure more on another
/// machine (a reported app measured 174 bytes more on its server than locally).
const NEAR_LIMIT: f64 = 0.02;
const NEAR_BYTES: f64 = 256.0;

/// Checks every route against `budget.viewKB`. `document` names what was measured (environment and site URL), because
/// `brain build` and `brain doctor` can measure different HTML for the same route. Ok holds the near-limit warnings.
pub(crate) fn enforce(root: &Path, report: &[(String, [usize; 8])], document: &str) -> Result<Vec<String>, String> {
    let path = root.join("brain.json");
    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("cannot read brain.json budgets: {e}")),
    };
    let config = Json::parse(&source).map_err(|e| format!("invalid brain.json: {e}"))?;
    enforce_config(&config, report, document)
}

/// What a build measured: the environment and whether its pages carry absolute URLs to a site origin.
pub(crate) fn document(env_name: &str, site: bool) -> String {
    format!("measured on the {env_name} build with this machine's [{env_name}] .brain_env values and {}", if site { "absolute URLs to its site URL" } else { "no site URL (no absolute URLs)" })
}

fn enforce_config(config: &Json, report: &[(String, [usize; 8])], document: &str) -> Result<Vec<String>, String> {
    let Some(budget) = config.get("budget") else { return Ok(vec![]) };
    let Json::Obj(fields) = budget else { return Err("budget must be an object; fix: use {\"budget\":{\"viewKB\":30}}".into()) };
    let mut limit = None;
    for (name, value) in fields {
        if matches!(name.as_str(), "mountMs" | "heapMB") {
            return Err(format!("budget.{name} requires browser measurements and is not implemented; fix: remove this planned budget and use viewKB for build enforcement"));
        }
        if name != "viewKB" { return Err(format!("unknown budget.{name}; fix: use viewKB")); }
        let Json::Num(n) = value else { return Err("budget.viewKB must be a finite non-negative number".into()) };
        if !n.is_finite() || *n < 0.0 || *n > (usize::MAX / 1024) as f64 {
            return Err("budget.viewKB must be finite, non-negative and representable in bytes".into());
        }
        if limit.replace(n * 1024.0).is_some() { return Err("duplicate budget.viewKB; fix: declare one limit".into()); }
    }
    let Some(limit) = limit else { return Ok(vec![]) };
    let (mut failures, mut near) = (Vec::new(), Vec::new());
    // A route can have more than one page behind it (a prerendered and a dynamic one, a fallback): the largest answers.
    let mut largest: Vec<(&str, usize, &[usize; 8])> = Vec::new();
    for (route, bytes) in report {
        let total = bytes[2].checked_add(bytes[4]).and_then(|n| n.checked_add(bytes[6].saturating_sub(bytes[7]))).ok_or("route byte count overflow")?;
        match largest.iter_mut().find(|(r, ..)| *r == route.as_str()) {
            Some(seen) => if total > seen.1 { (seen.1, seen.2) = (total, bytes) },
            None => largest.push((route, total, bytes)),
        }
    }
    for (route, total, bytes) in largest {
        let parts = format!("JS {} + CSS {} + HTML {}, compressed, {document}", bytes[2], bytes[4], bytes[6].saturating_sub(bytes[7]));
        if total as f64 > limit {
            failures.push(format!("budget.viewKB exceeded on {route}: {total} bytes ({:.3} KiB) > {:.3} KiB ({parts}); fix: reduce this route's JS/CSS/HTML or explicitly adjust budget.viewKB; `brain build --report` lists the assets", total as f64 / 1024.0, limit / 1024.0));
        } else if limit > 0.0 && limit - total as f64 <= (limit * NEAR_LIMIT).max(NEAR_BYTES) {
            let headroom = (limit - total as f64).floor() as usize;
            near.push(format!("budget.viewKB nearly reached on {route}: {total} bytes ({:.3} KiB) of {:.3} KiB, {headroom} bytes ({:.1}%) of headroom ({parts}); the HTML carries the environment's PUBLIC_* values and site URL, so it can differ between machines and the same commit can fail the build where those values are longer (the production server); fix: reduce this route's JS/CSS/HTML or raise budget.viewKB with margin; `brain build --report` lists the assets", total as f64 / 1024.0, limit / 1024.0, headroom as f64 * 100.0 / limit));
        }
    }
    if failures.is_empty() { Ok(near) } else { Err(failures.join("\n")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cfg(s: &str) -> Json { Json::parse(s).unwrap() }
    #[test]
    fn exact_boundary_fractional_limits_and_each_route_are_enforced() {
        let config = cfg(r#"{"budget":{"viewKB":0.5}}"#);
        assert!(enforce_config(&config, &[("/ok".into(), [9999, 9999, 200, 9999, 100, 9999, 212,0])], "D").is_ok());
        let error = enforce_config(&config, &[("/large".into(), [0,0,200,0,100,0,213,0]), ("/other".into(), [0,0,513,0,0,0,0,0])], "D").unwrap_err();
        assert!(error.contains("/large: 513 bytes") && error.contains("/other: 513 bytes"));
    }
    #[test]
    fn a_route_with_two_pages_fails_once_at_its_largest_with_the_bytes_broken_down() {
        let config = cfg(r#"{"budget":{"viewKB":0.5}}"#);
        let error = enforce_config(&config, &[("/s".into(), [0,0,600,0,10,0,20,0]), ("/s".into(), [0,0,700,0,10,0,20,0])], "D").unwrap_err();
        assert_eq!(error.matches("exceeded on /s").count(), 1, "{error}");
        assert!(error.contains("730 bytes") && error.contains("JS 700 + CSS 10 + HTML 20"), "{error}");
    }
    #[test]
    fn malformed_unknown_and_unmeasured_budgets_never_silently_pass() {
        for value in [r#"{"viewKB":-1}"#, r#"{"viewKB":"30"}"#, r#"{"viewKB":1,"viewKB":2}"#, r#"{"viewKb":1}"#, r#"{"mountMs":16}"#, r#"{"heapMB":20}"#, "null"] {
            assert!(enforce_config(&cfg(&format!("{{\"budget\":{value}}}")), &[], "D").is_err(), "{value}");
        }
        assert!(enforce_config(&cfg("{}"), &[], "D").is_ok());
        assert!(enforce_config(&cfg(r#"{"budget":{"viewKB":0}}"#), &[("/".into(), [0;8])], "D").is_ok());
        assert!(enforce_config(&cfg(r#"{"budget":{"viewKB":0}}"#), &[("/".into(), [0,0,1,0,0,0,0,0])], "D").is_err());
    }
    #[test]
    fn a_route_within_two_percent_of_the_limit_passes_with_a_warning_that_names_the_measured_document() {
        let config = cfg(r#"{"budget":{"viewKB":74}}"#);
        let doc = document("production", false);
        // the reported case: 75806 bytes failed on the server; locally the same commit measured ~30 bytes less
        let near = enforce_config(&config, &[("/company".into(), [0,0,63623,0,4960,0,7193,0])], &doc).unwrap();
        assert_eq!(near.len(), 1, "{near:?}");
        assert!(near[0].contains("nearly reached on /company: 75776 bytes") && near[0].contains("0 bytes (0.0%) of headroom"), "{}", near[0]);
        assert!(near[0].contains("[production] .brain_env values and no site URL") && near[0].contains("differ between machines"), "{}", near[0]);
        let error = enforce_config(&config, &[("/company".into(), [0,0,63623,0,4960,0,7223,0])], &document("production", true)).unwrap_err();
        assert!(error.contains("75806 bytes") && error.contains("and absolute URLs to its site URL"), "{error}");
        // 3% below the limit is quiet
        assert!(enforce_config(&config, &[("/".into(), [0,0,73000,0,0,0,0,0])], &doc).unwrap().is_empty());
        // a small budget still warns within 256 bytes (2% of 4 KiB is only 82 bytes)
        let small = cfg(r#"{"budget":{"viewKB":4}}"#);
        assert_eq!(enforce_config(&small, &[("/".into(), [0,0,3900,0,0,0,0,0])], &doc).unwrap().len(), 1);
        assert!(enforce_config(&small, &[("/".into(), [0,0,3800,0,0,0,0,0])], &doc).unwrap().is_empty());
    }
}
