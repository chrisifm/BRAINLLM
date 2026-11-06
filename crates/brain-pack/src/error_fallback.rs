//! Only explicitly public generic error messages enter the server manifest; no request or exception text.
use brain_lex::Json;
use std::path::Path;

fn locale(value: &str) -> bool {
    !value.is_empty() && value.len() <= 35 && value.split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()))
}

pub(crate) fn metadata(root: &Path) -> String {
    let config = std::fs::read_to_string(root.join("brain.json")).ok().and_then(|text| Json::parse(&text).ok());
    let i18n = crate::read_i18n(root);
    let app = std::fs::read_to_string(root.join("app.brain")).unwrap_or_default();
    let app_lang = app.find("<html").and_then(|start| app[start..].find('>').map(|end| &app[start..start + end])).and_then(|tag| crate::cache::attr(tag, "lang"));
    let default = i18n.as_ref().map(|(default, _)| default.as_str())
        .or_else(|| config.as_ref().and_then(|config| config.get("lang")).and_then(Json::as_str))
        .or(app_lang.as_deref()).filter(|value| locale(value)).unwrap_or("en");
    let mut languages = i18n.as_ref().map(|(_, locales)| locales.clone()).unwrap_or_default();
    if !languages.iter().any(|lang| lang == default) { languages.push(default.into()); }
    let dictionaries = languages.iter().filter(|lang| locale(lang)).take(32).map(|lang| {
        let entries = crate::read_dict(root, lang).into_iter().filter_map(|(key, value)| {
            if !matches!(key.as_str(), "http.error.title" | "http.error.message" | "http.error.reference" | "http.error.home") { return None; }
            let value = Json::parse(&value).ok()?;
            let text = value.as_str()?;
            (!text.is_empty() && text.len() <= 2048 && !text.chars().any(|ch| ch.is_control() && ch != '\n')).then_some((key, value))
        }).collect();
        (lang.clone(), Json::Obj(entries))
    }).collect();
    Json::obj([("defaultLocale", Json::str(default)), ("locales", Json::Obj(dictionaries))]).stringify()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_metadata_uses_app_language_and_only_explicit_public_message_keys() {
        let root = std::env::temp_dir().join(format!("brain-error-metadata-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(root.join("i18n")).unwrap();
        std::fs::write(root.join("app.brain"), "<html lang=\"fr\"><body></body></html>").unwrap();
        std::fs::write(root.join("i18n/fr.json"), r#"{"http.error.title":"Localized heading","http.error.message":"<b>Plain public message</b>","private.note":"private-marker","http.error.home":17}"#).unwrap();
        let result = metadata(&root);
        assert!(result.contains("\"defaultLocale\":\"fr\"") && result.contains("Localized heading"), "{result}");
        assert!(!result.contains("private-marker") && !result.contains("http.error.home"));
        std::fs::write(root.join("brain.json"), r#"{"i18n":{"default":"de","locales":["de","fr"]}}"#).unwrap();
        assert!(metadata(&root).contains("\"defaultLocale\":\"de\""));
        std::fs::remove_dir_all(root).unwrap();
    }
}
