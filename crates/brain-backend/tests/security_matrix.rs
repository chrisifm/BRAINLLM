//! Role x resource x operation x row state on the native backend. The expected answer of every cell comes from an
//! evaluator of the declared contract written here, apart from the server's, so a change on either side turns a cell red.
use axum::{Router, body::{Body, to_bytes}, http::Request};
use brain_backend::{auth, contract::{Field, Hooks, Policy, Resource}, db};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::{collections::BTreeMap, str::FromStr, sync::atomic::{AtomicU32, Ordering}};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5195";

/// One entry of a policy list, in the words of the contract.
#[derive(Clone, Copy)]
enum Grant {
    Public,
    PublicWhenOpen,
    Role(&'static str),
    Owner,
    OwnerOfParent,
}
use Grant::*;
impl Grant {
    fn contract(self) -> Value {
        match self {
            Public => json!("public"),
            PublicWhenOpen => json!({"public":{"where":{"status":"open"}}}),
            Role(role) => json!(role),
            Owner => json!("owner"),
            OwnerOfParent => json!("owner.parent"),
        }
    }
}
struct Spec {
    name: &'static str,
    read: &'static [Grant],
    create: &'static [Grant],
    update: &'static [Grant],
    delete: &'static [Grant],
    all_rows: bool,
}
const SPECS: &[Spec] = &[
    Spec { name: "notices", read: &[PublicWhenOpen, Owner, Role("admin")], create: &[Role("member")], update: &[Owner, Role("admin")], delete: &[Role("admin")], all_rows: false },
    Spec { name: "posts", read: &[Public], create: &[Role("member"), Role("seller")], update: &[Owner], delete: &[Owner], all_rows: true },
    Spec { name: "vault", read: &[Owner], create: &[Role("member")], update: &[Owner], delete: &[Owner], all_rows: false },
    Spec { name: "staff", read: &[Role("seller"), Role("admin")], create: &[Role("admin")], update: &[Role("admin")], delete: &[Role("admin")], all_rows: false },
    Spec { name: "dropbox", read: &[Role("admin")], create: &[Role("member")], update: &[Owner], delete: &[Role("admin")], all_rows: false },
    Spec { name: "children", read: &[Owner, OwnerOfParent, Role("admin")], create: &[Role("member")], update: &[OwnerOfParent], delete: &[Owner], all_rows: false },
];
fn field(kind: &str) -> Field {
    Field { kind: kind.into(), ..Default::default() }
}
fn resources() -> Vec<Resource> {
    let make = |name: &str, policy: Value, fields: Vec<(&str, Field)>| Resource {
        name: name.into(),
        search: vec![],
        fields: fields.into_iter().map(|(n, f)| (n.to_string(), f)).collect(),
        policy: serde_json::from_value::<Policy>(policy).unwrap(),
        hooks: Hooks::default(),
        constraints: Default::default(),
        query: Default::default(),
    };
    let list = |grants: &[Grant]| grants.iter().map(|g| g.contract()).collect::<Vec<_>>();
    let mut out = vec![make("parents", json!({"read":["public"],"expose_all_rows":true,"create":["member"],"update":["owner"],"delete":["owner"]}), vec![("title", field("str"))])];
    for spec in SPECS {
        let mut fields = vec![("title", field("str")), ("status", field("enum(open|closed)"))];
        if spec.name == "children" {
            fields.push(("parent", field("ref:parents")));
        }
        let policy = json!({"read":list(spec.read),"expose_all_rows":spec.all_rows,"create":list(spec.create),"update":list(spec.update),"delete":list(spec.delete)});
        out.push(make(spec.name, policy, fields));
    }
    out
}
struct Identity { label: &'static str, id: Option<Uuid>, role: Option<&'static str>, token: Option<String> }
/// What this evaluator knows about one row.
struct Row { owner: Uuid, parent_owner: Uuid, open: bool }
fn matches(grant: Grant, who: &Identity, row: &Row) -> bool {
    match grant {
        Public => true,
        PublicWhenOpen => row.open,
        Role(role) => who.role == Some(role),
        Owner => who.id == Some(row.owner),
        OwnerOfParent => who.id == Some(row.parent_owner),
    }
}
/// Whether a row could ever decide this grant for this identity, so a denial is about the row and not about the role.
fn depends_on_the_row(grant: Grant, who: &Identity) -> bool {
    match grant {
        Public | PublicWhenOpen => true,
        Role(role) => who.role == Some(role),
        Owner | OwnerOfParent => who.id.is_some(),
    }
}
async fn call(http: &Router, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> (u16, Value) {
    // A new client per request: this test measures the policy, not the rate limit.
    static CLIENT: AtomicU32 = AtomicU32::new(1);
    let n = CLIENT.fetch_add(1, Ordering::Relaxed);
    let ip = std::net::SocketAddr::from(([10, (n >> 16) as u8, (n >> 8) as u8, n as u8], 3456));
    let mut b = Request::builder().extension(axum::extract::ConnectInfo(ip)).method(method).uri(uri).header("origin", ORIGIN).header("accept", "application/json");
    if let Some(token) = token { b = b.header("cookie", format!("__Host-brain={token}")); }
    let req = match body { Some(body) => b.header("content-type", "application/json").body(Body::from(body.to_string())), None => b.body(Body::empty()) }.unwrap();
    let res = http.clone().oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = to_bytes(res.into_body(), 1 << 22).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
async fn actor(pool: &sqlx::PgPool, label: &'static str, role: &'static str) -> Identity {
    let id = Uuid::new_v4();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO _brain_users(id,email,password_hash,role) VALUES($1,$2,'synthetic-unused-hash',$3)")
        .bind(id).bind(format!("{id}@example.test")).bind(role).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO _brain_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')")
        .bind(auth::digest(&token)).bind(id).execute(pool).await.unwrap();
    Identity { label, id: Some(id), role: Some(role), token: Some(token) }
}
fn denied(status: u16, who: &Identity) -> bool {
    if who.id.is_none() { matches!(status, 401 | 403) } else { status == 403 }
}

#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn every_role_resource_operation_and_row_state_answers_what_the_contract_declares() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_security_matrix_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    base.close().await;
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name);
    let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
    let all = resources();
    db::migrate(&pool, &all).await.unwrap();
    let roles = brain_backend::auth_flows::RolesConfig { roles: ["admin", "member", "seller"].map(String::from).to_vec(), signup_role: "member".into(), ..Default::default() };
    roles.register(&pool).await.unwrap();
    let identities = vec![
        Identity { label: "anonymous", id: None, role: None, token: None },
        actor(&pool, "owner", "member").await,
        actor(&pool, "parent owner", "member").await,
        actor(&pool, "stranger", "member").await,
        actor(&pool, "seller", "seller").await,
        actor(&pool, "admin", "admin").await,
    ];
    let (owner, parent_owner) = (identities[1].id.unwrap(), identities[2].id.unwrap());
    let config = brain_backend::Config {
        api_keys: None, roles, mail: None, auth_public_base_url: None, webhooks: None, before_hooks: None, sockets: None, gcs: None, jwt: None, google: None, apple: None, linkedin: None,
        resources: all.clone(), jobs: Vec::new(), uploads: std::env::temp_dir().join(format!("brain-security-matrix-{}", Uuid::new_v4().simple())),
        origins: vec![ORIGIN.into()], production: false,
    };
    let http = brain_backend::http::router(pool.clone(), config).await.unwrap();
    let by_name: BTreeMap<&str, &Resource> = all.iter().map(|r| (r.name.as_str(), r)).collect();
    let parent = db::create(&pool, by_name["parents"], parent_owner, json!({"title":"Parent"})).await.unwrap()["id"].as_str().unwrap().to_string();
    let data = |spec: &Spec, status: &str, title: &str| {
        let mut value = json!({"title":title,"status":status});
        if spec.name == "children" { value["parent"] = json!(parent); }
        value
    };
    let (mut cells, mut wrong) = (0, Vec::new());
    for spec in SPECS {
        let resource = by_name[spec.name];
        let table = format!("_brain_res_{}", spec.name);
        let exists = async |id: &str| -> bool {
            sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id=$1::uuid)"))).bind(id).fetch_one(&pool).await.unwrap()
        };
        let title_of = async |id: &str| -> Option<String> {
            sqlx::query_scalar::<_, Option<String>>(sqlx::AssertSqlSafe(format!("SELECT data->>'title' FROM {table} WHERE id=$1::uuid"))).bind(id).fetch_one(&pool).await.unwrap()
        };
        for who in &identities {
            let token = who.token.as_deref();
            // create: the role decides, a row does not exist yet.
            cells += 1;
            let allowed = spec.create.iter().any(|&g| matches!(g, Role(r) if who.role == Some(r)));
            let (status, body) = call(&http, "POST", &format!("/api/{}", spec.name), token, Some(data(spec, "open", "Created"))).await;
            if allowed != (status == 200) || (!allowed && !denied(status, who)) {
                wrong.push(format!("{} create as {}: {status} {body}", spec.name, who.label));
            }
            let reads_own = spec.read.iter().any(|&g| matches(g, who, &Row { owner: who.id.unwrap_or_default(), parent_owner, open: true }));
            if allowed && !reads_own && body["data"] != json!({}) {
                wrong.push(format!("{} create returned a hidden row as {}: {status} {body}", spec.name, who.label));
            }
            for (state, open) in [("open", true), ("closed", false)] {
                let row = Row { owner, parent_owner, open };
                let readable = spec.read.iter().any(|&g| matches(g, who, &row));
                let may_read_some = spec.read.iter().any(|&g| depends_on_the_row(g, who));
                let id = db::create(&pool, resource, owner, data(spec, state, "Original")).await.unwrap()["id"].as_str().unwrap().to_string();
                let cell = |operation: &str, status: u16, body: &Value| format!("{} {operation} of a {state} row as {}: {status} {body}", spec.name, who.label);

                // list: the row is there exactly when it is readable.
                cells += 1;
                let (status, body) = call(&http, "GET", &format!("/api/{}?limit=100", spec.name), token, None).await;
                let listed = body["items"].as_array().is_some_and(|items| items.iter().any(|item| item["id"] == json!(id)));
                if listed != readable || status >= 500 || (!may_read_some && !denied(status, who)) {
                    wrong.push(cell("list", status, &body));
                }
                // get: outside the scope answers like a missing row; a role with no grant at all learns nothing either.
                cells += 1;
                let (status, body) = call(&http, "GET", &format!("/api/{}/{id}", spec.name), token, None).await;
                let right = if readable { status == 200 } else if may_read_some { status == 404 } else { denied(status, who) };
                if !right { wrong.push(cell("get", status, &body)); }
                // A hidden row answers exactly like an id that does not exist, so nobody can probe which ids are real.
                if !readable {
                    let (missing, missing_body) = call(&http, "GET", &format!("/api/{}/{}", spec.name, Uuid::new_v4()), token, None).await;
                    if (missing, &missing_body["error"]["code"]) != (status, &body["error"]["code"]) {
                        wrong.push(cell(&format!("get differs from a missing id ({missing}), so"), status, &body));
                    }
                }

                // update, by PATCH and by PUT.
                let writable = spec.update.iter().any(|&g| matches(g, who, &row));
                let may_write_some = spec.update.iter().any(|&g| depends_on_the_row(g, who));
                for (verb, operation) in [("PATCH", "patch"), ("PUT", "put")] {
                    cells += 1;
                    let (status, body) = call(&http, verb, &format!("/api/{}/{id}", spec.name), token, Some(data(spec, state, operation))).await;
                    let changed = title_of(&id).await.as_deref() == Some(operation);
                    let right = if writable { status == 200 && changed }
                        else if !may_write_some { denied(status, who) && !changed }
                        else if readable { status == 403 && !changed }
                        else { status == 404 && !changed };
                    if !right { wrong.push(cell(operation, status, &body)); }
                    // A write never returns a row its author may not read.
                    if writable && !readable && body["data"] != json!({}) { wrong.push(cell(&format!("{operation} returned a hidden row"), status, &body)); }
                }
                // delete last: it ends the row.
                cells += 1;
                let deletable = spec.delete.iter().any(|&g| matches(g, who, &row));
                let may_delete_some = spec.delete.iter().any(|&g| depends_on_the_row(g, who));
                let (status, body) = call(&http, "DELETE", &format!("/api/{}/{id}", spec.name), token, None).await;
                let gone = !exists(&id).await;
                let right = if deletable { status < 300 && gone }
                    else if !may_delete_some { denied(status, who) && !gone }
                    else if readable { status == 403 && !gone }
                    else { status == 404 && !gone };
                if !right { wrong.push(cell("delete", status, &body)); }
            }
        }
    }
    // The matrix is generated: a contract edit that silently shrinks it must not pass.
    assert_eq!(cells, SPECS.len() * 6 * (1 + 2 * 5), "cells");
    assert!(wrong.is_empty(), "{} of {cells} cells differ from the declared contract:\n{}", wrong.len(), wrong.join("\n"));
}
