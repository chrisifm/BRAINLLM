//! Read-only local setup plan. This command neither contacts Google nor creates or changes remote resources.
use serde_json::{Value, json};
use std::{path::Path, process::ExitCode};
const USAGE: &str = "usage: brain google setup [--env production] [--json]; prints a local review plan only, never --apply";
#[derive(Debug, PartialEq)]
struct Options {
    environment: String,
    json: bool,
}
fn options(args: &[String]) -> Result<Options, &'static str> {
    if args.first().map(String::as_str) != Some("setup") {
        return Err(USAGE);
    }
    let mut out = Options {
        environment: "production".into(),
        json: false,
    };
    let mut seen = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--json" if !out.json => out.json = true,
            "--env" if !seen => {
                i += 1;
                let value = args.get(i).ok_or(USAGE)?;
                if value.is_empty()
                    || value.len() > 64
                    || !value.as_bytes()[0].is_ascii_alphabetic()
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return Err(USAGE);
                }
                out.environment = value.clone();
                seen = true
            }
            _ => return Err(USAGE),
        }
        i += 1;
    }
    Ok(out)
}
fn failure(code: &str, message: &str) -> Value {
    json!({"schemaVersion":1,"ok":false,"readOnly":true,"applied":false,"error":{"code":code,"message":message}})
}
pub fn plan(root: &Path, environment: &str) -> Result<Value, Value> {
    let cfg=brain_seo::Config::load(root).map_err(|_|failure("GOOGLE_SETUP_CONFIG","Cannot read valid brain.json configuration; fix: run brain check and correct the Google configuration without logging its values"))?.with_env(root,environment);
    let bridge = &cfg.google_bridge;
    let legacy =
        !bridge.enabled && (cfg.google.analytics.is_some() || cfg.google.tag_manager.is_some());
    let mappings: Vec<_> = bridge
        .events
        .iter()
        .map(|(source, event)| json!({"source":source,"event":event}))
        .collect();
    let funnels: Vec<_> = bridge
        .funnels
        .iter()
        .map(|(name, steps)| json!({"name":name,"steps":steps,"remoteResourceCreated":false}))
        .collect();
    let csp: Vec<_> = if bridge.active() {
        brain_seo::google::CSP
            .iter()
            .map(|(directive, source)| json!({"directive":directive,"source":source}))
            .collect()
    } else {
        Vec::new()
    };
    let mut checks = vec![
        "Review the site's consent wording and applicable policy requirements; the optional banner is a starting control, not a universal compliance guarantee.",
        "Test accept, deny, revoke, keyboard navigation and returning visitors before deployment.",
        "A form submit mapping records an attempt, not backend success. Dispatch a declared brain:track event after confirmed success for conversion events.",
        "The basic bridge never replays pre-consent events. Only declared event names are forwarded; application properties, field values, URLs, titles and referrers are omitted or redacted.",
    ];
    if bridge.destination.analytics.is_some() {
        checks.push("In the GA4 web stream, review or disable Enhanced Measurement, automatic form measurement and configured events that could collect page data independently of this bridge. Default page views are disabled by this integration.");
    }
    if bridge.destination.tag_manager.is_some() {
        checks.push("In the reviewed GTM container, configure custom-event triggers for the declared event names, consent checks and opt-out triggers for every tag. An already-executed container can run arbitrary third-party code; this bridge cannot inspect or retract it.");
    }
    if legacy {
        checks.push("Legacy Google tags remain configured and may load before consent; set google.mode to basic to opt in to the gated bridge. This plan does not change their existing behavior.");
    }
    if !bridge.active() {
        checks.push("No destination is enabled in the selected environment. Configure one explicit basic-mode GA4 or GTM destination; environment maps never inherit top-level or another environment's ids.");
    }
    if bridge.events.is_empty() {
        checks.push("Declare google.events mappings to opt in each data-track or brain:track event; undeclared names are never forwarded.");
    }
    Ok(
        json!({"schemaVersion":1,"ok":true,"readOnly":true,"applied":false,"status":"local-plan-only","environment":environment,"bridgeEnabled":bridge.enabled,"destinationConfigured":bridge.active(),"legacyTagsUnchanged":!bridge.enabled,"legacyTagsMayLoadBeforeConsent":legacy,"destination":{"analytics":bridge.destination.analytics,"tagManager":bridge.destination.tag_manager},"mode":if bridge.enabled {Some("basic")} else {None},"consent":{"required":true,"banner":bridge.banner,"initialState":"denied","deniedEventsQueued":if bridge.enabled {Some(false)} else {None},"tagsLoadBeforeConsent":legacy},"events":mappings,"funnels":funnels,"csp":csp,"operatorChecks":checks,"remoteActions":[],"resourcesCreated":false,"verification":{"networkRequests":0,"remoteDeliveryVerified":false,"containerAudited":false},"references":["https://developers.google.com/tag-platform/security/concepts/consent-mode","https://developers.google.com/tag-platform/security/guides/consent","https://developers.google.com/tag-platform/tag-manager/datalayer","https://developers.google.com/analytics/devguides/collection/ga4/reference/config"]}),
    )
}
fn emit(value: &Value, json_output: bool) {
    if json_output {
        println!("{value}")
    } else if let Some(error) = value.get("error") {
        println!("{}: {}", error["code"], error["message"])
    } else {
        println!(
            "Google setup: local review plan only; no network requests or resources created.\nEnvironment: {}\nBasic bridge enabled: {}\nDestination configured: {}",
            value["environment"], value["bridgeEnabled"], value["destinationConfigured"]
        );
        for item in value["operatorChecks"].as_array().into_iter().flatten() {
            println!("- {}", item.as_str().unwrap_or("Review configuration"));
        }
    }
}
pub fn entry(args: &[String]) -> ExitCode {
    let json_output = args.iter().any(|a| a == "--json");
    let opt = match options(args) {
        Ok(v) => v,
        Err(message) => {
            emit(&failure("GOOGLE_SETUP_ARGUMENTS", message), json_output);
            return ExitCode::from(2);
        }
    };
    let root = match crate::app_root() {
        Ok(v) => v,
        Err(_) => {
            emit(
                &failure(
                    "GOOGLE_SETUP_APP_REQUIRED",
                    "Run brain google setup inside the app to review its own Google configuration",
                ),
                opt.json,
            );
            return ExitCode::from(2);
        }
    };
    match plan(&root, &opt.environment) {
        Ok(v) => {
            emit(&v, opt.json);
            ExitCode::SUCCESS
        }
        Err(v) => {
            emit(&v, opt.json);
            ExitCode::from(2)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|v| v.to_string()).collect()
    }
    #[test]
    fn setup_is_a_strict_read_only_plan() {
        assert_eq!(
            options(&args(&["setup"])).unwrap().environment,
            "production"
        );
        for s in [
            &["setup", "--apply"][..],
            &["setup", "--env", "../other"],
            &["setup", "--json", "--json"],
            &["setup", "--env", "production", "--env", "development"],
            &["create"],
        ] {
            assert!(options(&args(s)).is_err());
        }
    }
    #[test]
    fn selected_app_environment_is_the_only_destination_and_no_files_are_written() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "brain-google-setup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let text = r#"{"google":{"mode":"basic","environments":{"production":{"analytics":"G-PROD"},"development":{"tagManager":"GTM-DEV"}},"events":{"save":"save_item","submit":"generate_lead"},"funnels":{"flow":["save_item","generate_lead"]}}}"#;
        std::fs::write(root.join("brain.json"), text).unwrap();
        let result = plan(&root, "development");
        let untouched = std::fs::read_to_string(root.join("brain.json")).unwrap() == text
            && std::fs::read_dir(&root).unwrap().count() == 1;
        std::fs::remove_dir_all(&root).unwrap();
        assert!(untouched);
        let result = result.unwrap();
        let output = result.to_string();
        assert!(output.contains("GTM-DEV"));
        assert!(!output.contains("G-PROD"));
        assert_eq!(result["verification"]["networkRequests"], 0);
        assert_eq!(result["resourcesCreated"], false);
        assert_eq!(result["remoteActions"], json!([]));
    }
    #[test]
    fn duplicate_basic_destinations_are_rejected_without_echoing_ids_or_writing_files() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "brain-google-duplicates-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let text = r#"{"google":{"mode":"basic","environments":{"production":{"analytics":"G-FIRST","analytics":"G-SECOND"}}}}"#;
        std::fs::write(root.join("brain.json"), text).unwrap();
        let result = plan(&root, "production");
        let unchanged = std::fs::read_to_string(root.join("brain.json")).unwrap() == text
            && std::fs::read_dir(&root).unwrap().count() == 1;
        std::fs::remove_dir_all(&root).unwrap();
        assert!(unchanged);
        let error = result.unwrap_err();
        assert_eq!(error["error"]["code"], "GOOGLE_SETUP_CONFIG");
        let error = error.to_string();
        assert!(!error.contains("G-FIRST") && !error.contains("G-SECOND"));
    }
    #[test]
    fn duplicate_outer_google_selector_cannot_hide_basic_mode_in_either_order() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "brain-google-selector-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut results = Vec::new();
        for text in [
            r#"{"google":{"analytics":"G-FIRST"},"google":{"mode":"basic","analytics":"G-SECOND"}}"#,
            r#"{"google":{"mode":"basic","analytics":"G-FIRST"},"google":{"analytics":"G-SECOND"}}"#,
        ] {
            std::fs::write(root.join("brain.json"), text).unwrap();
            let result = plan(&root, "production");
            let unchanged = std::fs::read_to_string(root.join("brain.json")).unwrap() == text;
            results.push((result, unchanged));
        }
        std::fs::remove_dir_all(&root).unwrap();
        for (result, unchanged) in results {
            assert!(unchanged);
            let error = result.unwrap_err();
            assert_eq!(error["error"]["code"], "GOOGLE_SETUP_CONFIG");
            assert!(
                !error.to_string().contains("G-FIRST") && !error.to_string().contains("G-SECOND")
            );
        }
    }
}
