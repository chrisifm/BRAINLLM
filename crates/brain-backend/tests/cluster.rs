//! Two backend "processes" (two routers on one database) share rate windows and socket fan-out.
use brain_backend::{
    auth, db, http,
    platform::Platform,
    routes::{Route, RouteFailure, RouteHandlers},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::{net::SocketAddr, str::FromStr, sync::Arc, time::Duration};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::{Message, client::IntoClientRequest}};
use uuid::Uuid;

const ORIGIN: &str = "https://shop.example.test";
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct Quote;
impl RouteHandlers for Quote {
    fn names(&self) -> Vec<String> {
        vec!["quote".into()]
    }
    fn run(&self, _: &str, _: Value, _: Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, RouteFailure>> + Send + '_>> {
        Box::pin(async { Ok(json!({"price": "12.50"})) })
    }
}

async fn database(url: &str) -> sqlx::PgPool {
    let base = db::connect(url).await.unwrap();
    let name = format!("brain_cluster_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    base.close().await;
    let options = sqlx::postgres::PgConnectOptions::from_str(url).unwrap().database(&name);
    db::connect(options.to_url_lossy().as_str()).await.unwrap()
}

fn resources() -> Vec<brain_backend::Resource> {
    vec![serde_json::from_value(json!({"name":"orders","fields":{"title":{"kind":"str","required":true}},"policy":{"read":["owner","admin"],"create":["candidate","admin"],"update":["owner","admin"],"delete":["owner","admin"]}})).unwrap()]
}

/// One backend process: its own router, broker and limiter memory over the shared database.
async fn process(pool: &sqlx::PgPool, shared: bool) -> (String, http::SocketControl) {
    let config = brain_backend::Config {
        api_keys: None,
        roles: Default::default(),
        mail: None,
        auth_public_base_url: None,
        webhooks: None,
        before_hooks: None,
        sockets: Some(http::SocketConfig::new(vec![ORIGIN.into()]).unwrap()),
        resources: resources(),
        jobs: vec![],
        uploads: std::env::temp_dir().join(format!("brain-cluster-{}", Uuid::new_v4())),
        origins: vec![ORIGIN.into()],
        production: true,
        jwt: None,
        google: None,
        gcs: None,
    };
    let quote = Route::from_contract("quote", &json!({"method": "GET", "path": "/quotes", "auth": "public", "rate": "3/min"}), &|_: &str| None).unwrap();
    let platform = Platform {
        routes: vec![quote],
        handlers: Some(Arc::new(Quote)),
        cluster: brain_backend::cluster::Cluster { shared },
        ..Default::default()
    };
    let (router, control) = http::router_with_platform(pool.clone(), config, platform).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await;
    });
    (format!("127.0.0.1:{}", address.port()), control)
}

async fn session(pool: &sqlx::PgPool) -> String {
    let id = Uuid::new_v4();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role) VALUES($1,$2,'synthetic-unused-hash','candidate')")
        .bind(id).bind(format!("{id}@example.test")).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO _brain_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')")
        .bind(auth::digest(&token)).bind(id).execute(pool).await.unwrap();
    format!("{}={token}", auth::COOKIE)
}

async fn frame(socket: &mut Socket, wait: Duration) -> Option<Value> {
    loop {
        match tokio::time::timeout(wait, socket.next()).await {
            Err(_) => return None,
            Ok(message) => match message.unwrap().unwrap() {
                Message::Text(text) => return Some(serde_json::from_str(&text).unwrap()),
                Message::Ping(_) => socket.flush().await.unwrap(),
                _ => return None,
            },
        }
    }
}

async fn connect(address: &str, cookie: &str) -> Socket {
    let mut request = format!("ws://{address}/_brain/socket").into_client_request().unwrap();
    request.headers_mut().insert("Sec-WebSocket-Protocol", "brain.v1".parse().unwrap());
    request.headers_mut().insert("Cookie", cookie.parse().unwrap());
    request.headers_mut().insert("Origin", ORIGIN.parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(frame(&mut socket, Duration::from_secs(5)).await.unwrap()["type"], "ready");
    socket.send(Message::Text(json!({"type":"join","resource":"orders"}).to_string().into())).await.unwrap();
    assert_eq!(frame(&mut socket, Duration::from_secs(5)).await.unwrap()["type"], "resync");
    socket
}

async fn quote_status(client: &reqwest::Client, address: &str) -> (u16, Option<String>) {
    let response = client.get(format!("http://{address}/quotes")).send().await.unwrap();
    (response.status().as_u16(), response.headers().get("retry-after").map(|v| v.to_str().unwrap().to_string()))
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn shared_cluster_counts_route_rates_once_and_relays_socket_events_between_processes() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let pool = database(&url).await;
    db::migrate(&pool, &resources()).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
    let (a, control_a) = process(&pool, true).await;
    let (b, control_b) = process(&pool, true).await;

    // Route rate `3/min` is one budget for the deployment: 2 hits on A + 1 on B exhaust it for both.
    assert_eq!(quote_status(&client, &a).await.0, 200);
    assert_eq!(quote_status(&client, &a).await.0, 200);
    assert_eq!(quote_status(&client, &b).await.0, 200);
    let (status, retry) = quote_status(&client, &b).await;
    assert_eq!(status, 429, "process B must see the hits counted by process A");
    assert!(retry.unwrap().parse::<u64>().unwrap() >= 1);
    assert_eq!(quote_status(&client, &a).await.0, 429);
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_rate_windows WHERE scope='route:quote'").fetch_one(&pool).await.unwrap();
    assert_eq!(stored, 1, "one window row per (scope, subject), the subject stored as a digest");

    // A write committed through process A reaches a socket connected to process B.
    let cookie = session(&pool).await;
    let mut on_b = connect(&b, &cookie).await;
    tokio::time::sleep(Duration::from_millis(300)).await; // listener subscribed before the write
    let created: Value = client
        .post(format!("http://{a}/api/orders"))
        .header("Origin", ORIGIN)
        .header("Cookie", &cookie)
        .json(&json!({"title": "Two lamps"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let event = frame(&mut on_b, Duration::from_secs(5)).await.expect("process B relays the change committed on A");
    assert_eq!(event["id"], created["id"]);
    assert_eq!(event["operation"], "create");
    assert!(!event.to_string().contains("Two lamps"), "row values never cross the relay");
    control_a.shutdown();
    control_b.shutdown();

    // Without the shared cluster each process keeps its own memory: B never sees A's hits or writes.
    let (c, control_c) = process(&pool, false).await;
    let (d, control_d) = process(&pool, false).await;
    for _ in 0..3 {
        assert_eq!(quote_status(&client, &c).await.0, 200);
    }
    assert_eq!(quote_status(&client, &d).await.0, 200, "separate in-memory windows per process");
    let mut on_d = connect(&d, &cookie).await;
    let status = client.post(format!("http://{c}/api/orders")).header("Origin", ORIGIN).header("Cookie", &cookie).json(&json!({"title": "Desk"})).send().await.unwrap().status();
    assert_eq!(status, 200);
    assert!(frame(&mut on_d, Duration::from_millis(800)).await.is_none(), "single-process brokers do not relay");
    control_c.shutdown();
    control_d.shutdown();
}
