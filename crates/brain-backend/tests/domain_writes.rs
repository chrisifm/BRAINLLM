//! Declarative domain writes: PATCH, multi-resource operations, atomic actions, upserts, batches and state
//! transitions. Unit checks run everywhere; the Postgres test runs through scripts/backend-check.mjs.
use axum::body::{Body, to_bytes};
use brain_backend::{
    auth::{Auth, Credentials},
    contract::Resource,
    db,
    domain::{self, Domain},
};
use axum::http::Request;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use tower::ServiceExt;

/// Contracts loaded by the CLI from `docs/backend/examples/shop` (kept equal by a brain-cli test).
fn fixture() -> Value {
    serde_json::from_str(include_str!("shop_contracts.json")).unwrap()
}
fn resources() -> Vec<Resource> {
    serde_json::from_value(fixture()["resources"].clone()).unwrap()
}
fn resource(name: &str) -> Resource {
    resources().into_iter().find(|r| r.name == name).unwrap()
}
fn products() -> Resource {
    resource("shop_products")
}
fn shop_domain() -> Domain {
    serde_json::from_value(fixture()["domain"].clone()).unwrap()
}

#[test]
fn patch_merges_only_sent_fields_and_rejects_unknown_ones() {
    let r = products();
    let stored = json!({"name":"Mug","sku":"M-1","stock":3,"price":"9.50","status":"active","note":"blue"});
    let merged = domain::merge_patch(&r, &stored, &json!({"stock":4,"note":null})).unwrap();
    assert_eq!(merged, json!({"name":"Mug","sku":"M-1","stock":4,"price":"9.50","status":"active"}));
    // The merged record goes through full validation: a cleared defaulted field gets its default again.
    let cleared = domain::merge_patch(&r, &stored, &json!({"price":null})).unwrap();
    assert_eq!(r.input(&cleared).unwrap()["price"], "0.00");
    assert_eq!(domain::merge_patch(&r, &stored, &json!({})).unwrap_err()["body"], "Send at least one declared field to change");
    assert_eq!(domain::merge_patch(&r, &stored, &json!({"colour":"red"})).unwrap_err()["colour"], "Unknown field");
    assert!(domain::merge_patch(&r, &stored, &json!([1])).is_err());
    let schema = domain::patch_schema(&r);
    assert_eq!(schema["required"], json!([]));
    assert_eq!(schema["minProperties"], 1);
    assert!(schema["properties"]["note"]["anyOf"].is_array(), "optional fields accept null");
    assert!(schema["properties"]["name"]["anyOf"].is_null(), "required fields cannot be cleared");
}

#[test]
fn domain_contracts_validate_with_actionable_messages() {
    let rs = resources();
    shop_domain().validate(&rs).unwrap();
    let broken = |patch: Value| -> String {
        let mut value = serde_json::to_value(shop_domain()).unwrap();
        json_patch(&mut value, &patch);
        let domain: Domain = serde_json::from_value(value).unwrap();
        domain.validate(&rs).unwrap_err()
    };
    assert!(broken(json!({"resources":{"shop_products":{"actions":{"restock":{"increment":{"name":1}}}}}})).contains("int, num or decimal"));
    assert!(broken(json!({"resources":{"shop_products":{"actions":{"restock":{"increment":{"stock":"$input.missing"}}}}}})).contains("undeclared input"));
    assert!(broken(json!({"resources":{"shop_products":{"actions":{"restock":{"roles":["public"]}}}}})).contains("public cannot write"));
    assert!(broken(json!({"resources":{"shop_orders":{"transitions":{"transitions":{"ship":{"to":"lost"}}}}}})).contains("from and to must be values"));
    assert!(broken(json!({"resources":{"shop_orders":{"transitions":{"field":"reason"}}}})).contains("declared enum field"));
    assert!(broken(json!({"resources":{"task_items":{"batch":{"max":5000}}}})).contains("max must be 1..1000"));
    assert!(broken(json!({"operations":{"place_order":{"steps":[{"as":"order","create":"shop_orders","data":{"product":"$steps.product.id","quantity":1}}]}}})).contains("earlier step"));
    assert!(broken(json!({"operations":{"place_order":{"steps":[{"as":"x","upsert":"task_items","key":["title"],"data":{"title":"a","code":"b"}}]}}})).contains("declared unique"));
    assert!(broken(json!({"operations":{"place_order":{"steps":[{"as":"x","update":"shop_orders","id":"$input.product_id","data":{"status":"paid"}}]}}})).contains("use a transition step"));
    assert!(broken(json!({"operations":{"place_order":{"roles":["owner"]}}})).contains("owner and public are not operation grants"));
    // A mandatory before-hook can never be bypassed by a declared write.
    let mut hooked = resources();
    hooked[0].hooks.before_update = Some("check_product".into());
    assert!(shop_domain().validate(&hooked).unwrap_err().contains("before-hook"));
    // Unknown keys are rejected when the file is parsed.
    assert!(serde_json::from_value::<Domain>(json!({"resources":{"task_items":{"batch":{"max":1,"size":2}}}})).is_err());
}
/// Minimal recursive merge used to derive broken contracts from the valid one.
fn json_patch(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                json_patch(t.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (t, p) => *t = p.clone(),
    }
}

#[test]
fn openapi_documents_every_domain_route() {
    let rs = resources();
    let mut api = brain_backend::contract::openapi(&rs);
    domain::openapi(&shop_domain(), &rs, &mut api);
    let paths = &api["paths"];
    assert_eq!(paths["/api/shop_products/{id}"]["patch"]["operationId"], "shop_products_patch");
    assert_eq!(paths["/api/task_items/batch"]["post"]["x-brain-batch-max"], 5);
    assert_eq!(paths["/api/shop_products/batch"]["post"]["x-brain-batch-max"], domain::DEFAULT_BATCH_MAX);
    assert_eq!(paths["/api/task_items/upsert"]["post"]["requestBody"]["content"]["application/json"]["schema"]["properties"]["key"]["enum"], json!([["code"]]));
    assert!(paths["/api/shop_orders/upsert"].is_null(), "no unique set, no upsert route");
    assert_eq!(paths["/api/shop_products/{id}/actions/reserve"]["post"]["x-brain-action"]["decrement"]["stock"], "$input.quantity");
    assert_eq!(paths["/api/shop_orders/{id}/transitions/pay"]["post"]["x-brain-state-field"], "status");
    let op = &paths["/_brain/operations/place_order"]["post"];
    assert_eq!(op["requestBody"]["content"]["application/json"]["schema"]["required"], json!(["product_id", "quantity"]));
    assert_eq!(op["responses"]["200"]["content"]["application/json"]["schema"]["properties"]["result"]["required"], json!(["order", "product"]));
    assert!(op["responses"]["409"].is_object());
}

// ---------------------------------------------------------------------------------------------------------------
// Postgres + HTTP
// ---------------------------------------------------------------------------------------------------------------
static ADDRESS: AtomicU32 = AtomicU32::new(1);
/// Each request gets its own client address so the per-IP rate limiter never interferes with the assertions.
fn request(method: &str, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let n = ADDRESS.fetch_add(1, Ordering::Relaxed);
    let ip = std::net::Ipv4Addr::new(10, (n >> 16) as u8, (n >> 8) as u8, n as u8);
    let mut b = Request::builder()
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((ip, 4000))))
        .method(method)
        .uri(uri)
        .header("origin", "http://localhost:5182")
        .header("cookie", format!("__Host-brain={token}"));
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    b.body(body.map_or_else(Body::empty, |v| Body::from(v.to_string()))).unwrap()
}
async fn call(app: &axum::Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (u16, Value) {
    let response = app.clone().oneshot(request(method, uri, token, body)).await.unwrap();
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
#[allow(dead_code)]
struct Shop {
    app: axum::Router,
    pool: sqlx::PgPool,
    admin: String,
    alice: String,
    bob: String,
    run: String,
}
async fn shop() -> Shop {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL")
        .expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let pool = db::connect(&url).await.unwrap();
    db::migrate(&pool, &resources()).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let mut tokens = Vec::new();
    for name in ["admin", "alice", "bob"] {
        let email = format!("{name}-{run}@example.test");
        let user = auth.signup(Credentials { email: email.clone(), password: "a long test password".into() }).await.unwrap();
        if name == "admin" {
            sqlx::query("UPDATE _brain_users SET role='admin' WHERE id=$1").bind(user.id).execute(&pool).await.unwrap();
        }
        let (_, token) = auth.login(Credentials { email, password: "a long test password".into() }).await.unwrap().unwrap();
        tokens.push(token);
    }
    let config = brain_backend::Config {
        roles: Default::default(),
        mail: None,
        auth_public_base_url: None,
        webhooks: None,
        before_hooks: None,
        sockets: None,
        gcs: None,
        jwt: None,
        google: None,
        resources: resources(),
        jobs: Vec::new(),
        uploads: std::env::temp_dir().join(format!("brain-domain-test-{}", std::process::id())),
        origins: vec!["http://localhost:5182".into()],
        production: true,
    };
    let (app, _) = brain_backend::http::router_with_domain(pool.clone(), config, shop_domain()).await.unwrap();
    let [admin, alice, bob] = <[String; 3]>::try_from(tokens).unwrap();
    Shop { app, pool, admin, alice, bob, run }
}
async fn product(s: &Shop, label: &str, extra: Value) -> Value {
    let mut body = json!({"name": format!("Product {label}"), "sku": format!("{label}-{}", s.run)});
    json_patch(&mut body, &extra);
    let (status, row) = call(&s.app, "POST", "/api/shop_products", &s.admin, Some(body)).await;
    assert_eq!(status, 200, "{row}");
    row
}

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_patch_updates_only_sent_fields_with_policy_and_conflict_checks() {
    let s = shop().await;
    let row = product(&s, "patch", json!({"price":"9.50","note":"blue"})).await;
    let uri = format!("/api/shop_products/{}", row["id"].as_str().unwrap());
    let (status, patched) = call(&s.app, "PATCH", &uri, &s.admin, Some(json!({"stock":5,"note":null}))).await;
    assert_eq!(status, 200, "{patched}");
    assert_eq!(patched["data"]["stock"], 5);
    assert_eq!(patched["data"]["price"], "9.50", "unsent fields keep their stored value");
    assert_eq!(patched["data"]["name"], row["data"]["name"]);
    assert!(patched["data"].get("note").is_none(), "null clears an optional field");
    // Every sent field is validated against the full contract and its database constraints.
    let (status, body) = call(&s.app, "PATCH", &uri, &s.admin, Some(json!({"stock":-1}))).await;
    assert_eq!((status, &body["error"]["fields"]["stock"]), (422, &json!("Must be >= 0")), "{body}");
    let (status, body) = call(&s.app, "PATCH", &uri, &s.admin, Some(json!({"colour":"red"}))).await;
    assert_eq!((status, &body["error"]["fields"]["colour"]), (422, &json!("Unknown field")));
    let (status, _) = call(&s.app, "PATCH", &uri, &s.admin, Some(json!({}))).await;
    assert_eq!(status, 422);
    let (status, _) = call(&s.app, "PATCH", &uri, &s.alice, Some(json!({"stock":1}))).await;
    assert_eq!(status, 403, "the update policy applies");
    let missing = format!("/api/shop_products/{}", uuid::Uuid::new_v4());
    let (status, _) = call(&s.app, "PATCH", &missing, &s.admin, Some(json!({"stock":1}))).await;
    assert_eq!(status, 404);
    // Owner-only update policies hide other owners' records.
    let (status, order) = call(&s.app, "POST", "/api/shop_orders", &s.alice, Some(json!({"product":"p","quantity":1}))).await;
    assert_eq!(status, 200, "{order}");
    let order_uri = format!("/api/shop_orders/{}", order["id"].as_str().unwrap());
    let (status, _) = call(&s.app, "PATCH", &order_uri, &s.bob, Some(json!({"quantity":2}))).await;
    assert_eq!(status, 404);
    let (status, body) = call(&s.app, "PATCH", &order_uri, &s.alice, Some(json!({"quantity":2}))).await;
    assert_eq!((status, &body["data"]["quantity"]), (200, &json!(2)));
    // A state field changes only through declared transitions, whatever the write path.
    let (status, body) = call(&s.app, "PATCH", &order_uri, &s.alice, Some(json!({"status":"paid"}))).await;
    assert_eq!(status, 422, "{body}");
    assert!(body["error"]["fields"]["status"].as_str().unwrap().contains("pay"));
    let (status, _) = call(&s.app, "POST", "/api/shop_orders", &s.alice, Some(json!({"product":"p","quantity":1,"status":"shipped"}))).await;
    assert_eq!(status, 422, "records start in a declared initial state");
    // Concurrent PATCHes of different fields both land: the conflict check retries nothing silently.
    let (a, b) = tokio::join!(
        call(&s.app, "PATCH", &uri, &s.admin, Some(json!({"note":"first"}))),
        call(&s.app, "PATCH", &uri, &s.admin, Some(json!({"price":"11.00"})))
    );
    for (status, body) in [&a, &b] {
        assert!(*status == 200 || *status == 409 && body["error"]["code"] == "RESOURCE_CHANGED", "{status} {body}");
    }
}

fn id(row: &Value) -> String {
    row["id"].as_str().unwrap().to_string()
}
async fn stock(s: &Shop, product: &str) -> i64 {
    sqlx::query_scalar("SELECT (data->>'stock')::bigint FROM _brain_res_shop_products WHERE id=$1::uuid")
        .bind(product).fetch_one(&s.pool).await.unwrap()
}
async fn place(s: &Shop, token: &str, product: &str, quantity: i64) -> (u16, Value) {
    call(&s.app, "POST", "/_brain/operations/place_order", token, Some(json!({"product_id":product,"quantity":quantity}))).await
}
async fn orders_for(s: &Shop, product: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM _brain_res_shop_orders WHERE data->>'product'=$1")
        .bind(product).fetch_one(&s.pool).await.unwrap()
}

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_operations_commit_every_step_or_roll_all_of_them_back() {
    let s = shop().await;
    let p = id(&product(&s, "ops", json!({"stock":100,"status":"active"})).await);
    let (status, body) = place(&s, &s.alice, &p, 2).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["operation"], "place_order");
    assert_eq!(body["result"]["product"]["data"]["stock"], 98);
    let order = &body["result"]["order"];
    assert_eq!(order["data"]["product"], p.as_str());
    assert_eq!(order["data"]["status"], "pending", "declared initial state");
    let pay_by = order["data"]["pay_by"].as_str().unwrap();
    assert!(pay_by.len() == 10 && pay_by > &order["created_at"].as_str().unwrap()[..10], "$now+3d as a date: {pay_by}");
    assert_eq!(order["owner"], body["result"]["order"]["owner"]);
    // Step 1 (reserve 60) succeeds, step 2 violates the order's quantity check: nothing is kept.
    let (status, body) = place(&s, &s.alice, &p, 60).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["fields"]["order.quantity"], "Must be <= 50");
    assert_eq!(stock(&s, &p).await, 98, "the reservation rolled back with the failed order");
    assert_eq!(orders_for(&s, &p).await, 1);
    // A failed guard is a 409 naming the step and field; nothing changes.
    let (status, body) = place(&s, &s.alice, &p, 99).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "GUARD_FAILED");
    assert_eq!(body["error"]["fields"]["product.stock"], "Must be >= 99");
    // Typed input, unknown record, missing role, missing operation.
    let (status, body) = call(&s.app, "POST", "/_brain/operations/place_order", &s.alice, Some(json!({"product_id":p,"quantity":"2"}))).await;
    assert_eq!((status, body["error"]["fields"]["input.quantity"].is_string()), (422, true), "{body}");
    let (status, body) = call(&s.app, "POST", "/_brain/operations/place_order", &s.alice, Some(json!({"product_id":uuid::Uuid::new_v4().to_string(),"quantity":1}))).await;
    assert_eq!((status, &body["error"]["fields"]["product"]), (404, &json!("Not found")), "{body}");
    let (status, _) = call(&s.app, "POST", "/_brain/operations/unknown", &s.alice, Some(json!({}))).await;
    assert_eq!(status, 404);
    let (status, _) = call(&s.app, "POST", "/_brain/operations/place_order", "not-a-session", Some(json!({"product_id":p,"quantity":1}))).await;
    assert_eq!(status, 401);
    assert_eq!(stock(&s, &p).await, 98);
}

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_atomic_actions_and_upserts_are_race_safe_under_concurrent_requests() {
    let s = shop().await;
    // Guarded decrement: 30 concurrent orders of 1 against a stock of 10 → exactly 10 succeed, stock never negative.
    let p = id(&product(&s, "race", json!({"stock":10,"status":"active"})).await);
    let tasks: Vec<_> = (0..30)
        .map(|_| {
            let (app, token, product) = (s.app.clone(), s.alice.clone(), p.clone());
            tokio::spawn(async move {
                call(&app, "POST", "/_brain/operations/place_order", &token, Some(json!({"product_id":product,"quantity":1}))).await
            })
        })
        .collect();
    let mut outcomes = BTreeMap::new();
    for task in tasks {
        let (status, body) = task.await.unwrap();
        assert!(status == 200 || status == 409 && body["error"]["code"] == "GUARD_FAILED", "{status} {body}");
        *outcomes.entry(status).or_insert(0) += 1;
    }
    assert_eq!(outcomes, BTreeMap::from([(200, 10), (409, 20)]));
    assert_eq!(stock(&s, &p).await, 0);
    assert_eq!(orders_for(&s, &p).await, 10);
    // Guarded increment through the action route, with typed input and role check.
    let restock = format!("/api/shop_products/{p}/actions/restock");
    let (status, body) = call(&s.app, "POST", &restock, &s.admin, Some(json!({"amount":7}))).await;
    assert_eq!((status, &body["data"]["stock"]), (200, &json!(7)), "{body}");
    let (status, _) = call(&s.app, "POST", &restock, &s.alice, Some(json!({"amount":7}))).await;
    assert_eq!(status, 403, "action roles apply");
    let (status, body) = call(&s.app, "POST", &restock, &s.admin, Some(json!({"amount":-8}))).await;
    assert_eq!((status, body["error"]["fields"]["stock"].as_str()), (422, Some("Must be >= 0")), "{body}");
    let (status, _) = call(&s.app, "POST", &format!("/api/shop_products/{p}/actions/explode"), &s.admin, Some(json!({}))).await;
    assert_eq!(status, 404);
    // Concurrent increments never lose an update.
    let tasks: Vec<_> = (0..20)
        .map(|_| {
            let (app, token, uri) = (s.app.clone(), s.admin.clone(), restock.clone());
            tokio::spawn(async move { call(&app, "POST", &uri, &token, Some(json!({"amount":1}))).await.0 })
        })
        .collect();
    for task in tasks {
        assert_eq!(task.await.unwrap(), 200);
    }
    assert_eq!(stock(&s, &p).await, 27);
    // Compare-and-set: two clients both read price 0.00; only the first reprice wins.
    let reprice = format!("/api/shop_products/{p}/actions/reprice");
    let (a, b) = tokio::join!(
        call(&s.app, "POST", &reprice, &s.admin, Some(json!({"expected":"0.00","price":"12.00"}))),
        call(&s.app, "POST", &reprice, &s.admin, Some(json!({"expected":"0","price":"15.00"})))
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 409], "{a:?} {b:?}");
    let loser = if a.0 == 409 { &a.1 } else { &b.1 };
    assert_eq!(loser["error"]["fields"]["price"], "Must equal 0.00");
    // Insert-if-absent: 12 concurrent upserts of one key create exactly one row.
    let code = format!("T-{}", s.run);
    let tasks: Vec<_> = (0..12)
        .map(|i| {
            let (app, token, code) = (s.app.clone(), s.alice.clone(), code.clone());
            tokio::spawn(async move {
                call(&app, "POST", "/api/task_items/upsert", &token, Some(json!({"key":["code"],"on_conflict":"ignore","data":{"title":format!("Attempt {i}"),"code":code}}))).await
            })
        })
        .collect();
    let mut created = 0;
    let mut ids = std::collections::BTreeSet::new();
    for task in tasks {
        let (status, body) = task.await.unwrap();
        assert_eq!(status, 200, "{body}");
        created += usize::from(body["created"] == true);
        ids.insert(id(&body["record"]));
    }
    assert_eq!((created, ids.len()), (1, 1));
    // on_conflict update merges only the sent fields into the existing row (fields not sent keep their values).
    let (status, body) = call(&s.app, "POST", "/api/task_items/upsert", &s.alice, Some(json!({"key":["code"],"data":{"title":"Renamed","code":code,"done":true}}))).await;
    assert_eq!((status, &body["created"], &body["record"]["data"]["done"]), (200, &json!(false), &json!(true)), "{body}");
    assert_eq!(body["record"]["data"]["points"], 0);
    // Another user's row is not updatable through upsert.
    let (status, _) = call(&s.app, "POST", "/api/task_items/upsert", &s.bob, Some(json!({"key":["code"],"data":{"title":"Hijack","code":code}}))).await;
    assert_eq!(status, 403);
    let (status, body) = call(&s.app, "POST", "/api/task_items/upsert", &s.alice, Some(json!({"key":["title"],"data":{"title":"x","code":"y"}}))).await;
    assert_eq!((status, body["error"]["fields"]["key"].is_string()), (422, true));
}

async fn batch(s: &Shop, body: Value) -> (u16, Value) {
    call(&s.app, "POST", "/api/task_items/batch", &s.alice, Some(body)).await
}
async fn tasks_with_prefix(s: &Shop, prefix: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM _brain_res_task_items WHERE data->>'code' LIKE $1")
        .bind(format!("{prefix}%")).fetch_one(&s.pool).await.unwrap()
}

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_batches_are_all_or_nothing_with_per_item_errors() {
    let s = shop().await;
    let pre = format!("B-{}-", s.run);
    let item = |n: usize| json!({"title":format!("Task {n}"),"code":format!("{pre}{n}")});
    let (status, body) = batch(&s, json!({"op":"create","items":[item(1),item(2),item(3)]})).await;
    assert_eq!((status, &body["count"]), (200, &json!(3)), "{body}");
    let ids: Vec<String> = body["items"].as_array().unwrap().iter().map(id).collect();
    // One invalid item: per-index errors, nothing written.
    let (status, body) = batch(&s, json!({"op":"create","items":[item(4),{"code":format!("{pre}5")},{"title":"x","code":format!("{pre}6"),"points":"9"}]})).await;
    assert_eq!(status, 422, "{body}");
    assert!(body["error"]["fields"]["items.1.title"].is_string() && body["error"]["fields"]["items.2.points"].is_string(), "{body}");
    assert_eq!(tasks_with_prefix(&s, &pre).await, 3);
    // A database conflict on the third item rolls back the first two and names the item.
    let (status, body) = batch(&s, json!({"op":"create","items":[item(7),item(8),item(1)]})).await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"]["fields"]["items.2.code"].is_string(), "{body}");
    assert_eq!(tasks_with_prefix(&s, &pre).await, 3);
    // Declared max (5) and the shape of each operation.
    let (status, body) = batch(&s, json!({"op":"create","items":(10..16).map(item).collect::<Vec<_>>()})).await;
    assert_eq!((status, body["error"]["fields"]["items"].as_str()), (422, Some("Send 1..5 items")));
    let (status, _) = batch(&s, json!({"op":"merge","items":[item(1)]})).await;
    assert_eq!(status, 422);
    // Update: PATCH semantics per item; an unknown id makes the whole batch a 404.
    let (status, body) = batch(&s, json!({"op":"update","items":[{"id":ids[0],"data":{"done":true}},{"id":ids[1],"data":{"points":5}}]})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!((&body["items"][0]["data"]["done"], &body["items"][0]["data"]["title"]), (&json!(true), &json!("Task 1")));
    let (status, body) = batch(&s, json!({"op":"update","items":[{"id":ids[2],"data":{"done":true}},{"id":uuid::Uuid::new_v4().to_string(),"data":{"done":true}}]})).await;
    assert_eq!((status, &body["error"]["fields"]["items.1"]), (404, &json!("Not found")), "{body}");
    let done: Option<bool> = sqlx::query_scalar("SELECT (data->>'done')::boolean FROM _brain_res_task_items WHERE id=$1::uuid").bind(&ids[2]).fetch_one(&s.pool).await.unwrap();
    assert_eq!(done, None, "the first item rolled back with the missing second one");
    let (status, body) = batch(&s, json!({"op":"update","items":[{"id":ids[0],"data":{"done":false}},{"id":ids[0],"data":{"done":true}}]})).await;
    assert_eq!((status, body["error"]["fields"]["items.1.id"].as_str()), (422, Some("Duplicate id in this batch")));
    // Other owners' rows are invisible (owner-only policy).
    let (status, _) = call(&s.app, "POST", "/api/task_items/batch", &s.bob, Some(json!({"op":"delete","items":[{"id":ids[0]}]}))).await;
    assert_eq!(status, 404);
    // Upsert batch reports which items were created.
    let (status, body) = batch(&s, json!({"op":"upsert","key":["code"],"items":[item(1),item(9)]})).await;
    assert_eq!((status, &body["created"]), (200, &json!([false, true])), "{body}");
    // Delete returns ids only.
    let (status, body) = batch(&s, json!({"op":"delete","items":ids.iter().map(|i| json!({"id":i})).collect::<Vec<_>>()})).await;
    assert_eq!((status, &body["count"], body["items"][0].get("data").is_none()), (200, &json!(3), true), "{body}");
    assert_eq!(tasks_with_prefix(&s, &pre).await, 1);
}

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_state_transitions_are_the_only_way_to_change_state() {
    let s = shop().await;
    let (status, order) = call(&s.app, "POST", "/api/shop_orders", &s.alice, Some(json!({"product":"p","quantity":1}))).await;
    assert_eq!((status, &order["data"]["status"]), (200, &json!("pending")));
    let t = |name: &str| format!("/api/shop_orders/{}/transitions/{name}", id(&order));
    let (status, _) = call(&s.app, "POST", &t("pay"), &s.bob, Some(json!({}))).await;
    assert_eq!(status, 403, "owner role means the caller owns the row");
    let (status, _) = call(&s.app, "POST", &t("ship"), &s.alice, Some(json!({}))).await;
    assert_eq!(status, 403, "only admin ships");
    let (status, paid) = call(&s.app, "POST", &t("pay"), &s.alice, Some(json!({}))).await;
    assert_eq!((status, &paid["data"]["status"]), (200, &json!("paid")), "{paid}");
    let paid_at = paid["data"]["paid_at"].as_str().unwrap();
    assert!(paid_at.ends_with('Z') && paid_at.len() == 24, "side effect in the same write: {paid_at}");
    let (status, body) = call(&s.app, "POST", &t("pay"), &s.alice, Some(json!({}))).await;
    assert_eq!((status, &body["error"]["fields"]["status"]), (409, &json!("Transition pay is allowed from pending")), "{body}");
    // Concurrent ship and cancel: exactly one wins.
    let (ship, cancel) = (t("ship"), t("cancel"));
    let (a, b) = tokio::join!(
        call(&s.app, "POST", &ship, &s.admin, Some(json!({}))),
        call(&s.app, "POST", &cancel, &s.alice, Some(json!({"reason":"Changed my mind"})))
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 409], "{a:?} {b:?}");
    let (status, body) = call(&s.app, "POST", &t("cancel"), &s.alice, Some(json!({}))).await;
    assert!(status == 409 || status == 422, "{body}");
    let (status, body) = call(&s.app, "POST", &t("cancel"), &s.admin, Some(json!({"reason":1}))).await;
    assert_eq!(status, 422, "typed input: {body}");
    let (status, _) = call(&s.app, "POST", &t("teleport"), &s.admin, Some(json!({}))).await;
    assert_eq!(status, 404);
    // PUT and batch cannot change the state field either.
    let (status, current) = call(&s.app, "GET", &format!("/api/shop_orders/{}", id(&order)), &s.admin, None).await;
    assert_eq!(status, 200);
    let mut data = current["data"].clone();
    data["status"] = json!("pending");
    let (status, _) = call(&s.app, "PUT", &format!("/api/shop_orders/{}", id(&order)), &s.admin, Some(data)).await;
    assert_eq!(status, 422);
    let (status, _) = call(&s.app, "POST", "/api/shop_orders/batch", &s.admin, Some(json!({"op":"update","items":[{"id":id(&order),"data":{"status":"pending"}}]}))).await;
    assert_eq!(status, 422);
}
