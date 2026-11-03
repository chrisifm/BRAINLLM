//! System identity: the one key that passes policy, field access and action roles. It exists only through the operator
//! API, leaves a marked trace, and ordinary or admin-owned keys never get any of its reach.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use brain_backend::{
    api_keys::{Keys, NewKey},
    auth::{Auth, Credentials},
    contract::{Field, Hooks, Policy, Resource},
    db,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "https://app.example.test";

fn sealed() -> Resource {
    let field = |kind: &str, access: Option<&str>| Field { kind: kind.into(), access: access.map(Into::into), ..Default::default() };
    Resource {
        name: "sys_notes".into(),
        search: vec![],
        fields: BTreeMap::from([
            ("title".into(), Field { required: true, ..field("str", None) }),
            ("internal".into(), field("str", Some("private"))),
            ("grade".into(), field("str", Some("readonly"))),
            ("token".into(), field("str", Some("writeonly"))),
            ("hits".into(), field("int", None)),
        ]),
        policy: Policy { read: vec!["owner".into()], create: vec!["user".into(), "admin".into()], update: vec!["owner".into()], delete: vec!["owner".into()], ..Default::default() },
        hooks: Hooks::default(),
        constraints: Default::default(),
        query: Default::default(),
    }
}
fn config(resources: Vec<Resource>) -> brain_backend::Config {
    brain_backend::Config {
        api_keys: Some(Default::default()),
        resources,
        jobs: vec![],
        uploads: std::env::temp_dir().join(format!("brain-system-keys-{}", Uuid::new_v4().simple())),
        origins: vec![ORIGIN.into()],
        production: false,
        jwt: None,
        google: None, apple: None, linkedin: None,
        gcs: None,
        roles: Default::default(),
        mail: None,
        auth_public_base_url: None,
        webhooks: None,
        before_hooks: None,
        sockets: None,
    }
}
async fn call(router: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
fn keyed(method: &str, uri: &str, key: &str, body: Option<Value>) -> Request<Body> {
    Request::builder()
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from(([203, 0, 113, 7], 40000))))
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("x-api-key", key)
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_default())
        .unwrap()
}
fn session(method: &str, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    Request::builder()
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from(([203, 0, 113, 7], 40000))))
        .method(method)
        .uri(uri)
        .header("cookie", format!("__Host-brain={token}"))
        .header("origin", ORIGIN)
        .header("content-type", "application/json")
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_default())
        .unwrap()
}
async fn user(auth: &Auth, pool: &sqlx::PgPool, role: &str) -> (Uuid, String) {
    let email = format!("sys-{}@example.test", Uuid::new_v4().simple());
    let password = "synthetic-system-key-password".to_string();
    let created = auth.signup(Credentials { email: email.clone(), password: password.clone() }).await.unwrap();
    sqlx::query("UPDATE _brain_users SET role=$1 WHERE id=$2").bind(role).bind(created.id).execute(pool).await.unwrap();
    let (_, token) = auth.login(Credentials { email, password }).await.unwrap().unwrap();
    (created.id, token)
}
fn new_key(scopes: &[&str]) -> NewKey {
    NewKey { name: "synthetic".into(), scopes: scopes.iter().map(|s| s.to_string()).collect(), expires_in_days: None, daily_quota: None, monthly_quota: None }
}

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn system_identity_is_operator_only_audited_and_no_other_key_reaches_it() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use a disposable database");
    let pool = db::connect(&url).await.unwrap();
    let notes = sealed();
    db::migrate(&pool, std::slice::from_ref(&notes)).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let domain: brain_backend::domain::Domain = serde_json::from_value(json!({"resources":{"sys_notes":{"actions":{"bump":{"roles":["owner"],"increment":{"hits":1}}}}},"operations":{}})).unwrap();
    let (router, _) = brain_backend::http::router_with_domain(pool.clone(), config(vec![notes]), domain).await.unwrap();
    let keys = Keys::new(pool.clone(), Default::default()).unwrap();
    let (author, _) = user(&auth, &pool, "user").await;
    let (member, _) = user(&auth, &pool, "user").await;
    let (admin, admin_session) = user(&auth, &pool, "admin").await;
    let (operator, operator_session) = user(&auth, &pool, "user").await;
    let scopes = ["sys_notes:*"];
    let (_, author_key) = keys.create(author, new_key(&scopes)).await.unwrap().unwrap();
    let (_, member_key) = keys.create(member, new_key(&scopes)).await.unwrap().unwrap();
    let (_, admin_key) = keys.create(admin, new_key(&scopes)).await.unwrap().unwrap();
    let (system_info, system_key) = keys.create_system(operator, new_key(&scopes)).await.unwrap().unwrap();
    assert_eq!(system_info["system"], true);
    let (_, read_only) = keys.create_system(operator, new_key(&["sys_notes:read"])).await.unwrap().unwrap();

    // Wildcards never reach the system identity, and no session route can create or re-issue one.
    assert!(keys.create_system(operator, new_key(&["*:*"])).await.unwrap().is_err());
    let forged = call(&router, session("POST", "/_brain/keys", &admin_session, Some(json!({"name":"x","scopes":["sys_notes:*"],"system":true})))).await;
    assert_ne!(forged.0, StatusCode::OK, "a session cannot ask for a system key: {}", forged.1);
    let system_id = system_info["id"].as_str().unwrap();
    for token in [&admin_session, &operator_session] {
        let rotated = call(&router, session("POST", &format!("/_brain/keys/{system_id}/rotate"), token, None)).await;
        assert_eq!(rotated.0, StatusCode::NOT_FOUND, "{}", rotated.1);
    }
    let management = call(&router, keyed("GET", "/_brain/keys", &system_key, None)).await;
    assert_eq!(management.1["error"]["code"], "API_KEY_NOT_ACCEPTED");

    // The record belongs to `author`; `internal` was set by the system key below.
    let created = call(&router, keyed("POST", "/api/sys_notes", &author_key, Some(json!({"title":"Plan"})))).await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    let id = created.1["id"].as_str().unwrap().to_string();
    let row = format!("/api/sys_notes/{id}");

    // Security matrix: what the system key may do and neither a member's nor an admin-owned key may.
    let sealed_write = json!({"internal":"secret","grade":"A"});
    for key in [&member_key, &admin_key] {
        let list = call(&router, keyed("GET", "/api/sys_notes?limit=100", key, None)).await;
        assert!(list.1["items"].as_array().is_none_or(|items| items.iter().all(|r| r["id"] != json!(id))), "owner scope: {}", list.1);
        assert_ne!(call(&router, keyed("GET", &row, key, None)).await.0, StatusCode::OK, "row policy on get");
        assert_ne!(call(&router, keyed("PATCH", &row, key, Some(json!({"title":"Taken"})))).await.0, StatusCode::OK, "row policy on patch");
        assert_ne!(call(&router, keyed("DELETE", &row, key, None)).await.0, StatusCode::NO_CONTENT, "row policy on delete");
        assert_ne!(call(&router, keyed("POST", &format!("{row}/actions/bump"), key, Some(json!({})))).await.0, StatusCode::OK, "action role");
    }
    for key in [&author_key, &admin_key] {
        let write = call(&router, keyed("POST", "/api/sys_notes", key, Some(json!({"title":"Mine","internal":"x"})))).await;
        assert_eq!(write.0, StatusCode::UNPROCESSABLE_ENTITY, "private field: {}", write.1);
        let write = call(&router, keyed("POST", "/api/sys_notes", key, Some(json!({"title":"Mine","grade":"x"})))).await;
        assert_eq!(write.0, StatusCode::UNPROCESSABLE_ENTITY, "readonly field: {}", write.1);
    }
    let own = call(&router, keyed("PATCH", &row, &author_key, Some(sealed_write.clone()))).await;
    assert_eq!(own.0, StatusCode::UNPROCESSABLE_ENTITY, "even the owner: {}", own.1);

    let patched = call(&router, keyed("PATCH", &row, &system_key, Some(json!({"internal":"secret","grade":"A","token":"hidden"})))).await;
    assert_eq!(patched.0, StatusCode::OK, "{}", patched.1);
    assert_eq!(patched.1["data"]["internal"], "secret");
    assert_eq!(patched.1["data"]["grade"], "A");
    assert!(patched.1["data"].get("token").is_none(), "writeonly stays unreadable for every identity");
    let read = call(&router, keyed("GET", &row, &system_key, None)).await;
    assert_eq!(read.0, StatusCode::OK, "{}", read.1);
    assert_eq!(read.1["data"]["internal"], "secret");
    assert_eq!(read.1["owner"].as_str(), Some(author.to_string().as_str()), "the row keeps its owner");
    let listed = call(&router, keyed("GET", "/api/sys_notes?limit=100", &system_key, None)).await;
    assert!(listed.1["items"].as_array().unwrap().iter().any(|r| r["id"] == json!(id)), "{}", listed.1);
    // Only this identity may name an account: a job finds one person's rows; for anyone else it would reveal an owner.
    let by_owner = call(&router, keyed("GET", &format!("/api/sys_notes?owner={author}&limit=100"), &system_key, None)).await;
    assert_eq!(by_owner.0, StatusCode::OK, "{}", by_owner.1);
    let items = by_owner.1["items"].as_array().unwrap();
    assert!(items.iter().any(|r| r["id"] == json!(id)) && items.iter().all(|r| r["owner"] == json!(author)), "{}", by_owner.1);
    let nobody = call(&router, keyed("GET", &format!("/api/sys_notes?owner={}", Uuid::new_v4()), &system_key, None)).await;
    assert_eq!(nobody.1["items"], json!([]), "{}", nobody.1);
    for key in [&author_key, &admin_key] {
        let denied = call(&router, keyed("GET", &format!("/api/sys_notes?owner={author}"), key, None)).await;
        assert_eq!(denied.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", denied.1);
    }
    let bumped = call(&router, keyed("POST", &format!("{row}/actions/bump"), &system_key, Some(json!({})))).await;
    assert_eq!(bumped.0, StatusCode::OK, "{}", bumped.1);
    let owner_view = call(&router, keyed("GET", &row, &author_key, None)).await;
    assert!(owner_view.1["data"].get("internal").is_none(), "the bypass ends with the request: {}", owner_view.1);

    // Scopes, validation and the audit trail still hold for the system key.
    assert_eq!(call(&router, keyed("PATCH", &row, &read_only, Some(json!({"title":"No"})))).await.1["error"]["code"], "API_KEY_SCOPE_DENIED");
    assert_eq!(call(&router, keyed("POST", "/api/sys_notes", &system_key, Some(json!({})))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let marked: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_api_key_events WHERE system AND key_id=$1").bind(system_id.parse::<Uuid>().unwrap()).fetch_one(&pool).await.unwrap();
    assert!(marked >= 4, "every use is marked, saw {marked}");
    let unmarked: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_api_key_events e JOIN _brain_api_keys k ON k.id=e.key_id WHERE e.system <> k.system").fetch_one(&pool).await.unwrap();
    assert_eq!(unmarked, 0);
    let rotated = keys.rotate_operator(system_id.parse().unwrap()).await.unwrap().unwrap();
    assert_eq!(rotated.0["system"], true, "the operator's replacement stays a system key");
    assert_eq!(call(&router, keyed("GET", &row, &rotated.1, None)).await.0, StatusCode::OK);
}

const BOUNDARY: &str = "systemkeysboundary";
fn upload(uri: &str, key: &str) -> Request<Body> {
    let body = format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n--{BOUNDARY}--\r\n");
    Request::builder()
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from(([203, 0, 113, 7], 40000))))
        .method("POST")
        .uri(uri)
        .header("content-type", format!("multipart/form-data; boundary={BOUNDARY}"))
        .header("x-api-key", key)
        .body(Body::from(body))
        .unwrap()
}

/// A key opens a bucket only through a scope naming it, then answers to the bucket's grants; system passes the grants.
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn keys_reach_buckets_only_by_named_scope_and_under_the_bucket_rules() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use a disposable database");
    let pool = db::connect(&url).await.unwrap();
    db::migrate(&pool, &[]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let reports: brain_backend::files::Bucket = serde_json::from_value(json!({"name":"sys_reports","visibility":"private","maxBytes":1024,"types":["text/plain"],"create":["user"],"read":["owner","admin"],"delete":["owner"]})).unwrap();
    let (router, _) = brain_backend::http::router_with_platform(pool.clone(), config(vec![]), brain_backend::platform::Platform { buckets: vec![reports], ..Default::default() }).await.unwrap();
    let keys = Keys::new(pool.clone(), Default::default()).unwrap();
    let (author, _) = user(&auth, &pool, "user").await;
    let (member, _) = user(&auth, &pool, "user").await;
    let (admin, _) = user(&auth, &pool, "admin").await;
    let (operator, _) = user(&auth, &pool, "user").await;
    let named = ["files.sys_reports:*"];
    let (_, author_key) = keys.create(author, new_key(&named)).await.unwrap().unwrap();
    let (_, member_key) = keys.create(member, new_key(&named)).await.unwrap().unwrap();
    let (_, admin_key) = keys.create(admin, new_key(&named)).await.unwrap().unwrap();
    let (_, wildcard) = keys.create(author, new_key(&["*:*"])).await.unwrap().unwrap();
    let (_, reader) = keys.create(author, new_key(&["files.sys_reports:read"])).await.unwrap().unwrap();
    let (_, system_key) = keys.create_system(operator, new_key(&named)).await.unwrap().unwrap();
    let base = "/_brain/files/sys_reports";

    assert_eq!(call(&router, keyed("GET", base, &wildcard, None)).await.1["error"]["code"], "API_KEY_SCOPE_DENIED", "a wildcard does not open a bucket");
    assert_eq!(call(&router, upload(base, &wildcard)).await.1["error"]["code"], "API_KEY_SCOPE_DENIED");
    assert_eq!(call(&router, upload(base, &reader)).await.1["error"]["code"], "API_KEY_SCOPE_DENIED", "read scope cannot upload");
    let created = call(&router, upload(base, &author_key)).await;
    assert_eq!(created.0, StatusCode::CREATED, "{}", created.1);
    assert_eq!(created.1["owner"].as_str(), Some(author.to_string().as_str()), "a key uploads as its owner");
    let id = created.1["id"].as_str().unwrap();
    let file = format!("{base}/{id}");
    assert_eq!(call(&router, keyed("GET", &format!("{file}/meta"), &author_key, None)).await.0, StatusCode::OK);
    assert_eq!(call(&router, keyed("GET", &format!("{file}/meta"), &member_key, None)).await.0, StatusCode::FORBIDDEN, "bucket grants apply");
    assert_eq!(call(&router, keyed("DELETE", &file, &member_key, None)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&router, keyed("DELETE", &file, &admin_key, None)).await.0, StatusCode::FORBIDDEN, "delete grants only the owner, not admin");
    let listing = call(&router, keyed("GET", base, &member_key, None)).await;
    assert!(listing.1["items"].as_array().is_none_or(|i| i.iter().all(|f| f["id"].as_str() != Some(id))), "{}", listing.1);
    assert_eq!(call(&router, keyed("GET", &format!("{file}/meta"), &system_key, None)).await.0, StatusCode::OK);
    let everything = call(&router, keyed("GET", base, &system_key, None)).await;
    assert!(everything.1["items"].as_array().unwrap().iter().any(|f| f["id"].as_str() == Some(id)), "{}", everything.1);
    assert_eq!(call(&router, keyed("DELETE", &file, &system_key, None)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&router, keyed("GET", "/_brain/keys", &author_key, None)).await.1["error"]["code"], "API_KEY_NOT_ACCEPTED");
}

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn self_service_off_closes_the_key_routes_and_keeps_operator_keys_working() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use a disposable database");
    let pool = db::connect(&url).await.unwrap();
    let notes = sealed();
    db::migrate(&pool, std::slice::from_ref(&notes)).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let mut closed = config(vec![notes]);
    closed.api_keys = Some(brain_backend::api_keys::Config { self_service: false, ..Default::default() });
    let router = brain_backend::http::router(pool.clone(), closed).await.unwrap();
    let keys = Keys::new(pool.clone(), Default::default()).unwrap();
    let (admin, admin_session) = user(&auth, &pool, "admin").await;
    let (_, system_key) = keys.create_system(admin, new_key(&["sys_notes:*"])).await.unwrap().unwrap();

    let created = call(&router, session("POST", "/_brain/keys", &admin_session, Some(json!({"name":"mine","scopes":["sys_notes:read"]})))).await;
    assert_eq!(created.0, StatusCode::NOT_FOUND, "a session cannot create a key: {}", created.1);
    assert_eq!(call(&router, session("GET", "/_brain/keys?all=true", &admin_session, None)).await.0, StatusCode::NOT_FOUND);
    let listed = call(&router, keyed("GET", "/api/sys_notes", &system_key, None)).await;
    assert_eq!(listed.0, StatusCode::OK, "the operator key still authenticates: {}", listed.1);
    let inserted = call(&router, keyed("POST", "/api/sys_notes", &system_key, Some(json!({"title":"from the job"})))).await;
    assert_eq!(inserted.0, StatusCode::OK, "{}", inserted.1);
    let api = call(&router, session("GET", "/_brain/api", &admin_session, None)).await.1;
    assert!(api["paths"].get("/_brain/keys").is_none(), "the description leaves the closed routes out");
    assert!(api["components"]["securitySchemes"].get("apiKey").is_some(), "keys remain a way in");
}


#[tokio::test]
#[ignore]
async fn an_operation_takes_a_named_scope_and_the_system_key_passes_its_roles() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use a disposable database");
    let pool = db::connect(&url).await.unwrap();
    let mut notes = sealed();
    notes.name = "op_notes".into();
    db::migrate(&pool, std::slice::from_ref(&notes)).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let domain: brain_backend::domain::Domain = serde_json::from_value(json!({"operations":{"stamp":{"roles":["admin"],"steps":[{"as":"made","create":"op_notes","data":{"title":"made"}}]}}})).unwrap();
    let (router, _) = brain_backend::http::router_with_domain(pool.clone(), config(vec![notes]), domain).await.unwrap();
    let keys = Keys::new(pool.clone(), Default::default()).unwrap();
    let (member, _) = user(&auth, &pool, "user").await;
    let (operator, _) = user(&auth, &pool, "user").await;
    let path = "/_brain/operations/stamp";
    let scope = ["operations.stamp:write"];

    let (_, plain) = keys.create(member, new_key(&scope)).await.unwrap().unwrap();
    let denied = call(&router, keyed("POST", path, &plain, Some(json!({})))).await;
    assert_eq!(denied.0, StatusCode::FORBIDDEN, "the owner's role still has to fit: {}", denied.1);

    let (_, wild) = keys.create_system(operator, new_key(&["op_notes:*"])).await.unwrap().unwrap();
    let denied = call(&router, keyed("POST", path, &wild, Some(json!({})))).await;
    assert_eq!(denied.1["error"]["code"], "API_KEY_SCOPE_DENIED", "a resource scope does not open an operation: {}", denied.1);

    let (_, system) = keys.create_system(operator, new_key(&scope)).await.unwrap().unwrap();
    let done = call(&router, keyed("POST", path, &system, Some(json!({})))).await;
    assert_eq!(done.0, StatusCode::OK, "{}", done.1);
    assert_eq!(done.1["operation"], "stamp");
    assert_eq!(done.1["result"]["made"]["data"]["title"], "made", "{}", done.1);
    let other = call(&router, keyed("POST", "/_brain/operations/other", &system, Some(json!({})))).await;
    assert_eq!(other.0, StatusCode::FORBIDDEN, "{}", other.1);
}
