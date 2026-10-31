//! Declarative domain writes: PATCH, multi-resource operations, atomic actions, upserts, batches and state
//! transitions. Unit checks run everywhere; the Postgres test runs through scripts/backend-check.mjs.
use axum::body::{Body, to_bytes};
use brain_backend::{
    auth::{Auth, Credentials},
    contract::{Check, Constraints, Field, Hooks, Policy, Resource},
    db,
    domain::{self, Domain},
};
use axum::http::Request;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use tower::ServiceExt;

fn field(kind: &str, required: bool) -> Field {
    Field { kind: kind.into(), required, ..Default::default() }
}
fn grants(read: &[&str], create: &[&str], update: &[&str], delete: &[&str]) -> Policy {
    let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect();
    Policy { read: v(read), create: v(create), update: v(update), delete: v(delete) }
}
fn products() -> Resource {
    let mut fields = BTreeMap::from([
        ("name".into(), field("str", true)),
        ("sku".into(), field("str", true)),
        ("stock".into(), field("int", false)),
        ("price".into(), field("decimal(10,2)", false)),
        ("status".into(), field("enum(draft|active|archived)", false)),
        ("note".into(), field("str", false)),
    ]);
    fields.get_mut("sku").unwrap().unique = true;
    fields.get_mut("stock").unwrap().default = Some(json!(0));
    fields.get_mut("price").unwrap().default = Some(json!("0.00"));
    fields.get_mut("status").unwrap().default = Some(json!("draft"));
    Resource {
        name: "shop_products".into(),
        search: vec!["name".into()],
        fields,
        policy: grants(&["candidate", "admin"], &["admin"], &["admin"], &["admin"]),
        hooks: Hooks::default(),
        constraints: Constraints {
            check: vec![Check { field: Some("stock".into()), min: Some(json!(0)), ..Default::default() }],
            ..Default::default()
        },
    }
}
fn orders() -> Resource {
    let mut fields = BTreeMap::from([
        ("product".into(), field("str", true)),
        ("quantity".into(), field("int", true)),
        ("status".into(), field("enum(pending|paid|shipped|cancelled)", false)),
        ("pay_by".into(), field("date", false)),
        ("paid_at".into(), field("datetime", false)),
        ("reason".into(), field("str", false)),
    ]);
    fields.get_mut("status").unwrap().default = Some(json!("pending"));
    Resource {
        name: "shop_orders".into(),
        search: Vec::new(),
        fields,
        policy: grants(&["owner", "admin"], &["candidate", "admin"], &["owner", "admin"], &["admin"]),
        hooks: Hooks::default(),
        constraints: Constraints {
            check: vec![Check { field: Some("quantity".into()), min: Some(json!(1)), max: Some(json!(50)), ..Default::default() }],
            ..Default::default()
        },
    }
}
fn tasks() -> Resource {
    let mut fields = BTreeMap::from([
        ("title".into(), field("str", true)),
        ("code".into(), field("str", true)),
        ("done".into(), field("bool", false)),
        ("points".into(), field("int", false)),
    ]);
    fields.get_mut("code").unwrap().unique = true;
    fields.get_mut("points").unwrap().default = Some(json!(0));
    Resource {
        name: "task_items".into(),
        search: Vec::new(),
        fields,
        policy: grants(&["owner"], &["candidate"], &["owner"], &["owner"]),
        hooks: Hooks::default(),
        constraints: Constraints::default(),
    }
}
/// The domain exactly as the CLI loads it from `server/**.jsbrain` files.
fn shop_domain() -> Domain {
    serde_json::from_value(json!({
        "resources": {
            "shop_products": {"actions": {
                "restock": {"roles": ["admin"], "input": {"amount": "int!"}, "when": {"status": {"ne": "archived"}}, "increment": {"stock": "$input.amount"}},
                "reserve": {"roles": ["admin"], "input": {"quantity": "int!"}, "when": {"status": "active", "stock": {"gte": "$input.quantity"}}, "decrement": {"stock": "$input.quantity"}},
                "reprice": {"roles": ["admin"], "input": {"expected": "decimal(10,2)!", "price": "decimal(10,2)!"}, "when": {"price": "$input.expected"}, "set": {"price": "$input.price"}}
            }},
            "shop_orders": {"transitions": {"field": "status", "initial": ["pending"], "transitions": {
                "pay": {"from": ["pending"], "to": "paid", "roles": ["owner", "admin"], "set": {"paid_at": "$now"}},
                "ship": {"from": ["paid"], "to": "shipped", "roles": ["admin"]},
                "cancel": {"from": ["pending", "paid"], "to": "cancelled", "roles": ["owner", "admin"], "input": {"reason": "str!"}, "set": {"reason": "$input.reason"}}
            }}},
            "task_items": {"batch": {"max": 5}}
        },
        "operations": {
            "place_order": {
                "roles": ["candidate", "admin"],
                "input": {"product_id": "str!", "quantity": "int!"},
                "steps": [
                    {"as": "product", "action": "shop_products.reserve", "id": "$input.product_id", "input": {"quantity": "$input.quantity"}},
                    {"as": "order", "create": "shop_orders", "data": {"product": "$steps.product.id", "quantity": "$input.quantity", "pay_by": "$now+3d"}}
                ],
                "result": ["order", "product"]
            }
        }
    }))
    .unwrap()
}
fn resources() -> Vec<Resource> {
    vec![products(), orders(), tasks()]
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
