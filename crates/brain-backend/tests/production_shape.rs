//! A production-shaped table: thousands of rows of long text. Search stays a client outcome for every hostile text, and
//! a plain list keeps using its index. Assertions read the planner's counters, never the clock.
use axum::{Router, body::{Body, to_bytes}, http::Request};
use brain_backend::{auth::{Auth, Credentials}, auth_flows::RolesConfig, contract::{Field, Hooks, Policy, Resource}, db};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::str::FromStr;
use tower::ServiceExt;
use uuid::Uuid;

const ROWS: i32 = 6000;
const ORIGIN: &str = "http://localhost:5190";

fn items() -> Resource {
    let field = |kind: &str| Field { kind: kind.into(), ..Default::default() };
    Resource {
        name: "items".into(),
        search: vec!["title".into(), "summary".into(), "body".into()],
        fields: [("title", field("str")), ("summary", field("str")), ("body", field("str")), ("state", field("enum(open|closed)")), ("rank", field("int"))].into_iter().map(|(n, f)| (n.to_string(), f)).collect(),
        policy: serde_json::from_value::<Policy>(json!({"read": ["public"], "create": ["user"], "update": ["owner"], "delete": ["owner"]})).unwrap(),
        hooks: Hooks::default(),
        constraints: Default::default(),
        query: serde_json::from_value(json!({"filter": {"state": ["eq"]}, "sort": ["rank"], "search": {"language": "english", "weights": {"title": "A", "summary": "B", "body": "C"}}})).unwrap(),
    }
}
async fn database() -> sqlx::PgPool {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_production_shape_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    base.close().await;
    db::connect(sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name).to_url_lossy().as_str()).await.unwrap()
}
async fn get(http: &Router, uri: &str) -> (u16, Value) {
    let request = Request::builder().extension(axum::extract::ConnectInfo("127.0.0.1:3456".parse::<std::net::SocketAddr>().unwrap())).uri(uri).header("origin", ORIGIN).body(Body::empty()).unwrap();
    let response = http.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    (status, serde_json::from_slice(&to_bytes(response.into_body(), 1 << 24).await.unwrap()).unwrap_or(Value::Null))
}
#[derive(Debug, PartialEq, Clone, Copy)]
struct Counters { seq_scan: i64, seq_read: i64, idx_scan: i64, idx_fetch: i64 }
async fn counters(admin: &mut sqlx::PgConnection) -> Counters {
    let (seq_scan, seq_read, idx_scan, idx_fetch): (i64, i64, i64, i64) = sqlx::query_as("SELECT seq_scan,seq_tup_read,coalesce(idx_scan,0),coalesce(idx_tup_fetch,0) FROM pg_stat_user_tables WHERE relname='_brain_res_items'").fetch_one(admin).await.unwrap();
    Counters { seq_scan, seq_read, idx_scan, idx_fetch }
}
/// A backend publishes its table counters at the latest when it exits. Ending the idle connections makes the read exact,
/// with no waiting on a statistics interval; the pool opens new ones. `admin` stays out of the pool, so it survives.
async fn settled(admin: &mut sqlx::PgConnection) -> Counters {
    let ended: Vec<i32> = sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid() AND backend_type='client backend'").fetch_all(&mut *admin).await.unwrap();
    sqlx::query("SELECT pg_terminate_backend(pid) FROM unnest($1::int[]) pid").bind(&ended).execute(&mut *admin).await.unwrap();
    for _ in 0..300 {
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE pid=ANY($1)").bind(&ended).fetch_one(&mut *admin).await.unwrap();
        if left == 0 { return counters(&mut *admin).await; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("The connections of the test database did not end within their bound");
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn long_text_tables_search_without_errors_and_list_through_their_index() {
    let pool = database().await;
    let resources = vec![items()];
    db::migrate(&pool, &resources).await.unwrap();
    RolesConfig::default().register(&pool).await.unwrap();
    let http = brain_backend::http::router(pool.clone(), brain_backend::Config {
        api_keys: None, roles: Default::default(), mail: None, auth_public_base_url: None, webhooks: None, before_hooks: None, sockets: None, gcs: None, jwt: None, google: None,
        resources, jobs: Vec::new(), uploads: std::env::temp_dir().join(format!("brain-production-shape-{}", Uuid::new_v4().simple())), origins: vec![ORIGIN.into()], production: false,
    }).await.unwrap();
    let author = Auth::new(pool.clone()).await.unwrap().signup(Credentials { email: "author@example.test".into(), password: "correct horse battery".into() }).await.unwrap();
    // 4 to 8 KB of incompressible text per row, with the words and symbols of the searches below.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL statement_timeout=0").execute(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO _brain_res_items(id,owner,data) SELECT gen_random_uuid(),$1,jsonb_build_object(\
         'title','Item '||n,'summary',CASE WHEN n%7=0 THEN 'qa review' ELSE 'long record' END,'state',CASE WHEN n%2=0 THEN 'open' ELSE 'closed' END,'rank',n,\
         'body',CASE WHEN n%5=0 THEN 'react ' ELSE '' END||CASE WHEN n%11=0 THEN '100% done ' ELSE '' END||CASE WHEN n%13=0 THEN 'a\\b ' ELSE '' END||\
         (SELECT string_agg(md5((n*k)::text),' ') FROM generate_series(1,121+n%121) k)) FROM generate_series(1,$2) n",
    ).bind(author.id).bind(ROWS).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let (rows, shortest, longest): (i64, i32, i32) = sqlx::query_as("SELECT count(*),min(length(data->>'body')),max(length(data->>'body')) FROM _brain_res_items").fetch_one(&pool).await.unwrap();
    assert_eq!(rows, ROWS as i64);
    assert!(shortest >= 4000 && longest <= 8200, "control: the fixture has the production shape ({shortest}..{longest})");
    sqlx::query("ALTER TABLE _brain_res_items SET (autovacuum_enabled=false)").execute(&pool).await.unwrap();
    sqlx::query("VACUUM ANALYZE _brain_res_items").execute(&pool).await.unwrap();

    for (text, encoded) in [("react", "react"), ("qa", "qa"), ("100%", "100%25"), ("a\\b", "a%5Cb"), ("%%%", "%25%25%25"), ("\\\\\\", "%5C%5C%5C"), ("%", "%25"), ("\\", "%5C"), ("_", "_"), ("a", "a"), ("%a%", "%25a%25"), ("''", "%27%27"), ("react qa 100%", "react%20qa%20100%25"), ("zzzzqqq", "zzzzqqq")] {
        for extra in ["", "&state=open", "&sort=rank", "&sort=-rank&limit=100"] {
            let uri = format!("/api/items?search={encoded}{extra}");
            let (status, body) = get(&http, &uri).await;
            // A database deadline under load is a defined retry; only a 5xx without that meaning is a defect.
            let busy = status == 503 && body["error"]["code"] == "DATABASE_BUSY";
            assert!(status == 200 || busy, "search {text:?}{extra}: {status} {body}");
            if status == 200 { assert!(body["items"].is_array(), "search {text:?}{extra}: status {status}"); }
        }
    }
    let (status, found) = get(&http, "/api/items?search=react").await;
    assert_eq!((status, found["total"].clone()), (200, json!(ROWS / 5)), "control: full-text search reads the long field to its end {found}");

    let mut admin = pool.acquire().await.unwrap().detach();
    // Plain lists: the first page never walks the table, whatever its size.
    for (uri, page) in [("/api/items", 25), ("/api/items?limit=20&sort=-created_at", 20), ("/api/items?limit=20&sort=created_at", 20), ("/api/items?limit=20&sort=rank", 20)] {
        let before = settled(&mut admin).await;
        for _ in 0..3 {
            let (status, body) = get(&http, uri).await;
            assert_eq!(status, 200, "{uri} {body}");
            assert_eq!(body["items"].as_array().unwrap().len(), page, "{uri}");
        }
        let after = settled(&mut admin).await;
        assert_eq!((after.seq_scan - before.seq_scan, after.seq_read - before.seq_read), (0, 0), "{uri} scanned the table: {before:?} -> {after:?}");
        assert!(after.idx_scan - before.idx_scan >= 3, "{uri} used no index: {before:?} -> {after:?}");
        assert!(after.idx_fetch - before.idx_fetch <= 3 * (page as i64 + 1) * 2, "{uri} read far more rows than its page: {before:?} -> {after:?}");
    }
    pool.close().await;
}
