//! Storage integration uses only an explicitly selected disposable Postgres cluster and owned temporary files.
use brain_backend::{
    auth::{Auth, Credentials},
    media::{self, Preset, Store},
};
use image::{DynamicImage, ImageFormat};
use std::{io::Cursor, path::PathBuf};
use uuid::Uuid;
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn originals_versions_library_metadata_owner_isolation_and_durable_cleanup_work_together() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL")
        .expect("Disposable Postgres only; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    brain_backend::db::migrate(&pool, &[]).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let user = auth
        .signup(Credentials {
            email: format!("media-{}@example.test", Uuid::new_v4().simple()),
            password: "synthetic-media-account-password".into(),
        })
        .await
        .unwrap();
    let root = Scratch(std::env::temp_dir().join(format!("brain-media-db-{}", Uuid::new_v4())));
    let store = Store::new(root.0.clone(), None).unwrap();
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(640, 360)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let original = encoded.into_inner();
    let created = media::insert(
        &store,
        &pool,
        user.id,
        media::prepare(original.clone(), &media::defaults()).unwrap(),
        "Example image",
        "Example caption",
    )
    .await
    .unwrap();
    let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let source = media::asset(&pool, id, 1, "original", user.id, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.read(&source).await.unwrap(), original);
    let first = media::asset(&pool, id, 1, "full", user.id, false)
        .await
        .unwrap()
        .unwrap();
    let first_bytes = store.read(&first).await.unwrap();
    // Exercise the actual private HTTP transport, including HEAD and API negotiation.
    use axum::{
        body::{Body, to_bytes},
        extract::ConnectInfo,
        http::Request,
    };
    use tower::ServiceExt;
    let (_, session) = auth
        .login(Credentials {
            email: user.email.clone(),
            password: "synthetic-media-account-password".into(),
        })
        .await
        .unwrap()
        .unwrap();
    let http = brain_backend::http::router(
        pool.clone(),
        brain_backend::Config { api_keys: None,
            roles: Default::default(),
            mail: None,
            auth_public_base_url: None,
            webhooks: None,before_hooks:None,sockets:None,
            resources: vec![],
            jobs: vec![],
            uploads: root.0.clone(),
            origins: vec!["https://media.example.test".into()],
            production: true,
            jwt: None,
            google: None,
            gcs: None,
        },
    )
    .await
    .unwrap();
    let request = |method: &str, path: String, authenticated: bool| {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .extension(ConnectInfo(
                "127.0.0.1:42000".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .header("accept", "text/html");
        if authenticated {
            request = request.header("cookie", format!("__Host-brain={session}"));
        }
        request.body(Body::empty()).unwrap()
    };
    let asset_path = format!("/_brain/media/{id}/v/1/full");
    let response = http
        .clone()
        .oneshot(request("GET", asset_path.clone(), true))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        to_bytes(response.into_body(), 20 * 1024 * 1024)
            .await
            .unwrap()
            .as_ref(),
        first_bytes.as_slice()
    );
    let response = http
        .clone()
        .oneshot(request("HEAD", asset_path.clone(), true))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .is_empty()
    );
    let response = http
        .clone()
        .oneshot(request("GET", asset_path, false))
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(response.headers()["content-type"], "application/json");
    let response = http
        .oneshot(request(
            "GET",
            format!("/_brain/media/{id}/v/1/original"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-disposition"], "attachment");
    assert_eq!(
        to_bytes(response.into_body(), 20 * 1024 * 1024)
            .await
            .unwrap()
            .as_ref(),
        original.as_slice()
    );
    let listing = media::library(&pool, user.id, 24, 0).await.unwrap();
    assert_eq!(listing["items"].as_array().unwrap().len(), 1);
    assert_eq!(listing["items"][0]["alt"], "Example image");
    assert!(
        listing["items"][0]["srcset"]["webp"]
            .as_str()
            .unwrap()
            .contains("320w")
    );
    assert!(!listing.to_string().contains("object_key"));
    assert!(
        !listing
            .to_string()
            .contains(&root.0.to_string_lossy().to_string())
    );
    assert!(
        media::asset(&pool, id, 1, "full", Uuid::new_v4(), false)
            .await
            .unwrap()
            .is_none()
    );
    let rebuilt = media::build_variants(
        &store,
        &pool,
        id,
        user.id,
        vec![Preset {
            name: "square".into(),
            width: 160,
            height: 160,
            crop: true,
            format: "jpeg".into(),
            quality: Some(80),
        }],
    )
    .await
    .unwrap();
    assert_eq!(rebuilt["version"], 2);
    assert_eq!(
        store
            .read(
                &media::asset(&pool, id, 1, "full", user.id, false)
                    .await
                    .unwrap()
                    .unwrap()
            )
            .await
            .unwrap(),
        first_bytes
    );
    let square = media::asset(&pool, id, 2, "square", user.id, false)
        .await
        .unwrap()
        .unwrap();
    let decoded = image::load_from_memory(&store.read(&square).await.unwrap()).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (160, 160));
    assert!(
        media::update_metadata(&pool, id, user.id, false, "Updated alt", "Updated caption")
            .await
            .unwrap()
    );
    assert_eq!(
        media::library(&pool, user.id, 1, 0).await.unwrap()["items"][0]["alt"],
        "Updated alt"
    );
    assert!(
        !media::delete(&pool, id, Uuid::new_v4(), false)
            .await
            .unwrap()
    );
    std::fs::write(root.0.join("unrelated.txt"), "Keep this unrelated file").unwrap();
    assert!(media::delete(&pool, id, user.id, false).await.unwrap());
    for _ in 0..8 {
        let result = media::cleanup(&pool, &store, 8).await.unwrap();
        assert_eq!(result["failed"], 0);
        if result["completed"] == 0 {
            break;
        }
    }
    assert!(store.read(&source).await.is_err());
    assert!(store.read(&first).await.is_err());
    assert!(store.read(&square).await.is_err());
    assert!(root.0.join("unrelated.txt").exists());
    assert!(media::library(&pool, user.id, 0, 0).await.is_err());
    assert!(media::cleanup(&pool, &store, 9).await.is_err());
    sqlx::query("DELETE FROM _brain_users WHERE id=$1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
