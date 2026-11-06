//! Explicit basic-mode Google bridge. Legacy Google config stays separate until an opt-in field is present.
use brain_lex::Json;
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Destination {
    pub analytics: Option<String>,
    pub tag_manager: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bridge {
    pub enabled: bool,
    pub destination: Destination,
    pub environments: Option<Vec<(String, Destination)>>,
    pub events: Vec<(String, String)>,
    pub funnels: Vec<(String, Vec<String>)>,
    pub banner: bool,
}
fn problem() -> String {
    "brain.json: invalid Google basic bridge; fix: use mode basic, one GA4 or GTM id, explicit environment ids, declared event mappings and bounded named funnel steps".into()
}
fn object(j: &Json) -> Result<&[(String, Json)], String> {
    if let Json::Obj(rows) = j {
        if rows.len() > 100 {
            return Err(problem());
        }
        let mut seen = std::collections::HashSet::with_capacity(rows.len());
        if rows.iter().any(|(key, _)| !seen.insert(key)) {
            return Err("brain.json: duplicate keys in Google basic configuration; fix: keep each object key exactly once, including destination ids".into());
        }
        Ok(rows)
    } else {
        Err(problem())
    }
}
fn name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !["ga_", "google_", "firebase_"]
            .iter()
            .any(|p| s.to_ascii_lowercase().starts_with(p))
}
fn source(s: &str) -> bool {
    !["__proto__", "constructor", "prototype"].contains(&s)
        && !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b':' || b == b'-')
}
fn environment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn id(j: &Json, key: &str, prefix: &str) -> Result<Option<String>, String> {
    let Some(v) = j.get(key) else { return Ok(None) };
    if matches!(v, Json::Null) {
        return Ok(None);
    }
    let text = v.as_str().ok_or_else(problem)?;
    let suffix = text.strip_prefix(prefix).ok_or_else(problem)?;
    if suffix.is_empty()
        || suffix.len() > 32
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err(problem());
    }
    Ok(Some(text.into()))
}
fn destination(j: &Json) -> Result<Destination, String> {
    object(j)?;
    let d = Destination {
        analytics: id(j, "analytics", "G-")?,
        tag_manager: id(j, "tagManager", "GTM-")?,
    };
    if d.analytics.is_some() && d.tag_manager.is_some() {
        return Err(problem());
    }
    Ok(d)
}
fn opted_in(j: &Json) -> bool {
    matches!(j, Json::Obj(rows) if rows.iter().any(|(key, _)| matches!(key.as_str(), "mode" | "environments" | "events" | "funnels" | "banner")))
}
/// Validate the outer selector before Config uses get("google"). Only new basic contracts opt in to strictness.
pub fn validate_config_google(root: &Json) -> Result<(), String> {
    if let Json::Obj(rows) = root {
        let google: Vec<_> = rows
            .iter()
            .filter(|(key, _)| key == "google")
            .map(|(_, value)| value)
            .collect();
        if google.len() > 1 && google.iter().any(|value| opted_in(value)) {
            return Err("brain.json: duplicate google objects with basic opt-in; fix: keep exactly one google object".into());
        }
    }
    Ok(())
}
/// Pass the `google` object, not the full brain.json. An untouched legacy config returns a disabled bridge.
pub fn parse(j: &Json) -> Result<Bridge, String> {
    let Json::Obj(_) = j else {
        return Err(problem());
    };
    let enabled = opted_in(j);
    if !enabled {
        return Ok(Bridge::default());
    }
    object(j)?; // duplicate validation belongs to the opted-in basic contract, preserving legacy behavior
    if j.get("mode").is_some_and(|v| v.as_str() != Some("basic")) {
        return Err(problem());
    }
    let mut out = Bridge {
        enabled: true,
        destination: destination(j)?,
        ..Bridge::default()
    };
    if let Some(v) = j.get("banner") {
        out.banner = match v {
            Json::Bool(value) => *value,
            _ => return Err(problem()),
        };
    }
    if let Some(v) = j.get("environments") {
        let rows = object(v)?;
        if rows.is_empty() || rows.len() > 20 {
            return Err(problem());
        }
        let mut environments = Vec::new();
        for (key, v) in rows {
            if !environment(key) || environments.iter().any(|(k, _)| k == key) {
                return Err(problem());
            }
            environments.push((key.clone(), destination(v)?));
        }
        out.environments = Some(environments);
        // A declared environment map is authoritative: top-level ids never leak into an undeclared environment.
        out.destination = Destination::default();
    }
    if let Some(v) = j.get("events") {
        let rows = object(v)?;
        if rows.len() > 100 {
            return Err(problem());
        }
        for (key, v) in rows {
            let value = v.as_str().ok_or_else(problem)?;
            if !source(key) || !name(value) || out.events.iter().any(|(k, _)| k == key) {
                return Err(problem());
            }
            out.events.push((key.clone(), value.into()));
        }
    }
    if let Some(v) = j.get("funnels") {
        let rows = object(v)?;
        if rows.len() > 20 {
            return Err(problem());
        }
        for (key, v) in rows {
            let Json::Arr(steps) = v else {
                return Err(problem());
            };
            if !name(key)
                || steps.len() < 2
                || steps.len() > 20
                || out.funnels.iter().any(|(k, _)| k == key)
            {
                return Err(problem());
            }
            let mut names = Vec::new();
            for step in steps {
                let value = step.as_str().ok_or_else(problem)?;
                if !name(value)
                    || !out.events.iter().any(|(_, v)| v == value)
                    || names.iter().any(|n| n == value)
                {
                    return Err(problem());
                }
                names.push(value.to_string());
            }
            out.funnels.push((key.clone(), names));
        }
    }
    Ok(out)
}
impl Bridge {
    /// Selection can be repeated safely after Config::load's production default. No fallback across environments.
    pub fn select(mut self, environment: &str) -> Self {
        if let Some(rows) = &self.environments {
            self.destination = rows
                .iter()
                .find(|(key, _)| key == environment)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
        }
        self
    }
    pub fn json(&self) -> Json {
        let mut rows = vec![
            ("mode".into(), Json::str("basic")),
            (
                "events".into(),
                Json::Obj(
                    self.events
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::str(v)))
                        .collect(),
                ),
            ),
            (
                "funnels".into(),
                Json::Obj(
                    self.funnels
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::Arr(v.iter().map(Json::str).collect())))
                        .collect(),
                ),
            ),
            ("banner".into(), Json::Bool(self.banner)),
        ];
        if let Some(id) = &self.destination.analytics {
            rows.push(("analytics".into(), Json::str(id)));
        }
        if let Some(id) = &self.destination.tag_manager {
            rows.push(("tagManager".into(), Json::str(id)));
        }
        Json::Obj(rows)
    }
    pub fn active(&self) -> bool {
        self.enabled
            && (self.destination.analytics.is_some() || self.destination.tag_manager.is_some())
    }
}
/// Same-origin ESM, no inline code or inline event handlers, usable in SSR and static HTML alike.
/// The Google loader is created at runtime only after an actual analytics consent decision.
pub fn bootstrap(config: &Bridge) -> Option<String> {
    if !config.active() {
        return None;
    }
    let mut text = String::from(include_str!("../../../runtime/brain-consent.js"));
    text.push('\n');
    text.push_str(include_str!("../../../runtime/brain-google.js"));
    text.push_str("\nconst __brainGoogleBridge=startGoogle(");
    text.push_str(&config.json().stringify());
    text.push_str(");\n");
    if config.banner {
        text.push_str("consentBanner();\n");
    }
    Some(text)
}
/// These are permissions, not requests. No external script is present in the initial head in basic mode.
pub const CSP: &[(&str, &str)] = &[
    ("script-src", "https://www.googletagmanager.com"),
    ("connect-src", "https://*.google-analytics.com"),
    ("connect-src", "https://*.analytics.google.com"),
    ("connect-src", "https://*.googletagmanager.com"),
    ("img-src", "https://*.google-analytics.com"),
    ("img-src", "https://*.googletagmanager.com"),
];
#[cfg(test)]
mod tests {
    use super::*;
    fn cfg(s: &str) -> Result<Bridge, String> {
        parse(&Json::parse(s).unwrap())
    }
    #[test]
    fn legacy_is_unchanged_and_environment_selection_never_falls_back() {
        assert!(!cfg(r#"{"analytics":"G-ABC123"}"#).unwrap().enabled);
        let c=cfg(r#"{"mode":"basic","analytics":"G-TOP","environments":{"production":{"analytics":"G-PROD"},"development":{"tagManager":"GTM-DEV"}},"events":{"save":"save_item"}}"#).unwrap();
        let production = c.clone().select("production");
        assert_eq!(production.destination.analytics.as_deref(), Some("G-PROD"));
        let dev = production.clone().select("development");
        assert_eq!(dev.destination.tag_manager.as_deref(), Some("GTM-DEV"));
        assert!(!dev.json().stringify().contains("G-PROD"));
        assert!(!c.select("preview").active());
    }
    #[test]
    fn strict_configuration_rejects_unsafe_ids_names_ambiguous_destinations_and_unmapped_steps() {
        for s in [
            r#"{"mode":"advanced"}"#,
            r#"{"mode":"basic","analytics":"G-ABC","tagManager":"GTM-ABC"}"#,
            r#"{"events":{"save":"person@example.com"}}"#,
            r#"{"events":{"save":"ga_reserved"}}"#,
            r#"{"environments":{"../production":{"analytics":"G-ABC"}}}"#,
            r#"{"banner":"true"}"#,
            r#"{"events":{"save":"save_item"},"funnels":{"flow":["save_item","unknown"]}}"#,
        ] {
            assert!(cfg(s).is_err(), "{s}");
        }
    }
    #[test]
    fn same_origin_bootstrap_contains_only_selected_ids_and_no_eager_google_loader() {
        let c=cfg(r#"{"mode":"basic","environments":{"production":{"analytics":"G-PROD"},"development":{"analytics":"G-DEV"}},"banner":true,"events":{"save":"save_item","submit":"generate_lead"},"funnels":{"flow":["save_item","generate_lead"]}}"#).unwrap().select("development");
        let js = bootstrap(&c).unwrap();
        assert!(js.contains("startGoogle({"));
        assert!(js.contains("consentBanner();"));
        assert!(js.contains("G-DEV"));
        assert!(!js.contains("G-PROD"));
        assert!(bootstrap(&Bridge::default()).is_none());
    }
    #[test]
    fn declared_metadata_and_bootstrap_have_a_finite_byte_budget() {
        let events: Vec<_> = (0..100)
            .map(|i| {
                (
                    format!("source_{i:057}"),
                    Json::str(format!("event_{i:034}")),
                )
            })
            .collect();
        let steps: Vec<_> = events.iter().take(20).map(|(_, v)| v.clone()).collect();
        let funnels = (0..20)
            .map(|i| (format!("flow_{i:035}"), Json::Arr(steps.clone())))
            .collect();
        let j = Json::obj([
            ("mode", Json::str("basic")),
            ("analytics", Json::str("G-ABC123")),
            ("events", Json::Obj(events)),
            ("funnels", Json::Obj(funnels)),
        ]);
        let bridge = parse(&j).unwrap();
        let script = bootstrap(&bridge).unwrap();
        assert!(script.len() < 65536, "bootstrap bytes: {}", script.len());
        let bad = Json::obj([(
            "events",
            Json::obj([("private", Json::str("x".repeat(100000)))]),
        )]);
        let error = parse(&bad).unwrap_err();
        assert!(error.len() < 300);
        assert!(!error.contains(&"x".repeat(100)));
    }
    #[test]
    fn every_basic_object_rejects_duplicate_keys_while_legacy_remains_unchanged() {
        for source in [
            r#"{"mode":"basic","mode":"basic"}"#,
            r#"{"mode":"basic","analytics":"G-FIRST","analytics":"G-SECOND"}"#,
            r#"{"mode":"basic","environments":{"production":{"analytics":"G-FIRST","analytics":"G-SECOND"}}}"#,
            r#"{"mode":"basic","environments":{"production":{},"production":{}}}"#,
            r#"{"mode":"basic","events":{"save":"save_item","save":"save_item"}}"#,
            r#"{"mode":"basic","events":{"save":"save_item","submit":"generate_lead"},"funnels":{"flow":["save_item","generate_lead"],"flow":["save_item","generate_lead"]}}"#,
            r#"{"mode":"basic","environments":{"production":{"other":true,"other":false}}}"#,
        ] {
            let error = cfg(source).unwrap_err();
            assert!(error.contains("duplicate"));
            assert!(!error.contains("G-FIRST") && !error.contains("G-SECOND"));
        }
        assert!(
            !cfg(r#"{"analytics":"G-FIRST","analytics":"G-SECOND"}"#)
                .unwrap()
                .enabled
        );
    }
    #[test]
    fn outer_google_selector_is_checked_before_any_first_value_lookup() {
        for text in [
            r#"{"google":{"analytics":"G-FIRST"},"google":{"mode":"basic","analytics":"G-SECOND"}}"#,
            r#"{"google":{"mode":"basic","analytics":"G-FIRST"},"google":{"analytics":"G-SECOND"}}"#,
        ] {
            let root = Json::parse(text).unwrap();
            let error = validate_config_google(&root).unwrap_err();
            assert!(error.contains("duplicate google"));
            assert!(!error.contains("G-FIRST") && !error.contains("G-SECOND"));
        }
        validate_config_google(
            &Json::parse(r#"{"google":{"analytics":"G-FIRST"},"google":{"analytics":"G-SECOND"}}"#)
                .unwrap(),
        )
        .unwrap();
    }
}
