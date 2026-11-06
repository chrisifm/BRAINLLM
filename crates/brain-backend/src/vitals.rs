//! Explicit, anonymous document-performance aggregates, independent from behavior analytics.
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::{collections::BTreeSet, time::Duration};

pub const MAX_BODY: usize = 4096;
pub const ROW_CAP: i64 = 250000;
pub const RATE_LIMIT: u32 = 600;
pub const RATE_SCOPE: &str = "native-vitals";
const SUBJECT: &str = "anonymous-global-budget";
const BUDGET: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct Config {
    pub routes: Vec<String>,
    pub retention_days: u32,
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=90).contains(&self.retention_days) || self.routes.is_empty() || self.routes.len() > 512 || self.routes.iter().any(|route| !valid_route(route)) || self.routes.iter().collect::<BTreeSet<_>>().len() != self.routes.len() {
            return Err("Native vitals requires 1..512 distinct public route patterns and retentionDays 1..90".into());
        }
        Ok(())
    }
    /// Independent opt-in. The host must provide its native backend and normal Node/SSR output.
    pub fn from_value(value: Option<&Value>) -> Result<Option<Self>, String> {
        let Some(value) = value else { return Ok(None) };
        if value == &Value::Bool(false) { return Ok(None) }
        let map = value.as_object().ok_or("vitals requires false or an object with explicit enabled and public routes")?;
        if map.keys().any(|key| !["enabled", "routes", "retentionDays"].contains(&key.as_str())) { return Err("vitals has an unknown field; use enabled, routes and retentionDays".into()) }
        let enabled = map.get("enabled").and_then(Value::as_bool).ok_or("vitals.enabled must be an explicit boolean")?;
        let retention_days = match map.get("retentionDays") {
            None => 30,
            Some(value) => u32::try_from(value.as_u64().ok_or("vitals.retentionDays must be an integer from 1 to 90")?).map_err(|_| "vitals.retentionDays must be an integer from 1 to 90")?,
        };
        if !(1..=90).contains(&retention_days) { return Err("vitals.retentionDays must be an integer from 1 to 90".into()) }
        let routes = match map.get("routes") {
            None if !enabled => Vec::new(),
            Some(Value::Array(items)) if items.len() <= 512 => items.iter().map(|value| value.as_str().filter(|s| valid_route(s)).map(str::to_owned).ok_or("vitals.routes must contain public route patterns of at most 160 ASCII characters")).collect::<Result<Vec<_>, _>>()?,
            _ => return Err("vitals.routes must declare from 1 to 512 public route patterns".into()),
        };
        if enabled && routes.is_empty() || routes.iter().collect::<BTreeSet<_>>().len() != routes.len() { return Err("vitals.routes must declare from 1 to 512 distinct public route patterns".into()) }
        Ok(enabled.then_some(Self { routes, retention_days }))
    }
}
pub fn valid_route(route: &str) -> bool {
    if route == "/" { return true }
    if route.len() > 160 || !route.starts_with('/') { return false }
    route[1..].split('/').all(|part| {
        if let Some(param) = part.strip_prefix(':') {
            param.as_bytes().first().is_some_and(u8::is_ascii_lowercase) && param.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        } else {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        }
    })
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub route: String,
    pub consent: bool,
    pub metrics: Vec<Metric>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metric { pub name: String, pub value: f64 }
#[derive(Debug, PartialEq, Eq)]
pub enum Error { Invalid, RateLimited, Capacity, Unavailable }
impl Error {
    pub fn code(&self) -> &'static str { match self { Self::Invalid => "VITALS_INVALID", Self::RateLimited => "VITALS_RATE_LIMITED", Self::Capacity => "VITALS_CAPACITY", Self::Unavailable => "VITALS_UNAVAILABLE" } }
}
fn limits(name: &str) -> Option<(f64, f64, f64)> {
    Some(match name {
        "LCP" => (2500., 4000., 3600000.), "CLS" => (0.1, 0.25, 1000.),
        "INP" => (200., 500., 3600000.), "FCP" => (1800., 3000., 3600000.), "TTFB" => (800., 1800., 3600000.), _ => return None,
    })
}
impl Batch {
    pub fn validate(&self, config: &Config) -> Result<(), Error> {
        config.validate().map_err(|_| Error::Invalid)?;
        if !self.consent || !config.routes.contains(&self.route) || self.metrics.is_empty() || self.metrics.len() > 5 { return Err(Error::Invalid) }
        let mut names = BTreeSet::new();
        for metric in &self.metrics {
            let Some((_, _, max)) = limits(&metric.name) else { return Err(Error::Invalid) };
            if !metric.value.is_finite() || !(0.0..=max).contains(&metric.value) || !names.insert(&metric.name) { return Err(Error::Invalid) }
        }
        Ok(())
    }
}
/// No retries: a consumed shared quota remains consumed even when the aggregate transaction fails.
pub async fn ingest(pool: &PgPool, config: &Config, batch: Batch) -> Result<(), Error> {
    batch.validate(config)?;
    tokio::time::timeout(BUDGET, async {
        match crate::cluster::hit(pool, RATE_SCOPE, SUBJECT, RATE_LIMIT, Duration::from_secs(60)).await.map_err(|_| Error::Unavailable)? {
            Ok(()) => (), Err(_) => return Err(Error::RateLimited),
        }
        let mut tx = pool.begin().await.map_err(|_| Error::Unavailable)?;
        sqlx::raw_sql("SET LOCAL lock_timeout='250ms'; SET LOCAL statement_timeout='1000ms'; SET LOCAL idle_in_transaction_session_timeout='2s';").execute(&mut *tx).await.map_err(|_| Error::Unavailable)?;
        let used: i64 = sqlx::query_scalar("SELECT rows FROM public._brain_vitals_capacity WHERE singleton=true FOR UPDATE").fetch_one(&mut *tx).await.map_err(|_| Error::Unavailable)?;
        let purged: i64 = sqlx::query_scalar("WITH expired AS (SELECT day,route_pattern,metric FROM public._brain_vitals_daily WHERE day<(CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date-($1::int-1) ORDER BY day,route_pattern,metric LIMIT 5000), removed AS (DELETE FROM public._brain_vitals_daily d USING expired e WHERE d.day=e.day AND d.route_pattern=e.route_pattern AND d.metric=e.metric RETURNING 1) SELECT count(*) FROM removed")
            .bind(config.retention_days as i32).fetch_one(&mut *tx).await.map_err(|_| Error::Unavailable)?;
        let names: Vec<&str> = batch.metrics.iter().map(|m| m.name.as_str()).collect();
        let existing: Vec<String> = sqlx::query_scalar("SELECT metric FROM public._brain_vitals_daily WHERE day=(CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date AND route_pattern=$1 AND metric=ANY($2)")
            .bind(&batch.route).bind(&names).fetch_all(&mut *tx).await.map_err(|_| Error::Unavailable)?;
        let next = used - purged + (names.len() - existing.len()) as i64;
        if !(0..=ROW_CAP).contains(&next) { return Err(Error::Capacity) }
        for metric in batch.metrics {
            let (good, needs, _) = limits(&metric.name).ok_or(Error::Invalid)?;
            let (g, n, p): (i64, i64, i64) = if metric.value <= good { (1,0,0) } else if metric.value <= needs { (0,1,0) } else { (0,0,1) };
            sqlx::query("INSERT INTO public._brain_vitals_daily AS d(day,route_pattern,metric,count,total,min,max,good,needs_improvement,poor) VALUES((CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date,$1,$2,1,$3,$3,$3,$4,$5,$6) ON CONFLICT(day,route_pattern,metric) DO UPDATE SET count=d.count+1,total=d.total+EXCLUDED.total,min=least(d.min,EXCLUDED.min),max=greatest(d.max,EXCLUDED.max),good=d.good+EXCLUDED.good,needs_improvement=d.needs_improvement+EXCLUDED.needs_improvement,poor=d.poor+EXCLUDED.poor")
                .bind(&batch.route).bind(&metric.name).bind(metric.value).bind(g).bind(n).bind(p).execute(&mut *tx).await.map_err(|_| Error::Unavailable)?;
        }
        sqlx::query("UPDATE public._brain_vitals_capacity SET rows=$1 WHERE singleton=true").bind(next).execute(&mut *tx).await.map_err(|_| Error::Unavailable)?;
        tx.commit().await.map_err(|_| Error::Unavailable)
    }).await.unwrap_or(Err(Error::Unavailable))
}
/// Current database only. Reports hide expired or no-longer-declared patterns and never infer percentiles or visitors.
pub async fn report(pool: &PgPool, config: &Config, days: u32, limit: u32, offset: u32) -> Result<Value, Error> {
    config.validate().map_err(|_| Error::Invalid)?;
    if !(1..=90).contains(&days) || !(1..=500).contains(&limit) || offset > 10000 { return Err(Error::Invalid) }
    let days = days.min(config.retention_days);
    tokio::time::timeout(BUDGET, async {
        let mut tx = pool.begin().await.map_err(|_| Error::Unavailable)?;
        sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL lock_timeout='250ms'; SET LOCAL statement_timeout='1500ms'; SET LOCAL idle_in_transaction_session_timeout='2s';").execute(&mut *tx).await.map_err(|_| Error::Unavailable)?;
        let rows = sqlx::query("SELECT day::text AS day,route_pattern,metric,count,total,min,max,good,needs_improvement,poor FROM public._brain_vitals_daily WHERE day BETWEEN (CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date-($1::int-1) AND (CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date AND route_pattern=ANY($2) ORDER BY day DESC,route_pattern,metric LIMIT $3 OFFSET $4")
            .bind(days as i32).bind(&config.routes).bind(i64::from(limit)+1).bind(i64::from(offset)).fetch_all(&mut *tx).await.map_err(|_| Error::Unavailable)?;
        let more = rows.len() > limit as usize;
        let mut groups = Vec::new();
        for row in rows.iter().take(limit as usize) {
            let count: i64 = row.get("count"); let total: f64 = row.get("total");
            let good: i64 = row.get("good"); let needs: i64 = row.get("needs_improvement"); let poor: i64 = row.get("poor");
            groups.push(json!({"day":row.get::<String,_>("day"),"route":row.get::<String,_>("route_pattern"),"metric":row.get::<String,_>("metric"),"count":count,"mean":total/count as f64,"min":row.get::<f64,_>("min"),"max":row.get::<f64,_>("max"),"categories":{"good":{"count":good,"proportion":good as f64/count as f64},"needsImprovement":{"count":needs,"proportion":needs as f64/count as f64},"poor":{"count":poor,"proportion":poor as f64/count as f64}}}));
        }
        tx.commit().await.map_err(|_| Error::Unavailable)?;
        Ok(json!({"schemaVersion":1,"days":days,"limit":limit,"offset":offset,"groups":groups,"nextOffset":if more && offset+limit<=10000 {Some(offset+limit)} else {None},"truncated":more,"percentileAvailable":false,"uniqueVisitorsAvailable":false,"duplicateBeaconsMayCountTwice":true,"measurement":"initial-document-estimates","readOnly":true,"valuesCaptured":false}))
    }).await.unwrap_or(Err(Error::Unavailable))
}
