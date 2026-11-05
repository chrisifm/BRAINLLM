//! Sign in with Apple for the web (Sign in with Apple JS, popup flow): ID-token and server-to-server notification
//! verification. No token-controlled URLs; linking rules live in `Auth::identity_login`. The Services ID is the
//! audience of a browser sign-in; account notifications are addressed to the primary App ID (or the Services ID).
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
pub use crate::oidc::Identity;
const JWKS_URL: &str = "https://appleid.apple.com/auth/keys";
const ISSUER: &str = "https://appleid.apple.com";
/// Mailboxes Apple hosts: a shared address there belongs to the Apple Account that signs in with it.
const APPLE_MAIL: [&str; 3] = ["icloud.com", "me.com", "mac.com"];
/// Private email relay domains: such an address reaches only this person, but only through this team's relay.
const RELAY_MAIL: [&str; 2] = ["privaterelay.appleid.com", "private.icloud.com"];
/// A notification older than this is refused; its id is remembered at least as long, so a replay is never re-applied.
pub(crate) const EVENT_WINDOW_SECS: u64 = 7 * 86400;
#[derive(Clone)]
pub struct Config {
    /// The Services ID registered for the website (`com.example.web`).
    pub client_id: String,
    /// The primary App ID the Services ID belongs to (`com.example.app`), the audience of account notifications.
    pub app_id: Option<String>,
    /// The Return URL registered for the Services ID; Sign in with Apple JS must name it even in the popup flow.
    pub redirect_uri: String,
}
impl Config {
    pub fn new(client_id: String, app_id: Option<String>, redirect_uri: String) -> Result<Self, String> {
        if !bundle_id(&client_id) {
            return Err("Apple client id must be the Services ID registered for the website".into());
        }
        if app_id.as_deref().is_some_and(|id| !bundle_id(id)) {
            return Err("Apple app id must be the primary App ID of the Services ID".into());
        }
        if !return_url(&redirect_uri) {
            return Err("Apple redirect URL must be the https Return URL registered for the Services ID".into());
        }
        Ok(Self { client_id, app_id, redirect_uri })
    }
}
/// Apple accepts only https Return URLs, without credentials or a fragment.
fn return_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        value.len() <= 512
            && url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    })
}
/// A reverse-DNS identifier as Apple registers Services IDs and bundle IDs.
fn bundle_id(value: &str) -> bool {
    (3..=255).contains(&value.len())
        && value.contains('.')
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}
/// Apple sends some flags as JSON booleans and others as the strings `"true"`/`"false"`.
#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum Flag {
    Bool(bool),
    Text(String),
}
impl Flag {
    fn on(flag: &Option<Flag>) -> bool {
        match flag {
            Some(Flag::Bool(value)) => *value,
            Some(Flag::Text(value)) => value == "true",
            None => false,
        }
    }
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
    #[serde(default)]
    is_private_email: Option<Flag>,
}
#[derive(Deserialize)]
struct NotificationClaims {
    iat: u64,
    #[serde(default)]
    exp: Option<u64>,
    jti: String,
    events: serde_json::Value,
}
#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    sub: String,
}
/// A verified account change. `kind` is Apple's event type (`consent-revoked`, `account-deleted`, `email-disabled`,
/// `email-enabled`); an unknown type is kept so it can be acknowledged without effect.
pub struct Notification {
    pub(crate) id: String,
    pub(crate) subject: String,
    pub(crate) kind: String,
}
#[derive(Clone)]
pub struct Apple {
    config: Config,
    keys: crate::oidc::KeySet,
}
impl Apple {
    pub fn new(config: Config) -> Result<Self, String> {
        Config::new(config.client_id.clone(), config.app_id.clone(), config.redirect_uri.clone())?;
        Ok(Self {
            config,
            keys: crate::oidc::KeySet::new(JWKS_URL, "Apple")?,
        })
    }
    pub fn client_id(&self) -> &str {
        &self.config.client_id
    }
    pub fn redirect_uri(&self) -> &str {
        &self.config.redirect_uri
    }
    /// Verifies the `id_token` of a Sign in with Apple JS response against the browser-bound nonce, sent raw.
    pub async fn verify(&self, id_token: &str, nonce: &str) -> Result<Option<Identity>, String> {
        if id_token.len() > 16384 || !crate::jwt::valid_opaque(nonce) {
            return Ok(None);
        }
        self.keys
            .verify(id_token, |key| self.verify_key(id_token, nonce, key))
            .await
    }
    fn verify_key(&self, id_token: &str, nonce: &str, key: &DecodingKey) -> Option<Identity> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = 0;
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[&self.config.client_id]);
        let claims = decode::<Claims>(id_token, key, &validation).ok()?.claims;
        let now = crate::jwt::now();
        if !crate::oidc::identifier(&claims.sub)
            || claims.nonce.as_deref() != Some(nonce)
            || claims.iat > now + 30
            || claims.exp <= claims.iat
            || claims.exp - claims.iat > 86400
        {
            return None;
        }
        // Apple shares an address only after it verified it; an address it did not verify is treated as absent.
        let email = claims
            .email
            .filter(|_| Flag::on(&claims.email_verified))
            .and_then(|email| crate::auth::Auth::email(&email).ok());
        let domain = email
            .as_deref()
            .and_then(|email| email.rsplit_once('@'))
            .map(|(_, domain)| domain.to_owned());
        let relay = Flag::on(&claims.is_private_email)
            || domain.as_deref().is_some_and(|domain| RELAY_MAIL.contains(&domain));
        let hosted = !relay && domain.as_deref().is_some_and(|domain| APPLE_MAIL.contains(&domain));
        Some(Identity {
            linkable: hosted,
            verified: hosted || relay && email.is_some(),
            subject: claims.sub,
            issuer: claims.iss,
            email,
            name: None, picture: None,
        })
    }
    /// Verifies the signed `payload` of a server-to-server notification. `Ok(None)`: not a notification Apple signed
    /// for this website, or one outside the replay window.
    pub async fn notification(&self, payload: &str) -> Result<Option<Notification>, String> {
        if payload.len() > 16384 {
            return Ok(None);
        }
        self.keys
            .verify(payload, |key| self.notification_key(payload, key))
            .await
    }
    fn notification_key(&self, payload: &str, key: &DecodingKey) -> Option<Notification> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = 0;
        // Notifications carry `iat` and `jti` but no `exp`; freshness is bounded by `iat` and replay by `jti`.
        validation.validate_exp = false;
        validation.set_required_spec_claims(&["iss", "aud"]);
        validation.set_issuer(&[ISSUER]);
        let audiences: Vec<&str> = std::iter::once(self.config.client_id.as_str())
            .chain(self.config.app_id.as_deref())
            .collect();
        validation.set_audience(&audiences);
        let claims = decode::<NotificationClaims>(payload, key, &validation).ok()?.claims;
        let now = crate::jwt::now();
        if !crate::oidc::identifier(&claims.jti)
            || claims.iat > now + 30
            || now.saturating_sub(claims.iat) > EVENT_WINDOW_SECS
            || claims.exp.is_some_and(|exp| exp <= now)
        {
            return None;
        }
        let event: Event = match claims.events {
            serde_json::Value::String(text) => serde_json::from_str(&text).ok()?,
            value @ serde_json::Value::Object(_) => serde_json::from_value(value).ok()?,
            _ => return None,
        };
        if !crate::oidc::identifier(&event.sub)
            || event.kind.is_empty()
            || event.kind.len() > 64
            || !event
                .kind
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        {
            return None;
        }
        Some(Notification {
            id: claims.jti,
            subject: event.sub,
            kind: event.kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    const SERVICES_ID: &str = "com.example.web";
    const APP_ID: &str = "com.example.app";
    const RETURN_URL: &str = "https://example.com/";
    async fn apple() -> (Apple, EncodingKey, Header) {
        let apple = Apple::new(Config::new(SERVICES_ID.into(), Some(APP_ID.into()), RETURN_URL.into()).unwrap()).unwrap();
        let (encoding, set) = crate::oidc::synthetic::keys("synthetic-apple-key");
        apple.keys.preload(set).await;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("synthetic-apple-key".into());
        (apple, encoding, header)
    }
    #[tokio::test]
    async fn synthetic_apple_tokens_validate_signature_audience_issuer_nonce_and_email_rules() {
        let (apple, encoding, mut header) = apple().await;
        let nonce = crate::jwt::opaque();
        let now = crate::jwt::now();
        let claims = serde_json::json!({"sub":"000000.0123456789abcdef0123456789abcdef.0000","iss":ISSUER,"aud":SERVICES_ID,"exp":now+600,"iat":now,"nonce":nonce,"nonce_supported":true,"email":"Person@iCloud.com","email_verified":"true","is_private_email":"false"});
        let token = encode(&header, &claims, &encoding).unwrap();
        let identity = apple.verify(&token, &nonce).await.unwrap().unwrap();
        assert_eq!(identity.subject, "000000.0123456789abcdef0123456789abcdef.0000");
        assert_eq!(identity.email.as_deref(), Some("person@icloud.com"));
        assert!(identity.linkable && identity.verified);
        // Booleans and strings are the same flag; a relay address is verified for a new account but never links.
        let mut relay = claims.clone();
        relay["email"] = serde_json::json!("abcdef1234@privaterelay.appleid.com");
        relay["email_verified"] = serde_json::json!(true);
        relay["is_private_email"] = serde_json::json!(true);
        let identity = apple
            .verify(&encode(&header, &relay, &encoding).unwrap(), &nonce)
            .await
            .unwrap()
            .unwrap();
        assert!(!identity.linkable && identity.verified);
        relay["email"] = serde_json::json!("abcdef1234@private.icloud.com");
        relay["is_private_email"] = serde_json::json!("false");
        let identity = apple
            .verify(&encode(&header, &relay, &encoding).unwrap(), &nonce)
            .await
            .unwrap()
            .unwrap();
        assert!(!identity.linkable && identity.verified, "the relay domain alone marks a private address");
        // An address Apple does not host neither links nor counts as verified; an unverified one is treated as absent.
        let mut other = claims.clone();
        other["email"] = serde_json::json!("person@example.com");
        let identity = apple
            .verify(&encode(&header, &other, &encoding).unwrap(), &nonce)
            .await
            .unwrap()
            .unwrap();
        assert!(!identity.linkable && !identity.verified);
        other["email_verified"] = serde_json::json!("false");
        let identity = apple
            .verify(&encode(&header, &other, &encoding).unwrap(), &nonce)
            .await
            .unwrap()
            .unwrap();
        assert!(identity.email.is_none() && !identity.linkable && !identity.verified);
        let mut managed = claims.clone();
        managed.as_object_mut().unwrap().remove("email");
        let identity = apple
            .verify(&encode(&header, &managed, &encoding).unwrap(), &nonce)
            .await
            .unwrap()
            .unwrap();
        assert!(identity.email.is_none() && !identity.verified);
        assert!(apple.verify(&token, &crate::jwt::opaque()).await.unwrap().is_none());
        let mut no_nonce = claims.clone();
        no_nonce.as_object_mut().unwrap().remove("nonce");
        assert!(apple.verify(&encode(&header, &no_nonce, &encoding).unwrap(), &nonce).await.unwrap().is_none());
        for (key, value) in [
            ("aud", serde_json::json!(APP_ID)),
            ("aud", serde_json::json!("com.other.web")),
            ("iss", serde_json::json!("https://evil.example")),
            ("iss", serde_json::json!("appleid.apple.com")),
            ("exp", serde_json::json!(now - 1)),
            ("iat", serde_json::json!(now + 120)),
            ("sub", serde_json::json!("")),
        ] {
            let mut bad = claims.clone();
            bad[key] = value;
            let token = encode(&header, &bad, &encoding).unwrap();
            assert!(apple.verify(&token, &nonce).await.unwrap().is_none(), "{key}");
        }
        let mut tampered = token.clone();
        let at = tampered.rfind('.').unwrap() + 1;
        let replacement = if tampered.as_bytes()[at] == b'A' { "B" } else { "A" };
        tampered.replace_range(at..at + 1, replacement);
        assert!(apple.verify(&tampered, &nonce).await.unwrap().is_none());
        header.kid = Some("unknown-key".into());
        assert!(apple.verify(&encode(&header, &claims, &encoding).unwrap(), &nonce).await.unwrap().is_none());
        header.kid = Some("synthetic-apple-key".into());
        header.jku = Some("https://evil.example/keys".into());
        assert!(apple.verify(&encode(&header, &claims, &encoding).unwrap(), &nonce).await.unwrap().is_none());
        let wrong = encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(b"synthetic-not-apple-key")).unwrap();
        assert!(apple.verify(&wrong, &nonce).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn synthetic_apple_notifications_accept_object_or_string_events_within_the_replay_window() {
        let (apple, encoding, header) = apple().await;
        let now = crate::jwt::now();
        let claims = serde_json::json!({"iss":ISSUER,"aud":APP_ID,"iat":now,"jti":"synthetic-event-1","events":{"type":"consent-revoked","sub":"000000.abc.0000","event_time":now}});
        let notice = apple
            .notification(&encode(&header, &claims, &encoding).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((notice.id.as_str(), notice.subject.as_str(), notice.kind.as_str()), ("synthetic-event-1", "000000.abc.0000", "consent-revoked"));
        let mut text = claims.clone();
        text["aud"] = serde_json::json!(SERVICES_ID);
        text["events"] = serde_json::json!(r#"{"type":"email-disabled","sub":"000000.abc.0000","email":"abcdef1234@privaterelay.appleid.com","is_private_email":"true","event_time":1}"#);
        let notice = apple
            .notification(&encode(&header, &text, &encoding).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(notice.kind, "email-disabled");
        for (key, value) in [
            ("aud", serde_json::json!("com.other.app")),
            ("iss", serde_json::json!("https://evil.example")),
            ("iat", serde_json::json!(now - EVENT_WINDOW_SECS - 60)),
            ("iat", serde_json::json!(now + 120)),
            ("exp", serde_json::json!(now - 1)),
            ("jti", serde_json::json!("")),
            ("events", serde_json::json!({"type":"consent-revoked"})),
            ("events", serde_json::json!({"type":"Consent Revoked","sub":"000000.abc.0000"})),
            ("events", serde_json::json!(["consent-revoked"])),
        ] {
            let mut bad = claims.clone();
            bad[key] = value;
            assert!(apple.notification(&encode(&header, &bad, &encoding).unwrap()).await.unwrap().is_none(), "{key}");
        }
    }
    #[test]
    fn configured_services_and_app_ids_are_explicit_and_bounded() {
        assert!(Config::new(SERVICES_ID.into(), None, RETURN_URL.into()).is_ok());
        for url in ["http://example.com/", "https://user@example.com/", "https://example.com/#x", "not a url"] {
            assert!(Config::new(SERVICES_ID.into(), None, url.into()).is_err(), "{url}");
        }
        for value in ["", "web", ".example.web", "com..web", "com.example.web\n", "https://example.com"] {
            assert!(Config::new(value.into(), None, RETURN_URL.into()).is_err(), "{value:?}");
            assert!(Config::new(SERVICES_ID.into(), Some(value.into()), RETURN_URL.into()).is_err(), "{value:?}");
        }
    }
}
