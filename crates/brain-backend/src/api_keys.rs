//! Project API keys: provisioning, verification, scopes, quotas and an append-only request trace.
//!
//! Format: `bk_live_<prefix>_<secret>` where `prefix` is 12 lowercase hex characters (public, indexed) and
//! `secret` is 64 lowercase hex characters (244 random bits). Only `sha256(secret)` is stored and compared in
//! constant time. A key always acts as its owner: the owner's current role is loaded on every request and the
//! resource policy still applies, so scopes can only narrow what the owner may do, never widen it.
use crate::auth::{User, digest};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use uuid::Uuid;

pub const KEY_PREFIX: &str = "bk_live_";
/// Header that carries a key without using `Authorization`.
pub const HEADER: &str = "x-api-key";
pub const MAX_KEYS_PER_USER: i64 = 50;

/// Opt-in settings (`apiKeys` in the backend contract). `None` on the server config disables keys entirely.
#[derive(Clone, Debug)]
pub struct Config {
    /// Default requests per UTC day for keys without their own quota. `None` = unlimited.
    pub default_daily_quota: Option<i64>,
    /// Default requests per UTC month for keys without their own quota. `None` = unlimited.
    pub default_monthly_quota: Option<i64>,
    /// Days of request trace kept in `_brain_api_key_events`.
    pub retention_days: i64,
    /// Burst-smoothing limit per key, independent of the per-IP limiter.
    pub per_key_per_minute: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            default_daily_quota: None,
            default_monthly_quota: None,
            retention_days: 90,
            per_key_per_minute: 600,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3650).contains(&self.retention_days) {
            return Err("apiKeys.retentionDays must be 1..3650".into());
        }
        if self.per_key_per_minute == 0 || self.per_key_per_minute > 100_000 {
            return Err("apiKeys.perKeyPerMinute must be 1..100000".into());
        }
        for quota in [self.default_daily_quota, self.default_monthly_quota]
            .into_iter()
            .flatten()
        {
            if quota < 1 {
                return Err("apiKeys default quotas must be positive".into());
            }
        }
        Ok(())
    }
}

/// Returns the raw key from `X-API-Key` or `Authorization: Bearer bk_...`, if one was presented.
pub fn presented(headers: &axum::http::HeaderMap) -> Option<&str> {
    if let Some(value) = headers.get(HEADER) {
        return Some(value.to_str().unwrap_or(""));
    }
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| token.starts_with("bk_"))
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}
/// Splits a well-formed key into (prefix, secret).
pub fn parse(raw: &str) -> Option<(&str, &str)> {
    let (prefix, secret) = raw.strip_prefix(KEY_PREFIX)?.split_once('_')?;
    (hex(prefix, 12) && hex(secret, 64)).then_some((prefix, secret))
}
fn generate() -> (String, String) {
    let prefix = Uuid::new_v4().simple().to_string()[..12].to_string();
    let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    (prefix, secret)
}
/// `resource:action` where resource is a resource name or `*` and action is `read`, `write` or `*`.
pub fn valid_scope(scope: &str) -> bool {
    scope.split_once(':').is_some_and(|(resource, action)| {
        (resource == "*" || crate::contract::identifier(resource))
            && matches!(action, "read" | "write" | "*")
    })
}
pub fn allows(scopes: &[String], resource: &str, action: &str) -> bool {
    scopes.iter().any(|scope| {
        scope.split_once(':').is_some_and(|(r, a)| {
            (r == "*" || r == resource) && (a == "*" || a == action)
        })
    })
}
/// The HTTP method decides the action: safe methods read, everything else writes.
pub fn action(method: &axum::http::Method) -> &'static str {
    if matches!(*method, axum::http::Method::GET | axum::http::Method::HEAD) {
        "read"
    } else {
        "write"
    }
}

#[derive(Clone, Debug)]
pub struct QuotaState {
    pub limit: i64,
    pub remaining: i64,
    pub reset_seconds: i64,
}
#[derive(Clone, Debug)]
pub struct Principal {
    pub key_id: Uuid,
    pub prefix: String,
    pub user: User,
    pub scopes: Vec<String>,
    daily: Option<i64>,
    monthly: Option<i64>,
}
#[derive(Debug)]
pub enum Denied {
    Invalid,
    Revoked(Uuid),
    Expired(Uuid),
}
pub enum Admission {
    Allowed(Option<QuotaState>),
    RateLimited,
    QuotaExhausted(QuotaState),
}

/// One traced request. Secrets, headers, bodies and query values are never captured.
#[derive(Clone, Debug)]
pub struct Event {
    pub key_id: Uuid,
    pub rid: Option<String>,
    pub method: String,
    pub route: String,
    pub path: String,
    pub resource: Option<String>,
    pub action: Option<String>,
    pub status: i32,
    pub latency_ms: i32,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    pub bytes_out: Option<i64>,
    pub query_keys: Vec<String>,
}

#[derive(Clone)]
pub struct Keys {
    pool: PgPool,
    config: Config,
    events: mpsc::Sender<Event>,
    dropped: Arc<AtomicU64>,
    limiter: Arc<DefaultKeyedRateLimiter<Uuid>>,
    shared: bool,
}

/// Public key fields as JSON; never includes the secret hash.
macro_rules! key_json {
    () => {
        "json_build_object('id',k.id,'userId',k.user_id,'name',k.name,'prefix',k.prefix,'scopes',k.scopes,'dailyQuota',k.daily_quota,'monthlyQuota',k.monthly_quota,'createdAt',k.created_at,'updatedAt',k.updated_at,'lastUsedAt',k.last_used_at,'expiresAt',k.expires_at,'revokedAt',k.revoked_at,'rotatedFrom',k.rotated_from,'usedToday',coalesce((SELECT u.count FROM public._brain_api_key_usage u WHERE u.key_id=k.id AND u.period='day' AND u.starts_on=(now() AT TIME ZONE 'UTC')::date),0),'usedThisMonth',coalesce((SELECT u.count FROM public._brain_api_key_usage u WHERE u.key_id=k.id AND u.period='month' AND u.starts_on=date_trunc('month',now() AT TIME ZONE 'UTC')::date),0))"
    };
}

pub struct NewKey {
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_in_days: Option<i64>,
    pub daily_quota: Option<i64>,
    pub monthly_quota: Option<i64>,
}
impl NewKey {
    pub fn validate(&self) -> Result<(), &'static str> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err("name must be 1..80 printable characters");
        }
        if self.scopes.is_empty() || self.scopes.len() > 32 || !self.scopes.iter().all(|s| valid_scope(s)) {
            return Err("scopes must be 1..32 entries shaped resource:read|write|* (resource may be *)");
        }
        if self.expires_in_days.is_some_and(|d| !(1..=3650).contains(&d)) {
            return Err("expiresInDays must be 1..3650");
        }
        if [self.daily_quota, self.monthly_quota]
            .into_iter()
            .flatten()
            .any(|q| q < 1)
        {
            return Err("quotas must be positive integers");
        }
        Ok(())
    }
}

impl Keys {
    /// Spawns the trace writer and the retention sweeper on the current Tokio runtime.
    pub fn new(pool: PgPool, config: Config) -> Result<Self, String> {
        config.validate()?;
        let (events, receiver) = mpsc::channel(4096);
        tokio::spawn(write_events(pool.clone(), receiver));
        tokio::spawn(retain(pool.clone(), config.retention_days));
        let per_minute = NonZeroU32::new(config.per_key_per_minute).ok_or("Invalid per-key limit")?;
        Ok(Self {
            limiter: Arc::new(RateLimiter::keyed(Quota::per_minute(per_minute))),
            pool,
            config,
            events,
            dropped: Arc::new(AtomicU64::new(0)),
            shared: false,
        })
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    /// Counts the per-minute limit in Postgres so every process of the deployment shares one budget per key.
    pub fn shared(mut self, shared: bool) -> Self {
        self.shared = shared;
        self
    }
    /// Trace events not written because the bounded channel was full.
    pub fn dropped_events(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    /// Never blocks the request: a full channel counts a drop instead.
    pub fn record(&self, event: Event) {
        if self.events.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub async fn create(&self, owner: Uuid, key: NewKey) -> Result<Result<(Value, String), &'static str>, String> {
        if let Err(error) = key.validate() {
            return Ok(Err(error));
        }
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        sqlx::query("SELECT id FROM public._brain_users WHERE id=$1 FOR UPDATE")
            .bind(owner)
            .fetch_optional(&mut *tx)
            .await
            .map_err(crate::db::safe)?
            .ok_or("Unknown key owner")?;
        let live: i64 = sqlx::query_scalar("SELECT count(*) FROM public._brain_api_keys WHERE user_id=$1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>now())")
            .bind(owner).fetch_one(&mut *tx).await.map_err(crate::db::safe)?;
        if live >= MAX_KEYS_PER_USER {
            return Ok(Err("Active key limit reached; revoke an unused key first"));
        }
        let issued = self.insert(&mut tx, owner, &key, None).await?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(Ok(issued))
    }
    async fn insert(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        owner: Uuid,
        key: &NewKey,
        rotated_from: Option<Uuid>,
    ) -> Result<(Value, String), String> {
        let (prefix, secret) = generate();
        let id = Uuid::new_v4();
        let mut scopes = key.scopes.clone();
        scopes.sort();
        scopes.dedup();
        let sql = concat!("WITH k AS (INSERT INTO public._brain_api_keys(id,user_id,name,prefix,secret_hash,scopes,daily_quota,monthly_quota,expires_at,rotated_from) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,CASE WHEN $9::bigint IS NULL THEN NULL ELSE now()+make_interval(days=>$9::int) END,$10) RETURNING *) SELECT ", key_json!(), " FROM k");
        let info: Value = sqlx::query_scalar(sql)
            .bind(id)
            .bind(owner)
            .bind(key.name.trim())
            .bind(&prefix)
            .bind(digest(&secret))
            .bind(&scopes)
            .bind(key.daily_quota)
            .bind(key.monthly_quota)
            .bind(key.expires_in_days)
            .bind(rotated_from)
            .fetch_one(&mut **tx)
            .await
            .map_err(crate::db::safe)?;
        Ok((info, format!("{KEY_PREFIX}{prefix}_{secret}")))
    }
    /// Lists keys. `owner = None` lists every key (admin/operator only); secrets are never returned.
    pub async fn list(&self, owner: Option<Uuid>) -> Result<Vec<Value>, String> {
        let sql = concat!("SELECT ", key_json!(), " FROM public._brain_api_keys k WHERE $1::uuid IS NULL OR k.user_id=$1 ORDER BY k.created_at DESC, k.id LIMIT 500");
        sqlx::query_scalar(sql)
            .bind(owner)
            .fetch_all(&self.pool)
            .await
            .map_err(crate::db::safe)
    }
    /// Resolves a key id or public prefix, restricted to `owner` unless `None`.
    pub async fn find(&self, reference: &str, owner: Option<Uuid>) -> Result<Option<Value>, String> {
        let id = Uuid::parse_str(reference).ok();
        let sql = concat!("SELECT ", key_json!(), " FROM public._brain_api_keys k WHERE (k.id=$1 OR k.prefix=$2) AND ($3::uuid IS NULL OR k.user_id=$3)");
        sqlx::query_scalar(sql)
            .bind(id)
            .bind(reference)
            .bind(owner)
            .fetch_optional(&self.pool)
            .await
            .map_err(crate::db::safe)
    }
    pub async fn revoke(&self, id: Uuid, owner: Option<Uuid>) -> Result<bool, String> {
        Ok(sqlx::query("UPDATE public._brain_api_keys SET revoked_at=coalesce(revoked_at,now()),updated_at=now() WHERE id=$1 AND ($2::uuid IS NULL OR user_id=$2)")
            .bind(id).bind(owner).execute(&self.pool).await.map_err(crate::db::safe)?.rows_affected() == 1)
    }
    /// Issues a new secret with the same name, scopes, quotas and remaining lifetime; the old key is revoked atomically.
    pub async fn rotate(&self, id: Uuid, owner: Option<Uuid>) -> Result<Option<(Value, String)>, String> {
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        let Some(row) = sqlx::query("UPDATE public._brain_api_keys SET revoked_at=now(),updated_at=now() WHERE id=$1 AND ($2::uuid IS NULL OR user_id=$2) AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>now()) RETURNING user_id,name,scopes,daily_quota,monthly_quota,ceil(extract(epoch FROM expires_at-now())/86400)::bigint AS days")
            .bind(id).bind(owner).fetch_optional(&mut *tx).await.map_err(crate::db::safe)? else {
            return Ok(None);
        };
        let key = NewKey {
            name: row.get("name"),
            scopes: row.get("scopes"),
            expires_in_days: row.get::<Option<i64>, _>("days").map(|d| d.max(1)),
            daily_quota: row.get("daily_quota"),
            monthly_quota: row.get("monthly_quota"),
        };
        let issued = self.insert(&mut tx, row.get("user_id"), &key, Some(id)).await?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(Some(issued))
    }

    /// Verifies the secret in constant time and loads the owner's current role.
    pub async fn authenticate(&self, raw: &str) -> Result<Result<Principal, Denied>, String> {
        let Some((prefix, secret)) = parse(raw) else {
            return Ok(Err(Denied::Invalid));
        };
        let row = sqlx::query("SELECT k.id,k.secret_hash,k.scopes,k.daily_quota,k.monthly_quota,k.revoked_at IS NOT NULL AS revoked,coalesce(k.expires_at<=now(),false) AS expired,u.id AS user_id,u.email,u.role,u.verified FROM public._brain_api_keys k JOIN public._brain_users u ON u.id=k.user_id WHERE k.prefix=$1 AND u.disabled_at IS NULL")
            .bind(prefix).fetch_optional(&self.pool).await.map_err(crate::db::safe)?;
        let presented = digest(secret);
        let stored: String = row
            .as_ref()
            .map(|r| r.get("secret_hash"))
            .unwrap_or_else(|| "0".repeat(64));
        let matches: bool = presented.as_bytes().ct_eq(stored.as_bytes()).into();
        let (Some(row), true) = (row, matches) else {
            return Ok(Err(Denied::Invalid));
        };
        let id: Uuid = row.get("id");
        if row.get::<bool, _>("revoked") {
            return Ok(Err(Denied::Revoked(id)));
        }
        if row.get::<bool, _>("expired") {
            return Ok(Err(Denied::Expired(id)));
        }
        Ok(Ok(Principal {
            key_id: id,
            prefix: prefix.to_string(),
            scopes: row.get("scopes"),
            daily: row.get::<Option<i64>, _>("daily_quota").or(self.config.default_daily_quota),
            monthly: row
                .get::<Option<i64>, _>("monthly_quota")
                .or(self.config.default_monthly_quota),
            user: User {
                id: row.get("user_id"),
                email: row.get("email"),
                role: row.get("role"),
                verified: row.get("verified"),
            },
        }))
    }
    /// Applies the per-key rate limit, then consumes one request from the UTC day and month counters atomically.
    pub async fn admit(&self, principal: &Principal) -> Result<Admission, String> {
        if self.shared {
            let window = std::time::Duration::from_secs(60);
            if crate::cluster::hit(&self.pool, "api-key", &principal.key_id.to_string(), self.config.per_key_per_minute, window).await?.is_err() {
                return Ok(Admission::RateLimited);
            }
        } else if self.limiter.check_key(&principal.key_id).is_err() {
            return Ok(Admission::RateLimited);
        }
        if self.limiter.len() > 10_000 {
            self.limiter.retain_recent();
        }
        let mut tx = self.pool.begin().await.map_err(crate::db::safe)?;
        let resets = sqlx::query("SELECT extract(epoch FROM date_trunc('day',now() AT TIME ZONE 'UTC')+interval '1 day'-(now() AT TIME ZONE 'UTC'))::bigint AS day, extract(epoch FROM date_trunc('month',now() AT TIME ZONE 'UTC')+interval '1 month'-(now() AT TIME ZONE 'UTC'))::bigint AS month")
            .fetch_one(&mut *tx).await.map_err(crate::db::safe)?;
        let mut tightest: Option<QuotaState> = None;
        for (period, sql, limit) in [
            ("day", "INSERT INTO public._brain_api_key_usage(key_id,period,starts_on,count) VALUES ($1,'day',(now() AT TIME ZONE 'UTC')::date,1) ON CONFLICT (key_id,period,starts_on) DO UPDATE SET count=public._brain_api_key_usage.count+1,updated_at=now() WHERE public._brain_api_key_usage.count<$2 RETURNING count", principal.daily),
            ("month", "INSERT INTO public._brain_api_key_usage(key_id,period,starts_on,count) VALUES ($1,'month',date_trunc('month',now() AT TIME ZONE 'UTC')::date,1) ON CONFLICT (key_id,period,starts_on) DO UPDATE SET count=public._brain_api_key_usage.count+1,updated_at=now() WHERE public._brain_api_key_usage.count<$2 RETURNING count", principal.monthly),
        ] {
            let reset: i64 = resets.get(period);
            let used: Option<i64> = sqlx::query_scalar(sql)
                .bind(principal.key_id)
                .bind(limit.unwrap_or(i64::MAX))
                .fetch_optional(&mut *tx)
                .await
                .map_err(crate::db::safe)?;
            let Some(limit) = limit else { continue };
            let Some(used) = used else {
                return Ok(Admission::QuotaExhausted(QuotaState {
                    limit,
                    remaining: 0,
                    reset_seconds: reset.max(1),
                }));
            };
            let state = QuotaState {
                limit,
                remaining: (limit - used).max(0),
                reset_seconds: reset.max(1),
            };
            if tightest.as_ref().is_none_or(|t| state.remaining < t.remaining) {
                tightest = Some(state);
            }
        }
        sqlx::query("UPDATE public._brain_api_keys SET last_used_at=now() WHERE id=$1 AND (last_used_at IS NULL OR last_used_at<now()-interval '1 minute')")
            .bind(principal.key_id).execute(&mut *tx).await.map_err(crate::db::safe)?;
        tx.commit().await.map_err(crate::db::safe)?;
        Ok(Admission::Allowed(tightest))
    }

    /// Keyset-paginated trace for one key, newest first.
    pub async fn events(&self, key: Uuid, filter: &EventFilter) -> Result<Value, String> {
        let limit = filter.limit.unwrap_or(50).clamp(1, 200);
        let rows: Vec<Value> = sqlx::query_scalar("SELECT json_build_object('id',e.id,'at',e.at,'rid',e.rid,'method',e.method,'route',e.route,'path',e.path,'resource',e.resource,'action',e.action,'status',e.status,'latencyMs',e.latency_ms,'clientIp',e.client_ip,'userAgent',e.user_agent,'bytesOut',e.bytes_out,'queryKeys',e.query_keys) FROM public._brain_api_key_events e WHERE e.key_id=$1 AND ($2::timestamptz IS NULL OR e.at>=$2::timestamptz) AND ($3::timestamptz IS NULL OR e.at<$3::timestamptz) AND ($4::int IS NULL OR e.status=$4) AND ($5::text IS NULL OR e.resource=$5) AND ($6::bigint IS NULL OR e.id<$6) ORDER BY e.id DESC LIMIT $7")
            .bind(key).bind(filter.from.as_deref()).bind(filter.to.as_deref()).bind(filter.status).bind(filter.resource.as_deref()).bind(filter.before).bind(limit)
            .fetch_all(&self.pool).await.map_err(crate::db::safe)?;
        let next = if rows.len() as i64 == limit {
            rows.last().and_then(|row| row["id"].as_i64())
        } else {
            None
        };
        Ok(json!({"items":rows,"next":next}))
    }
    /// Requests per UTC day grouped by resource, action and status.
    pub async fn summary(&self, key: Uuid, days: i64) -> Result<Value, String> {
        let rows: Vec<Value> = sqlx::query_scalar("SELECT json_build_object('day',(e.at AT TIME ZONE 'UTC')::date,'resource',e.resource,'action',e.action,'status',e.status,'count',count(*)) FROM public._brain_api_key_events e WHERE e.key_id=$1 AND e.at>=now()-make_interval(days=>$2::int) GROUP BY (e.at AT TIME ZONE 'UTC')::date,e.resource,e.action,e.status ORDER BY (e.at AT TIME ZONE 'UTC')::date DESC,e.resource,e.action,e.status LIMIT 1000")
            .bind(key).bind(days.clamp(1, 366)).fetch_all(&self.pool).await.map_err(crate::db::safe)?;
        Ok(json!({"days":days.clamp(1,366),"items":rows,"droppedEvents":self.dropped_events()}))
    }
}

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventFilter {
    pub from: Option<String>,
    pub to: Option<String>,
    pub status: Option<i32>,
    pub resource: Option<String>,
    pub before: Option<i64>,
    pub limit: Option<i64>,
}
impl EventFilter {
    pub fn validate(&self) -> Result<(), &'static str> {
        let stamp = |s: &Option<String>| {
            s.as_ref().is_none_or(|s| {
                (10..=35).contains(&s.len())
                    && s.bytes().all(|c| c.is_ascii_digit() || b"-:T.+Z ".contains(&c))
                    && s.as_bytes()[..4].iter().all(u8::is_ascii_digit)
            })
        };
        if !stamp(&self.from) || !stamp(&self.to) {
            return Err("from/to must be ISO-8601 timestamps");
        }
        if self.status.is_some_and(|s| !(100..=599).contains(&s))
            || self.resource.as_ref().is_some_and(|r| !crate::contract::identifier(r))
            || self.before.is_some_and(|b| b < 1)
            || self.limit.is_some_and(|l| !(1..=200).contains(&l))
        {
            return Err("status 100..599, resource identifier, before >= 1, limit 1..200");
        }
        Ok(())
    }
}

async fn write_events(pool: PgPool, mut receiver: mpsc::Receiver<Event>) {
    let mut batch = Vec::with_capacity(256);
    while let Some(first) = receiver.recv().await {
        batch.push(first);
        while batch.len() < 256 {
            match receiver.try_recv() {
                Ok(event) => batch.push(event),
                Err(_) => break,
            }
        }
        let column = |f: fn(&Event) -> Option<String>| batch.iter().map(f).collect::<Vec<_>>();
        let result = sqlx::query("INSERT INTO public._brain_api_key_events(key_id,rid,method,route,path,resource,action,status,latency_ms,client_ip,user_agent,bytes_out,query_keys) SELECT k,r,m,ro,p,re,a,s,l,ip,ua,b,CASE WHEN q='' THEN '{}'::text[] ELSE string_to_array(q,',') END FROM UNNEST($1::uuid[],$2::text[],$3::text[],$4::text[],$5::text[],$6::text[],$7::text[],$8::int[],$9::int[],$10::text[],$11::text[],$12::bigint[],$13::text[]) AS t(k,r,m,ro,p,re,a,s,l,ip,ua,b,q) WHERE EXISTS (SELECT 1 FROM public._brain_api_keys x WHERE x.id=t.k)")
            .bind(batch.iter().map(|e| e.key_id).collect::<Vec<_>>())
            .bind(column(|e| e.rid.clone()))
            .bind(batch.iter().map(|e| e.method.clone()).collect::<Vec<_>>())
            .bind(batch.iter().map(|e| e.route.clone()).collect::<Vec<_>>())
            .bind(batch.iter().map(|e| e.path.clone()).collect::<Vec<_>>())
            .bind(column(|e| e.resource.clone()))
            .bind(column(|e| e.action.clone()))
            .bind(batch.iter().map(|e| e.status).collect::<Vec<_>>())
            .bind(batch.iter().map(|e| e.latency_ms).collect::<Vec<_>>())
            .bind(column(|e| e.client_ip.clone()))
            .bind(column(|e| e.user_agent.clone()))
            .bind(batch.iter().map(|e| e.bytes_out).collect::<Vec<_>>())
            .bind(batch.iter().map(|e| e.query_keys.join(",")).collect::<Vec<_>>())
            .execute(&pool)
            .await;
        if result.is_err() {
            eprintln!(
                "{}",
                json!({"event":"api-key-trace-write-failed","dropped":batch.len(),"private":true,"valuesCaptured":false})
            );
        }
        batch.clear();
    }
}

/// Purges trace rows older than the retention window on boot and then daily, in bounded batches.
async fn retain(pool: PgPool, days: i64) {
    let mut tick = tokio::time::interval(Duration::from_secs(86_400));
    loop {
        tick.tick().await;
        let _ = purge(&pool, days).await;
    }
}
pub async fn purge(pool: &PgPool, days: i64) -> Result<u64, String> {
    let mut total = 0;
    loop {
        let removed = sqlx::query("DELETE FROM public._brain_api_key_events WHERE id IN (SELECT id FROM public._brain_api_key_events WHERE at<now()-make_interval(days=>$1::int) ORDER BY at LIMIT 5000)")
            .bind(days).execute(pool).await.map_err(crate::db::safe)?.rows_affected();
        total += removed;
        if removed < 5000 {
            break;
        }
    }
    sqlx::query("DELETE FROM public._brain_api_key_usage WHERE starts_on<(now() AT TIME ZONE 'UTC')::date-400")
        .execute(pool)
        .await
        .map_err(crate::db::safe)?;
    Ok(total)
}

/// Adds the API-key security schemes and the key routes to a generated OpenAPI document.
pub fn document(doc: &mut Value) {
    let Some(root) = doc.as_object_mut() else { return };
    let components = root.entry("components").or_insert_with(|| json!({}));
    if let Some(components) = components.as_object_mut() {
        let schemes = components.entry("securitySchemes").or_insert_with(|| json!({}));
        if let Some(schemes) = schemes.as_object_mut() {
            schemes.insert("apiKey".into(), json!({"type":"apiKey","in":"header","name":"X-API-Key","description":"Project API key bk_live_<prefix>_<secret>; accepted only on /api resource routes. Scopes resource:read|write."}));
            schemes.insert("apiKeyBearer".into(), json!({"type":"http","scheme":"bearer","bearerFormat":"bk_live_<prefix>_<secret>","description":"The same project API key sent as Authorization: Bearer."}));
        }
    }
    let paths = root.entry("paths").or_insert_with(|| json!({}));
    if let Some(paths) = paths.as_object_mut() {
        for (path, item) in paths.iter_mut() {
            if !path.starts_with("/api/") {
                continue;
            }
            if let Some(item) = item.as_object_mut() {
                for (_, operation) in item.iter_mut() {
                    if let Some(operation) = operation.as_object_mut() {
                        let security = operation.entry("security").or_insert_with(|| json!([]));
                        if let Some(list) = security.as_array_mut() {
                            list.push(json!({"apiKey":[]}));
                            list.push(json!({"apiKeyBearer":[]}));
                        }
                    }
                }
            }
        }
        paths.insert("/_brain/keys".into(), json!({
            "get":{"summary":"List your API keys (admins: ?all=true)","responses":{"200":{"description":"Keys without secrets"}}},
            "post":{"summary":"Create an API key; the secret is returned once","responses":{"200":{"description":"Created key and its one-time secret"}}}}));
        paths.insert("/_brain/keys/{id}".into(), json!({"delete":{"summary":"Revoke an API key","responses":{"200":{"description":"Revoked"}}}}));
        paths.insert("/_brain/keys/{id}/rotate".into(), json!({"post":{"summary":"Rotate: issue a new secret and revoke the old key","responses":{"200":{"description":"New key and its one-time secret"}}}}));
        paths.insert("/_brain/keys/{id}/events".into(), json!({"get":{"summary":"Request trace for one key (keyset pagination)","responses":{"200":{"description":"Events"}}}}));
        paths.insert("/_brain/keys/{id}/summary".into(), json!({"get":{"summary":"Daily request counts by resource, action and status","responses":{"200":{"description":"Summary"}}}}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_parse_scopes_match_and_never_widen() {
        let raw = format!("{KEY_PREFIX}0123456789ab_{}", "f".repeat(64));
        assert_eq!(parse(&raw).unwrap().0, "0123456789ab");
        assert!(parse("bk_live_0123456789ab_short").is_none());
        assert!(parse(&raw.to_uppercase()).is_none());
        assert!(valid_scope("posts:read") && valid_scope("*:write") && valid_scope("posts:*"));
        assert!(!valid_scope("posts:delete") && !valid_scope("Posts:read") && !valid_scope("posts"));
        let scopes = vec!["posts:read".to_string(), "*:write".to_string()];
        assert!(allows(&scopes, "posts", "read"));
        assert!(!allows(&scopes, "orders", "read"));
        assert!(allows(&scopes, "orders", "write"));
    }
}
