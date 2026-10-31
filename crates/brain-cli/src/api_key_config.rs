//! Opt-in project API keys (`server/api-keys.jsbrain`) and the operator CLI `brain backend keys ...`.
use brain_backend::api_keys::{Config, EventFilter, Keys, NewKey};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

const SOURCE: &str = "server/api-keys.jsbrain";

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Definition {
    enabled: bool,
    daily_quota: Option<i64>,
    monthly_quota: Option<i64>,
    retention_days: Option<i64>,
    per_key_per_minute: Option<u32>,
}

/// `None` when the file is absent or `enabled` is false: any presented key is then rejected.
pub fn load(root: &Path) -> Result<Option<Config>, String> {
    if let Err(error) = root.join(SOURCE).symlink_metadata() {
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(format!("{SOURCE}: source is unreadable"));
    }
    let value = crate::backend::read_contract(root, &root.join(SOURCE))
        .map_err(|_| format!("{SOURCE}: use one regular literal configuration <=64 KiB"))?;
    let definition: Definition = serde_json::from_value(value).map_err(|_| {
        format!("{SOURCE}: use enabled, dailyQuota, monthlyQuota, retentionDays and perKeyPerMinute")
    })?;
    if !definition.enabled {
        return Ok(None);
    }
    let defaults = Config::default();
    let config = Config {
        default_daily_quota: definition.daily_quota,
        default_monthly_quota: definition.monthly_quota,
        retention_days: definition.retention_days.unwrap_or(defaults.retention_days),
        per_key_per_minute: definition
            .per_key_per_minute
            .unwrap_or(defaults.per_key_per_minute),
    };
    config.validate().map_err(|error| format!("{SOURCE}: {error}"))?;
    Ok(Some(config))
}

const USAGE: &str = "usage: brain backend keys create --user <uuid> --name <name> --scopes posts:read,posts:write [--expires-days N] [--daily N] [--monthly N] | list [--user <uuid>] | revoke <prefix|id> | rotate <prefix|id> | events <prefix|id> [--limit 50] [--status 200] [--resource posts] [--from ISO] [--to ISO] [--before ID] | summary <prefix|id> [--days 7] [--env development]";

fn number(args: &[String], flag: &str) -> Result<Option<i64>, String> {
    crate::flag(args, flag)
        .map(|v| v.parse().map_err(|_| format!("{flag} must be an integer")))
        .transpose()
}

/// Operator commands act on the database directly (no session); keys are created for an explicit user.
pub fn run(root: &Path, args: &[String], url: String) -> Result<(), String> {
    let config = load(root)?.ok_or(format!(
        "API keys are disabled; create {SOURCE} with {{\"enabled\": true}} first"
    ))?;
    let operation = args.first().map(String::as_str).unwrap_or("");
    if !matches!(
        operation,
        "create" | "list" | "revoke" | "rotate" | "events" | "summary"
    ) {
        return Err(USAGE.into());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let pool = brain_backend::db::connect(&url).await?;
        let keys = Keys::new(pool.clone(), config)?;
        let reference = || args.get(1).filter(|v| !v.starts_with("--")).ok_or_else(|| USAGE.to_string());
        let find = |reference: String| {
            let keys = keys.clone();
            async move {
                keys.find(&reference, None)
                    .await?
                    .ok_or_else(|| "No API key has that id or prefix".to_string())
            }
        };
        let id_of = |key: &Value| key["id"].as_str().and_then(|v| v.parse().ok()).ok_or("Malformed key id".to_string());
        let output: Value = match operation {
            "create" => {
                let owner = crate::flag(args, "--user")
                    .ok_or("create requires --user <uuid> of an existing user")?
                    .parse()
                    .map_err(|_| "--user must be a UUID")?;
                let key = NewKey {
                    name: crate::flag(args, "--name").ok_or("create requires --name")?,
                    scopes: crate::flag(args, "--scopes")
                        .ok_or("create requires --scopes resource:action[,...]")?
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect(),
                    expires_in_days: number(args, "--expires-days")?,
                    daily_quota: number(args, "--daily")?,
                    monthly_quota: number(args, "--monthly")?,
                };
                let (key, secret) = keys.create(owner, key).await?.map_err(str::to_string)?;
                json!({"schemaVersion":1,"key":key,"secret":secret,"note":"The secret is shown only once; store it in a secret manager"})
            }
            "list" => {
                let owner = crate::flag(args, "--user")
                    .map(|v| v.parse().map_err(|_| "--user must be a UUID"))
                    .transpose()?;
                json!({"schemaVersion":1,"items":keys.list(owner).await?})
            }
            "revoke" => {
                let key = find(reference()?.clone()).await?;
                json!({"schemaVersion":1,"ok":keys.revoke(id_of(&key)?, None).await?,"prefix":key["prefix"]})
            }
            "rotate" => {
                let key = find(reference()?.clone()).await?;
                let (key, secret) = keys
                    .rotate(id_of(&key)?, None)
                    .await?
                    .ok_or("Only an active, unexpired key can be rotated")?;
                json!({"schemaVersion":1,"key":key,"secret":secret,"note":"The secret is shown only once; the previous key is revoked"})
            }
            "events" => {
                let key = find(reference()?.clone()).await?;
                let filter = EventFilter {
                    from: crate::flag(args, "--from"),
                    to: crate::flag(args, "--to"),
                    status: number(args, "--status")?.map(|s| s as i32),
                    resource: crate::flag(args, "--resource"),
                    before: number(args, "--before")?,
                    limit: number(args, "--limit")?,
                };
                filter.validate().map_err(str::to_string)?;
                keys.events(id_of(&key)?, &filter).await?
            }
            _ => {
                let key = find(reference()?.clone()).await?;
                keys.summary(id_of(&key)?, number(args, "--days")?.unwrap_or(7)).await?
            }
        };
        pool.close().await;
        println!("{output}");
        Ok(())
    })
}
