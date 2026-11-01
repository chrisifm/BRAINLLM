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
    brain_backend::auth_flows::RolesConfig { roles: ["admin", "user"].map(String::from).to_vec(), signup_role: "user".into() }
}
fn config(resources: Vec<Resource>) -> brain_backend::Config {
    brain_backend::Config {
        api_keys: Some(Default::default()), roles: roles(), mail: None, auth_public_base_url: None, webhooks: None, before_hooks: None,
        sockets: None, gcs: None, jwt: None, google: None, resources, jobs: Vec::new(),
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
    let stats = call(&f.http, "GET", "/api/projects?aggregate=stats,by_tag&total=true", Some(a), None).await.1;
    let text = stats.to_string();
    assert!(!text.contains("700") && !text.contains("beta"), "{text}");
    assert!(text.contains('5'), "{text}");
    // Relation filters and counts stay inside the caller's organization.
    assert!(call(&f.http, "GET", &format!("/api/tasks?project={}", f.project_b), Some(a), None).await.1["items"].as_array().unwrap().is_empty());
    assert!(call(&f.http, "GET", "/api/tasks?project.tag=beta", Some(a), None).await.1["items"].as_array().unwrap().is_empty());
    // Anonymous readers have no grant; a global administrator holds no membership and sees nothing.
    assert_eq!(call(&f.http, "GET", "/api/projects", None, None).await.0, 403);
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
