//! Application-declared routes over real HTTP routing: access, signatures, typed input, limits.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, Response},
};
use brain_backend::{
    auth::{Auth, Credentials},
    platform::Platform,
    routes::{Route, RouteFailure, RouteHandlers},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "https://shop.example.test";
const SECRET: &str = "whsec_synthetic_route_secret_01";
const INTERNAL: &str = "synthetic-internal-route-secret-32-bytes";

/// Echoes its context so the test can see exactly what a handler receives.
struct Echo;
impl RouteHandlers for Echo {
    fn names(&self) -> Vec<String> {
        ["payment_events", "order_quote", "admin_report", "slow", "internal_open", "internal_local"].map(String::from).to_vec()
    }
    fn run(&self, name: &str, ctx: Value, _: Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, RouteFailure>> + Send + '_>> {
        let name = name.to_string();
        Box::pin(async move {
            match name.as_str() {
                "slow" => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    Ok(json!(null))
                }
                "order_quote" if ctx["body"]["items"] == 0 => Err(RouteFailure::Status(422, json!({"error": {"code": "INVALID_INPUT", "fields": {"items": "add at least one item"}}}))),
                "order_quote" => Ok(json!({"status": 201, "body": {"ctx": ctx}, "headers": {"location": "/orders/quotes/1"}})),
                _ => Ok(json!({"ctx": ctx})),
            }
        })
    }
}

fn routes() -> Vec<Route> {
    let env = |name: &str| match name {
        "PAYMENTS_WEBHOOK_SECRET" => Some(SECRET.to_string()),
        "INTERNAL_SECRET" => Some(INTERNAL.to_string()),
        _ => None,
    };
    [
        ("payment_events", json!({"method": "POST", "path": "/payments/events", "auth": "public", "body": "raw", "bodyLimit": "2kb", "rate": "3/min",
            "verify": {"scheme": "timestamped", "header": "x-payment-signature", "secret": "PAYMENTS_WEBHOOK_SECRET", "tolerance": "5m"}})),
        ("order_quote", json!({"method": "POST", "path": "/orders/{id}/quote", "params": {"id": "uuid"}, "query": {"currency": "str!", "rush": "bool"}, "headers": ["x-shop-channel"]})),
        ("admin_report", json!({"method": "GET", "path": "/reports/daily", "auth": ["admin"]})),
        ("slow", json!({"method": "GET", "path": "/reports/slow", "auth": "public", "timeout": "200ms"})),
        ("internal_open", json!({"method": "POST", "path": "/ops/open", "auth": "internal", "internal": {"secret": "INTERNAL_SECRET"}})),
        ("internal_local", json!({"method": "POST", "path": "/ops/local", "auth": "internal", "internal": {"secret": "INTERNAL_SECRET", "loopback": true}})),
    ]
    .iter()
    .map(|(name, contract)| Route::from_contract(name, contract, &env).unwrap())
    .collect()
}

fn sign(body: &[u8], t: u64) -> String {
    let mut message = format!("{t}.").into_bytes();
    message.extend_from_slice(body);
    let tag = ring::hmac::sign(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, SECRET.as_bytes()), &message);
    format!("t={t},v1={}", tag.as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>())
}
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}
async fn send(http: &Router, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Response<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("accept", "application/json")
        .extension(ConnectInfo("127.0.0.1:42003".parse::<std::net::SocketAddr>().unwrap()));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    http.clone().oneshot(req.body(Body::from(body.to_vec())).unwrap()).await.unwrap()
}
async fn json_body(response: Response<Body>) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap()
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn declared_routes_verify_signatures_type_input_and_shape_responses() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let email = format!("routes-{}@example.test", Uuid::new_v4().simple());
    let password = "synthetic-routes-account-password";
    let user = auth.signup(Credentials { email: email.clone(), password: password.into() }).await.unwrap();
    let (_, session) = auth.login(Credentials { email, password: password.into() }).await.unwrap().unwrap();
    let cookie = format!("__Host-brain={session}");
    let (http, _) = brain_backend::http::router_with_platform(
        pool.clone(),
        brain_backend::Config {
        api_keys: None,
            roles: Default::default(),
            mail: None,
            auth_public_base_url: None,
            webhooks: None,
            before_hooks: None,
            sockets: None,
            resources: vec![],
            jobs: vec![],
            uploads: std::env::temp_dir().join(format!("brain-routes-{}", Uuid::new_v4())),
            origins: vec![ORIGIN.into()],
            production: true,
            jwt: None,
            google: None,
            gcs: None,
        },
        Platform { routes: routes(), handlers: Some(Arc::new(Echo)), ..Default::default() },
    )
    .await
    .unwrap();

    // Internal routes: a secret instead of a browser Origin or a session; loopback refuses anything forwarded.
    let secret = ("x-internal-secret", INTERNAL);
    let typed = ("content-type", "application/json");
    assert_eq!(send(&http, "POST", "/ops/open", &[typed, secret], b"{}").await.status(), 200);
    assert_eq!(send(&http, "POST", "/ops/open", &[typed, ("x-internal-secret", "wrong")], b"{}").await.status(), 403);
    assert_eq!(send(&http, "POST", "/ops/open", &[typed, ("cookie", cookie.as_str())], b"{}").await.status(), 403, "a session is no secret");
    assert_eq!(send(&http, "POST", "/ops/local", &[typed, secret], b"{}").await.status(), 200);
    assert_eq!(send(&http, "POST", "/ops/local", &[typed, secret, ("x-forwarded-for", "192.0.2.9")], b"{}").await.status(), 403);
    let remote = Request::builder().method("POST").uri("/ops/local").header("content-type", "application/json").header("x-internal-secret", INTERNAL).extension(ConnectInfo("192.0.2.9:42003".parse::<std::net::SocketAddr>().unwrap())).body(Body::from("{}")).unwrap();
    assert_eq!(http.clone().oneshot(remote).await.unwrap().status(), 403);

    // Signed webhook: no browser Origin needed, exact raw bytes, timestamp tolerance.
    let event = br#"{"id":"evt_1","type":"payment.succeeded","amount":4200}"#;
    let t = now();
    let ok = send(&http, "POST", "/payments/events", &[("content-type", "application/json"), ("x-payment-signature", &sign(event, t))], event).await;
    assert_eq!(ok.status(), 200);
    let ctx = json_body(ok).await["ctx"].clone();
    assert_eq!(ctx["rawBody"], std::str::from_utf8(event).unwrap());
    assert_eq!(ctx["rawBodyEncoding"], "utf8");
    assert_eq!(ctx["body"]["amount"], 4200);
    assert_eq!(ctx["verified"]["timestamp"], t);
    assert_eq!(ctx["user"], Value::Null);
    let tampered = br#"{"id":"evt_1","type":"payment.succeeded","amount":1}"#;
    let bad = send(&http, "POST", "/payments/events", &[("x-payment-signature", &sign(event, t))], tampered).await;
    assert_eq!(bad.status(), 401);
    assert_eq!(json_body(bad).await["error"]["code"], "SIGNATURE_INVALID");
    let old = send(&http, "POST", "/payments/events", &[("x-payment-signature", &sign(event, t - 600))], event).await;
    assert_eq!(json_body(old).await["error"]["code"], "SIGNATURE_EXPIRED");
    // Per-IP rate limit: three calls per minute, counted before the signature check.
    let limited = send(&http, "POST", "/payments/events", &[("x-payment-signature", &sign(event, t))], event).await;
    assert_eq!(limited.status(), 429);
    assert!(limited.headers().contains_key("retry-after"));

    // Signed-in JSON route with typed path, strict query and forwarded headers.
    let order = Uuid::new_v4().to_string();
    let path = format!("/orders/{order}/quote?currency=USD&rush=true");
    let json_headers = [("origin", ORIGIN), ("content-type", "application/json"), ("cookie", cookie.as_str()), ("x-shop-channel", "web"), ("x-other", "dropped")];
    let anonymous = send(&http, "POST", &path, &[("origin", ORIGIN), ("content-type", "application/json")], br#"{"items":2}"#).await;
    assert_eq!(anonymous.status(), 401);
    let created = send(&http, "POST", &path, &json_headers, br#"{"items":2}"#).await;
    assert_eq!(created.status(), 201);
    assert_eq!(created.headers()["location"], "/orders/quotes/1");
    let ctx = json_body(created).await["ctx"].clone();
    assert_eq!(ctx["params"]["id"], order);
    assert_eq!(ctx["query"], json!({"currency": "USD", "rush": true}));
    assert_eq!(ctx["headers"], json!({"x-shop-channel": "web"}));
    assert_eq!(ctx["user"]["id"], user.id.to_string());
    assert_eq!(ctx["rawBody"], Value::Null);
    let rejected = send(&http, "POST", &path, &json_headers, br#"{"items":0}"#).await;
    assert_eq!(rejected.status(), 422);
    assert_eq!(json_body(rejected).await["error"]["fields"]["items"], "add at least one item");
    let query = send(&http, "POST", &format!("/orders/{order}/quote?rush=maybe&debug=1"), &json_headers, br#"{"items":2}"#).await;
    assert_eq!(query.status(), 422);
    let fields = json_body(query).await["error"]["fields"]["query"].clone();
    assert_eq!(fields["debug"], "unknown query parameter");
    assert_eq!(fields["currency"], "required");
    assert_eq!(fields["rush"], "expected bool");
    assert_eq!(send(&http, "POST", "/orders/not-a-uuid/quote?currency=USD", &json_headers, b"{}").await.status(), 404);
    let polluted = send(&http, "POST", &path, &json_headers, br#"{"items":2,"meta":{"__proto__":{"admin":true}}}"#).await;
    assert_eq!(polluted.status(), 400);
    let text = send(&http, "POST", &path, &[("origin", ORIGIN), ("content-type", "text/plain"), ("cookie", cookie.as_str())], b"items=2").await;
    assert_eq!(text.status(), 415);
    let no_origin = send(&http, "POST", &path, &[("content-type", "application/json"), ("cookie", cookie.as_str())], br#"{"items":2}"#).await;
    assert_eq!(no_origin.status(), 403, "unsigned routes keep the browser Origin check");
    let big = vec![b' '; 70 * 1024];
    assert_eq!(send(&http, "POST", &path, &json_headers, &big).await.status(), 413);

    // Role route and handler deadline.
    let report = send(&http, "GET", "/reports/daily", &[("cookie", cookie.as_str())], b"").await;
    assert_eq!(report.status(), 403);
    let slow = send(&http, "GET", "/reports/slow", &[], b"").await;
    assert_eq!(slow.status(), 504);
    assert_eq!(json_body(slow).await["error"]["code"], "ROUTE_TIMEOUT");
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn routes_without_compiled_handlers_fail_before_serving() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    let config = || brain_backend::Config {
        api_keys: None,
        roles: Default::default(),
        mail: None,
        auth_public_base_url: None,
        webhooks: None,
        before_hooks: None,
        sockets: None,
        resources: vec![],
        jobs: vec![],
        uploads: std::env::temp_dir().join("brain-routes-missing"),
        origins: vec![ORIGIN.into()],
        production: true,
        jwt: None,
        google: None,
        gcs: None,
    };
    let err = brain_backend::http::router_with_platform(pool.clone(), config(), Platform { routes: routes(), handlers: None, ..Default::default() })
        .await
        .err()
        .unwrap();
    assert!(err.contains("handler.jsbrain"), "{err}");
}

#[test]
fn internal_routes_compare_a_rotatable_secret_and_declare_it_by_name() {
    let (current, old) = ("synthetic-internal-secret-current-32b", "synthetic-internal-secret-previous-3");
    let env = |name: &str| match name {
        "INTERNAL_SECRET" => Some(current.to_string()),
        "INTERNAL_SECRET_OLD" => Some(old.to_string()),
        "INTERNAL_SHORT" => Some("short".to_string()),
        _ => None,
    };
    let route = Route::from_contract("ops", &json!({"method": "POST", "path": "/ops/run", "auth": "internal", "internal": {"secret": ["INTERNAL_SECRET", "INTERNAL_SECRET_OLD"], "loopback": true}}), &env).unwrap();
    let brain_backend::routes::Access::Internal(internal) = &route.access else { panic!("{route:?}") };
    assert!(internal.accepts(current) && internal.accepts(old) && !internal.accepts("wrong") && !internal.accepts(""));
    assert!(internal.loopback);
    assert_eq!(internal.header, "x-internal-secret");
    assert_eq!(route.schema()["auth"], "internal");
    assert!(!format!("{route:?}").contains(current), "the secret never reaches Debug");
    for bad in [
        json!({"path": "/a", "auth": "internal"}),
        json!({"path": "/a", "auth": "internal", "internal": {"secret": "INTERNAL_SHORT"}}),
        json!({"path": "/a", "auth": "internal", "internal": {"secret": "MISSING"}}),
        json!({"path": "/a", "auth": "internal", "internal": {"secret": "literal-secret-value"}}),
        json!({"path": "/a", "auth": "internal", "internal": {"secret": "INTERNAL_SECRET", "header": "cookie"}}),
        json!({"path": "/a", "auth": "public", "internal": {"secret": "INTERNAL_SECRET"}}),
        json!({"path": "/a", "auth": "internal", "internal": {"secret": "INTERNAL_SECRET"}, "rate": "1/min per user"}),
    ] {
        assert!(Route::from_contract("ops", &bad, &env).is_err(), "{bad}");
    }
}
