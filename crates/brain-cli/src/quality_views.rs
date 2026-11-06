//! Bounded per-view projections of existing static findings and actual build facts. No visual judgment or I/O.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const MAX_DOCUMENTS: usize = 512;
const MAX_VIEWS: usize = 128;
const MAX_EDGES: usize = 4096;
const MAX_REFERENCES: usize = 256;
const BUILD_SCOPE: &str = "Actual emitted artifact bytes, not measured transfer, browser rendering, field vitals or comparison with another build. Route-to-source matching is not guessed.";
fn route(s: &str) -> bool {
    s.starts_with('/')
        && !s.starts_with("//")
        && s.len() <= 240
        && !s.contains(['?', '#', '\\'])
        && !s.chars().any(char::is_control)
        && brain_check::scan_secrets(s).is_empty()
}
fn file(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.contains(':'))
        && brain_check::scan_secrets(value).is_empty()
}
fn code(value: &str) -> bool {
    value.len() == 5 && value.starts_with('B') && value[1..].bytes().all(|b| b.is_ascii_digit())
}
fn action(code: &str) -> Option<(&'static str, &'static str)> {
    match code {
        "B0221" => Some((
            "images",
            "Supply reviewed dimensions or an aspect ratio for the existing image so its layout space is reserved.",
        )),
        "B0401" | "B0413" => Some((
            "images",
            "Review the existing image's purpose and write meaningful alt text, or an empty alt for a decorative image.",
        )),
        "B0501" => Some((
            "page-content",
            "Author this view's title and description from its actual purpose; no new page or copy is generated.",
        )),
        "B1301" | "B1303" | "B1305" | "B1310" | "B1311" | "B1312" | "B1313" | "B1106" => Some((
            "performance",
            "Review the native diagnostic and verify behavior before changing concurrency, traversal, rendering or DOM access; measure any claimed improvement.",
        )),
        "B0201" | "B0202" => Some((
            "rendering",
            "Review the native rendering diagnostic with its source and verify updates in the affected view.",
        )),
        "QSTATE" => Some((
            "interaction-states",
            "Verify applicable permission, loading, error, empty, success and disabled scenarios with rendered interactions.",
        )),
        _ => None,
    }
}
fn edges(document: &Value) -> (Vec<String>, usize) {
    let mut result = BTreeSet::new();
    let mut omitted = 0;
    for value in document["contract"]["imports"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(name) = value["file"].as_str().filter(|name| file(name)) {
            result.insert(name.into());
        } else {
            omitted += 1;
        }
    }
    // These are already the manifest generator's bounded, revisioned dependency closure.
    for name in document["dependencies"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| name)
    {
        if file(name) {
            result.insert(name.clone());
        } else {
            omitted += 1;
        }
    }
    (result.into_iter().collect(), omitted)
}
/// Projects only supplied evidence; missing imports, dynamic edges, new files and browser behavior stay unknown.
pub fn project(documents: &[Value], items: &[Value]) -> Value {
    let mut graph = BTreeMap::<String, Vec<String>>::new();
    let mut views = BTreeSet::<String>::new();
    let mut incomplete = BTreeSet::<String>::new();
    let mut edge_count = 0;
    let mut skipped = 0;
    for document in documents.iter().take(MAX_DOCUMENTS) {
        let Some(name) = document["file"].as_str().filter(|name| file(name)) else {
            skipped += 1;
            continue;
        };
        if document["kind"] == "view" {
            views.insert(name.into());
        }
        if document["truncated"] == true
            || document["kind"] == "unknown"
            || document["contract"]["imports"]
                .as_array()
                .is_some_and(|imports| imports.iter().any(|entry| entry["file"].as_str().is_none()))
        {
            incomplete.insert(name.into());
        }
        let (all, unsafe_or_unknown) = edges(document);
        let remaining = MAX_EDGES.saturating_sub(edge_count);
        let included = all.len().min(remaining);
        if all.len() > included || unsafe_or_unknown > 0 {
            incomplete.insert(name.into());
        }
        edge_count += included;
        skipped += all.len() - included + unsafe_or_unknown;
        graph.insert(name.into(), all.into_iter().take(included).collect());
    }
    let total_views = views.len();
    let mut references = 0usize;
    let mut omitted = 0usize;
    let mut rows = vec![];
    for view in views.into_iter().take(MAX_VIEWS) {
        let mut reached = BTreeSet::new();
        let mut pending = vec![(view.clone(), 0usize)];
        let mut closure_complete = true;
        while let Some((name, depth)) = pending.pop() {
            if reached.contains(&name) {
                continue;
            }
            if reached.len() >= MAX_DOCUMENTS || depth > 32 {
                closure_complete = false;
                continue;
            }
            reached.insert(name.clone());
            if incomplete.contains(&name) {
                closure_complete = false;
            }
            if let Some(targets) = graph.get(&name) {
                for target in targets {
                    pending.push((target.clone(), depth + 1));
                }
            } else if name.ends_with(".brain") || name.ends_with(".jsbrain") {
                closure_complete = false;
            }
        }
        let mut findings = vec![];
        let mut recommendations = BTreeMap::<&str, BTreeSet<&str>>::new();
        let mut matching = 0usize;
        for item in items.iter().take(128) {
            let (Some(source), Some(item_code)) = (
                item["location"]["file"].as_str().filter(|name| file(name)),
                item["code"].as_str(),
            ) else {
                continue;
            };
            if !reached.contains(source) {
                continue;
            }
            let Some((category, guidance)) = action(item_code) else {
                continue;
            };
            matching += 1;
            recommendations
                .entry(category)
                .or_default()
                .insert(guidance);
            if references < MAX_REFERENCES && findings.len() < 16 {
                references += 1;
                let id = item["id"]
                    .as_str()
                    .filter(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()));
                findings.push(json!({"id":id,"code":item_code,"file":source,"line":item["location"]["line"].as_u64().filter(|n|*n>0&&*n<=u32::MAX as u64),"evidence":if item["evidence"]=="unmeasured" {"unmeasured"}else{"static"}}));
            } else {
                omitted += 1;
            }
        }
        rows.push(json!({"viewFile":view,"sourceCount":reached.len(),"closureComplete":closure_complete,"findingCount":matching,"findings":findings,"recommendations":recommendations.into_iter().map(|(category,guidance)|json!({"category":category,"actions":guidance})).collect::<Vec<_>>(),"visualAssessment":"not-run","renderedBehavior":"not-run"}));
    }
    json!({"schemaVersion":1,"views":rows,"totalViews":total_views,"omittedViews":total_views.saturating_sub(MAX_VIEWS),"findingReferences":references,"omittedFindingReferences":omitted,"coverage":{"inputDocuments":documents.len(),"graphDocuments":graph.len(),"graphEdges":edge_count,"skippedEdgesOrDocuments":skipped+documents.len().saturating_sub(MAX_DOCUMENTS),"dynamicImports":"not-inferred","renderedVisuals":"not-assessed","newImagesOrPages":"not-generated","completeAudit":false},"limits":{"documents":MAX_DOCUMENTS,"views":MAX_VIEWS,"edges":MAX_EDGES,"depth":32,"findingReferences":MAX_REFERENCES,"findingsPerView":16},"scope":"Affected views and review actions follow supplied source dependencies and actual findings. No screenshots are judged, no new pages/images are invented, and no performance comparison is inferred."})
}
/// Called from the existing verified build recorder: sanitizes actual emitted bytes per route, without a rebuild.
pub fn build_routes(build: &Value) -> Value {
    let input = build["report"].as_array();
    let total = input.map_or(0, Vec::len);
    let mut rows = vec![];
    let mut skipped = 0;
    for row in input.into_iter().flatten().take(MAX_VIEWS) {
        let Some(route) = row["route"].as_str().filter(|s| route(s)) else {
            skipped += 1;
            continue;
        };
        let mut bytes = serde_json::Map::new();
        for key in ["jsRaw", "jsMin", "jsGz", "css", "cssGz", "html", "htmlGz"] {
            if let Some(n) = row[key].as_u64().filter(|n| *n <= 9_007_199_254_740_991) {
                bytes.insert(key.into(), json!(n));
            }
        }
        rows.push(json!({"route":route,"emittedBytes":bytes}));
    }
    json!({"routes":rows,"totalRoutes":total,"omittedRoutes":total.saturating_sub(MAX_VIEWS)+skipped,"source":"verified-build-report","scope":BUILD_SCOPE})
}
pub fn valid_build_routes(value: &Value) -> bool {
    value.as_object().is_some_and(|o| {
        o.len() == 5
            && o.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "routes" | "totalRoutes" | "omittedRoutes" | "source" | "scope"
                )
            })
    }) && value["source"] == "verified-build-report"
        && value["scope"] == BUILD_SCOPE
        && value["routes"].as_array().is_some_and(|rows| {
            rows.len() <= MAX_VIEWS
                && rows.iter().all(|row| {
                    row.as_object().is_some_and(|o| {
                        o.len() == 2 && o.contains_key("route") && o.contains_key("emittedBytes")
                    }) && row["route"].as_str().is_some_and(route)
                        && row["emittedBytes"].as_object().is_some_and(|o| {
                            o.len() <= 7
                                && o.iter().all(|(key, value)| {
                                    ["jsRaw", "jsMin", "jsGz", "css", "cssGz", "html", "htmlGz"]
                                        .contains(&key.as_str())
                                        && value
                                            .as_u64()
                                            .is_some_and(|n| n <= 9_007_199_254_740_991)
                                })
                        })
                })
                && value["totalRoutes"].as_u64().is_some_and(|total| {
                    value["omittedRoutes"].as_u64().is_some_and(|omitted| {
                        omitted.checked_add(rows.len() as u64) == Some(total)
                    })
                })
        })
}
/// Enrich the existing save-impact feed using its actual affected views and new native diagnostics only.
pub fn impact(record: &Value) -> Value {
    let mut views = BTreeSet::new();
    let mut recommendations = BTreeSet::new();
    let mut codes = BTreeSet::new();
    if let Some(view) = record["file"]
        .as_str()
        .filter(|s| s.starts_with("views/") && file(s))
    {
        views.insert(view.to_string());
    }
    for key in ["bytes", "seo"] {
        for row in record[key].as_array().into_iter().flatten().take(MAX_VIEWS) {
            if let Some(view) = row["view"].as_str().filter(|s| file(s)) {
                views.insert(view.to_string());
            }
        }
    }
    for row in record["new"].as_array().into_iter().flatten().take(64) {
        if let Some(view) = row["file"]
            .as_str()
            .filter(|s| s.starts_with("views/") && file(s))
        {
            views.insert(view.to_string());
        }
        if let Some(value) = row["code"].as_str().filter(|s| code(s)) {
            if let Some((category, guidance)) = action(value) {
                codes.insert(value.to_string());
                recommendations.insert((category, guidance));
            }
        }
    }
    json!({"affectedViews":views.into_iter().take(MAX_VIEWS).collect::<Vec<_>>(),"newFindingCodes":codes,"recommendations":recommendations.into_iter().map(|(category,guidance)|json!({"category":category,"action":guidance})).collect::<Vec<_>>(),"visualAssessment":"not-run","scope":"Recommendations use this save's existing impact/diagnostic evidence. No new watcher, build, browser execution or external service is started."})
}
pub fn enrich_impact(raw: &str) -> String {
    if raw.len() > 1024 * 1024 {
        return raw.into();
    }
    let Ok(mut value) = serde_json::from_str::<Value>(raw) else {
        return raw.into();
    };
    if value.is_object() {
        value["viewAdvice"] = impact(&value);
        value.to_string()
    } else {
        raw.into()
    }
}
