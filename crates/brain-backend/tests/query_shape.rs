//! The shape of a list answer: nested includes, `fields=`, declared `or` and `owner=me`. Each level and option keeps the
//! read policy, field visibility and key scopes of the plain list.
use axum::{Router, body::{Body, to_bytes}, http::Request};
use brain_backend::{
    api_keys::{Keys, NewKey},
    auth::{Auth, Credentials},
    contract::{Field, Hooks, Policy, Resource},
    db,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, str::FromStr};
use sqlx::ConnectOptions;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5190";

fn field(kind: &str) -> Field {
    Field { kind: kind.into(), ..Default::default() }
}
fn resource(name: &str, fields: Vec<(&str, Field)>, read: Value, query: Value) -> Resource {
    let policy: Policy = serde_json::from_value(json!({"read": read, "create": ["user"], "update": ["owner"], "delete": ["owner"]})).unwrap();
    Resource {
        name: name.into(),
        search: vec![],
        fields: fields.into_iter().map(|(n, f)| (n.to_string(), f)).collect(),
        policy,
        hooks: Hooks::default(),
        constraints: Default::default(),
        query: serde_json::from_value(query).unwrap(),
    }
}
/// regions <- shops <- products <- reviews. Shops are public only while `open`; each hides a private field.
fn graph() -> Vec<Resource> {
    let hidden = Field { access: Some("private".into()), ..field("str") };
    let mut cost = field("str");
    cost.visible = Some(vec!["owner".into()]);
    vec![
        resource("regions", vec![("name", field("str")), ("note", hidden.clone())], json!(["public"]), json!({})),
        resource("shops", vec![("region", field("ref:regions")), ("name", field("str")), ("status", field("enum(open|closed)")), ("note", hidden.clone())], json!([{"public": {"where": {"status": "open"}}}]), json!({"include": ["region"]})),
        resource("products", vec![("shop", field("ref:shops")), ("title", field("str")), ("summary", field("str")), ("cost", cost), ("rank", field("int")), ("note", hidden)], json!(["public"]), json!({
            "include": ["shop"],
            "filter": {"title": ["eq"], "rank": ["gte"], "shop": ["eq"]},
            "sort": ["rank"],
        })),
        resource("reviews", vec![("product", field("ref:products")), ("body", field("str"))], json!(["public"]), json!({"include": ["product"]})),
    ]
}
struct Api { http: Router, auth: Auth, keys: Keys, pool: sqlx::PgPool }
impl Api {
    async fn call(&self, method: &str, uri: &str, token: Option<&str>, key: Option<&str>, body: Option<Value>) -> (u16, Value) {
        let mut b = Request::builder().extension(axum::extract::ConnectInfo("127.0.0.1:3456".parse::<std::net::SocketAddr>().unwrap())).method(method).uri(uri).header("origin", ORIGIN);
        if let Some(token) = token { b = b.header("cookie", format!("__Host-brain={token}")); }
        if let Some(key) = key { b = b.header("x-api-key", key); }
        let req = match body { Some(body) => b.header("content-type", "application/json").body(Body::from(body.to_string())), None => b.body(Body::empty()) }.unwrap();
        let res = self.http.clone().oneshot(req).await.unwrap();
        let status = res.status().as_u16();
        let bytes = to_bytes(res.into_body(), 1 << 22).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
    async fn get(&self, uri: &str) -> (u16, Value) { self.call("GET", uri, None, None, None).await }
    async fn post(&self, uri: &str, token: &str, body: Value) -> Value {
        let (status, value) = self.call("POST", uri, Some(token), None, Some(body)).await;
        assert!(status < 300, "{uri}: {status} {value}");
        value
    }
    async fn user(&self, email: &str) -> (Uuid, String) {
        let credentials = || Credentials { email: email.into(), password: "correct horse battery".into() };
        let user = self.auth.signup(credentials()).await.unwrap();
        let (_, token) = self.auth.login(credentials()).await.unwrap().unwrap();
        (user.id, token)
    }
    async fn key(&self, owner: Uuid, scopes: &[&str], system: bool) -> String {
        let new = NewKey { name: "shape".into(), scopes: scopes.iter().map(|s| s.to_string()).collect(), expires_in_days: None, daily_quota: None, monthly_quota: None };
        let created = if system { self.keys.create_system(owner, new).await } else { self.keys.create(owner, new).await };
        created.unwrap().unwrap().1
    }
}
async fn api(prefix: &str, resources: Vec<Resource>) -> Api {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let base = db::connect(&url).await.unwrap();
    let name = format!("{prefix}_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name);
    let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
    base.close().await;
    db::migrate(&pool, &resources).await.unwrap();
    brain_backend::auth_flows::RolesConfig::default().register(&pool).await.unwrap();
    let http = brain_backend::http::router(pool.clone(), brain_backend::Config {
        api_keys: Some(Default::default()), roles: Default::default(), mail: None, auth_public_base_url: None, webhooks: None, before_hooks: None, sockets: None, gcs: None, jwt: None, google: None,
        resources, jobs: Vec::new(), uploads: std::env::temp_dir().join(format!("brain-query-shape-{}", Uuid::new_v4().simple())), origins: vec![ORIGIN.into()], production: false,
    }).await.unwrap();
    Api { http, auth: Auth::new(pool.clone()).await.unwrap(), keys: Keys::new(pool.clone(), Default::default()).unwrap(), pool }
}
fn id(v: &Value) -> String { v["id"].as_str().unwrap().to_string() }

struct Seeded { owner: Uuid, token: String, open_review: String, closed_review: String, product: String }
async fn seed(a: &Api) -> Seeded {
    let (owner, token) = a.user("owner@example.test").await;
    let region = id(&a.post("/api/regions", &token, json!({"name": "North"})).await);
    let open = id(&a.post("/api/shops", &token, json!({"region": region, "name": "Open shop", "status": "open"})).await);
    let closed = id(&a.post("/api/shops", &token, json!({"region": region, "name": "Closed shop", "status": "closed"})).await);
    let p1 = id(&a.post("/api/products", &token, json!({"shop": open, "title": "One", "summary": "s1", "cost": "9", "rank": 1})).await);
    let p2 = id(&a.post("/api/products", &token, json!({"shop": closed, "title": "Two", "summary": "s2", "cost": "9", "rank": 2})).await);
    let open_review = id(&a.post("/api/reviews", &token, json!({"product": p1, "body": "good"})).await);
    let closed_review = id(&a.post("/api/reviews", &token, json!({"product": p2, "body": "bad"})).await);
    Seeded { owner, token, open_review, closed_review, product: p1 }
}

#[tokio::test]
#[ignore]
async fn nested_includes_apply_policy_visibility_scopes_and_bounds() {
    let resources = graph();
    let a = api("brain_shape_inc", resources.clone()).await;
    let s = seed(&a).await;
    let (status, list) = a.get("/api/reviews?include=product.shop.region&sort=created_at").await;
    assert_eq!(status, 200, "{list}");
    let item = |rid: &str| list["items"].as_array().unwrap().iter().find(|i| i["id"] == rid).unwrap().clone();
    let open = item(&s.open_review);
    let shop = &open["included"]["product"]["included"]["shop"];
    assert_eq!(shop["data"]["name"], "Open shop");
    assert_eq!(shop["included"]["region"]["data"]["name"], "North");
    // Every level projects: private and owner-only fields do not travel inside `included`.
    assert!(shop["included"]["region"]["data"].get("note").is_none());
    assert!(shop["data"].get("note").is_none());
    assert!(open["included"]["product"]["data"].get("cost").is_none());
    // A row the middle level hides renders as null, like a missing one, and nothing below it is read.
    let closed = item(&s.closed_review);
    assert_eq!(closed["included"]["product"]["included"]["shop"], Value::Null, "{closed}");

    // Undeclared levels, too many levels and field names are refused.
    for bad in ["product.title", "product.shop.region.name", "shop", "product..shop"] {
        let (status, body) = a.get(&format!("/api/reviews?include={bad}")).await;
        assert_eq!(status, 422, "{bad}: {body}");
    }
    let (status, one) = a.get(&format!("/api/reviews/{}?include=product.shop", s.open_review)).await;
    assert_eq!(status, 200, "{one}");
    assert_eq!(one["included"]["product"]["included"]["shop"]["data"]["name"], "Open shop");
    let (status, _) = a.get(&format!("/api/reviews/{}?include=product.shop.region.name", s.open_review)).await;
    assert_eq!(status, 422);

    // The record budget refuses instead of truncating.
    let mut rows = list["items"].as_array().unwrap().clone();
    let map: BTreeMap<String, Resource> = resources.into_iter().map(|r| (r.name.clone(), r)).collect();
    let viewer = brain_backend::visibility::Viewer::default();
    let result = brain_backend::relations::include_within(&a.pool, &map, &map["reviews"], &["product.shop".to_string()], &mut rows, viewer, 1).await;
    assert!(matches!(result, Err(brain_backend::relations::IncludeError::TooMany)), "{result:?}");

    // A key's scopes narrow every level; a system key does not escape them.
    for system in [false, true] {
        let narrow = a.key(s.owner, &["reviews:read"], system).await;
        let (status, body) = a.call("GET", "/api/reviews?include=product", None, Some(&narrow), None).await;
        assert_eq!(status, 403, "system={system}: {body}");
        let two = a.key(s.owner, &["reviews:read", "products:read"], system).await;
        let (status, _) = a.call("GET", "/api/reviews?include=product", None, Some(&two), None).await;
        assert_eq!(status, 200);
        let (status, _) = a.call("GET", "/api/reviews?include=product.shop", None, Some(&two), None).await;
        assert_eq!(status, 403, "system={system}");
    }
    // A system key reading a level sees rows the policy hides, and only through scopes that name it.
    let wide = a.key(s.owner, &["reviews:read", "products:read", "shops:read"], true).await;
    let (status, one) = a.call("GET", &format!("/api/reviews/{}?include=product.shop", s.closed_review), None, Some(&wide), None).await;
    assert_eq!(status, 200, "{one}");
    assert_eq!(one["included"]["product"]["included"]["shop"]["data"]["name"], "Closed shop");
    let _ = (&s.token, &s.product);
}

#[tokio::test]
#[ignore]
async fn fields_narrow_lists_without_revealing_or_breaking_anything() {
    let a = api("brain_shape_fld", graph()).await;
    let s = seed(&a).await;
    let (status, list) = a.get("/api/products?fields=title,rank&sort=rank").await;
    assert_eq!(status, 200, "{list}");
    let one = &list["items"][0];
    assert_eq!(one["data"], json!({"title": "One", "rank": 1}));
    assert!(one["id"].is_string() && one["created_at"].is_string());
    assert_eq!(list["total"], 2);

    // Private fields, undeclared names and an empty name answer the same 422; nothing reveals which exists.
    let (_, unknown) = a.get("/api/products?fields=nothing").await;
    let (status, private) = a.get("/api/products?fields=note").await;
    assert_eq!(status, 422, "{private}");
    assert_eq!(a.get("/api/products?fields=").await.0, 422);
    let strip = |v: &Value| v.pointer("/error/fields/fields").or_else(|| v.pointer("/fields/fields")).expect("field error").to_string().replace("nothing", "X").replace("note", "X");
    assert!(unknown.to_string().contains("cannot be selected"), "{unknown}");
    assert_eq!(strip(&unknown), strip(&private));
    // A field only some readers see stays hidden when named by others, and appears for its owner.
    let (status, anon) = a.get("/api/products?fields=title,cost").await;
    assert_eq!(status, 200);
    assert!(anon["items"][0]["data"].get("cost").is_none(), "{anon}");
    let (_, own) = a.call("GET", "/api/products?fields=cost&sort=rank", Some(&s.token), None, None).await;
    assert_eq!(own["items"][0]["data"], json!({"cost": "9"}), "{own}");
    // The policy still filters rows: fields never widens what the row condition hides.
    let (_, shops) = a.get("/api/shops?fields=name").await;
    assert_eq!(shops["total"], 1, "{shops}");

    // Includes follow a reference that `fields` drops; cursors keep paging.
    let (status, nested) = a.get("/api/products?fields=title&include=shop&sort=rank").await;
    assert_eq!(status, 200);
    assert_eq!(nested["items"][0]["data"], json!({"title": "One"}));
    assert_eq!(nested["items"][0]["included"]["shop"]["data"]["name"], "Open shop");
    let (_, page) = a.get("/api/products?fields=title&sort=rank&limit=1").await;
    let cursor = page["next_cursor"].as_str().unwrap();
    let (_, next) = a.get(&format!("/api/products?fields=title&sort=rank&limit=1&cursor={cursor}")).await;
    assert_eq!(next["items"][0]["data"], json!({"title": "Two"}));

    // A system key may name a private field within its scope; an ordinary key may not.
    let system = a.key(s.owner, &["products:read"], true).await;
    let ordinary = a.key(s.owner, &["products:read"], false).await;
    assert_eq!(a.call("GET", "/api/products?fields=note", None, Some(&system), None).await.0, 200);
    assert_eq!(a.call("GET", "/api/products?fields=note", None, Some(&ordinary), None).await.0, 422);
}

fn listings() -> Vec<Resource> {
    vec![resource("listings", vec![("title", field("str")), ("expires_on", field("date")), ("source", field("enum(direct|legacy)")), ("rank", field("int")), ("secret", Field { access: Some("private".into()), ..field("str") })], json!(["public"]), json!({
        "filter": {"rank": ["gte"]},
        "sort": ["rank"],
        "or": {"live": {"expires_on": ["gte", "exists"], "source": ["eq"]}},
    }))]
}

#[tokio::test]
#[ignore]
async fn declared_or_groups_and_owner_me() {
    let a = api("brain_shape_or", listings()).await;
    let (first, token) = a.user("first@example.test").await;
    let (_, other) = a.user("other@example.test").await;
    for (who, title, expires, source, rank) in [(&token, "future", "2999-01-01", "direct", 1), (&token, "legacy", "2000-01-01", "legacy", 2), (&other, "expired", "2000-01-01", "direct", 3), (&other, "both", "2999-01-01", "legacy", 4)] {
        a.post("/api/listings", who, json!({"title": title, "expires_on": expires, "source": source, "rank": rank})).await;
    }
    let titles = |v: &Value| v["items"].as_array().unwrap().iter().map(|i| i["data"]["title"].as_str().unwrap().to_string()).collect::<Vec<_>>();

    // Alternatives OR with each other and AND with filters; total and pages agree.
    let (status, either) = a.get("/api/listings?live.expires_on[gte]=2026-01-01&live.source=legacy&sort=rank").await;
    assert_eq!(status, 200, "{either}");
    assert_eq!(titles(&either), ["future", "legacy", "both"]);
    assert_eq!(either["total"], 3);
    let (_, narrowed) = a.get("/api/listings?live.expires_on[gte]=2026-01-01&live.source=legacy&rank[gte]=2&sort=rank").await;
    assert_eq!(titles(&narrowed), ["legacy", "both"]);
    let (_, single) = a.get("/api/listings?live.source=legacy&sort=rank").await;
    assert_eq!(titles(&single), ["legacy", "both"]);
    // Injection attempts stay bound values.
    let (status, none) = a.get("/api/listings?live.source=direct%27%20OR%201=1--").await;
    assert_eq!(status, 422, "{none}");
    // Undeclared branches, operators and groups are refused.
    for bad in ["live.title=x", "live.source[ne]=direct", "other.source=legacy", "live.secret=x"] {
        assert_eq!(a.get(&format!("/api/listings?{bad}")).await.0, 422, "{bad}");
    }
    // Every alternative has an index the planner can combine.
    let defs: Vec<String> = sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE tablename='_brain_res_listings'").fetch_all(&a.pool).await.unwrap();
    for name in ["expires_on", "source"] {
        assert!(defs.iter().any(|d| d.contains(name)), "{name}: {defs:?}");
    }

    // owner=me comes from the session; no client value can name someone else, and it never replaces the read scope.
    let (status, mine) = a.call("GET", "/api/listings?owner=me&sort=rank", Some(&token), None, None).await;
    assert_eq!(status, 200, "{mine}");
    assert_eq!(titles(&mine), ["future", "legacy"]);
    assert_eq!(mine["total"], 2);
    assert_eq!(a.get("/api/listings?owner=me").await.0, 401);
    let (status, _) = a.call("GET", &format!("/api/listings?owner={first}"), Some(&other), None, None).await;
    assert_eq!(status, 422);
    let (_, combined) = a.call("GET", "/api/listings?owner=me&live.source=legacy", Some(&token), None, None).await;
    assert_eq!(titles(&combined), ["legacy"]);
    // A key resolves to its owner.
    let key = a.key(first, &["listings:read"], false).await;
    let (_, by_key) = a.call("GET", "/api/listings?owner=me&sort=rank", None, Some(&key), None).await;
    assert_eq!(titles(&by_key), ["future", "legacy"]);
}

#[test]
fn or_groups_are_validated() {
    let bad = |or: Value| {
        let mut r = listings().remove(0);
        r.query.or = serde_json::from_value(or).unwrap();
        r.validate().unwrap_err()
    };
    assert!(bad(json!({"live": {"source": ["eq"]}})).contains("2..4 fields"));
    assert!(bad(json!({"live": {"source": ["eq"], "ghost": ["eq"]}})).contains("ghost"));
    assert!(bad(json!({"live": {"source": ["eq"], "secret": ["eq"]}})).contains("restricted"));
    assert!(bad(json!({"live": {"source": ["eq"], "rank": ["prefix"]}})).contains("does not support"));
    assert!(bad(json!({"rank": {"source": ["eq"], "title": ["eq"]}})).contains("own name"));
    assert!(bad(json!({"owner": {"source": ["eq"], "title": ["eq"]}})).contains("own name"));
}

#[tokio::test]
#[ignore]
async fn max_per_group_is_exact_and_the_group_cap_is_reported() {
    let events = resource("events", vec![("kind", field("str")), ("score", field("int")), ("day", field("date"))], json!(["public"]), json!({
        "aggregate": {"best": {"group": "kind", "max": ["score", "day"], "min": ["score"]}},
    }));
    let a = api("brain_shape_agg", vec![events]).await;
    let (_, token) = a.user("agg@example.test").await;
    for (kind, score, day) in [("a", 1, "2026-01-01"), ("a", 9, "2026-03-01"), ("a", 4, "2026-02-01"), ("b", 7, "2026-05-01"), ("b", 2, "2026-06-01")] {
        a.post("/api/events", &token, json!({"kind": kind, "score": score, "day": day})).await;
    }
    let (status, small) = a.get("/api/events?aggregate=best&limit=1").await;
    assert_eq!(status, 200, "{small}");
    let groups = small["aggregates"]["best"].as_array().unwrap();
    assert_eq!(groups[0], json!({"key": "a", "count": 3, "max": {"score": 9, "day": "2026-03-01"}, "min": {"score": 1}}));
    assert_eq!(groups[1], json!({"key": "b", "count": 2, "max": {"score": 7, "day": "2026-06-01"}, "min": {"score": 2}}));
    assert!(small.get("aggregates_truncated").is_none());

    sqlx::query("INSERT INTO _brain_res_events(id,owner,data) SELECT gen_random_uuid(),(SELECT id FROM _brain_users LIMIT 1),jsonb_build_object('kind','k'||n,'score',n,'day','2026-01-01') FROM generate_series(1,120) n")
        .execute(&a.pool).await.unwrap();
    let (_, large) = a.get("/api/events?aggregate=best&limit=1").await;
    assert_eq!(large["aggregates"]["best"].as_array().unwrap().len(), 100);
    assert_eq!(large["aggregates_truncated"], json!(["best"]));
}

/// `when` runs on the locked row with `$input`, so "this write must change the field" needs no new operator.
#[tokio::test]
#[ignore]
async fn a_guard_can_require_that_the_field_changes() {
    let items = resource("items", vec![("price", field("int"))], json!(["public"]), json!({}));
    let a = api("brain_shape_when", vec![items.clone()]).await;
    let domain: brain_backend::domain::Domain = serde_json::from_value(json!({"resources": {"items": {"actions": {"reprice": {
        "roles": ["owner"], "input": {"price": "int!"}, "when": {"price": {"ne": "$input.price"}}, "set": {"price": "$input.price"}
    }}}}})).unwrap();
    let (http, _) = brain_backend::http::router_with_domain(a.pool.clone(), brain_backend::Config {
        api_keys: None, roles: Default::default(), mail: None, auth_public_base_url: None, webhooks: None, before_hooks: None, sockets: None, gcs: None, jwt: None, google: None,
        resources: vec![items], jobs: Vec::new(), uploads: std::env::temp_dir().join(format!("brain-query-shape-{}", Uuid::new_v4().simple())), origins: vec![ORIGIN.into()], production: false,
    }, domain).await.unwrap();
    let a = Api { http, auth: Auth::new(a.pool.clone()).await.unwrap(), keys: Keys::new(a.pool.clone(), Default::default()).unwrap(), pool: a.pool };
    let (_, token) = a.user("guard@example.test").await;
    let item = id(&a.post("/api/items", &token, json!({"price": 5})).await);
    let action = format!("/api/items/{item}/actions/reprice");
    let (status, same) = a.call("POST", &action, Some(&token), None, Some(json!({"price": 5}))).await;
    assert_eq!(status, 409, "{same}");
    let (status, moved) = a.call("POST", &action, Some(&token), None, Some(json!({"price": 6}))).await;
    assert!(status < 300, "{moved}");
}

#[tokio::test]
#[ignore]
async fn order_without_a_null_position_pages_the_same_and_punctuation_finds_nothing() {
    let mut notes = resource("notes", vec![("title", field("str")), ("rank", field("int"))], json!(["public"]), json!({"sort": ["rank", "updated_at", {"field": "title", "nulls": "first"}]}));
    notes.search = vec!["title".into()];
    let a = api("brain_shape_order", vec![notes]).await;
    let (_, token) = a.user("order@example.test").await;
    for (title, rank) in [("% off", json!(1)), ("alpha", json!(3)), ("bravo", Value::Null), ("charlie", json!(2)), ("qa desk", json!(4))] {
        let body = if rank.is_null() { json!({"title": title}) } else { json!({"title": title, "rank": rank}) };
        a.post("/api/notes", &token, body).await;
    }
    let walk = |sort: &'static str| {
        let a = &a;
        async move {
            let (mut seen, mut cursor) = (Vec::new(), None::<String>);
            loop {
                let after = cursor.as_ref().map_or(String::new(), |c| format!("&cursor={c}"));
                let (status, page) = a.get(&format!("/api/notes?limit=2&sort={sort}{after}")).await;
                assert_eq!(status, 200, "{page}");
                seen.extend(page["items"].as_array().unwrap().iter().map(|i| i["data"]["title"].as_str().unwrap().to_string()));
                match page["next_cursor"].as_str() {
                    Some(next) => cursor = Some(next.to_string()),
                    None => return seen,
                }
            }
        }
    };
    // Timestamps have no null position to name; a cursor still walks each row once, in both directions.
    let newest = walk("-created_at").await;
    assert_eq!(newest, ["qa desk", "charlie", "bravo", "alpha", "% off"]);
    assert_eq!(walk("created_at").await, ["% off", "alpha", "bravo", "charlie", "qa desk"]);
    assert_eq!(walk("-updated_at").await, newest);
    // A field that can be empty keeps its declared position in both directions.
    assert_eq!(walk("rank").await, ["% off", "charlie", "alpha", "qa desk", "bravo"]);
    assert_eq!(walk("-rank").await, ["qa desk", "alpha", "charlie", "% off", "bravo"]);
    // Both directions of a declared sort have an index, whichever side its nulls are on.
    let indexed: Vec<String> = sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE tablename='_brain_res_notes'").fetch_all(&a.pool).await.unwrap();
    assert!(indexed.iter().any(|d| d.contains("rank") && d.contains("DESC NULLS LAST")), "{indexed:?}");
    assert!(indexed.iter().any(|d| d.contains("title") && d.contains("NULLS FIRST")), "{indexed:?}");

    // Lone punctuation finds nothing without a scan, like longer punctuation; a term keeps its prefix match.
    for text in ["%25", "%5C", "_", "%25%25", "%25%25%25", "%5C%5C%5C"] {
        let (status, found) = a.get(&format!("/api/notes?search={text}")).await;
        assert_eq!((status, found["total"].as_i64()), (200, Some(0)), "{text}: {found}");
    }
    let (_, prefix) = a.get("/api/notes?search=qa").await;
    assert_eq!(prefix["total"], 1, "{prefix}");
    for text in ["100%25", "a%5Cb"] {
        assert_eq!(a.get(&format!("/api/notes?search={text}")).await.0, 200, "{text}");
    }
}
