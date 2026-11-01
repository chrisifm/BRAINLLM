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
    /// A new session, which carries the role the account holds now.
    async fn user_login(&self, email: &str) -> (Uuid, String) {
        let credentials = Credentials { email: email.into(), password: "correct horse battery".into() };
        let (user, token) = self.auth.login(credentials).await.unwrap().unwrap();
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

/// Documents large enough to live out of line, where each read of `data` decompresses it again.
#[tokio::test]
#[ignore]
async fn counts_over_large_documents_agree_with_their_filters() {
    let posts = resource("posts", vec![("status", field("str")), ("kind", field("str")), ("rank", field("int")), ("body", field("str"))], json!(["public"]), json!({
        "filter": {"status": ["eq", "in"], "kind": ["eq", "in"], "rank": ["gte"]},
        "aggregate": {"ranks": {"group": "kind", "sum": ["rank"]}},
        "facets": ["kind"],
    }));
    let a = api("brain_shape_count", vec![posts]).await;
    let (_, token) = a.user("count@example.test").await;
    // Not compressible: a repeated text would shrink back into the row and never leave it.
    let body = |seed: u64| {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..12_000).map(|_| { state ^= state << 13; state ^= state >> 7; state ^= state << 17; char::from(b'a' + (state % 26) as u8) }).collect::<String>()
    };
    for i in 0..12u64 {
        let status = if i % 3 == 0 { "draft" } else { "live" };
        let kind = if i % 2 == 0 { "even" } else { "odd" };
        a.post("/api/posts", &token, json!({"status": status, "kind": kind, "rank": i, "body": body(i + 1)})).await;
    }
    let toast: bool = sqlx::query_scalar("SELECT pg_relation_size(reltoastrelid)>pg_relation_size(oid) FROM pg_class WHERE oid='_brain_res_posts'::regclass").fetch_one(&a.pool).await.unwrap();
    assert!(toast, "the documents must live out of line for this test to reach the single-read count");
    let (status, list) = a.get("/api/posts?status=live&kind[in]=even,odd&rank[gte]=2&limit=1&aggregate=ranks&facets=kind").await;
    assert_eq!(status, 200, "{list}");
    // Live ranks from 2 up: 2,4,5,7,8,10,11.
    assert_eq!(list["total"], 7, "{list}");
    let even = list["aggregates"]["ranks"].as_array().unwrap().iter().find(|g| g["key"] == "even").unwrap().clone();
    assert_eq!((&even["count"], &even["sum"]["rank"]), (&json!(4), &json!("24")), "{list}");
    assert_eq!(list["facets"]["kind"].as_array().unwrap().len(), 2, "{list}");
    // `fields` leaves the document out of the page without changing the row set.
    let (_, trimmed) = a.get("/api/posts?status=live&rank[gte]=2&limit=1&fields=rank").await;
    assert_eq!(trimmed["total"], 7);
    assert_eq!(trimmed["items"][0]["data"].as_object().unwrap().len(), 1, "{trimmed}");

    // A value that fills the text key must not match a longer text that shares it.
    let exact = "k".repeat(256);
    a.post("/api/posts", &token, json!({"status": exact, "kind": "even", "rank": 1})).await;
    a.post("/api/posts", &token, json!({"status": format!("{exact}z"), "kind": "even", "rank": 1})).await;
    let (status, found) = a.get(&format!("/api/posts?status={exact}&rank[gte]=0")).await;
    assert_eq!((status, found["total"].as_i64()), (200, Some(1)), "{found}");
    assert_eq!(found["items"][0]["data"]["status"], exact.as_str());
}

/// Every client write path keeps server-set fields out: the single-record routes always did, batch and upsert did not.
#[tokio::test]
#[ignore]
async fn field_write_access_holds_on_single_batch_and_upsert_writes() {
    let mut tier = field("str");
    tier.access = Some(serde_json::from_value(json!({"write": ["admin"]})).unwrap());
    let locked = Field { access: Some("readonly".into()), ..field("str") };
    let hidden = Field { access: Some("private".into()), ..field("str") };
    let code = Field { unique: true, ..field("str") };
    let mut items = resource("items", vec![("code", code), ("title", field("str")), ("tier", tier), ("state", locked), ("note", hidden)], json!(["public"]), json!({}));
    items.policy.create = vec!["user".into(), "admin".into()];
    items.policy.update = vec!["owner".into(), "admin".into()];
    let a = api("brain_shape_write", vec![items]).await;
    let (member, member_token) = a.user("member@example.test").await;
    let (boss, boss_token) = a.user("boss@example.test").await;
    sqlx::query("UPDATE _brain_users SET role='admin' WHERE id=$1").bind(boss).execute(&a.pool).await.unwrap();
    let (_, boss_token) = { let _ = boss_token; a.user_login("boss@example.test").await };
    let sends = |status: u16, body: &Value, path: &str, what: &str| {
        assert_eq!(status, 422, "{what}: {body}");
        assert!(body.pointer(path).is_some(), "{what}: {body}");
    };

    // A client never sets readonly or private, by any route and with any role.
    for (sender, token) in [("member", &member_token), ("admin", &boss_token)] {
        for (name, value) in [("state", "open"), ("note", "x")] {
            let (status, body) = a.call("POST", "/api/items", Some(token), None, Some(json!({"code": "a", name: value}))).await;
            sends(status, &body, &format!("/error/fields/{name}"), &format!("{sender} create {name}"));
            let (status, body) = a.call("POST", "/api/items/batch", Some(token), None, Some(json!({"op": "create", "items": [{"code": "a", name: value}]}))).await;
            sends(status, &body, &format!("/error/fields/items.0.{name}"), &format!("{sender} batch create {name}"));
            let (status, body) = a.call("POST", "/api/items/batch", Some(token), None, Some(json!({"op": "upsert", "key": ["code"], "items": [{"code": "a", name: value}]}))).await;
            sends(status, &body, &format!("/error/fields/items.0.{name}"), &format!("{sender} batch upsert {name}"));
            let (status, body) = a.call("POST", "/api/items/upsert", Some(token), None, Some(json!({"key": ["code"], "data": {"code": "a", name: value}}))).await;
            sends(status, &body, &format!("/error/fields/{name}"), &format!("{sender} upsert {name}"));
        }
    }
    let (_, count) = a.get("/api/items").await;
    assert_eq!(count["total"], 0, "nothing was written: {count}");

    // `{ write: [roles] }`: those roles send it on every route, other clients on none.
    let (status, body) = a.call("POST", "/api/items/batch", Some(&member_token), None, Some(json!({"op": "create", "items": [{"code": "m", "tier": "gold"}]}))).await;
    sends(status, &body, "/error/fields/items.0.tier", "member batch tier");
    let (status, body) = a.call("POST", "/api/items", Some(&member_token), None, Some(json!({"code": "m", "tier": "gold"}))).await;
    sends(status, &body, "/error/fields/tier", "member create tier");
    let mine = a.post("/api/items", &member_token, json!({"code": "m", "title": "Mine"})).await;
    let uri = format!("/api/items/{}", id(&mine));
    let (status, body) = a.call("PATCH", &uri, Some(&member_token), None, Some(json!({"tier": "gold"}))).await;
    sends(status, &body, "/error/fields/tier", "member patch tier");
    let (status, body) = a.call("POST", "/api/items/batch", Some(&member_token), None, Some(json!({"op": "update", "items": [{"id": id(&mine), "data": {"tier": "gold"}}]}))).await;
    sends(status, &body, "/error/fields/items.0.tier", "member batch update tier");
    let (status, body) = a.call("POST", "/api/items/batch", Some(&boss_token), None, Some(json!({"op": "update", "items": [{"id": id(&mine), "data": {"tier": "gold"}}]}))).await;
    assert_eq!((status, &body["items"][0]["data"]["tier"]), (200, &json!("gold")), "{body}");
    let (status, body) = a.call("POST", "/api/items/upsert", Some(&boss_token), None, Some(json!({"key": ["code"], "data": {"code": "b", "tier": "silver"}}))).await;
    assert_eq!((status, &body["record"]["data"]["tier"]), (200, &json!("silver")), "{body}");
    // The owner edits the rest and the value stays; every reader sees it.
    let (status, body) = a.call("PATCH", &uri, Some(&member_token), None, Some(json!({"title": "Renamed"}))).await;
    assert_eq!((status, &body["data"]["tier"]), (200, &json!("gold")), "{body}");
    let (_, read) = a.get(&uri).await;
    assert_eq!(read["data"]["tier"], "gold");

    // A system key stays the way to set server fields in bulk.
    let system = a.key(member, &["items:write", "items:read"], true).await;
    let (status, body) = a.call("POST", "/api/items/batch", None, Some(&system), Some(json!({"op": "create", "items": [{"code": "s", "state": "open", "note": "kept"}]}))).await;
    assert_eq!(status, 200, "{body}");
}
#[test]
fn field_write_roles_are_validated() {
    let with = |access: Value| {
        let mut tier = field("str");
        tier.access = Some(serde_json::from_value(access).unwrap());
        resource("items", vec![("tier", tier)], json!(["public"]), json!({})).validate()
    };
    assert!(with(json!({"write": ["admin"]})).is_ok());
    assert!(with(json!({"write": []})).unwrap_err().contains("write needs"));
    assert!(with(json!("nope")).unwrap_err().contains("is invalid"));
}
/// `pattern` governs each element of a list and each key of an object, on the write paths that validate a value.
#[tokio::test]
#[ignore]
async fn patterns_govern_list_elements_and_object_keys() {
    let tags = Field { pattern: Some("[a-z]+".into()), ..field("list<str>") };
    let meta = Field { pattern: Some("[a-z_]+".into()), ..field("obj") };
    let a = api("brain_shape_pattern", vec![resource("notes", vec![("tags", tags), ("meta", meta)], json!(["public"]), json!({}))]).await;
    let (_, token) = a.user("pattern@example.test").await;
    let (status, body) = a.call("POST", "/api/notes", Some(&token), None, Some(json!({"tags": ["ok", "Not OK"]}))).await;
    assert_eq!((status, &body["error"]["fields"]["tags"]), (422, &json!("Invalid format")), "{body}");
    let (status, body) = a.call("POST", "/api/notes", Some(&token), None, Some(json!({"meta": {"good_key": 1, "Bad-Key": 2}}))).await;
    assert_eq!((status, &body["error"]["fields"]["meta"]), (422, &json!("Invalid format")), "{body}");
    let (status, body) = a.call("POST", "/api/notes/batch", Some(&token), None, Some(json!({"op": "create", "items": [{"tags": ["NOPE"]}]}))).await;
    assert_eq!(status, 422, "{body}");
    let note = a.post("/api/notes", &token, json!({"tags": ["ok", "fine"], "meta": {"good_key": {"Any": "nested"}}})).await;
    let (status, body) = a.call("PATCH", &format!("/api/notes/{}", id(&note)), Some(&token), None, Some(json!({"tags": ["NOPE"]}))).await;
    assert_eq!(status, 422, "{body}");
    let bad = Field { pattern: Some("x".into()), ..field("int") };
    assert!(resource("n", vec![("rank", bad)], json!(["public"]), json!({})).validate().unwrap_err().contains("only to a str, list<str> or obj"));
}

#[tokio::test]
#[ignore]
async fn one_record_per_owner_holds_on_every_write_and_with_a_role_exception() {
    let writable = |name: &str| {
        let mut r = resource(name, vec![("title", field("str"))], json!(["public"]), json!({}));
        r.policy.create = vec!["user".into(), "admin".into()];
        r.policy.update = vec!["owner".into(), "admin".into()];
        r
    };
    let mut strict = writable("profiles");
    strict.constraints.unique = vec![vec!["$owner".into()]];
    let mut lenient = writable("companies");
    lenient.constraints.per_owner = Some(brain_backend::contract::PerOwner { except: vec!["admin".into()] });
    let mut scoped = writable("slots");
    scoped.constraints.unique = vec![vec!["$owner".into(), "title".into()]];
    let a = api("brain_shape_owner", vec![strict, lenient, scoped]).await;
    let (_, member) = a.user("member@example.test").await;
    let (boss, _) = a.user("boss@example.test").await;
    sqlx::query("UPDATE _brain_users SET role='admin' WHERE id=$1").bind(boss).execute(&a.pool).await.unwrap();
    let (_, boss_token) = a.user_login("boss@example.test").await;
    let (_, other) = a.user("other@example.test").await;
    let post = |uri: &'static str, token: String, body: Value| {
        let a = &a;
        async move { a.call("POST", uri, Some(&token), None, Some(body)).await }
    };

    for uri in ["/api/profiles", "/api/companies"] {
        let (status, body) = post(uri, member.clone(), json!({"title": "first"})).await;
        assert_eq!(status, 200, "{uri}: {body}");
        let (status, body) = post(uri, member.clone(), json!({"title": "second"})).await;
        assert_eq!(status, 409, "{uri}: {body}");
        assert_eq!(body["error"]["code"], "UNIQUE_CONFLICT", "{uri}: {body}");
        assert!(body["error"]["fields"].get("owner").is_some(), "{uri}: {body}");
        let (status, body) = post(uri, other.clone(), json!({"title": "first"})).await;
        assert_eq!(status, 200, "another owner is free: {uri}: {body}");
    }
    let batch = |uri: &'static str| format!("{uri}/batch");
    for uri in ["/api/profiles", "/api/companies"] {
        let (_, fresh) = a.user(&format!("fresh{}@example.test", uri.len())).await;
        let (status, body) = a.call("POST", &batch(uri), Some(&fresh), None, Some(json!({"op": "create", "items": [{"title": "a"}, {"title": "b"}]}))).await;
        assert_eq!(status, 409, "a batch cannot go around it: {uri}: {body}");
    }

    // The exception is the owner's role, and only where it was declared.
    for title in ["a", "b"] {
        let (status, body) = post("/api/companies", boss_token.clone(), json!({"title": title})).await;
        assert_eq!(status, 200, "{body}");
    }
    let (status, _) = post("/api/profiles", boss_token.clone(), json!({"title": "a"})).await;
    assert_eq!(status, 200);
    let (status, body) = post("/api/profiles", boss_token.clone(), json!({"title": "b"})).await;
    assert_eq!(status, 409, "{body}");

    // With other fields, the set is per owner: another title is free, the same title is not.
    let (status, _) = post("/api/slots", member.clone(), json!({"title": "a"})).await;
    assert_eq!(status, 200);
    let (status, _) = post("/api/slots", member.clone(), json!({"title": "b"})).await;
    assert_eq!(status, 200);
    let (status, body) = post("/api/slots", member.clone(), json!({"title": "a"})).await;
    assert_eq!(status, 409, "{body}");
    let (status, _) = post("/api/slots", other.clone(), json!({"title": "a"})).await;
    assert_eq!(status, 200);

    // The owner is not a value a client can name as an upsert key.
    let (status, body) = a.call("POST", "/api/profiles/upsert", Some(&other), None, Some(json!({"key": ["$owner"], "data": {"title": "x"}}))).await;
    assert_eq!(status, 422, "{body}");

    // Updating an existing record is not a second record.
    let (_, fresh) = a.user("updater@example.test").await;
    let (_, created) = post("/api/companies", fresh.clone(), json!({"title": "one"})).await;
    let (status, body) = a.call("PATCH", &format!("/api/companies/{}", id(&created)), Some(&fresh), None, Some(json!({"title": "two"}))).await;
    assert_eq!(status, 200, "{body}");

    // Rows already over the limit refuse the migration.
    let over: Vec<Resource> = { let mut p = writable("slots"); p.constraints.per_owner = Some(brain_backend::contract::PerOwner { except: vec!["admin".into()] }); vec![p] };
    let refused = brain_backend::db::migrate_approving(&a.pool, &over, &["slots".to_string()]).await.unwrap_err();
    assert!(refused.contains("more than one record"), "{refused}");
}
