//! Relations, declared filters, sort, totals, aggregates, keyset cursors, relational ownership and field visibility
//! (B-01, B-08, B-09/B-21, B-11..B-15, B-20) through the real HTTP router against a disposable Postgres database.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use brain_backend::{
    auth::{Auth, Credentials},
    auth_flows::RolesConfig,
    contract::{Field, Hooks, Policy, Resource},
    db,
};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::{collections::BTreeMap, str::FromStr};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5190";

fn field(kind: &str, required: bool) -> Field {
    Field { kind: kind.into(), required, ..Default::default() }
}
fn policy(read: &[&str], create: &[&str], update: &[&str], delete: &[&str]) -> Policy {
    let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect();
    Policy { read: v(read), create: v(create), update: v(update), delete: v(delete) }
}
fn resource(name: &str, fields: Vec<(&str, Field)>, policy: Policy, query: Value) -> Resource {
    Resource {
        name: name.into(),
        search: Vec::new(),
        fields: fields.into_iter().map(|(n, f)| (n.to_string(), f)).collect(),
        policy,
        hooks: Hooks::default(),
        constraints: Default::default(),
        query: serde_json::from_value(query).unwrap(),
    }
}
/// A generic shop catalog: shops own products, products collect reviews, orders point at products.
pub fn catalog() -> Vec<Resource> {
    let mut shop_ref = field("ref:shops", true);
    shop_ref.on_delete = Some("cascade".into());
    let mut cost = field("decimal(10,2)", false);
    cost.visible = Some(vec!["owner".into(), "admin".into()]);
    let mut product_ref = field("ref:products", false);
    product_ref.on_delete = Some("set_null".into());
    let mut products = resource(
        "products",
        vec![
            ("shop", shop_ref),
            ("title", field("str", true)),
            ("price", field("decimal(10,2)", true)),
            ("stock", field("int", false)),
            ("category", field("enum(tools|garden|kitchen)", true)),
            ("released_on", field("date", false)),
            ("cost", cost),
        ],
        policy(&["public"], &["owner.shop"], &["owner.shop"], &["owner.shop", "admin"]),
        json!({
            "filter": {
                "price": ["eq", "gte", "lte", "between"],
                "stock": ["gt", "exists"],
                "category": ["eq", "in"],
                "title": ["prefix", "iprefix"],
                "released_on": ["lt", "gte"],
                "shop": ["eq"],
                "shop.country": ["eq"],
                "created_at": ["gte"]
            },
            "sort": ["price", "title", {"field": "stock", "nulls": "first"}, "released_on"],
            "default_sort": ["-created_at"],
            "include": ["shop"],
            "aggregate": {
                "totals": {"sum": ["price", "stock"], "avg": ["price"], "min": ["price", "released_on"], "max": ["stock"]},
                "by_category": {"group": "category", "sum": ["price"]}
            },
            "count": {"reviews": {"resource": "reviews", "field": "product"}, "five_star": {"resource": "reviews", "field": "product", "where": {"rating": 5}}}
        }),
    );
    products.search = vec!["title".into()];
    vec![
        resource(
            "shops",
            vec![("name", field("str", true)), ("country", field("str", true))],
            policy(&["public"], &["candidate"], &["owner"], &["owner"]),
            json!({"filter": {"country": ["eq"]}}),
        ),
        products,
        resource(
            "reviews",
            vec![("product", field("ref:products", true)), ("rating", field("int", true)), ("body", field("str", false))],
            policy(&["owner", "owner.product.shop"], &["candidate"], &["owner"], &["owner"]),
            json!({"filter": {"rating": ["eq"]}}),
        ),
        resource(
            "orders",
            vec![("product", product_ref), ("quantity", field("int", true))],
            policy(&["owner"], &["candidate"], &["owner"], &["owner"]),
            json!({}),
        ),
    ]
}

struct Api {
    http: Router,
    auth: Auth,
}
impl Api {
    async fn call(&self, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> (u16, Value) {
        let mut b = Request::builder()
            .extension(axum::extract::ConnectInfo("127.0.0.1:3456".parse::<std::net::SocketAddr>().unwrap()))
            .method(method)
            .uri(uri)
            .header("origin", ORIGIN);
        if let Some(token) = token {
            b = b.header("cookie", format!("__Host-brain={token}"));
        }
        let req = match body {
            Some(body) => b.header("content-type", "application/json").body(Body::from(body.to_string())),
            None => b.body(Body::empty()),
        }
        .unwrap();
        let res = self.http.clone().oneshot(req).await.unwrap();
        let status = res.status().as_u16();
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
    async fn get(&self, uri: &str, token: Option<&str>) -> (u16, Value) {
        self.call("GET", uri, token, None).await
    }
    async fn post(&self, uri: &str, token: &str, body: Value) -> (u16, Value) {
        self.call("POST", uri, Some(token), Some(body)).await
    }
    async fn user(&self, email: &str) -> (Uuid, String) {
        let credentials = || Credentials { email: email.into(), password: "a long test password".into() };
        let user = self.auth.signup(credentials()).await.unwrap();
        let (_, token) = self.auth.login(credentials()).await.unwrap().unwrap();
        (user.id, token)
    }
}

async fn database(prefix: &str) -> sqlx::PgPool {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let base = db::connect(&url).await.unwrap();
    let name = format!("{prefix}_{}", Uuid::new_v4().simple());
    // Fixed ASCII prefix plus UUID hex, never request input.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name);
    let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
    base.close().await;
    pool
}
async fn api(pool: &sqlx::PgPool, resources: Vec<Resource>) -> Api {
    db::migrate(pool, &resources).await.unwrap();
    RolesConfig::default().register(pool).await.unwrap();
    let http = brain_backend::http::router(
        pool.clone(),
        brain_backend::Config {
            roles: Default::default(),
            mail: None,
            auth_public_base_url: None,
            webhooks: None,
            before_hooks: None,
            sockets: None,
            gcs: None,
            jwt: None,
            google: None,
            resources,
            jobs: Vec::new(),
            uploads: std::env::temp_dir().join(format!("brain-data-queries-{}", Uuid::new_v4().simple())),
            origins: vec![ORIGIN.into()],
            production: false,
        },
    )
    .await
    .unwrap();
    Api { http, auth: Auth::new(pool.clone()).await.unwrap() }
}
fn ids(list: &Value) -> Vec<String> {
    list["items"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
}
fn titles(list: &Value) -> Vec<String> {
    list["items"].as_array().unwrap().iter().map(|r| r["data"]["title"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
#[ignore = "requires scripts/backend-check.mjs disposable Postgres"]
async fn relations_filters_sort_totals_aggregates_cursors_ownership_and_visibility() {
    let pool = database("brain_data_queries").await;
    let api = api(&pool, catalog()).await;
    let (owner_id, owner) = api.user("owner@shop.test").await;
    let (_, rival) = api.user("rival@shop.test").await;
    let (_, buyer) = api.user("buyer@shop.test").await;

    // B-01: the reference is a real foreign key with the declared action and an index.
    let actions: Vec<(String, String)> = sqlx::query_as("SELECT conrelid::regclass::text, confdeltype::text FROM pg_constraint WHERE contype='f' AND conrelid::regclass::text LIKE '_brain_res_%' ORDER BY 1")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(actions.contains(&("_brain_res_products".into(), "c".into())), "{actions:?}");
    assert!(actions.contains(&("_brain_res_reviews".into(), "r".into())), "{actions:?}");
    assert!(actions.contains(&("_brain_res_orders".into(), "n".into())), "{actions:?}");

    let (s, mx) = api.post("/api/shops", &owner, json!({"name": "Hammer & Co", "country": "MX"})).await;
    assert_eq!(s, 200, "{mx}");
    let mx_id = mx["id"].as_str().unwrap().to_string();
    let (_, us) = api.post("/api/shops", &owner, json!({"name": "Garden Corner", "country": "US"})).await;
    let us_id = us["id"].as_str().unwrap().to_string();
    let (_, rival_shop) = api.post("/api/shops", &rival, json!({"name": "Rival Goods", "country": "MX"})).await;

    // A reference to a missing record is a field error, not a 500.
    let (s, e) = api.post("/api/products", &owner, json!({"shop": Uuid::new_v4().to_string(), "title": "Ghost", "price": "1.00", "category": "tools"})).await;
    assert!(s == 403 || s == 422, "{s} {e}");
    let (s, e) = api.post("/api/orders", &buyer, json!({"product": Uuid::new_v4().to_string(), "quantity": 1})).await;
    assert_eq!((s, e["error"]["code"].as_str()), (422, Some("REFERENCE_NOT_FOUND")), "{e}");
    assert!(e["error"]["fields"]["product"].is_string());
    // B-21: creating under somebody else's shop is denied.
    let (s, _) = api.post("/api/products", &owner, json!({"shop": rival_shop["id"], "title": "Stolen", "price": "1.00", "category": "tools"})).await;
    assert_eq!(s, 403);

    let rows = [
        (&mx_id, "Blue hammer", "10.50", Some(5), "tools", Some("2026-01-10"), "4.00"),
        (&mx_id, "blue rake", "7.25", None, "garden", Some("2026-03-01"), "2.00"),
        (&us_id, "Copper pan", "30.00", Some(2), "kitchen", None, "12.00"),
        (&us_id, "Garden hose", "12.75", Some(9), "garden", Some("2026-02-15"), "5.10"),
        (&mx_id, "Anvil", "10.50", Some(1), "tools", Some("2025-12-31"), "6.00"),
    ];
    let mut product = BTreeMap::new();
    for (shop, title, price, stock, category, released, cost) in rows {
        let mut body = json!({"shop": shop, "title": title, "price": price, "category": category, "cost": cost});
        if let Some(stock) = stock {
            body["stock"] = json!(stock);
        }
        if let Some(released) = released {
            body["released_on"] = json!(released);
        }
        let (s, row) = api.post("/api/products", &owner, body).await;
        assert_eq!(s, 200, "{row}");
        assert_eq!(row["data"]["cost"], cost, "the relational owner sees restricted fields");
        product.insert(title, row["id"].as_str().unwrap().to_string());
    }

    // B-20: anonymous readers never receive restricted fields; owners do.
    let (s, list) = api.get("/api/products?limit=100", None).await;
    assert_eq!(s, 200, "{list}");
    assert!(list["items"].as_array().unwrap().iter().all(|r| r["data"].get("cost").is_none()));
    let (_, list) = api.get("/api/products?limit=100", Some(&owner)).await;
    assert!(list["items"].as_array().unwrap().iter().all(|r| r["data"].get("cost").is_some()));
    let (_, one) = api.get(&format!("/api/products/{}", product["Anvil"]), Some(&rival)).await;
    assert!(one["data"].get("cost").is_none() && one["data"]["title"] == "Anvil");

    // B-13: total accompanies every page.
    let (_, page) = api.get("/api/products?limit=2", None).await;
    assert_eq!(page["total"], 5);
    assert_eq!(page["items"].as_array().unwrap().len(), 2);

    // B-11: declared filters with operators; undeclared parameters and operators are rejected.
    let (_, r) = api.get("/api/products?price[gte]=10.50&sort=title", None).await;
    assert_eq!(titles(&r), ["Anvil", "Blue hammer", "Copper pan", "Garden hose"]);
    assert_eq!(r["total"], 4);
    let (_, r) = api.get("/api/products?price[between]=7.00,10.50&category[in]=tools,garden&sort=price,title", None).await;
    assert_eq!(titles(&r), ["blue rake", "Anvil", "Blue hammer"]);
    let (_, r) = api.get("/api/products?title[prefix]=Blue", None).await;
    assert_eq!(titles(&r), ["Blue hammer"]);
    let (_, r) = api.get("/api/products?title[iprefix]=BLUE&sort=title", None).await;
    assert_eq!(titles(&r), ["Blue hammer", "blue rake"]);
    let (_, r) = api.get("/api/products?released_on[lt]=2026-01-01", None).await;
    assert_eq!(titles(&r), ["Anvil"]);
    let (_, r) = api.get("/api/products?released_on[lt]=now&sort=title", None).await;
    assert_eq!(r["total"], 4, "date-vs-now comparisons run on the database clock");
    let (_, r) = api.get("/api/products?created_at[gte]=now-1d", None).await;
    assert_eq!(r["total"], 5);
    let (_, r) = api.get("/api/products?stock[exists]=false", None).await;
    assert_eq!(titles(&r), ["blue rake"]);
    let (_, r) = api.get("/api/products?search=hammer&category=tools", None).await;
    assert_eq!((titles(&r), r["total"].clone()), (vec!["Blue hammer".to_string()], json!(1)), "search composes with filters");
    let (_, r) = api.get(&format!("/api/products?shop={us_id}&sort=title"), None).await;
    assert_eq!(titles(&r), ["Copper pan", "Garden hose"]);
    let (_, r) = api.get("/api/products?shop.country=US&sort=title", None).await;
    assert_eq!(titles(&r), ["Copper pan", "Garden hose"], "filter by a related field");
    for bad in ["/api/products?colour=red", "/api/products?price[gt]=1", "/api/products?price=abc", "/api/products?cost[gte]=1", "/api/products?sort=cost", "/api/products?limit=0", "/api/products?price=1&price=2"] {
        let (s, e) = api.get(bad, None).await;
        assert_eq!(s, 422, "{bad} {e}");
    }

    // B-12: per-field sort with direction, nulls placement and a stable id tie-break.
    let (_, r) = api.get("/api/products?sort=-price,title", None).await;
    assert_eq!(titles(&r), ["Copper pan", "Garden hose", "Anvil", "Blue hammer", "blue rake"]);
    let (_, r) = api.get("/api/products?sort=stock", None).await;
    assert_eq!(titles(&r)[0], "blue rake", "declared nulls: first");
    let (_, r) = api.get("/api/products?sort=-released_on", None).await;
    assert_eq!(titles(&r).last().unwrap(), "Copper pan", "nulls last by default");
    let (_, r) = api.get("/api/products?sort=price", None).await;
    let tied: Vec<String> = ids(&r)[1..3].to_vec();
    let mut sorted = tied.clone();
    sorted.sort();
    assert_eq!(tied, sorted, "ties break by id in a stable direction");

    // B-15: keyset pages walk the full ordering exactly once, including null keys.
    for sort in ["price", "-price", "stock", "-released_on", "title", "-created_at"] {
        let (_, all) = api.get(&format!("/api/products?sort={sort}&limit=100"), None).await;
        let mut seen = Vec::new();
        let mut uri = format!("/api/products?sort={sort}&limit=2");
        loop {
            let (s, page) = api.get(&uri, None).await;
            assert_eq!(s, 200, "{sort} {page}");
            seen.extend(ids(&page));
            let Some(next) = page["next_cursor"].as_str() else { break };
            uri = format!("/api/products?sort={sort}&limit=2&cursor={next}");
        }
        assert_eq!(seen, ids(&all), "cursor walk for {sort}");
    }
    let (_, page) = api.get("/api/products?sort=price&limit=2", None).await;
    let cursor = page["next_cursor"].as_str().unwrap();
    let (s, _) = api.get(&format!("/api/products?sort=-price&limit=2&cursor={cursor}"), None).await;
    assert_eq!(s, 422, "a cursor is bound to its sort");
    let (s, _) = api.get(&format!("/api/products?sort=price&limit=2&offset=2&cursor={cursor}"), None).await;
    assert_eq!(s, 422);

    // B-13: declared aggregates over the filtered set; decimals stay exact.
    let (_, r) = api.get("/api/products?aggregate=totals,by_category&limit=1", None).await;
    let totals = &r["aggregates"]["totals"];
    assert_eq!(totals["count"], 5);
    assert_eq!(totals["sum"]["price"], "71.00");
    assert_eq!(totals["sum"]["stock"], "17");
    assert_eq!(totals["avg"]["price"], "14.2000");
    assert_eq!(totals["min"]["price"], "7.25");
    assert_eq!(totals["min"]["released_on"], "2025-12-31");
    assert_eq!(totals["max"]["stock"], 9);
    let groups = r["aggregates"]["by_category"].as_array().unwrap();
    assert_eq!(groups[0]["key"], "garden");
    assert_eq!(groups[0]["count"], 2);
    assert_eq!(groups[0]["sum"]["price"], "20.00");
    let (_, r) = api.get("/api/products?category=tools&aggregate=totals&limit=1", None).await;
    assert_eq!(r["aggregates"]["totals"]["sum"]["price"], "21.00");
    let (s, _) = api.get("/api/products?aggregate=nope", None).await;
    assert_eq!(s, 422);

    // B-01: include attaches the referenced record (one level).
    let (_, r) = api.get("/api/products?include=shop&sort=title&limit=1", None).await;
    assert_eq!(r["items"][0]["included"]["shop"]["data"]["name"], "Hammer & Co");
    let (_, one) = api.get(&format!("/api/products/{}?include=shop", product["Copper pan"]), None).await;
    assert_eq!(one["included"]["shop"]["id"], us_id);

    // Reviews: counts by relation, restrict on delete, relational read ownership (B-09/B-21).
    for (title, rating) in [("Anvil", 5), ("Anvil", 3), ("Copper pan", 5)] {
        let (s, row) = api.post("/api/reviews", &buyer, json!({"product": product[title], "rating": rating})).await;
        assert_eq!(s, 200, "{row}");
    }
    let (_, r) = api.get("/api/products?count=reviews,five_star&sort=title", Some(&rival)).await;
    assert_eq!(r["items"][0]["counts"], json!({"reviews": 0, "five_star": 0}), "counts follow the reader's scope of reviews");
    let (_, r) = api.get("/api/products?count=reviews&sort=title", Some(&buyer)).await;
    assert_eq!(r["items"][0]["counts"], json!({"reviews": 2}), "authors count their own reviews");
    let (_, r) = api.get("/api/products?count=reviews,five_star&sort=title", Some(&owner)).await;
    assert_eq!(r["items"][0]["data"]["title"], "Anvil");
    assert_eq!(r["items"][0]["counts"], json!({"reviews": 2, "five_star": 1}));
    let (s, r) = api.get("/api/reviews?limit=100", Some(&owner)).await;
    assert_eq!((s, r["total"].clone()), (200, json!(3)), "the shop owner reads reviews of its products: {r}");
    let (_, r) = api.get("/api/reviews?limit=100", Some(&rival)).await;
    assert_eq!(r["total"], 0);
    let (s, _) = api.get("/api/reviews", None).await;
    assert_eq!(s, 403);
    let (s, e) = api.call("DELETE", &format!("/api/products/{}", product["Anvil"]), Some(&owner), None).await;
    assert_eq!(s, 409, "restrict keeps referenced rows: {e}");
    assert_eq!(e["error"]["code"], "REFERENCE_IN_USE");
    let (s, _) = api.call("DELETE", &format!("/api/products/{}", product["Anvil"]), Some(&rival), None).await;
    assert_eq!(s, 404, "a non-owner cannot delete through the relation");

    // set_null: deleting the product clears the order's reference in both the column and the record.
    let (_, order) = api.post("/api/orders", &buyer, json!({"product": product["Garden hose"], "quantity": 2})).await;
    let (s, _) = api.call("DELETE", &format!("/api/products/{}", product["Garden hose"]), Some(&owner), None).await;
    assert_eq!(s, 200);
    let (_, order) = api.get(&format!("/api/orders/{}", order["id"].as_str().unwrap()), Some(&buyer)).await;
    assert!(order["data"].get("product").is_none(), "{order}");
    assert_eq!(order["data"]["quantity"], 2);

    // cascade: deleting the shop deletes its products (and needs the restricting reviews gone first).
    let (s, _) = api.call("DELETE", &format!("/api/shops/{us_id}"), Some(&owner), None).await;
    assert_eq!(s, 409, "the cascade is blocked by restricted reviews on Copper pan");
    sqlx::query("DELETE FROM _brain_res_reviews").execute(&pool).await.unwrap();
    let (s, _) = api.call("DELETE", &format!("/api/shops/{us_id}"), Some(&owner), None).await;
    assert_eq!(s, 200);
    let (_, r) = api.get("/api/products?limit=100", None).await;
    assert_eq!(r["total"], 3);

    // Migration is idempotent and verification accepts the installed keys.
    db::migrate(&pool, &catalog()).await.unwrap();
    db::verify(&pool, &catalog()).await.unwrap();
    // Dropping a key is drift: serving refuses it.
    sqlx::raw_sql("DO $$ DECLARE c text; BEGIN SELECT conname INTO c FROM pg_constraint WHERE conrelid='_brain_res_orders'::regclass AND contype='f' AND conname LIKE '_brain_fk_%'; EXECUTE format('ALTER TABLE _brain_res_orders DROP CONSTRAINT %I', c); END $$")
        .execute(&pool)
        .await
        .unwrap();
    assert!(db::verify(&pool, &catalog()).await.unwrap_err().contains("BDB_SCHEMA_DRIFT"));
    let _ = owner_id;
    pool.close().await;
}

#[test]
fn contracts_reject_unknown_targets_unsafe_actions_and_restricted_query_fields() {
    let mut r = catalog();
    assert!(brain_backend::contract::validate_graph(&r, &[]).is_ok());
    r[1].fields.get_mut("shop").unwrap().on_delete = Some("explode".into());
    assert!(r[1].validate().is_err());
    let mut r = catalog();
    r[3].fields.get_mut("quantity").unwrap().on_delete = Some("cascade".into());
    assert!(r[3].validate().unwrap_err().contains("only to ref"));
    let mut r = catalog();
    r[2].fields.insert("shop".into(), field("ref:warehouses", false));
    assert!(brain_backend::contract::validate_graph(&r, &[]).unwrap_err().contains("unknown resource"));
    let mut r = catalog();
    r[1].query.filter.insert("cost".into(), vec!["eq".into()]);
    assert!(r[1].validate().unwrap_err().contains("restricted visibility"));
    let mut r = catalog();
    r[1].query.filter.insert("price".into(), vec!["prefix".into()]);
    assert!(r[1].validate().is_err());
    let mut r = catalog();
    r[2].policy.read.push("owner.rating".into());
    assert!(brain_backend::contract::validate_graph(&r, &[]).unwrap_err().contains("not a ref"));
    let mut r = catalog();
    r[1].query.filter.insert("shop.name".into(), vec!["eq".into()]);
    assert!(brain_backend::contract::validate_graph(&r, &[]).unwrap_err().contains("query filter"));
}
