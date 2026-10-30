//! Google Identity Services ID-token verification. No token-controlled URLs or implicit email linking.
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
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
/// Produced only after signature and fixed issuer/audience/nonce validation.
pub struct Identity {
    pub(crate) subject: String,
    pub(crate) issuer: String,
    pub(crate) email: String,
    pub(crate) authoritative_email: bool,
}
struct Keys {
    set: JwkSet,
    until: Instant,
    last_attempt: Option<Instant>,
}
#[derive(Clone)]
pub struct Google {
    config: Config,
    client: reqwest::Client,
    keys: Arc<Mutex<Keys>>,
    slots: Arc<Semaphore>,
}
impl Google {
    pub fn new(config: Config) -> Result<Self, String> {
        Config::new(config.client_id.clone())?;
        let _ = jsonwebtoken::crypto::rust_crypto::DEFAULT_PROVIDER.install_default();
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .connect_timeout(Duration::from_secs(2))
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "Google identity verifier unavailable")?;
        Ok(Self {
            config,
            client,
            keys: Arc::new(Mutex::new(Keys {
                set: JwkSet { keys: vec![] },
                until: Instant::now(),
                last_attempt: None,
            })),
            slots: Arc::new(Semaphore::new(2)),
        })
    }
    pub fn client_id(&self) -> &str {
        &self.config.client_id
    }
    async fn key(&self, kid: &str) -> Result<Option<DecodingKey>, String> {
        let mut keys = tokio::time::timeout(Duration::from_millis(100), self.keys.lock())
            .await
            .map_err(|_| "Google verifier busy; retry shortly")?;
        let time = Instant::now();
        if time < keys.until {
            if let Some(key) = keys.set.find(kid) {
                return Ok(DecodingKey::from_jwk(key).ok());
            }
        }
        // One fixed issuer's key set, capped at sixteen keys/64 KiB; unknown kids cannot trigger a fetch storm.
        if keys
            .last_attempt
            .is_some_and(|last| time.duration_since(last) < Duration::from_secs(30))
        {
            return Ok(None);
        }
        keys.last_attempt = Some(time);
        let mut response = self
            .client
            .get(JWKS_URL)
            .send()
            .await
            .map_err(|_| "Google public keys temporarily unavailable")?;
        if !response.status().is_success()
            || response.content_length().is_some_and(|len| len > 65536)
        {
            return Err("Google public keys unavailable".into());
        }
        let ttl = response
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .and_then(|value| {
                value
                    .split(',')
                    .find_map(|part| part.trim().strip_prefix("max-age=")?.parse::<u64>().ok())
            })
            .unwrap_or(300)
            .min(3600);
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Google public key response interrupted")?
        {
            if body.len() + chunk.len() > 65536 {
                return Err("Google public key response exceeded its limit".into());
            }
            body.extend_from_slice(&chunk);
        }
        let set: JwkSet =
            serde_json::from_slice(&body).map_err(|_| "Google public key response is invalid")?;
        if set.keys.is_empty() || set.keys.len() > 16 {
            return Err("Google public key count exceeded its limit".into());
        }
        let mut ids = std::collections::HashSet::new();
        if set.keys.iter().any(|key| {
            key.common
                .key_id
                .as_ref()
                .is_none_or(|kid| kid.len() > 128 || !ids.insert(kid.clone()))
        }) {
            return Err("Google public key identities are invalid".into());
        }
        keys.set = set;
        keys.until = Instant::now() + Duration::from_secs(ttl);
        Ok(keys
            .set
            .find(kid)
            .and_then(|key| DecodingKey::from_jwk(key).ok()))
    }
    pub async fn verify(&self, credential: &str, nonce: &str) -> Result<Option<Identity>, String> {
        if credential.len() > 16384 || !crate::jwt::valid_opaque(nonce) {
            return Ok(None);
        }
        let _permit = self
            .slots
            .try_acquire()
            .map_err(|_| "Google verifier busy; retry shortly")?;
        let Ok(header) = decode_header(credential) else {
            return Ok(None);
        };
        if header.alg != Algorithm::RS256
            || header.crit.is_some()
            || header.jku.is_some()
            || header.jwk.is_some()
            || header.x5u.is_some()
            || header.enc.is_some()
            || header.zip.is_some()
        {
            return Ok(None);
        }
        let Some(kid) = header.kid.filter(|kid| {
            !kid.is_empty()
                && kid.len() <= 128
                && kid
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        }) else {
            return Ok(None);
        };
        let Some(key) = self.key(&kid).await? else {
            return Ok(None);
        };
        Ok(self.verify_key(credential, nonce, &key))
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
        if claims.sub.is_empty()
            || claims.sub.len() > 255
            || !claims.sub.bytes().all(|byte| byte.is_ascii_graphic())
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
        Some(Identity {
            authoritative_email,
            subject: claims.sub,
            issuer: claims.iss,
            email,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use std::{
        io::Read,
        process::{Command, Stdio},
    };
    // Generate disposable synthetic RSA material in memory; no private PEM lives in source, disk or test logs.
    fn synthetic_keys() -> (EncodingKey, JwkSet) {
        let script = "const c=require('node:crypto');const {privateKey,publicKey}=c.generateKeyPairSync('rsa',{modulusLength:2048});process.stdout.write(JSON.stringify({privateKey:privateKey.export({format:'pem',type:'pkcs8'}),jwk:{...publicKey.export({format:'jwk'}),kid:'synthetic-key',alg:'RS256',use:'sig'}}))";
        let mut child = Command::new("node")
            .args(["-e", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("Node 22 is required for disposable synthetic key tests");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "Synthetic key generator failed");
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Synthetic key generator exceeded its deadline")
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut data = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take(16385)
            .read_to_end(&mut data)
            .unwrap();
        assert!(data.len() <= 16384);
        let value: serde_json::Value = serde_json::from_slice(&data).unwrap();
        let encoding =
            EncodingKey::from_rsa_pem(value["privateKey"].as_str().unwrap().as_bytes()).unwrap();
        let set =
            serde_json::from_value(serde_json::json!({"keys":[value["jwk"].clone()]})).unwrap();
        (encoding, set)
    }
    #[tokio::test]
    async fn cached_synthetic_google_keys_validate_signature_audience_issuer_nonce_and_account_claims()
     {
        let google =
            Google::new(Config::new("1234567890-test.apps.googleusercontent.com".into()).unwrap())
                .unwrap();
        let (encoding, set) = synthetic_keys();
        {
            let mut cache = google.keys.lock().await;
            cache.set = set;
            cache.until = Instant::now() + Duration::from_secs(60);
            cache.last_attempt = Some(Instant::now());
        }
        let nonce = crate::jwt::opaque();
        let now = crate::jwt::now();
        let claims = serde_json::json!({"sub":"12345678901234567890","iss":"https://accounts.google.com","aud":google.client_id(),"exp":now+3600,"iat":now,"nonce":nonce,"email":"synthetic@example.test","email_verified":true});
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("synthetic-key".into());
        let token = encode(&header, &claims, &encoding).unwrap();
        let identity = google.verify(&token, &nonce).await.unwrap().unwrap();
        assert_eq!(identity.subject, "12345678901234567890");
        assert_eq!(identity.email, "synthetic@example.test");
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
