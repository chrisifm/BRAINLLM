use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use brain_backend::{
    auth::{Auth, Credentials},
    contract::{Field, Hooks, Policy, Resource},
    db,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "https://app.example.test";

fn resource(name: &str, policy: Policy) -> Resource {
    Resource {
        name: name.into(),
        search: vec!["title".into()],
        fields: BTreeMap::from([(
            "title".into(),
            Field {
                kind: "str".into(),
                required: true,
                ..Default::default()
            },
        )]),
        policy,
        hooks: Hooks::default(),
        constraints: Default::default(),
    }
}
fn config(resources: Vec<Resource>, api_keys: Option<brain_backend::api_keys::Config>) -> brain_backend::Config {
    brain_backend::Config {
        api_keys,
        resources,
        jobs: vec![],
        uploads: std::env::temp_dir().join(format!("brain-api-keys-{}", Uuid::new_v4().simple())),
        origins: vec![ORIGIN.into()],
        production: false,
        jwt: None,
        google: None,
        gcs: None,
        roles: Default::default(),
        mail: None,
        auth_public_base_url: None,
        webhooks: None,
        before_hooks: None,
        sockets: None,
    }
}
struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Value,
}
async fn call(router: &Router, request: Request<Body>) -> Reply {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}
fn session(method: &str, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", format!("__Host-brain={token}"))
        .header("origin", ORIGIN)
        .header("content-type", "application/json")
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from(([203, 0, 113, 7], 40000))))
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_default())
        .unwrap()
}
/// A server-to-server call: no cookie and no Origin header.
fn keyed(method: &str, uri: &str, key: &str, bearer: bool, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder()
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from(([203, 0, 113, 7], 40000))))
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    let builder = if bearer {
        builder.header("authorization", format!("Bearer {key}"))
    } else {
        builder.header("x-api-key", key)
    };
    builder
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_default())
        .unwrap()
}
async fn user(auth: &Auth) -> (Uuid, String) {
    let email = format!("keys-{}@example.test", Uuid::new_v4().simple());
    let password = "synthetic-api-key-password".to_string();
    let user = auth
        .signup(Credentials {
            email: email.clone(),
            password: password.clone(),
        })
        .await
        .unwrap();
    let (_, token) = auth
        .login(Credentials { email, password })
        .await
        .unwrap()
        .unwrap();
    (user.id, token)
}
async fn create_key(router: &Router, token: &str, body: Value) -> (String, String) {
    let reply = call(router, session("POST", "/_brain/keys", token, Some(body))).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let secret = reply.body["secret"].as_str().unwrap().to_string();
    assert!(secret.starts_with("bk_live_"));
    assert!(reply.body["key"].get("secretHash").is_none());
    (reply.body["key"]["id"].as_str().unwrap().to_string(), secret)
}

/// Run against a disposable Postgres database via scripts/backend-check.mjs. Never selects a developer database.
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn api_keys_authenticate_scope_quota_trace_rotate_and_revoke() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL")
        .expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let pool = db::connect(&url).await.unwrap();
    let notes = resource(
        "key_notes",
        Policy {
            read: vec!["candidate".into()],
            create: vec!["candidate".into()],
            update: vec!["owner".into()],
            delete: vec!["owner".into()],
        },
    );
    let ledgers = resource(
        "key_ledgers",
        Policy {
            read: vec!["admin".into()],
            create: vec!["admin".into()],
            update: vec!["admin".into()],
            delete: vec!["admin".into()],
        },
    );
    db::migrate(&pool, &[notes.clone(), ledgers.clone()]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let router = brain_backend::http::router(
        pool.clone(),
        config(vec![notes.clone(), ledgers.clone()], Some(Default::default())),
    )
    .await
    .unwrap();
    let (owner, token) = user(&auth).await;
    let (_, stranger) = user(&auth).await;

    // Provisioning validates scopes and returns the plaintext exactly once.
    let bad = call(&router, session("POST", "/_brain/keys", &token, Some(json!({"name":"bad","scopes":["key_notes:delete"]})))).await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    let (reader_id, reader) = create_key(&router, &token, json!({"name":"Reader","scopes":["key_notes:read"]})).await;
    let (writer_id, writer) = create_key(&router, &token, json!({"name":"Writer","scopes":["*:*"]})).await;

    // Use: X-API-Key and Bearer both work without Origin or cookies.
    let read = call(&router, keyed("GET", "/api/key_notes?search=private-value&limit=5", &reader, false, None)).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    let created = call(&router, keyed("POST", "/api/key_notes", &writer, true, Some(json!({"title":"From a key"})))).await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    assert_eq!(created.body["owner"].as_str(), Some(owner.to_string().as_str()));

    // Scope narrows; it never widens the owner's role.
    let scoped = call(&router, keyed("POST", "/api/key_notes", &reader, false, Some(json!({"title":"Denied"})))).await;
    assert_eq!(scoped.status, StatusCode::FORBIDDEN);
    assert_eq!(scoped.body["error"]["code"], "API_KEY_SCOPE_DENIED");
    let above_owner = call(&router, keyed("GET", "/api/key_ledgers", &writer, false, None)).await;
    assert_eq!(above_owner.status, StatusCode::FORBIDDEN);
    assert_eq!(above_owner.body["error"]["code"], "POLICY_DENIED");

    // Keys are refused on auth and key-management routes, and malformed keys are typed 401s.
    for uri in ["/_brain/auth/me", "/_brain/keys"] {
        let refused = call(&router, keyed("GET", uri, &writer, true, None)).await;
        assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
        assert_eq!(refused.body["error"]["code"], "API_KEY_NOT_ACCEPTED");
    }
    let forged = format!("{}{}", &writer[..writer.len() - 1], if writer.ends_with('0') { '1' } else { '0' });
    let invalid = call(&router, keyed("GET", "/api/key_notes", &forged, false, None)).await;
    assert_eq!(invalid.body["error"]["code"], "API_KEY_INVALID");
    assert_eq!(invalid.status, StatusCode::UNAUTHORIZED);

    // Listing never exposes secrets; strangers cannot see, rotate or revoke another user's key.
    let listed = call(&router, session("GET", "/_brain/keys", &token, None)).await;
    assert_eq!(listed.body["items"].as_array().unwrap().len(), 2);
    assert!(!listed.body.to_string().contains(&writer[20..]));
    let foreign = call(&router, session("DELETE", &format!("/_brain/keys/{writer_id}"), &stranger, None)).await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND);
    let all = call(&router, session("GET", "/_brain/keys?all=true", &stranger, None)).await;
    assert_eq!(all.status, StatusCode::FORBIDDEN);

    // Rotation revokes the old secret atomically and keeps scopes.
    let rotated = call(&router, session("POST", &format!("/_brain/keys/{reader_id}/rotate"), &token, None)).await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.body);
    let fresh = rotated.body["secret"].as_str().unwrap().to_string();
    assert_eq!(rotated.body["key"]["scopes"], json!(["key_notes:read"]));
    assert_eq!(call(&router, keyed("GET", "/api/key_notes", &reader, false, None)).await.body["error"]["code"], "API_KEY_REVOKED");
    assert_eq!(call(&router, keyed("GET", "/api/key_notes", &fresh, false, None)).await.status, StatusCode::OK);

    // Revocation and expiry.
    let revoked = call(&router, session("DELETE", &format!("/_brain/keys/{writer_id}"), &token, None)).await;
    assert_eq!(revoked.status, StatusCode::OK);
    assert_eq!(call(&router, keyed("GET", "/api/key_notes", &writer, true, None)).await.body["error"]["code"], "API_KEY_REVOKED");
    let fresh_id = rotated.body["key"]["id"].as_str().unwrap().to_string();
    sqlx::query("UPDATE _brain_api_keys SET expires_at=now()-interval '1 second' WHERE id=$1::uuid")
        .bind(&fresh_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(call(&router, keyed("GET", "/api/key_notes", &fresh, false, None)).await.body["error"]["code"], "API_KEY_EXPIRED");

    // Quotas: Postgres counters, headers on every keyed response, typed 429 with Retry-After.
    let (quota_id, quota) = create_key(&router, &token, json!({"name":"Quota","scopes":["key_notes:read"],"dailyQuota":2})).await;
    for remaining in ["1", "0"] {
        let ok = call(&router, keyed("GET", "/api/key_notes", &quota, false, None)).await;
        assert_eq!(ok.status, StatusCode::OK);
        assert_eq!(ok.headers["x-quota-limit"], "2");
        assert_eq!(ok.headers["x-quota-remaining"], remaining);
        assert!(ok.headers["x-quota-reset"].to_str().unwrap().parse::<i64>().unwrap() > 0);
    }
    let exhausted = call(&router, keyed("GET", "/api/key_notes", &quota, false, None)).await;
    assert_eq!(exhausted.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(exhausted.body["error"]["code"], "API_KEY_QUOTA_EXHAUSTED");
    assert!(exhausted.headers.contains_key("retry-after"));
    let used: Value = call(&router, session("GET", "/_brain/keys", &token, None)).await.body;
    let quota_row = used["items"].as_array().unwrap().iter().find(|k| k["id"] == quota_id.as_str()).unwrap().clone();
    assert_eq!(quota_row["usedToday"], 2);
    assert!(!quota_row["lastUsedAt"].is_null());

    // Trace: written off the hot path, filtered, keyset-paginated; query values never stored.
    let mut trace = Value::Null;
    for _ in 0..100 {
        trace = call(&router, session("GET", &format!("/_brain/keys/{reader_id}/events?limit=1"), &token, None)).await.body;
        if trace["items"].as_array().is_some_and(|items| !items.is_empty()) && trace["next"].is_i64() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let newest = &trace["items"][0];
    assert_eq!(newest["status"], 401);
    let older = call(&router, session("GET", &format!("/_brain/keys/{reader_id}/events?before={}&status=200", trace["next"]), &token, None)).await.body;
    let first = &older["items"][0];
    assert_eq!(first["route"], "/api/{name}");
    assert_eq!(first["resource"], "key_notes");
    assert_eq!(first["action"], "read");
    assert_eq!(first["queryKeys"], json!(["search", "limit"]));
    assert_eq!(first["clientIp"], "203.0.113.7");
    assert!(!older.to_string().contains("private-value"));
    let filtered = call(&router, session("GET", &format!("/_brain/keys/{reader_id}/events?resource=key_ledgers"), &token, None)).await.body;
    assert_eq!(filtered["items"], json!([]));
    let bad_filter = call(&router, session("GET", &format!("/_brain/keys/{reader_id}/events?from=yesterday"), &token, None)).await;
    assert_eq!(bad_filter.status, StatusCode::UNPROCESSABLE_ENTITY);
    let summary = call(&router, session("GET", &format!("/_brain/keys/{reader_id}/summary?days=1"), &token, None)).await.body;
    assert!(summary["items"].as_array().unwrap().iter().any(|row| row["status"] == 200 && row["count"].as_i64() >= Some(1)));
    let hidden = call(&router, session("GET", &format!("/_brain/keys/{reader_id}/events"), &stranger, None)).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);

    // OpenAPI documents both schemes when keys are enabled.
    let doc = call(&router, Request::get("/_brain/api").extension(axum::extract::ConnectInfo(std::net::SocketAddr::from(([203, 0, 113, 7], 40000)))).body(Body::empty()).unwrap()).await.body;
    assert_eq!(doc["components"]["securitySchemes"]["apiKey"]["name"], "X-API-Key");

    // Disabled projects reject any presented key with a typed error.
    let disabled = brain_backend::http::router(pool.clone(), config(vec![notes, ledgers], None)).await.unwrap();
    let off = call(&disabled, keyed("GET", "/api/key_notes", &quota, false, None)).await;
    assert_eq!(off.status, StatusCode::UNAUTHORIZED);
    assert_eq!(off.body["error"]["code"], "API_KEYS_DISABLED");
    assert_eq!(call(&disabled, session("GET", "/_brain/keys", &token, None)).await.status, StatusCode::NOT_FOUND);

    // Retention purge is bounded and owner deletion cascades keys, counters and trace.
    assert_eq!(brain_backend::api_keys::purge(&pool, 1).await.unwrap(), 0);
    sqlx::query("DELETE FROM _brain_res_key_notes WHERE owner=$1").bind(owner).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM _brain_users WHERE id=$1").bind(owner).execute(&pool).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM _brain_api_keys)+(SELECT count(*) FROM _brain_api_key_usage)+(SELECT count(*) FROM _brain_api_key_events)")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    pool.close().await;
}
