//! Optional project-local agent hooks. No personal client settings are changed.
use serde_json::{Value, json};
use std::{fs, path::Path};
const SCRIPT: &str = include_str!("../hooks/check.mjs");
const COMMAND: &str = "node \"$CLAUDE_PROJECT_DIR/.claude/hooks/brain-check.mjs\"";

pub fn setup(root: &Path, hooks: bool) -> Result<Vec<String>, String> {
    let mut files = crate::mcp_setup::registrations(root)?;
    if hooks {
        for rel in [
            ".claude",
            ".claude/hooks",
            ".claude/hooks/brain-check.mjs",
            ".claude/settings.json",
        ] {
            if fs::symlink_metadata(root.join(rel)).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!(
                    "{rel} is a symlink; use regular project-local hook files"
                ));
            }
        }
        let mut settings: Value = match fs::read_to_string(root.join(".claude/settings.json")) {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|_| "Invalid .claude/settings.json; fix its JSON before setup")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(e) => return Err(e.to_string()),
        };
        let obj = settings
            .as_object_mut()
            .ok_or("Claude settings must be an object")?;
        let hooks = obj
            .entry("hooks")
            .or_insert(json!({}))
            .as_object_mut()
            .ok_or("settings.hooks must be an object")?;
        let post = hooks
            .entry("PostToolUse")
            .or_insert(json!([]))
            .as_array_mut()
            .ok_or("PostToolUse must be an array")?;
        let exists = post.iter().any(|e| {
            e.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
                hs.iter()
                    .any(|h| h.get("command").and_then(Value::as_str) == Some(COMMAND))
            })
        });
        if !exists {
            post.push(json!({"matcher":"Write|Edit|MultiEdit","hooks":[{"type":"command","command":COMMAND,"timeout":20}]}));
        }
        if let Ok(existing) = fs::read_to_string(root.join(".claude/hooks/brain-check.mjs")) {
            if existing != SCRIPT {
                return Err(
                    "Existing brain-check hook differs; preserve or rename it before setup".into(),
                );
            }
        }
        files.push((
            ".claude/settings.json".into(),
            serde_json::to_string_pretty(&settings).unwrap() + "\n",
        ));
        files.push((".claude/hooks/brain-check.mjs".into(), SCRIPT.into()));
    }
    // All existing configuration was validated before the first write.
    let mut written = Vec::new();
    for (rel, body) in files {
        let path = root.join(&rel);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(path, body).map_err(|e| e.to_string())?;
        written.push(rel);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hook_setup_preserves_unrelated_settings_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!("brain-agent-hooks-{}", std::process::id()));
        fs::create_dir_all(root.join(".claude")).unwrap();
        fs::write(root.join(".claude/settings.json"),r#"{"permissions":{"allow":["Read"]},"hooks":{"PostToolUse":[{"matcher":"Other","hooks":[]}]}}"#).unwrap();
        setup(&root, true).unwrap();
        let once = fs::read_to_string(root.join(".claude/settings.json")).unwrap();
        setup(&root, true).unwrap();
        assert_eq!(
            once,
            fs::read_to_string(root.join(".claude/settings.json")).unwrap()
        );
        let j: Value = serde_json::from_str(&once).unwrap();
        assert_eq!(j["permissions"]["allow"][0], "Read");
        assert_eq!(j["hooks"]["PostToolUse"].as_array().unwrap().len(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
