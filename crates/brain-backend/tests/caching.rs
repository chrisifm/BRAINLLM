//! HTTP caching: validators, 304, declared public lifetimes for anonymous reads and If-Match on writes.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, Response},
};
use brain_backend::{auth, db, http::cache_http::CachePolicy, platform::Platform};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::str::FromStr;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "https://shop.example.test";

fn resources() -> Vec<brain_backend::Resource> {
    let policy = json!({"read":["public"],"create":["candidate"],"update":["owner"],"delete":["owner"]});
    vec![
        serde_json::from_value(json!({"name":"products","fields":{"title":{"kind":"str","required":true}},"policy":policy})).unwrap(),
        serde_json::from_value(json!({"name":"notes","fields":{"title":{"kind":"str","required":true}},"policy":policy})).unwrap(),
    ]
}

async fn send(http: &Router, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Value>) -> Response<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("accept", "application/json")
        .extension(ConnectInfo("127.0.0.1:42011".parse::<std::net::SocketAddr>().unwrap()));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let body = match body {
        Some(value) => {
            req = req.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    http.clone().oneshot(req.body(body).unwrap()).await.unwrap()
}
fn header(response: &Response<Body>, name: &str) -> String {
    response.headers().get(name).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default()
}
async fn json_body(response: Response<Body>) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap()
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn reads_carry_validators_public_lifetimes_and_writes_honor_if_match() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_caching_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    base.close().await;
    let pool = db::connect(sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name).to_url_lossy().as_str()).await.unwrap();
    db::migrate(&pool, &resources()).await.unwrap();
    let user = Uuid::new_v4();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role) VALUES($1,$2,'synthetic-unused-hash','candidate')").bind(user).bind(format!("{user}@example.test")).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO _brain_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')").bind(auth::digest(&token)).bind(user).execute(&pool).await.unwrap();
    let cookie = format!("{}={token}", auth::COOKIE);
    let signed = [("origin", ORIGIN), ("cookie", cookie.as_str())];

    let cache = CachePolicy::from_contract(&json!({"products": {"maxAge": "1m", "staleWhileRevalidate": "10m"}}), &resources()).unwrap();
    let config = brain_backend::Config {
        api_keys: None, roles: Default::default(), mail: None, auth_public_base_url: None, webhooks: None, before_hooks: None,
        sockets: None, resources: resources(), jobs: vec![],
        uploads: std::env::temp_dir().join(format!("brain-caching-{}", Uuid::new_v4())),
        origins: vec![ORIGIN.into()], production: true, jwt: None, google: None, gcs: None,
    };
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config, Platform { cache, ..Default::default() }).await.unwrap();

    let created = json_body(send(&http, "POST", "/api/products", &signed, Some(json!({"title": "Garden hose"}))).await).await;
    let id = created["id"].as_str().unwrap().to_string();
    send(&http, "POST", "/api/notes", &signed, Some(json!({"title": "Restock"}))).await;

    // Anonymous list of a declared resource: public lifetime, weak validator, Vary on credentials.
    let list = send(&http, "GET", "/api/products", &[], None).await;
    assert_eq!(list.status(), 200);
    assert_eq!(header(&list, "cache-control"), "public, max-age=60, stale-while-revalidate=600");
    let list_tag = header(&list, "etag");
    assert!(list_tag.starts_with("W/\""), "{list_tag}");
    assert!(list.headers().get_all("vary").iter().any(|v| v.to_str().unwrap().contains("Cookie")));
    let again = send(&http, "GET", "/api/products", &[("if-none-match", list_tag.as_str())], None).await;
    assert_eq!(again.status(), 304);
    assert_eq!(header(&again, "etag"), list_tag);
    assert_eq!(header(&again, "cache-control"), "public, max-age=60, stale-while-revalidate=600");
    assert!(to_bytes(again.into_body(), 1024).await.unwrap().is_empty());

    // Credentials or an undeclared resource keep no-store; validators still work for API clients.
    let private = send(&http, "GET", "/api/products", &[("cookie", cookie.as_str())], None).await;
    assert_eq!(header(&private, "cache-control"), "no-store");
    assert!(!header(&private, "etag").is_empty());
    let notes = send(&http, "GET", "/api/notes", &[], None).await;
    assert_eq!(header(&notes, "cache-control"), "no-store");

    // One record: strong revision validator, 304 on match.
    let path = format!("/api/products/{id}");
    let read = send(&http, "GET", &path, &[], None).await;
    let tag = header(&read, "etag");
    assert!(tag.starts_with("\"r"), "{tag}");
    assert_eq!(send(&http, "GET", &path, &[("if-none-match", tag.as_str())], None).await.status(), 304);

    // If-Match: a stale or weak tag is refused with 412 and nothing changes; the current tag writes.
    let stale = send(&http, "PATCH", &path, &[signed[0], signed[1], ("if-match", "\"r000000000000000000000000\"")], Some(json!({"title": "Lost update"}))).await;
    assert_eq!(stale.status(), 412);
    assert_eq!(json_body(stale).await["error"]["code"], "PRECONDITION_FAILED");
    let weak = format!("W/{tag}");
    assert_eq!(send(&http, "PATCH", &path, &[signed[0], signed[1], ("if-match", weak.as_str())], Some(json!({"title": "Lost update"}))).await.status(), 412);
    let ok = send(&http, "PATCH", &path, &[signed[0], signed[1], ("if-match", tag.as_str())], Some(json!({"title": "Garden hose 20 m"}))).await;
    assert_eq!(ok.status(), 200);
    let fresh = header(&send(&http, "GET", &path, &[], None).await, "etag");
    assert_ne!(fresh, tag, "a write changes the record validator");
    assert_eq!(send(&http, "GET", &path, &[("if-none-match", tag.as_str())], None).await.status(), 200);
    assert_eq!(send(&http, "PUT", &path, &[signed[0], signed[1], ("if-match", tag.as_str())], Some(json!({"title": "Overwrite"}))).await.status(), 412);
    assert_eq!(send(&http, "DELETE", &path, &[signed[0], signed[1], ("if-match", tag.as_str())], None).await.status(), 412);
    assert_eq!(send(&http, "DELETE", &path, &[signed[0], signed[1], ("if-match", fresh.as_str())], None).await.status(), 200);
    let title = json_body(send(&http, "GET", "/api/products", &[], None).await).await;
    assert_eq!(title["items"].as_array().unwrap().len(), 0);
}
