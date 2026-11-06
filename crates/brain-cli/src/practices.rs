//! Agent-facing views of the compiler's actual diagnostic catalog. Never executes application source.
use serde_json::{Value, json};

pub fn nudge(code: &str) -> bool {
    matches!(code, "B1706" | "B1707" | "B1708")
}
pub fn adjustable(code: &str) -> bool {
    matches!(code.get(..3), Some("B09" | "B10" | "B11" | "B12" | "B13")) || nudge(code)
}
/// WebMCP discovery advice is optional, but cannot be promoted into an authoring obligation.
pub fn nudge_level<'a>(code: &str, requested: Option<&'a str>, default: &'a str) -> &'a str {
    if nudge(code) {
        requested
            .filter(|value| matches!(*value, "off" | "info" | "warning"))
            .unwrap_or(default)
    } else {
        default
    }
}
fn family(code: &str) -> &'static str {
    match code.get(..3) {
        Some("B03") => "css",
        Some("B02" | "B04") => "html",
        Some("B05") => "seo",
        Some("B06" | "B09" | "B10" | "B11" | "B12" | "B13") => "performance",
        Some("B17") => "webmcp",
        _ => "language",
    }
}
pub fn query(request: &Value) -> Result<Value, String> {
    let object = request
        .as_object()
        .ok_or("practices request must be an object")?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "family" | "codePrefix" | "limit" | "cursor"))
    {
        return Err(
            "unknown practices request key; use family, codePrefix, limit or cursor".into(),
        );
    }
    let string = |key: &str, default: &str| -> Result<String, String> {
        match request.get(key) {
            None => Ok(default.into()),
            Some(value) => value
                .as_str()
                .map(String::from)
                .ok_or_else(|| format!("{key} must be a string")),
        }
    };
    let selected = string("family", "all")?;
    if !matches!(
        selected.as_str(),
        "all" | "html" | "css" | "seo" | "performance" | "webmcp" | "language"
    ) {
        return Err("family must be all, html, css, seo, performance, webmcp or language".into());
    }
    let prefix = string("codePrefix", "B")?;
    if prefix.len() > 5
        || !prefix.starts_with('B')
        || !prefix[1..].bytes().all(|b| b.is_ascii_digit())
    {
        return Err("codePrefix must be B followed by up to four digits".into());
    }
    let number = |key: &str, default: u64| -> Result<usize, String> {
        match request.get(key) {
            None => Ok(default as usize),
            Some(value) => value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| format!("{key} must be a nonnegative integer")),
        }
    };
    let limit = number("limit", 30)?;
    let cursor = number("cursor", 0)?;
    if !(1..=100).contains(&limit) {
        return Err("limit must be between 1 and 100".into());
    }
    let mut rows = brain_lex::codes::CODES
        .iter()
        .filter(|row| {
            row.0.starts_with(&prefix) && (selected == "all" || family(row.0) == selected)
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.0);
    let total = rows.len();
    if cursor > total {
        return Err("cursor is beyond the filtered catalog; restart with cursor 0".into());
    }
    let end = cursor.saturating_add(limit).min(total);
    let rows = rows[cursor..end].iter().map(|row|json!({"code":row.0,"family":family(row.0),"defaultLevel":row.1.as_str(),"title":row.2,"why":row.3,"example":row.4,"adjustable":adjustable(row.0),"allowedLevels":if nudge(row.0) {vec!["off","info","warning"]} else if adjustable(row.0) {vec!["off","info","warning","error"]} else {vec![row.1.as_str()]}})).collect::<Vec<_>>();
    Ok(
        json!({"schemaVersion":1,"source":"brain_lex::codes::CODES","scope":"Compiler diagnostic catalog; family labels are navigation hints, not proof that every possible practice is checked. SEO built/live audit has its own B18xx catalog.","family":selected,"codePrefix":prefix,"total":total,"cursor":cursor,"limit":limit,"nextCursor":if end < total {Some(end)} else {None},"practices":rows,"agentOnly":true,"executesApplication":false}),
    )
}
