#[path = "../src/quality_views.rs"]
mod quality_views;
use serde_json::{Value, json};

fn doc(file: &str, kind: &str, imports: &[&str]) -> Value {
    json!({"file":file,"kind":kind,"contract":{"imports":imports.iter().map(|name|json!({"file":name})).collect::<Vec<_>>()}})
}
fn finding(code: &str, file: &str) -> Value {
    json!({"code":code,"location":{"file":file,"line":4},"evidence":"static","title":"private-source-value","fix":"private-fix-value"})
}
#[test]
fn actual_findings_map_through_cycles_to_affected_views_without_source_or_visual_claims() {
    let documents = vec![
        doc("views/home.brain", "view", &["comp/photo.brain"]),
        doc("views/other.brain", "view", &[]),
        doc("comp/photo.brain", "component", &["comp/cycle.brain"]),
        doc("comp/cycle.brain", "component", &["comp/photo.brain"]),
    ];
    let result = quality_views::project(
        &documents,
        &[
            finding("B0401", "comp/photo.brain"),
            finding("B0501", "views/home.brain"),
        ],
    );
    let home = &result["views"][0];
    assert_eq!(home["viewFile"], "views/home.brain");
    assert_eq!(home["findingCount"], 2);
    assert_eq!(home["closureComplete"], true);
    assert_eq!(result["views"][1]["findingCount"], 0);
    assert_eq!(home["visualAssessment"], "not-run");
    assert!(!result.to_string().contains("private-"));
    let unknown = quality_views::project(
        &[
            json!({"file":"views/unknown.brain","kind":"view","contract":{"imports":[{"file":null}]}}),
        ],
        &[],
    );
    assert_eq!(unknown["views"][0]["closureComplete"], false);
    assert!(
        home["recommendations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["category"] == "images")
    );
    assert!(
        home["recommendations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["category"] == "page-content")
    );
}
#[test]
fn depth_edges_and_reference_output_are_bounded_and_omissions_are_explicit() {
    let mut documents = vec![doc("views/home.brain", "view", &["comp/0.brain"])];
    for i in 0..40 {
        documents.push(doc(
            &format!("comp/{i}.brain"),
            "component",
            &[&format!("comp/{}.brain", i + 1)],
        ));
    }
    let deep = quality_views::project(&documents, &[]);
    assert_eq!(deep["views"][0]["closureComplete"], false);
    let mut many = vec![];
    for i in 0..150 {
        many.push(doc(
            &format!("views/view-{i}.brain"),
            "view",
            &["comp/shared.brain"],
        ));
    }
    many.push(doc("comp/shared.brain", "component", &[]));
    let items = (0..100)
        .map(|_| finding("B0401", "comp/shared.brain"))
        .collect::<Vec<_>>();
    let report = quality_views::project(&many, &items);
    assert_eq!(report["views"].as_array().unwrap().len(), 128);
    assert_eq!(report["omittedViews"], 22);
    assert!(report["findingReferences"].as_u64().unwrap() <= 256);
    assert!(report["omittedFindingReferences"].as_u64().unwrap() > 0);
}

#[test]
fn clipped_or_filtered_edges_make_reachable_view_closures_incomplete() {
    let targets = (0..150)
        .map(|i| format!("comp/{i:03}.brain"))
        .collect::<Vec<_>>();
    let mut documents = vec![];
    for file in &targets {
        documents.push(doc(
            file,
            "component",
            &targets.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
    }
    documents.push(doc("views/home.brain", "view", &["comp/000.brain"]));
    let clipped = quality_views::project(&documents, &[]);
    assert_eq!(clipped["coverage"]["graphEdges"], 4096);
    assert!(
        clipped["coverage"]["skippedEdgesOrDocuments"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(clipped["views"][0]["closureComplete"], false);
    let mut view = doc("views/ancestor.brain", "view", &["comp/000.brain"]);
    view["file"] = json!("views/000-first.brain");
    documents.insert(0, view);
    let indirect = quality_views::project(&documents, &[]);
    assert_eq!(indirect["views"][0]["closureComplete"], false);
    for document in [
        json!({"file":"views/home.brain","kind":"view","contract":{"imports":[{"file":"../private.brain"}]}}),
        json!({"file":"views/home.brain","kind":"view","dependencies":{"../private.brain":"revision"}}),
    ] {
        assert_eq!(
            quality_views::project(&[document], &[])["views"][0]["closureComplete"],
            false
        );
    }
}
#[test]
fn build_records_only_actual_safe_route_bytes_and_never_infers_transfer_or_rendering() {
    let build = json!({"report":[{"route":"/catalog","jsGz":123,"htmlGz":45,"private":"private-value"},{"route":"/private?token=value","jsGz":1},{"route":"//example.com/x","jsGz":2}],"warnings":["private-warning-value"]});
    let result = quality_views::build_routes(&build);
    assert!(quality_views::valid_build_routes(&result));
    let mut invalid = result.clone();
    invalid["routes"][0]["raw"] = json!("unrecognized");
    assert!(!quality_views::valid_build_routes(&invalid));
    let mut invalid = result.clone();
    invalid["scope"] = json!("unrecognized");
    assert!(!quality_views::valid_build_routes(&invalid));
    assert_eq!(result["routes"].as_array().unwrap().len(), 1);
    assert_eq!(result["omittedRoutes"], 2);
    assert_eq!(result["routes"][0]["emittedBytes"]["jsGz"], 123);
    assert!(!result.to_string().contains("private-"));
    assert!(result["scope"].as_str().unwrap().contains("not measured"));
}
#[test]
fn save_feed_reuses_existing_impact_views_and_only_real_known_diagnostic_actions() {
    let result = quality_views::impact(
        &json!({"bytes":[{"view":"views/home.brain","delta":1}],"seo":[{"view":"views/home.brain"}],"new":[{"code":"B0401","msg":"private-content"},{"code":"B9999"}]}),
    );
    assert_eq!(result["affectedViews"], json!(["views/home.brain"]));
    assert_eq!(result["newFindingCodes"], json!(["B0401"]));
    assert_eq!(result["recommendations"][0]["category"], "images");
    assert_eq!(result["visualAssessment"], "not-run");
    assert!(!result.to_string().contains("private-content"));
    let enriched: Value = serde_json::from_str(&quality_views::enrich_impact(
        "{\"file\":\"views/changed.brain\",\"seq\":3,\"new\":[]}",
    ))
    .unwrap();
    assert_eq!(enriched["seq"], 3);
    assert_eq!(
        enriched["viewAdvice"]["affectedViews"],
        json!(["views/changed.brain"])
    );
    assert_eq!(quality_views::enrich_impact("invalid-json"), "invalid-json");
}
