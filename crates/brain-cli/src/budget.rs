//! Build budgets use measured emitted bytes, never source size or estimated browser timings.
use std::path::Path;
use brain_mcp::Json;

pub(crate) fn enforce(root: &Path, report: &[(String, [usize; 7])]) -> Result<(), String> {
    let path = root.join("brain.json");
    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot read brain.json budgets: {e}")),
    };
    let config = Json::parse(&source).map_err(|e| format!("invalid brain.json: {e}"))?;
    enforce_config(&config, report)
}

fn enforce_config(config: &Json, report: &[(String, [usize; 7])]) -> Result<(), String> {
    let Some(budget) = config.get("budget") else { return Ok(()) };
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
    let Some(limit) = limit else { return Ok(()) };
    let mut failures = Vec::new();
    for (route, bytes) in report {
        let total = bytes[2].checked_add(bytes[4]).and_then(|n| n.checked_add(bytes[6])).ok_or("route byte count overflow")?;
        if total as f64 > limit {
            failures.push(format!("budget.viewKB exceeded on {route}: {total} bytes ({:.3} KiB) > {:.3} KiB; fix: reduce this route's JS/CSS/HTML or explicitly adjust budget.viewKB", total as f64 / 1024.0, limit / 1024.0));
        }
    }
    if failures.is_empty() { Ok(()) } else { Err(failures.join("\n")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cfg(s: &str) -> Json { Json::parse(s).unwrap() }
    #[test]
    fn exact_boundary_fractional_limits_and_each_route_are_enforced() {
        let config = cfg(r#"{"budget":{"viewKB":0.5}}"#);
        assert!(enforce_config(&config, &[("/ok".into(), [9999, 9999, 200, 9999, 100, 9999, 212])]).is_ok());
        let error = enforce_config(&config, &[("/large".into(), [0,0,200,0,100,0,213]), ("/other".into(), [0,0,513,0,0,0,0])]).unwrap_err();
        assert!(error.contains("/large: 513 bytes") && error.contains("/other: 513 bytes"));
    }
    #[test]
    fn malformed_unknown_and_unmeasured_budgets_never_silently_pass() {
        for value in [r#"{"viewKB":-1}"#, r#"{"viewKB":"30"}"#, r#"{"viewKB":1,"viewKB":2}"#, r#"{"viewKb":1}"#, r#"{"mountMs":16}"#, r#"{"heapMB":20}"#, "null"] {
            assert!(enforce_config(&cfg(&format!("{{\"budget\":{value}}}")), &[]).is_err(), "{value}");
        }
        assert!(enforce_config(&cfg("{}"), &[]).is_ok());
        assert!(enforce_config(&cfg(r#"{"budget":{"viewKB":0}}"#), &[("/".into(), [0;7])]).is_ok());
        assert!(enforce_config(&cfg(r#"{"budget":{"viewKB":0}}"#), &[("/".into(), [0,0,1,0,0,0,0])]).is_err());
    }
}
