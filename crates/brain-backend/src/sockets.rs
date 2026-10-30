//! Ephemeral, single-process invalidation metadata. Authorization is never delegated to room names.
use super::{App, Error, Result};
use axum::{
    extract::{
        OriginalUri, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, broadcast, watch};
use uuid::Uuid;

const CONNECTIONS: usize = 128;
const PER_USER: usize = 4;
const ROOMS: usize = 32;
const EVENTS: usize = 128;
const FRAME: usize = 4096;
const CHECKS: usize = 8;
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const LIFETIME: Duration = Duration::from_secs(15 * 60);
/// Explicit opt-in. Origins must also appear in the backend's HTTP origin allowlist.
#[derive(Clone)]
pub struct SocketConfig {
    origins: Vec<String>,
}
impl SocketConfig {
    pub fn new(origins: Vec<String>) -> std::result::Result<Self, String> {
        if origins.is_empty() || origins.len() > 8 {
            return Err("Socket configuration requires 1..8 explicit frontend origins".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for origin in &origins {
            if origin.len() > 2048 || !seen.insert(origin) {
                return Err("Socket origins must be distinct and at most 2048 bytes".into());
            }
            let parsed = url::Url::parse(origin).map_err(|_| "Invalid socket origin")?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || parsed.path() != "/"
                || parsed.origin().ascii_serialization() != *origin
                || origin.contains('*')
            {
                return Err("Socket origins must be canonical http(s) origins without credentials, paths, query, fragment or wildcards".into());
            }
        }
        Ok(Self { origins })
    }
    pub(super) fn allowed(&self) -> &[String] {
        &self.origins
    }
}
#[derive(Clone)]
struct Event {
    resource: String,
    id: Uuid,
    owner: Uuid,
    operation: &'static str,
    event_id: Uuid,
}
pub(super) struct Broker {
    events: broadcast::Sender<Event>,
    users: Mutex<BTreeMap<Uuid, usize>>,
    stop: watch::Sender<bool>,
    active: watch::Sender<usize>,
    checks: Semaphore,
    origins: Vec<String>,
}
impl Broker {
    pub(super) fn new(origins: Vec<String>) -> Self {
        let (events, _) = broadcast::channel(EVENTS);
        Self {
            events,
            users: Mutex::new(BTreeMap::new()),
            stop: watch::channel(false).0,
            active: watch::channel(0).0,
            checks: Semaphore::new(CHECKS),
            origins,
        }
    }
    /// Call only AFTER the repository transaction commits. Row values are never retained or emitted.
    pub(super) fn changed(&self, resource: &str, operation: &str, record: &Value) {
        if self.events.receiver_count() == 0 {
            return;
        }
        let operation = match operation {
            "create" => "create",
            "update" => "update",
            "delete" => "delete",
            _ => return,
        };
        if !name(resource) {
            return;
        }
        let Some(id) = record["id"].as_str().and_then(|s| s.parse().ok()) else {
            return;
        };
        let Some(owner) = record["owner"].as_str().and_then(|s| s.parse().ok()) else {
            return;
        };
        let _ = self.events.send(Event {
            resource: resource.into(),
            id,
            owner,
            operation,
            event_id: Uuid::new_v4(),
        });
    }
    fn acquire(self: &Arc<Self>, user: Uuid) -> Option<Permit> {
        let mut users = self.users.lock().ok()?;
        if *self.stop.borrow()
            || users.values().sum::<usize>() >= CONNECTIONS
            || users.get(&user).copied().unwrap_or(0) >= PER_USER
        {
            return None;
        }
        *users.entry(user).or_default() += 1;
        self.active.send_replace(users.values().sum());
        Some(Permit {
            broker: self.clone(),
            user,
        })
    }
}
struct Permit {
    broker: Arc<Broker>,
    user: Uuid,
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut users) = self.broker.users.lock() {
            if let Some(count) = users.get_mut(&self.user) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    users.remove(&self.user);
                }
            }
            self.broker.active.send_replace(users.values().sum());
        }
    }
}

/// Process-local cancellation for the backend's owned upgraded connections.
#[derive(Clone)]
pub struct SocketControl(Arc<Broker>);
impl SocketControl {
    pub(super) fn new(broker: Arc<Broker>) -> Self {
        Self(broker)
    }
    pub fn shutdown(&self) {
        // Serialize with admission so closed() cannot miss an in-flight new permit.
        let _admission = self
            .0
            .users
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.0.stop.send_replace(true);
    }
    /// Wait for only this broker's permits; never hang the embedding host indefinitely.
    pub async fn closed(&self) -> std::result::Result<(), &'static str> {
        if !*self.0.stop.borrow() {
            return Err("Call socket shutdown before waiting for closure");
        }
        let mut active = self.0.active.subscribe();
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if *active.borrow_and_update() == 0 {
                    return Ok(());
                }
                if active.changed().await.is_err() {
                    return Err("Native socket lifecycle observer unavailable");
                }
            }
        })
        .await
        .map_err(|_| "Native sockets did not close within eight seconds")?
    }
}
#[derive(Clone, PartialEq, Eq)]
enum Room {
    Own,
    Resource { name: String, id: Option<Uuid> },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    #[serde(rename = "type")]
    kind: String,
    resource: Option<String>,
    id: Option<Uuid>,
    room: Option<String>,
}
fn name(s: &str) -> bool {
    crate::contract::identifier(s)
}

impl Command {
    fn target(self) -> Option<Room> {
        match (self.room, self.resource, self.id) {
            (Some(room), None, None) if room == "user:self" => Some(Room::Own),
            (None, Some(resource), id) if name(&resource) => {
                Some(Room::Resource { name: resource, id })
            }
            _ => None,
        }
    }
}
fn forbidden() -> Error {
    Error::new(
        StatusCode::FORBIDDEN,
        "SOCKET_ORIGIN_REQUIRED",
        "Connect from an explicitly allowed frontend origin; never put credentials in the URL",
    )
}
pub(super) async fn upgrade(
    State(app): State<App>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    if uri.query().is_some() || headers.get_all(header::ORIGIN).iter().count() != 1 {
        return Err(forbidden());
    }
    if headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .count()
        != 1
        || headers
            .get(header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|v| v.to_str().ok())
            != Some("brain.v1")
    {
        return Err(forbidden());
    }
    let mut credentials = HeaderMap::new();
    let mut credential_bytes = 0;
    for key in [header::COOKIE, header::AUTHORIZATION] {
        if headers.get_all(&key).iter().count() > 1 {
            return Err(forbidden());
        }
        if let Some(value) = headers.get(&key) {
            credential_bytes += value.len();
            if credential_bytes > 16384 {
                return Err(forbidden());
            }
            credentials.insert(key, value.clone());
        }
    }
    // Browser upgrades are GETs: the normal mutation-origin middleware is insufficient here.
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(forbidden)?;
    if !app.sockets.origins.iter().any(|allowed| allowed == origin) {
        return Err(forbidden());
    }
    let admission = app.sockets.checks.try_acquire().map_err(|_| {
        Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "SOCKET_CHECK_CAPACITY",
            "Retry after active authentication checks finish",
        )
    })?;
    let user = tokio::time::timeout(CHECK_TIMEOUT, app.required(&credentials))
        .await
        .map_err(|_| {
            Error::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "SOCKET_AUTH_UNAVAILABLE",
                "Retry authentication before connecting",
            )
        })??;
    drop(admission);
    let permit = app.sockets.acquire(user.id).ok_or_else(|| {
        Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "SOCKET_CAPACITY",
            "Close unused connections before retrying",
        )
    })?;
    let rid = super::request_reference().unwrap_or_else(|| Uuid::new_v4().to_string());
    let failed_rid = rid.clone();
    Ok(ws
        .protocols(["brain.v1"])
        .read_buffer_size(FRAME)
        .write_buffer_size(0)
        .max_write_buffer_size(FRAME * 2)
        .max_frame_size(FRAME)
        .max_message_size(FRAME)
        .on_failed_upgrade(move |_| {
            eprintln!("{}", json!({"schemaVersion":1,"event":"socket-upgrade-failed","code":"SOCKET_UPGRADE_FAILED","rid":failed_rid,"location":{"file":file!(),"line":line!()},"fix":"Check the native session and allowed origin, reconnect with bounded backoff and resynchronize HTTP state","private":true,"valuesCaptured":false}));
        })
        .on_upgrade(move |socket| {
            super::REQUEST_ID.scope(rid, async move {
                let _ =
                    tokio::time::timeout(LIFETIME, run(app, credentials, user.id, socket, permit))
                        .await;
            })
        })
        .into_response())
}
#[derive(Debug)]
enum CheckFailure {
    Authentication,
    Unavailable,
    Stopping,
}
async fn check_failure(socket: &mut WebSocket, failure: CheckFailure) {
    match failure {
        CheckFailure::Authentication => close(socket, 1008, "Authentication expired").await,
        CheckFailure::Stopping => close(socket, 1001, "Backend stopping").await,
        CheckFailure::Unavailable => {
            close(socket, 1013, "Resynchronize after unavailable checks").await
        }
    }
}
async fn checked<T>(
    app: &App,
    work: impl std::future::Future<Output = std::result::Result<T, CheckFailure>>,
) -> std::result::Result<T, CheckFailure> {
    let mut stop = app.sockets.stop.subscribe();
    if *stop.borrow() {
        return Err(CheckFailure::Stopping);
    }
    // Each admitted connection runs one check at a time: at most 128 waiters, never new tasks per event.
    // One deadline includes semaphore wait, authentication and the authoritative row projection.
    tokio::select! {
        biased;
        _ = stop.changed() => Err(CheckFailure::Stopping),
        result = tokio::time::timeout(CHECK_TIMEOUT, async {
            let _permit = app.sockets.checks.acquire().await.map_err(|_| CheckFailure::Unavailable)?;
            work.await
        }) => result.map_err(|_| CheckFailure::Unavailable)?,
    }
}
async fn current_user(
    app: &App,
    headers: &HeaderMap,
    id: Uuid,
) -> std::result::Result<crate::auth::User, CheckFailure> {
    let user = app.required(headers).await.map_err(|error| {
        if error.status == StatusCode::UNAUTHORIZED {
            CheckFailure::Authentication
        } else {
            CheckFailure::Unavailable
        }
    })?;
    if user.id != id {
        return Err(CheckFailure::Authentication);
    }
    Ok(user)
}
async fn user(
    app: &App,
    headers: &HeaderMap,
    id: Uuid,
) -> std::result::Result<crate::auth::User, CheckFailure> {
    checked(app, current_user(app, headers, id)).await
}
async fn authorize(
    app: &App,
    headers: &HeaderMap,
    identity: Uuid,
    room: &Room,
    event: Option<&Event>,
) -> std::result::Result<bool, CheckFailure> {
    checked(app, async {
        let user = current_user(app, headers, identity).await?;
        let Room::Resource { name, id } = room else {
            return Ok(true);
        };
        let Ok(resource) = app.resource(name) else {
            return Ok(false);
        };
        if let Some(event) = event.filter(|event| event.operation == "delete") {
            // Only fixed UUID authority metadata captured before hooks and successful COMMIT is retained.
            return Ok(resource
                .policy
                .allows("read", Some(&user.role), event.owner == user.id));
        }
        let id = match (id, event) {
            (Some(id), _) => *id,
            (None, Some(event)) => event.id,
            // Joining a collection authorizes interest only; each delivered id is checked independently.
            (None, None) => return Ok(resource.policy.allows("read", Some(&user.role), true)),
        };
        // Current policies depend only on ownership and role: never fetch or retain arbitrary row JSON.
        if !crate::contract::identifier(&resource.name) {
            return Ok(false);
        }
        let sql = sqlx::AssertSqlSafe(format!(
            "SELECT owner FROM _brain_res_{} WHERE id=$1",
            resource.name
        ));
        let owner = sqlx::query_scalar::<_, Uuid>(sql)
            .bind(id)
            .fetch_optional(&app.pool)
            .await
            .map_err(|error| {
                let _ = Error::internal(crate::db::safe(error));
                CheckFailure::Unavailable
            })?;
        let Some(owner) = owner else {
            return Ok(false);
        };
        Ok(resource
            .policy
            .allows("read", Some(&user.role), owner == user.id))
    })
    .await
}
fn payload(event: &Event) -> Value {
    json!({"type":"invalidate","resource":event.resource,"id":event.id,"operation":event.operation,"eventId":event.event_id})
}
struct Window {
    start: Instant,
    inbound: u8,
    outbound: u8,
}
impl Window {
    fn new(now: Instant) -> Self {
        Self {
            start: now,
            inbound: 0,
            outbound: 0,
        }
    }
    fn accept(&mut self, now: Instant, outbound: bool) -> bool {
        if now.saturating_duration_since(self.start) >= Duration::from_secs(1) {
            self.start = now;
            self.inbound = 0;
            self.outbound = 0;
        }
        let (count, limit) = if outbound {
            (&mut self.outbound, 32)
        } else {
            (&mut self.inbound, 16)
        };
        if *count >= limit {
            return false;
        }
        *count += 1;
        true
    }
}
fn join_room(rooms: &mut Vec<Room>, room: Room) -> bool {
    if rooms.contains(&room) {
        return true;
    }
    if rooms.len() >= ROOMS {
        return false;
    }
    rooms.push(room);
    true
}
async fn send(socket: &mut WebSocket, rate: &mut Window, value: Value) -> bool {
    if !rate.accept(Instant::now(), true) {
        close(socket, 1013, "Resynchronize after overload").await;
        return false;
    }
    let Ok(body) = serde_json::to_string(&value) else {
        return false;
    };
    body.len() <= FRAME
        && matches!(
            tokio::time::timeout(
                Duration::from_secs(2),
                socket.send(Message::Text(body.into()))
            )
            .await,
            Ok(Ok(()))
        )
}
async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        socket.send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))),
    )
    .await;
}
async fn run(app: App, headers: HeaderMap, id: Uuid, mut socket: WebSocket, _permit: Permit) {
    let mut events = app.sockets.events.subscribe();
    let mut stop = app.sockets.stop.subscribe();
    if *stop.borrow() {
        close(&mut socket, 1001, "Backend stopping").await;
        return;
    }
    let mut last_pong = Instant::now();
    let mut rooms: Vec<Room> = Vec::new();
    let started = Instant::now();
    let mut rate = Window::new(Instant::now());
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now()
            + Duration::from_secs(25)
            + Duration::from_millis((id.as_u128() % 5000) as u64),
        Duration::from_secs(25),
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    if !send(
        &mut socket,
        &mut rate,
        json!({"type":"ready","protocol":1,"durable":false,"resyncRequired":true}),
    )
    .await
    {
        return;
    }
    loop {
        if started.elapsed() >= LIFETIME {
            close(&mut socket, 1001, "Reconnect and resynchronize").await;
            return;
        }
        tokio::select! {
            _ = stop.changed() => { close(&mut socket, 1001, "Backend stopping").await; return; },
            _ = heartbeat.tick() => {
                if let Err(error) = user(&app,&headers,id).await { check_failure(&mut socket,error).await;return; }
                if last_pong.elapsed() > Duration::from_secs(55) { close(&mut socket,1001,"Heartbeat expired").await;return; }
                if !matches!(tokio::time::timeout(Duration::from_secs(2),socket.send(Message::Ping(Vec::new().into()))).await,Ok(Ok(()))) {return;}
            },
            message = socket.recv() => {
                let Some(Ok(message)) = message else { return; };
                if !rate.accept(Instant::now(), false) {close(&mut socket,1008,"Message limit exceeded").await;return;}
                match message {
                    Message::Text(body) => {
                        if body.len()>FRAME {close(&mut socket,1008,"Message limit exceeded").await;return;}
                        let Ok(command) = serde_json::from_str::<Command>(&body) else {close(&mut socket,1008,"Invalid socket command").await;return;};
                        let kind = command.kind.clone();
                        if kind == "ping" && command.resource.is_none() && command.id.is_none() && command.room.is_none() {
                            if let Err(error) = user(&app,&headers,id).await {check_failure(&mut socket,error).await;return;}
                            if !send(&mut socket,&mut rate,json!({"type":"pong"})).await {return;}
                            continue;
                        }
                        let Some(room) = command.target() else {close(&mut socket,1008,"Invalid room").await;return;};
                        if kind == "leave" { rooms.retain(|r|r!=&room);continue; }
                        if kind != "join" {close(&mut socket,1008,"Client publication is unavailable").await;return;}
                        let allowed = match authorize(&app,&headers,id,&room,None).await {Ok(allowed)=>allowed,Err(error)=>{check_failure(&mut socket,error).await;return;}};
                        if !allowed {
                            if !send(&mut socket,&mut rate,json!({"type":"error","code":"ROOM_FORBIDDEN"})).await {return;}
                            continue;
                        }
                        if !join_room(&mut rooms, room) {close(&mut socket,1008,"Room limit exceeded").await;return;}
                        if !send(&mut socket,&mut rate,json!({"type":"resync","reason":"joined"})).await {return;}
                    },
                    Message::Close(_) => return,
                    Message::Binary(_) => {close(&mut socket,1008,"Text commands required").await;return;},
                    Message::Pong(_) => last_pong = Instant::now(),
                    Message::Ping(_) => {},
                }
            },
            event = events.recv() => {
                let event = match event {Ok(event)=>event,Err(broadcast::error::RecvError::Lagged(_))=>{close(&mut socket,1013,"Resynchronize after overload").await;return;},Err(_)=>return};
                let interested = rooms.iter().any(|room|match room {Room::Own=>event.owner==id,Room::Resource{name,id}=>name==&event.resource && id.is_none_or(|id|id==event.id)});
                if !interested {continue;}
                let effective = Room::Resource { name: event.resource.clone(), id: Some(event.id) };
                let allowed = match authorize(&app, &headers, id, &effective, Some(&event)).await {Ok(allowed)=>allowed,Err(error)=>{check_failure(&mut socket,error).await;return;}};
                if allowed && !send(&mut socket,&mut rate,payload(&event)).await {return;}
            },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_opt_in_requires_bounded_distinct_canonical_origins() {
        assert!(SocketConfig::new(vec![]).is_err());
        for invalid in [
            "null",
            "https://*.example.test",
            "https://user@example.test",
            "https://example.test/path",
            "https://example.test?token=x",
            "https://example.test#x",
            "https://example.test/",
            "HTTPS://example.test",
            "https://example.test\n",
        ] {
            assert!(SocketConfig::new(vec![invalid.into()]).is_err());
        }
        assert!(SocketConfig::new(vec!["https://example.test".into(); 2]).is_err());
        assert!(
            SocketConfig::new(
                (0..9)
                    .map(|i| format!("https://host{i}.example.test"))
                    .collect()
            )
            .is_err()
        );
        assert!(
            SocketConfig::new(vec![
                "https://example.test".into(),
                "http://127.0.0.1:4000".into(),
                "http://[::1]:4000".into()
            ])
            .is_ok()
        );
    }

    #[test]
    fn windows_and_room_admission_enforce_real_caps_without_growing_on_failure() {
        let now = Instant::now();
        let mut rate = Window::new(now);
        for _ in 0..16 {
            assert!(rate.accept(now, false));
        }
        assert!(!rate.accept(now, false));
        for _ in 0..32 {
            assert!(rate.accept(now, true));
        }
        assert!(!rate.accept(now, true));
        assert!(rate.accept(now + Duration::from_secs(1), false));
        assert!(rate.accept(now + Duration::from_secs(1), true));
        let mut rooms = Vec::new();
        for _ in 0..ROOMS {
            assert!(join_room(
                &mut rooms,
                Room::Resource {
                    name: "posts".into(),
                    id: Some(Uuid::new_v4())
                }
            ));
        }
        let duplicate = rooms[0].clone();
        assert!(join_room(&mut rooms, duplicate));
        assert!(!join_room(&mut rooms, Room::Own));
        assert_eq!(rooms.len(), ROOMS);
    }

    #[test]
    fn capacity_is_bounded_and_owned_permits_release_slots() {
        let broker = Arc::new(Broker::new(vec![]));
        let id = Uuid::new_v4();
        let permits: Vec<_> = (0..PER_USER).map(|_| broker.acquire(id).unwrap()).collect();
        assert!(broker.acquire(id).is_none());
        drop(permits);
        assert!(broker.acquire(id).is_some());
        assert!(broker.users.lock().unwrap().is_empty());
    }
    #[test]
    fn process_capacity_shutdown_and_event_retention_are_finite() {
        let broker = Arc::new(Broker::new(vec![]));
        let permits = (0..CONNECTIONS)
            .map(|_| broker.acquire(Uuid::new_v4()).unwrap())
            .collect::<Vec<_>>();
        assert!(broker.acquire(Uuid::new_v4()).is_none());
        assert_eq!(*broker.active.borrow(), CONNECTIONS);
        drop(permits);
        assert_eq!(*broker.active.borrow(), 0);
        let mut receiver = broker.events.subscribe();
        let record = json!({"id":Uuid::new_v4(),"owner":Uuid::new_v4()});
        for _ in 0..=EVENTS {
            broker.changed("posts", "create", &record);
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
        let control = SocketControl::new(broker.clone());
        control.shutdown();
        assert!(broker.acquire(Uuid::new_v4()).is_none());
    }

    #[tokio::test]
    async fn shutdown_completion_waits_for_owned_permits_and_releases_checks() {
        let broker = Arc::new(Broker::new(vec![]));
        let permit = broker.acquire(Uuid::new_v4()).unwrap();
        let control = SocketControl::new(broker.clone());
        control.shutdown();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), control.closed())
                .await
                .is_err()
        );
        drop(permit);
        tokio::time::timeout(Duration::from_secs(1), control.closed())
            .await
            .unwrap()
            .unwrap();
        let permits = (0..CHECKS)
            .map(|_| broker.checks.try_acquire().unwrap())
            .collect::<Vec<_>>();
        assert!(broker.checks.try_acquire().is_err());
        drop(permits);
        assert_eq!(broker.checks.available_permits(), CHECKS);
    }

    #[tokio::test]
    async fn closure_reports_its_own_deadline_instead_of_hanging_on_retained_work() {
        let broker = Arc::new(Broker::new(vec![]));
        let permit = broker.acquire(Uuid::new_v4()).unwrap();
        let control = SocketControl::new(broker);
        assert!(control.closed().await.is_err());
        control.shutdown();
        let error = tokio::time::timeout(Duration::from_secs(10), control.closed())
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.contains("eight seconds"));
        drop(permit);
        control.closed().await.unwrap();
    }

    #[test]
    fn metadata_never_contains_record_data_and_unknown_commands_are_rejected() {
        let broker = Broker::new(vec![]);
        let mut events = broker.events.subscribe();
        let id = Uuid::new_v4();
        broker.changed(
            "posts",
            "create",
            &json!({"id":id,"owner":Uuid::new_v4(),"data":{"password":"must never be emitted", "large":"q".repeat(2 * 1024 * 1024)}}),
        );
        let event = events.try_recv().unwrap();
        assert_eq!(event.id, id);
        assert_eq!(event.operation, "create");
        let encoded = serde_json::to_string(&payload(&event)).unwrap();
        assert!(!encoded.contains("must never be emitted"));
        assert!(!encoded.contains("password"));
        assert!(!encoded.contains("owner"));
        assert_eq!(payload(&event).as_object().unwrap().len(), 5);
        let mut widest = event.clone();
        widest.resource = "a".repeat(48);
        assert!(serde_json::to_vec(&payload(&widest)).unwrap().len() <= 256);
        broker.changed(&"a".repeat(49), "create", &json!({"id":id,"owner":id}));
        broker.changed("posts", "publish", &json!({"id":id,"owner":id}));
        broker.changed("posts", "create", &json!({"id":id}));
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(
            serde_json::from_value::<Command>(json!({"type":"join","room":"user:someone-else"}))
                .unwrap()
                .target()
                .is_none()
        );
        assert!(
            serde_json::from_value::<Command>(
                json!({"type":"join","resource":"posts","payload":{}})
            )
            .is_err()
        );
    }
}
