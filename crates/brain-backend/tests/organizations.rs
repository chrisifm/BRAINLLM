//! Tenant isolation through the real HTTP router against a disposable Postgres. Every test acts as a user of one
//! organization and tries to read or change the data of another; each attempt must fail or see nothing.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use brain_backend::{auth, contract::Resource, db, organizations::Organizations, platform::Platform};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::str::FromStr;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5191";

fn resources() -> Vec<Resource> {
    let member_policy = json!({"read":["org"],"create":["org"],"update":["org.owner","org.admin","org.own"],"delete":["org.owner"]});
    let list: Vec<Value> = serde_json::from_value(json!([
        {"name":"projects","search":["title"],
         "fields":{"org":{"kind":"org","required":true},"title":{"kind":"str","required":true,"unique":true},"tag":{"kind":"str"},"size":{"kind":"int"}},
         "policy":member_policy,
         "query":{"filter":{"org":["eq"],"tag":["eq"]},"sort":["size"],"aggregate":{"stats":{"sum":["size"]},"by_tag":{"group":"tag"}},"count":{"tasks":{"resource":"tasks","field":"project"}}}},
        {"name":"tasks",
         "fields":{"org":{"kind":"org","required":true},"project":{"kind":"ref:projects","required":true},"title":{"kind":"str","required":true}},
         "policy":member_policy,
         "query":{"include":["project"],"filter":{"project":["eq"],"project.tag":["eq"]}}},
        {"name":"bulletins","fields":{"org":{"kind":"org","required":true},"status":{"kind":"enum(draft|live)","required":true},"body":{"kind":"str","required":true}},
         "policy":{"read":[{"public":{"where":{"status":"live"}}},"org"],"create":["org.owner"],"update":["org.owner"],"delete":["org.owner"]}},
        {"name":"catalog","fields":{"name":{"kind":"str","required":true}},"policy":{"read":["public"],"create":["admin"],"update":["admin"],"delete":["admin"]}}
    ])).unwrap();
    list.into_iter().map(|v| serde_json::from_value(v).unwrap()).collect()
}
async fn database() -> sqlx::PgPool {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_orgs_{}", Uuid::new_v4().simple());
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
    brain_backend::auth_flows::RolesConfig { roles: ["admin", "user"].map(String::from).to_vec(), signup_role: "user".into(), ..Default::default() }
}
fn config(resources: Vec<Resource>) -> brain_backend::Config {
    brain_backend::Config {
        vitals: None,
        api_keys: Some(Default::default()), roles: roles(), mail: None, auth_public_base_url: None, webhooks: None, before_hooks: None,
        sockets: None, gcs: None, jwt: None, google: None, apple: None, apple_native: None, linkedin: None, resources, jobs: Vec::new(),
        uploads: std::env::temp_dir().join(format!("brain-orgs-{}", Uuid::new_v4().simple())),
        origins: vec![ORIGIN.into()], production: false,
    }
}
async fn call(http: &Router, method: &str, path: &str, who: Option<&Actor>, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(path).header("origin", ORIGIN)
        .extension(ConnectInfo("127.0.0.1:42031".parse::<std::net::SocketAddr>().unwrap()));
    if let Some(a) = who {
        b = b.header("cookie", format!("{}={}", auth::COOKIE, a.token));
    }
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    };
    let response = http.clone().oneshot(req.unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
struct Fix {
    http: Router,
    pool: sqlx::PgPool,
    alice: Actor,
    bob: Actor,
    global_admin: Actor,
    org_a: String,
    org_b: String,
    project_a: String,
    project_b: String,
}
async fn fixture() -> Fix {
    let pool = database().await;
    let all = resources();
    db::migrate(&pool, &all).await.unwrap();
    roles().register(&pool).await.unwrap();
    let (alice, bob, global_admin) = (actor(&pool, "user").await, actor(&pool, "user").await, actor(&pool, "admin").await);
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(all), Platform::default()).await.unwrap();
    let (s, a) = call(&http, "POST", "/_brain/orgs", Some(&alice), Some(json!({"name":"Org A"}))).await;
    assert_eq!(s, 200, "{a}");
    assert_eq!(a["role"], "owner");
    let (_, b) = call(&http, "POST", "/_brain/orgs", Some(&bob), Some(json!({"name":"Org B"}))).await;
    let (org_a, org_b) = (a["id"].as_str().unwrap().to_string(), b["id"].as_str().unwrap().to_string());
    let (s, pa) = call(&http, "POST", "/api/projects", Some(&alice), Some(json!({"org":org_a,"title":"Shared title","tag":"alpha","size":5}))).await;
    assert_eq!(s, 200, "{pa}");
    let (s, pb) = call(&http, "POST", "/api/projects", Some(&bob), Some(json!({"org":org_b,"title":"Secret of B","tag":"beta","size":700}))).await;
    assert_eq!(s, 200, "{pb}");
    Fix { http, pool, alice, bob, global_admin, org_a, org_b, project_a: pa["id"].as_str().unwrap().into(), project_b: pb["id"].as_str().unwrap().into() }
}
async fn invite_and_join(f: &Fix, manager: &Actor, org: &str, role: &str, who: &Actor) {
    let (s, inv) = call(&f.http, "POST", &format!("/_brain/orgs/{org}/invites"), Some(manager), Some(json!({"role":role}))).await;
    assert_eq!(s, 200, "{inv}");
    let (s, joined) = call(&f.http, "POST", "/_brain/orgs/invites/accept", Some(who), Some(json!({"token":inv["token"]}))).await;
    assert_eq!(s, 200, "{joined}");
    assert_eq!(joined["role"], role);
}
fn titles(list: &Value) -> Vec<String> {
    list["items"].as_array().unwrap().iter().map(|r| r["data"]["title"].as_str().unwrap().to_string()).collect()
}
const IGNORED: &str = "";

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn no_read_channel_crosses_an_organization() {
    let _ = IGNORED;
    let f = fixture().await;
    let (a, b) = (&f.alice, &f.bob);
    let task_b = call(&f.http, "POST", "/api/tasks", Some(b), Some(json!({"org":f.org_b,"project":f.project_b,"title":"B task"}))).await.1;
    // List, explicit filter on the other organization, search and include.
    assert_eq!(titles(&call(&f.http, "GET", "/api/projects", Some(a), None).await.1), vec!["Shared title"]);
    assert!(titles(&call(&f.http, "GET", &format!("/api/projects?org={}", f.org_b), Some(a), None).await.1).is_empty());
    assert!(titles(&call(&f.http, "GET", "/api/projects?search=Secret", Some(a), None).await.1).is_empty());
    assert!(titles(&call(&f.http, "GET", "/api/projects?tag=beta", Some(a), None).await.1).is_empty());
    // Single reads answer exactly like a missing record.
    let hidden = call(&f.http, "GET", &format!("/api/projects/{}", f.project_b), Some(a), None).await;
    let absent = call(&f.http, "GET", &format!("/api/projects/{}", Uuid::new_v4()), Some(a), None).await;
    assert_eq!(hidden.0, 404);
    assert_eq!(hidden.0, absent.0);
    assert_eq!(hidden.1["error"]["code"], absent.1["error"]["code"]);
    assert_eq!(call(&f.http, "GET", &format!("/api/tasks/{}", task_b["id"].as_str().unwrap()), Some(a), None).await.0, 404);
    // Totals and aggregates count only the caller's rows.
    let stats = call(&f.http, "GET", "/api/projects?aggregate=stats,by_tag", Some(a), None).await.1;
    assert!(stats["aggregates"].is_object(), "{stats}");
    let text = stats["aggregates"].to_string();
    assert!(!text.contains("700") && !text.contains("beta"), "{text}");
    assert!(text.contains('5'), "{text}");
    // Relation filters and counts stay inside the caller's organization.
    assert!(call(&f.http, "GET", &format!("/api/tasks?project={}", f.project_b), Some(a), None).await.1["items"].as_array().unwrap().is_empty());
    assert!(call(&f.http, "GET", "/api/tasks?project.tag=beta", Some(a), None).await.1["items"].as_array().unwrap().is_empty());
    // Anonymous readers have no grant; a global administrator holds no membership and sees nothing.
    assert_eq!(call(&f.http, "GET", "/api/projects", None, None).await.0, 401);
    assert!(titles(&call(&f.http, "GET", "/api/projects", Some(&f.global_admin), None).await.1).is_empty());
    assert_eq!(call(&f.http, "GET", &format!("/api/projects/{}", f.project_b), Some(&f.global_admin), None).await.0, 404);
    // Member listing of another organization is invisible.
    assert_eq!(call(&f.http, "GET", &format!("/_brain/orgs/{}/members", f.org_b), Some(a), None).await.0, 404);
    assert_eq!(call(&f.http, "GET", &format!("/_brain/orgs/{}/members", Uuid::new_v4()), Some(a), None).await.0, 404);
    let mine = call(&f.http, "GET", "/_brain/orgs", Some(a), None).await.1;
    assert_eq!(mine["items"].as_array().unwrap().len(), 1);
    assert_eq!(mine["items"][0]["id"], f.org_a.as_str());
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn no_write_channel_crosses_an_organization() {
    let f = fixture().await;
    let (a, b) = (&f.alice, &f.bob);
    let target = format!("/api/projects/{}", f.project_b);
    // Create in an organization the caller does not belong to: the same refusal as for one that does not exist.
    let denied = call(&f.http, "POST", "/api/projects", Some(a), Some(json!({"org":f.org_b,"title":"Planted"}))).await;
    let unknown = call(&f.http, "POST", "/api/projects", Some(a), Some(json!({"org":Uuid::new_v4(),"title":"Planted"}))).await;
    assert_eq!(denied.0, 403, "{}", denied.1);
    assert_eq!(unknown.0, 403, "{}", unknown.1);
    assert_eq!(denied.1["error"]["code"], unknown.1["error"]["code"]);
    // Update, patch and delete of another organization's row look like a missing row.
    assert_eq!(call(&f.http, "PUT", &target, Some(a), Some(json!({"org":f.org_b,"title":"Stolen"}))).await.0, 404);
    assert_eq!(call(&f.http, "PATCH", &target, Some(a), Some(json!({"title":"Stolen"}))).await.0, 404);
    assert_eq!(call(&f.http, "DELETE", &target, Some(a), None).await.0, 404);
    assert_eq!(call(&f.http, "PATCH", &target, Some(&f.global_admin), Some(json!({"title":"Stolen"}))).await.0, 404);
    assert_eq!(call(&f.http, "DELETE", &target, Some(&f.global_admin), None).await.0, 404);
    // Batch and upsert have no membership grant, so they fail closed.
    let batch = call(&f.http, "POST", "/api/projects/batch", Some(a), Some(json!({"items":[{"op":"delete","id":f.project_b}]}))).await;
    assert_ne!(batch.0, 200, "{}", batch.1);
    // Moving a row to another organization is refused for its own members, and by the database for any other path.
    let moved = call(&f.http, "PATCH", &format!("/api/projects/{}", f.project_a), Some(a), Some(json!({"org":f.org_b}))).await;
    assert_eq!(moved.0, 422, "{}", moved.1);
    let raw = sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE _brain_res_projects SET data=jsonb_set(data,'{{org}}','\"{}\"') WHERE id='{}'", f.org_b, f.project_a))).execute(&f.pool).await;
    assert!(raw.is_err(), "the trigger must refuse a direct move");
    // A reference into another organization answers like a missing record.
    let cross = call(&f.http, "POST", "/api/tasks", Some(a), Some(json!({"org":f.org_a,"project":f.project_b,"title":"Probe"}))).await;
    let gone = call(&f.http, "POST", "/api/tasks", Some(a), Some(json!({"org":f.org_a,"project":Uuid::new_v4(),"title":"Probe"}))).await;
    assert_eq!(cross.0, 422, "{}", cross.1);
    assert_eq!(cross.1["error"]["code"], "REFERENCE_NOT_FOUND");
    assert_eq!(cross.1["error"], gone.1["error"]);
    // The same unique value in two organizations is no conflict and no oracle.
    let (s, same) = call(&f.http, "POST", "/api/projects", Some(b), Some(json!({"org":f.org_b,"title":"Shared title"}))).await;
    assert_eq!(s, 200, "{same}");
    let (s, dup) = call(&f.http, "POST", "/api/projects", Some(a), Some(json!({"org":f.org_a,"title":"Shared title"}))).await;
    assert_eq!(s, 409, "{dup}");
    assert_eq!(dup["error"]["fields"], json!({"title":"Already used"}));
    // Nothing of B changed.
    let (_, still) = call(&f.http, "GET", &target, Some(b), None).await;
    assert_eq!(still["data"]["title"], "Secret of B");
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn roles_are_per_membership_and_removal_ends_access() {
    let f = fixture().await;
    let (a, b) = (&f.alice, &f.bob);
    let carol = actor(&f.pool, "user").await;
    let dave = actor(&f.pool, "user").await;
    invite_and_join(&f, a, &f.org_a, "member", &carol).await;
    // Same user, different role in each organization.
    invite_and_join(&f, b, &f.org_b, "admin", &carol).await;
    // A member reads, creates, updates their own row, but not another member's, and cannot delete.
    let (s, own) = call(&f.http, "POST", "/api/projects", Some(&carol), Some(json!({"org":f.org_a,"title":"Carol row"}))).await;
    assert_eq!(s, 200, "{own}");
    let path_a = format!("/api/projects/{}", f.project_a);
    let (_, seen) = call(&f.http, "GET", &path_a, Some(&carol), None).await;
    assert_eq!(seen["data"]["title"], "Shared title");
    assert!(seen.get("owner").is_none(), "another member's user id stays hidden: {seen}");
    assert_eq!(call(&f.http, "PATCH", &path_a, Some(&carol), Some(json!({"tag":"x"}))).await.0, 403);
    assert_eq!(call(&f.http, "DELETE", &path_a, Some(&carol), None).await.0, 403);
    assert_eq!(call(&f.http, "PATCH", &format!("/api/projects/{}", own["id"].as_str().unwrap()), Some(&carol), Some(json!({"tag":"mine"}))).await.0, 200);
    // Her admin role applies in B, and the refusals above show it did not carry over to A.
    assert_eq!(call(&f.http, "PATCH", &format!("/api/projects/{}", f.project_b), Some(&carol), Some(json!({"tag":"x"}))).await.0, 200);
    // A member cannot manage, invite or raise themself.
    let self_raise = call(&f.http, "PATCH", &format!("/_brain/orgs/{}/members/{}", f.org_a, carol.id), Some(&carol), Some(json!({"role":"owner"}))).await;
    assert_eq!(self_raise.0, 403, "{}", self_raise.1);
    assert_eq!(call(&f.http, "POST", &format!("/_brain/orgs/{}/invites", f.org_a), Some(&carol), Some(json!({"role":"member"}))).await.0, 403);
    // An admin cannot grant or touch the rank above their own.
    assert_eq!(call(&f.http, "POST", &format!("/_brain/orgs/{}/invites", f.org_b), Some(&carol), Some(json!({"role":"owner"}))).await.0, 403);
    assert_eq!(call(&f.http, "PATCH", &format!("/_brain/orgs/{}/members/{}", f.org_b, carol.id), Some(&carol), Some(json!({"role":"owner"}))).await.0, 403);
    assert_eq!(call(&f.http, "DELETE", &format!("/_brain/orgs/{}/members/{}", f.org_b, b.id), Some(&carol), None).await.0, 403);
    // Managers of one organization have no power in another.
    assert_eq!(call(&f.http, "DELETE", &format!("/_brain/orgs/{}/members/{}", f.org_b, b.id), Some(a), None).await.0, 404);
    // Invitations are single use and carry a known role.
    let (_, inv) = call(&f.http, "POST", &format!("/_brain/orgs/{}/invites", f.org_a), Some(a), Some(json!({"role":"member"}))).await;
    let token = inv["token"].clone();
    assert_eq!(call(&f.http, "POST", "/_brain/orgs/invites/accept", Some(&dave), Some(json!({"token":token}))).await.0, 200);
    assert_eq!(call(&f.http, "POST", "/_brain/orgs/invites/accept", Some(&dave), Some(json!({"token":token}))).await.0, 400);
    assert_eq!(call(&f.http, "POST", &format!("/_brain/orgs/{}/invites", f.org_a), Some(a), Some(json!({"role":"emperor"}))).await.0, 422);
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_org_invites WHERE token_hash=$1").bind(token.as_str().unwrap()).fetch_one(&f.pool).await.unwrap();
    assert_eq!(stored, 0, "only the digest of an invitation token is stored");
    // Last top-role member cannot leave or be demoted, even concurrently with a second owner.
    assert_eq!(call(&f.http, "DELETE", &format!("/_brain/orgs/{}/members/{}", f.org_a, a.id), Some(a), None).await.0, 409);
    assert_eq!(call(&f.http, "PATCH", &format!("/_brain/orgs/{}/members/{}", f.org_a, a.id), Some(a), Some(json!({"role":"member"}))).await.0, 409);
    // Removal ends access at once, including to the rows that person created.
    assert_eq!(call(&f.http, "DELETE", &format!("/_brain/orgs/{}/members/{}", f.org_a, carol.id), Some(a), None).await.0, 200);
    assert_eq!(call(&f.http, "GET", &format!("/api/projects/{}", own["id"].as_str().unwrap()), Some(&carol), None).await.0, 404);
    assert_eq!(call(&f.http, "PATCH", &format!("/api/projects/{}", own["id"].as_str().unwrap()), Some(&carol), Some(json!({"tag":"back"}))).await.0, 404);
    assert!(!titles(&call(&f.http, "GET", "/api/projects", Some(&carol), None).await.1).contains(&"Carol row".to_string()));
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn conditioned_public_read_shows_only_published_tenant_rows() {
    let f = fixture().await;
    let (a, _) = (&f.alice, &f.bob);
    let (s, live) = call(&f.http, "POST", "/api/bulletins", Some(a), Some(json!({"org":f.org_a,"status":"live","body":"Open"}))).await;
    assert_eq!(s, 200, "{live}");
    let (_, draft) = call(&f.http, "POST", "/api/bulletins", Some(a), Some(json!({"org":f.org_a,"status":"draft","body":"Private"}))).await;
    let anon = call(&f.http, "GET", "/api/bulletins", None, None).await.1;
    assert_eq!(anon["items"].as_array().unwrap().len(), 1, "{anon}");
    assert_eq!(call(&f.http, "GET", &format!("/api/bulletins/{}", draft["id"].as_str().unwrap()), None, None).await.0, 404);
    assert_eq!(call(&f.http, "GET", &format!("/api/bulletins/{}", draft["id"].as_str().unwrap()), Some(&f.bob), None).await.0, 404);
    assert_eq!(call(&f.http, "GET", "/api/bulletins", Some(a), None).await.1["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn membership_changes_cannot_race_past_the_last_owner() {
    let f = fixture().await;
    let second = actor(&f.pool, "user").await;
    invite_and_join(&f, &f.alice, &f.org_a, "owner", &second).await;
    // Both owners leave at once: exactly one may succeed.
    let (first_path, second_path) = (format!("/_brain/orgs/{}/members/{}", f.org_a, f.alice.id), format!("/_brain/orgs/{}/members/{}", f.org_a, second.id));
    let (one, two) = tokio::join!(
        call(&f.http, "DELETE", &first_path, Some(&f.alice), None),
        call(&f.http, "DELETE", &second_path, Some(&second), None),
    );
    let mut codes = [one.0.as_u16(), two.0.as_u16()];
    codes.sort();
    assert_eq!(codes, [200, 409]);
    let owners: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_org_members WHERE org_id=$1::uuid AND role='owner'").bind(&f.org_a).fetch_one(&f.pool).await.unwrap();
    assert_eq!(owners, 1);
}

#[test]
fn contracts_that_could_cross_an_organization_are_refused() {
    let tenant = |policy: Value| -> Result<(), String> {
        let r: Resource = serde_json::from_value(json!({"name":"items","fields":{"org":{"kind":"org","required":true}},"policy":policy})).unwrap();
        r.validate()
    };
    let ok = json!({"read":["org"],"create":["org"],"update":["org.own"],"delete":["org.admin"]});
    assert!(tenant(ok).is_ok());
    for (policy, needle) in [
        (json!({"read":["admin"],"create":["org"],"update":["org"],"delete":["org"]}), "cross organizations"),
        (json!({"read":["owner"],"create":["org"],"update":["org"],"delete":["org"]}), "cross organizations"),
        (json!({"read":["public"],"create":["org"],"update":["org"],"delete":["org"]}), "exposes every organization"),
        (json!({"read":["org"],"create":["org.own"],"update":["org"],"delete":["org"]}), "no row to own"),
    ] {
        let error = tenant(policy).unwrap_err();
        assert!(error.contains(needle), "{error}");
    }
    let shared: Resource = serde_json::from_value(json!({"name":"items","fields":{"name":{"kind":"str"}},"policy":{"read":["org.admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}})).unwrap();
    assert!(shared.validate().unwrap_err().contains("needs tenant data"));
    let two: Resource = serde_json::from_value(json!({"name":"items","fields":{"a":{"kind":"org","required":true},"b":{"kind":"org","required":true}},"policy":{"read":["org"],"create":["org"],"update":["org"],"delete":["org"]}})).unwrap();
    assert!(two.validate().unwrap_err().contains("one org field"));
    let optional: Resource = serde_json::from_value(json!({"name":"items","fields":{"a":{"kind":"org"}},"policy":{"read":["org"],"create":["org"],"update":["org"],"delete":["org"]}})).unwrap();
    assert!(optional.validate().unwrap_err().contains("must be required"));
    let slug: Resource = serde_json::from_value(json!({"name":"items","fields":{"a":{"kind":"org","required":true},"s":{"kind":"str","required":true,"unique":true,"slug":true}},"policy":{"read":["org"],"create":["org"],"update":["org"],"delete":["org"]}})).unwrap();
    assert!(slug.validate().unwrap_err().contains("slug history"));
    // A shared resource may not point into tenant data: the reference would prove a row exists.
    let a: Resource = serde_json::from_value(json!({"name":"secrets","fields":{"o":{"kind":"org","required":true}},"policy":{"read":["org"],"create":["org"],"update":["org"],"delete":["org"]}})).unwrap();
    let b: Resource = serde_json::from_value(json!({"name":"refs","fields":{"s":{"kind":"ref:secrets"}},"policy":{"read":["public"],"create":["admin"],"update":["admin"],"delete":["admin"]}})).unwrap();
    assert!(brain_backend::relations::validate_graph(&[a, b]).unwrap_err().contains("tenant data"));
    assert!(Organizations::from_contract(&json!({"roles":["boss","staff"],"managers":["staff"]})).unwrap_err().contains("must be a manager"));
    assert_eq!(Organizations::from_contract(&json!({"roles":["boss","staff"]})).unwrap().managers, vec!["boss"]);
    assert!(Organizations::from_contract(&json!({"roles":["own"]})).is_err());
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn serving_refuses_a_tenant_table_without_its_trigger() {
    let f = fixture().await;
    sqlx::raw_sql("ALTER TABLE _brain_res_projects DISABLE TRIGGER _brain_org").execute(&f.pool).await.unwrap();
    let error = db::verify(&f.pool, &resources()).await.unwrap_err();
    assert!(error.contains("organization trigger"), "{error}");
}

fn audited() -> Vec<Resource> {
    let tenant = json!({"read":["org"],"create":["org"],"update":["org"],"delete":["org"],"audit":true});
    let shared = json!({"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"],"audit":true});
    let list: Vec<Value> = serde_json::from_value(json!([
        {"name":"cases","fields":{"org":{"kind":"org","required":true},"title":{"kind":"str","required":true},"secret":{"kind":"str","access":"private"},"pin":{"kind":"str","access":"writeonly"}},"policy":tenant},
        {"name":"settings","fields":{"name":{"kind":"str","required":true}},"policy":shared},
        {"name":"quiet","fields":{"name":{"kind":"str","required":true}},"policy":{"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}}
    ])).unwrap();
    list.into_iter().map(|v| serde_json::from_value(v).unwrap()).collect()
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn audit_log_records_mutations_without_values_and_cannot_be_rewritten() {
    let pool = database().await;
    let all = audited();
    db::migrate(&pool, &all).await.unwrap();
    roles().register(&pool).await.unwrap();
    let (alice, bob, admin) = (actor(&pool, "user").await, actor(&pool, "user").await, actor(&pool, "admin").await);
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(all.clone()), Platform::default()).await.unwrap();
    let org_a = call(&http, "POST", "/_brain/orgs", Some(&alice), Some(json!({"name":"A"}))).await.1["id"].as_str().unwrap().to_string();
    let org_b = call(&http, "POST", "/_brain/orgs", Some(&bob), Some(json!({"name":"B"}))).await.1["id"].as_str().unwrap().to_string();
    // A row carrying a private and a write-only value, set by a server path, then changed over HTTP.
    let (s, made) = call(&http, "POST", "/api/cases", Some(&alice), Some(json!({"org":org_a,"title":"Public-title-marker","pin":"writeonly-value-marker"}))).await;
    assert_eq!(s, 200, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    assert_eq!(call(&http, "PATCH", &format!("/api/cases/{id}"), Some(&alice), Some(json!({"title":"Second-title-marker"}))).await.0, 200);
    let mut rewrite = sqlx::query("UPDATE _brain_res_cases SET data=data||'{\"secret\":\"private-value-marker\"}' WHERE id=$1::uuid").bind(&id).execute(&pool).await;
    assert!(rewrite.is_ok());
    rewrite = sqlx::query("UPDATE _brain_res_cases SET data=data WHERE id=$1::uuid").bind(&id).execute(&pool).await;
    assert!(rewrite.is_ok());
    assert_eq!(call(&http, "DELETE", &format!("/api/cases/{id}"), Some(&alice), None).await.0, 200);
    // A refused mutation leaves no entry.
    assert_eq!(call(&http, "PATCH", &format!("/api/cases/{id}"), Some(&bob), Some(json!({"title":"x"}))).await.0, 404);

    let (s, log) = call(&http, "GET", &format!("/_brain/orgs/{org_a}/audit?resource=cases"), Some(&alice), None).await;
    assert_eq!(s, 200, "{log}");
    let items = log["items"].as_array().unwrap();
    let ops: Vec<&str> = items.iter().map(|i| i["operation"].as_str().unwrap()).collect();
    assert_eq!(ops, ["delete", "update", "update", "insert"], "{log}");
    assert_eq!(items[2]["fields"], json!(["title"]));
    assert_eq!(items[2]["actor"], alice.id.to_string());
    assert_eq!(items[2]["actor_kind"], "user");
    assert!(items[2]["request"].as_str().is_some_and(|r| !r.is_empty()));
    assert_eq!(items[1]["actor_kind"], "server", "a write outside a request is recorded as such");
    assert_eq!(items[0]["actor"], alice.id.to_string());
    // No value is stored anywhere in the log, whatever the field's access.
    let dump: String = sqlx::query_scalar("SELECT coalesce(string_agg(to_jsonb(l)::text,' '),'') FROM _brain_audit_log l").fetch_one(&pool).await.unwrap();
    for marker in ["marker", "Public-title", "writeonly-value", "private-value"] {
        assert!(!dump.contains(marker), "`{marker}` reached the audit log: {dump}");
    }
    // Isolation: another organization's manager and a plain non-member cannot read it; each sees their own.
    assert_eq!(call(&http, "GET", &format!("/_brain/orgs/{org_a}/audit"), Some(&bob), None).await.0, 404);
    assert_eq!(call(&http, "GET", &format!("/_brain/orgs/{org_a}/audit"), Some(&admin), None).await.0, 404);
    let theirs = call(&http, "GET", &format!("/_brain/orgs/{org_b}/audit"), Some(&bob), None).await.1;
    assert!(theirs["items"].as_array().unwrap().iter().all(|i| i["org"] == org_b.as_str()), "{theirs}");
    // A plain member is refused; membership events are recorded.
    let carol = actor(&pool, "user").await;
    let (_, inv) = call(&http, "POST", &format!("/_brain/orgs/{org_a}/invites"), Some(&alice), Some(json!({"role":"member"}))).await;
    assert_eq!(call(&http, "POST", "/_brain/orgs/invites/accept", Some(&carol), Some(json!({"token":inv["token"]}))).await.0, 200);
    assert_eq!(call(&http, "GET", &format!("/_brain/orgs/{org_a}/audit"), Some(&carol), None).await.0, 403);
    assert_eq!(call(&http, "PATCH", &format!("/_brain/orgs/{org_a}/members/{}", carol.id), Some(&alice), Some(json!({"role":"admin"}))).await.0, 200);
    assert_eq!(call(&http, "DELETE", &format!("/_brain/orgs/{org_a}/members/{}", carol.id), Some(&alice), None).await.0, 200);
    let org_log = call(&http, "GET", &format!("/_brain/orgs/{org_a}/audit?resource=organization"), Some(&alice), None).await.1;
    let events: Vec<&str> = org_log["items"].as_array().unwrap().iter().map(|i| i["operation"].as_str().unwrap()).collect();
    assert_eq!(events, ["member_removed", "member_role_changed", "member_added", "invite_created", "org_created"]);
    assert!(!org_log.to_string().contains(inv["token"].as_str().unwrap()));
    // Shared resources: administrators only, and a resource without `audit` records nothing.
    assert_eq!(call(&http, "POST", "/api/settings", Some(&admin), Some(json!({"name":"one"}))).await.0, 200);
    assert_eq!(call(&http, "POST", "/api/quiet", Some(&admin), Some(json!({"name":"two"}))).await.0, 200);
    let global = call(&http, "GET", "/_brain/audit", Some(&admin), None).await.1;
    let resources: Vec<&str> = global["items"].as_array().unwrap().iter().map(|i| i["resource"].as_str().unwrap()).collect();
    assert_eq!(resources, ["settings"], "{global}");
    assert_eq!(call(&http, "GET", "/_brain/audit", Some(&alice), None).await.0, 403);
    assert_eq!(call(&http, "GET", "/_brain/audit?limit=0", Some(&admin), None).await.0, 422);
    assert_eq!(call(&http, "GET", "/_brain/audit?unknown=1", Some(&admin), None).await.0, 422);
    // Paging walks every entry once.
    let first = call(&http, "GET", &format!("/_brain/orgs/{org_a}/audit?limit=2"), Some(&alice), None).await.1;
    let next = first["next"].as_i64().unwrap();
    let second = call(&http, "GET", &format!("/_brain/orgs/{org_a}/audit?limit=2&before={next}"), Some(&alice), None).await.1;
    assert!(second["items"][0]["id"].as_i64().unwrap() < next);
    // Append-only against the application role itself.
    for statement in ["UPDATE _brain_audit_log SET actor=NULL", "DELETE FROM _brain_audit_log", "TRUNCATE _brain_audit_log"] {
        let error = sqlx::raw_sql(sqlx::AssertSqlSafe(statement)).execute(&pool).await.unwrap_err();
        assert!(error.to_string().contains("append-only"), "{statement}: {error}");
    }
    // Serving refuses an audited table whose trigger was removed.
    sqlx::raw_sql("DROP TRIGGER _brain_audit ON _brain_res_settings").execute(&pool).await.unwrap();
    assert!(db::verify(&pool, &all).await.unwrap_err().contains("audit trigger"));
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn audit_log_attributes_domain_actions_and_keeps_a_refused_write_out() {
    let pool = database().await;
    let list: Vec<Resource> = serde_json::from_value(json!([{
        "name":"counters",
        "fields":{"name":{"kind":"str","required":true},"total":{"kind":"int","required":true},"note":{"kind":"str","access":"private"}},
        "policy":{"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"],"audit":true}
    }])).unwrap();
    db::migrate(&pool, &list).await.unwrap();
    roles().register(&pool).await.unwrap();
    let admin = actor(&pool, "admin").await;
    let domain: brain_backend::domain::Domain = serde_json::from_value(json!({"resources":{"counters":{"actions":{
        "bump":{"roles":["admin"],"input":{"amount":"int!"},"increment":{"total":"$input.amount"},"when":{"total":{"lt":100}}}
    }}}})).unwrap();
    let (http, _) = brain_backend::http::router_with_domain(pool.clone(), config(list), domain).await.unwrap();
    let id = call(&http, "POST", "/api/counters", Some(&admin), Some(json!({"name":"one","total":1}))).await.1["id"].as_str().unwrap().to_string();
    assert_eq!(call(&http, "POST", &format!("/api/counters/{id}/actions/bump"), Some(&admin), Some(json!({"amount":2}))).await.0, 200);
    assert_eq!(call(&http, "POST", &format!("/api/counters/{id}/actions/bump"), Some(&admin), Some(json!({"amount":200}))).await.0, 200);
    let refused = call(&http, "POST", &format!("/api/counters/{id}/actions/bump"), Some(&admin), Some(json!({"amount":1}))).await.0;
    assert_ne!(refused, 200, "the precondition no longer holds");
    let log = call(&http, "GET", "/_brain/audit?resource=counters", Some(&admin), None).await.1;
    let items = log["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "a refused action records nothing: {log}");
    assert_eq!(items[0]["fields"], json!(["total"]));
    assert_eq!(items[0]["operation"], "update");
    assert_eq!(items[0]["actor"], admin.id.to_string());
    assert_eq!(items[0]["actor_kind"], "user");
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn facets_count_only_rows_the_reader_may_list() {
    let pool = database().await;
    let list: Vec<Resource> = serde_json::from_value(json!([
        {"name":"tickets","fields":{"org":{"kind":"org","required":true},"status":{"kind":"str","required":true},"tags":{"kind":"list<str>"},"rank":{"kind":"int"},"open":{"kind":"bool"}},
         "policy":{"read":["org"],"create":["org"],"update":["org"],"delete":["org"]},
         "query":{"filter":{"status":["eq"],"rank":["gte"]},"facets":["status","tags","rank","open"]}},
        {"name":"notes","fields":{"state":{"kind":"str","required":true},"body":{"kind":"str"}},"search":["body"],
         "policy":{"read":["owner"],"create":["user"],"update":["owner"],"delete":["owner"]},
         "query":{"filter":{"state":["eq"]},"facets":["state"]}}
    ])).unwrap();
    db::migrate(&pool, &list).await.unwrap();
    roles().register(&pool).await.unwrap();
    let (alice, bob) = (actor(&pool, "user").await, actor(&pool, "user").await);
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(list), Platform::default()).await.unwrap();
    let org_a = call(&http, "POST", "/_brain/orgs", Some(&alice), Some(json!({"name":"A"}))).await.1["id"].as_str().unwrap().to_string();
    let org_b = call(&http, "POST", "/_brain/orgs", Some(&bob), Some(json!({"name":"B"}))).await.1["id"].as_str().unwrap().to_string();
    for (who, org, status, tags, rank, open) in [
        (&alice, &org_a, "new", json!(["x","y"]), 1, true),
        (&alice, &org_a, "new", json!(["x","x"]), 1, true),
        (&alice, &org_a, "done", json!([]), 2, false),
        (&bob, &org_b, "foreign", json!(["foreign"]), 9, false),
        (&bob, &org_b, "new", json!(["x"]), 9, false),
    ] {
        let (s, body) = call(&http, "POST", "/api/tickets", Some(who), Some(json!({"org":org,"status":status,"tags":tags,"rank":rank,"open":open}))).await;
        assert_eq!(s, 200, "{body}");
    }
    let (s, out) = call(&http, "GET", "/api/tickets?facets=status,tags,rank,open", Some(&alice), None).await;
    assert_eq!(s, 200, "{out}");
    assert_eq!(out["facets"]["status"], json!([{"key":"new","count":2},{"key":"done","count":1}]), "{out}");
    assert_eq!(out["facets"]["tags"], json!([{"key":"x","count":2},{"key":"y","count":1}]), "a row counts once per distinct element");
    assert_eq!(out["facets"]["rank"], json!([{"key":1,"count":2},{"key":2,"count":1}]));
    assert_eq!(out["facets"]["open"], json!([{"key":true,"count":2},{"key":false,"count":1}]));
    assert!(!out.to_string().contains("foreign"), "another organization's values leaked: {out}");
    // A field's own filter does not narrow its facet; other filters do. The scope stays even so.
    let out = call(&http, "GET", "/api/tickets?status=done&facets=status,rank", Some(&alice), None).await.1;
    assert_eq!(out["total"], 1);
    assert_eq!(out["facets"]["status"], json!([{"key":"new","count":2},{"key":"done","count":1}]));
    assert_eq!(out["facets"]["rank"], json!([{"key":2,"count":1}]));
    let out = call(&http, "GET", "/api/tickets?status=foreign&facets=status", Some(&alice), None).await.1;
    assert_eq!(out["total"], 0);
    assert!(!out.to_string().contains("foreign\",\"count"), "{out}");
    assert_eq!(out["facets"]["status"].as_array().unwrap().len(), 2);
    // An owner-scoped resource counts only the reader's rows, including with search.
    for (who, state, body) in [(&alice, "mine", "shared words"), (&bob, "theirs", "shared words"), (&bob, "theirs", "shared words")] {
        assert_eq!(call(&http, "POST", "/api/notes", Some(who), Some(json!({"state":state,"body":body}))).await.0, 200);
    }
    let out = call(&http, "GET", "/api/notes?search=shared&facets=state", Some(&alice), None).await.1;
    assert_eq!(out["facets"]["state"], json!([{"key":"mine","count":1}]), "{out}");
    // Only declared facets; a restricted field cannot be declared.
    assert_eq!(call(&http, "GET", "/api/notes?facets=body", Some(&alice), None).await.0, 422);
    let restricted: Result<Resource, _> = serde_json::from_value(json!({"name":"bad","fields":{"code":{"kind":"str","access":"private"}},"policy":{"read":["admin"]},"query":{"facets":["code"]}}));
    assert!(restricted.is_err() || brain_backend::query::validate(&restricted.unwrap()).unwrap_err().contains("facet `code`"));
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn geo_filters_are_exact_scoped_and_served_by_a_builtin_index() {
    let pool = database().await;
    let list: Vec<Resource> = serde_json::from_value(json!([
        {"name":"sites","fields":{"name":{"kind":"str","required":true},"at":{"kind":"geo"}},
         "policy":{"read":["owner"],"create":["user"],"update":["owner"],"delete":["owner"]},
         "query":{"filter":{"at":["near","bbox","exists"]}}}
    ])).unwrap();
    db::migrate(&pool, &list).await.unwrap();
    roles().register(&pool).await.unwrap();
    let extensions: Vec<String> = sqlx::query_scalar("SELECT extname::text FROM pg_extension").fetch_all(&pool).await.unwrap();
    assert!(!extensions.iter().any(|e| e == "postgis" || e == "earthdistance" || e == "cube"), "{extensions:?}");
    let (alice, bob) = (actor(&pool, "user").await, actor(&pool, "user").await);
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(list), Platform::default()).await.unwrap();
    for (name, lat, lng) in [
        ("origin", 0.0, 0.0), ("east-100km", 0.0, 0.899), ("east-120km", 0.0, 1.0791), ("corner-box-only", 0.8, 0.8),
        ("dateline-west", 10.0, 179.9), ("dateline-east", 10.0, -179.9), ("pole-a", 89.9, 0.0), ("pole-b", 89.9, 180.0),
    ] {
        let (s, body) = call(&http, "POST", "/api/sites", Some(&alice), Some(json!({"name":name,"at":{"lat":lat,"lng":lng}}))).await;
        assert_eq!(s, 200, "{body}");
    }
    assert_eq!(call(&http, "POST", "/api/sites", Some(&alice), Some(json!({"name":"nowhere"}))).await.0, 200);
    assert_eq!(call(&http, "POST", "/api/sites", Some(&bob), Some(json!({"name":"foreign","at":{"lat":0.0,"lng":0.0}}))).await.0, 200);
    let names = async |query: &str| -> Vec<String> {
        let (s, out) = call(&http, "GET", &format!("/api/sites?{query}&limit=50"), Some(&alice), None).await;
        let _ = s;
        let mut found: Vec<String> = out["items"].as_array().unwrap_or_else(|| panic!("{query}: {out}")).iter().map(|i| i["data"]["name"].as_str().unwrap().to_string()).collect();
        found.sort();
        found
    };
    // Great-circle distance, not the box: the corner is inside the box but about 125 km away.
    assert_eq!(names("at[near]=0,0,110").await, ["east-100km", "origin"]);
    assert_eq!(names("at[near]=0,0,130").await, ["corner-box-only", "east-100km", "east-120km", "origin"]);
    // Across the antimeridian and over a pole.
    assert_eq!(names("at[near]=10,180,50").await, ["dateline-east", "dateline-west"]);
    assert_eq!(names("at[near]=89.95,90,50").await, ["pole-a", "pole-b"]);
    assert_eq!(names("at[bbox]=9,179,11,-179").await, ["dateline-east", "dateline-west"]);
    assert_eq!(names("at[bbox]=-1,-1,1,1").await, ["corner-box-only", "east-100km", "origin"]);
    assert_eq!(names("at[exists]=false").await, ["nowhere"]);
    // Refused input names the shape.
    for query in ["at[near]=91,0,1", "at[near]=0,0,0", "at[near]=0,0", "at[near]=0,0,99999", "at[near]=NaN,0,1", "at[near]=0,0,inf", "at[bbox]=2,0,1,1", "at[bbox]=0,0,1", "at[bbox]=0,181,1,0", "at[eq]=0"] {
        assert_eq!(call(&http, "GET", &format!("/api/sites?{query}"), Some(&alice), None).await.0, 422, "{query}");
    }
    // Stored values are validated at the API and in the database.
    for at in [json!({"lat":91,"lng":0}), json!({"lat":0}), json!({"lat":"0","lng":0}), json!({"lat":0,"lng":0,"x":1}), json!([0,0]), json!("0,0")] {
        assert_eq!(call(&http, "POST", "/api/sites", Some(&alice), Some(json!({"name":"bad","at":at}))).await.0, 422, "{at}");
    }
    let error = sqlx::query("UPDATE _brain_res_sites SET data=data||'{\"at\":{\"lat\":\"x\",\"lng\":0}}'").execute(&pool).await.unwrap_err();
    assert!(error.to_string().contains("check constraint"), "{error}");
    // The filter uses the built-in GiST index.
    let mut conn = pool.acquire().await.unwrap();
    sqlx::raw_sql("SET enable_seqscan=off").execute(&mut *conn).await.unwrap();
    let plan: Vec<String> = sqlx::query_scalar("EXPLAIN SELECT id FROM _brain_res_sites WHERE point((query_keys#>>'{at,lng}')::float8,(query_keys#>>'{at,lat}')::float8) <@ box(point(-1,-1),point(1,1))").fetch_all(&mut *conn).await.unwrap();
    assert!(plan.join(" ").contains("_brain_ix_"), "{plan:?}");
    sqlx::raw_sql("RESET enable_seqscan").execute(&mut *conn).await.unwrap();
}

/// Fake verification endpoint: answers by token, and records each form body it received.
async fn fake_provider() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use axum::{body::Bytes, routing::post};
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let app = Router::new().route("/siteverify", post(move |body: Bytes| {
        let log = log.clone();
        async move {
            let text = String::from_utf8_lossy(&body).to_string();
            log.lock().unwrap().push(text.clone());
            let token = url::form_urlencoded::parse(text.as_bytes()).find(|(k, _)| k == "response").map(|(_, v)| v.to_string()).unwrap_or_default();
            match token.as_str() {
                "good" => (axum::http::StatusCode::OK, r#"{"success":true,"hostname":"app.test"}"#.to_string()),
                "other-site" => (axum::http::StatusCode::OK, r#"{"success":true,"hostname":"evil.test"}"#.to_string()),
                "bad" => (axum::http::StatusCode::OK, r#"{"success":false,"error-codes":["invalid-input-response"]}"#.to_string()),
                "provider-fault" => (axum::http::StatusCode::OK, r#"{"success":false,"error-codes":["invalid-input-secret"]}"#.to_string()),
                "truthy" => (axum::http::StatusCode::OK, r#"{"success":"true","hostname":"app.test"}"#.to_string()),
                "garbage" => (axum::http::StatusCode::OK, "not json".to_string()),
                "huge" => (axum::http::StatusCode::OK, format!(r#"{{"success":true,"hostname":"app.test","x":"{}"}}"#, "x".repeat(100_000))),
                _ => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "{}".to_string()),
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}/siteverify"), seen)
}
async fn post_token(http: &Router, path: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut request = axum::http::Request::builder().method("POST").uri(path).header("content-type", "application/json").header("origin", ORIGIN)
        .extension(ConnectInfo("127.0.0.1:42031".parse::<std::net::SocketAddr>().unwrap()));
    if let Some(token) = token {
        request = request.header("x-brain-captcha", token);
    }
    let answer = tower::ServiceExt::oneshot(http.clone(), request.body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = answer.status();
    let bytes = axum::body::to_bytes(answer.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn captcha_admits_only_a_token_the_provider_confirmed() {
    use brain_backend::captcha::Captcha;
    let pool = database().await;
    db::migrate(&pool, &[]).await.unwrap();
    roles().register(&pool).await.unwrap();
    let (url, seen) = fake_provider().await;
    let contract = json!({"verify_url":url,"secret_env":"CAPTCHA_SECRET","hostname":"app.test","protect":["signup","login","password_request"]});
    let secret = "synthetic-captcha-secret";
    let captcha = Captcha::from_contract(&contract, |_| Some(secret.into())).unwrap();
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(Vec::new()), Platform { captcha, ..Default::default() }).await.unwrap();
    let users = async || sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _brain_users").fetch_one(&pool).await.unwrap();
    let before = users().await;
    let person = json!({"email":"someone@example.test","password":"synthetic-password-1234"});
    for (token, status, code) in [
        (None, 403, "CAPTCHA_FAILED"), (Some(""), 403, "CAPTCHA_FAILED"), (Some("bad"), 403, "CAPTCHA_FAILED"), (Some("other-site"), 403, "CAPTCHA_FAILED"),
        (Some("provider-fault"), 503, "CAPTCHA_UNAVAILABLE"), (Some("truthy"), 503, "CAPTCHA_UNAVAILABLE"), (Some("garbage"), 503, "CAPTCHA_UNAVAILABLE"),
        (Some("huge"), 503, "CAPTCHA_UNAVAILABLE"), (Some("server-error"), 503, "CAPTCHA_UNAVAILABLE"),
    ] {
        let (s, body) = post_token(&http, "/_brain/auth/signup", token, person.clone()).await;
        assert_eq!((s.as_u16(), body["error"]["code"].as_str()), (status, Some(code)), "{token:?}: {body}");
        assert_eq!(users().await, before, "{token:?} created an account");
    }
    let (s, body) = post_token(&http, "/_brain/auth/signup", Some("good"), person.clone()).await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(users().await, before + 1);
    // Every protected action, including the one a provider outage must not open.
    for (path, body) in [("/_brain/auth/login", person.clone()), ("/_brain/auth/password/request", json!({"email":"someone@example.test"}))] {
        assert_eq!(post_token(&http, path, None, body.clone()).await.0, 403, "{path}");
        assert_ne!(post_token(&http, path, Some("good"), body).await.0, 403, "{path}");
    }
    // The form follows the shared provider contract, and the secret never reaches a client.
    let form = seen.lock().unwrap().last().unwrap().clone();
    let pairs: std::collections::BTreeMap<String, String> = url::form_urlencoded::parse(form.as_bytes()).into_owned().collect();
    assert_eq!((pairs["secret"].as_str(), pairs["response"].as_str()), (secret, "good"));
    assert!(!Captcha::from_contract(&contract, |_| Some(secret.into())).unwrap().schema().to_string().contains(secret));
    // A public write route runs its handler only with a confirmed token; its reads and other routes stay open.
    struct Count(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl brain_backend::routes::RouteHandlers for Count {
        fn names(&self) -> Vec<String> {
            vec!["contact".into(), "open_write".into()]
        }
        fn run(&self, name: &str, _: Value, _: std::time::Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, brain_backend::routes::RouteFailure>> + Send + '_>> {
            if name == "contact" {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            Box::pin(async { Ok(json!({"ok":true})) })
        }
    }
    let ran = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let declared = |name: &str, path: &str| brain_backend::routes::Route::from_contract(name, &json!({"method":"POST","path":path,"auth":"public"}), &|_| None).unwrap();
    let guarded = Captcha::from_contract(&json!({"verify_url":url,"secret_env":"S","protect":["route:contact"]}), |_| Some("x".into())).unwrap();
    let platform = Platform { routes: vec![declared("contact", "/contact"), declared("open_write", "/open")], handlers: Some(std::sync::Arc::new(Count(ran.clone()))), captcha: guarded, ..Default::default() };
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(Vec::new()), platform).await.unwrap();
    assert_eq!(post_token(&http, "/contact", None, json!({})).await.0, 403);
    assert_eq!(post_token(&http, "/contact", Some("bad"), json!({})).await.0, 403);
    assert_eq!(post_token(&http, "/contact", Some("server-error"), json!({})).await.0, 503);
    assert_eq!(ran.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(post_token(&http, "/contact", Some("good"), json!({})).await.0, 200);
    assert_eq!(ran.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(post_token(&http, "/open", None, json!({})).await.0, 200);
    // An unreachable provider and an unset secret both refuse.
    let dead = Captcha::from_contract(&json!({"verify_url":"http://127.0.0.1:9/x","secret_env":"S","protect":["signup"]}), |_| Some("x".into())).unwrap();
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(Vec::new()), Platform { captcha: dead, ..Default::default() }).await.unwrap();
    assert_eq!(post_token(&http, "/_brain/auth/signup", Some("good"), person.clone()).await.0, 503);
    let unset = Captcha::from_contract(&contract, |_| None).unwrap();
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(Vec::new()), Platform { captcha: unset, ..Default::default() }).await.unwrap();
    assert_eq!(post_token(&http, "/_brain/auth/signup", Some("good"), person.clone()).await.0, 503);
    // Contracts that could not protect anything are refused.
    for bad in [json!({"verify_url":"http://example.test/x","secret_env":"S","protect":["signup"]}), json!({"verify_url":"https://user:pw@example.test/x","secret_env":"S","protect":["signup"]}), json!({"verify_url":"https://example.test/x","secret_env":"S","protect":[]}), json!({"verify_url":"https://example.test/x","secret_env":"S","protect":["everything"]})] {
        assert!(Captcha::from_contract(&bad, |_| None).is_err(), "{bad}");
    }
    let missing = Captcha::from_contract(&json!({"verify_url":"https://example.test/x","secret_env":"S","protect":["route:nothing"]}), |_| Some("x".into())).unwrap();
    assert!(brain_backend::http::router_with_platform(pool.clone(), config(Vec::new()), Platform { captcha: missing, ..Default::default() }).await.is_err());
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn credits_never_go_negative_and_charge_once_under_real_concurrency() {
    use brain_backend::credits::{self, Refusal};
    let pool = database().await;
    db::migrate(&pool, &[]).await.unwrap();
    roles().register(&pool).await.unwrap();
    let (alice, bob, admin) = (actor(&pool, "user").await, actor(&pool, "user").await, actor(&pool, "admin").await);
    let subject = alice.id;
    credits::apply(&pool, subject, 100, "grant", "start", None).await.unwrap();

    // Each debit is its own transaction on its own connection, all started together.
    let gate = std::sync::Arc::new(tokio::sync::Barrier::new(60));
    let mut tasks = Vec::new();
    for i in 0..60 {
        let (pool, gate) = (pool.clone(), gate.clone());
        tasks.push(tokio::spawn(async move {
            gate.wait().await;
            credits::apply(&pool, subject, -3, "debit", &format!("debit-{i}"), None).await
        }));
    }
    let (mut won, mut refused) = (0, 0);
    for task in tasks {
        match task.await.unwrap() {
            Ok(entry) => { assert!(entry.balance >= 0); won += 1 }
            Err(Refusal::Insufficient) => refused += 1,
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!((won, refused), (33, 27));
    let balance = async || sqlx::query_scalar::<_, i64>("SELECT balance FROM _brain_credit_balances WHERE subject=$1").bind(subject).fetch_one(&pool).await.unwrap();
    let ledger = async || sqlx::query_scalar::<_, i64>("SELECT sum(delta)::bigint FROM _brain_credit_ledger WHERE subject=$1").bind(subject).fetch_one(&pool).await.unwrap();
    assert_eq!(balance().await, 1);
    assert_eq!(ledger().await, 1, "the ledger explains the balance");

    // The same key, concurrently, charges once and every caller gets the same result.
    credits::apply(&pool, subject, 50, "grant", "more", None).await.unwrap();
    let gate = std::sync::Arc::new(tokio::sync::Barrier::new(20));
    let mut tasks = Vec::new();
    for _ in 0..20 {
        let (pool, gate) = (pool.clone(), gate.clone());
        tasks.push(tokio::spawn(async move {
            gate.wait().await;
            credits::apply(&pool, subject, -7, "once", "the-key", None).await.unwrap()
        }));
    }
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.unwrap());
    }
    assert_eq!(results.iter().filter(|e| !e.replayed).count(), 1);
    assert!(results.iter().all(|e| e.balance == 44 && e.id == results[0].id));
    assert_eq!(balance().await, 44);
    assert_eq!(ledger().await, 44);
    // A brand new subject created by racing first grants ends with all of them.
    let gate = std::sync::Arc::new(tokio::sync::Barrier::new(10));
    let mut tasks = Vec::new();
    for i in 0..10 {
        let (pool, gate, who) = (pool.clone(), gate.clone(), bob.id);
        tasks.push(tokio::spawn(async move { gate.wait().await; credits::apply(&pool, who, 5, "grant", &format!("g{i}"), None).await.unwrap() }));
    }
    for task in tasks { task.await.unwrap(); }
    assert_eq!(credits::statement(&pool, bob.id, None, 1).await.unwrap()["balance"], 50);

    // Over HTTP: only an administrator changes a balance; a reader sees only what is theirs.
    let (http, _) = brain_backend::http::router_with_platform(pool.clone(), config(Vec::new()), Platform::default()).await.unwrap();
    let path = format!("/_brain/credits/{subject}");
    let debit = |delta: i64, key: &str| json!({"delta":delta,"reason":"http","idempotency_key":key});
    assert_eq!(call(&http, "POST", &path, Some(&alice), Some(debit(1000, "self-grant"))).await.0, 403);
    assert_eq!(call(&http, "POST", &path, None, Some(debit(1000, "anon"))).await.0, 401);
    let (s, out) = call(&http, "POST", &path, Some(&admin), Some(debit(-4, "h1"))).await;
    assert_eq!((s.as_u16(), &out["balance"], &out["replayed"]), (200, &json!(40), &json!(false)), "{out}");
    assert_eq!(call(&http, "POST", &path, Some(&admin), Some(debit(-4, "h1"))).await.1["replayed"], true);
    assert_eq!(call(&http, "POST", &path, Some(&admin), Some(debit(-5, "h1"))).await.1["error"]["code"], "CREDITS_KEY_REUSED");
    let (s, out) = call(&http, "POST", &path, Some(&admin), Some(debit(-1000, "h2"))).await;
    assert_eq!((s.as_u16(), out["error"]["code"].as_str()), (409, Some("CREDITS_INSUFFICIENT")));
    assert_eq!(call(&http, "POST", &path, Some(&admin), Some(debit(0, "h3"))).await.0, 422);
    assert_eq!(call(&http, "POST", &path, Some(&admin), Some(debit(i64::MAX, "h4"))).await.0, 422);
    assert_eq!(call(&http, "POST", &format!("/_brain/credits/{}", Uuid::new_v4()), Some(&admin), Some(debit(1, "h5"))).await.0, 404);
    assert_eq!(call(&http, "GET", &path, Some(&alice), None).await.1["balance"], 40);
    assert_ne!(call(&http, "GET", &path, Some(&bob), None).await.0, 200);
    // An organization's balance: managers read it, a plain member and an outsider do not.
    let org = call(&http, "POST", "/_brain/orgs", Some(&alice), Some(json!({"name":"A"}))).await.1["id"].as_str().unwrap().to_string();
    let org_path = format!("/_brain/credits/{org}");
    assert_eq!(call(&http, "POST", &org_path, Some(&admin), Some(debit(9, "o1"))).await.1["balance"], 9);
    assert_eq!(call(&http, "GET", &org_path, Some(&alice), None).await.1["balance"], 9);
    assert_ne!(call(&http, "GET", &org_path, Some(&bob), None).await.0, 200);
    // Neither the database role nor a bug can rewrite history, or hold a negative balance.
    for statement in ["UPDATE _brain_credit_ledger SET delta=1", "DELETE FROM _brain_credit_ledger", "TRUNCATE _brain_credit_ledger"] {
        assert!(sqlx::raw_sql(sqlx::AssertSqlSafe(statement)).execute(&pool).await.unwrap_err().to_string().contains("append-only"), "{statement}");
    }
    assert!(sqlx::query("UPDATE _brain_credit_balances SET balance=-1").execute(&pool).await.is_err());
}
