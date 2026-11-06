//! Anonymous aggregate ingestion, transactional capacity and read-only reports on an owned database.
use brain_backend::{db, vitals::{self, Batch, Config, Metric}};
use serde_json::json;
use sqlx::{ConnectOptions, Row};
use std::{str::FromStr, time::Duration};
use uuid::Uuid;
fn config() -> Config { Config::from_value(Some(&json!({"enabled":true,"routes":["/","/items/:id"],"retentionDays":30}))).unwrap().unwrap() }
fn batch(name: &str, value: f64) -> Batch { Batch { route:"/items/:id".into(),consent:true,metrics:vec![Metric{name:name.into(),value}] } }
#[test]
fn configuration_and_payloads_are_strict_and_bounded() {
    assert!(Config::from_value(None).unwrap().is_none()); assert!(Config::from_value(Some(&json!(false))).unwrap().is_none());
    for value in [json!(true),json!({"enabled":true}),json!({"enabled":true,"routes":["/items/42?email=private"]}),json!({"enabled":true,"routes":["/","/"]}),json!({"enabled":true,"routes":["/"],"retentionDays":91}),json!({"enabled":true,"routes":["/"],"user":"private"})] { assert!(Config::from_value(Some(&value)).is_err()); }
    for route in ["//example.test","/email@example.test","/x/..","/x/","/x?secret=1","/x%20","/:1id",""] { assert!(!vitals::valid_route(route),"{route}"); }
    for route in ["/","/items/:id","/item-list/:item_id"] { assert!(vitals::valid_route(route)); }
    let c=config();
    for (name,max) in [("LCP",3600000.),("CLS",1000.),("INP",3600000.),("FCP",3600000.),("TTFB",3600000.)] {
        assert!(batch(name,0.).validate(&c).is_ok());assert!(batch(name,max).validate(&c).is_ok());
        for value in [-1.,max+1.,f64::NAN,f64::INFINITY] { assert_eq!(batch(name,value).validate(&c),Err(vitals::Error::Invalid)); }
    }
    let mut b=batch("TTFB",1.);b.metrics.push(Metric{name:"TTFB".into(),value:2.});assert_eq!(b.validate(&c),Err(vitals::Error::Invalid));
    for body in [r#"{"route":"/","consent":"true","metrics":[]}"#,r#"{"route":"/","consent":true,"metrics":[{"name":"TTFB","value":2,"url":"private"}]}"#,r#"{"route":"/","consent":true,"metrics":[],"url":"private"}"#,r#"{"route":"/","consent":false,"consent":true,"metrics":[]}"#] { assert!(serde_json::from_str::<Batch>(body).is_err()); }
}
// A real second executable process, not another task sharing a process-local limiter.
#[test]
fn isolated_process_ingestion() {
    let Ok(url)=std::env::var("BRAIN_NATIVE_VITALS_CHILD_URL") else { return };
    let name=std::env::var("BRAIN_NATIVE_VITALS_CHILD_METRIC").unwrap();
    let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let pool=db::connect(&url).await.unwrap();
        let result=vitals::ingest(&pool,&config(),batch(&name,42.)).await;
        assert!(matches!(result,Ok(())|Err(vitals::Error::RateLimited)|Err(vitals::Error::Capacity)),"{result:?}");
        assert!(db::close_until(&pool,tokio::time::Instant::now()+Duration::from_secs(2)).await);
    });
}
fn http_config(vitals: Option<Config>) -> brain_backend::Config {
    brain_backend::Config { vitals,resources:vec![],jobs:vec![],uploads:std::env::temp_dir().join(format!("brain-vitals-http-{}",Uuid::new_v4().simple())),origins:vec!["https://example.test".into()],production:true,jwt:None,google:None,apple:None,apple_native:None,linkedin:None,gcs:None,roles:Default::default(),mail:None,auth_public_base_url:None,webhooks:None,before_hooks:None,sockets:None,api_keys:None }
}
async fn post(router:&axum::Router,body:serde_json::Value,headers:&[(&str,&str)]) -> (u16,serde_json::Value) {
    use tower::ServiceExt;
    use http_body_util::BodyExt;
    let mut request=axum::http::Request::post("/_brain/vitals").header("content-type","application/json");
    if !headers.iter().any(|(key,_)|*key=="origin") { request=request.header("origin","https://example.test"); }
    for (key,value) in headers { request=request.header(*key,*value); }
    let mut request=request.body(axum::body::Body::from(body.to_string())).unwrap();
    request.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127,0,0,1],9))));
    let response=router.clone().oneshot(request).await.unwrap();let status=response.status().as_u16();
    let body=response.into_body().collect().await.unwrap().to_bytes();
    (status,serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
}
fn wait(child: &mut std::process::Child) -> bool {
    let deadline=std::time::Instant::now()+Duration::from_secs(20);
    loop {
        if let Some(status)=child.try_wait().unwrap() { return status.success() }
        if std::time::Instant::now()>=deadline { let _=child.kill();let _=child.wait();return false }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn child(url:&str,metric:&str)->std::process::Child {
    std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","vitals::isolated_process_ingestion","--nocapture"])
        .env("BRAIN_NATIVE_VITALS_CHILD_URL",url).env("BRAIN_NATIVE_VITALS_CHILD_METRIC",metric).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).spawn().unwrap()
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="requires an explicitly owned disposable PostgreSQL cluster"]
async fn owned_postgres_aggregates_capacity_rate_privacy_retention_and_read_only_reports() {
    let url=std::env::var("BRAIN_BACKEND_TEST_URL").expect("explicit owned cluster required");
    let base=db::connect(&url).await.unwrap();let name=format!("brain_vitals_{}",Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    let options=sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name);
    let owned_url=options.to_url_lossy().to_string();let pool=db::connect(&owned_url).await.unwrap();db::migrate(&pool,&[]).await.unwrap();
    let c=config();
    let disabled=brain_backend::http::router(pool.clone(),http_config(None)).await.unwrap();
    let payload=json!({"route":"/items/:id","consent":true,"metrics":[{"name":"TTFB","value":800}]});
    assert_eq!(post(&disabled,payload.clone(),&[]).await.0,404);
    let (router,_,edge)=brain_backend::http::router_full(pool.clone(),http_config(Some(c.clone())),Default::default(),Default::default()).await.unwrap();
    let mut invalid=payload.clone();invalid["consent"]=json!(false);assert_eq!(post(&router,invalid,&[]).await.0,400);
    let mut invalid=payload.clone();invalid["route"]=json!("/items/private-identifier");assert_eq!(post(&router,invalid,&[]).await.0,400);
    let mut invalid=payload.clone();invalid["metrics"][0]["value"]=json!(3600001);assert_eq!(post(&router,invalid,&[]).await.0,400);
    assert_eq!(post(&router,payload.clone(),&[("origin","https://foreign.example.test")]).await.0,403);
    assert_eq!(post(&router,payload.clone(),&[("authorization","Bearer synthetic-unrelated-credential"),("cookie","__Host-brain=invalid-session")]).await.0,204);
    // The HTTP deadline includes a body that never completes, not only database execution.
    {
        use tower::ServiceExt;
        use http_body_util::BodyExt;
        let stream=futures_util::stream::pending::<Result<axum::body::Bytes,std::convert::Infallible>>();
        let mut request=axum::http::Request::post("/_brain/vitals").header("origin","https://example.test").header("content-type","application/json").body(axum::body::Body::from_stream(stream)).unwrap();
        request.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127,0,0,1],9))));
        let response=tokio::time::timeout(Duration::from_secs(6),router.clone().oneshot(request)).await.unwrap().unwrap();assert_eq!(response.status().as_u16(),503);
        let bytes=response.into_body().collect().await.unwrap().to_bytes();let value:serde_json::Value=serde_json::from_slice(&bytes).unwrap();assert_eq!(value["error"]["code"],"VITALS_UNAVAILABLE");
    }
    // A held capacity lock is cancelled when the native lifecycle starts draining.
    let mut blocked=pool.begin().await.unwrap();sqlx::query("SELECT rows FROM _brain_vitals_capacity WHERE singleton=true FOR UPDATE").fetch_one(&mut *blocked).await.unwrap();
    let pending_router=router.clone();let pending_body=payload.clone();
    let pending=tokio::spawn(async move {post(&pending_router,pending_body,&[]).await});
    tokio::time::sleep(Duration::from_millis(60)).await;let stopped=std::time::Instant::now();edge.drain();
    assert_eq!(tokio::time::timeout(Duration::from_millis(150),pending).await.unwrap().unwrap().0,503);assert!(stopped.elapsed()<Duration::from_millis(150));blocked.rollback().await.unwrap();drop(router);drop(disabled);
    let privacy_pool=db::connect(&owned_url).await.unwrap();let privacy=brain_backend::http::router(privacy_pool.clone(),http_config(Some(c.clone()))).await.unwrap();privacy_pool.close().await;
    for signal in ["dnt","sec-gpc"] { assert_eq!(post(&privacy,payload.clone(),&[(signal,"1")]).await.0,204,"privacy must drop before a closed database is consulted"); }
    assert_eq!(post(&privacy,payload.clone(),&[]).await.0,503);drop(privacy);
    sqlx::raw_sql("TRUNCATE _brain_vitals_daily; UPDATE _brain_vitals_capacity SET rows=0; DELETE FROM _brain_rate_windows WHERE scope='native-vitals';").execute(&pool).await.unwrap();
    let mut denied=batch("TTFB",1.);denied.consent=false;assert_eq!(vitals::ingest(&pool,&c,denied).await,Err(vitals::Error::Invalid));
    let windows:i64=sqlx::query_scalar("SELECT count(*) FROM _brain_rate_windows WHERE scope='native-vitals'").fetch_one(&pool).await.unwrap();assert_eq!(windows,0);
    vitals::ingest(&pool,&c,batch("TTFB",400.)).await.unwrap();vitals::ingest(&pool,&c,batch("TTFB",1000.)).await.unwrap();vitals::ingest(&pool,&c,batch("TTFB",2000.)).await.unwrap();
    for _ in 0..20 { vitals::ingest(&pool,&c,batch("CLS",0.1)).await.unwrap(); }
    let before: (i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM _brain_analytics_visitors),(SELECT count(*) FROM _brain_analytics_sessions)").fetch_one(&pool).await.unwrap();assert_eq!(before,(0,0));
    let report=vitals::report(&pool,&c,7,1,0).await.unwrap();assert_eq!(report["percentileAvailable"],false);assert_eq!(report["readOnly"],true);assert_eq!(report["nextOffset"],1);assert!(!report.to_string().contains("private"));
    let report=vitals::report(&pool,&c,7,500,0).await.unwrap();let group=report["groups"].as_array().unwrap().iter().find(|g|g["metric"]=="TTFB").unwrap();assert_eq!(group["count"],3);assert_eq!(group["mean"],3400./3.);assert_eq!(group["categories"]["good"]["count"],1);assert_eq!(group["categories"]["needsImprovement"]["count"],1);assert_eq!(group["categories"]["poor"]["count"],1);
    assert_eq!(vitals::report(&pool,&c,91,1,0).await,Err(vitals::Error::Invalid));
    let used:i64=sqlx::query_scalar("SELECT rows FROM _brain_vitals_capacity").fetch_one(&pool).await.unwrap();assert_eq!(used,2);
    // Expiration is hidden by read-only reports before lazy physical removal runs.
    sqlx::query("UPDATE _brain_vitals_daily SET day=(CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date-31").execute(&pool).await.unwrap();
    assert!(vitals::report(&pool,&c,90,500,0).await.unwrap()["groups"].as_array().unwrap().is_empty());
    let unchanged:i64=sqlx::query_scalar("SELECT rows FROM _brain_vitals_capacity").fetch_one(&pool).await.unwrap();assert_eq!(unchanged,2);
    vitals::ingest(&pool,&c,batch("TTFB",1.)).await.unwrap();let used:i64=sqlx::query_scalar("SELECT rows FROM _brain_vitals_capacity").fetch_one(&pool).await.unwrap();assert_eq!(used,1);
    // Quota is one atomic shared fixed subject, with two independent backend executable processes.
    sqlx::query("UPDATE _brain_rate_windows SET hits=599,window_start=now() WHERE scope='native-vitals'").execute(&pool).await.unwrap();
    let mut first=child(&owned_url,"FCP");let mut second=child(&owned_url,"INP");let a=wait(&mut first);let b=wait(&mut second);assert!(a&&b);
    let hits:i64=sqlx::query_scalar("SELECT hits FROM _brain_rate_windows WHERE scope='native-vitals'").fetch_one(&pool).await.unwrap();assert_eq!(hits,601);
    let metrics:i64=sqlx::query_scalar("SELECT count(*) FROM _brain_vitals_daily WHERE metric IN ('FCP','INP')").fetch_one(&pool).await.unwrap();assert_eq!(metrics,1);
    sqlx::query("DELETE FROM _brain_rate_windows WHERE scope='native-vitals'").execute(&pool).await.unwrap();
    sqlx::raw_sql("TRUNCATE _brain_vitals_daily; UPDATE _brain_vitals_capacity SET rows=0;").execute(&pool).await.unwrap();
    // Historical declarations can exceed the current 512-pattern declaration. Populate real rows up to cap-1.
    sqlx::query("INSERT INTO _brain_vitals_daily(day,route_pattern,metric,count,total,min,max,good,needs_improvement,poor) SELECT (CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date,'/history-'||n,'TTFB',1,1,1,1,1,0,0 FROM generate_series(1,249999) n").execute(&pool).await.unwrap();
    sqlx::query("UPDATE _brain_vitals_capacity SET rows=249999").execute(&pool).await.unwrap();
    let mut first=child(&owned_url,"LCP");let mut second=child(&owned_url,"INP");let a=wait(&mut first);let b=wait(&mut second);assert!(a&&b);
    let actual:i64=sqlx::query_scalar("SELECT count(*) FROM _brain_vitals_daily").fetch_one(&pool).await.unwrap();let recorded:i64=sqlx::query_scalar("SELECT rows FROM _brain_vitals_capacity").fetch_one(&pool).await.unwrap();assert_eq!(actual,vitals::ROW_CAP);assert_eq!(actual,recorded);
    assert_eq!(vitals::ingest(&pool,&c,batch("FCP",1.)).await,Err(vitals::Error::Capacity));
    let quota:i64=sqlx::query_scalar("SELECT hits FROM _brain_rate_windows WHERE scope='native-vitals'").fetch_one(&pool).await.unwrap();assert_eq!(quota,3,"failed capacity still consumes shared quota");
    // Pool restart preserves aggregates and counter; no raw-sample table or behavior identities are introduced.
    assert!(db::close_until(&pool,tokio::time::Instant::now()+Duration::from_secs(2)).await);let pool=db::connect(&owned_url).await.unwrap();
    let recorded:i64=sqlx::query_scalar("SELECT rows FROM _brain_vitals_capacity").fetch_one(&pool).await.unwrap();assert_eq!(recorded,vitals::ROW_CAP);
    let columns:Vec<String>=sqlx::query("SELECT column_name FROM information_schema.columns WHERE table_name='_brain_vitals_daily' ORDER BY ordinal_position").fetch_all(&pool).await.unwrap().into_iter().map(|r|r.get("column_name")).collect();assert_eq!(columns,vec!["day","route_pattern","metric","count","total","min","max","good","needs_improvement","poor"]);
    assert!(db::close_until(&pool,tokio::time::Instant::now()+Duration::from_secs(2)).await);sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}"))).execute(&base).await.unwrap();base.close().await;
}
