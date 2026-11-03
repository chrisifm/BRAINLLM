//! OpenID Connect ID-token plumbing shared by the fixed identity providers (Google, Apple). Each provider pins its own
//! key-set URL: a token never names where its keys live, and only RS256 signed with a plain key id is admitted.
use jsonwebtoken::{Algorithm, DecodingKey, decode_header, jwk::JwkSet};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
/// Produced only by a provider verifier after signature and fixed issuer/audience/nonce validation.
pub struct Identity {
    pub(crate) subject: String,
    pub(crate) issuer: String,
    /// Absent when the provider shares no usable address; such a subject signs in only once it is linked.
    pub(crate) email: Option<String>,
    /// The provider hosts the address, so the holder owns it today: only this links the subject to an existing account.
    pub(crate) linkable: bool,
    /// The address reaches only this holder, so an account created from it starts verified.
    pub(crate) verified: bool,
    /// Display name the provider shared, stored with a new link; a hint for the app, never an identity.
    pub(crate) name: Option<String>,
}
impl Identity {
    /// Keeps a display name only when it is plain, bounded text.
    pub fn with_name(mut self, name: Option<&str>) -> Self {
        self.name = name
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.chars().count() <= 200 && !name.chars().any(char::is_control))
            .map(str::to_owned);
        self
    }
}
struct Keys {
    set: JwkSet,
    until: Instant,
    last_attempt: Option<Instant>,
}
/// One provider's public keys: a fixed URL, at most sixteen keys in 64 KiB, refetched at most every thirty seconds, so
/// unknown key ids cannot trigger a fetch storm. Two verifications run at a time; the rest are told to retry.
#[derive(Clone)]
pub(crate) struct KeySet {
    url: &'static str,
    provider: &'static str,
    client: reqwest::Client,
    keys: Arc<Mutex<Keys>>,
    slots: Arc<Semaphore>,
}
impl KeySet {
    pub(crate) fn new(url: &'static str, provider: &'static str) -> Result<Self, String> {
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
            .map_err(|_| format!("{provider} identity verifier unavailable"))?;
        Ok(Self {
            url,
            provider,
            client,
            keys: Arc::new(Mutex::new(Keys {
                set: JwkSet { keys: vec![] },
                until: Instant::now(),
                last_attempt: None,
            })),
            slots: Arc::new(Semaphore::new(2)),
        })
    }
    /// Runs `check` with the token's signing key while holding a verification slot. `Ok(None)`: the header is not a
    /// plain RS256 header with a key id, or no published key has that id. `Err`: the provider's keys are unreachable.
    pub(crate) async fn verify<T>(
        &self,
        token: &str,
        check: impl FnOnce(&DecodingKey) -> Option<T>,
    ) -> Result<Option<T>, String> {
        let _permit = self
            .slots
            .try_acquire()
            .map_err(|_| format!("{} verifier busy; retry shortly", self.provider))?;
        let Some(kid) = rs256_kid(token) else {
            return Ok(None);
        };
        let Some(key) = self.key(&kid).await? else {
            return Ok(None);
        };
        Ok(check(&key))
    }
    async fn key(&self, kid: &str) -> Result<Option<DecodingKey>, String> {
        let provider = self.provider;
        let mut keys = tokio::time::timeout(Duration::from_millis(100), self.keys.lock())
            .await
            .map_err(|_| format!("{provider} verifier busy; retry shortly"))?;
        let time = Instant::now();
        if time < keys.until {
            if let Some(key) = keys.set.find(kid) {
                return Ok(DecodingKey::from_jwk(key).ok());
            }
        }
        if keys
            .last_attempt
            .is_some_and(|last| time.duration_since(last) < Duration::from_secs(30))
        {
            return Ok(None);
        }
        keys.last_attempt = Some(time);
        let mut response = self
            .client
            .get(self.url)
            .send()
            .await
            .map_err(|_| format!("{provider} public keys temporarily unavailable"))?;
        if !response.status().is_success()
            || response.content_length().is_some_and(|len| len > 65536)
        {
            return Err(format!("{provider} public keys unavailable"));
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
            .map_err(|_| format!("{provider} public key response interrupted"))?
        {
            if body.len() + chunk.len() > 65536 {
                return Err(format!("{provider} public key response exceeded its limit"));
            }
            body.extend_from_slice(&chunk);
        }
        let set: JwkSet = serde_json::from_slice(&body)
            .map_err(|_| format!("{provider} public key response is invalid"))?;
        if set.keys.is_empty() || set.keys.len() > 16 {
            return Err(format!("{provider} public key count exceeded its limit"));
        }
        let mut ids = std::collections::HashSet::new();
        if set.keys.iter().any(|key| {
            key.common
                .key_id
                .as_ref()
                .is_none_or(|kid| kid.len() > 128 || !ids.insert(kid.clone()))
        }) {
            return Err(format!("{provider} public key identities are invalid"));
        }
        keys.set = set;
        keys.until = Instant::now() + Duration::from_secs(ttl);
        Ok(keys
            .set
            .find(kid)
            .and_then(|key| DecodingKey::from_jwk(key).ok()))
    }
    #[cfg(test)]
    pub(crate) async fn preload(&self, set: JwkSet) {
        let mut cache = self.keys.lock().await;
        cache.set = set;
        cache.until = Instant::now() + Duration::from_secs(60);
        cache.last_attempt = Some(Instant::now());
    }
}
/// The key id of a token whose header is plain RS256: no critical extensions, no embedded or remote key, no encryption.
pub(crate) fn rs256_kid(token: &str) -> Option<String> {
    let header = decode_header(token).ok()?;
    if header.alg != Algorithm::RS256
        || header.crit.is_some()
        || header.jku.is_some()
        || header.jwk.is_some()
        || header.x5u.is_some()
        || header.enc.is_some()
        || header.zip.is_some()
    {
        return None;
    }
    header.kid.filter(|kid| {
        !kid.is_empty()
            && kid.len() <= 128
            && kid
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}
/// A bounded printable identifier from a provider claim (`sub`, `jti`).
pub(crate) fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

#[cfg(test)]
pub(crate) mod synthetic {
    use jsonwebtoken::{EncodingKey, jwk::JwkSet};
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    /// Disposable synthetic RSA material generated in memory; no private PEM lives in source, disk or test logs.
    pub(crate) fn keys(kid: &str) -> (EncodingKey, JwkSet) {
        let script = format!("const c=require('node:crypto');const {{privateKey,publicKey}}=c.generateKeyPairSync('rsa',{{modulusLength:2048}});process.stdout.write(JSON.stringify({{privateKey:privateKey.export({{format:'pem',type:'pkcs8'}}),jwk:{{...publicKey.export({{format:'jwk'}}),kid:{kid:?},alg:'RS256',use:'sig'}}}}))");
        let mut child = Command::new("node")
            .args(["-e", &script])
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
}
