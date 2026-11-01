//! Provider-neutral captcha verification on public write endpoints. Turnstile, hCaptcha and reCAPTCHA share one contract:
//! a form POST of `secret`, `response` and `remoteip` answered with JSON holding `success` and `hostname`
//! (official docs read 2026-10-31). A write without a verified token never runs: a rejected token is 403 and an unreachable
//! or unintelligible provider is 503. Tokens are single use, so verification never retries.
use reqwest::Url;
use serde_json::Value;
use std::{collections::BTreeSet, net::IpAddr, time::Duration};

pub const CONTRACT: &str = "server/auth/captcha.jsbrain";
pub const HEADER: &str = "x-brain-captcha";
/// Spent by the login right after a verified signup, instead of a second challenge (`crate::sessions`).
pub const LOGIN_GRANT_HEADER: &str = "x-brain-login-grant";
/// Lets an operator's script sign accounts up in a batch. Never forwarded by the site's own proxies.
pub const OPERATOR_HEADER: &str = "x-brain-operator-key";
const OPERATOR_KEY_MIN: usize = 32;
/// The providers' published always-pass secrets (docs read 2026-11-01): fine locally, an open door in production.
const TEST_SECRETS: &[&str] = &["1x0000000000000000000000000000000AA", "0x0000000000000000000000000000000000000000", "6LeIxAcTAAAAAGG-vFI1TnRWxMZNFuojJ4WifJWe"];
const AUTH_ACTIONS: &[&str] = &["signup", "login", "password_request"];
const MAX_TOKEN: usize = 8192;
const MAX_ANSWER: usize = 64 * 1024;

#[derive(Clone, Default)]
pub struct Captcha {
    verify_url: Option<Url>,
    secret: Option<String>,
    secret_env: String,
    hostname: Option<String>,
    protect: BTreeSet<String>,
    client: Option<reqwest::Client>,
    operator_env: Option<String>,
    operator_key: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    Rejected,
    Unavailable,
}
fn secure_or_local(url: &Url) -> bool {
    let local = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(name)) => name == "localhost",
        None => false,
    };
    url.username().is_empty() && url.password().is_none() && url.host().is_some() && (url.scheme() == "https" || url.scheme() == "http" && local)
}
impl Captcha {
    /// Reads the literal contract. `env` resolves the secret, which never appears in the contract.
    pub fn from_contract(value: &Value, env: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let fix = "fix: export { verify_url: 'https://...', secret_env: 'NAME', hostname: 'app.example.com', protect: ['signup', 'login', 'password_request', 'route:<name>'], operator_key_env: 'OPTIONAL_NAME' }";
        let map = value.as_object().ok_or_else(|| format!("{CONTRACT} must export an object; {fix}"))?;
        if let Some(key) = map.keys().find(|k| !matches!(k.as_str(), "verify_url" | "secret_env" | "hostname" | "protect" | "operator_key_env")) {
            return Err(format!("{CONTRACT}: unknown key `{key}`; {fix}"));
        }
        let text = |key: &str| map.get(key).and_then(Value::as_str).filter(|s| !s.is_empty());
        let url = text("verify_url").and_then(|u| Url::parse(u).ok()).filter(secure_or_local).ok_or_else(|| format!("{CONTRACT}: verify_url must be an https URL (http only on loopback, for a local fake); {fix}"))?;
        let secret_env = text("secret_env").filter(|n| n.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')).ok_or_else(|| format!("{CONTRACT}: secret_env must name an environment variable; {fix}"))?;
        let protect: Option<BTreeSet<String>> = map.get("protect").and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect());
        let protect = protect.filter(|p| !p.is_empty() && p.len() == map["protect"].as_array().map_or(0, Vec::len)).ok_or_else(|| format!("{CONTRACT}: protect must list what requires a token, without repeats; {fix}"))?;
        if let Some(name) = protect.iter().find(|p| !AUTH_ACTIONS.contains(&p.as_str()) && !p.strip_prefix("route:").is_some_and(|n| !n.is_empty())) {
            return Err(format!("{CONTRACT}: unknown protected action `{name}`; {fix}"));
        }
        if map.get("hostname").is_some() && text("hostname").is_none() {
            return Err(format!("{CONTRACT}: hostname must be a non-empty string; {fix}"));
        }
        let env_name = |n: &&str| n.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_');
        if map.get("operator_key_env").is_some() && text("operator_key_env").filter(env_name).is_none() {
            return Err(format!("{CONTRACT}: operator_key_env must name an environment variable; {fix}"));
        }
        let operator_env = text("operator_key_env").map(str::to_string);
        let operator_key = operator_env.as_deref().and_then(&env).filter(|key| !key.is_empty());
        Ok(Self { secret: env(secret_env).filter(|s| !s.is_empty()), secret_env: secret_env.to_string(), verify_url: Some(url), hostname: text("hostname").map(str::to_string), protect, client: None, operator_env, operator_key })
    }
    pub fn enabled(&self) -> bool {
        self.verify_url.is_some()
    }
    /// The variable a protected action needs and this environment does not hold; its name, never a value.
    pub fn missing_secret(&self) -> Option<&str> {
        (self.enabled() && self.secret.is_none()).then_some(self.secret_env.as_str())
    }
    pub fn protects(&self, action: &str) -> bool {
        self.protect.contains(action)
    }
    /// True only for the declared key, compared in constant time. An undeclared or unset key admits nobody.
    pub fn operator(&self, presented: Option<&str>) -> bool {
        let (Some(expected), Some(presented)) = (&self.operator_key, presented) else { return false };
        bool::from(subtle::ConstantTimeEq::ct_eq(crate::auth::digest(expected).as_bytes(), crate::auth::digest(presented).as_bytes()))
    }
    /// Metadata for `brain backend schema`; carries no secret or environment value.
    pub fn schema(&self) -> Value {
        serde_json::json!({"enabled": self.enabled(), "header": HEADER, "protect": self.protect, "secretEnv": self.secret_env, "operatorKeyEnv": self.operator_env})
    }
    /// Refuses a configuration that could never admit a request or that protects something unprotectable.
    pub(crate) fn ready(mut self, routes: &[crate::routes::Route], production: bool) -> Result<Self, String> {
        if !self.enabled() {
            return Ok(self);
        }
        for name in self.protect.iter().filter_map(|p| p.strip_prefix("route:")) {
            let route = routes.iter().find(|r| r.name == name).ok_or_else(|| format!("{CONTRACT}: protected route `{name}` is not declared; fix: name a route in server/routes"))?;
            if !matches!(route.access, crate::routes::Access::Public) || route.method == axum::http::Method::GET {
                return Err(format!("{CONTRACT}: route `{name}` must be a public write to be protected; fix: remove it from protect"));
            }
        }
        if production && self.secret.is_none() {
            return Err(format!("{CONTRACT}: {} is not set; fix: set it in the server environment", self.secret_env));
        }
        if production && self.secret.as_deref().is_some_and(|s| TEST_SECRETS.contains(&s)) {
            return Err(format!("{CONTRACT}: {} holds a provider's always-pass test secret; fix: set the real secret in the server environment", self.secret_env));
        }
        if let Some(name) = &self.operator_env {
            if production && self.operator_key.as_ref().is_none_or(|key| key.len() < OPERATOR_KEY_MIN) {
                return Err(format!("{CONTRACT}: {name} must hold a generated key of at least {OPERATOR_KEY_MIN} bytes; fix: set it in the server environment or remove operator_key_env"));
            }
        }
        if production && self.verify_url.as_ref().is_some_and(|u| u.scheme() != "https") {
            return Err(format!("{CONTRACT}: verify_url must use https in production"));
        }
        self.client = Some(
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(1))
                .timeout(Duration::from_secs(3))
                .build()
                .map_err(|_| format!("{CONTRACT}: HTTPS client unavailable"))?,
        );
        Ok(self)
    }
    /// Asks the provider about one token. Anything but an explicit `success: true` admits nothing.
    pub async fn verify(&self, token: Option<&str>, client_ip: Option<IpAddr>) -> Verdict {
        let Some(token) = token.filter(|t| !t.is_empty() && t.len() <= MAX_TOKEN) else { return Verdict::Rejected };
        let (Some(url), Some(secret), Some(client)) = (&self.verify_url, &self.secret, &self.client) else { return Verdict::Unavailable };
        let mut form = Url::parse("http://form.invalid/").expect("static URL");
        {
            let mut pairs = form.query_pairs_mut();
            pairs.append_pair("secret", secret).append_pair("response", token);
            if let Some(ip) = client_ip {
                pairs.append_pair("remoteip", &ip.to_string());
            }
        }
        let body = form.query().unwrap_or_default().to_string();
        let sent = client.post(url.clone()).header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded").body(body).send().await;
        let Ok(mut answer) = sent else { return Verdict::Unavailable };
        if !answer.status().is_success() {
            return Verdict::Unavailable;
        }
        let mut bytes = Vec::new();
        loop {
            match answer.chunk().await {
                Ok(Some(chunk)) if bytes.len() + chunk.len() <= MAX_ANSWER => bytes.extend_from_slice(&chunk),
                Ok(None) => break,
                _ => return Verdict::Unavailable,
            }
        }
        let Ok(Value::Object(verdict)) = serde_json::from_slice::<Value>(&bytes) else { return Verdict::Unavailable };
        match verdict.get("success") {
            Some(Value::Bool(true)) => {}
            Some(Value::Bool(false)) => {
                // A provider-side fault is not the visitor's failure (names shared by the three providers' docs).
                let ours = verdict.get("error-codes").and_then(Value::as_array).is_some_and(|codes| codes.iter().filter_map(Value::as_str).any(|c| matches!(c, "missing-input-secret" | "invalid-input-secret" | "internal-error" | "sitekey-secret-mismatch")));
                return if ours { Verdict::Unavailable } else { Verdict::Rejected };
            }
            _ => return Verdict::Unavailable,
        }
        match &self.hostname {
            Some(expected) if verdict.get("hostname").and_then(Value::as_str) != Some(expected.as_str()) => Verdict::Rejected,
            _ => Verdict::Passed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn production_refuses_what_would_leave_the_gate_open() {
        let check = |contract: Value, secret: &'static str, key: Option<&'static str>| {
            Captcha::from_contract(&contract, move |name| if name == "S" { Some(secret.to_string()) } else { key.map(str::to_string) }).unwrap().ready(&[], true).map(|_| ())
        };
        const KEY: &str = "synthetic-operator-key-0123456789abcdef";
        let https = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
        let plain = json!({"verify_url":https,"secret_env":"S","protect":["signup"]});
        let operated = json!({"verify_url":https,"secret_env":"S","protect":["signup"],"operator_key_env":"OPERATOR_KEY"});
        assert!(check(plain.clone(), "synthetic-captcha-secret", None).is_ok());
        assert!(check(plain.clone(), "1x0000000000000000000000000000000AA", None).unwrap_err().contains("test secret"));
        assert!(check(operated.clone(), "synthetic-captcha-secret", None).unwrap_err().contains("OPERATOR_KEY"));
        assert!(check(operated.clone(), "synthetic-captcha-secret", Some("short")).unwrap_err().contains("OPERATOR_KEY"));
        assert!(check(operated, "synthetic-captcha-secret", Some(KEY)).is_ok());
    }
}
