//! One bounded source declaration for native HTTP and live-resource authorization.
use brain_mcp::Json;
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug, Default)]
pub struct SocketSettings {
    pub http_origins: Vec<String>,
    pub allowed_origins: Option<Vec<String>>,
}

impl SocketSettings {
    pub fn enabled(&self) -> bool {
        self.allowed_origins.is_some()
    }

    pub fn native_config(&self) -> Result<Option<brain_backend::http::SocketConfig>, String> {
        self.allowed_origins
            .clone()
            .map(brain_backend::http::SocketConfig::new)
            .transpose()
    }

    pub fn build_options(&self, site: Option<&str>) -> Result<brain_pack::BuildOptions, String> {
        let Some(allowed) = &self.allowed_origins else {
            return Ok(brain_pack::BuildOptions::default());
        };
        let site = site.ok_or("Native sockets require PUBLIC_SITE_URL or --site naming this frontend's public HTTP(S) URL")?;
        let origin = public_origin(site)?;
        if !allowed.contains(&origin) {
            return Err("The selected frontend origin must appear in backend.sockets.allowedOrigins; configure the exact environment/port instead of adding a wildcard".into());
        }
        Ok(brain_pack::BuildOptions {
            native_sockets: Some(brain_pack::NativeSocketOptions::new(&origin)?),
        })
    }

    pub fn frontend_enabled(&self, frontend: &str) -> Result<bool, String> {
        let Some(allowed) = &self.allowed_origins else {
            return Ok(false);
        };
        let origin = public_origin(frontend)?;
        if !allowed.contains(&origin) {
            return Err("This frontend's exact origin/port is absent from backend.sockets.allowedOrigins; update the shared declaration and backend HTTP origins before enabling the proxy".into());
        }
        Ok(true)
    }
}

fn public_origin(site: &str) -> Result<String, String> {
    if site.len() > 2048 || site.chars().any(char::is_control) {
        return Err("Use a bounded public HTTP(S) site URL without credentials".into());
    }
    brain_backend::links::origin(site)
        .ok_or_else(|| "Use a public HTTP(S) site URL without credentials".into())
}

fn object<'a>(value: &'a Json, allowed: &[&str]) -> Result<&'a [(String, Json)], String> {
    let Json::Obj(fields) = value else {
        return Err("brain.json backend and sockets declarations must be objects".into());
    };
    let mut seen = BTreeSet::new();
    for (name, _) in fields {
        if !allowed.contains(&name.as_str()) || !seen.insert(name) {
            return Err("brain.json backend declaration has an unknown or duplicate property; use origins and sockets.allowedOrigins only".into());
        }
    }
    Ok(fields)
}

fn origins(value: &Json) -> Result<Vec<String>, String> {
    let Json::Arr(values) = value else {
        return Err("Declare 1..8 distinct canonical HTTP(S) origins".into());
    };
    if values.is_empty() || values.len() > 8 {
        return Err("Declare 1..8 distinct canonical HTTP(S) origins".into());
    }
    let values = values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "Origins must be canonical HTTP(S) strings".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    brain_backend::http::SocketConfig::new(values.clone()).map_err(|_| "Origins must be distinct canonical HTTP(S) origins without paths, credentials, query, fragment or wildcards")?;
    Ok(values)
}

/// Explicit --origin REPLACES the HTTP origin list. It never unions or edits
/// sockets.allowedOrigins; that declaration must remain a subset afterwards.
pub fn socket_settings(
    root: &Path,
    site: Option<&str>,
    explicit_origin: Option<&str>,
) -> Result<SocketSettings, String> {
    let value = if root
        .join("brain.json")
        .symlink_metadata()
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        Json::Obj(Vec::new())
    } else {
        let source = crate::map::read_source_limit(root, "brain.json", 2 * 1024 * 1024)
            .map_err(|_| "Read brain.json as a regular application source file of at most 2 MiB")?;
        Json::parse(&source).map_err(|_| "brain.json must contain valid bounded JSON")?
    };
    let Json::Obj(fields) = &value else {
        return Err("brain.json must contain an object".into());
    };
    let declarations: Vec<_> = fields
        .iter()
        .filter(|(name, _)| name == "backend")
        .collect();
    if declarations.len() > 1 {
        return Err("brain.json must declare backend at most once".into());
    }
    let mut configured = None;
    let mut sockets = None;
    if let Some((_, backend)) = declarations.first() {
        let fields = object(backend, &["origins", "sockets"])?;
        if let Some((_, value)) = fields.iter().find(|(name, _)| name == "origins") {
            configured = Some(origins(value)?);
        }
        if let Some((_, value)) = fields.iter().find(|(name, _)| name == "sockets") {
            if value != &Json::Bool(false) {
                let fields = object(value, &["allowedOrigins"])?;
                let value = fields
                    .iter()
                    .find(|(name, _)| name == "allowedOrigins")
                    .ok_or("backend.sockets requires explicit allowedOrigins")?;
                sockets = Some(origins(&value.1)?);
            }
        }
    }
    let http_origins = if let Some(origin) = explicit_origin {
        origins(&Json::Arr(vec![Json::Str(origin.into())]))?
    } else if let Some(origins) = configured {
        origins
    } else if let Some(site) = site {
        vec![public_origin(site)?]
    } else {
        Vec::new()
    };
    if let Some(allowed) = &sockets {
        if http_origins.is_empty() {
            return Err(
                "Set PUBLIC_SITE_URL or backend.origins before enabling native sockets".into(),
            );
        }
        if allowed.iter().any(|origin| !http_origins.contains(origin)) {
            return Err("backend.sockets.allowedOrigins must be a subset of HTTP backend.origins after any --origin replacement".into());
        }
    }
    Ok(SocketSettings {
        http_origins,
        allowed_origins: sockets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture(source: &str) -> Fixture {
        let path = std::env::temp_dir().join(format!(
            "brain-socket-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("brain.json"), source).unwrap();
        Fixture(path)
    }
    #[test]
    fn absent_and_false_preserve_disabled_native_defaults() {
        for source in ["{}", r#"{"backend":{"sockets":false}}"#] {
            let root = fixture(source);
            let selected =
                socket_settings(&root.0, Some("https://app.example.test/nested/"), None).unwrap();
            assert_eq!(selected.http_origins, ["https://app.example.test"]);
            assert!(selected.native_config().unwrap().is_none());
            assert!(!selected.frontend_enabled("http://localhost:4000").unwrap());
            assert!(
                selected
                    .build_options(None)
                    .unwrap()
                    .native_sockets
                    .is_none()
            );
        }
    }
    #[test]
    fn legacy_app_without_brain_json_remains_disabled() {
        let root = fixture("{}");
        fs::remove_file(root.0.join("brain.json")).unwrap();
        let settings = socket_settings(&root.0, None, None).unwrap();
        assert!(settings.native_config().unwrap().is_none());
        assert!(settings.http_origins.is_empty());
    }
    #[test]
    fn one_declaration_drives_http_socket_subset_and_selected_public_site() {
        let root = fixture(
            r#"{"backend":{"origins":["https://app.example.test","http://localhost:4000"],"sockets":{"allowedOrigins":["https://app.example.test","http://localhost:4000"]}}}"#,
        );
        let selected =
            socket_settings(&root.0, Some("https://app.example.test/nested/"), None).unwrap();
        assert_eq!(selected.http_origins.len(), 2);
        assert!(selected.native_config().unwrap().is_some());
        assert!(
            selected
                .build_options(Some("https://app.example.test/nested/"))
                .unwrap()
                .native_sockets
                .is_some()
        );
        assert!(
            selected
                .frontend_enabled("http://localhost:4000/sub/")
                .unwrap()
        );
        assert!(selected.frontend_enabled("http://localhost:4001").is_err());
        assert!(
            socket_settings(&root.0, None, Some("https://app.example.test")).is_err(),
            "An explicit override must not silently union previous origins"
        );
    }
    #[test]
    fn duplicate_unknown_unbounded_and_credential_declarations_fail_without_echoing_values() {
        for source in [
            r#"{"backend":{},"backend":{}}"#,
            r#"{"backend":{"origins":[],"origins":[]}}"#,
            r#"{"backend":{"sockets":true}}"#,
            r#"{"backend":{"sockets":{"allowedOrigins":[],"allow":true}}}"#,
            r#"{"backend":{"origins":["https://secret:password@app.example.test"]}}"#,
            r#"{"backend":{"origins":["https://app.example.test/path"]}}"#,
            r#"{"backend":{"origins":["https://app.example.test"],"sockets":{"allowedOrigins":["https://other.example.test"]}}}"#,
        ] {
            let root = fixture(source);
            let error = socket_settings(&root.0, None, None).unwrap_err();
            assert!(!error.contains("password"));
        }
        let root =
            fixture(r#"{"backend":{"sockets":{"allowedOrigins":["https://app.example.test"]}}}"#);
        assert!(socket_settings(&root.0, None, None).is_err());
        assert!(socket_settings(&root.0, Some("https://app.example.test/path"), None).is_ok());
    }
}
