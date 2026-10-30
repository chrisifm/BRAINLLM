//! Real TCP account routes, without calling an external mail provider. Synthetic hashes are seeded by the fixture.
use brain_backend::{
    auth::{self, Auth, Credentials},
    auth_flows::{self, RolesConfig},
    db, jwt,
};
use serde_json::{Value, json};
use sqlx::ConnectOptions;
use std::{path::PathBuf, str::FromStr, time::Duration};
use uuid::Uuid;

struct Server {
    url: String,
    client: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    files: PathBuf,
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.files);
    }
}
impl Server {
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        cookie: Option<&str>,
        origin: &str,
    ) -> (u16, reqwest::header::HeaderMap, Value) {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.url))
            .header("Origin", origin)
            .header("Accept", "application/json");
        if let Some(cookie) = cookie {
            request = request.header("Cookie", cookie)
        }
        if let Some(body) = body {
            request = request.json(&body)
        }
        let mut response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.unwrap() {
            assert!(bytes.len() + chunk.len() <= 256 * 1024);
            bytes.extend_from_slice(&chunk)
        }
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, body)
    }
    async fn post(
        &self,
        path: &str,
        body: Value,
        cookie: Option<&str>,
    ) -> (u16, reqwest::header::HeaderMap, Value) {
        self.call(
            reqwest::Method::POST,
            path,
            Some(body),
            cookie,
            "https://app.example.test",
        )
        .await
    }
    async fn get(
        &self,
        path: &str,
        cookie: Option<&str>,
    ) -> (u16, reqwest::header::HeaderMap, Value) {
        self.call(
            reqwest::Method::GET,
            path,
            None,
            cookie,
            "https://app.example.test",
        )
        .await
    }
}
async fn database() -> sqlx::PgPool {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL")
        .expect("Only the explicitly owned disposable Postgres harness is supported");
    let base = db::connect(&url).await.unwrap();
    let name = format!("brain_auth_http_{}", Uuid::new_v4().simple());
    // The identifier contains a fixed ASCII prefix plus UUID hex, never request input.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&base)
        .await
        .unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url)
        .unwrap()
        .database(&name);
    let pool = db::connect(options.to_url_lossy().as_str()).await.unwrap();
    base.close().await;
    db::migrate(&pool, &[]).await.unwrap();
    pool
}
fn cookie(headers: &reqwest::header::HeaderMap) -> String {
    for name in [jwt::ACCESS_COOKIE, auth::COOKIE] {
        if let Some(cookie) = headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|h| h.to_str().ok())
            .filter(|h| !h.contains("Max-Age=0"))
            .find_map(|h| {
                h.split(';')
                    .next()
                    .filter(|h| h.starts_with(&format!("{name}=")))
                    .map(str::to_owned)
            })
        {
            return cookie;
        }
    }
    panic!("Native login must set its backend-selected session cookie")
}
async fn challenge(pool: &sqlx::PgPool, user: Uuid, purpose: &str, expired: bool) -> String {
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO _brain_auth_tokens(token_hash,user_id,purpose,expires_at,email_bound,auth_version,activated_at) SELECT $1,id,$2,now()+$3::integer*interval '1 second',email,auth_version,now() FROM _brain_users WHERE id=$4")
        .bind(auth::digest(&token)).bind(purpose).bind(if expired{-1}else{300}).bind(user).execute(pool).await.unwrap();
    token
}
#[tokio::test]
#[ignore = "Run with the explicitly owned disposable Postgres harness; never sends external mail"]
async fn real_http_account_handlers_enforce_identity_replay_revocation_custom_roles_and_contracts()
{
    let pool = database().await;
    let mut roles = RolesConfig::default();
    roles.roles.push("editor".into());
    roles.signup_role = "user".into();
    roles.register(&pool).await.unwrap();
    let files =
        std::env::temp_dir().join(format!("brain-auth-http-files-{}", Uuid::new_v4().simple()));
    use brain_backend::webhooks::{Config as WebhookConfig, Inbound, Outbound, Secret};
    let secret = |byte: &str| Secret::from_hex(&byte.repeat(32)).unwrap();
    let webhook_config = WebhookConfig::new(
        vec![secret("01")],
        std::collections::BTreeMap::from([(
            "incoming".into(),
            Inbound::new("incoming_work".into(), vec![secret("02")]).unwrap(),
        )]),
        std::collections::BTreeMap::from([(
            "partner".into(),
            Outbound::new(
                "https://example.test/private".into(),
                secret("03"),
                vec!["https://example.test".into()],
            )
            .unwrap(),
        )]),
    )
    .unwrap();
    let incoming=serde_json::from_value(json!({"name":"incoming_work","fields":{"message":{"kind":"str","required":true}},"policy":{"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}})).unwrap();
    let config = brain_backend::Config {
        resources: vec![],
        jobs: vec![incoming],
        uploads: files.clone(),
        origins: vec!["https://app.example.test".into()],
        production: true,
        jwt: Some(
            jwt::Config::new(
                "synthetic-http-signing-key-over-32-bytes".into(),
                "http-test".into(),
                "http-audience".into(),
            )
            .unwrap(),
        ),
        google: None,
        gcs: None,
        roles: roles.clone(),
        mail: None,
        auth_public_base_url: Some("https://app.example.test/project/app".into()),
        webhooks: Some(webhook_config),
        before_hooks: None,
        sockets: None,
    };
    let router = brain_backend::http::router(pool.clone(), config)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .await
        .unwrap()
    });
    let mut server = Server {
        url,
        task,
        files,
        stop: Some(stop),
        client: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .pool_max_idle_per_host(0)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
    };
    // Provider calls deliberately omit Origin. Only correctly signed inbound work is accepted.
    let event = Uuid::new_v4().to_string();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    let body = br#"{"message":"private-http-fixture"}"#;
    let mut frame = format!("brain-webhook-v1\n{event}\n{timestamp}\n").into_bytes();
    frame.extend_from_slice(body);
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &[2u8; 32]);
    let signature = format!(
        "v1={}",
        ring::hmac::sign(&key, &frame)
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let signed = || {
        server
            .client
            .post(format!("{}/_brain/webhooks/in/incoming", server.url))
            .header("Content-Type", "application/json")
            .header("x-brain-webhook-id", &event)
            .header("x-brain-webhook-timestamp", &timestamp)
            .header("x-brain-webhook-signature", &signature)
            .body(body.to_vec())
    };
    let receipt: Value = signed()
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(receipt["duplicate"], false);
    let duplicate: Value = signed().send().await.unwrap().json().await.unwrap();
    assert_eq!(receipt["id"], duplicate["id"]);
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(
        signed()
            .body(br#"{"message":"tampered"}"#.to_vec())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        signed()
            .header("x-brain-webhook-signature", "v1=00")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        server
            .client
            .post(format!("{}/_brain/webhooks/out/partner", server.url))
            .json(&json!({"eventId":Uuid::new_v4(),"payload":{}}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let address = format!("http-{}@example.test", Uuid::new_v4().simple());
    let password = "synthetic-http-original-password";
    let (status, _, signup) = server
        .post(
            "/_brain/auth/signup",
            json!({"email":address,"password":password}),
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(signup["user"]["role"], "user");
    let user: Uuid = signup["user"]["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        server
            .post(
                "/_brain/auth/signup",
                json!({"email":"attacker@example.test","password":password,"role":"admin"}),
                None
            )
            .await
            .0,
        422
    );
    let (status, headers, logged_in) = server
        .post(
            "/_brain/auth/login",
            json!({"email":address,"password":password}),
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert!(
        logged_in["access_token"].as_str().is_some(),
        "Configured JWT mode is selected by /login itself"
    );
    assert_eq!(logged_in["token_type"], "Bearer");
    let old_cookie = cookie(&headers);
    let auth = Auth::new_with_role(pool.clone(), "user").await.unwrap();
    let (_, old_opaque) = auth
        .login(Credentials {
            email: address.clone(),
            password: password.into(),
        })
        .await
        .unwrap()
        .unwrap();
    let old_opaque_cookie = format!("{}={old_opaque}", auth::COOKIE);
    let other = auth
        .signup(Credentials {
            email: format!("other-{}@example.test", Uuid::new_v4().simple()),
            password: password.into(),
        })
        .await
        .unwrap();
    let (_, opaque) = auth
        .login(Credentials {
            email: other.email.clone(),
            password: password.into(),
        })
        .await
        .unwrap()
        .unwrap();
    let other_cookie = format!("{}={opaque}", auth::COOKIE);
    let (known, _, known_body) = server
        .post(
            "/_brain/auth/password/request",
            json!({"email":address}),
            None,
        )
        .await;
    let (unknown, _, unknown_body) = server
        .post(
            "/_brain/auth/password/request",
            json!({"email":"unknown@example.test"}),
            None,
        )
        .await;
    assert_eq!((known, unknown), (503, 503));
    assert_eq!(known_body["error"], unknown_body["error"]);
    assert_eq!(known_body["error"]["code"], "AUTH_MAIL_NOT_CONFIGURED");
    assert_eq!(
        server
            .post("/_brain/auth/email/request", json!({}), None)
            .await
            .0,
        401
    );
    assert_eq!(
        server
            .post("/_brain/auth/email/request", json!({}), Some(&old_cookie))
            .await
            .0,
        503
    );
    assert_eq!(
        server
            .post(
                "/_brain/auth/email/request",
                json!({"role":"admin"}),
                Some(&old_cookie)
            )
            .await
            .0,
        422
    );
    let verify = challenge(&pool, user, "verify", false).await;
    assert_eq!(
        server
            .post("/_brain/auth/email/verify", json!({"token":verify}), None)
            .await
            .0,
        401
    );
    assert_eq!(
        server
            .post(
                "/_brain/auth/email/verify",
                json!({"token":verify}),
                Some(&other_cookie)
            )
            .await
            .0,
        422
    );
    assert_eq!(
        server
            .call(
                reqwest::Method::POST,
                "/_brain/auth/email/verify",
                Some(json!({"token":verify})),
                Some(&old_cookie),
                "https://other.example.test"
            )
            .await
            .0,
        403
    );
    assert_eq!(
        server
            .post(
                "/_brain/auth/email/verify",
                json!({"token":verify}),
                Some(&old_cookie)
            )
            .await
            .0,
        200
    );
    assert_eq!(
        server
            .post(
                "/_brain/auth/email/verify",
                json!({"token":verify}),
                Some(&old_cookie)
            )
            .await
            .0,
        422
    );
    assert!(
        server.get("/_brain/auth/me", Some(&old_cookie)).await.2["user"]["verified"]
            .as_bool()
            .unwrap()
    );

    let (status, token_headers, tokens) = server
        .post(
            "/_brain/auth/token",
            json!({"email":address,"password":password}),
            None,
        )
        .await;
    assert_eq!(status, 200);
    let refresh_cookie = token_headers
        .get_all("set-cookie")
        .iter()
        .find_map(|v| {
            v.to_str()
                .ok()?
                .split(';')
                .next()
                .filter(|v| v.starts_with("__Host-brain-refresh="))
                .map(str::to_owned)
        })
        .unwrap();
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let reset = challenge(&pool, user, "reset", false).await;
    let replacement = "synthetic-http-replacement-password";
    let (status, headers, body) = server
        .post(
            "/_brain/auth/password/reset",
            json!({"token":reset,"password":replacement}),
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    assert_eq!(headers["clear-site-data"], "\"storage\"");
    let cleared: Vec<_> = headers
        .get_all("set-cookie")
        .iter()
        .map(|h| h.to_str().unwrap())
        .collect();
    for name in [
        auth::COOKIE,
        jwt::ACCESS_COOKIE,
        jwt::REFRESH_COOKIE,
        "__Host-brain-google",
    ] {
        assert!(
            cleared
                .iter()
                .any(|h| h.starts_with(&format!("{name}=")) && h.contains("Max-Age=0"))
        )
    }
    let serialized = serde_json::to_string(&body).unwrap();
    assert!(!serialized.contains(&reset) && !serialized.contains(replacement));
    assert_eq!(
        server
            .post(
                "/_brain/auth/password/reset",
                json!({"token":reset,"password":replacement}),
                None
            )
            .await
            .0,
        422
    );
    assert_eq!(
        server.get("/_brain/auth/me", Some(&old_cookie)).await.0,
        401
    );
    assert_eq!(
        server
            .get("/_brain/auth/me", Some(&old_opaque_cookie))
            .await
            .0,
        401
    );
    let response = server
        .client
        .get(format!("{}/_brain/auth/me", server.url))
        .bearer_auth(access)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    drop(response);
    assert_eq!(
        server
            .post("/_brain/auth/refresh", json!({}), Some(&refresh_cookie))
            .await
            .0,
        401
    );
    let expired = challenge(&pool, user, "reset", true).await;
    assert_eq!(
        server
            .post(
                "/_brain/auth/password/reset",
                json!({"token":expired,"password":replacement}),
                None
            )
            .await
            .0,
        422
    );
    let (status, headers, _) = server
        .post(
            "/_brain/auth/login",
            json!({"email":address,"password":replacement}),
            None,
        )
        .await;
    assert_eq!(status, 200);
    let user_cookie = cookie(&headers);
    assert_eq!(
        server
            .post(
                &format!("/_brain/auth/users/{}/role", other.id),
                json!({"role":"admin"}),
                Some(&user_cookie)
            )
            .await
            .0,
        403
    );
    assert_eq!(
        server
            .post("/_brain/auth/bootstrap-admin", json!({"user":user}), None)
            .await
            .0,
        404
    );

    assert!(
        auth_flows::bootstrap_first_admin(&pool, other.id)
            .await
            .unwrap()
    );
    let (_, headers, _) = server
        .post(
            "/_brain/auth/login",
            json!({"email":other.email,"password":password}),
            None,
        )
        .await;
    let admin_cookie = cookie(&headers);
    let (status, _, published) = server
        .post(
            "/_brain/webhooks/out/partner",
            json!({"eventId":Uuid::new_v4(),"payload":{"message":"private-outgoing-http"}}),
            Some(&admin_cookie),
        )
        .await;
    assert_eq!(status, 202);
    let (status, _, delivery) = server
        .get(
            &format!(
                "/_brain/webhooks/out/partner/{}",
                published["id"].as_str().unwrap()
            ),
            Some(&admin_cookie),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(delivery["state"], "pending");
    assert!(!delivery.to_string().contains("private-outgoing-http"));
    assert_eq!(
        server
            .post(
                "/_brain/webhooks/out/partner",
                json!({"eventId":Uuid::new_v4(),"payload":{}}),
                Some(&user_cookie)
            )
            .await
            .0,
        403
    );
    let (status, _, api) = server.get("/_brain/api", Some(&admin_cookie)).await;
    assert_eq!(status, 200);
    let responses=api["paths"]["/_brain/auth/login"]["post"]["responses"]["200"]["content"]["application/json"]["schema"]["oneOf"].as_array().unwrap();
    assert_eq!(responses.len(), 2);
    for schema in responses {
        assert_eq!(
            schema["properties"]["user"]["properties"]["role"]["enum"],
            json!(roles.roles)
        )
    }
    assert_eq!(
        api["paths"]["/_brain/auth/password/request"]["post"]["responses"]["202"]["content"]["application/json"]
            ["schema"]["properties"]["accepted"]["const"],
        true
    );
    for path in [
        "/_brain/auth/email/request",
        "/_brain/auth/email/verify",
        "/_brain/auth/users/{id}/role",
    ] {
        assert!(api["paths"][path]["post"]["security"].is_array())
    }
    assert_eq!(
        server
            .post(
                &format!("/_brain/auth/users/{user}/role"),
                json!({"role":"editor"}),
                Some(&admin_cookie)
            )
            .await
            .0,
        200
    );
    assert_eq!(
        server.get("/_brain/auth/me", Some(&user_cookie)).await.0,
        401
    );
    assert_eq!(
        server
            .post(
                &format!("/_brain/auth/users/{user}/role"),
                json!({"role":"unconfigured"}),
                Some(&admin_cookie)
            )
            .await
            .0,
        422
    );
    assert_eq!(
        server
            .post(
                &format!("/_brain/auth/users/{}/role", other.id),
                json!({"role":"user"}),
                Some(&admin_cookie)
            )
            .await
            .0,
        403
    );
    let (status, _, login) = server
        .post(
            "/_brain/auth/login",
            json!({"email":address,"password":replacement}),
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(login["user"]["role"], "editor");
    let _ = server.stop.take().unwrap().send(());
    tokio::time::timeout(Duration::from_secs(2), &mut server.task)
        .await
        .expect("Owned HTTP listener must shut down")
        .unwrap();
    pool.close().await;
}

struct HookFixture {
    calls: std::sync::atomic::AtomicUsize,
    deny_delete: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl brain_backend::before_hooks::BeforeHooks for HookFixture {
    fn names(&self) -> Vec<String> {
        vec!["validate_item".into()]
    }
    fn run(
        &self,
        name: &str,
        context: Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Value, brain_backend::before_hooks::HookFailure>,
                > + Send
                + '_,
        >,
    > {
        assert_eq!(name, "validate_item");
        Box::pin(async move {
            use brain_backend::before_hooks::HookFailure;
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            assert_eq!(context["resource"], "hook_items");
            assert_eq!(context["user"]["role"], "candidate");
            assert!(
                context["rid"]
                    .as_str()
                    .is_some_and(|s| Uuid::parse_str(s).is_ok())
            );
            if context["operation"] == "delete"
                && self.deny_delete.load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err(HookFailure::Rejected(vec!["title".into()]));
            }
            match context["payload"]["title"].as_str().unwrap_or("") {
                "raw" => Ok(json!({"title":"normalized"})),
                "deny" => Err(HookFailure::Rejected(vec![
                    "title".into(),
                    "not_a_schema_field".into(),
                ])),
                "invalid" => Ok(json!({"unexpected":"Synthetic value must not reach storage"})),
                "pause" => {
                    assert!(context["existing"]["id"].is_string());
                    self.entered.notify_one();
                    self.release.notified().await;
                    Ok(context["payload"].clone())
                }
                "timeout" => {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    Ok(context["payload"].clone())
                }
                _ => Ok(context["payload"].clone()),
            }
        })
    }
}
#[tokio::test]
#[ignore = "Use the explicitly owned disposable Postgres harness; mandatory hooks and real HTTP CAS"]
async fn all_native_mutations_enforce_before_hooks_policy_validation_deadlines_and_row_conflicts() {
    use std::sync::{Arc, atomic::Ordering};
    let pool = database().await;
    let resource:brain_backend::Resource=serde_json::from_value(json!({"name":"hook_items","fields":{"title":{"kind":"str","required":true}},"policy":{"read":["owner"],"create":["candidate"],"update":["owner"],"delete":["owner"]},"hooks":{"before_create":"validate_item","before_update":"validate_item","before_delete":"validate_item","after_create":"item_changed","after_update":"item_changed","after_delete":"item_changed"}})).unwrap();
    let job:brain_backend::Resource=serde_json::from_value(json!({"name":"item_changed","fields":{"resource":{"kind":"str","required":true},"operation":{"kind":"str","required":true},"record":{"kind":"obj","required":true}},"policy":{"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}})).unwrap();
    db::migrate(&pool, &[resource.clone()]).await.unwrap();
    let hooks = Arc::new(HookFixture {
        calls: 0.into(),
        deny_delete: false.into(),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let files = std::env::temp_dir().join(format!("brain-before-http-{}", Uuid::new_v4().simple()));
    let config = |executor| brain_backend::Config {
        resources: vec![resource.clone()],
        jobs: vec![job.clone()],
        uploads: files.clone(),
        origins: vec!["https://app.example.test".into()],
        production: true,
        jwt: None,
        google: None,
        gcs: None,
        roles: Default::default(),
        mail: None,
        auth_public_base_url: None,
        webhooks: None,
        before_hooks: executor,
        sockets: None,
    };
    assert!(
        brain_backend::http::router(pool.clone(), config(None))
            .await
            .is_err(),
        "Serving a before-hook without an executor must fail closed"
    );
    let router = brain_backend::http::router(
        pool.clone(),
        config(Some(hooks.clone() as brain_backend::before_hooks::Executor)),
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .await
        .unwrap()
    });
    let mut server = Server {
        url,
        client: reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        task,
        stop: Some(stop),
        files,
    };
    let auth = Auth::new(pool.clone()).await.unwrap();
    let credentials = || Credentials {
        email: format!("hook-{}@example.test", Uuid::new_v4().simple()),
        password: "synthetic-before-hook-password".into(),
    };
    let account = credentials();
    auth.signup(Credentials {
        email: account.email.clone(),
        password: account.password.clone(),
    })
    .await
    .unwrap();
    let (_, token) = auth.login(account).await.unwrap().unwrap();
    let cookie = format!("{}={token}", auth::COOKIE);
    let stranger = credentials();
    auth.signup(Credentials {
        email: stranger.email.clone(),
        password: stranger.password.clone(),
    })
    .await
    .unwrap();
    let (_, token) = auth.login(stranger).await.unwrap().unwrap();
    let stranger_cookie = format!("{}={token}", auth::COOKIE);
    let (status, _, created) = server
        .post("/api/hook_items", json!({"title":"raw"}), Some(&cookie))
        .await;
    assert_eq!(status, 200);
    assert_eq!(created["data"]["title"], "normalized");
    let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let search: (String, String) = sqlx::query_as(
        "SELECT search_text,search_vector::text FROM public._brain_res_hook_items WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        search,
        (String::new(), String::new()),
        "An empty search contract must store and index no resource values"
    );

    let path = format!("/api/hook_items/{id}");
    let call_count = hooks.calls.load(Ordering::Relaxed);
    assert_eq!(
        server
            .call(
                reqwest::Method::PUT,
                &path,
                Some(json!({"title":"ignored"})),
                Some(&stranger_cookie),
                "https://app.example.test"
            )
            .await
            .0,
        404
    );
    assert_eq!(hooks.calls.load(Ordering::Relaxed), call_count);
    assert_eq!(
        server
            .post("/api/hook_items", json!([]), Some(&cookie))
            .await
            .0,
        422
    );
    assert_eq!(hooks.calls.load(Ordering::Relaxed), call_count);
    let (status, _, denied) = server
        .post("/api/hook_items", json!({"title":"deny"}), Some(&cookie))
        .await;
    assert_eq!(status, 422);
    assert_eq!(denied["error"]["fields"].as_object().unwrap().len(), 1);
    assert!(denied["error"]["fields"].get("title").is_some());
    assert_eq!(
        server
            .call(
                reqwest::Method::PUT,
                &path,
                Some(json!({"title":"invalid"})),
                Some(&cookie),
                "https://app.example.test"
            )
            .await
            .0,
        422
    );
    let pause = server.call(
        reqwest::Method::PUT,
        &path,
        Some(json!({"title":"pause"})),
        Some(&cookie),
        "https://app.example.test",
    );
    let race = async {
        tokio::time::timeout(Duration::from_secs(1), hooks.entered.notified())
            .await
            .unwrap();
        let winner = server
            .call(
                reqwest::Method::PUT,
                &path,
                Some(json!({"title":"winner"})),
                Some(&cookie),
                "https://app.example.test",
            )
            .await;
        hooks.release.notify_one();
        assert_eq!(winner.0, 200);
    };
    let (paused, ()) = tokio::join!(pause, race);
    assert_eq!(paused.0, 409);
    assert_eq!(
        db::get(&pool, &resource, id).await.unwrap().unwrap()["data"]["title"],
        "winner"
    );
    assert_eq!(
        server
            .call(
                reqwest::Method::PUT,
                &path,
                Some(json!({"title":"timeout"})),
                Some(&cookie),
                "https://app.example.test"
            )
            .await
            .0,
        503
    );
    hooks.deny_delete.store(true, Ordering::Relaxed);
    assert_eq!(
        server
            .call(
                reqwest::Method::DELETE,
                &path,
                None,
                Some(&cookie),
                "https://app.example.test"
            )
            .await
            .0,
        422
    );
    assert!(db::get(&pool, &resource, id).await.unwrap().is_some());
    hooks.deny_delete.store(false, Ordering::Relaxed);
    assert_eq!(
        server
            .call(
                reqwest::Method::DELETE,
                &path,
                None,
                Some(&cookie),
                "https://app.example.test"
            )
            .await
            .0,
        200
    );
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public._brain_jobs WHERE queue='item_changed'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        events, 3,
        "Only successful create/update/delete commit after-hooks"
    );
    if let Some(stop) = server.stop.take() {
        let _ = stop.send(());
    }
    tokio::time::timeout(Duration::from_secs(2), &mut server.task)
        .await
        .unwrap()
        .unwrap();
    pool.close().await;
}
