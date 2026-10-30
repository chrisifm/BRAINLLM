//! Explicit build-time native socket opt-in; no backend/database dependency.

#[derive(Clone, Debug, Default)]
pub struct BuildOptions {
    pub native_sockets: Option<NativeSocketOptions>,
}

#[derive(Clone, Debug)]
pub struct NativeSocketOptions {
    public_origin: String,
    connect_source: String,
}

impl NativeSocketOptions {
    pub fn new(public_origin: &str) -> Result<Self, String> {
        if public_origin.len() > 2048
            || public_origin.chars().any(char::is_control)
            || public_origin.contains(['*', '\'', '"'])
        {
            return Err("Native sockets require one canonical public HTTP(S) origin".into());
        }
        let mut parsed = url::Url::parse(public_origin)
            .map_err(|_| "Native sockets require one canonical public HTTP(S) origin")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.origin().ascii_serialization() != public_origin
        {
            return Err("Native sockets require one canonical public HTTP(S) origin without credentials, paths, query, fragment or wildcard".into());
        }
        let scheme = if parsed.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        parsed
            .set_scheme(scheme)
            .map_err(|_| "Cannot derive the native socket origin")?;
        Ok(Self {
            public_origin: public_origin.into(),
            connect_source: parsed.as_str().trim_end_matches('/').into(),
        })
    }

    pub fn public_origin(&self) -> &str {
        &self.public_origin
    }
    pub fn connect_source(&self) -> &str {
        &self.connect_source
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_source_is_exact_and_keeps_the_owned_public_port() {
        assert_eq!(
            NativeSocketOptions::new("https://app.example.test")
                .unwrap()
                .connect_source(),
            "wss://app.example.test"
        );
        assert_eq!(
            NativeSocketOptions::new("http://localhost:4000")
                .unwrap()
                .connect_source(),
            "ws://localhost:4000"
        );
        assert_eq!(
            NativeSocketOptions::new("http://[::1]:4000")
                .unwrap()
                .connect_source(),
            "ws://[::1]:4000"
        );
        for invalid in [
            "https://app.example.test/",
            "https://app.example.test/path",
            "wss://app.example.test",
            "https://user:secret@app.example.test",
            "https://app.example.test?token=secret",
            "https://app.example.test#x",
            "HTTPS://APP.EXAMPLE.TEST",
            "https://*.example.test",
            "https://app.example.test\r\n",
        ] {
            let error = NativeSocketOptions::new(invalid).unwrap_err();
            assert!(!error.contains("secret"));
        }
    }
}
