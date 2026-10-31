//! HTTP edge: trusted proxies, readiness, draining, backpressure and CORS preflight cache.
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::ConnectInfo,
    http::{Request, Response},
};
use brain_backend::{
    Resource,
    platform::{Edge, Platform},
};
use serde_json::{Value, json};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "https://shop.example.test";
const ADMIN: &str = "https://admin.shop.example.test";

fn request(method: &str, path: &str, headers: &[(&str, &str)]) -> axum::http::request::Builder {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("accept", "application/json")
        .extension(ConnectInfo("127.0.0.1:42002".parse::<std::net::SocketAddr>().unwrap()));
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    builder
}
async fn send(http: &Router, method: &str, path: &str, headers: &[(&str, &str)]) -> Response<Body> {
    http.clone().oneshot(request(method, path, headers).body(Body::empty()).unwrap()).await.unwrap()
}
async fn json_body(response: Response<Body>) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap()
}
fn config(resources: Vec<Resource>) -> brain_backend::Config {
    brain_backend::Config {
        api_keys: None,
        roles: Default::default(),
        mail: None,
        auth_public_base_url: None,
        webhooks: None,
        before_hooks: None,
        sockets: None,
        resources,
        jobs: vec![],
        uploads: std::env::temp_dir().join(format!("brain-edge-{}", Uuid::new_v4())),
        origins: vec![ORIGIN.into()],
        production: true,
        jwt: None,
        google: None,
        gcs: None,
    }
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn edge_resolves_clients_reports_readiness_drains_and_sheds_load() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    let edge = Edge::from_contract(&json!({
        "trustProxy": ["127.0.0.1"],
        "cors": {"origins": [ADMIN], "maxAge": 600},
        "limits": {"maxInFlight": 1}
    }))
    .unwrap();
    let (http, _, control) = brain_backend::http::router_with_control(
        pool.clone(),
        config(vec![]),
        Platform { edge, ..Default::default() },
    )
    .await
    .unwrap();

    // Probes.
    let live = send(&http, "GET", "/health/live", &[]).await;
    assert_eq!(live.status(), 200);
    assert_eq!(live.headers()["strict-transport-security"], "max-age=31536000; includeSubDomains", "production pins HTTPS");
    assert_eq!(json_body(live).await["status"], "live");
    let ready = send(&http, "GET", "/health/ready", &[]).await;
    assert_eq!(ready.status(), 200);
    assert_eq!(json_body(ready).await["status"], "ready");

    // Rate limiting keys the forwarded client, not the loopback proxy.
    let mut limited = false;
    for _ in 0..400 {
        let response = send(&http, "GET", "/health/live", &[("x-forwarded-for", "203.0.113.9")]).await;
        if response.status() == 429 {
            limited = true;
            break;
        }
    }
    assert!(limited, "one forwarded client exhausts its own bucket");
    let other = send(&http, "GET", "/health/live", &[("x-forwarded-for", "1.1.1.1, 203.0.113.10")]).await;
    assert_eq!(other.status(), 200, "another client behind the same proxy keeps its own bucket");
    let spoof = send(&http, "GET", "/health/live", &[("x-forwarded-for", "198.51.100.1, 203.0.113.9")]).await;
    assert_eq!(spoof.status(), 429, "a spoofed left-most hop does not mint a new bucket");

    // CORS: the extra origin is allowed and preflights are cacheable.
    let preflight = send(&http, "OPTIONS", "/api/anything", &[
        ("origin", ADMIN),
        ("access-control-request-method", "POST"),
        ("x-forwarded-for", "203.0.113.20"),
    ])
    .await;
    assert_eq!(preflight.headers()["access-control-allow-origin"], ADMIN);
    assert_eq!(preflight.headers()["access-control-max-age"], "600");
    let logout = send(&http, "POST", "/_brain/auth/logout", &[("origin", ADMIN), ("x-forwarded-for", "203.0.113.21")]).await;
    assert_ne!(logout.status(), 403, "extra origins may send mutations");

    // Backpressure: a request whose body never arrives holds the only slot.
    let held = {
        let http = http.clone();
        tokio::spawn(async move {
            let body = Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
            let req = request("POST", "/_brain/auth/login", &[
                ("origin", ORIGIN),
                ("content-type", "application/json"),
                ("x-forwarded-for", "203.0.113.30"),
            ])
            .body(body)
            .unwrap();
            http.oneshot(req).await
        })
    };
    for _ in 0..500 {
        if control.in_flight() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(control.in_flight(), 1);
    let shed = send(&http, "GET", "/api/anything", &[("x-forwarded-for", "203.0.113.31")]).await;
    assert_eq!(shed.status(), 503);
    assert_eq!(shed.headers()["retry-after"], "1");
    assert_eq!(json_body(shed).await["error"]["code"], "OVERLOADED");
    let ready = send(&http, "GET", "/health/ready", &[]).await;
    assert_eq!(ready.status(), 503, "readiness reports saturation");
    assert_eq!(send(&http, "GET", "/health/live", &[("x-forwarded-for", "203.0.113.32")]).await.status(), 200, "liveness ignores load");
    held.abort();
    let _ = held.await;
    assert_eq!(control.in_flight(), 0, "a cancelled request releases its slot");
    assert_eq!(send(&http, "GET", "/health/ready", &[]).await.status(), 200);

    // Draining: readiness fails, liveness stays up.
    control.drain();
    let ready = send(&http, "GET", "/health/ready", &[]).await;
    assert_eq!(ready.status(), 503);
    assert_eq!(ready.headers()["retry-after"], "5");
    let raw = to_bytes(ready.into_body(), 1 << 20).await.unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("DRAINING"), "{raw:?}");
    assert_eq!(send(&http, "GET", "/health/live", &[("x-forwarded-for", "203.0.113.33")]).await.status(), 200);
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn a_saturated_pool_answers_overloaded_instead_of_internal_error() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let setup = brain_backend::db::connect(&url).await.unwrap();
    let suffix = Uuid::new_v4().simple().to_string()[..8].to_string();
    let notes: Resource = serde_json::from_value(json!({
        "name": format!("notes_{suffix}"),
        "fields": {"title": {"kind": "str", "required": true}},
        "policy": {"read": ["public"], "create": ["candidate"], "update": ["owner"], "delete": ["owner"]}
    }))
    .unwrap();
    brain_backend::db::migrate(&setup, std::slice::from_ref(&notes)).await.unwrap();
    setup.close().await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_millis(1000))
        .after_connect(|conn, _| Box::pin(async move {
            sqlx::raw_sql("SET search_path=public").execute(conn).await?;
            Ok(())
        }))
        .connect(&url)
        .await
        .unwrap();
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(vec![notes.clone()]), Platform::default())
        .await
        .map_err(|e| e.to_string())
        .unwrap();
    let held = (pool.acquire().await.unwrap(), pool.acquire().await.unwrap());
    let response = send(&http, "GET", &format!("/api/{}", notes.name), &[]).await;
    assert_eq!(response.status(), 503);
    assert_eq!(response.headers()["retry-after"], "2");
    assert_eq!(json_body(response).await["error"]["code"], "OVERLOADED");
    drop(held);
    assert_eq!(send(&http, "GET", &format!("/api/{}", notes.name), &[]).await.status(), 200);
}
