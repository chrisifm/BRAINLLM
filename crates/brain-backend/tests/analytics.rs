//! Behavior analytics: privacy filters (pure) and real owned-Postgres ingestion, queries, retention and erasure.
use axum::{body::Body, extract::ConnectInfo, http::Request};
use brain_backend::{
    analytics::{self, Segment},
    auth::{Auth, Credentials},
    db,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::str::FromStr;
use tower::ServiceExt;
use uuid::Uuid;

#[test]
fn routes_lose_queries_and_identifier_segments() {
    assert_eq!(analytics::clean_route("/orders/42?email=a@b.c#x").as_deref(), Some("/orders/:id"));
    assert_eq!(
        analytics::clean_route("/u/1f0e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b/edit").as_deref(),
        Some("/u/:id/edit")
    );
    assert_eq!(analytics::clean_route("/reset/a8Fk2LmQ9zXw7Rt5Yp3").as_deref(), Some("/reset/:id"));
    assert_eq!(analytics::clean_route("/pricing").as_deref(), Some("/pricing"));
    assert_eq!(analytics::clean_route("https://evil.test/"), None);
    assert_eq!(analytics::clean_route("//evil.test/"), None);
}

#[test]
fn element_names_mask_contact_data_and_unknown_keys_are_dropped() {
    assert_eq!(analytics::clean_text("Email jane@example.test now", 64), "Email @ now");
    assert_eq!(analytics::clean_text("Card 4111 1111 1111 1111", 64), "Card #");
    assert_eq!(analytics::clean_text("Order  A-1234", 64), "Order A-1234");
    let el = json!({"c":"ab12","t":"BUTTON","n":"Buy  now","i":2,"value":"secret","x":"buy-plan"});
    let cleaned = analytics::clean_element(el.as_object().unwrap()).unwrap();
    assert_eq!(cleaned, json!({"c":"ab12","t":"button","n":"Buy now","i":2,"x":"buy-plan"}));
    assert!(analytics::clean_element(json!({"n":"no tag"}).as_object().unwrap()).is_none());
}

#[test]
fn automated_clients_and_missing_user_agents_are_excluded() {
    assert!(analytics::is_bot(None));
    assert!(analytics::is_bot(Some("Mozilla/5.0 (compatible; Googlebot/2.1)")));
    assert!(analytics::is_bot(Some("Mozilla/5.0 HeadlessChrome/131.0")));
    assert!(analytics::is_bot(Some("curl/8.4.0")));
    assert!(!analytics::is_bot(Some("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15")));
}

#[test]
fn configuration_bounds_are_enforced() {
    assert!(analytics::Config { retention_days: 0, ..Default::default() }.validate().is_err());
    assert!(analytics::Config { country_header: Some("CF IPCountry".into()), ..Default::default() }.validate().is_err());
    assert!(analytics::Config { country_header: Some("cf-ipcountry".into()), ..Default::default() }.validate().is_ok());
    let limiter = analytics::Limiter::new(10);
    let v = Uuid::new_v4();
    assert!(limiter.admit(v, 8));
    assert!(!limiter.admit(v, 3));
    assert!(limiter.admit(Uuid::new_v4(), 10));
}

const BROWSER: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15";
const ORIGIN: &str = "https://shop.example.test";

async fn post(router: &axum::Router, body: Value, extra: &[(&str, &str)]) -> (u16, Value) {
    let mut req = Request::post("/_brain/analytics")
        .header("origin", ORIGIN)
        .header("content-type", "text/plain;charset=UTF-8");
    if !extra.iter().any(|(k, _)| *k == "user-agent") {
        req = req.header("user-agent", BROWSER);
    }
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let mut req = req.body(Body::from(body.to_string())).unwrap();
    req.extensions_mut().insert(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 9))));
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn page(route: &str, pv: u32) -> Value {
    json!({"k":"page","r":route,"pv":pv,"t":0,"at":0})
}

#[tokio::test]
#[ignore = "requires an explicitly owned disposable PostgreSQL cluster"]
async fn ingestion_reports_segments_exports_and_retention_on_owned_postgres() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("explicit owned test cluster required");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_analytics_{}", Uuid::new_v4().simple());
    // Audited identifier: fixed ASCII prefix and UUID hex, with no request data.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name);
    let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
    db::migrate(&pool, &[]).await.unwrap();
    db::migrate(&pool, &[]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let shopper = auth
        .signup(Credentials { email: "shopper@example.test".into(), password: "Synthetic shopper password".into() })
        .await
        .unwrap();
    let files = std::env::temp_dir().join(format!("brain-analytics-{}", Uuid::new_v4().simple()));
    let config = brain_backend::Config {
        api_keys: None,
        resources: vec![],
        jobs: vec![],
        uploads: files.clone(),
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
    };
    let router = brain_backend::http::router(pool.clone(), config).await.unwrap();

    // Anonymous visitor A: pricing twice, never checkout; clicks, a rage burst and a custom event.
    let a = Uuid::new_v4();
    let a1 = Uuid::new_v4();
    let el = json!({"c":"ab12","t":"button","n":"Choose plan","i":0,"x":"choose-plan"});
    let first = json!({"v":1,"vid":a,"sid":a1,
        "ctx":{"ref":"https://news.example.test/post?id=7","utm":{"source":"newsletter","campaign":"autumn","secret":"x"},"lang":"en-US","w":1280},
        "ev":[page("/",1), page("/pricing?plan=pro",2),
              {"k":"click","r":"/pricing","pv":2,"t":900,"at":0,"el":el},
              {"k":"input","r":"/pricing","pv":2,"t":1200,"at":0,"el":{"c":"ab12","t":"input","f":"email","y":"email"},"p":{"changed":true}},
              {"k":"rage","r":"/pricing","pv":2,"t":1500,"at":0,"el":el,"v":4},
              {"k":"track","r":"/pricing","pv":2,"t":1600,"at":0,"p":{"name":"plan_compare","plan":"pro"}},
              {"k":"leave","r":"/pricing","pv":2,"t":5000,"at":0,"v":5000,"p":{"scroll":80}},
              {"k":"unknown","r":"/pricing","pv":2,"t":1,"at":0}]});
    let (status, _) = post(&router, first, &[]).await;
    assert_eq!(status, 204);
    let (status, _) = post(&router, json!({"v":1,"vid":a,"sid":Uuid::new_v4(),"ev":[page("/pricing",1)]}), &[]).await;
    assert_eq!(status, 204);

    // Visitor B signs in and completes checkout.
    let b = Uuid::new_v4();
    let login = auth
        .login(Credentials { email: "shopper@example.test".into(), password: "Synthetic shopper password".into() })
        .await
        .unwrap()
        .unwrap();
    let cookie = format!("__Host-brain={}", login.1);
    let (status, _) = post(
        &router,
        json!({"v":1,"vid":b,"sid":Uuid::new_v4(),"ev":[page("/pricing",1),page("/checkout",2),
            {"k":"submit","r":"/checkout","pv":2,"t":3000,"at":0,"el":{"c":"cd34","t":"form","n":"Checkout"},"p":{"outcome":"sent"}}]}),
        &[("cookie", &cookie)],
    )
    .await;
    assert_eq!(status, 204);

    // Filters: privacy signals and automated clients are acknowledged but never stored.
    let ignored = Uuid::new_v4();
    for headers in [vec![("sec-gpc", "1")], vec![("dnt", "1")], vec![("user-agent", "Googlebot/2.1")]] {
        let (status, _) = post(&router, json!({"v":1,"vid":ignored,"sid":Uuid::new_v4(),"ev":[page("/",1)]}), &headers).await;
        assert_eq!(status, 204);
    }
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_analytics_events WHERE visitor=$1")
        .bind(ignored).fetch_one(&pool).await.unwrap();
    assert_eq!(stored, 0, "privacy signals and bots are never stored");

    // Validation: shape, version, size and session ownership.
    assert_eq!(post(&router, json!({"v":1,"vid":a,"sid":a1}), &[]).await.0, 400);
    assert_eq!(post(&router, json!({"v":2,"vid":a,"sid":a1,"ev":[page("/",1)]}), &[]).await.0, 400);
    let too_many: Vec<Value> = (0..65).map(|i| page("/", i)).collect();
    assert_eq!(post(&router, json!({"v":1,"vid":a,"sid":a1,"ev":too_many}), &[]).await.0, 400);
    let (status, body) = post(&router, json!({"v":1,"vid":Uuid::new_v4(),"sid":a1,"ev":[page("/",1)]}), &[]).await;
    assert_eq!((status, body["error"]["code"].as_str()), (409, Some("ANALYTICS_SESSION_CONFLICT")));
    let mut no_origin = Request::post("/_brain/analytics").header("user-agent", BROWSER).body(Body::from("{}")).unwrap();
    no_origin.extensions_mut().insert(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 9))));
    assert_eq!(router.clone().oneshot(no_origin).await.unwrap().status().as_u16(), 403);
    let mut script = Request::get("/_brain/analytics.js").body(Body::empty()).unwrap();
    script.extensions_mut().insert(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 9))));
    let script = router.clone().oneshot(script).await.unwrap();
    assert_eq!(script.status().as_u16(), 200);
    assert!(script.headers()["content-type"].to_str().unwrap().starts_with("text/javascript"));

    // Stored shape: no query string, no unknown kinds, no UTM keys outside the allowlist, no referrer path.
    let routes: Vec<String> = sqlx::query_scalar("SELECT DISTINCT route FROM _brain_analytics_events ORDER BY 1")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(routes, vec!["/", "/checkout", "/pricing"]);
    let (referrer, utm, device): (Option<String>, Value, Option<String>) = sqlx::query_as(
        "SELECT referrer_host,utm,device FROM _brain_analytics_sessions WHERE id=$1").bind(a1).fetch_one(&pool).await.unwrap();
    assert_eq!(referrer.as_deref(), Some("news.example.test"));
    assert_eq!(utm, json!({"source":"newsletter","campaign":"autumn"}));
    assert_eq!(device.as_deref(), Some("desktop"));
    let linked: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM _brain_analytics_visitors WHERE id=$1")
        .bind(b).fetch_one(&pool).await.unwrap();
    assert_eq!(linked, Some(shopper.id), "a signed-in batch links the visitor to the account");

    // Report.
    let report = analytics::report(&pool, 7).await.unwrap();
    assert_eq!(report["totals"]["visitors"], 2);
    let pricing = report["routes"].as_array().unwrap().iter().find(|r| r["route"] == "/pricing").unwrap();
    assert_eq!(pricing["views"], 3);
    assert_eq!(pricing["visitors"], 2);
    assert_eq!(pricing["avgDwellMs"], 5000);
    assert_eq!(pricing["avgScrollPct"], 80);
    assert_eq!(pricing["medianTimeToFirstInteractionMs"], 900);
    assert_eq!(pricing["rageClicks"], 1);
    let button = report["elements"].as_array().unwrap().iter().find(|e| e["element"]["x"] == "choose-plan").unwrap();
    assert_eq!(button["interactions"], 1);
    assert_eq!(button["rageClicks"], 1);
    assert!(report["transitions"].as_array().unwrap().iter().any(|t| t["from"] == "/" && t["to"] == "/pricing"));
    assert!(report["sources"].as_array().unwrap().iter().any(|s| s["source"] == "newsletter" && s["campaign"] == "autumn"));
    assert_eq!(report["tracked"][0]["name"], "plan_compare");

    // Funnel and segments (internal retargeting).
    let steps = analytics::funnel(&pool, &["/pricing".into(), "/checkout".into()], 7).await.unwrap();
    assert_eq!((steps[0].visitors, steps[1].visitors), (2, 1));
    let hesitant = analytics::segment(&pool, &Segment {
        viewed: "/pricing".into(), min_views: Some(2), within_days: Some(7), not_viewed: Some("/checkout".into()), ..Default::default()
    }).await.unwrap();
    assert_eq!(hesitant.len(), 1);
    assert_eq!((hesitant[0].visitor, hesitant[0].user_id, hesitant[0].views), (a, None, 2));
    let buyers = analytics::segment(&pool, &Segment { viewed: "/check*".into(), signed_in: Some(true), ..Default::default() })
        .await.unwrap();
    assert_eq!(buyers.iter().map(|m| m.user_id).collect::<Vec<_>>(), vec![Some(shopper.id)]);
    let comparers = analytics::segment(&pool, &Segment { viewed: "/pricing".into(), tracked: Some("plan_compare".into()), ..Default::default() })
        .await.unwrap();
    assert_eq!(comparers.len(), 1);

    // Export for audits.
    let csv = analytics::export(&pool, 7, 1000, true).await.unwrap();
    assert!(csv.starts_with("at,visitor,session,user_id,page,since_ms,kind,route,element,value,props\n"));
    assert!(!csv.contains("secret") && !csv.contains("plan=pro") && !csv.contains("news.example.test/post"));
    let exported: Value = serde_json::from_str(&analytics::export(&pool, 7, 2, false).await.unwrap()).unwrap();
    assert_eq!(exported["events"].as_array().unwrap().len(), 2);

    // Retention and erasure.
    sqlx::query("UPDATE _brain_analytics_events SET at=now()-interval '200 days' WHERE visitor=$1").bind(a).execute(&pool).await.unwrap();
    sqlx::query("UPDATE _brain_analytics_sessions SET last_seen_at=now()-interval '200 days' WHERE visitor=$1").bind(a).execute(&pool).await.unwrap();
    sqlx::query("UPDATE _brain_analytics_visitors SET last_seen_at=now()-interval '200 days' WHERE id=$1").bind(a).execute(&pool).await.unwrap();
    let purged = analytics::purge(&pool, 180).await.unwrap();
    assert_eq!((purged.events, purged.sessions, purged.visitors), (8, 2, 1));
    assert!(analytics::erase_user(&pool, shopper.id).await.unwrap() >= 3);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_analytics_events").fetch_one(&pool).await.unwrap();
    assert_eq!(left, 0);
    pool.close().await;
    let _ = std::fs::remove_dir_all(files);
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {name} WITH (FORCE)"))).execute(&base).await.unwrap();
}

#[test]
fn postgres_sessions_carry_a_bounded_application_name() {
    use brain_backend::db::application_name;
    assert_eq!(application_name("shop", "http"), "brain:shop:http");
    assert_eq!(application_name("my shop", "worker:emails"), "brain:my-shop:worker:emails");
    assert_eq!(application_name("", "migrate"), "brain:app:migrate");
    assert!(application_name(&"x".repeat(200), "http").len() <= 63);
}

#[tokio::test]
#[ignore = "needs BRAIN_BACKEND_TEST_URL"]
async fn named_connections_show_up_in_pg_stat_activity() {
    let Ok(url) = std::env::var("BRAIN_BACKEND_TEST_URL") else { return };
    let pool = brain_backend::db::connect_as(&url, "brain:shop:worker:emails").await.unwrap();
    let name: String = sqlx::query_scalar("SELECT application_name FROM pg_stat_activity WHERE pid = pg_backend_pid()")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(name, "brain:shop:worker:emails");
    let default: String = sqlx::query_scalar("SELECT current_setting('application_name')")
        .fetch_one(&brain_backend::db::connect(&url).await.unwrap()).await.unwrap();
    assert_eq!(default, "brain:app:http");
}

#[test]
fn collector_module_starts_with_settings_and_drops_comment_lines() {
    let on = analytics::collector_js(Some(&analytics::Config::default()));
    assert!(on.contains("export function start("));
    assert!(on.trim_end().ends_with("start({endpoint:new URL('analytics',import.meta.url).pathname})"));
    assert!(!on.lines().any(|l| l.trim_start().starts_with("//")));
    let consent = analytics::collector_js(Some(&analytics::Config { consent_required: true, ..Default::default() }));
    assert!(consent.contains(",consent:'required'})"));
    assert_eq!(analytics::collector_js(None), "export {}\n");
}
