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
        google: None, apple: None, linkedin: None,
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
        "policy": {"read": ["public"], "create": ["user"], "update": ["owner"], "delete": ["owner"]}
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

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn refused_origins_explain_themselves_in_development_and_stay_minimal_in_production() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    const STRANGER: &str = "https://blog.example.test";
    let refused = |http: Router| async move {
        let response = send(&http, "POST", "/_brain/auth/logout", &[("origin", STRANGER), ("content-type", "application/json")]).await;
        assert_eq!(response.status(), 403);
        assert!(response.headers().get("access-control-allow-origin").is_none(), "a refusal never reflects the origin");
        json_body(response).await
    };

    let (production, _) = brain_backend::http::router_with_platform(pool.clone(), config(vec![]), Platform::default()).await.map_err(|e| e.to_string()).unwrap();
    let body = refused(production).await;
    assert_eq!(body["error"]["code"], "ORIGIN_DENIED");
    assert_eq!(body["error"]["brainCode"], "B0740");
    assert!(body["error"].get("origin").is_none());
    assert!(!body.to_string().contains(ORIGIN), "production never reveals the allowlist: {body}");

    let mut development = config(vec![]);
    development.production = false;
    let (development, _) = brain_backend::http::router_with_platform(pool.clone(), development, Platform::default()).await.map_err(|e| e.to_string()).unwrap();
    let body = refused(development).await;
    let detail = &body["error"]["origin"];
    assert_eq!(detail["brainCode"], "B0740");
    assert_eq!(detail["seenOrigin"], STRANGER);
    assert_eq!(detail["allowedOrigins"], json!([ORIGIN]));
    assert!(detail["fix"].as_str().unwrap().contains(STRANGER) && !detail["fix"].as_str().unwrap().contains('*'), "{detail}");

    let mut outside = config(vec![]);
    outside.auth_public_base_url = Some(format!("{STRANGER}/app"));
    let error = brain_backend::http::router_with_platform(pool.clone(), outside, Platform::default()).await.map(|_| ()).unwrap_err().to_string();
    for needle in ["B0748", "https://blog.example.test/app", ORIGIN, "--origin https://blog.example.test"] {
        assert!(error.contains(needle), "{needle} missing from {error}");
    }
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn a_refused_origin_receives_no_cors_header_and_an_allowed_one_keeps_them() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    let edge = Edge::from_contract(&json!({"cors": {"origins": [ADMIN], "maxAge": 600}})).unwrap();
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(vec![]), Platform { edge, ..Default::default() }).await.map_err(|e| e.to_string()).unwrap();
    const STRANGER: &str = "https://evil.example";
    let cors_headers = |response: &Response<Body>| response.headers().keys().filter(|k| k.as_str().starts_with("access-control-")).map(|k| k.to_string()).collect::<Vec<_>>();
    let preflight = |origin: &'static str| [("origin", origin), ("access-control-request-method", "POST"), ("access-control-request-headers", "content-type")];
    for (method, path, headers) in [
        ("GET", "/health/live", vec![("origin", STRANGER)]),
        ("HEAD", "/health/live", vec![("origin", STRANGER)]),
        ("POST", "/_brain/auth/logout", vec![("origin", STRANGER), ("content-type", "application/json")]),
        ("OPTIONS", "/_brain/auth/logout", preflight(STRANGER).to_vec()),
    ] {
        let response = send(&http, method, path, &headers).await;
        assert_eq!(cors_headers(&response), Vec::<String>::new(), "{method} {path} from a refused origin");
    }
    for origin in [ORIGIN, ADMIN] {
        let read = send(&http, "GET", "/health/live", &[("origin", origin)]).await;
        assert_eq!(read.headers()["access-control-allow-origin"], origin);
        assert_eq!(read.headers()["access-control-allow-credentials"], "true");
        assert!(read.headers().contains_key("access-control-expose-headers"));
        assert!(read.headers().get_all("vary").iter().any(|v| v.to_str().unwrap().to_ascii_lowercase().contains("origin")), "shared caches must key on Origin");
        let allowed = send(&http, "OPTIONS", "/_brain/auth/logout", &preflight(origin)).await;
        assert_eq!(allowed.headers()["access-control-allow-origin"], origin);
        assert_eq!(allowed.headers()["access-control-max-age"], "600");
        assert!(allowed.headers().contains_key("access-control-allow-methods"));
    }
    let refused = send(&http, "GET", "/health/live", &[("origin", STRANGER)]).await;
    assert!(refused.headers().get_all("vary").iter().any(|v| v.to_str().unwrap().to_ascii_lowercase().contains("origin")), "a refusal varies on Origin too");
    let plain = send(&http, "GET", "/health/live", &[]).await;
    assert_eq!(cors_headers(&plain).iter().filter(|h| h.as_str() == "access-control-allow-origin").count(), 0);
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn auth_errors_separate_bad_input_from_a_busy_database() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let setup = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&setup, &[]).await.unwrap();
    setup.close().await;
    let pool = sqlx::postgres::PgPoolOptions::new().max_connections(2).acquire_timeout(Duration::from_millis(1000)).connect(&url).await.unwrap();
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(vec![]), Platform::default()).await.map_err(|e| e.to_string()).unwrap();
    let auth = |path: &'static str, email: String, password: &'static str| {
        let http = http.clone();
        async move {
            let body = json!({"email": email, "password": password}).to_string();
            let response = http.oneshot(request("POST", path, &[("origin", ORIGIN), ("content-type", "application/json")]).body(Body::from(body)).unwrap()).await.unwrap();
            let status = response.status().as_u16();
            (status, json_body(response).await["error"]["code"].as_str().unwrap_or("").to_string())
        }
    };
    const PASSWORD: &str = "synthetic-long-enough-password";
    let unique = || format!("shopper-{}@example.test", Uuid::new_v4().simple());

    let unknown = auth("/_brain/auth/login", unique(), PASSWORD).await;
    assert_eq!(unknown, (401, "LOGIN_FAILED".to_string()));
    assert_eq!(auth("/_brain/auth/login", "not-an-email".into(), PASSWORD).await, unknown, "a malformed email looks like any failed sign-in");
    assert_eq!(auth("/_brain/auth/signup", "not-an-email".into(), PASSWORD).await, (422, "SIGNUP_FAILED".to_string()));
    assert_eq!(auth("/_brain/auth/signup", unique(), "short").await, (422, "SIGNUP_FAILED".to_string()));
    let taken = unique();
    assert_eq!(auth("/_brain/auth/signup", taken.clone(), PASSWORD).await.0, 200);
    assert_eq!(auth("/_brain/auth/signup", taken, PASSWORD).await, (422, "SIGNUP_FAILED".to_string()), "a taken email looks like any refused signup");

    let held = (pool.acquire().await.unwrap(), pool.acquire().await.unwrap());
    for path in ["/_brain/auth/signup", "/_brain/auth/login"] {
        let (status, code) = auth(path, unique(), PASSWORD).await;
        assert_eq!(status, 503, "{path} on a busy database answered {code}");
        assert!(code != "SIGNUP_FAILED" && code != "LOGIN_FAILED", "{path}: {code}");
    }
    drop(held);
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn a_production_list_refusal_names_the_parameter_without_the_declared_catalog() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    let resource: Resource = serde_json::from_value(json!({
        "name": "probes",
        "fields": {"title": {"kind": "str", "required": true}, "secret_rank": {"kind": "int", "required": true}},
        "policy": {"read": ["public"], "create": ["admin"], "update": ["admin"], "delete": ["admin"]},
        "query": {"filter": {"secret_rank": ["eq"]}, "sort": ["secret_rank"]}
    })).unwrap();
    brain_backend::db::migrate(&pool, std::slice::from_ref(&resource)).await.unwrap();
    let (production, _) = brain_backend::http::router_with_platform(pool.clone(), config(vec![resource.clone()]), Platform::default()).await.map_err(|e| e.to_string()).unwrap();
    for path in ["/api/probes?nope=1", "/api/probes?sort=title", "/api/probes?secret_rank.gt=1"] {
        let response = send(&production, "GET", path, &[]).await;
        assert_eq!(response.status(), 422, "{path}");
        let body = json_body(response).await.to_string();
        assert!(!body.contains("declared:") && !body.contains("filterable:"), "{path}: {body}");
    }
    let mut development = config(vec![resource]);
    development.production = false;
    let (development, _) = brain_backend::http::router_with_platform(pool, development, Platform::default()).await.map_err(|e| e.to_string()).unwrap();
    let body = json_body(send(&development, "GET", "/api/probes?nope=1", &[]).await).await.to_string();
    assert!(body.contains("filterable: secret_rank"), "development keeps the fix: {body}");
}

// A server that stops admitting this role (pg_hba edit, role change, failover) is an operator's problem that clears: clients
// get a 503 to come back to, and the log keeps the SQLSTATE. A password that changed is the same case.
#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn a_database_that_refuses_the_connection_answers_503_with_retry_after() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let setup = brain_backend::db::connect(&url).await.unwrap();
    let suffix = Uuid::new_v4().simple().to_string()[..8].to_string();
    let role = format!("refused_{suffix}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE ROLE {role} LOGIN SUPERUSER PASSWORD 'synthetic'"))).execute(&setup).await.unwrap();
    let notes: Resource = serde_json::from_value(json!({
        "name": format!("refused_{suffix}"),
        "fields": {"title": {"kind": "str", "required": true}},
        "policy": {"read": ["public"], "create": ["user"], "update": ["owner"], "delete": ["owner"]}
    }))
    .unwrap();
    brain_backend::db::migrate(&setup, std::slice::from_ref(&notes)).await.unwrap();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new().max_connections(4).acquire_timeout(Duration::from_secs(3)).connect_with(options.username(&role).password("synthetic")).await.unwrap();
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(vec![notes.clone()]), Platform::default()).await.map_err(|e| e.to_string()).unwrap();
    assert_eq!(send(&http, "GET", &format!("/api/{}", notes.name), &[]).await.status(), 200);
    let kept = std::env::temp_dir().join(format!("brain-refused-{suffix}.jsonl"));
    brain_backend::db::keep_diagnostics(kept.clone());
    // the running connection goes away and a new one is refused with 28000
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER ROLE {role} NOLOGIN; SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = '{role}'"))).execute(&setup).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let response = send(&http, "GET", &format!("/api/{}", notes.name), &[]).await;
    let (status, retry) = (response.status(), response.headers().get("retry-after").cloned());
    let body = json_body(response).await;
    pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP ROLE {role}"))).execute(&setup).await.unwrap();
    setup.close().await;
    assert_eq!((status.as_u16(), body["error"]["code"].as_str()), (503, Some("DATABASE_UNAVAILABLE")), "{body}");
    assert_eq!(retry.unwrap(), "5");
    assert!(body["error"]["fix"].as_str().unwrap().contains("Retry-After"), "{body}");
    let events = std::fs::read_to_string(&kept).unwrap();
    std::fs::remove_file(kept).unwrap();
    assert!(events.lines().any(|l| l.contains("\"sqlstate\":\"28000\"") && l.contains("refused the connection")), "the cause stays in the log: {events}");
}

// The default limit has to carry a crawler from one address; a page spends one token per backend read.
#[test]
fn the_default_rate_limit_carries_a_crawler_at_two_reads_per_page() {
    let edge = Platform::default().edge;
    assert!(edge.requests_per_second >= 10 && edge.burst >= 300, "{} / {}", edge.requests_per_second, edge.burst);
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn head_answers_the_validator_of_get_and_honours_if_none_match() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    let suffix = Uuid::new_v4().simple().to_string()[..8].to_string();
    let notes: Resource = serde_json::from_value(json!({
        "name": format!("head_{suffix}"),
        "fields": {"title": {"kind": "str", "required": true}},
        "policy": {"read": ["public"], "create": ["user"], "update": ["owner"], "delete": ["owner"]}
    }))
    .unwrap();
    brain_backend::db::migrate(&pool, std::slice::from_ref(&notes)).await.unwrap();
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(vec![notes.clone()]), Platform::default()).await.map_err(|e| e.to_string()).unwrap();
    let path = format!("/api/{}", notes.name);
    let get = send(&http, "GET", &path, &[]).await;
    let etag = get.headers()["etag"].to_str().unwrap().to_string();
    let head = send(&http, "HEAD", &path, &[]).await;
    assert_eq!(head.status(), 200);
    assert_eq!(head.headers().get("etag").map(|v| v.to_str().unwrap()), Some(etag.as_str()));
    assert!(to_bytes(head.into_body(), 64).await.unwrap().is_empty());
    let unchanged = send(&http, "HEAD", &path, &[("if-none-match", &etag)]).await;
    assert_eq!(unchanged.status(), 304);
    let missing = send(&http, "HEAD", "/api/no_such_resource", &[]).await;
    assert_eq!(missing.status(), 404);
    assert!(to_bytes(missing.into_body(), 64).await.unwrap().is_empty());
    pool.close().await;
}
