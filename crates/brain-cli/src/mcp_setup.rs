//! Project MCP registration, shared by scaffolding and doctor. Never launches a client or changes global settings.
use brain_mcp::Json;
use std::{fs, path::Path};

const CONFIGS: &[&str] = &[".mcp.json", ".cursor/mcp.json"];
const COMMAND: &str = "{\"command\":\"npx\",\"args\":[\"--no-install\",\"brain\",\"mcp\"]}";

/// Merge only our entry. An existing brain registration is user-owned, including wrappers and environment.
pub fn registrations(root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut files = Vec::new();
    for rel in CONFIGS {
        let path = root.join(rel);
        // Hidden folders are allowed by brain new, but must not redirect its writes outside the app.
        for p in path.ancestors().take_while(|p| *p != root) {
            if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!(
                    "{rel} is a symlink; fix: use a regular project MCP configuration"
                ));
            }
        }
        let mut config = match fs::read_to_string(&path) {
            Ok(s) => Json::parse(&s).map_err(|_| {
                format!("invalid {rel}; fix: repair its JSON before creating the app")
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Json::obj([] as [(&str, Json); 0])
            }
            Err(e) => return Err(format!("cannot read {rel}: {e}")),
        };
        let Json::Obj(ref mut fields) = config else {
            return Err(format!("{rel} must be a JSON object"));
        };
        if !fields.iter().any(|(k, _)| k == "mcpServers") {
            fields.push(("mcpServers".into(), Json::obj([] as [(&str, Json); 0])));
        }
        let (_, servers) = fields.iter_mut().find(|(k, _)| k == "mcpServers").unwrap();
        let Json::Obj(servers) = servers else {
            return Err(format!("{rel}: mcpServers must be an object"));
        };
        if servers.iter().any(|(k, _)| k == "brain") {
            continue;
        }
        servers.push(("brain".into(), Json::parse(COMMAND).unwrap()));
        files.push((rel.to_string(), config.stringify() + "\n"));
    }
    Ok(files)
}

/// Local configuration advice only: user/global registrations cannot be inferred from this app's files.
pub fn advice(root: &Path) -> Vec<Json> {
    let registered = CONFIGS.iter().any(|rel| {
        fs::read_to_string(root.join(rel))
            .ok()
            .and_then(|s| Json::parse(&s).ok())
            .is_some_and(|j| {
                j.get("mcpServers")
                    .and_then(|s| s.get("brain"))
                    .is_some_and(|b| {
                        b.get("command")
                            .and_then(Json::as_str)
                            .is_some_and(|s| !s.trim().is_empty())
                            || b.get("url")
                                .and_then(Json::as_str)
                                .is_some_and(|s| !s.trim().is_empty())
                    })
            })
    });
    if registered {
        return vec![];
    }
    vec![Json::obj([
        ("code", Json::str("B0734")),
        ("level", Json::str("warning")),
        ("file", Json::str(".mcp.json")),
        (
            "msg",
            Json::str(
                "No local brain MCP registration found; global or other-client registrations are not inspected",
            ),
        ),
        (
            "fix",
            Json::str(
                "run `claude mcp add --scope project brain -- npx --no-install brain mcp`, or configure brain in your client's MCP settings; restart the client and verify brain_spec is available",
            ),
        ),
    ])]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaffold_merges_servers_and_preserves_custom_brain() {
        let root = std::env::temp_dir().join(format!("brain-mcp-config-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(".mcp.json"),
            r#"{"mcpServers":{"other":{"command":"custom"}},"setting":true}"#,
        )
        .unwrap();
        let files = registrations(&root).unwrap();
        let j = Json::parse(&files.iter().find(|(p, _)| p == ".mcp.json").unwrap().1).unwrap();
        assert_eq!(j.get("setting"), Some(&Json::Bool(true)));
        assert_eq!(
            j.get("mcpServers")
                .unwrap()
                .get("other")
                .unwrap()
                .get("command"),
            Some(&Json::str("custom"))
        );
        fs::write(
            root.join(".mcp.json"),
            r#"{"mcpServers":{"brain":{"command":"wrapper"}}}"#,
        )
        .unwrap();
        assert!(
            !registrations(&root)
                .unwrap()
                .iter()
                .any(|(p, _)| p == ".mcp.json")
        );
        assert!(advice(&root).is_empty());
        fs::write(root.join(".mcp.json"), "{}").unwrap();
        assert_eq!(advice(&root)[0].get("code"), Some(&Json::str("B0734")));
        fs::write(root.join(".mcp.json"), "broken").unwrap();
        assert!(registrations(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
