//! Real loopback Upgrade and Postgres policies; no external sockets or production databases.
use brain_backend::{auth, auth_flows::RolesConfig, db, http, jwt};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::{net::SocketAddr, path::PathBuf, str::FromStr, time::Duration};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest, protocol::frame::coding::CloseCode},
};
use uuid::Uuid;

const ORIGIN: &str = "https://app.example.test";
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
struct Actor {
    id: Uuid,
    token: String,
    cookie: String,
}
struct Server {
    url: String,
    ws: String,
    pool: sqlx::PgPool,
    client: reqwest::Client,
    control: http::SocketControl,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
    files: PathBuf,
    jwt: jwt::Config,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.control.shutdown();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let _ = std::fs::remove_dir_all(&self.files);
    }
}
impl Server {
    async fn new() -> Self {
        Self::configured(true).await
    }
    async fn configured(enabled: bool) -> Self {
        let url = std::env::var("BRAIN_BACKEND_TEST_URL")
            .expect("Use the explicitly disposable backend harness only");
        let base = db::connect(&url).await.unwrap();
        let name = format!("brain_sockets_{}", Uuid::new_v4().simple());
        // Fixed prefix and UUID hex, never request data.
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&base)
            .await
            .unwrap();
        let options = sqlx::postgres::PgConnectOptions::from_str(&url)
            .unwrap()
            .database(&name);
        let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
        base.close().await;
        let owner = json!({"name":"posts","fields":{"title":{"kind":"str","required":true}},"policy":{"read":["owner","admin"],"create":["candidate","admin"],"update":["owner","admin"],"delete":["owner","admin"]}});
        let staff = json!({"name":"staff_posts","fields":{"title":{"kind":"str","required":true}},"policy":{"read":["candidate","admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}});
        let resources = vec![
            serde_json::from_value(owner).unwrap(),
            serde_json::from_value(staff).unwrap(),
        ];
        db::migrate(&pool, &resources).await.unwrap();
        let files =
            std::env::temp_dir().join(format!("brain-socket-files-{}", Uuid::new_v4().simple()));
        let jwt = jwt::Config::new(
            "synthetic-socket-signing-key-over-32-bytes".into(),
            "socket-test".into(),
            "socket-test".into(),
        )
        .unwrap();
        let config = brain_backend::Config { api_keys: None,
            resources,
            jobs: vec![],
            uploads: files.clone(),
            origins: vec![ORIGIN.into()],
            production: true,
            jwt: Some(jwt.clone()),
            google: None,
            gcs: None,
            roles: RolesConfig::default(),
            mail: None,
            auth_public_base_url: None,
            webhooks: None,
            before_hooks: None,
            sockets: enabled.then(|| http::SocketConfig::new(vec![ORIGIN.into()]).unwrap()),
        };
        let (router, control) = http::router_with_socket_control(pool.clone(), config)
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
        });
        Self {
            url: format!("http://{address}"),
            ws: format!("ws://{address}/_brain/socket"),
            pool,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            control,
            stop: Some(stop),
            task: Some(task),
            files,
            jwt,
        }
    }
    async fn actor(&self, role: &str) -> Actor {
        let id = Uuid::new_v4();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role) VALUES($1,$2,'synthetic-unused-hash',$3)")
            .bind(id).bind(format!("{id}@example.test")).bind(role).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO _brain_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')")
            .bind(auth::digest(&token)).bind(id).execute(&self.pool).await.unwrap();
        Actor {
            id,
            cookie: format!("{}={token}", auth::COOKIE),
            token,
        }
    }
    fn request(
        &self,
        cookie: Option<&str>,
        origin: Option<&str>,
        suffix: &str,
    ) -> tokio_tungstenite::tungstenite::http::Request<()> {
        let mut request = format!("{}{suffix}", self.ws)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", "brain.v1".parse().unwrap());
        if let Some(cookie) = cookie {
            request
                .headers_mut()
                .insert("Cookie", cookie.parse().unwrap());
        }
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("Origin", origin.parse().unwrap());
        }
        request
    }
    async fn connect(&self, actor: &Actor) -> Socket {
        let (mut socket, response) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::connect_async(self.request(Some(&actor.cookie), Some(ORIGIN), "")),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.headers()["sec-websocket-protocol"], "brain.v1");
        assert_eq!(
            frame(&mut socket).await,
            json!({"type":"ready","protocol":1,"durable":false,"resyncRequired":true})
        );
        socket
    }
    async fn call(
        &self,
        actor: &Actor,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.url))
            .header("Origin", ORIGIN)
            .header("Cookie", &actor.cookie);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req.send().await.unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap())
    }
    async fn finish(mut self) {
        self.control.shutdown();
        tokio::time::timeout(Duration::from_secs(9), self.control.closed())
            .await
            .unwrap()
            .unwrap();
        let _ = self.stop.take().unwrap().send(());
        tokio::time::timeout(Duration::from_secs(6), self.task.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        self.pool.close().await;
    }
}
async fn frame(socket: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(6), socket.next())
            .await
            .expect("socket response deadline")
            .unwrap()
            .unwrap();
        match message {
            Message::Text(text) => {
                assert!(text.len() <= 4096);
                return serde_json::from_str(&text).unwrap();
            }
            Message::Ping(_) => {
                socket.flush().await.unwrap();
            }
            _ => panic!("Expected bounded JSON metadata"),
        }
    }
}
async fn closed(socket: &mut Socket, wait: Duration) -> CloseCode {
    loop {
        match tokio::time::timeout(wait, socket.next())
            .await
            .expect("owned socket must close")
            .unwrap()
            .unwrap()
        {
            Message::Close(Some(frame)) => return frame.code,
            Message::Ping(_) => socket.flush().await.unwrap(),
            _ => panic!("No invalidation may follow revocation"),
        }
    }
}
async fn command(socket: &mut Socket, value: Value) -> Value {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
    frame(socket).await
}
async fn denied(request: tokio_tungstenite::tungstenite::http::Request<()>, code: u16) {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .unwrap();
    match result {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status().as_u16(), code)
        }
        _ => panic!("The upgrade must be rejected before establishing a socket"),
    }
}

#[tokio::test]
#[ignore = "Use the owned disposable Postgres harness; opens loopback TCP only"]
async fn authenticated_metadata_rechecks_row_policy_revocation_origins_and_committed_mutations() {
    let server = Server::new().await;
    let alice = server.actor("candidate").await;
    let bob = server.actor("candidate").await;
    let admin = server.actor("admin").await;
    denied(server.request(Some(&alice.cookie), None, ""), 403).await;
    denied(
        server.request(
            Some(&alice.cookie),
            Some("https://untrusted.example.test"),
            "",
        ),
        403,
    )
    .await;
    denied(server.request(None, Some(ORIGIN), ""), 401).await;
    denied(
        server.request(
            Some(&alice.cookie),
            Some(ORIGIN),
            "?token=forbidden-url-token",
        ),
        403,
    )
    .await;
    let mut duplicate = server.request(Some(&alice.cookie), Some(ORIGIN), "");
    duplicate
        .headers_mut()
        .append("Origin", ORIGIN.parse().unwrap());
    denied(duplicate, 403).await;
    let mut protocol = server.request(Some(&alice.cookie), Some(ORIGIN), "");
    protocol.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "token-is-not-a-protocol".parse().unwrap(),
    );
    denied(protocol, 403).await;

    let mut a = server.connect(&alice).await;
    let mut b = server.connect(&bob).await;
    for socket in [&mut a, &mut b] {
        assert_eq!(
            command(socket, json!({"type":"join","resource":"posts"})).await["type"],
            "resync"
        );
    }
    assert_eq!(
        command(&mut a, json!({"type":"join","resource":"staff_posts"})).await["type"],
        "resync"
    );
    let (status, bob_record) = server
        .call(
            &bob,
            reqwest::Method::POST,
            "/api/posts",
            Some(json!({"title":"private-bob-record"})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(frame(&mut b).await["id"], bob_record["id"]);
    assert_eq!(
        command(
            &mut a,
            json!({"type":"join","resource":"posts","id":bob_record["id"]})
        )
        .await["code"],
        "ROOM_FORBIDDEN"
    );

    // Force a real transaction failure, then prove no rolled-back event precedes the next committed record.
    sqlx::raw_sql("CREATE FUNCTION socket_reject() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.data->>'title'='reject-transaction' THEN RAISE EXCEPTION 'synthetic transaction rejected'; END IF; RETURN NEW; END $$; CREATE TRIGGER socket_reject BEFORE INSERT ON _brain_res_posts FOR EACH ROW EXECUTE FUNCTION socket_reject()")
        .execute(&server.pool).await.unwrap();
    let (status, _) = server
        .call(
            &alice,
            reqwest::Method::POST,
            "/api/posts",
            Some(json!({"title":"reject-transaction"})),
        )
        .await;
    assert_eq!(status, 500);
    let (status, record) = server
        .call(
            &alice,
            reqwest::Method::POST,
            "/api/posts",
            Some(json!({"title":"private-alice-record"})),
        )
        .await;
    assert_eq!(status, 200);
    let event = frame(&mut a).await;
    assert_eq!(
        event["id"], record["id"],
        "foreign-owner and failed-transaction ids must never reach this socket"
    );
    assert_eq!(event["operation"], "create");
    assert_eq!(event.as_object().unwrap().len(), 5);
    assert!(!event.to_string().contains("private-alice-record"));
    assert!(event.get("owner").is_none());
    assert!(event.get("data").is_none());
    assert!(Uuid::parse_str(event["eventId"].as_str().unwrap()).is_ok());

    // A database role change applies to an already joined room on its very next delivery.
    sqlx::query("UPDATE _brain_users SET role='company' WHERE id=$1")
        .bind(alice.id)
        .execute(&server.pool)
        .await
        .unwrap();
    let (status, _) = server
        .call(
            &admin,
            reqwest::Method::POST,
            "/api/staff_posts",
            Some(json!({"title":"staff-only-record"})),
        )
        .await;
    assert_eq!(status, 200);
    let path = format!("/api/posts/{}", record["id"].as_str().unwrap());
    let (status, _) = server
        .call(
            &admin,
            reqwest::Method::PUT,
            &path,
            Some(json!({"title":"updated-private-record"})),
        )
        .await;
    assert_eq!(status, 200);
    let event = frame(&mut a).await;
    assert_eq!(event["id"], record["id"]);
    assert_eq!(event["operation"], "update");
    // Legacy/direct SQL can contain a large row. Socket authority queries and the ring retain only UUIDs.
    sqlx::query("UPDATE _brain_res_posts SET data=jsonb_build_object('title',repeat('q',2097152)) WHERE id=$1")
        .bind(Uuid::parse_str(record["id"].as_str().unwrap()).unwrap()).execute(&server.pool).await.unwrap();
    assert_eq!(
        command(
            &mut a,
            json!({"type":"join","resource":"posts","id":record["id"]})
        )
        .await["type"],
        "resync"
    );
    let (status, _) = server
        .call(&admin, reqwest::Method::DELETE, &path, None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        frame(&mut a).await["operation"],
        "delete",
        "deleted rows use their authoritative pre-hook owner snapshot"
    );

    // Opaque session revocation is checked before the next interested event is emitted.
    sqlx::query("DELETE FROM _brain_sessions WHERE user_id=$1")
        .bind(alice.id)
        .execute(&server.pool)
        .await
        .unwrap();
    let (status, _) = server
        .call(
            &admin,
            reqwest::Method::POST,
            "/api/staff_posts",
            Some(json!({"title":"must-not-be-delivered"})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        closed(&mut a, Duration::from_secs(6)).await,
        CloseCode::Policy
    );

    // JWT uses the same current-user authority, including disabled state, without a URL token.
    let native = auth::Auth::new(server.pool.clone()).await.unwrap();
    let user = native.user(Some(&bob.token)).await.unwrap().unwrap();
    let tokens = jwt::Jwt::new(server.pool.clone(), server.jwt.clone())
        .issue(user, &bob.token)
        .await
        .unwrap();
    let mut request = server.request(None, Some(ORIGIN), "");
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", tokens.access_token).parse().unwrap(),
    );
    let (mut jwt_socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(frame(&mut jwt_socket).await["type"], "ready");
    let mut invalid = server.request(Some(&bob.cookie), Some(ORIGIN), "");
    invalid
        .headers_mut()
        .insert("Authorization", "Bearer invalid".parse().unwrap());
    denied(invalid, 401).await;
    sqlx::query("UPDATE _brain_users SET disabled_at=now() WHERE id=$1")
        .bind(bob.id)
        .execute(&server.pool)
        .await
        .unwrap();
    assert_eq!(
        closed(&mut jwt_socket, Duration::from_secs(40)).await,
        CloseCode::Policy,
        "idle JWT connections must reauthenticate on heartbeat"
    );
    assert_eq!(
        closed(&mut b, Duration::from_secs(40)).await,
        CloseCode::Policy
    );

    // Native stop owns upgraded connections as well as the HTTP listener.
    let mut live = server.connect(&admin).await;
    server.control.shutdown();
    assert_eq!(
        closed(&mut live, Duration::from_secs(6)).await,
        CloseCode::Away
    );
    denied(server.request(Some(&admin.cookie), Some(ORIGIN), ""), 503).await;
    server.finish().await;
}

#[tokio::test]
#[ignore = "Use the owned disposable Postgres harness; opens loopback TCP only"]
async fn native_protocol_caps_connections_frames_and_client_publication() {
    let server = Server::new().await;
    let actor = server.actor("candidate").await;
    let mut sockets = Vec::new();
    for _ in 0..4 {
        sockets.push(server.connect(&actor).await);
    }
    denied(server.request(Some(&actor.cookie), Some(ORIGIN), ""), 503).await;
    sockets[0]
        .send(Message::Text(
            json!({"type":"publish","resource":"posts"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        closed(&mut sockets[0], Duration::from_secs(6)).await,
        CloseCode::Policy
    );
    sockets[1]
        .send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    assert_eq!(
        closed(&mut sockets[1], Duration::from_secs(6)).await,
        CloseCode::Policy
    );
    sockets[2]
        .send(Message::Text("a".repeat(4097).into()))
        .await
        .unwrap();
    let oversized = tokio::time::timeout(Duration::from_secs(6), sockets[2].next())
        .await
        .unwrap();
    assert!(matches!(
        oversized,
        None | Some(Err(_)) | Some(Ok(Message::Close(_)))
    ));
    server.control.shutdown();
    assert_eq!(
        closed(&mut sockets[3], Duration::from_secs(6)).await,
        CloseCode::Away
    );
    drop(sockets);
    server.finish().await;
}

#[tokio::test]
#[ignore = "Use the owned disposable Postgres harness; opens loopback TCP only"]
async fn native_socket_opt_in_is_absent_by_default_and_origin_subset_is_required() {
    let server = Server::configured(false).await;
    let actor = server.actor("candidate").await;
    denied(server.request(Some(&actor.cookie), Some(ORIGIN), ""), 404).await;
    assert_eq!(
        server
            .client
            .get(format!("{}/health", server.url))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        server
            .call(
                &actor,
                reqwest::Method::POST,
                "/api/posts",
                Some(json!({"title":"http-still-available"}))
            )
            .await
            .0,
        200
    );
    let invalid = brain_backend::Config { api_keys: None,
        resources: vec![],
        jobs: vec![],
        uploads: server.files.clone(),
        origins: vec![ORIGIN.into()],
        production: true,
        jwt: None,
        google: None,
        gcs: None,
        roles: RolesConfig::default(),
        mail: None,
        auth_public_base_url: None,
        webhooks: None,
        before_hooks: None,
        sockets: Some(http::SocketConfig::new(vec!["https://other.example.test".into()]).unwrap()),
    };
    let error = http::router(server.pool.clone(), invalid)
        .await
        .err()
        .unwrap();
    assert!(error.contains("subset"));
    server.finish().await;
}

#[tokio::test]
#[ignore = "Use the owned disposable Postgres harness; opens loopback TCP only"]
async fn sixteen_admitted_users_share_one_committed_event_without_a_reconnect_storm() {
    let server = Server::new().await;
    let admin = server.actor("admin").await;
    let mut sockets = Vec::new();
    for _ in 0..16 {
        let actor = server.actor("candidate").await;
        let mut socket = server.connect(&actor).await;
        assert_eq!(
            command(&mut socket, json!({"type":"join","resource":"staff_posts"})).await["type"],
            "resync"
        );
        sockets.push(socket);
    }
    let (status, record) = server
        .call(
            &admin,
            reqwest::Method::POST,
            "/api/staff_posts",
            Some(json!({"title":"shared-event-private-row"})),
        )
        .await;
    assert_eq!(status, 200);
    let events = futures_util::future::join_all(sockets.iter_mut().map(frame)).await;
    assert_eq!(events.len(), 16);
    for event in &events {
        assert_eq!(event["id"], record["id"]);
        assert_eq!(event["eventId"], events[0]["eventId"]);
        assert_eq!(event["type"], "invalidate");
        assert!(serde_json::to_vec(event).unwrap().len() <= 256);
        assert!(!event.to_string().contains("shared-event-private-row"));
    }
    let replies = futures_util::future::join_all(
        sockets
            .iter_mut()
            .map(|socket| command(socket, json!({"type":"ping"}))),
    )
    .await;
    assert!(replies.iter().all(|value| value["type"] == "pong"));
    server.control.shutdown();
    for socket in &mut sockets {
        assert_eq!(
            closed(socket, Duration::from_secs(6)).await,
            CloseCode::Away
        );
    }
    drop(sockets);
    server.finish().await;
}
