//! Literal webhook discovery and private environment resolution. Source contains references, never key/URL values.
use brain_backend::webhooks::{Config, Inbound, Outbound, Secret};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
const SOURCE: &str = "server/webhooks/config.jsbrain";
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Definition {
    encryption_keys: Vec<String>,
    #[serde(default)]
    inbound: BTreeMap<String, Incoming>,
    #[serde(default)]
    outbound: BTreeMap<String, Outgoing>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Incoming {
    queue: String,
    signing_keys: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Outgoing {
    url_env: String,
    signing_key: String,
    allowed_origins: Vec<String>,
}
fn reference(name: &str) -> bool {
    name.len() <= 96
        && name.strip_prefix("BRAIN_WEBHOOK_").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        })
}
fn definition(root: &Path) -> Result<Option<Definition>, String> {
    if let Err(error) = root.join(SOURCE).symlink_metadata() {
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(format!("{SOURCE}: source is unreadable"));
    }
    // Do not include parser snippets/serde values: a malformed source may accidentally contain a pasted key.
    let value=crate::backend::read_contract(root,&root.join(SOURCE)).map_err(|_|format!("{SOURCE}: use one regular literal configuration <=64 KiB with environment references, not secret values"))?;
    let definition: Definition = serde_json::from_value(value).map_err(|_| {
        format!("{SOURCE}: use encryptionKeys, inbound and outbound with the documented fields")
    })?;
    if definition.encryption_keys.is_empty()
        || definition.encryption_keys.len() > 2
        || definition.inbound.len() > 16
        || definition.outbound.len() > 16
        || definition.inbound.len() + definition.outbound.len() == 0
    {
        return Err(format!(
            "{SOURCE}: configure 1..2 encryption references and 1..32 endpoints (at most 16 per direction)"
        ));
    }
    for (name, hook) in &definition.inbound {
        if !brain_backend::contract::identifier(name)
            || name.len() > 48
            || !brain_backend::contract::identifier(&hook.queue)
            || hook.queue == brain_backend::webhooks::OUTBOUND_QUEUE
            || hook.signing_keys.is_empty()
            || hook.signing_keys.len() > 2
        {
            return Err(format!(
                "{SOURCE}: inbound names/queues must be identifiers with one or two signing references"
            ));
        }
    }
    for (name, hook) in &definition.outbound {
        if !brain_backend::contract::identifier(name)
            || name.len() > 48
            || hook.allowed_origins.is_empty()
            || hook.allowed_origins.len() > 8
            || hook.allowed_origins.iter().any(|o| {
                o.len() > 512
                    || !o.starts_with("https://")
                    || brain_backend::links::origin(o).as_deref() != Some(o.as_str())
            })
        {
            return Err(format!(
                "{SOURCE}: outbound names must be identifiers with explicit canonical HTTPS origins"
            ));
        }
    }
    if names(&definition).iter().any(|n| !reference(n)) {
        return Err(format!(
            "{SOURCE}: key and URL references must be BRAIN_WEBHOOK_* environment names, never literal credentials or target URLs"
        ));
    }
    Ok(Some(definition))
}
fn names(config: &Definition) -> Vec<String> {
    let mut names = config.encryption_keys.clone();
    for hook in config.inbound.values() {
        names.extend(hook.signing_keys.iter().cloned())
    }
    for hook in config.outbound.values() {
        names.push(hook.url_env.clone());
        names.push(hook.signing_key.clone())
    }
    names.sort();
    names.dedup();
    names
}
pub fn references(root: &Path) -> Result<Vec<String>, String> {
    Ok(definition(root)?.as_ref().map(names).unwrap_or_default())
}
pub fn metadata(root: &Path) -> Result<Value, String> {
    let Some(config) = definition(root)? else {
        return Ok(json!({"configuredSource":false}));
    };
    Ok(
        json!({"configuredSource":true,"runtimeReadiness":"unverified","sourceFile":SOURCE,"environmentReferences":names(&config),"inbound":config.inbound.iter().map(|(name,h)|json!({"name":name,"queue":h.queue})).collect::<Vec<_>>(),"outbound":config.outbound.iter().map(|(name,h)|json!({"name":name,"allowedOrigins":h.allowed_origins})).collect::<Vec<_>>(),"automaticResourceSubscription":false}),
    )
}
fn value(root: &Path, environment: &str, name: &str) -> Result<String, String> {
    crate::env::value(root, environment, name).ok_or_else(|| {
        format!(
            "{SOURCE}: configure private {name} for this environment; values are never returned"
        )
    })
}
fn key(root: &Path, environment: &str, name: &str) -> Result<Secret, String> {
    Secret::from_hex(&value(root,environment,name)?).map_err(|_|format!("{SOURCE}: private {name} must contain an independently generated 32-byte key encoded as 64 hex characters"))
}
pub fn load(root: &Path, environment: &str) -> Result<Option<Config>, String> {
    let Some(config) = definition(root)? else {
        return Ok(None);
    };
    let keys = config
        .encryption_keys
        .iter()
        .map(|name| key(root, environment, name))
        .collect::<Result<Vec<_>, _>>()?;
    let inbound = config
        .inbound
        .into_iter()
        .map(|(name, h)| {
            let keys = h
                .signing_keys
                .iter()
                .map(|k| key(root, environment, k))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((name, Inbound::new(h.queue, keys)?))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let outbound = config
        .outbound
        .into_iter()
        .map(|(name, h)| {
            Ok((
                name,
                Outbound::new(
                    value(root, environment, &h.url_env)?,
                    key(root, environment, &h.signing_key)?,
                    h.allowed_origins,
                )?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    Config::new(keys,inbound,outbound).map(Some).map_err(|_|format!("{SOURCE}: keep endpoint signing keys distinct and separate from encryption keys; configured values were not captured"))
}

#[cfg(test)]
mod tests {
    use super::*;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct App(std::path::PathBuf);
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn app() -> App {
        let root = std::env::temp_dir().join(format!(
            "brain-webhook-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("server/webhooks")).unwrap();
        App(root)
    }
    #[test]
    fn metadata_is_available_without_credentials_and_never_resolves_private_urls_or_keys() {
        let app = app();
        std::fs::write(app.0.join(SOURCE),"export default { encryptionKeys: ['BRAIN_WEBHOOK_ENCRYPTION_KEY'], inbound: { orders: { queue: 'order_received', signingKeys: ['BRAIN_WEBHOOK_ORDER_KEY'] } }, outbound: { partner: { urlEnv: 'BRAIN_WEBHOOK_PARTNER_URL', signingKey: 'BRAIN_WEBHOOK_PARTNER_KEY', allowedOrigins: ['https://partner.example.test'] } } }").unwrap();
        let info = metadata(&app.0).unwrap();
        assert_eq!(info["runtimeReadiness"], "unverified");
        assert_eq!(references(&app.0).unwrap().len(), 4);
        assert!(load(&app.0, "development").is_err());
        let encoded = info.to_string();
        assert!(!encoded.contains("ciphertext"));
        assert_eq!(info["outbound"][0]["name"], "partner");
    }
    #[test]
    fn literal_key_or_url_values_and_oversized_reference_lists_fail_without_echoing_source() {
        let app = app();
        let leaked = "synthetic-private-marker-that-must-not-be-echoed";
        std::fs::write(app.0.join(SOURCE),format!("export default {{ encryptionKeys: ['{leaked}'], outbound: {{ partner: {{ urlEnv: 'https://partner.example.test/private?key={leaked}', signingKey: 'BRAIN_WEBHOOK_PARTNER_KEY', allowedOrigins: ['https://partner.example.test'] }} }} }}")).unwrap();
        let error = metadata(&app.0).unwrap_err();
        assert!(!error.contains(leaked));
        assert!(error.contains("BRAIN_WEBHOOK_*"));
    }
}
