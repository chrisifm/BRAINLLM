//! Row-conditioned read grants and server-only resources through the real HTTP router against a disposable Postgres.
//! Every read channel proves that a row hidden by a grant's `where` does not leak.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, Response},
};
use brain_backend::{
    auth,
    contract::{Policy, Resource},
    db,
    files::Bucket,
    http::cache_http::CachePolicy,
    platform::Platform,
};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::str::FromStr;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5191";
const BOUNDARY: &str = "rowpolicyboundary";

fn conditioned_read() -> Value {
    json!([{"public":{"where":{"status":"published"}}},"owner.shop","admin"])
}
fn resources() -> Vec<Resource> {
    let value = json!([
        {"name":"shops","fields":{"name":{"kind":"str","required":true},"status":{"kind":"enum(open|closed)","required":true}},
         "policy":{"read":["owner","admin"],"create":["member","admin"],"update":["owner","admin"],"delete":["admin"]},
         "query":{"filter":{"status":["eq"]}}},
        {"name":"jobs","search":["title"],
         "fields":{"shop":{"kind":"ref:shops","required":true},"title":{"kind":"str","required":true,"unique":true},
                   "status":{"kind":"enum(pending|published)","required":true},"pay":{"kind":"int","required":true},"section":{"kind":"ref:sections"}},
         "policy":{"read":conditioned_read(),"create":["member","admin"],"update":["seller","admin"],"delete":["admin"]},
         "query":{"expose_owner":false,"filter":{"status":["eq"],"pay":["gte"]},"sort":["pay"],"include":["shop"],
                  "aggregate":{"stats":{"sum":["pay"]},"by_status":{"group":"status"}},
                  "count":{"reviews":{"resource":"reviews","field":"job"}}}},
        {"name":"reviews","fields":{"job":{"kind":"ref:jobs","required":true},"body":{"kind":"str","required":true}},
         "policy":{"read":["public"],"create":["member"],"update":["admin"],"delete":["admin"]},
         "query":{"include":["job"],"filter":{"job.status":["eq"],"shop.status":["eq"]},"count":{}}},
        {"name":"sections","fields":{"name":{"kind":"str","required":true}},
         "policy":{"read":["public"],"create":["admin"],"update":["admin"],"delete":["admin"]},
         "query":{"count":{"jobs":{"resource":"jobs","field":"section"}}}},
        {"name":"articles","fields":{"status":{"kind":"enum(draft|live)","required":true},"slug":{"kind":"str","required":true,"unique":true,"slug":true}},
         "policy":{"read":[{"public":{"where":{"status":"live"}}},"admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}},
        {"name":"ledgers","fields":{"note":{"kind":"str","required":true}},"policy":{"api":false}},
        {"name":"notices","fields":{"body":{"kind":"str","required":true}},
         "policy":{"read":["public"],"create":["member"],"update":["owner"],"delete":["owner"]}},
        {"name":"diaries","fields":{"body":{"kind":"str","required":true}},
         "policy":{"read":["owner"],"create":["member"],"update":["owner"],"delete":["owner"]}}
    ]);
    let mut list: Vec<Value> = value.as_array().unwrap().clone();
    // reviews.shop lets a public resource filter through a target its reader may not read at all.
    list[2]["fields"]["shop"] = json!({"kind":"ref:shops"});
    list.into_iter().map(|v| serde_json::from_value(v).unwrap()).collect()
}
async fn database() -> sqlx::PgPool {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_row_policy_{}", Uuid::new_v4().simple());
    // Fixed ASCII prefix plus UUID hex, never request input.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name);
    let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
    base.close().await;
    pool
}
#[derive(Clone)]
struct Actor {
    id: Uuid,
    token: String,
}
async fn actor(pool: &sqlx::PgPool, role: &str) -> Actor {
    let id = Uuid::new_v4();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role) VALUES($1,$2,'synthetic-unused-hash',$3)")
        .bind(id).bind(format!("{id}@example.test")).bind(role).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO _brain_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')")
        .bind(auth::digest(&token)).bind(id).execute(pool).await.unwrap();
    Actor { id, token }
}
fn roles() -> brain_backend::auth_flows::RolesConfig {
    brain_backend::auth_flows::RolesConfig { roles: ["admin", "member", "seller"].map(String::from).to_vec(), signup_role: "member".into(), ..Default::default() }
}
fn config(resources: Vec<Resource>, uploads: &str) -> brain_backend::Config {
    brain_backend::Config {
        api_keys: Some(Default::default()), roles: roles(), mail: None, auth_public_base_url: None, webhooks: None, before_hooks: None,
        sockets: None, gcs: None, jwt: None, google: None, resources, jobs: Vec::new(),
        uploads: std::env::temp_dir().join(format!("brain-row-policy-{uploads}-{}", Uuid::new_v4().simple())),
        origins: vec![ORIGIN.into()], production: false,
    }
}
struct Fix {
    http: Router,
    pool: sqlx::PgPool,
    shop_owner: Actor,
    stranger: Actor,
    admin: Actor,
    editor: Actor,
    shop: String,
    published: String,
    draft: String,
}
async fn fixture(platform: Platform) -> Fix {
    let pool = database().await;
    let all = resources();
    db::migrate(&pool, &all).await.unwrap();
    roles().register(&pool).await.unwrap();
    let (shop_owner, stranger, admin, editor) = (actor(&pool, "member").await, actor(&pool, "member").await, actor(&pool, "admin").await, actor(&pool, "seller").await);
    let cfg = config(all.clone(), "http");
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), cfg, platform).await.unwrap();
    let shops = all[0].clone();
    let jobs = all[1].clone();
    let shop = db::create(&pool, &shops, shop_owner.id, json!({"name":"Corner shop","status":"open"})).await.unwrap();
    let shop = shop["id"].as_str().unwrap().to_string();
    let published = db::create(&pool, &jobs, admin.id, json!({"shop":shop,"title":"Open role","status":"published","pay":10})).await.unwrap();
    let draft = db::create(&pool, &jobs, admin.id, json!({"shop":shop,"title":"Hidden draft","status":"pending","pay":900})).await.unwrap();
    Fix {
        http, pool, shop_owner, stranger, admin, editor, shop,
        published: published["id"].as_str().unwrap().into(), draft: draft["id"].as_str().unwrap().into(),
    }
}
async fn send(http: &Router, method: &str, path: &str, who: Option<&Actor>, body: Option<Value>) -> Response<Body> {
    let mut b = Request::builder().method(method).uri(path).header("origin", ORIGIN)
        .extension(ConnectInfo("127.0.0.1:42002".parse::<std::net::SocketAddr>().unwrap()));
    if let Some(a) = who {
        b = b.header("cookie", format!("{}={}", auth::COOKIE, a.token));
    }
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    };
    http.clone().oneshot(req.unwrap()).await.unwrap()
}
async fn call(http: &Router, method: &str, path: &str, who: Option<&Actor>, body: Option<Value>) -> (u16, Value) {
    let response = send(http, method, path, who, body).await;
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
fn ids(list: &Value) -> Vec<String> {
    list["items"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
}
fn without_rid(mut v: Value) -> Value {
    v.as_object_mut().map(|o| o.remove("rid"));
    v
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn list_total_filter_sort_cursor_aggregate_and_search_hide_rows() {
    let f = fixture(Platform::default()).await;
    let (status, list) = call(&f.http, "GET", "/api/jobs", None, None).await;
    assert_eq!((status, ids(&list), list["total"].clone()), (200, vec![f.published.clone()], json!(1)), "{list}");
    // Filter, range and sort oracles: the hidden row is outside every predicate.
    for path in ["/api/jobs?status=pending", "/api/jobs?pay[gte]=500", "/api/jobs?search=Hidden", "/api/jobs?search=dr"] {
        let (status, list) = call(&f.http, "GET", path, None, None).await;
        assert_eq!((status, list["total"].clone()), (200, json!(0)), "{path}: {list}");
    }
    let (_, sorted) = call(&f.http, "GET", "/api/jobs?sort=-pay&limit=1", None, None).await;
    assert_eq!(ids(&sorted), vec![f.published.clone()]);
    assert!(sorted.get("next_cursor").is_none() || call(&f.http, "GET", &format!("/api/jobs?sort=-pay&limit=1&cursor={}", sorted["next_cursor"].as_str().unwrap()), None, None).await.1["items"].as_array().unwrap().is_empty());
    let (_, stats) = call(&f.http, "GET", "/api/jobs?aggregate=stats,by_status", None, None).await;
    assert_eq!(stats["aggregates"]["stats"]["count"], 1, "{stats}");
    assert_eq!(stats["aggregates"]["stats"]["sum"]["pay"], "10", "{stats}");
    assert_eq!(stats["aggregates"]["by_status"].as_array().unwrap().len(), 1, "{stats}");
    // Grants are OR-ed: relational owners and admins see both rows, a signed-in stranger only the published one.
    for who in [&f.shop_owner, &f.admin] {
        assert_eq!(call(&f.http, "GET", "/api/jobs", Some(who), None).await.1["total"], 2);
    }
    assert_eq!(call(&f.http, "GET", "/api/jobs", Some(&f.stranger), None).await.1["total"], 1);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn get_of_a_hidden_row_answers_exactly_like_a_missing_one() {
    let f = fixture(Platform::default()).await;
    assert_eq!(call(&f.http, "GET", &format!("/api/jobs/{}", f.published), None, None).await.0, 200);
    let (status, hidden) = call(&f.http, "GET", &format!("/api/jobs/{}", f.draft), None, None).await;
    let (_, missing) = call(&f.http, "GET", &format!("/api/jobs/{}", Uuid::new_v4()), None, None).await;
    assert_eq!(status, 404);
    assert_eq!(without_rid(hidden), without_rid(missing));
    assert_eq!(call(&f.http, "GET", &format!("/api/jobs/{}", f.draft), Some(&f.stranger), None).await.0, 404);
    assert_eq!(call(&f.http, "GET", &format!("/api/jobs/{}", f.draft), Some(&f.shop_owner), None).await.0, 200);
    assert_eq!(call(&f.http, "GET", &format!("/api/jobs/{}", f.draft), Some(&f.admin), None).await.0, 200);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn includes_counts_and_relational_filters_apply_the_target_rows() {
    let f = fixture(Platform::default()).await;
    let reviews = resources()[2].clone();
    let on_draft = db::create(&f.pool, &reviews, f.admin.id, json!({"job":f.draft,"body":"On draft","shop":f.shop})).await.unwrap();
    db::create(&f.pool, &reviews, f.admin.id, json!({"job":f.published,"body":"On open"})).await.unwrap();
    let (_, list) = call(&f.http, "GET", "/api/reviews?include=job", None, None).await;
    for row in list["items"].as_array().unwrap() {
        let included = &row["included"]["job"];
        if row["id"] == on_draft["id"] {
            assert!(included.is_null(), "{row}");
        } else {
            assert_eq!(included["data"]["status"], "published", "{row}");
        }
    }
    // The relational filter cannot confirm a hidden parent: a draft parent matches nothing anonymously.
    let (status, filtered) = call(&f.http, "GET", "/api/reviews?job.status=pending", None, None).await;
    assert_eq!((status, filtered["total"].clone()), (200, json!(0)), "{filtered}");
    assert_eq!(call(&f.http, "GET", "/api/reviews?job.status=pending", Some(&f.shop_owner), None).await.1["total"], 1);
    // A reader who may not read the target at all gets a denial that depends on no row.
    let closed = call(&f.http, "GET", "/api/reviews?shop.status=closed", None, None).await;
    let open = call(&f.http, "GET", "/api/reviews?shop.status=open", None, None).await;
    assert_eq!((closed.0, open.0), (401, 401), "{closed:?} {open:?}");
    assert_eq!(without_rid(closed.1), without_rid(open.1));
    // Per-record counts only see the rows of the counted resource this reader may read.
    let all = resources();
    let section = db::create(&f.pool, &all[3], f.admin.id, json!({"name":"Kitchen"})).await.unwrap();
    for (title, status) in [("Counted open", "published"), ("Counted draft", "pending")] {
        db::create(&f.pool, &all[1], f.admin.id, json!({"shop":f.shop,"title":title,"status":status,"pay":1,"section":section["id"]})).await.unwrap();
    }
    let counted = |who: Option<Actor>| {
        let http = f.http.clone();
        async move { call(&http, "GET", "/api/sections?count=jobs", who.as_ref(), None).await.1["items"][0]["counts"]["jobs"].clone() }
    };
    assert_eq!(counted(None).await, 1);
    assert_eq!(counted(Some(f.stranger.clone())).await, 1);
    assert_eq!(counted(Some(f.shop_owner.clone())).await, 2);
    assert_eq!(counted(Some(f.admin.clone())).await, 2);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn writes_never_return_a_row_their_author_may_not_read() {
    let f = fixture(Platform::default()).await;
    // Unique conflict names the field only: never the row, its id or its other values.
    let (status, conflict) = call(&f.http, "POST", "/api/jobs", Some(&f.stranger), Some(json!({"shop":f.shop,"title":"Hidden draft","status":"pending","pay":1}))).await;
    assert_eq!(status, 409, "{conflict}");
    assert_eq!(conflict["error"]["fields"], json!({"title":"Already used"}));
    // Only the error body: the random request id may legitimately contain any digits.
    assert!(!conflict["error"].to_string().contains(&f.draft) && !conflict["error"].to_string().contains("900"), "{conflict}");
    // A writer whose new row stays hidden receives the id and no data.
    let (status, created) = call(&f.http, "POST", "/api/jobs", Some(&f.stranger), Some(json!({"shop":f.shop,"title":"Secret plan","status":"pending","pay":7}))).await;
    assert_eq!(status, 200);
    assert_eq!(created["data"], json!({}), "{created}");
    assert!(created.get("owner").is_none());
    let (status, patched) = call(&f.http, "PATCH", &format!("/api/jobs/{}", f.draft), Some(&f.editor), Some(json!({"pay":901}))).await;
    assert_eq!((status, patched["data"].clone()), (200, json!({})), "{patched}");
    let (status, put) = call(&f.http, "PUT", &format!("/api/jobs/{}", f.draft), Some(&f.editor), Some(json!({"shop":f.shop,"title":"Hidden draft","status":"pending","pay":902}))).await;
    assert_eq!((status, put["data"].clone()), (200, json!({})), "{put}");
    // A write that publishes the row makes it readable, so the response carries it.
    let (_, opened) = call(&f.http, "PATCH", &format!("/api/jobs/{}", f.draft), Some(&f.editor), Some(json!({"status":"published"}))).await;
    assert_eq!(opened["data"]["status"], "published", "{opened}");
    // Upsert with on_conflict ignore used to return the existing row untouched.
    let (status, upsert) = call(&f.http, "POST", "/api/jobs/upsert", Some(&f.stranger), Some(json!({"key":["title"],"on_conflict":"ignore","data":{"shop":f.shop,"title":"Secret plan","status":"pending","pay":1}}))).await;
    assert_eq!((status, upsert["record"]["data"].clone(), upsert["created"].clone()), (200, json!({}), json!(false)), "{upsert}");
    let (status, batch) = call(&f.http, "POST", "/api/jobs/batch", Some(&f.stranger), Some(json!({"op":"create","items":[{"shop":f.shop,"title":"Batch secret","status":"pending","pay":3}]}))).await;
    assert_eq!((status, batch["items"][0]["data"].clone()), (200, json!({})), "{batch}");
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn http_cache_is_public_only_for_anonymous_readers_and_never_turns_a_hidden_row_into_a_304() {
    let cache = CachePolicy::from_contract(&json!({"jobs":{"maxAge":"1m"}}), &resources()).unwrap();
    let f = fixture(Platform { cache, ..Default::default() }).await;
    let anonymous = send(&f.http, "GET", "/api/jobs", None, None).await;
    assert!(anonymous.headers()["cache-control"].to_str().unwrap().starts_with("public, max-age=60"));
    let signed = send(&f.http, "GET", "/api/jobs", Some(&f.shop_owner), None).await;
    assert!(!signed.headers().get("cache-control").is_some_and(|v| v.to_str().unwrap().starts_with("public")));
    let request = Request::builder().method("GET").uri(format!("/api/jobs/{}", f.draft)).header("origin", ORIGIN).header("if-none-match", "*")
        .extension(ConnectInfo("127.0.0.1:42003".parse::<std::net::SocketAddr>().unwrap())).body(Body::empty()).unwrap();
    let hidden = f.http.clone().oneshot(request).await.unwrap();
    assert_eq!(hidden.status().as_u16(), 404);
    assert!(hidden.headers().get("etag").is_none());
    f.pool.close().await;
}

fn multipart(bytes: &[u8]) -> Vec<u8> {
    let mut body = format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\n").into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}
async fn upload(http: &Router, path: &str, who: &Actor) -> Value {
    let request = Request::builder().method("POST").uri(path).header("origin", ORIGIN)
        .header("cookie", format!("{}={}", auth::COOKIE, who.token))
        .header("content-type", format!("multipart/form-data; boundary={BOUNDARY}"))
        .extension(ConnectInfo("127.0.0.1:42004".parse::<std::net::SocketAddr>().unwrap()))
        .body(Body::from(multipart(b"plain file"))).unwrap();
    let response = http.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    assert!(status.is_success(), "{status} {}", String::from_utf8_lossy(&bytes));
    serde_json::from_slice(&bytes).unwrap()
}

/// Reported on a real app: the file of a row its relational owner may read answered 403. `related` is now the get rule.
#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn related_files_follow_the_get_rule_for_relational_owners_and_hidden_rows() {
    let bucket = Bucket::from_contract("job_files", json!({"resource":"jobs","field":"title","types":"*","create":["member","admin"],"read":["related"]})).unwrap();
    let banners = Bucket::from_contract("banners", json!({"visibility":"public","types":"*","create":["member"]})).unwrap();
    let f = fixture(Platform { buckets: vec![bucket, banners], ..Default::default() }).await;
    let uploaded = upload(&f.http, &format!("/_brain/files/job_files?record={}", f.draft), &f.admin).await;
    let meta = format!("/_brain/files/job_files/{}/meta", uploaded["id"].as_str().unwrap());
    let meta = if call(&f.http, "GET", &meta, Some(&f.admin), None).await.0 == 200 { meta } else { format!("/_brain/files/job_files/{}", uploaded["id"].as_str().unwrap()) };
    // Attaching to a row needs its update grant: denied on a readable row, like a missing row on a hidden one.
    let attach = |record: &str| {
        let path = format!("/_brain/files/job_files?record={record}");
        let (http, token) = (f.http.clone(), f.stranger.token.clone());
        async move {
            let request = Request::builder().method("POST").uri(path).header("origin", ORIGIN)
                .header("cookie", format!("{}={token}", auth::COOKIE))
                .header("content-type", format!("multipart/form-data; boundary={BOUNDARY}"))
                .extension(ConnectInfo("127.0.0.1:42007".parse::<std::net::SocketAddr>().unwrap()))
                .body(Body::from(multipart(b"plain file"))).unwrap();
            http.oneshot(request).await.unwrap().status().as_u16()
        }
    };
    assert_eq!(attach(&f.published).await, 403);
    assert_eq!(attach(&f.draft).await, 404);
    assert_eq!(attach(&Uuid::new_v4().to_string()).await, 404);
    // Same outcome as a get of the linked row, in both directions.
    assert_eq!(call(&f.http, "GET", &format!("/api/jobs/{}", f.draft), Some(&f.shop_owner), None).await.0, 200);
    assert_eq!(call(&f.http, "GET", &meta, Some(&f.shop_owner), None).await.0, 200);
    let get = call(&f.http, "GET", &format!("/api/jobs/{}", f.draft), Some(&f.stranger), None).await;
    let file = call(&f.http, "GET", &meta, Some(&f.stranger), None).await;
    assert_eq!((get.0, file.0), (404, 404), "{file:?}");
    assert_eq!(without_rid(get.1)["error"]["code"], without_rid(file.1)["error"]["code"]);
    assert_eq!(call(&f.http, "GET", &meta, None, None).await.0, 404);
    // Publishing the row opens its file exactly when the row opens.
    let (status, _) = call(&f.http, "PATCH", &format!("/api/jobs/{}", f.draft), Some(&f.admin), Some(json!({"status":"published"}))).await;
    assert_eq!(status, 200);
    assert_eq!(call(&f.http, "GET", &meta, None, None).await.0, 200);
    // Anonymous metadata of a public bucket keeps who uploaded it and which record it belongs to private.
    let banner = upload(&f.http, "/_brain/files/banners", &f.stranger).await;
    let public = format!("/_brain/files/banners/{}", banner["id"].as_str().unwrap());
    let path = if call(&f.http, "GET", &format!("{public}/meta"), None, None).await.0 == 200 { format!("{public}/meta") } else { public };
    let (status, anonymous) = call(&f.http, "GET", &path, None, None).await;
    assert_eq!(status, 200, "{anonymous}");
    for hidden in ["owner", "resource", "record", "field", "sha256"] {
        assert!(anonymous.get(hidden).is_none(), "{hidden}: {anonymous}");
    }
    assert_eq!(call(&f.http, "GET", &path, Some(&f.stranger), None).await.1["owner"], json!(f.stranger.id));
    f.pool.close().await;
}

// B0735 refused `filter: { priority: ['eq'] }` on a field `visible: ['admin']`, so admins could not filter by it.
#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn a_filter_on_a_role_only_field_serves_its_role_and_looks_undeclared_to_everyone_else() {
    let pool = database().await;
    let tickets: Resource = serde_json::from_value(json!({"name":"tickets",
        "fields":{"title":{"kind":"str","required":true},"priority":{"kind":"enum(low|high)","required":true,"visible":["admin"]}},
        "policy":{"read":["public"],"create":["admin"],"update":["admin"],"delete":["admin"]},
        "query":{"filter":{"priority":["eq"],"title":["prefix"]}}}))
    .unwrap();
    tickets.validate().unwrap();
    let all = vec![tickets.clone()];
    db::migrate(&pool, &all).await.unwrap();
    roles().register(&pool).await.unwrap();
    let (admin, member) = (actor(&pool, "admin").await, actor(&pool, "member").await);
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(all, "role-filter"), Platform::default()).await.unwrap();
    let high = db::create(&pool, &tickets, admin.id, json!({"title":"Outage","priority":"high"})).await.unwrap();
    db::create(&pool, &tickets, admin.id, json!({"title":"Typo","priority":"low"})).await.unwrap();
    let (status, list) = call(&http, "GET", "/api/tickets?priority=high", Some(&admin), None).await;
    assert_eq!((status, ids(&list)), (200, vec![high["id"].as_str().unwrap().to_string()]), "{list}");
    for who in [Some(&member), None] {
        let (_, undeclared) = call(&http, "GET", "/api/tickets?nothing=high", who, None).await;
        for path in ["/api/tickets?priority=high", "/api/tickets?priority[ne]=low"] {
            let (status, error) = call(&http, "GET", path, who, None).await;
            let key = path.split('?').nth(1).unwrap().split('=').next().unwrap();
            assert_eq!(status, 422, "{path}: {error}");
            assert_eq!(error["error"]["code"], undeclared["error"]["code"], "{path}: {error}");
            assert_eq!(error["error"]["fields"][key], undeclared["error"]["fields"]["nothing"], "{path}: {error}");
            assert!(!error["error"]["fields"][key].as_str().unwrap().contains("priority"), "{error}");
        }
        // The rows are still listed, without the field.
        let (status, list) = call(&http, "GET", "/api/tickets", who, None).await;
        assert_eq!((status, list["total"].clone()), (200, json!(2)), "{list}");
        assert!(list["items"].as_array().unwrap().iter().all(|item| item["data"].get("priority").is_none()), "{list}");
    }
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn server_only_resources_have_no_public_surface_but_a_named_key_reaches_them() {
    let f = fixture(Platform::default()).await;
    let ledgers = resources()[5].clone();
    let row = db::create(&f.pool, &ledgers, f.admin.id, json!({"note":"internal"})).await.unwrap();
    let id = row["id"].as_str().unwrap();
    let (_, unknown) = call(&f.http, "GET", "/api/never_declared", Some(&f.admin), None).await;
    for (method, path, body) in [
        ("GET", "/api/ledgers".to_string(), None),
        ("GET", format!("/api/ledgers/{id}"), None),
        ("POST", "/api/ledgers".to_string(), Some(json!({"note":"x"}))),
        ("PATCH", format!("/api/ledgers/{id}"), Some(json!({"note":"x"}))),
        ("DELETE", format!("/api/ledgers/{id}"), None),
        ("POST", "/api/ledgers/batch".to_string(), Some(json!({"op":"create","items":[{"note":"x"}]}))),
        ("POST", "/api/ledgers/upsert".to_string(), Some(json!({"key":["note"],"data":{"note":"x"}}))),
    ] {
        let (status, reply) = call(&f.http, method, &path, Some(&f.admin), body).await;
        assert_eq!(status, 404, "{method} {path}: {reply}");
        assert_eq!(without_rid(reply), without_rid(unknown.clone()), "{method} {path}");
    }
    let key = |scopes: Value| {
        let (http, admin) = (f.http.clone(), f.admin.clone());
        async move {
            let (status, reply) = call(&http, "POST", "/_brain/keys", Some(&admin), Some(json!({"name":Uuid::new_v4().to_string(),"scopes":scopes}))).await;
            assert_eq!(status, 200, "{reply}");
            reply["secret"].as_str().unwrap().to_string()
        }
    };
    let with_key = |secret: String| Request::builder().method("GET").uri("/api/ledgers").header("x-api-key", secret)
        .extension(ConnectInfo("127.0.0.1:42005".parse::<std::net::SocketAddr>().unwrap())).body(Body::empty()).unwrap();
    assert_eq!(f.http.clone().oneshot(with_key(key(json!(["*:*"])).await)).await.unwrap().status().as_u16(), 404);
    assert_eq!(f.http.clone().oneshot(with_key(key(json!(["ledgers:read"])).await)).await.unwrap().status().as_u16(), 200);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn domain_actions_keep_hidden_rows_hidden_from_callers_and_responses() {
    let f = fixture(Platform::default()).await;
    let domain: brain_backend::domain::Domain = serde_json::from_value(json!({"resources":{"jobs":{"actions":{"bump":{"roles":["seller","admin"],"increment":{"pay":1}}}}}})).unwrap();
    let (http, _) = brain_backend::http::router_with_domain(f.pool.clone(), config(resources(), "domain"), domain).await.unwrap();
    let action = |id: &str| format!("/api/jobs/{id}/actions/bump");
    // The action runs, and the row its caller may not read stays out of the response.
    let (status, done) = call(&http, "POST", &action(&f.draft), Some(&f.editor), Some(json!({}))).await;
    assert_eq!((status, done["data"].clone()), (200, json!({})), "{done}");
    assert_eq!(db::get(&f.pool, &resources()[1], f.draft.parse().unwrap()).await.unwrap().unwrap()["data"]["pay"], 901);
    // A caller without the action's role cannot tell a hidden row from a missing one.
    let hidden = call(&http, "POST", &action(&f.draft), Some(&f.stranger), Some(json!({}))).await;
    let missing = call(&http, "POST", &action(&Uuid::new_v4().to_string()), Some(&f.stranger), Some(json!({}))).await;
    assert_eq!((hidden.0, missing.0), (404, 404), "{hidden:?} {missing:?}");
    assert_eq!(without_rid(hidden.1), without_rid(missing.1));
    f.pool.close().await;
}

/// A write the policy does not grant: 403 when the writer may read the row, the answer of a missing row when not.
#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn denied_writes_answer_403_on_readable_rows_and_like_a_missing_row_on_hidden_ones() {
    let f = fixture(Platform::default()).await;
    let all = resources();
    let notice = db::create(&f.pool, &all[6], f.shop_owner.id, json!({"body":"Open"})).await.unwrap()["id"].as_str().unwrap().to_string();
    let diary = db::create(&f.pool, &all[7], f.shop_owner.id, json!({"body":"Private"})).await.unwrap()["id"].as_str().unwrap().to_string();
    let domain: brain_backend::domain::Domain = serde_json::from_value(json!({"resources":{
        "notices":{"actions":{"touch":{"roles":["owner"],"set":{"body":"Touched"}}}},
        "diaries":{"actions":{"touch":{"roles":["owner"],"set":{"body":"Touched"}}}},
        "jobs":{"actions":{"bump":{"roles":["admin"],"increment":{"pay":1}}}}}})).unwrap();
    let (http, _) = brain_backend::http::router_with_domain(f.pool.clone(), config(all.clone(), "denied"), domain).await.unwrap();
    let writes = |name: &'static str, id: String| -> Vec<(&'static str, String, Option<Value>)> { vec![
        ("DELETE", format!("/api/{name}/{id}"), None),
        ("PUT", format!("/api/{name}/{id}"), Some(json!({"body":"Changed"}))),
        ("PATCH", format!("/api/{name}/{id}"), Some(json!({"body":"Changed"}))),
        ("POST", format!("/api/{name}/{id}/actions/touch"), Some(json!({}))),
        ("POST", format!("/api/{name}/batch"), Some(json!({"op":"delete","items":[{"id":id}]}))),
        ("POST", format!("/api/{name}/batch"), Some(json!({"op":"update","items":[{"id":id,"data":{"body":"Changed"}}]}))),
    ] };
    for (method, path, body) in writes("notices", notice.clone()) {
        let (status, reply) = call(&http, method, &path, Some(&f.stranger), body).await;
        assert_eq!((status, reply["error"]["code"].as_str()), (403, Some("POLICY_DENIED")), "{method} {path}: {reply}");
    }
    let hidden = writes("diaries", diary.clone());
    let missing = writes("diaries", Uuid::new_v4().to_string());
    for ((method, path, body), (_, absent, absent_body)) in hidden.into_iter().zip(missing) {
        let reply = call(&http, method, &path, Some(&f.stranger), body).await;
        let gone = call(&http, method, &absent, Some(&f.stranger), absent_body).await;
        assert_eq!((reply.0, gone.0), (404, 404), "{method} {path}: {reply:?}");
        assert_eq!(without_rid(reply.1), without_rid(gone.1), "{method} {path}");
    }
    // The row survived every attempt, and its owner still writes.
    assert_eq!(call(&http, "GET", &format!("/api/notices/{notice}"), None, None).await.1["data"]["body"], "Open");
    assert_eq!(call(&http, "DELETE", &format!("/api/notices/{notice}"), Some(&f.shop_owner), None).await.0, 200);
    // A conditioned row follows what this reader can see of it: published denies, draft hides.
    let bump = |id: &str| format!("/api/jobs/{id}/actions/bump");
    assert_eq!(call(&http, "POST", &bump(&f.published), Some(&f.stranger), Some(json!({}))).await.0, 403);
    assert_eq!(call(&http, "POST", &bump(&f.draft), Some(&f.stranger), Some(json!({}))).await.0, 404);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn slug_history_does_not_confirm_a_hidden_row() {
    let f = fixture(Platform::default()).await;
    let (status, created) = call(&f.http, "POST", "/api/articles", Some(&f.admin), Some(json!({"status":"draft","slug":"first-title"}))).await;
    assert_eq!(status, 200, "{created}");
    let id = created["id"].as_str().unwrap();
    assert_eq!(call(&f.http, "PATCH", &format!("/api/articles/{id}"), Some(&f.admin), Some(json!({"slug":"second-title"}))).await.0, 200);
    let redirect = "/api/articles/redirects/first-title";
    assert_eq!(call(&f.http, "GET", redirect, Some(&f.admin), None).await.1["slug"], "second-title");
    let hidden = call(&f.http, "GET", redirect, None, None).await;
    let unknown = call(&f.http, "GET", "/api/articles/redirects/never-used", None, None).await;
    assert_eq!((hidden.0, unknown.0), (404, 404), "{hidden:?}");
    assert_eq!(without_rid(hidden.1), without_rid(unknown.1));
    assert_eq!(call(&f.http, "PATCH", &format!("/api/articles/{id}"), Some(&f.admin), Some(json!({"status":"live"}))).await.0, 200);
    assert_eq!(call(&f.http, "GET", redirect, None, None).await.1["slug"], "second-title");
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn api_keys_see_the_rows_their_user_sees_and_denials_explain_the_rule_class() {
    let f = fixture(Platform::default()).await;
    let (status, created) = call(&f.http, "POST", "/_brain/keys", Some(&f.stranger), Some(json!({"name":"Reader","scopes":["jobs:read"]}))).await;
    assert_eq!(status, 200, "{created}");
    let keyed = |path: String| Request::builder().method("GET").uri(path).header("x-api-key", created["secret"].as_str().unwrap())
        .extension(ConnectInfo("127.0.0.1:42006".parse::<std::net::SocketAddr>().unwrap())).body(Body::empty()).unwrap();
    let list = f.http.clone().oneshot(keyed("/api/jobs".into())).await.unwrap();
    let list: Value = serde_json::from_slice(&to_bytes(list.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(ids(&list), vec![f.published.clone()], "{list}");
    assert_eq!(f.http.clone().oneshot(keyed(format!("/api/jobs/{}", f.draft))).await.unwrap().status().as_u16(), 404);
    // POLICY_DENIED names which rules were evaluated, and nothing about the row or its owner.
    let (status, hidden) = call(&f.http, "GET", &format!("/api/shops/{}", f.shop), Some(&f.stranger), None).await;
    assert_eq!((status, hidden["error"]["code"].as_str()), (404, Some("NOT_FOUND")), "a row outside the scope reads like a missing one: {hidden}");
    let (status, denied) = call(&f.http, "PATCH", &format!("/api/jobs/{}", f.published), Some(&f.stranger), Some(json!({"title":"Taken"}))).await;
    assert_eq!(status, 403, "{denied}");
    assert_eq!(denied["error"]["code"], "POLICY_DENIED");
    let fix = denied["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("role") && fix.contains("owner.<ref>") && fix.contains("row conditions"), "{fix}");
    assert!(!denied.to_string().contains(&f.shop_owner.id.to_string()) && !denied.to_string().contains("Corner shop"), "{denied}");
    f.pool.close().await;
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn socket(address: std::net::SocketAddr, who: &Actor) -> Socket {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = format!("ws://{address}/_brain/socket").into_client_request().unwrap();
    let headers = request.headers_mut();
    headers.insert("sec-websocket-protocol", "brain.v1".parse().unwrap());
    headers.insert("origin", ORIGIN.parse().unwrap());
    headers.insert("cookie", format!("{}={}", auth::COOKIE, who.token).parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(frame(&mut socket).await["type"], "ready");
    socket
}
async fn frame(socket: &mut Socket) -> Value {
    use futures_util::StreamExt;
    loop {
        let message = tokio::time::timeout(std::time::Duration::from_secs(6), socket.next()).await.expect("socket deadline").unwrap().unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}
async fn command(socket: &mut Socket, value: Value) -> Value {
    use futures_util::SinkExt;
    socket.send(tokio_tungstenite::tungstenite::Message::Text(value.to_string().into())).await.unwrap();
    frame(socket).await
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn realtime_rooms_and_collection_events_follow_the_row_condition() {
    let f = fixture(Platform::default()).await;
    let mut cfg = config(resources(), "sockets");
    cfg.production = true;
    cfg.jwt = Some(brain_backend::jwt::Config::new("synthetic-socket-signing-key-over-32-bytes".into(), "socket-test".into(), "socket-test".into()).unwrap());
    cfg.sockets = Some(brain_backend::http::SocketConfig::new(vec![ORIGIN.into()]).unwrap());
    let (router, control) = brain_backend::http::router_with_socket_control(f.pool.clone(), cfg).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap() });
    let mut stranger = socket(address, &f.stranger).await;
    let mut owner = socket(address, &f.shop_owner).await;
    // A hidden row refuses its room exactly like any room the viewer may not read; a readable one opens.
    assert_eq!(command(&mut stranger, json!({"type":"join","resource":"jobs","id":f.draft})).await["code"], "ROOM_FORBIDDEN");
    assert_eq!(command(&mut stranger, json!({"type":"join","resource":"jobs","id":f.published})).await["type"], "resync");
    assert_eq!(command(&mut owner, json!({"type":"join","resource":"jobs","id":f.draft})).await["type"], "resync");
    assert_eq!(command(&mut stranger, json!({"type":"join","resource":"jobs"})).await["type"], "resync");
    assert_eq!(command(&mut owner, json!({"type":"join","resource":"jobs"})).await["type"], "resync");
    // A change to the hidden row reaches the relational owner and never the stranger's collection room.
    let client = reqwest::Client::new();
    let cookie = format!("{}={}", auth::COOKIE, f.admin.token);
    let write = |id: String, body: Value| {
        let (client, cookie) = (client.clone(), cookie.clone());
        async move {
            let response = client.patch(format!("http://{address}/api/jobs/{id}")).header("origin", ORIGIN).header("cookie", cookie).json(&body).send().await.unwrap();
            assert_eq!(response.status().as_u16(), 200);
        }
    };
    write(f.draft.clone(), json!({"pay":905})).await;
    assert_eq!(frame(&mut owner).await["id"], json!(f.draft));
    // Next event the stranger sees is the readable row: the hidden id was never delivered in between.
    write(f.published.clone(), json!({"pay":11})).await;
    let seen = frame(&mut stranger).await;
    assert_eq!(seen["id"], json!(f.published), "{seen}");
    assert!(seen.get("data").is_none());
    control.shutdown();
    task.abort();
    f.pool.close().await;
}

#[test]
fn openapi_documents_conditions_and_omits_server_only_resources() {
    let api = brain_backend::contract::openapi_with_roles(&resources(), &Default::default());
    let list = &api["paths"]["/api/jobs"]["get"];
    assert_eq!(list["x-brain-policy"][0], json!({"public":{"where":{"status":"published"}}}));
    assert_eq!(list["security"], json!([]));
    assert!(api["paths"].as_object().unwrap().keys().all(|path| !path.contains("ledgers")));
}

#[test]
fn contract_validation_explains_every_bad_condition() {
    let policy = |p: Value| serde_json::from_value::<Policy>(p).map_err(|e| e.to_string());
    let write = policy(json!({"read":["admin"],"create":["admin"],"update":[{"owner":{"where":{"status":"open"}}}],"delete":["admin"]})).unwrap_err();
    assert!(write.contains("update policy cannot use `where`") && write.contains("fix:"), "{write}");
    assert!(policy(json!({"read":[{"public":{"where":{}}}],"create":[],"update":[],"delete":[]})).unwrap_err().contains("fix:"));
    assert!(policy(json!({"read":["public",{"public":{"where":{"status":"x"}}}],"create":[],"update":[],"delete":[]})).unwrap_err().contains("twice"));
    assert!(policy(json!({"read":["admin"]})).unwrap_err().contains("create list"));
    // The source syntax round-trips, and a plain policy serializes exactly as before.
    let conditioned = json!({"read":[{"public":{"where":{"status":"published"}}},"admin"],"create":["admin"],"update":["admin"],"delete":["admin"]});
    assert_eq!(serde_json::to_value(policy(conditioned.clone()).unwrap()).unwrap(), conditioned);

    let resource = |read: Value, extra: Value| {
        let mut p = json!({"read":read,"create":["admin"],"update":["admin"],"delete":["admin"]});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let r: Resource = serde_json::from_value(json!({"name":"posts","fields":{"status":{"kind":"enum(draft|live)","required":true},"note":{"kind":"str"},"tags":{"kind":"list<str,3>"},"blob":{"kind":"obj"}},"policy":p})).unwrap();
        r.validate().map(|_| ()).map_err(|e| e.to_string())
    };
    let cond = |field: &str, value: Value| json!([{"public":{"where":{field:value}}}]);
    resource(cond("status", json!("live")), json!({})).unwrap();
    assert!(resource(cond("status", json!(["draft", "live"])), json!({})).is_ok());
    assert!(resource(cond("note", Value::Null), json!({})).is_ok());
    let unknown = resource(cond("missing", json!("x")), json!({})).unwrap_err();
    assert!(unknown.contains("`missing` is not a declared field") && unknown.contains("fix:"), "{unknown}");
    let value = resource(cond("status", json!("gone")), json!({})).unwrap_err();
    assert!(value.contains("does not fit the field") && value.contains("fix:"), "{value}");
    assert!(resource(cond("status", Value::Null), json!({})).is_err(), "a required field is never null");
    assert!(resource(cond("blob", json!("x")), json!({})).unwrap_err().contains("scalar"));
    assert!(resource(json!(["public"]), json!({"api":false})).unwrap_err().contains("server-only"));
    assert!(resource(cond("status", json!("live")), json!({"expose_all_rows":true})).unwrap_err().contains("expose_all_rows"));
    assert!(resource(json!(["public"]), json!({"expose_all_rows":true})).is_ok());

    // An API-visible resource cannot include a server-only one.
    let mut all = resources();
    all[2].query.include = vec!["ledger".into()];
    all[2].fields.insert("ledger".into(), serde_json::from_value(json!({"kind":"ref:ledgers"})).unwrap());
    let graph = brain_backend::query::validate_graph(&all[2], &all).unwrap_err();
    assert!(graph.contains("server-only"), "{graph}");
}
