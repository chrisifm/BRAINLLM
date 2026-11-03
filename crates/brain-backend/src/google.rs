//! Google Identity Services ID-token verification. No token-controlled URLs; linking rules live in `Auth::google_login`.
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
pub use crate::oidc::Identity;
const JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
#[derive(Clone)]
pub struct Config {
    pub client_id: String,
}
impl Config {
    pub fn new(client_id: String) -> Result<Self, String> {
        if client_id.len() > 255
            || !client_id.ends_with(".apps.googleusercontent.com")
            || client_id.len() < 30
            || !client_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
        {
            return Err("Google client id must be the configured web OAuth client id".into());
        }
        Ok(Self { client_id })
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Claims {
    sub: String,
    iss: String,
    aud: serde_json::Value,
    exp: u64,
    iat: u64,
    nonce: String,
    email: String,
    email_verified: bool,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    hd: Option<String>,
}
#[derive(Clone)]
pub struct Google {
    config: Config,
    keys: crate::oidc::KeySet,
}
impl Google {
    pub fn new(config: Config) -> Result<Self, String> {
        Config::new(config.client_id.clone())?;
        Ok(Self {
            config,
            keys: crate::oidc::KeySet::new(JWKS_URL, "Google")?,
        })
    }
    pub fn client_id(&self) -> &str {
        &self.config.client_id
    }
    pub async fn verify(&self, credential: &str, nonce: &str) -> Result<Option<Identity>, String> {
        if credential.len() > 16384 || !crate::jwt::valid_opaque(nonce) {
            return Ok(None);
        }
        self.keys
            .verify(credential, |key| self.verify_key(credential, nonce, key))
            .await
    }
    fn verify_key(&self, credential: &str, nonce: &str, key: &DecodingKey) -> Option<Identity> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = 0;
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);
        validation.set_audience(&[&self.config.client_id]);
        let claims = decode::<Claims>(credential, key, &validation).ok()?.claims;
        let now = crate::jwt::now();
        let multiple = claims.aud.as_array().is_some_and(|aud| aud.len() > 1);
        if !crate::oidc::identifier(&claims.sub)
            || claims.nonce != nonce
            || !claims.email_verified
            || claims.iat > now + 30
            || claims.exp <= claims.iat
            || claims.exp - claims.iat > 86400
            || claims
                .azp
                .as_ref()
                .is_some_and(|azp| azp != &self.config.client_id)
            || multiple && claims.azp.is_none()
        {
            return None;
        }
        let email = crate::auth::Auth::email(&claims.email).ok()?;
        let authoritative_email = email.ends_with("@gmail.com")
            || claims.hd.as_ref().is_some_and(|domain| {
                !domain.is_empty()
                    && domain.len() <= 255
                    && domain
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
            });
        // Google hosts the address (gmail.com or a Workspace domain), so the holder owns it today.
        Some(Identity {
            linkable: authoritative_email,
            verified: authoritative_email,
            subject: claims.sub,
            issuer: claims.iss,
            email: Some(email),
            name: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    #[tokio::test]
    async fn cached_synthetic_google_keys_validate_signature_audience_issuer_nonce_and_account_claims()
     {
        let google =
            Google::new(Config::new("1234567890-test.apps.googleusercontent.com".into()).unwrap())
                .unwrap();
        let (encoding, set) = crate::oidc::synthetic::keys("synthetic-key");
        google.keys.preload(set).await;
        let nonce = crate::jwt::opaque();
        let now = crate::jwt::now();
        let claims = serde_json::json!({"sub":"12345678901234567890","iss":"https://accounts.google.com","aud":google.client_id(),"exp":now+3600,"iat":now,"nonce":nonce,"email":"synthetic@example.test","email_verified":true});
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("synthetic-key".into());
        let token = encode(&header, &claims, &encoding).unwrap();
        let identity = google.verify(&token, &nonce).await.unwrap().unwrap();
        assert_eq!(identity.subject, "12345678901234567890");
        assert_eq!(identity.email.as_deref(), Some("synthetic@example.test"));
        let mut tampered = token.clone();
        let at = tampered.rfind('.').unwrap() + 1;
        let replacement = if tampered.as_bytes()[at] == b'A' {
            "B"
        } else {
            "A"
        };
        tampered.replace_range(at..at + 1, replacement);
        assert!(google.verify(&tampered, &nonce).await.unwrap().is_none());
        assert!(
            google
                .verify(&token, &crate::jwt::opaque())
                .await
                .unwrap()
                .is_none()
        );
        for (key, value) in [
            ("aud", serde_json::json!("other-client")),
            ("iss", serde_json::json!("https://evil.example")),
            ("email_verified", serde_json::json!(false)),
            ("azp", serde_json::json!("other-client")),
            ("exp", serde_json::json!(now - 1)),
            ("sub", serde_json::json!("")),
        ] {
            let mut bad = claims.clone();
            bad[key] = value;
            let token = encode(&header, &bad, &encoding).unwrap();
            assert!(
                google.verify(&token, &nonce).await.unwrap().is_none(),
                "{key}"
            );
        }
        let mut future = claims.clone();
        future["nbf"] = serde_json::json!(now + 60);
        assert!(
            google
                .verify(&encode(&header, &future, &encoding).unwrap(), &nonce)
                .await
                .unwrap()
                .is_none()
        );
        header.kid = Some("unknown-key".into());
        assert!(
            google
                .verify(&encode(&header, &claims, &encoding).unwrap(), &nonce)
                .await
                .unwrap()
                .is_none()
        );
        header.kid = Some("synthetic-key".into());
        header.jku = Some("https://evil.example/keys".into());
        assert!(
            google
                .verify(&encode(&header, &claims, &encoding).unwrap(), &nonce)
                .await
                .unwrap()
                .is_none()
        );
        let wrong = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(b"synthetic-not-google-key"),
        )
        .unwrap();
        assert!(google.verify(&wrong, &nonce).await.unwrap().is_none());
    }
    #[test]
    fn configured_web_client_is_explicit_and_bounded() {
        for value in [
            "",
            "other",
            "https://evil.example",
            "x.apps.googleusercontent.com\n",
        ] {
            assert!(Config::new(value.into()).is_err())
        }
    }
}
