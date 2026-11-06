//! Cookie-independent Sign in with Apple for explicitly configured native app audiences.
//! Web Services IDs and notification App IDs grant no native login authority.
use crate::{
    auth::{Auth, User},
    oidc::{Identity, KeySet},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone)]
pub struct Config {
    app_ids: BTreeSet<String>,
    #[cfg(test)]
    synthetic_keys: Option<KeySet>,
}
impl Config {
    pub fn new(app_ids: Vec<String>) -> Result<Self, String> {
        let unique: BTreeSet<_> = app_ids.iter().cloned().collect();
        if app_ids.is_empty()
            || app_ids.len() > 16
            || unique.len() != app_ids.len()
            || app_ids.iter().any(|id| !crate::apple::bundle_id(id))
        {
            return Err("APPLE_NATIVE_APP_IDS must list 1..16 distinct native bundle IDs (for example com.example.app)".into());
        }
        Ok(Self {
            app_ids: unique,
            #[cfg(test)]
            synthetic_keys: None,
        })
    }
    /// An explicitly configured comma-separated list. No native audience is inferred from web or notification IDs.
    pub fn from_setting(value: &str) -> Result<Self, String> {
        if value.len() > 4096 {
            return Err("APPLE_NATIVE_APP_IDS exceeds its 4096-byte configuration limit".into());
        }
        Self::new(value.split(',').map(|id| id.trim().to_owned()).collect())
    }
    pub fn allows(&self, app_id: &str) -> bool {
        self.app_ids.contains(app_id)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeRequest {
    /// Selects one configured native app, never an arbitrary audience.
    pub client_id: String,
}
#[derive(Serialize)]
pub struct Challenge {
    pub client_id: String,
    /// Send this value as the native Apple authorization request's nonce, unchanged.
    pub nonce: String,
    pub expires_in: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub id_token: String,
    pub nonce: String,
    /// Unsigned first-authorization display hint only, normalized exactly as in web sign-in.
    #[serde(default)]
    pub name: Option<String>,
}
#[derive(Clone)]
pub struct AppleNative {
    config: Config,
    keys: KeySet,
}
impl AppleNative {
    pub fn new(config: Config) -> Result<Self, String> {
        let keys = KeySet::new(crate::apple::JWKS_URL, "Apple")?;
        #[cfg(test)]
        let keys = config.synthetic_keys.clone().unwrap_or(keys);
        Ok(Self { config, keys })
    }
    pub async fn challenge(&self, auth: &Auth, app_id: &str) -> Result<Option<Challenge>, String> {
        if !self.config.allows(app_id) {
            return Ok(None);
        }
        let nonce = auth.apple_native_challenge(app_id).await?;
        Ok(Some(Challenge {
            client_id: app_id.to_owned(),
            nonce,
            expires_in: 300,
        }))
    }
    async fn verify(
        &self,
        id_token: &str,
        nonce: &str,
        audience: &str,
    ) -> Result<Option<Identity>, String> {
        if !self.config.allows(audience)
            || id_token.len() > 16384
            || !crate::jwt::valid_opaque(nonce)
        {
            return Ok(None);
        }
        self.keys
            .verify(id_token, |key| {
                crate::apple::Apple::verify_identity_key(id_token, nonce, key, audience)
            })
            .await
    }
    /// The database selects the audience before verification; the request and token cannot widen the allowlist.
    /// The identity/session transaction consumes this exact realm/audience/nonce once, including concurrent calls.
    pub async fn login(
        &self,
        auth: &Auth,
        credential: &Credential,
    ) -> Result<Option<(User, String)>, String> {
        let Some(audience) = auth.apple_native_audience(&credential.nonce).await? else {
            return Ok(None);
        };
        let Some(identity) = self
            .verify(&credential.id_token, &credential.nonce, &audience)
            .await?
        else {
            return Ok(None);
        };
        auth.apple_native_login(
            &identity.with_name(credential.name.as_deref()),
            &credential.nonce,
            &audience,
        )
        .await
    }
}
/// Native callers receive the existing rotating refresh credential in the JSON body, never ambient cookies.
/// Browser serialization deliberately continues to omit this field elsewhere.
pub fn token_body(tokens: crate::jwt::Tokens) -> serde_json::Value {
    serde_json::json!({
        "user": tokens.user, "access_token": tokens.access_token, "token_type": tokens.token_type,
        "expires_in": tokens.expires_in, "refresh_token": tokens.refresh_token,
        "refresh_expires_in": tokens.refresh_age
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde_json::{Value, json};
    const APP: &str = "com.example.app";
    const OTHER_APP: &str = "com.example.other";
    const WEB: &str = "com.example.web";
    async fn native() -> (AppleNative, EncodingKey, Header) {
        let native =
            AppleNative::new(Config::new(vec![APP.into(), OTHER_APP.into()]).unwrap()).unwrap();
        let (encoding, set) = crate::oidc::synthetic::keys("synthetic-native-apple-key");
        native.keys.preload(set).await;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("synthetic-native-apple-key".into());
        (native, encoding, header)
    }
    fn claims(nonce: &str) -> Value {
        let now = crate::jwt::now();
        json!({
            "iss": "https://appleid.apple.com",
            "aud": APP,
            "sub": "synthetic-native-apple-subject",
            "iat": now,
            "exp": now + 600,
            "nonce": nonce,
            "email": "native@example.test",
            "email_verified": true
        })
    }
    #[test]
    fn native_audiences_are_explicit_bounded_and_never_inferred() {
        let config = Config::from_setting("com.example.app, com.example.other").unwrap();
        assert!(config.allows(APP));
        assert!(!config.allows(WEB));
        assert!(!config.allows("com.example.notification"));
        for value in [
            "",
            "com.example.app,",
            "com.example.app,com.example.app",
            "app",
            "com..app",
            "com.example.app\ninvalid",
            "https://example.com",
        ] {
            assert!(Config::from_setting(value).is_err(), "{value:?}");
        }
        assert!(Config::new(vec![]).is_err());
        assert!(Config::new((0..17).map(|i| format!("com.example.app{i}")).collect()).is_err());
        assert!(Config::new(vec![format!("com.example.{}", "x".repeat(255))]).is_err());
        assert!(
            serde_json::from_value::<ChallengeRequest>(
                json!({"client_id":APP,"audience":OTHER_APP})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<Credential>(
                json!({"id_token":"token","nonce":"nonce","client_id":APP})
            )
            .is_err()
        );
    }
    #[tokio::test]
    async fn synthetic_native_tokens_keep_signature_issuer_expiry_nonce_and_exact_bound_audience() {
        let (native, encoding, header) = native().await;
        let nonce = crate::jwt::opaque();
        let valid = claims(&nonce);
        let token = encode(&header, &valid, &encoding).unwrap();
        assert!(native.verify(&token, &nonce, APP).await.unwrap().is_some());
        assert!(
            native
                .verify(&token, &nonce, OTHER_APP)
                .await
                .unwrap()
                .is_none(),
            "another allowed app still cannot consume this audience"
        );
        assert!(native.verify(&token, &nonce, WEB).await.unwrap().is_none());
        assert!(
            native
                .verify(&token, &crate::jwt::opaque(), APP)
                .await
                .unwrap()
                .is_none()
        );
        let now = crate::jwt::now();
        for (key, value) in [
            ("aud", json!(WEB)),
            ("aud", json!("com.example.unconfigured")),
            ("aud", json!(OTHER_APP)),
            ("aud", json!([APP, OTHER_APP])),
            ("iss", json!("https://evil.example")),
            ("exp", json!(now - 1)),
            ("iat", json!(now + 120)),
            ("nonce", Value::Null),
            ("sub", json!("")),
        ] {
            let mut bad = valid.clone();
            bad[key] = value;
            assert!(
                native
                    .verify(&encode(&header, &bad, &encoding).unwrap(), &nonce, APP)
                    .await
                    .unwrap()
                    .is_none(),
                "{key}"
            );
        }
        let mut tampered = token.clone();
        let at = tampered.rfind('.').unwrap() + 1;
        tampered.replace_range(
            at..at + 1,
            if &tampered[at..at + 1] == "A" {
                "B"
            } else {
                "A"
            },
        );
        assert!(
            native
                .verify(&tampered, &nonce, APP)
                .await
                .unwrap()
                .is_none()
        );
        let wrong = encode(
            &Header::new(Algorithm::HS256),
            &valid,
            &EncodingKey::from_secret(b"synthetic-non-apple-key"),
        )
        .unwrap();
        assert!(native.verify(&wrong, &nonce, APP).await.unwrap().is_none());
        let mut injected = header.clone();
        injected.jku = Some("https://evil.example/keys".into());
        assert!(
            native
                .verify(&encode(&injected, &valid, &encoding).unwrap(), &nonce, APP)
                .await
                .unwrap()
                .is_none()
        );
        let mut missing_key = header;
        missing_key.kid = Some("unknown-native-key".into());
        assert!(
            native
                .verify(
                    &encode(&missing_key, &valid, &encoding).unwrap(),
                    &nonce,
                    APP
                )
                .await
                .unwrap()
                .is_none()
        );
    }
    #[tokio::test]
    #[ignore = "Use the disposable backend Postgres harness"]
    async fn native_nonce_audience_realm_replay_and_sessions_are_database_bound() {
        let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only");
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool, &[]).await.unwrap();
        let auth = Auth::new(pool.clone()).await.unwrap();
        let (native, key, header) = native().await;
        assert!(
            native
                .challenge(&auth, "com.example.unconfigured")
                .await
                .unwrap()
                .is_none()
        );
        assert!(native.challenge(&auth, WEB).await.unwrap().is_none());
        let nonce = native.challenge(&auth, APP).await.unwrap().unwrap().nonce;
        let stored: (String, Option<String>) = sqlx::query_as(
            "SELECT purpose,audience FROM _brain_auth_challenges WHERE token_hash=$1",
        )
        .bind(crate::auth::digest(&nonce))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored, ("apple_native_nonce".into(), Some(APP.into())));
        let mut valid = claims(&nonce);
        valid["sub"] = json!(format!("native-{}", uuid::Uuid::new_v4().simple()));
        valid["email"] = json!(format!(
            "native-{}@example.test",
            uuid::Uuid::new_v4().simple()
        ));
        let mut wrong = valid.clone();
        wrong["aud"] = json!(OTHER_APP);
        assert!(
            native
                .login(
                    &auth,
                    &Credential {
                        id_token: encode(&header, &wrong, &key).unwrap(),
                        nonce: nonce.clone(),
                        name: None
                    }
                )
                .await
                .unwrap()
                .is_none()
        );
        let credential = Credential {
            id_token: encode(&header, &valid, &key).unwrap(),
            nonce: nonce.clone(),
            name: None,
        };
        let (first, second) = tokio::join!(
            native.login(&auth, &credential),
            native.login(&auth, &credential)
        );
        let mut accepted = [first.unwrap(), second.unwrap()].into_iter().flatten();
        let (user, opaque) = accepted.next().expect("one concurrent login succeeds");
        assert!(accepted.next().is_none());
        assert!(auth.user(Some(&opaque)).await.unwrap().is_some());
        assert!(native.login(&auth, &credential).await.unwrap().is_none());
        // An ordinary web nonce and a native nonce are never interchangeable, even with a valid Apple identity.
        let web_nonce = auth
            .identity_challenge(crate::auth::Provider::Apple)
            .await
            .unwrap();
        let mut web_claims = valid.clone();
        web_claims["nonce"] = json!(&web_nonce);
        assert!(
            native
                .login(
                    &auth,
                    &Credential {
                        id_token: encode(&header, &web_claims, &key).unwrap(),
                        nonce: web_nonce,
                        name: None
                    }
                )
                .await
                .unwrap()
                .is_none()
        );
        let native_nonce = native.challenge(&auth, APP).await.unwrap().unwrap().nonce;
        let identity = native
            .verify(&credential.id_token, &nonce, APP)
            .await
            .unwrap()
            .unwrap();
        assert!(
            auth.identity_login(crate::auth::Provider::Apple, &identity, &native_nonce)
                .await
                .unwrap()
                .is_none()
        );
        let expired = native.challenge(&auth, APP).await.unwrap().unwrap().nonce;
        sqlx::query("UPDATE _brain_auth_challenges SET expires_at=now()-interval '1 second' WHERE token_hash=$1").bind(crate::auth::digest(&expired)).execute(&pool).await.unwrap();
        assert!(
            auth.apple_native_audience(&expired)
                .await
                .unwrap()
                .is_none()
        );
        for assignment in [
            "provider=NULL",
            "audience=NULL",
            "audience='not-a-bundle-id'",
            "purpose='apple_nonce'",
        ] {
            assert!(
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "UPDATE _brain_auth_challenges SET {assignment} WHERE token_hash=$1"
                )))
                .bind(crate::auth::digest(&nonce))
                .execute(&pool)
                .await
                .is_err(),
                "the database must reject {assignment}"
            );
        }
        let jwt = crate::jwt::Jwt::new(
            pool.clone(),
            crate::jwt::Config::new(
                "synthetic-native-session-secret-32-bytes".into(),
                "synthetic-issuer".into(),
                "synthetic-audience".into(),
            )
            .unwrap(),
        );
        let tokens = jwt.issue(user.clone(), &opaque).await.unwrap();
        assert!(
            jwt.user(Some(&tokens.access_token))
                .await
                .unwrap()
                .is_some()
        );
        assert!(tokens.refresh_token.len() >= 64);
        let rotated = jwt
            .refresh(Some(&tokens.refresh_token))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(rotated.refresh_token, tokens.refresh_token);
        assert!(
            jwt.user(Some(&rotated.access_token))
                .await
                .unwrap()
                .is_some()
        );
        crate::auth_flows::revoke_user_sessions(&pool, user.id)
            .await
            .unwrap();
        assert!(
            jwt.user(Some(&tokens.access_token))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            jwt.refresh(Some(&tokens.refresh_token))
                .await
                .unwrap()
                .is_none()
        );
        pool.close().await;
    }
    #[tokio::test]
    #[ignore = "Use the disposable backend Postgres harness"]
    async fn native_http_has_no_cookie_authority_or_cors_bypass_and_keeps_bearer_rotation() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only");
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool, &[]).await.unwrap();
        let roles = crate::auth_flows::RolesConfig::default();
        roles.register(&pool).await.unwrap();
        let (native, key, header) = native().await;
        let mut configured = native.config.clone();
        configured.synthetic_keys = Some(native.keys.clone());
        struct OwnedFiles(std::path::PathBuf);
        impl Drop for OwnedFiles {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let files = OwnedFiles(std::env::temp_dir().join(format!(
            "brain-native-apple-{}",
            uuid::Uuid::new_v4().simple()
        )));
        let config = |jwt: bool| crate::http::Config {
            resources: vec![],
            jobs: vec![],
            uploads: files.0.clone(),
            origins: vec!["https://example.test".into()],
            production: true,
            jwt: jwt.then(|| {
                crate::jwt::Config::new(
                    "synthetic-native-http-session-secret-32-bytes".into(),
                    "native-http-issuer".into(),
                    "native-http-audience".into(),
                )
                .unwrap()
            }),
            google: None,
            apple: None,
            apple_native: Some(configured.clone()),
            linkedin: None,
            gcs: None,
            roles: roles.clone(),
            mail: None,
            auth_public_base_url: None,
            webhooks: None,
            before_hooks: None,
            sockets: None,
            api_keys: None,
        };
        assert!(
            crate::http::router(pool.clone(), config(false))
                .await
                .err()
                .unwrap()
                .contains("JWT")
        );
        let app = crate::http::router(pool.clone(), config(true))
            .await
            .unwrap();
        let request = |path: &str, body: Value, headers: Vec<(&str, &str)>| {
            let mut req = Request::builder()
                .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                    [127, 0, 0, 1],
                    43000,
                ))))
                .method("POST")
                .uri(path)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json");
            for (name, value) in headers {
                req = req.header(name, value);
            }
            req.body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap()
        };
        let challenge_path = "/_brain/auth/apple/native/challenge";
        for (path, body) in [
            (challenge_path, json!({"client_id":APP})),
            (
                "/_brain/auth/apple/native/token",
                json!({"id_token":"invalid","nonce":crate::jwt::opaque()}),
            ),
            (
                "/_brain/auth/apple/native/refresh",
                json!({"refresh_token":crate::jwt::opaque()}),
            ),
        ] {
            for headers in [
                vec![("Cookie", "__Host-brain-apple=synthetic-ambient-cookie")],
                vec![("Origin", "https://example.test")],
                vec![("Origin", "https://evil.example")],
            ] {
                let response = app
                    .clone()
                    .oneshot(request(path, body.clone(), headers))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
                assert!(!response.headers().contains_key("Set-Cookie"));
            }
        }
        for app_id in [WEB, "com.example.unconfigured"] {
            let response = app
                .clone()
                .oneshot(request(challenge_path, json!({"client_id":app_id}), vec![]))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let response = app
            .clone()
            .oneshot(request(challenge_path, json!({"client_id":APP}), vec![]))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key("Set-Cookie"));
        assert!(
            !response
                .headers()
                .contains_key("Access-Control-Allow-Origin")
        );
        assert_eq!(response.headers()["Cache-Control"], "no-store");
        let challenge: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(challenge["client_id"], APP);
        let nonce = challenge["nonce"].as_str().unwrap();
        let mut valid = claims(nonce);
        valid["sub"] = json!(format!("native-http-{}", uuid::Uuid::new_v4().simple()));
        valid["email"] = json!(format!(
            "native-http-{}@example.test",
            uuid::Uuid::new_v4().simple()
        ));
        let mut wrong = valid.clone();
        wrong["aud"] = json!(OTHER_APP);
        let token_path = "/_brain/auth/apple/native/token";
        let response = app
            .clone()
            .oneshot(request(
                token_path,
                json!({"id_token":encode(&header,&wrong,&key).unwrap(),"nonce":nonce}),
                vec![],
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let credential = json!({"id_token":encode(&header,&valid,&key).unwrap(),"nonce":nonce,"name":"Synthetic Native User"});
        let response = app
            .clone()
            .oneshot(request(token_path, credential.clone(), vec![]))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key("Set-Cookie"));
        let tokens: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(tokens["token_type"], "Bearer");
        assert!(tokens["access_token"].as_str().unwrap().len() > 64);
        assert!(crate::jwt::valid_opaque(
            tokens["refresh_token"].as_str().unwrap()
        ));
        let response = app
            .clone()
            .oneshot(request(token_path, credential, vec![]))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let refresh_path = "/_brain/auth/apple/native/refresh";
        let response = app
            .clone()
            .oneshot(request(
                refresh_path,
                json!({"refresh_token":tokens["refresh_token"]}),
                vec![],
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key("Set-Cookie"));
        let rotated: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_ne!(rotated["refresh_token"], tokens["refresh_token"]);
        let me = Request::builder()
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                43000,
            ))))
            .uri("/_brain/auth/me")
            .header(
                "Authorization",
                format!("Bearer {}", rotated["access_token"].as_str().unwrap()),
            )
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(me).await.unwrap().status(),
            StatusCode::OK
        );
        let id = uuid::Uuid::parse_str(tokens["user"]["id"].as_str().unwrap()).unwrap();
        crate::auth_flows::revoke_user_sessions(&pool, id)
            .await
            .unwrap();
        let response = app
            .clone()
            .oneshot(request(
                refresh_path,
                json!({"refresh_token":rotated["refresh_token"]}),
                vec![],
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        pool.close().await;
    }
}
