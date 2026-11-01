//! B-28/B-39: an import preserves ids, owners and references, previews without writing, refuses as a whole, and a
//! legacy scrypt account signs in once and is stored with the native hash from then on. Hashes come from Node's
//! `crypto.scryptSync`, an implementation this crate does not use.
use brain_backend::{auth::{Auth, Credentials}, auth_flows::RolesConfig, contract::{Field, Hooks, Policy, Resource}, db, import};
use serde_json::json;
use sqlx::{ConnectOptions, Row};
use std::str::FromStr;
use uuid::Uuid;

const LONG: &str = "$scrypt$ln=14,r=8,p=1$c2FsdHNhbHQ$gmT3kBJViIHN6YEyrGyEbDU5WnAr6SuWLpsHCm6Hcs8";
const SHORT: &str = "$scrypt$ln=14,r=8,p=1$c2FsdHNhbHQ$H2Xb4EpCoC1Pzw7f8r8I1LKQ+AIIk7/hxw+uSRrXqtk";
const SHOP: &str = "11111111-1111-4111-8111-111111111111";
const USER: &str = "22222222-2222-4222-8222-222222222222";
const OTHER: &str = "33333333-3333-4333-8333-333333333333";
const ITEM: &str = "44444444-4444-4444-8444-444444444444";

fn resource(name: &str, fields: Vec<(&str, Field)>) -> Resource {
    Resource {
        name: name.into(),
        search: vec![],
        fields: fields.into_iter().map(|(n, f)| (n.to_string(), f)).collect(),
        policy: serde_json::from_value::<Policy>(json!({"read": ["public"], "create": ["user"], "update": ["owner"], "delete": ["owner"]})).unwrap(),
        hooks: Hooks::default(),
        constraints: Default::default(),
        query: Default::default(),
    }
}
async fn database() -> sqlx::PgPool {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_imports_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&base).await.unwrap();
    base.close().await;
    db::connect(sqlx::postgres::PgConnectOptions::from_str(&url).unwrap().database(&name).to_url_lossy().as_str()).await.unwrap()
}
async fn count(pool: &sqlx::PgPool, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}"))).fetch_one(pool).await.unwrap()
}
async fn stored_hash(pool: &sqlx::PgPool, id: &str) -> String {
    sqlx::query("SELECT password_hash FROM _brain_users WHERE id=$1").bind(Uuid::parse_str(id).unwrap()).fetch_one(pool).await.unwrap().get(0)
}

#[tokio::test]
async fn an_import_keeps_identity_previews_refuses_as_a_whole_and_upgrades_legacy_hashes() {
    let pool = database().await;
    let field = |kind: &str| Field { kind: kind.into(), required: true, ..Default::default() };
    let shops = resource("shops", vec![("name", field("str"))]);
    let items = resource("items", vec![("title", field("str")), ("shop", field("ref:shops"))]);
    db::migrate(&pool, &[shops.clone(), items.clone()]).await.unwrap();
    let config = RolesConfig::default();
    config.register(&pool).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let roles = vec![config.signup_role.clone()];
    let role = &config.signup_role;

    let accounts = format!(
        "{}\n{}\n{}\n",
        json!({"id": USER, "email": "Long@Example.test", "password_hash": LONG, "role": role, "verified": true, "created_at": "2024-03-01T10:00:00Z"}),
        json!({"id": OTHER, "email": "short@example.test", "password_hash": SHORT, "role": role}),
        json!({"id": "55555555-5555-4555-8555-555555555555", "email": "none@example.test", "role": role}),
    );
    let preview = import::users(&pool, &roles, auth.unusable_hash(), &accounts, false).await.unwrap();
    assert_eq!((preview.rows, preview.applied, preview.problems.len()), (3, false, 0), "{preview:?}");
    assert_eq!(count(&pool, "_brain_users").await, 0, "a preview writes nothing");
    assert!(import::users(&pool, &roles, auth.unusable_hash(), &accounts, true).await.unwrap().applied);
    let kept: (bool, String) = sqlx::query_as("SELECT verified,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD') FROM _brain_users WHERE id=$1").bind(Uuid::parse_str(USER).unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!(kept, (true, "2024-03-01".to_string()));

    // A bad line refuses the whole file, and the reason never repeats a value from it.
    let bad = format!(
        "{}\n{}\n{}\nnot json\n",
        json!({"id": "66666666-6666-4666-8666-666666666666", "email": "ok@example.test", "role": role}),
        json!({"id": "77777777-7777-4777-8777-777777777777", "email": "plain@example.test", "password_hash": "SECRET-plain-md5", "role": role}),
        json!({"id": USER, "email": "again@example.test", "role": role}),
    );
    let refused = import::users(&pool, &roles, auth.unusable_hash(), &bad, true).await.unwrap();
    assert!(!refused.applied);
    assert_eq!(refused.problems.iter().map(|p| p.0).collect::<Vec<_>>(), [2, 3, 4], "{refused:?}");
    assert!(!format!("{refused:?}").contains("SECRET"), "{refused:?}");
    assert_eq!(count(&pool, "_brain_users").await, 3);
    let wrong_role = json!({"id": "88888888-8888-4888-8888-888888888888", "email": "r@example.test", "role": "admin"}).to_string();
    assert_eq!(import::users(&pool, &roles, auth.unusable_hash(), &wrong_role, true).await.unwrap().problems.len(), 1, "an undeclared role never enters");

    // Legacy logins: right password once, native hash from then on; a short legacy password still works.
    let login = |email: &str, password: &str| Credentials { email: email.into(), password: password.into() };
    assert!(auth.login(login("long@example.test", "wrong password here")).await.unwrap().is_none());
    assert_eq!(stored_hash(&pool, USER).await, LONG, "a failed login changes nothing");
    let session = auth.login(login("long@example.test", "correct horse battery")).await.unwrap().expect("legacy password");
    assert_eq!(session.0.id.to_string(), USER, "the imported id is the session's user");
    let upgraded = stored_hash(&pool, USER).await;
    assert!(upgraded.starts_with("$argon2"), "{upgraded}");
    assert!(auth.login(login("long@example.test", "correct horse battery")).await.unwrap().is_some());
    assert!(auth.login(login("short@example.test", "short1")).await.unwrap().is_some());
    assert!(stored_hash(&pool, OTHER).await.starts_with("$argon2"));
    assert!(auth.login(login("none@example.test", "any password at all")).await.unwrap().is_none());

    // Records: ids, owners, references and dates stay; a missing target and bad data refuse the file.
    let shop_rows = json!({"id": SHOP, "owner": USER, "data": {"name": "North"}, "created_at": "2023-01-02T03:04:05+00:00"}).to_string();
    let item_rows = json!({"id": ITEM, "owner": OTHER, "data": {"title": "Rake", "shop": SHOP}}).to_string();
    let early = import::records(&pool, &items, &item_rows, true).await.unwrap();
    assert!(!early.applied && early.problems[0].1.contains("dependency order"), "{early:?}");
    assert!(import::records(&pool, &shops, &shop_rows, true).await.unwrap().applied);
    assert!(import::records(&pool, &items, &item_rows, true).await.unwrap().applied);
    let row: (Uuid, Uuid, String) = sqlx::query_as("SELECT id,owner,to_char(created_at AT TIME ZONE 'UTC','YYYY') FROM _brain_res_shops").fetch_one(&pool).await.unwrap();
    assert_eq!((row.0.to_string().as_str(), row.1.to_string().as_str(), row.2.as_str()), (SHOP, USER, "2023"));
    let again = import::records(&pool, &shops, &shop_rows, true).await.unwrap();
    assert!(again.problems[0].1.contains("already exists"), "{again:?}");
    let invalid = json!({"id": Uuid::new_v4(), "owner": USER, "data": {"nope": 1}}).to_string();
    let refused = import::records(&pool, &shops, &invalid, true).await.unwrap();
    assert!(!refused.applied && refused.problems[0].1.contains("invalid"), "{refused:?}");
    assert_eq!(count(&pool, "_brain_res_shops").await, 1);
    pool.close().await;
}
