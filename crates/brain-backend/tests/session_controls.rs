//! Disposable Postgres only: the signup grant, the operator key, and what an account can do with its own sessions.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use brain_backend::{captcha::Captcha, db, platform::Platform};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::str::FromStr;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://127.0.0.1:3000";
const PASSWORD: &str = "synthetic-password-1234";
const OPERATOR: &str = "synthetic-operator-key-0123456789abcdef";

async fn database() -> sqlx::PgPool {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use the disposable Postgres harness; no developer database is selected");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_sessions_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name);
    let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
    base.close().await;
    db::migrate(&pool, &[]).await.unwrap();
    pool
}
fn roles() -> brain_backend::auth_flows::RolesConfig {
    brain_backend::auth_flows::RolesConfig {
        roles: ["admin", "user"].map(String::from).to_vec(),
        signup_role: "user".into(),
        session_seconds: [("admin".to_string(), 900u32)].into_iter().collect(),
    }
}
fn config() -> brain_backend::Config {
    brain_backend::Config {
        vitals: None,
        api_keys: Some(Default::default()),
        roles: roles(),
        mail: None,
        auth_public_base_url: None,
        webhooks: None,
        before_hooks: None,
        sockets: None,
        gcs: None,
        jwt: None,
        google: None, apple: None, apple_native: None, linkedin: None,
        resources: Vec::new(),
        jobs: Vec::new(),
        uploads: std::env::temp_dir().join(format!("brain-sessions-{}", Uuid::new_v4().simple())),
        origins: vec![ORIGIN.into()],
        production: false,
    }
}
async fn router(pool: &sqlx::PgPool, platform: Platform) -> Router {
    roles().register(pool).await.unwrap();
    brain_backend::http::router_with_platform(pool.clone(), config(), platform).await.unwrap().0
}
/// Counts what reaches the provider, so a test can prove a path never asked it.
async fn provider() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use axum::{body::Bytes, routing::post};
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = asked.clone();
    let app = Router::new().route(
        "/verify",
        post(move |body: Bytes| {
            let count = count.clone();
            async move {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let good = String::from_utf8_lossy(&body).contains("response=good");
                format!(r#"{{"success":{good}}}"#)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}/verify"), asked)
}
struct Call<'a> {
    method: &'a str,
    path: &'a str,
    headers: &'a [(&'a str, &'a str)],
    body: Option<Value>,
}
async fn send(http: &Router, call: Call<'_>) -> (StatusCode, Value, Option<String>) {
    let mut request = Request::builder().method(call.method).uri(call.path).header("origin", ORIGIN).header("accept", "application/json").extension(ConnectInfo("127.0.0.1:42031".parse::<std::net::SocketAddr>().unwrap()));
    for (name, value) in call.headers {
        request = request.header(*name, *value);
    }
    let request = match call.body {
        Some(body) => request.header("content-type", "application/json").body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    };
    let answer = http.clone().oneshot(request.unwrap()).await.unwrap();
    let status = answer.status();
    let cookie = answer.headers().get("set-cookie").and_then(|v| v.to_str().ok()).map(str::to_string);
    let bytes = to_bytes(answer.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), cookie)
}
async fn post(http: &Router, path: &str, headers: &[(&str, &str)], body: Value) -> (StatusCode, Value, Option<String>) {
    send(http, Call { method: "POST", path, headers, body: Some(body) }).await
}
fn code(answer: &(StatusCode, Value, Option<String>)) -> (u16, &str) {
    (answer.0.as_u16(), answer.1["error"]["code"].as_str().unwrap_or(""))
}
fn person(email: &str) -> Value {
    json!({"email":email,"password":PASSWORD})
}
fn guarded(url: &str, operator: bool) -> Captcha {
    let mut contract = json!({"verify_url":url,"secret_env":"S","protect":["signup","login"]});
    if operator {
        contract["operator_key_env"] = json!("OPERATOR_KEY");
    }
    Captcha::from_contract(&contract, |name| match name {
        "S" => Some("synthetic-captcha-secret".to_string()),
        "OPERATOR_KEY" => Some(OPERATOR.to_string()),
        _ => None,
    })
    .unwrap()
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn a_signup_grant_spends_once_for_its_own_account() {
    let pool = database().await;
    let (url, asked) = provider().await;
    let http = router(&pool, Platform { captcha: guarded(&url, false), ..Default::default() }).await;
    let token = [("x-brain-captcha", "good")];
    let (ana, bea) = ("ana@example.test", "bea@example.test");
    let signed = post(&http, "/_brain/auth/signup", &token, person(ana)).await;
    assert_eq!(signed.0, 200, "{}", signed.1);
    let grant = signed.1["login_grant"].as_str().expect("a verified signup returns its grant").to_string();
    assert_eq!(post(&http, "/_brain/auth/signup", &token, person(bea)).await.0, 200);
    let before = asked.load(std::sync::atomic::Ordering::SeqCst);

    // No token and no grant is still refused: the grant is the only thing that replaces the challenge.
    assert_eq!(code(&post(&http, "/_brain/auth/login", &[], person(ana)).await), (403, "CAPTCHA_FAILED"));
    // A made-up grant and another account's address both fail, and neither falls through to the provider.
    assert_eq!(code(&post(&http, "/_brain/auth/login", &[("x-brain-login-grant", "not-a-grant")], person(ana)).await), (403, "CAPTCHA_FAILED"));
    let mine = [("x-brain-login-grant", grant.as_str())];
    assert_eq!(code(&post(&http, "/_brain/auth/login", &mine, person(bea)).await), (403, "CAPTCHA_FAILED"));
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), before, "a grant must not reach the provider");
    // The grant does not replace the password, and a wrong password spends it.
    let wrong = post(&http, "/_brain/auth/login", &mine, json!({"email":ana,"password":"synthetic-wrong-password"})).await;
    assert_eq!(wrong.0, 401, "{}", wrong.1);
    assert_eq!(code(&post(&http, "/_brain/auth/login", &mine, person(ana)).await), (403, "CAPTCHA_FAILED"));

    // A fresh grant logs in once; replaying it is refused.
    let again = post(&http, "/_brain/auth/signup", &token, person("cal@example.test")).await;
    let grant = again.1["login_grant"].as_str().unwrap().to_string();
    let mine = [("x-brain-login-grant", grant.as_str())];
    assert_eq!(post(&http, "/_brain/auth/login", &mine, person("cal@example.test")).await.0, 200);
    assert_eq!(code(&post(&http, "/_brain/auth/login", &mine, person("cal@example.test")).await), (403, "CAPTCHA_FAILED"));

    // An expired grant is refused.
    let late = post(&http, "/_brain/auth/signup", &token, person("dee@example.test")).await;
    let grant = late.1["login_grant"].as_str().unwrap().to_string();
    sqlx::query("UPDATE _brain_auth_challenges SET expires_at=now()-interval '1 second' WHERE purpose='signup_login'").execute(&pool).await.unwrap();
    assert_eq!(code(&post(&http, "/_brain/auth/login", &[("x-brain-login-grant", grant.as_str())], person("dee@example.test")).await), (403, "CAPTCHA_FAILED"));
    // Only a digest is stored.
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_auth_challenges WHERE token_hash=$1").bind(&grant).fetch_one(&pool).await.unwrap();
    assert_eq!(stored, 0);

    // With the provider down a signup fails closed and leaves no account and no grant.
    let dead = Captcha::from_contract(&json!({"verify_url":"http://127.0.0.1:9/x","secret_env":"S","protect":["signup","login"]}), |_| Some("synthetic-captcha-secret".into())).unwrap();
    let http = router(&pool, Platform { captcha: dead, ..Default::default() }).await;
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_users").fetch_one(&pool).await.unwrap();
    assert_eq!(code(&post(&http, "/_brain/auth/signup", &token, person("eve@example.test")).await), (503, "CAPTCHA_UNAVAILABLE"));
    assert_eq!(code(&post(&http, "/_brain/auth/login", &token, person(ana)).await), (503, "CAPTCHA_UNAVAILABLE"));
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _brain_users").fetch_one(&pool).await.unwrap(), users);

    // Login not protected: no grant is issued, since nothing would spend it.
    let only_signup = Captcha::from_contract(&json!({"verify_url":url,"secret_env":"S","protect":["signup"]}), |_| Some("synthetic-captcha-secret".into())).unwrap();
    let http = router(&pool, Platform { captcha: only_signup, ..Default::default() }).await;
    let signed = post(&http, "/_brain/auth/signup", &token, person("fay@example.test")).await;
    assert_eq!(signed.0, 200);
    assert!(signed.1.get("login_grant").is_none(), "{}", signed.1);
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn the_operator_key_admits_only_batch_signups_and_only_when_declared() {
    let pool = database().await;
    let (url, asked) = provider().await;
    let http = router(&pool, Platform { captcha: guarded(&url, true), ..Default::default() }).await;
    let users = async || sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _brain_users").fetch_one(&pool).await.unwrap();
    let before = users().await;
    let wrong = [("x-brain-operator-key", "synthetic-wrong-key")];
    assert_eq!(code(&post(&http, "/_brain/auth/signup", &wrong, person("one@example.test")).await), (403, "CAPTCHA_FAILED"));
    assert_eq!(users().await, before);
    // A wrong key is not a second chance through a valid token sent alongside.
    let both = [("x-brain-operator-key", "synthetic-wrong-key"), ("x-brain-captcha", "good")];
    assert_eq!(code(&post(&http, "/_brain/auth/signup", &both, person("one@example.test")).await), (403, "CAPTCHA_FAILED"));
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0);

    let right = [("x-brain-operator-key", OPERATOR)];
    let signed = post(&http, "/_brain/auth/signup", &right, person("one@example.test")).await;
    assert_eq!(signed.0, 200, "{}", signed.1);
    assert!(signed.1.get("login_grant").is_none(), "an operator's signup earns no login: {}", signed.1);
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0);
    let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_auth_events WHERE event='operator-signup'").fetch_one(&pool).await.unwrap();
    assert_eq!(audited, 1);
    // The key opens signup only: login still needs a challenge.
    assert_eq!(code(&post(&http, "/_brain/auth/login", &right, person("one@example.test")).await), (403, "CAPTCHA_FAILED"));
    // Browsers cannot send it across origins.
    let preflight = http.clone().oneshot(Request::builder().method("OPTIONS").uri("/_brain/auth/signup").header("origin", ORIGIN).header("access-control-request-method", "POST").body(Body::empty()).unwrap()).await.unwrap();
    let allowed = preflight.headers().get("access-control-allow-headers").and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    assert!(allowed.contains("x-brain-login-grant") && !allowed.contains("x-brain-operator-key"), "{allowed}");

    // Not declared, or declared and unset: no key opens anything.
    let (http_undeclared, http_unset) = (
        router(&pool, Platform { captcha: guarded(&url, false), ..Default::default() }).await,
        router(&pool, Platform { captcha: Captcha::from_contract(&json!({"verify_url":url,"secret_env":"S","protect":["signup"],"operator_key_env":"OPERATOR_KEY"}), |n| (n == "S").then(|| "synthetic-captcha-secret".to_string())).unwrap(), ..Default::default() }).await,
    );
    for http in [&http_undeclared, &http_unset] {
        assert_eq!(code(&post(http, "/_brain/auth/signup", &right, person("two@example.test")).await), (403, "CAPTCHA_FAILED"));
        assert_eq!(code(&post(http, "/_brain/auth/signup", &[("x-brain-operator-key", "")], person("two@example.test")).await), (403, "CAPTCHA_FAILED"));
    }

}

/// Two signed-in sessions of one account and one of another.
async fn signed_in(http: &Router, email: &str) -> String {
    let answer = post(http, "/_brain/auth/login", &[], person(email)).await;
    assert_eq!(answer.0, 200, "{}", answer.1);
    answer.2.expect("login sets the session cookie").split(';').next().unwrap().to_string()
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn an_account_manages_only_its_own_sessions() {
    let pool = database().await;
    let http = router(&pool, Platform::default()).await;
    assert_eq!(post(&http, "/_brain/auth/signup", &[], person("ana@example.test")).await.0, 200);
    assert_eq!(post(&http, "/_brain/auth/signup", &[], person("bea@example.test")).await.0, 200);
    let (ana_one, ana_two, ana_three, bea) = (signed_in(&http, "ana@example.test").await, signed_in(&http, "ana@example.test").await, signed_in(&http, "ana@example.test").await, signed_in(&http, "bea@example.test").await);
    let list = async |cookie: &str| send(&http, Call { method: "GET", path: "/_brain/auth/sessions", headers: &[("cookie", cookie)], body: None }).await;
    assert_eq!(list("").await.0, 401);
    let mine = list(&ana_one).await;
    assert_eq!(mine.0, 200, "{}", mine.1);
    let sessions = mine.1["sessions"].as_array().unwrap().clone();
    assert_eq!(sessions.len(), 3, "{sessions:?}");
    assert_eq!(sessions.iter().filter(|s| s["current"] == true).count(), 1);
    assert!(sessions.iter().all(|s| s["auth_time"].is_string() && s["last_used_at"].is_string() && s.get("token_hash").is_none()));
    let current = sessions.iter().find(|s| s["current"] == true).unwrap()["id"].as_str().unwrap().to_string();
    let other = sessions.iter().find(|s| s["current"] == false).unwrap()["id"].as_str().unwrap().to_string();
    let beas = list(&bea).await.1["sessions"][0]["id"].as_str().unwrap().to_string();

    // Another account's id answers exactly like an id that does not exist, and ends nothing.
    let foreign = post(&http, &format!("/_brain/auth/sessions/{beas}/revoke"), &[("cookie", &ana_one)], json!({})).await;
    let unknown = post(&http, &format!("/_brain/auth/sessions/{}/revoke", Uuid::new_v4()), &[("cookie", &ana_one)], json!({})).await;
    assert_eq!((foreign.0, &foreign.1["error"]), (unknown.0, &unknown.1["error"]));
    assert_eq!(foreign.0, 404);
    assert_eq!(list(&bea).await.0, 200);
    // No session, no revoke; and the current one ends through logout.
    assert_eq!(post(&http, &format!("/_brain/auth/sessions/{other}/revoke"), &[], json!({})).await.0, 401);
    assert_eq!(code(&post(&http, &format!("/_brain/auth/sessions/{current}/revoke"), &[("cookie", &ana_one)], json!({})).await), (409, "CURRENT_SESSION"));

    let done = post(&http, &format!("/_brain/auth/sessions/{other}/revoke"), &[("cookie", &ana_one)], json!({})).await;
    assert_eq!(done.0, 200, "{}", done.1);
    assert_eq!(post(&http, &format!("/_brain/auth/sessions/{other}/revoke"), &[("cookie", &ana_one)], json!({})).await.0, 404, "an ended session is gone");
    assert_eq!(list(&ana_one).await.1["sessions"].as_array().unwrap().len(), 2);

    // Ending the others keeps this one and leaves the other account alone.
    let others = post(&http, "/_brain/auth/sessions/revoke-others", &[("cookie", &ana_one)], json!({})).await;
    assert_eq!((others.0.as_u16(), others.1["revoked"].as_u64()), (200, Some(1)), "{}", others.1);
    assert_eq!(list(&ana_one).await.0, 200);
    assert_eq!(list(&ana_two).await.0.as_u16() + list(&ana_three).await.0.as_u16(), 802, "both other sessions are dead");
    assert_eq!(list(&bea).await.0, 200);
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn a_role_sets_session_length_and_a_route_can_demand_a_recent_password() {
    let pool = database().await;
    struct Ran;
    impl brain_backend::routes::RouteHandlers for Ran {
        fn names(&self) -> Vec<String> {
            vec!["sensitive".into()]
        }
        fn run(&self, _: &str, _: Value, _: std::time::Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, brain_backend::routes::RouteFailure>> + Send + '_>> {
            Box::pin(async { Ok(json!({"ok":true})) })
        }
    }
    let route = brain_backend::routes::Route::from_contract("sensitive", &json!({"method":"POST","path":"/sensitive","auth":"user","reauth":300}), &|_| None).unwrap();
    let http = router(&pool, Platform { routes: vec![route], handlers: Some(std::sync::Arc::new(Ran)), ..Default::default() }).await;
    assert_eq!(post(&http, "/_brain/auth/signup", &[], person("ana@example.test")).await.0, 200);
    let cookie = signed_in(&http, "ana@example.test").await;
    let headers = [("cookie", cookie.as_str())];

    // A fresh login is recent; an aged proof is refused until the password is confirmed again.
    assert_eq!(post(&http, "/sensitive", &headers, json!({})).await.0, 200);
    sqlx::query("UPDATE _brain_sessions SET auth_time=now()-interval '1 hour'").execute(&pool).await.unwrap();
    assert_eq!(code(&post(&http, "/sensitive", &headers, json!({})).await), (401, "REAUTH_REQUIRED"));
    assert_eq!(post(&http, "/sensitive", &[], json!({})).await.0, 401);
    assert_eq!(post(&http, "/_brain/auth/reauth", &[], json!({"password":PASSWORD})).await.0, 401);
    assert_eq!(post(&http, "/_brain/auth/reauth", &headers, json!({"password":"synthetic-wrong-password"})).await.0, 401);
    assert_eq!(code(&post(&http, "/sensitive", &headers, json!({})).await), (401, "REAUTH_REQUIRED"));
    let proved = post(&http, "/_brain/auth/reauth", &headers, json!({"password":PASSWORD})).await;
    assert_eq!(proved.0, 200, "{}", proved.1);
    assert_eq!(post(&http, "/sensitive", &headers, json!({})).await.0, 200);
    // Wrong passwords hit the same limiter as login.
    let mut last = StatusCode::OK;
    for _ in 0..12 {
        last = post(&http, "/_brain/auth/reauth", &headers, json!({"password":"synthetic-wrong-password"})).await.0;
    }
    assert_eq!(last, 401);
    assert_eq!(post(&http, "/_brain/auth/reauth", &headers, json!({"password":PASSWORD})).await.0, 401, "the limiter blocks even the right password");
    // A contract cannot demand it where nobody is signed in, or outside the range.
    for bad in [json!({"method":"POST","path":"/x","auth":"public","reauth":300}), json!({"method":"POST","path":"/x","auth":"user","reauth":5}), json!({"method":"POST","path":"/x","auth":"user","reauth":"soon"})] {
        assert!(brain_backend::routes::Route::from_contract("x", &bad, &|_| None).unwrap_err().contains("reauth"), "{bad}");
    }

    // Length per role: an admin's new session is shorter than the default, in the cookie and in the row.
    sqlx::query("UPDATE _brain_users SET role='admin' WHERE email='ana@example.test'").execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM _brain_login_limits").execute(&pool).await.unwrap();
    let login = post(&http, "/_brain/auth/login", &[], person("ana@example.test")).await;
    assert!(login.2.as_deref().unwrap().contains("Max-Age=900"), "{:?}", login.2);
    let remaining: f64 = sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM max(expires_at)-now())::float8 FROM _brain_sessions WHERE created_at>now()-interval '5 seconds' AND expires_at<now()+interval '1 hour'").fetch_one(&pool).await.unwrap();
    assert!((800.0..=900.0).contains(&remaining), "{remaining}");
    // A role outside the list, or a length outside the range, is refused at start.
    let mut bad = roles();
    bad.session_seconds = [("ghost".to_string(), 900)].into_iter().collect();
    assert!(bad.validate().unwrap_err().contains("ghost"));
    let mut bad = roles();
    bad.session_seconds = [("admin".to_string(), 10)].into_iter().collect();
    assert!(bad.validate().unwrap_err().contains("300"));
}
