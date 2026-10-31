use brain_backend::{
    auth::{Auth, Credentials},
    contract::{Field, Hooks, Policy, Resource},
    db,
    jobs::Queue,
};
use serde_json::json;
use std::collections::BTreeMap;
use uuid::Uuid;

fn resource() -> Resource {
    Resource {
        name: "articles".into(),
        search: vec!["title".into()],
        fields: BTreeMap::from([(
            "title".into(),
            Field {
                kind: "str".into(),
                required: true,
                ..Default::default()
            },
        )]),
        policy: Policy::default(),
        hooks: Hooks::default(),
        constraints: Default::default(),
        query: Default::default(),
    }
}
#[test]
fn contracts_reject_unsafe_names_public_writes_unknown_fields_and_missing_required_fields() {
    let mut r = resource();
    assert!(r.validate().is_ok());
    assert!(r.input(&json!({"title":"Hello"})).is_ok());
    assert!(r.input(&json!({"title":"Hello","role":"admin"})).is_err());
    assert!(r.input(&json!({})).is_err());
    r.policy.create = vec!["public".into()];
    assert!(r.validate().is_err());
    r.policy = Policy::default();
    r.name = "posts; DROP TABLE users".into();
    assert!(r.validate().is_err());
    assert!(!r.policy.allows("read", None, true));
    assert!(
        Policy {
            read: vec!["owner".into()],
            ..Policy::default()
        }
        .owners_only("read", Some("candidate"))
    );
}

/// Run against a disposable Postgres database via scripts/backend-check.mjs. Never selects a developer database.
#[tokio::test]
#[ignore = "Run scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn postgres_auth_policy_crud_transactional_hooks_and_leased_jobs() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL")
        .expect("Use scripts/backend-check.mjs; no developer database is selected implicitly");
    let pool = db::connect(&url).await.unwrap();
    let mut r = resource();
    r.hooks.after_create = Some("article_created".into());
    db::migrate(&pool, &[r.clone()]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let user = auth
        .signup(Credentials {
            email: "reader@example.test".into(),
            password: "a long test password".into(),
        })
        .await
        .unwrap();
    assert_eq!(user.role, "candidate");
    assert!(
        auth.login(Credentials {
            email: "reader@example.test".into(),
            password: "wrong".into()
        })
        .await
        .unwrap()
        .is_none()
    );
    let (_, token) = auth
        .login(Credentials {
            email: "reader@example.test".into(),
            password: "a long test password".into(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(auth.user(Some(&token)).await.unwrap().unwrap().id, user.id);
    let row = db::create(&pool, &r, user.id, json!({"title":"Searchable article"}))
        .await
        .unwrap();
    let id: Uuid = row["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        db::list(&pool, &r, Some(user.id), "searchable", 25, 0)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        db::update(
            &pool,
            &r,
            id,
            Some(Uuid::new_v4()),
            json!({"title":"Not allowed"})
        )
        .await
        .unwrap()
        .is_none()
    );
    let q = Queue::new(pool.clone());
    let hook = q.claim("article_created", 90).await.unwrap().unwrap();
    assert_eq!(hook.payload["record"]["id"], row["id"]);
    assert!(q.ack(&hook).await.unwrap());
    assert!(!q.ack(&hook).await.unwrap());
    let first = q
        .enqueue("mail", json!({"message":"one"}), Some("first"), 2)
        .await
        .unwrap();
    assert_eq!(
        first,
        q.enqueue(
            "mail",
            json!({"message":"ignored duplicate"}),
            Some("first"),
            2
        )
        .await
        .unwrap()
    );
    q.enqueue("mail", json!({"message":"two"}), None, 2)
        .await
        .unwrap();
    let (a, b) = tokio::join!(q.claim("mail", 90), q.claim("mail", 90));
    let a = a.unwrap().unwrap();
    let b = b.unwrap().unwrap();
    assert_ne!(a.id, b.id);
    sqlx::query("UPDATE _brain_jobs SET lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    let again = q.claim("mail", 90).await.unwrap().unwrap();
    assert_eq!(again.id, a.id);
    assert_ne!(again.lease, a.lease);
    assert_eq!(again.attempt, 2);
    assert!(!q.ack(&a).await.unwrap());
    assert!(q.fail(&again).await.unwrap());
    assert_eq!(q.status(again.id).await.unwrap().unwrap()["state"], "dead");
    assert!(q.ack(&b).await.unwrap());
    // Exercise the actual HTTP lifecycle as well as the repository: origins, cookie auth, field validation and ownership.
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let mut exposed = r.clone();
    exposed.policy = Policy {
        read: vec!["owner".into()],
        create: vec!["candidate".into()],
        update: vec!["owner".into()],
        delete: vec!["owner".into()],
    };
    let http = brain_backend::http::router(
        pool.clone(),
        brain_backend::Config { api_keys: None,
            roles: Default::default(),
            mail: None,
            auth_public_base_url: None,
            webhooks: None,before_hooks:None,sockets:None,
            gcs: None,
            jwt: None,
            google: None,
            resources: vec![exposed],
            jobs: vec![Resource {
                name: "article_created".into(),
                search: Vec::new(),
                fields: BTreeMap::from([
                    (
                        "resource".into(),
                        Field {
                            kind: "str".into(),
                            required: true,
                            ..Default::default()
                        },
                    ),
                    (
                        "operation".into(),
                        Field {
                            kind: "str".into(),
                            required: true,
                            ..Default::default()
                        },
                    ),
                    (
                        "record".into(),
                        Field {
                            kind: "obj".into(),
                            required: true,
                            ..Default::default()
                        },
                    ),
                ]),
                policy: Policy::default(),
                hooks: Hooks::default(),
                constraints: Default::default(),
                query: Default::default(),
            }],
            uploads: std::env::temp_dir().join(format!("brain-upload-test-{}", std::process::id())),
            origins: vec!["http://localhost:5182".into()],
            production: true,
        },
    )
    .await
    .unwrap();
    let request = |origin: bool, body: &str| {
        let mut b = Request::builder()
            .extension(axum::extract::ConnectInfo(
                "127.0.0.1:3456".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .method("POST")
            .uri("/api/articles")
            .header("content-type", "application/json")
            .header("cookie", format!("__Host-brain={token}"));
        if origin {
            b = b.header("origin", "http://localhost:5182");
        }
        b.body(Body::from(body.to_string())).unwrap()
    };
    let denied = http
        .clone()
        .oneshot(request(false, r#"{"title":"Origin denied"}"#))
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);
    assert!(denied.headers().contains_key("x-request-id"));
    let invalid = http
        .clone()
        .oneshot(request(true, r#"{"title":"Hello","owner":"forged"}"#))
        .await
        .unwrap();
    assert_eq!(invalid.status(), 422);
    let valid = http
        .clone()
        .oneshot(request(true, r#"{"title":"HTTP created"}"#))
        .await
        .unwrap();
    assert_eq!(valid.status(), 200);
    let body = to_bytes(valid.into_body(), 65536).await.unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(created["owner"], user.id.to_string());
    let schema = http
        .clone()
        .oneshot(
            Request::builder()
                .extension(axum::extract::ConnectInfo(
                    "127.0.0.1:3456".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .uri("/_brain/api")
                .header("cookie", format!("__Host-brain={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(schema.status(), 403, "production OpenAPI needs admin");
    // Schema changes cannot silently reinterpret existing rows.
    let mut changed = r.clone();
    changed.fields.get_mut("title").unwrap().kind = "bool".into();
    assert!(db::migrate(&pool, &[changed]).await.is_err());
    // A hook failure rolls back its row rather than losing its side effect.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_res_articles")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO _brain_jobs(id,queue,payload,max_attempts) SELECT gen_random_uuid(),'article_created','{}',1 FROM generate_series(1,10000)").execute(&pool).await.unwrap();
    assert!(
        db::create(&pool, &r, user.id, json!({"title":"Must roll back"}))
            .await
            .is_err()
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_res_articles")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, after);
    auth.logout(Some(&token)).await.unwrap();
    assert!(auth.user(Some(&token)).await.unwrap().is_none());
    pool.close().await;
}
