//! First-party behavior analytics: bounded ingestion, retention and audit queries over the project's own Postgres.
//!
//! Privacy contract (docs/backend/analytics.md): the browser never sends form values; this module additionally drops
//! batches from bots and from browsers that send Global Privacy Control or Do Not Track, stores no IP address and no
//! user agent string, masks e-mail addresses and long digit runs in element names, and collapses identifier-like path
//! segments to `:id`. Every value enters SQL through bind parameters.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

/// Event kinds the collector emits. `track` carries an application-defined `name`.
pub const KINDS: &[&str] = &[
    "page", "leave", "click", "key", "focus", "input", "change", "submit", "toggle", "copy", "cut", "paste", "media",
    "drag", "drop", "context", "hover", "rage", "dead", "track",
];
/// Kinds that count as an interaction with an element (time to first interaction, element usage).
pub const INTERACTIONS: &[&str] = &[
    "click", "key", "input", "change", "submit", "toggle", "copy", "cut", "paste", "media", "drag", "drop", "context",
];
pub const MAX_EVENTS: usize = 64;
pub const MAX_BODY: usize = 32 * 1024;
pub const DEFAULT_RETENTION_DAYS: u32 = 180;

#[derive(Clone, Debug)]
pub struct Config {
    /// Events, sessions and visitors older than this many days are deleted by [`purge`] (1–3650).
    pub retention_days: u32,
    /// Optional proxy header holding an ISO 3166-1 alpha-2 country (for example `cf-ipcountry`). `None` stores none.
    pub country_header: Option<String>,
    /// Accepted events per visitor per minute (1–6000).
    pub events_per_minute: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self { retention_days: DEFAULT_RETENTION_DAYS, country_header: None, events_per_minute: 600 }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3650).contains(&self.retention_days) {
            return Err("analytics.retentionDays must be between 1 and 3650".into());
        }
        if !(1..=6000).contains(&self.events_per_minute) {
            return Err("analytics.eventsPerMinute must be between 1 and 6000".into());
        }
        if let Some(h) = &self.country_header
            && (h.is_empty() || h.len() > 64 || !h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        {
            return Err("analytics.countryHeader must be a lowercase HTTP header name".into());
        }
        Ok(())
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub v: u8,
    pub vid: Uuid,
    pub sid: Uuid,
    #[serde(default)]
    pub ctx: Option<SessionContext>,
    pub ev: Vec<Event>,
}
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct SessionContext {
    #[serde(default)]
    pub r#ref: Option<String>,
    #[serde(default)]
    pub utm: Option<Map<String, Value>>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub w: Option<u32>,
}
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub k: String,
    pub r: String,
    #[serde(default)]
    pub pv: u32,
    #[serde(default)]
    pub t: f64,
    #[serde(default)]
    pub at: f64,
    #[serde(default)]
    pub el: Option<Map<String, Value>>,
    #[serde(default)]
    pub v: Option<f64>,
    #[serde(default)]
    pub p: Option<Map<String, Value>>,
}

/// Request facts the HTTP layer extracts; never stored as such.
#[derive(Default, Debug, Clone)]
pub struct Signals {
    pub user_agent: Option<String>,
    pub do_not_track: bool,
    pub global_privacy_control: bool,
    pub country: Option<String>,
    pub user: Option<Uuid>,
    /// Application origins: a referrer from one of them is not an external source.
    pub origins: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Rows written.
    Stored(usize),
    /// Dropped on purpose (bot, Do Not Track, Global Privacy Control, nothing valid).
    Ignored(&'static str),
}
#[derive(Debug)]
pub enum IngestError {
    Invalid(&'static str),
    RateLimited,
    SessionConflict,
    Database(String),
}

const BOT_MARKERS: &[&str] = &[
    "bot", "crawl", "spider", "slurp", "headless", "lighthouse", "pagespeed", "preview", "facebookexternalhit",
    "curl/", "wget/", "python-requests", "python-urllib", "httpclient", "okhttp", "go-http-client", "java/", "phantomjs",
    "selenium", "puppeteer", "playwright", "webdriver", "monitor", "uptime", "pingdom", "scrapy", "axios/", "node-fetch",
];
/// Bot heuristic on the user agent. A missing user agent counts as automated.
pub fn is_bot(user_agent: Option<&str>) -> bool {
    let Some(ua) = user_agent.map(str::trim).filter(|ua| !ua.is_empty()) else { return true };
    let lower = ua.to_ascii_lowercase();
    BOT_MARKERS.iter().any(|m| lower.contains(m))
}
fn device(user_agent: Option<&str>) -> Option<&'static str> {
    let ua = user_agent?;
    Some(if ua.contains("iPad") || ua.contains("Tablet") {
        "tablet"
    } else if ua.contains("Mobi") || ua.contains("Android") {
        "mobile"
    } else {
        "desktop"
    })
}

/// Whitespace collapsed, bounded by characters, e-mail addresses → `@`, digit runs (spaces, dots and dashes allowed
/// between digits) holding six or more digits → `#`.
pub fn clean_text(raw: &str, max_chars: usize) -> String {
    let words: Vec<&str> = raw.split_whitespace().map(|w| if w.contains('@') && w.contains('.') { "@" } else { w }).collect();
    mask_digits(&words.join(" ")).chars().filter(|c| !c.is_control()).take(max_chars).collect()
}
fn mask_digits(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let mut j = i;
            let mut end = i;
            let mut digits = 0;
            while j < chars.len() && (chars[j].is_ascii_digit() || matches!(chars[j], ' ' | '-' | '.')) {
                if chars[j].is_ascii_digit() {
                    digits += 1;
                    end = j;
                }
                j += 1;
            }
            if digits >= 6 {
                out.push('#');
            } else {
                out.extend(&chars[i..=end]);
            }
            i = end + 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Path only (no query or fragment), at most 256 bytes; UUIDs, numbers and long tokens become `:id`.
pub fn clean_route(raw: &str) -> Option<String> {
    let path = raw.split(['?', '#']).next()?.trim();
    if !path.starts_with('/') || path.starts_with("//") || path.chars().any(char::is_control) {
        return None;
    }
    let mut out = String::new();
    for segment in path.split('/').skip(1) {
        out.push('/');
        let digits = segment.bytes().filter(u8::is_ascii_digit).count();
        let id_like = Uuid::parse_str(segment).is_ok()
            || (!segment.is_empty() && digits == segment.len())
            || (segment.len() >= 16 && digits >= 2 && segment.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
            || segment.contains('@');
        if id_like {
            out.push_str(":id");
        } else {
            out.push_str(segment);
        }
        if out.len() > 256 {
            return None;
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Stable element descriptor: component hash `c`, tag `t`, accessible name `n`, ordinal `i`, `data-track` name `x`,
/// field type `y`, field name `f`, link route `h`, role `o`. Unknown keys and values are dropped.
pub fn clean_element(raw: &Map<String, Value>) -> Option<Value> {
    let mut out = Map::new();
    for (key, value) in raw {
        let text = |max: usize| value.as_str().map(|s| clean_text(s, max)).filter(|s| !s.is_empty());
        let token = |max: usize| {
            value
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':')))
                .map(|s| s.to_ascii_lowercase())
        };
        let cleaned = match key.as_str() {
            "c" => token(12).map(Value::from),
            "t" | "y" | "o" => token(24).map(Value::from),
            "n" => text(64).map(Value::from),
            "x" | "f" => token(64).map(Value::from),
            "h" => value.as_str().and_then(clean_route).map(Value::from),
            "i" => value.as_u64().filter(|n| *n <= 9999).map(Value::from),
            _ => None,
        };
        if let Some(v) = cleaned {
            out.insert(key.clone(), v);
        }
    }
    out.contains_key("t").then_some(Value::Object(out))
}
fn clean_props(raw: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    for (key, value) in raw.iter().take(16) {
        if key.is_empty() || key.len() > 32 || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let v = match value {
            Value::Bool(_) => value.clone(),
            Value::Number(n) if n.as_f64().is_some_and(f64::is_finite) => value.clone(),
            Value::String(s) => Value::from(clean_text(s, 128)),
            _ => continue,
        };
        out.insert(key.clone(), v);
    }
    Value::Object(out)
}
fn clean_utm(raw: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    for key in ["source", "medium", "campaign", "term", "content"] {
        if let Some(s) = raw.get(key).and_then(Value::as_str) {
            let v = clean_text(s, 100);
            if !v.is_empty() {
                out.insert(key.into(), Value::from(v));
            }
        }
    }
    Value::Object(out)
}
fn referrer_host(raw: &str, origins: &[String]) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let origin = url.origin().ascii_serialization();
    if origins.iter().any(|o| o == &origin) {
        return None;
    }
    url.host_str().map(|h| h.chars().take(253).collect())
}

/// Per-visitor token window. Bounded: at most 50 000 tracked visitors; the oldest windows are dropped first.
#[derive(Clone)]
pub struct Limiter {
    per_minute: u32,
    windows: Arc<Mutex<HashMap<Uuid, (Instant, u32)>>>,
}
impl Limiter {
    pub fn new(per_minute: u32) -> Self {
        Self { per_minute, windows: Arc::new(Mutex::new(HashMap::new())) }
    }
    pub fn admit(&self, visitor: Uuid, events: u32) -> bool {
        let now = Instant::now();
        let mut map = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 50_000 {
            map.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(60));
        }
        let entry = map.entry(visitor).or_insert((now, 0));
        if now.duration_since(entry.0) >= Duration::from_secs(60) {
            *entry = (now, 0);
        }
        if entry.1.saturating_add(events) > self.per_minute {
            return false;
        }
        entry.1 += events;
        true
    }
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

struct Row_ {
    at: f64,
    page: i32,
    since: i32,
    kind: String,
    route: String,
    element: Option<Value>,
    value: Option<i32>,
    props: Value,
}

/// Validates, privacy-filters and stores one browser batch in one transaction.
pub async fn ingest(pool: &PgPool, limiter: &Limiter, batch: Batch, signals: &Signals) -> Result<Outcome, IngestError> {
    if signals.global_privacy_control || signals.do_not_track {
        return Ok(Outcome::Ignored("privacy-signal"));
    }
    if is_bot(signals.user_agent.as_deref()) {
        return Ok(Outcome::Ignored("automated"));
    }
    if batch.v != 1 {
        return Err(IngestError::Invalid("Send collector protocol version 1"));
    }
    if batch.ev.is_empty() || batch.ev.len() > MAX_EVENTS {
        return Err(IngestError::Invalid("Send between 1 and 64 events per batch"));
    }
    if batch.vid.is_nil() || batch.sid.is_nil() {
        return Err(IngestError::Invalid("Visitor and session identifiers must be random UUIDs"));
    }
    if !limiter.admit(batch.vid, batch.ev.len() as u32) {
        return Err(IngestError::RateLimited);
    }
    let now = now_ms();
    let mut rows = Vec::with_capacity(batch.ev.len());
    for e in &batch.ev {
        if !KINDS.contains(&e.k.as_str()) {
            continue;
        }
        let Some(route) = clean_route(&e.r) else { continue };
        let at = if e.at.is_finite() && e.at > now - 86_400_000.0 && e.at < now + 300_000.0 { e.at } else { now };
        let mut props = e.p.as_ref().map(clean_props).unwrap_or_else(|| json!({}));
        if e.k == "track" && !props.get("name").and_then(Value::as_str).is_some_and(|n| !n.is_empty()) {
            continue;
        }
        if e.k == "page" && let Some(obj) = props.as_object_mut() {
            obj.retain(|k, _| matches!(k.as_str(), "nav" | "title"));
        }
        rows.push(Row_ {
            at,
            page: e.pv.min(100_000) as i32,
            since: if e.t.is_finite() && e.t >= 0.0 { e.t.min(i32::MAX as f64) as i32 } else { 0 },
            kind: e.k.clone(),
            route,
            element: e.el.as_ref().and_then(clean_element),
            value: e.v.filter(|v| v.is_finite()).map(|v| v.clamp(i32::MIN as f64, i32::MAX as f64) as i32),
            props,
        });
    }
    if rows.is_empty() {
        return Ok(Outcome::Ignored("no-valid-events"));
    }
    let ctx = batch.ctx.unwrap_or_default();
    let utm = ctx.utm.as_ref().map(clean_utm).unwrap_or_else(|| json!({}));
    let referrer = ctx.r#ref.as_deref().and_then(|r| referrer_host(r, &signals.origins));
    let language = ctx.lang.as_deref().map(|l| l.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(16).collect::<String>()).filter(|l| !l.is_empty());
    let device = ctx.w.map(|w| if w < 768 { "mobile" } else if w < 1100 { "tablet" } else { "desktop" }).or_else(|| device(signals.user_agent.as_deref()));
    let country = signals.country.as_deref().filter(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase()) && *c != "XX");
    let entry = rows.iter().find(|r| r.kind == "page").map(|r| r.route.clone());
    let mut tx = pool.begin().await.map_err(|e| IngestError::Database(crate::db::safe(e)))?;
    sqlx::query("INSERT INTO public._brain_analytics_visitors(id,user_id) VALUES($1,$2) ON CONFLICT(id) DO UPDATE SET last_seen_at=now(),user_id=COALESCE(EXCLUDED.user_id,public._brain_analytics_visitors.user_id)")
        .bind(batch.vid)
        .bind(signals.user)
        .execute(&mut *tx)
        .await
        .map_err(|e| IngestError::Database(crate::db::safe(e)))?;
    let owned = sqlx::query("INSERT INTO public._brain_analytics_sessions AS s(id,visitor,user_id,entry_route,referrer_host,utm,country,device,language) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(id) DO UPDATE SET last_seen_at=now(),user_id=COALESCE(EXCLUDED.user_id,s.user_id),entry_route=COALESCE(s.entry_route,EXCLUDED.entry_route) WHERE s.visitor=EXCLUDED.visitor RETURNING id")
        .bind(batch.sid)
        .bind(batch.vid)
        .bind(signals.user)
        .bind(entry)
        .bind(referrer)
        .bind(utm)
        .bind(country)
        .bind(device)
        .bind(language)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| IngestError::Database(crate::db::safe(e)))?;
    if owned.is_none() {
        return Err(IngestError::SessionConflict);
    }
    let count = rows.len();
    let mut at = Vec::with_capacity(count);
    let mut page = Vec::with_capacity(count);
    let mut since = Vec::with_capacity(count);
    let mut kind = Vec::with_capacity(count);
    let mut route = Vec::with_capacity(count);
    let mut element = Vec::with_capacity(count);
    let mut value = Vec::with_capacity(count);
    let mut props = Vec::with_capacity(count);
    for r in rows {
        at.push(r.at / 1000.0);
        page.push(r.page);
        since.push(r.since);
        kind.push(r.kind);
        route.push(r.route);
        element.push(r.element);
        value.push(r.value);
        props.push(r.props);
    }
    sqlx::query("INSERT INTO public._brain_analytics_events(at,visitor,session,user_id,page,since_ms,kind,route,element,value,props) SELECT to_timestamp(a),$2,$3,$4,p,s,k,r,e,v,pr FROM unnest($1::float8[],$5::int4[],$6::int4[],$7::text[],$8::text[],$9::jsonb[],$10::int4[],$11::jsonb[]) AS u(a,p,s,k,r,e,v,pr)")
        .bind(at)
        .bind(batch.vid)
        .bind(batch.sid)
        .bind(signals.user)
        .bind(page)
        .bind(since)
        .bind(kind)
        .bind(route)
        .bind(element)
        .bind(value)
        .bind(props)
        .execute(&mut *tx)
        .await
        .map_err(|e| IngestError::Database(crate::db::safe(e)))?;
    tx.commit().await.map_err(|e| IngestError::Database(crate::db::safe(e)))?;
    Ok(Outcome::Stored(count))
}

#[derive(Debug, Serialize, PartialEq, Eq, Default)]
pub struct Purged {
    pub events: u64,
    pub sessions: u64,
    pub visitors: u64,
}
/// Deletes data older than `days` in bounded batches (10 000 rows per statement).
pub async fn purge(pool: &PgPool, days: u32) -> Result<Purged, String> {
    if !(1..=3650).contains(&days) {
        return Err("Retention must be between 1 and 3650 days".into());
    }
    let mut out = Purged::default();
    loop {
        let n = sqlx::query("DELETE FROM public._brain_analytics_events WHERE id IN (SELECT id FROM public._brain_analytics_events WHERE at < now() - make_interval(days => $1) LIMIT 10000)")
            .bind(days as i32)
            .execute(pool)
            .await
            .map_err(crate::db::safe)?
            .rows_affected();
        out.events += n;
        if n < 10000 {
            break;
        }
    }
    out.sessions = sqlx::query("DELETE FROM public._brain_analytics_sessions WHERE last_seen_at < now() - make_interval(days => $1)")
        .bind(days as i32)
        .execute(pool)
        .await
        .map_err(crate::db::safe)?
        .rows_affected();
    out.visitors = sqlx::query("DELETE FROM public._brain_analytics_visitors v WHERE v.last_seen_at < now() - make_interval(days => $1) AND NOT EXISTS (SELECT 1 FROM public._brain_analytics_sessions s WHERE s.visitor=v.id)")
        .bind(days as i32)
        .execute(pool)
        .await
        .map_err(crate::db::safe)?
        .rows_affected();
    Ok(out)
}
/// Erasure request: every event, session and visitor row linked to the account.
pub async fn erase_user(pool: &PgPool, user: Uuid) -> Result<u64, String> {
    let mut tx = pool.begin().await.map_err(crate::db::safe)?;
    let visitors: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM public._brain_analytics_visitors WHERE user_id=$1 UNION SELECT visitor FROM public._brain_analytics_sessions WHERE user_id=$1")
        .bind(user)
        .fetch_all(&mut *tx)
        .await
        .map_err(crate::db::safe)?;
    let n = sqlx::query("DELETE FROM public._brain_analytics_events WHERE user_id=$1 OR visitor=ANY($2)")
        .bind(user)
        .bind(&visitors)
        .execute(&mut *tx)
        .await
        .map_err(crate::db::safe)?
        .rows_affected();
    sqlx::query("DELETE FROM public._brain_analytics_visitors WHERE id=ANY($1)")
        .bind(&visitors)
        .execute(&mut *tx)
        .await
        .map_err(crate::db::safe)?;
    tx.commit().await.map_err(crate::db::safe)?;
    Ok(n)
}

/// Retention loop for a serving backend: one purge per hour; several instances serialize on an advisory lock.
pub async fn retention_loop(pool: PgPool, days: u32) {
    let mut tick = tokio::time::interval(Duration::from_secs(3600));
    loop {
        tick.tick().await;
        let locked: Result<bool, _> = sqlx::query_scalar("SELECT pg_try_advisory_lock(728196411)").fetch_one(&pool).await;
        if locked.unwrap_or(false) {
            if let Ok(p) = purge(&pool, days).await {
                if p.events + p.sessions + p.visitors > 0 {
                    eprintln!("{}", json!({"schemaVersion":1,"event":"analytics-retention","days":days,"purged":p}));
                }
            }
            let _ = sqlx::query("SELECT pg_advisory_unlock(728196411)").execute(&pool).await;
        }
    }
}

static SETTINGS: std::sync::OnceLock<Option<Config>> = std::sync::OnceLock::new();
static LIMITER: std::sync::OnceLock<Limiter> = std::sync::OnceLock::new();
/// Process-wide analytics settings, set once by the host before serving (`None` disables ingestion: 404).
/// Without a call the defaults apply: analytics is on by default.
pub fn configure(config: Option<Config>) -> Result<(), String> {
    if let Some(c) = &config {
        c.validate()?;
    }
    let wanted = format!("{config:?}");
    let current = SETTINGS.get_or_init(|| config);
    if format!("{current:?}") != wanted {
        return Err("Analytics settings were already configured differently in this process".into());
    }
    Ok(())
}
pub fn settings() -> Option<&'static Config> {
    SETTINGS.get_or_init(|| Some(Config::default())).as_ref()
}
pub fn limiter() -> &'static Limiter {
    LIMITER.get_or_init(|| Limiter::new(settings().map_or(600, |c| c.events_per_minute)))
}

#[path = "analytics_query.rs"]
mod query;
pub use query::*;
