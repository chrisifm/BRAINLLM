//! A failed build as data: the phase that stopped it and the diagnostics behind it, with the same fields as `brain check`.
use crate::compiler::Diag;
use brain_mcp::Json;
use std::path::Path;

#[derive(Debug)]
pub(crate) struct BuildError {
    pub message: String,
    pub phase: &'static str,
    pub diagnostics: Vec<Diag>,
}

impl From<String> for BuildError {
    fn from(message: String) -> Self {
        Self { message, phase: "build", diagnostics: Vec::new() }
    }
}

impl From<BuildError> for String {
    fn from(error: BuildError) -> Self {
        error.message
    }
}

impl BuildError {
    pub fn with(phase: &'static str, diagnostics: Vec<Diag>, summary: &str) -> Self {
        let lines: Vec<String> = diagnostics.iter().filter(|d| !d.is_warning()).map(line).collect();
        Self { message: format!("{}\n{summary}", lines.join("\n")), phase, diagnostics }
    }

    /// `error` and `fix` as every command reports them, plus the typed failure. A location that is not a file of the app
    /// (a module the compiler ships) is marked `generated`, and its edits are dropped because they could not be applied.
    pub fn json(&self, root: Option<&Path>) -> String {
        let (error, fix) = self.message.split_once("; fix: ").unwrap_or((&self.message, ""));
        let mut items: Vec<Json> = match Json::parse(&Diag::list_to_json(&self.diagnostics)) {
            Ok(Json::Arr(items)) => items,
            _ => Vec::new(),
        };
        for (item, diag) in items.iter_mut().zip(&self.diagnostics) {
            if let (Json::Obj(fields), Some(root)) = (item, root)
                && !root.join(&diag.file).is_file()
            {
                fields.retain(|(key, _)| key != "edits");
                fields.push(("edits".into(), Json::Arr(Vec::new())));
                fields.push(("generated".into(), Json::Bool(true)));
            }
        }
        Json::obj([("error", Json::str(error)), ("fix", Json::str(fix)), ("phase", Json::str(self.phase)), ("diagnostics", Json::Arr(items))]).stringify()
    }
}

pub(crate) fn line(d: &Diag) -> String {
    format!("{} {}:{}:{} {}; fix: {}", d.code, d.file, d.line, d.col, d.msg, d.fix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(file: &str) -> Diag {
        let mut d = Diag::new("B0111", file, Default::default(), "`x` is not declared", "declare it");
        d.edits = vec![Default::default()];
        d
    }

    #[test]
    fn a_location_outside_the_app_is_marked_and_offers_no_edit() {
        let root = std::env::temp_dir().join(format!("brain-build-error-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("mine.brain"), "").unwrap();
        let error = BuildError::with("compile", vec![diag("mine.brain"), diag("ui/shipped.brain")], "stopped");
        let body: serde_json::Value = serde_json::from_str(&error.json(Some(&root))).unwrap();
        assert!(body["diagnostics"][0].get("generated").is_none(), "{body}");
        assert_eq!(body["diagnostics"][0]["edits"].as_array().unwrap().len(), 1, "{body}");
        assert_eq!(body["diagnostics"][1]["generated"], true, "{body}");
        assert_eq!(body["diagnostics"][1]["edits"], serde_json::json!([]), "{body}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
