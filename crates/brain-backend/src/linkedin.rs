//! Sign In with LinkedIn using OpenID Connect: the authorization-code flow of a confidential web client. The browser
//! is redirected to LinkedIn with a one-use state; the backend exchanges the code with the client secret, verifies the
//! returned ID token and discards the access token. LinkedIn hosts no mailbox, so its email never links an account.
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
pub use crate::oidc::Identity;
const AUTHORIZE_URL: &str = "https://www.linkedin.com/oauth/v2/authorization";
const TOKEN_URL: &str = "https://www.linkedin.com/oauth/v2/accessToken";
const JWKS_URL: &str = "https://www.linkedin.com/oauth/openid/jwks";
/// The issuer LinkedIn's discovery document publishes, and the bare form its documentation shows.
const ISSUERS: [&str; 2] = ["https://www.linkedin.com/oauth", "https://www.linkedin.com"];
#[derive(Clone)]
pub struct Config {
    pub client_id: String,
    /// Server-only; never sent to a browser, logged or returned.
    pub client_secret: String,
    /// The exact redirect URL registered for the app, ending in `/_brain/auth/linkedin/callback`.
    pub redirect_uri: String,
}
impl Config {
    pub fn new(client_id: String, client_secret: String, redirect_uri: String) -> Result<Self, String> {
        if !(8..=64).contains(&client_id.len()) || !client_id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err("LinkedIn client id must be the app's Client ID".into());
        }
        if !(8..=256).contains(&client_secret.len()) || !client_secret.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err("LinkedIn client secret must be the app's Primary Client Secret".into());
        }
        if !redirect(&redirect_uri) {
            return Err("LinkedIn redirect URL must be an https URL (or http on localhost) without credentials, query or fragment".into());
        }
        Ok(Self { client_id, client_secret, redirect_uri })
    }
}
/// An https URL, or http on a loopback host for local development, without credentials, query or fragment. It is the
/// native callback, or a page of the app that forwards `code` and `state` (with the state cookie) to it.
fn redirect(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else { return false };
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    value.len() <= 512
        && (url.scheme() == "https" || url.scheme() == "http" && local)
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Flag {
    Bool(bool),
    Text(String),
}
#[derive(Deserialize)]
struct Claims {
    sub: String,
    iss: String,
    exp: u64,
    iat: u64,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<Flag>,
    #[serde(default, deserialize_with = "crate::oidc::profile_text")]
    name: Option<String>,
    #[serde(default, deserialize_with = "crate::oidc::profile_text")]
    picture: Option<String>,
}
#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    id_token: Option<String>,
}
#[derive(Clone)]
pub struct LinkedIn {
    config: Config,
    client: reqwest::Client,
    keys: crate::oidc::KeySet,
}
impl LinkedIn {
    pub fn new(config: Config) -> Result<Self, String> {
        let config = Config::new(config.client_id, config.client_secret, config.redirect_uri)?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(2))
            .pool_max_idle_per_host(1)
            .build()
            .map_err(|_| "LinkedIn sign-in unavailable")?;
        Ok(Self { config, client, keys: crate::oidc::KeySet::new(JWKS_URL, "LinkedIn")? })
    }
    /// The nonce bound to a state: it never travels alone, so a leaked ID token cannot be replayed with another state.
    pub fn nonce(state: &str) -> String {
        Sha256::digest(format!("brain:linkedin:nonce:{state}").as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
    pub fn authorize_url(&self, state: &str) -> String {
        let mut url = reqwest::Url::parse(AUTHORIZE_URL).expect("fixed LinkedIn authorization URL");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &self.config.redirect_uri)
            .append_pair("state", state)
            .append_pair("scope", "openid profile email")
            .append_pair("nonce", &Self::nonce(state));
        url.into()
    }
    /// Exchanges an authorization code for an ID token and verifies it. `Ok(None)`: LinkedIn refused the code or the
    /// token is not one it issued for this app and state. `Err`: LinkedIn is unreachable.
    pub async fn sign_in(&self, code: &str, state: &str) -> Result<Option<Identity>, String> {
        if code.is_empty() || code.len() > 2048 || !code.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Ok(None);
        }
        let form = reqwest::Url::parse_with_params(
            "https://form.invalid/",
            [
                ("grant_type", "authorization_code"),
                ("code", code),
                ("client_id", &self.config.client_id),
                ("client_secret", &self.config.client_secret),
                ("redirect_uri", &self.config.redirect_uri),
            ],
        )
        .map_err(|_| "LinkedIn sign-in unavailable")?
        .query()
        .unwrap_or_default()
        .to_owned();
        let mut response = self
            .client
            .post(TOKEN_URL)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .await
            .map_err(|_| "LinkedIn token endpoint temporarily unavailable")?;
        if response.status().is_server_error() {
            return Err("LinkedIn token endpoint unavailable".into());
        }
        if !response.status().is_success() {
            return Ok(None);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "LinkedIn token response interrupted")? {
            if body.len() + chunk.len() > 65536 {
                return Ok(None);
            }
            body.extend_from_slice(&chunk);
        }
        // Only the ID token is kept; the access token in the same answer is dropped with the body.
        let Some(id_token) = serde_json::from_slice::<TokenResponse>(&body).ok().and_then(|token| token.id_token) else {
            return Ok(None);
        };
        self.verify(&id_token, &Self::nonce(state)).await
    }
    async fn verify(&self, id_token: &str, nonce: &str) -> Result<Option<Identity>, String> {
        if id_token.len() > 16384 {
            return Ok(None);
        }
        self.keys.verify(id_token, |key| self.verify_key(id_token, nonce, key)).await
    }
    fn verify_key(&self, id_token: &str, nonce: &str, key: &DecodingKey) -> Option<Identity> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = 0;
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.set_issuer(&ISSUERS);
        validation.set_audience(&[&self.config.client_id]);
        let claims = decode::<Claims>(id_token, key, &validation).ok()?.claims;
        let now = crate::jwt::now();
        // The state already binds the browser; a nonce LinkedIn echoes must still be ours.
        if !crate::oidc::identifier(&claims.sub)
            || claims.nonce.as_deref().is_some_and(|value| value != nonce)
            || claims.iat > now + 30
            || claims.exp <= claims.iat
            || claims.exp - claims.iat > 86400
        {
            return None;
        }
        let verified = match &claims.email_verified {
            Some(Flag::Bool(value)) => *value,
            Some(Flag::Text(value)) => value == "true",
            None => false,
        };
        let email = claims.email.filter(|_| verified).and_then(|email| crate::auth::Auth::email(&email).ok());
        Some(
            Identity { subject: claims.sub, issuer: claims.iss, email, linkable: false, verified: false, name: None, picture: None }
                .with_name(claims.name.as_deref()).with_picture(claims.picture.as_deref()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Header, encode};
    #[tokio::test]
    async fn synthetic_linkedin_tokens_validate_audience_issuer_nonce_and_never_link() {
        let linkedin = LinkedIn::new(
            Config::new("abcd1234efgh".into(), "synthetic-client-secret".into(), "https://example.com/_brain/auth/linkedin/callback".into()).unwrap(),
        )
        .unwrap();
        let (encoding, set) = crate::oidc::synthetic::keys("synthetic-linkedin-key");
        linkedin.keys.preload(set).await;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("synthetic-linkedin-key".into());
        let state = crate::jwt::opaque();
        let nonce = LinkedIn::nonce(&state);
        let now = crate::jwt::now();
        let claims = serde_json::json!({"sub":"AbC-123_xyz","iss":"https://www.linkedin.com/oauth","aud":"abcd1234efgh","exp":now+3600,"iat":now,"nonce":nonce,"email":"Person@Example.com","email_verified":true,"name":"Synthetic Person","picture":"https://images.example.test/profile.png"});
        let identity = linkedin.verify(&encode(&header, &claims, &encoding).unwrap(), &nonce).await.unwrap().unwrap();
        assert_eq!(identity.email.as_deref(), Some("person@example.com"));
        assert_eq!(identity.name.as_deref(), Some("Synthetic Person"));
        assert_eq!(identity.picture.as_deref(), Some("https://images.example.test/profile.png"));
        for hint in [serde_json::json!(123), serde_json::json!(true), serde_json::json!({"unexpected":"value"}), serde_json::json!(["value"])] {
            let mut decorated = claims.clone();
            decorated["name"] = hint.clone();
            decorated["picture"] = hint;
            let accepted = linkedin.verify(&encode(&header, &decorated, &encoding).unwrap(), &nonce).await.unwrap().expect("optional display claim types do not reject identity");
            assert!(accepted.name.is_none() && accepted.picture.is_none());
            assert_eq!(accepted.subject, identity.subject);
            assert_eq!(accepted.email, identity.email);
        }

        assert!(!identity.linkable && !identity.verified, "LinkedIn hosts no mailbox");
        let mut unverified = claims.clone();
        unverified["email_verified"] = serde_json::json!("false");
        assert!(linkedin.verify(&encode(&header, &unverified, &encoding).unwrap(), &nonce).await.unwrap().unwrap().email.is_none());
        let mut silent = claims.clone();
        silent.as_object_mut().unwrap().remove("nonce");
        assert!(linkedin.verify(&encode(&header, &silent, &encoding).unwrap(), &nonce).await.unwrap().is_some());
        for (key, value) in [
            ("nonce", serde_json::json!(LinkedIn::nonce("other-state"))),
            ("aud", serde_json::json!("other-client")),
            ("iss", serde_json::json!("https://evil.example")),
            ("exp", serde_json::json!(now - 1)),
            ("sub", serde_json::json!("")),
        ] {
            let mut bad = claims.clone();
            bad[key] = value;
            assert!(linkedin.verify(&encode(&header, &bad, &encoding).unwrap(), &nonce).await.unwrap().is_none(), "{key}");
        }
        let url = reqwest::Url::parse(&linkedin.authorize_url(&state)).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["state"], state);
        assert_eq!(pairs["nonce"], nonce);
        assert_eq!(pairs["scope"], "openid profile email");
        assert_eq!(pairs["redirect_uri"], "https://example.com/_brain/auth/linkedin/callback");
        assert!(!url.as_str().contains("synthetic-client-secret"), "the secret never reaches the browser");
    }
    #[test]
    fn configured_client_and_redirect_are_explicit_and_bounded() {
        let ok = |uri: &str| Config::new("abcd1234efgh".into(), "synthetic-client-secret".into(), uri.into()).is_ok();
        assert!(ok("https://example.com/_brain/auth/linkedin/callback"));
        assert!(ok("http://localhost:5190/_brain/auth/linkedin/callback"));
        // a page of the app that relays code and state to the native callback
        assert!(ok("https://example.com/signin/linkedin"));
        assert!(ok("http://localhost:5190/signin/linkedin"));
        for uri in [
            "http://example.com/_brain/auth/linkedin/callback",
            "http://example.com/signin/linkedin",
            "https://example.com/signin/linkedin#done",
            "https://example.com/_brain/auth/linkedin/callback?next=/",
            "https://user@example.com/_brain/auth/linkedin/callback",
        ] {
            assert!(!ok(uri), "{uri}");
        }
        assert!(Config::new("bad id!".into(), "synthetic-client-secret".into(), "https://example.com/_brain/auth/linkedin/callback".into()).is_err());
    }
}
