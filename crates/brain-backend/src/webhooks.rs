//! Signed, replay-protected inbound work and encrypted outbound delivery. Secrets never enter job payloads or logs.
use crate::{
    Resource, auth,
    jobs::{Job, Queue},
};
use ring::{
    aead, hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const OUTBOUND_QUEUE: &str = "brain_webhook_outbound";
pub const INBOX_MARKER: &str = "__brain_webhook_inbox";
const OUTBOX_MARKER: &str = "__brain_webhook_outbox";
const MAX_BODY: usize = 16384;
const MAX_EVENTS: i64 = 10000;

// Deliberately no Debug or Serialize on any configuration/key/URL type.
#[derive(Clone)]
pub struct Secret(Arc<SecretInner>);
struct SecretInner {
    bytes: [u8; 32],
    id: String,
    mac: hmac::Key,
}
impl Secret {
    pub fn from_hex(input: &str) -> Result<Self, String> {
        let bytes=unhex::<32>(input).ok_or("Webhook keys must be private independently generated 32-byte keys encoded as 64 hex characters")?;
        let id = auth::digest(input.to_ascii_lowercase().as_str());
        Ok(Self(Arc::new(SecretInner {
            mac: hmac::Key::new(hmac::HMAC_SHA256, &bytes),
            bytes,
            id,
        })))
    }
}
#[derive(Clone)]
pub struct Inbound {
    queue: String,
    keys: Vec<Secret>,
}
impl Inbound {
    pub fn new(queue: String, keys: Vec<Secret>) -> Result<Self, String> {
        if !crate::contract::identifier(&queue)
            || queue == OUTBOUND_QUEUE
            || keys.is_empty()
            || keys.len() > 2
        {
            return Err("Inbound webhooks need an existing application queue and one or two private signing keys".into());
        }
        Ok(Self { queue, keys })
    }
}
#[derive(Clone)]
pub struct Outbound {
    url: url::Url,
    key: Secret,
    origin: String,
}
impl Outbound {
    pub fn new(raw: String, key: Secret, allowed_origins: Vec<String>) -> Result<Self, String> {
        if raw.len() > 2048
            || raw.chars().any(char::is_control)
            || raw.contains('\\')
            || allowed_origins.is_empty()
            || allowed_origins.len() > 8
        {
            return Err(
                "Outbound webhook URL and explicit origin allowlist are required and bounded"
                    .into(),
            );
        }
        let url = url::Url::parse(&raw).map_err(|_| "Outbound webhook URL must be fixed HTTPS")?;
        let authority = raw
            .split_once("://")
            .map(|(_, s)| s.split('/').next().unwrap_or(""))
            .unwrap_or("");
        let origin = url.origin().ascii_serialization();
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || authority.contains('@')
            || url.fragment().is_some()
            || !allowed_origins.iter().any(|s| s == &origin)
        {
            return Err("Outbound webhooks require HTTPS and an exact operator-approved origin; user information and fragments are forbidden".into());
        }
        if let Some(ip) = url
            .host_str()
            .and_then(|s| s.trim_matches(['[', ']']).parse::<IpAddr>().ok())
        {
            if !public_ip(ip) {
                return Err("Outbound webhooks cannot target non-public IP addresses".into());
            }
        }
        for allowed in &allowed_origins {
            let parsed = url::Url::parse(allowed).map_err(|_| "Invalid webhook allowed origin")?;
            if parsed.scheme() != "https"
                || parsed.origin().ascii_serialization() != *allowed
                || parsed.path() != "/"
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err("Webhook allowed origins must be canonical HTTPS origins without paths or credentials".into());
            }
        }
        Ok(Self { url, key, origin })
    }
}
#[derive(Clone)]
pub struct Config {
    keys: Vec<Secret>,
    inbound: BTreeMap<String, Inbound>,
    outbound: BTreeMap<String, Outbound>,
}
impl Config {
    pub fn new(
        keys: Vec<Secret>,
        inbound: BTreeMap<String, Inbound>,
        outbound: BTreeMap<String, Outbound>,
    ) -> Result<Self, String> {
        if keys.is_empty()
            || keys.len() > 2
            || inbound.len() > 16
            || outbound.len() > 16
            || inbound.len() + outbound.len() == 0
            || inbound
                .keys()
                .chain(outbound.keys())
                .any(|n| !crate::contract::identifier(n) || n.len() > 48)
        {
            return Err("Webhook configuration needs one or two encryption keys and 1..32 named endpoints (at most 16 per direction)".into());
        }
        if keys
            .iter()
            .enumerate()
            .any(|(i, k)| keys[..i].iter().any(|p| p.0.id == k.0.id))
        {
            return Err("Webhook encryption keys must be distinct".into());
        }
        let mut used = std::collections::HashSet::new();
        for hook in inbound.values() {
            for key in &hook.keys {
                if !used.insert(key.0.id.clone()) || keys.iter().any(|k| k.0.id == key.0.id) {
                    return Err("Each inbound endpoint needs distinct signing keys, separate from encryption keys".into());
                }
            }
        }
        for hook in outbound.values() {
            if keys.iter().any(|k| k.0.id == hook.key.0.id) {
                return Err("Webhook signing and encryption keys must be independent".into());
            }
        }
        Ok(Self {
            keys,
            inbound,
            outbound,
        })
    }
    pub fn has_outbound(&self) -> bool {
        !self.outbound.is_empty()
    }
    pub fn metadata(&self) -> Value {
        json!({"configured":true,"inbound":self.inbound.iter().map(|(name,h)|json!({"name":name,"queue":h.queue})).collect::<Vec<_>>(),"outbound":self.outbound.iter().map(|(name,h)|json!({"name":name,"origin":h.origin})).collect::<Vec<_>>(),"automaticResourceSubscription":false,"bodyBytes":MAX_BODY,"attempts":5,"workers":if self.has_outbound(){2}else{0}})
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Missing,
    Signature,
    Timestamp,
    Payload,
    Conflict,
    Capacity,
    Unavailable,
}
impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "WEBHOOK_NOT_FOUND",
            Self::Signature => "WEBHOOK_SIGNATURE",
            Self::Timestamp => "WEBHOOK_TIMESTAMP",
            Self::Payload => "WEBHOOK_INPUT",
            Self::Conflict => "WEBHOOK_EVENT_CONFLICT",
            Self::Capacity => "WEBHOOK_CAPACITY",
            Self::Unavailable => "WEBHOOK_UNAVAILABLE",
        }
    }
    pub fn fix(self) -> &'static str {
        match self {
            Self::Missing => "Use an endpoint declared in the server webhook configuration",
            Self::Signature => {
                "Sign the exact raw request bytes with the endpoint's private key and required v1 headers"
            }
            Self::Timestamp => {
                "Synchronize the sender clock and sign a timestamp within five minutes"
            }
            Self::Payload => {
                "Use the declared queue input schema and a JSON object within 16 KiB and depth 32"
            }
            Self::Conflict => {
                "Reuse an event id only for the same event payload; do not replace accepted work"
            }
            Self::Capacity => {
                "Drain pending work and prune completed jobs after their retention window"
            }
            Self::Unavailable => {
                "Inspect private webhook/database configuration using the request reference; do not retry a delivery blindly"
            }
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
fn db(error: sqlx::Error) -> Error {
    let _ = crate::db::safe(error);
    Error::Unavailable
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Publish {
    pub event_id: Uuid,
    pub payload: Value,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub id: Uuid,
    pub event_id: Uuid,
    pub duplicate: bool,
}
#[derive(Clone)]
pub struct Webhooks {
    pool: PgPool,
    config: Arc<Config>,
    jobs: Arc<BTreeMap<String, Resource>>,
}
impl Webhooks {
    pub fn new(pool: PgPool, config: Config, jobs: &[Resource]) -> Result<Self, String> {
        if jobs.len() > 128 || jobs.iter().any(|j| j.name == OUTBOUND_QUEUE) {
            return Err("The native outbound webhook queue is reserved; application jobs cannot use that name".into());
        }
        for job in jobs {
            job.validate()?;
        }
        let count = jobs.len();
        let jobs: BTreeMap<_, _> = jobs.iter().cloned().map(|j| (j.name.clone(), j)).collect();
        if jobs.len() != count {
            return Err("Webhook jobs must have distinct validated names".into());
        }
        for hook in config.inbound.values() {
            if !jobs.contains_key(&hook.queue) {
                return Err("Each inbound webhook queue must have an existing declared job schema and handler".into());
            }
        }
        Ok(Self {
            pool,
            config: Arc::new(config),
            jobs: Arc::new(jobs),
        })
    }
    pub fn metadata(&self) -> Value {
        self.config.metadata()
    }
    pub async fn inbound(
        &self,
        name: &str,
        event: &str,
        timestamp: &str,
        signature: &str,
        raw: &[u8],
    ) -> Result<Receipt, Error> {
        let hook = self.config.inbound.get(name).ok_or(Error::Missing)?;
        if raw.len() > MAX_BODY {
            return Err(Error::Payload);
        }
        let id = Uuid::parse_str(event).map_err(|_| Error::Signature)?;
        if id.to_string() != event {
            return Err(Error::Signature);
        }
        let time = timestamp.parse::<u64>().map_err(|_| Error::Timestamp)?;
        if time.to_string() != timestamp || clock()?.abs_diff(time) > 300 {
            return Err(Error::Timestamp);
        }
        let signature = signature
            .strip_prefix("v1=")
            .and_then(unhex::<32>)
            .ok_or(Error::Signature)?;
        let mut valid = false;
        for key in &hook.keys {
            valid |= hmac::verify(&key.0.mac, &frame(event, timestamp, raw), &signature).is_ok()
        }
        if !valid {
            return Err(Error::Signature);
        }
        let input: Value = serde_json::from_slice(raw).map_err(|_| Error::Payload)?;
        let input = self
            .jobs
            .get(&hook.queue)
            .ok_or(Error::Unavailable)?
            .input(&input)
            .map_err(|_| Error::Payload)?;
        let body = canonical(&input)?;
        self.enqueue("in", name, id, &hook.queue, INBOX_MARKER, body)
            .await
    }
    pub async fn outbound(&self, name: &str, input: Publish) -> Result<Receipt, Error> {
        if !self.config.outbound.contains_key(name) {
            return Err(Error::Missing);
        }
        let body = canonical(&input.payload)?;
        self.enqueue(
            "out",
            name,
            input.event_id,
            OUTBOUND_QUEUE,
            OUTBOX_MARKER,
            body,
        )
        .await
    }
    async fn enqueue(
        &self,
        direction: &str,
        name: &str,
        event: Uuid,
        queue: &str,
        marker: &str,
        body: Vec<u8>,
    ) -> Result<Receipt, Error> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        // Capacity, replay and nonce reservations use one short database transaction, never network I/O.
        sqlx::query("SELECT pg_advisory_xact_lock(728196412)")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        if let Some(row)=sqlx::query("SELECT id,job_id,queue,key_id,nonce,ciphertext FROM public._brain_webhook_events WHERE direction=$1 AND endpoint=$2 AND event_id=$3").bind(direction).bind(name).bind(event).fetch_optional(&mut *tx).await.map_err(db)? {
            let id:Uuid=row.get("id");let job:Uuid=row.get("job_id");
            if row.get::<String,_>("queue")!=queue{return Err(Error::Conflict)}
            let previous=open(&self.config,&row.get::<String,_>("key_id"),row.get("nonce"),row.get("ciphertext"),&aad(direction,name,event,job,queue))?;
            if previous!=body{return Err(Error::Conflict)}
            return Ok(Receipt{id,event_id:event,duplicate:true})
        }
        let count:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM(SELECT 1 FROM public._brain_webhook_events LIMIT 10000) bounded").fetch_one(&mut *tx).await.map_err(db)?;
        if count >= MAX_EVENTS {
            return Err(Error::Capacity);
        }
        let id = Uuid::new_v4();
        let job = crate::jobs::enqueue_tx(
            &mut tx,
            queue,
            json!({(marker):id}),
            Some(&format!("webhook:{direction}:{name}:{event}")),
            5,
        )
        .await
        .map_err(|_| Error::Capacity)?;
        let key = &self.config.keys[0];
        let (nonce, ciphertext) = seal(key, &body, &aad(direction, name, event, job, queue))?;
        sqlx::query("INSERT INTO public._brain_webhook_events(id,direction,endpoint,event_id,job_id,queue,key_id,nonce,ciphertext) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(id).bind(direction).bind(name).bind(event).bind(job).bind(queue).bind(&key.0.id).bind(nonce.to_vec()).bind(ciphertext).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(Receipt {
            id,
            event_id: event,
            duplicate: false,
        })
    }
    /// Only a claimed job's exact row/queue can decrypt its own inbound payload. References planted in another job fail.
    pub async fn decode_job(&self, job: &Job) -> Result<Option<Value>, Error> {
        let Some(marker) = job.payload.get(INBOX_MARKER) else {
            return Ok(None);
        };
        if job.payload.as_object().is_none_or(|m| m.len() != 1) {
            return Err(Error::Payload);
        }
        let id = marker
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or(Error::Payload)?;
        if !self.live(job, 60).await? {
            return Err(Error::Unavailable);
        }
        let row=sqlx::query("SELECT endpoint,event_id,key_id,nonce,ciphertext FROM public._brain_webhook_events WHERE id=$1 AND direction='in' AND job_id=$2 AND queue=$3").bind(id).bind(job.id).bind(&job.queue).fetch_optional(&self.pool).await.map_err(db)?.ok_or(Error::Missing)?;
        let name: String = row.get("endpoint");
        if self
            .config
            .inbound
            .get(&name)
            .is_none_or(|h| h.queue != job.queue)
        {
            return Err(Error::Unavailable);
        }
        let bytes = open(
            &self.config,
            &row.get::<String, _>("key_id"),
            row.get("nonce"),
            row.get("ciphertext"),
            &aad("in", &name, row.get("event_id"), job.id, &job.queue),
        )?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::Unavailable)?;
        canonical(&value)?;
        self.jobs
            .get(&job.queue)
            .ok_or(Error::Unavailable)?
            .input(&value)
            .map(Some)
            .map_err(|_| Error::Payload)
    }
    pub async fn work(&self, shutdown: tokio::sync::watch::Receiver<bool>) -> Result<(), String> {
        if !self.config.has_outbound() {
            return Ok(());
        }
        let state = self.clone();
        Queue::new(self.pool.clone())
            .work(OUTBOUND_QUEUE, 2, shutdown, move |job| {
                let state = state.clone();
                async move {
                    state.deliver(&job).await.map_err(|e| {
                        private_failure(e);
                        e.code().to_owned()
                    })
                }
            })
            .await
    }
    async fn deliver(&self, job: &Job) -> Result<(), Error> {
        self.deliver_using(job, |target, event, body| async move {
            send(&target, event, body).await
        })
        .await
    }
    async fn live(&self, job: &Job, seconds: i32) -> Result<bool, Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public._brain_jobs WHERE id=$1 AND queue=$2 AND state='running' AND lease_token=$3 AND attempts=$4 AND lease_until>now()+$5::integer*interval '1 second')").bind(job.id).bind(&job.queue).bind(job.lease).bind(job.attempt).bind(seconds).fetch_one(&self.pool).await.map_err(db)
    }
    async fn deliver_using<F, Fut>(&self, job: &Job, sender: F) -> Result<(), Error>
    where
        F: FnOnce(Outbound, Uuid, Vec<u8>) -> Fut,
        Fut: std::future::Future<Output = Result<u16, Error>>,
    {
        if job.queue != OUTBOUND_QUEUE || job.payload.as_object().is_none_or(|m| m.len() != 1) {
            return Err(Error::Payload);
        }
        let id = job
            .payload
            .get(OUTBOX_MARKER)
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or(Error::Payload)?;
        if !self.live(job, 6).await? {
            return Err(Error::Unavailable);
        }
        let row=sqlx::query("SELECT endpoint,event_id,key_id,nonce,ciphertext FROM public._brain_webhook_events WHERE id=$1 AND direction='out' AND job_id=$2").bind(id).bind(job.id).fetch_optional(&self.pool).await.map_err(db)?.ok_or(Error::Missing)?;
        let name: String = row.get("endpoint");
        let event: Uuid = row.get("event_id");
        let target = self
            .config
            .outbound
            .get(&name)
            .ok_or(Error::Unavailable)?
            .clone();
        let body = open(
            &self.config,
            &row.get::<String, _>("key_id"),
            row.get("nonce"),
            row.get("ciphertext"),
            &aad("out", &name, event, job.id, &job.queue),
        )?;
        let started = tokio::time::Instant::now();
        let result =
            tokio::time::timeout(Duration::from_secs(5), sender(target, event, body)).await;
        let (status, code) = match result {
            Ok(Ok(status)) if (200..300).contains(&status) => (Some(status), "accepted"),
            Ok(Ok(status)) if (100..600).contains(&status) => (Some(status), "http-rejected"),
            Ok(Ok(_)) => (None, "transport-failed"),
            Ok(Err(_)) => (None, "transport-failed"),
            Err(_) => (None, "deadline"),
        };
        let recorded=sqlx::query("INSERT INTO public._brain_webhook_attempts(event_id,attempt,status,code,elapsed_ms) SELECT $1,$2,$3,$4,$5 WHERE EXISTS(SELECT 1 FROM public._brain_jobs WHERE id=$6 AND state='running' AND lease_token=$7 AND lease_until>now()) ON CONFLICT(event_id,attempt) DO NOTHING")
            .bind(id).bind(job.attempt).bind(status.map(|s|s as i32)).bind(code).bind(started.elapsed().as_millis().min(5000) as i32).bind(job.id).bind(job.lease).execute(&self.pool).await.map_err(db)?.rows_affected();
        if recorded != 1 {
            return Err(Error::Unavailable);
        }
        if code == "accepted" {
            Ok(())
        } else {
            Err(Error::Unavailable)
        }
    }
    pub async fn status(&self, name: &str, id: Uuid) -> Result<Option<Value>, Error> {
        if !self.config.outbound.contains_key(name) {
            return Err(Error::Missing);
        }
        let row=sqlx::query("SELECT e.event_id,j.state,j.attempts,j.max_attempts FROM public._brain_webhook_events e JOIN public._brain_jobs j ON j.id=e.job_id WHERE e.id=$1 AND e.endpoint=$2 AND e.direction='out'").bind(id).bind(name).fetch_optional(&self.pool).await.map_err(db)?;
        let Some(row) = row else { return Ok(None) };
        let attempts=sqlx::query("SELECT attempt,status,code,elapsed_ms FROM public._brain_webhook_attempts WHERE event_id=$1 ORDER BY attempt LIMIT 5").bind(id).fetch_all(&self.pool).await.map_err(db)?;
        Ok(Some(
            json!({"id":id,"eventId":row.get::<Uuid,_>("event_id"),"state":row.get::<String,_>("state"),"attempts":row.get::<i32,_>("attempts"),"maxAttempts":row.get::<i32,_>("max_attempts"),"deliveries":attempts.iter().map(|r|json!({"attempt":r.get::<i32,_>("attempt"),"status":r.get::<Option<i32>,_>("status"),"code":r.get::<String,_>("code"),"ms":r.get::<i32,_>("elapsed_ms")})).collect::<Vec<_>>()}),
        ))
    }
}
fn clock() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| Error::Unavailable)
}
#[track_caller]
fn private_failure(error: Error) {
    let at = std::panic::Location::caller();
    eprintln!(
        "{}",
        json!({"schemaVersion":1,"event":"webhook-delivery-failed","private":true,"code":error.code(),"fix":error.fix(),"location":{"file":at.file(),"line":at.line(),"column":at.column()},"valuesCaptured":false})
    );
}
fn frame(event: &str, time: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("brain-webhook-v1\n{event}\n{time}\n").into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
fn aad(direction: &str, name: &str, event: Uuid, job: Uuid, queue: &str) -> Vec<u8> {
    format!("brain-webhook-aead-v1\n{direction}\n{name}\n{event}\n{job}\n{queue}").into_bytes()
}
fn canonical(value: &Value) -> Result<Vec<u8>, Error> {
    fn bounded(v: &Value, depth: usize, n: &mut usize) -> bool {
        *n += 1;
        if depth > 32 || *n > 2048 {
            return false;
        }
        match v {
            Value::Array(a) => a.iter().all(|v| bounded(v, depth + 1, n)),
            Value::Object(o) => o
                .iter()
                .all(|(k, v)| k.len() <= 128 && bounded(v, depth + 1, n)),
            _ => true,
        }
    }
    if !value.is_object() || !bounded(value, 0, &mut 0) {
        return Err(Error::Payload);
    }
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Payload)?;
    if bytes.len() > MAX_BODY {
        return Err(Error::Payload);
    }
    Ok(bytes)
}
fn seal(key: &Secret, body: &[u8], aad: &[u8]) -> Result<([u8; 12], Vec<u8>), Error> {
    let mut nonce = [0u8; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| Error::Unavailable)?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &key.0.bytes).map_err(|_| Error::Unavailable)?,
    );
    let mut cipher = body.to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(aad),
        &mut cipher,
    )
    .map_err(|_| Error::Unavailable)?;
    Ok((nonce, cipher))
}
fn open(
    config: &Config,
    key_id: &str,
    nonce: Vec<u8>,
    mut cipher: Vec<u8>,
    aad: &[u8],
) -> Result<Vec<u8>, Error> {
    if cipher.len() > MAX_BODY + 16 || cipher.len() < 16 {
        return Err(Error::Unavailable);
    }
    let nonce: [u8; 12] = nonce.try_into().map_err(|_| Error::Unavailable)?;
    let key = config
        .keys
        .iter()
        .find(|k| k.0.id == key_id)
        .ok_or(Error::Unavailable)?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &key.0.bytes).map_err(|_| Error::Unavailable)?,
    );
    key.open_in_place(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(aad),
        &mut cipher,
    )
    .map(|plain| plain.to_vec())
    .map_err(|_| Error::Unavailable)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex<const N: usize>(input: &str) -> Option<[u8; N]> {
    if input.len() != N * 2 || !input.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&input[i * 2..i * 2 + 2], 16).ok()?
    }
    Some(out)
}
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let a = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && a[0] != 0
                && a[0] < 224
                && !(a[0] == 100 && (64..128).contains(&a[1]))
                && !(a[0] == 192 && (a[1] == 0 || a[1] == 88 && a[2] == 99))
                && !(a == [168, 63, 129, 16])
                && !(a[0] == 198 && (a[1] == 18 || a[1] == 19 || a[1] == 51 && a[2] == 100))
                && !(a[0] == 203 && a[1] == 0 && a[2] == 113)
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] & 0xfe00 == 0 || s[1] == 0x0db8))
                && !(s[0] == 0x3fff && s[1] & 0xf000 == 0)
        }
    }
}
async fn send(target: &Outbound, event: Uuid, body: Vec<u8>) -> Result<u16, Error> {
    let host = target
        .url
        .host_str()
        .ok_or(Error::Unavailable)?
        .trim_matches(['[', ']']);
    let port = target
        .url
        .port_or_known_default()
        .ok_or(Error::Unavailable)?;
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| Error::Unavailable)?;
    let mut pinned: Vec<SocketAddr> = Vec::new();
    for address in addresses {
        if pinned.len() >= 16 || !public_ip(address.ip()) {
            return Err(Error::Unavailable);
        }
        if !pinned.contains(&address) {
            pinned.push(address)
        }
    }
    if pinned.is_empty() {
        return Err(Error::Unavailable);
    }
    // Pin the validated addresses into this client's actual connection resolver. Never resolve again unrestricted.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .resolve_to_addrs(host, &pinned)
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|_| Error::Unavailable)?;
    let time = clock()?.to_string();
    let signature = format!(
        "v1={}",
        hex(hmac::sign(&target.key.0.mac, &frame(&event.to_string(), &time, &body)).as_ref())
    );
    let mut response = client
        .post(target.url.clone())
        .header("Content-Type", "application/json")
        .header("X-Brain-Webhook-ID", event.to_string())
        .header("X-Brain-Webhook-Timestamp", time)
        .header("X-Brain-Webhook-Signature", signature)
        .header("Idempotency-Key", event.to_string())
        .body(body)
        .send()
        .await
        .map_err(|_| Error::Unavailable)?;
    let status = response.status().as_u16();
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BODY as u64)
    {
        return Err(Error::Unavailable);
    }
    let mut bytes = 0;
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unavailable)? {
        bytes += chunk.len();
        if bytes > MAX_BODY {
            return Err(Error::Unavailable);
        }
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(byte: u8) -> Secret {
        Secret::from_hex(&format!("{byte:02x}").repeat(32)).unwrap()
    }
    fn config() -> Config {
        Config::new(
            vec![key(1)],
            BTreeMap::from([(
                "incoming".into(),
                Inbound::new("incoming_work".into(), vec![key(2)]).unwrap(),
            )]),
            BTreeMap::from([(
                "partner".into(),
                Outbound::new(
                    "https://example.test/private?credential=synthetic".into(),
                    key(3),
                    vec!["https://example.test".into()],
                )
                .unwrap(),
            )]),
        )
        .unwrap()
    }
    fn sign(event: &str, time: &str, body: &[u8]) -> String {
        format!(
            "v1={}",
            hex(hmac::sign(&key(2).0.mac, &frame(event, time, body)).as_ref())
        )
    }
    #[test]
    fn authenticated_encryption_binds_job_endpoint_and_payload_and_allows_key_rotation() {
        let cfg = config();
        let plain = br#"{"message":"synthetic-private-payload"}"#;
        let (nonce, cipher) = seal(&cfg.keys[0], plain, b"job-specific-aad").unwrap();
        assert!(!cipher.windows(plain.len()).any(|w| w == plain));
        assert_eq!(
            open(
                &cfg,
                &cfg.keys[0].0.id,
                nonce.to_vec(),
                cipher.clone(),
                b"job-specific-aad"
            )
            .unwrap(),
            plain
        );
        assert_eq!(
            open(
                &cfg,
                &cfg.keys[0].0.id,
                nonce.to_vec(),
                cipher.clone(),
                b"other-job"
            ),
            Err(Error::Unavailable)
        );
        let mut tampered = cipher.clone();
        tampered[0] ^= 1;
        assert_eq!(
            open(
                &cfg,
                &cfg.keys[0].0.id,
                nonce.to_vec(),
                tampered,
                b"job-specific-aad"
            ),
            Err(Error::Unavailable)
        );
        let rotated = Config::new(
            vec![key(4), key(1)],
            cfg.inbound.clone(),
            cfg.outbound.clone(),
        )
        .unwrap();
        assert_eq!(
            open(
                &rotated,
                &key(1).0.id,
                nonce.to_vec(),
                cipher.clone(),
                b"job-specific-aad"
            )
            .unwrap(),
            plain
        );
        assert!(
            open(
                &rotated,
                &key(9).0.id,
                nonce.to_vec(),
                cipher,
                b"job-specific-aad"
            )
            .is_err()
        );
    }
    #[test]
    fn signing_uses_standard_hmac_and_configuration_never_exposes_private_urls_or_keys() {
        // RFC 4231 HMAC-SHA256 test case 1.
        let mac = hmac::Key::new(hmac::HMAC_SHA256, &[0x0b; 20]);
        assert_eq!(
            hex(hmac::sign(&mac, b"Hi There").as_ref()),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        let report = config().metadata().to_string();
        assert!(!report.contains("credential"));
        assert!(!report.contains("private?"));
        assert!(!report.contains(&"01".repeat(32)));
        let cfg = config();
        assert!(Config::new(vec![key(2)], cfg.inbound.clone(), cfg.outbound.clone()).is_err());
        let mut inbound = cfg.inbound.clone();
        inbound.insert(
            "another".into(),
            Inbound::new("incoming_work".into(), vec![key(2)]).unwrap(),
        );
        assert!(Config::new(vec![key(1)], inbound, cfg.outbound).is_err());
    }
    #[test]
    fn ssrf_and_payload_limits_reject_private_and_ambiguous_destinations() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.0.1",
            "168.63.129.16",
            "198.18.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "2002::1",
            "3fff::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
        for url in [
            "http://example.test/x",
            "https://127.0.0.1/x",
            "https://user:pass@example.test/x",
            "https://example.test/x#fragment",
            "https://evil.test/x",
        ] {
            assert!(
                Outbound::new(url.into(), key(3), vec!["https://example.test".into()]).is_err(),
                "{url}"
            );
        }
        assert!(canonical(&json!([1, 2])).is_err());
        assert!(canonical(&json!({"x":"x".repeat(MAX_BODY)})).is_err());
        let mut value = json!({});
        for _ in 0..34 {
            value = json!({"nested":value});
        }
        assert!(canonical(&value).is_err());
    }
    #[tokio::test]
    #[ignore = "Use the explicitly owned disposable Postgres harness; no external delivery"]
    async fn signed_events_are_replay_safe_encrypted_lease_bound_and_finitely_retried() {
        use sqlx::ConnectOptions;
        use std::str::FromStr;
        let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable harness required");
        let base = crate::db::connect(&url).await.unwrap();
        let name = format!("brain_webhooks_{}", Uuid::new_v4().simple());
        // Identifier is a fixed ASCII prefix plus a generated UUID, never application input.
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&base)
            .await
            .unwrap();
        let options = sqlx::postgres::PgConnectOptions::from_str(&url)
            .unwrap()
            .database(&name);
        let pool = crate::db::connect(options.to_url_lossy().as_str())
            .await
            .unwrap();
        base.close().await;
        crate::db::migrate(&pool, &[]).await.unwrap();
        crate::db::verify(&pool, &[]).await.unwrap();
        let resource:Resource=serde_json::from_value(json!({"name":"incoming_work","fields":{"message":{"kind":"str","required":true}},"policy":{"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}})).unwrap();
        let hooks = Webhooks::new(pool.clone(), config(), &[resource]).unwrap();
        let queue = Queue::new(pool.clone());
        let event = Uuid::new_v4().to_string();
        let stamp = clock().unwrap().to_string();
        let body = br#"{"message":"synthetic-private-payload"}"#;
        let signature = sign(&event, &stamp, body);
        assert!(matches!(
            hooks
                .inbound("incoming", &event, &stamp, "v1=00", body)
                .await,
            Err(Error::Signature)
        ));
        assert!(matches!(
            hooks
                .inbound("incoming", &event, "1", &signature, body)
                .await,
            Err(Error::Timestamp)
        ));
        let first = hooks
            .inbound("incoming", &event, &stamp, &signature, body)
            .await
            .unwrap();
        assert!(!first.duplicate);
        let duplicate = hooks
            .inbound("incoming", &event, &stamp, &signature, body)
            .await
            .unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(first.id, duplicate.id);
        let changed = br#"{"message":"changed"}"#;
        assert!(matches!(
            hooks
                .inbound(
                    "incoming",
                    &event,
                    &stamp,
                    &sign(&event, &stamp, changed),
                    changed
                )
                .await,
            Err(Error::Conflict)
        ));
        let concurrent_event = Uuid::new_v4().to_string();
        let concurrent_signature = sign(&concurrent_event, &stamp, body);
        let (a, b) = tokio::join!(
            hooks.inbound(
                "incoming",
                &concurrent_event,
                &stamp,
                &concurrent_signature,
                body
            ),
            hooks.inbound(
                "incoming",
                &concurrent_event,
                &stamp,
                &concurrent_signature,
                body
            )
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.id, b.id);
        assert_ne!(a.duplicate, b.duplicate);
        let job = queue.claim("incoming_work", 90).await.unwrap().unwrap();
        assert_eq!(job.payload.as_object().unwrap().len(), 1);
        assert!(!job.payload.to_string().contains("synthetic-private"));
        assert_eq!(
            hooks.decode_job(&job).await.unwrap(),
            Some(serde_json::from_slice::<Value>(body).unwrap())
        );
        let mut forged = job.clone();
        forged.lease = Uuid::new_v4();
        assert!(hooks.decode_job(&forged).await.is_err());
        assert!(queue.ack(&job).await.unwrap());
        assert!(hooks.decode_job(&job).await.is_err());
        let cipher: Vec<u8> =
            sqlx::query_scalar("SELECT ciphertext FROM public._brain_webhook_events WHERE id=$1")
                .bind(first.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!cipher.windows(7).any(|w| w == b"message"));
        let outgoing = hooks
            .outbound(
                "partner",
                Publish {
                    event_id: Uuid::new_v4(),
                    payload: json!({"message":"private-outbound"}),
                },
            )
            .await
            .unwrap();
        for attempt in 1..=5 {
            let job = queue.claim(OUTBOUND_QUEUE, 30).await.unwrap().unwrap();
            assert_eq!(job.attempt, attempt);
            assert!(
                hooks
                    .deliver_using(&job, |_, _, body| async move {
                        assert_eq!(
                            serde_json::from_slice::<Value>(&body).unwrap(),
                            json!({"message":"private-outbound"})
                        );
                        Ok(503)
                    })
                    .await
                    .is_err()
            );
            assert!(queue.fail(&job).await.unwrap());
            // Advance only this synthetic job's retry schedule; production backoff remains intact.
            sqlx::query("UPDATE public._brain_jobs SET available_at=now() WHERE id=$1")
                .bind(job.id)
                .execute(&pool)
                .await
                .unwrap();
        }
        assert!(queue.claim(OUTBOUND_QUEUE, 30).await.unwrap().is_none());
        let status = hooks.status("partner", outgoing.id).await.unwrap().unwrap();
        assert_eq!(status["state"], "dead");
        assert_eq!(status["deliveries"].as_array().unwrap().len(), 5);
        assert!(!status.to_string().contains("private-outbound"));
        // A reference copied into a different job cannot decrypt another event.
        let planted_id = queue
            .enqueue("incoming_work", json!({(INBOX_MARKER):first.id}), None, 1)
            .await
            .unwrap();
        // The second legitimate concurrent event remains ahead of this planted job.
        let legitimate = queue.claim("incoming_work", 90).await.unwrap().unwrap();
        assert!(hooks.decode_job(&legitimate).await.unwrap().is_some());
        queue.ack(&legitimate).await.unwrap();
        let planted = queue.claim("incoming_work", 90).await.unwrap().unwrap();
        assert_eq!(planted.id, planted_id);
        assert!(hooks.decode_job(&planted).await.is_err());
        queue.fail(&planted).await.unwrap();
        let accepted = hooks
            .outbound(
                "partner",
                Publish {
                    event_id: Uuid::new_v4(),
                    payload: json!({"ok":true}),
                },
            )
            .await
            .unwrap();
        let job = queue.claim(OUTBOUND_QUEUE, 30).await.unwrap().unwrap();
        let mut forged = job.clone();
        forged.lease = Uuid::new_v4();
        assert!(
            hooks
                .deliver_using(&forged, |_, _, _| async {
                    panic!("Forged lease must never call sender");
                    #[allow(unreachable_code)]
                    Ok(200)
                })
                .await
                .is_err()
        );
        hooks
            .deliver_using(&job, |_, _, _| async { Ok(204) })
            .await
            .unwrap();
        assert!(queue.ack(&job).await.unwrap());
        assert_eq!(
            hooks.status("partner", accepted.id).await.unwrap().unwrap()["state"],
            "succeeded"
        );
        pool.close().await;
    }
}
