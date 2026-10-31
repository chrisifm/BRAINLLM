//! Declared file buckets over real HTTP routing against the disposable Postgres cluster.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, Response},
};
use brain_backend::{
    Resource,
    auth::{Auth, Credentials},
    files::Bucket,
    platform::Platform,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "https://files.example.test";
const BOUNDARY: &str = "brainfilesboundary";
struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn multipart(name: &str, filename: &str, kind: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {kind}\r\n\r\n").into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}
struct Client {
    http: Router,
}
impl Client {
    async fn send(&self, method: &str, path: &str, session: Option<&str>, headers: &[(&str, &str)], body: Vec<u8>) -> Response<Body> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .extension(ConnectInfo("127.0.0.1:42001".parse::<std::net::SocketAddr>().unwrap()))
            .header("origin", ORIGIN);
        if let Some(session) = session {
            request = request.header("cookie", format!("__Host-brain={session}"));
        }
        for (k, v) in headers {
            request = request.header(*k, *v);
        }
        self.http.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap()
    }
    async fn upload(&self, path: &str, session: &str, filename: &str, kind: &str, bytes: &[u8]) -> Response<Body> {
        let ct = format!("multipart/form-data; boundary={BOUNDARY}");
        self.send("POST", path, Some(session), &[("content-type", &ct)], multipart("file", filename, kind, bytes)).await
    }
}
async fn body(response: Response<Body>) -> Vec<u8> {
    to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap().to_vec()
}
async fn json_body(response: Response<Body>) -> Value {
    serde_json::from_slice(&body(response).await).unwrap()
}

#[tokio::test]
#[ignore = "Use scripts/backend-check.mjs against its disposable Postgres cluster"]
async fn declared_buckets_stream_any_type_enforce_policy_and_serve_ranges() {
    let url = std::env::var("BRAIN_BACKEND_TEST_URL").expect("Disposable Postgres only; no developer database is selected");
    let pool = brain_backend::db::connect(&url).await.unwrap();
    let suffix = Uuid::new_v4().simple().to_string()[..8].to_string();
    let orders: Resource = serde_json::from_value(json!({
        "name": format!("orders_{suffix}"),
        "fields": {"title": {"kind": "str", "required": true}, "invoice": {"kind": "str"}},
        "policy": {"read": ["owner", "company"], "create": ["candidate"], "update": ["owner"], "delete": ["owner"]}
    }))
    .unwrap();
    brain_backend::db::migrate(&pool, std::slice::from_ref(&orders)).await.unwrap();
    let auth = Auth::new(pool.clone()).await.unwrap();
    let mut sessions = Vec::new();
    for role in ["candidate", "company", "candidate"] {
        let email = format!("files-{}@example.test", Uuid::new_v4().simple());
        let password = "synthetic-files-account-password";
        let user = auth.signup(Credentials { email: email.clone(), password: password.into() }).await.unwrap();
        sqlx::query("UPDATE _brain_users SET role=$1 WHERE id=$2").bind(role).bind(user.id).execute(&pool).await.unwrap();
        let (_, session) = auth.login(Credentials { email, password: password.into() }).await.unwrap().unwrap();
        sessions.push((user, session));
    }
    let (owner, owner_session) = &sessions[0];
    let (_, company_session) = &sessions[1];
    let (_, stranger_session) = &sessions[2];
    let order = brain_backend::db::create(&pool, &orders, owner.id, json!({"title": "Desk lamp"})).await.unwrap();
    let order_id = order["id"].as_str().unwrap().to_string();

    let invoices = Bucket::from_contract("invoices", json!({
        "resource": orders.name, "field": "invoice", "types": ["application/pdf"], "maxBytes": "1kb",
        "create": ["candidate"], "read": ["owner", "related", "admin"]
    }))
    .unwrap();
    let attachments = Bucket::from_contract("attachments", json!({"create": ["candidate"], "maxBytes": 4096})).unwrap();
    let banners = Bucket::from_contract("banners", json!({"visibility": "public", "create": ["candidate"]})).unwrap();
    let root = Scratch(std::env::temp_dir().join(format!("brain-files-http-{}", Uuid::new_v4())));
    let (http, _) = brain_backend::http::router_with_platform(
        pool.clone(),
        brain_backend::Config {
            roles: Default::default(),
            mail: None,
            auth_public_base_url: None,
            webhooks: None,
            before_hooks: None,
            sockets: None,
            resources: vec![orders.clone()],
            jobs: vec![],
            uploads: root.0.clone(),
            origins: vec![ORIGIN.into()],
            production: true,
            jwt: None,
            google: None,
            gcs: None,
        },
        Platform { buckets: vec![invoices, attachments, banners], ..Default::default() },
    )
    .await
    .unwrap();
    let client = Client { http };
    let pdf = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n1 0 obj << /Type /Catalog >> endobj\ntrailer\n%%EOF\n".to_vec();
    let invoice_path = format!("/_brain/files/invoices?record={order_id}");

    // A linked bucket needs the record, and update rights on it.
    let response = client.upload("/_brain/files/invoices", owner_session, "invoice.pdf", "application/pdf", &pdf).await;
    assert_eq!(response.status(), 422);
    let response = client.upload(&invoice_path, stranger_session, "invoice.pdf", "application/pdf", &pdf).await;
    assert_eq!(response.status(), 403, "only a user who may update the order attaches files to it");
    // Content decides the type: PNG bytes declared as PDF are rejected by a PDF-only bucket.
    let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    let response = client.upload(&invoice_path, owner_session, "fake.pdf", "application/pdf", &png).await;
    assert_eq!(response.status(), 415);
    assert_eq!(json_body(response).await["error"]["code"], "FILE_TYPE_DENIED");
    let response = client.upload(&invoice_path, owner_session, "big.pdf", "application/pdf", &vec![b'%'; 2048]).await;
    assert_eq!(response.status(), 413);
    assert_eq!(json_body(response).await["error"]["code"], "FILE_TOO_LARGE");
    let response = client.upload(&invoice_path, owner_session, "../Invoice 7 €.pdf", "application/octet-stream", &pdf).await;
    assert_eq!(response.status(), 201);
    let location = response.headers()["location"].to_str().unwrap().to_string();
    let created = json_body(response).await;
    assert_eq!(created["contentType"], "application/pdf");
    assert_eq!(created["sniffedType"], "application/pdf");
    assert_eq!(created["declaredType"], "application/octet-stream");
    assert_eq!(created["filename"], "Invoice 7 €.pdf");
    assert_eq!(created["bytes"], pdf.len());
    assert_eq!(created["record"], order_id);
    assert_eq!(created["field"], "invoice");
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(location, format!("/_brain/files/invoices/{id}"));
    assert_eq!(std::fs::read_dir(root.0.join("files/.incoming")).unwrap().count(), 0);

    // Owner and a user who may read the related order download; others do not.
    let response = client.send("GET", &location, Some(owner_session), &[], vec![]).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/pdf");
    assert_eq!(response.headers()["accept-ranges"], "bytes");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(response.headers()["content-disposition"].to_str().unwrap().starts_with("attachment;"));
    let etag = response.headers()["etag"].to_str().unwrap().to_string();
    assert_eq!(body(response).await, pdf);
    assert_eq!(client.send("GET", &location, Some(company_session), &[], vec![]).await.status(), 200);
    assert_eq!(client.send("GET", &location, Some(stranger_session), &[], vec![]).await.status(), 403);
    assert_eq!(client.send("GET", &location, None, &[], vec![]).await.status(), 401);

    // Conditional and partial requests.
    let response = client.send("GET", &location, Some(owner_session), &[("if-none-match", &etag)], vec![]).await;
    assert_eq!(response.status(), 304);
    let response = client.send("GET", &location, Some(owner_session), &[("range", "bytes=0-3")], vec![]).await;
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-range"], format!("bytes 0-3/{}", pdf.len()));
    assert_eq!(body(response).await, b"%PDF");
    let response = client.send("GET", &location, Some(owner_session), &[("range", "bytes=-5")], vec![]).await;
    assert_eq!(body(response).await, &pdf[pdf.len() - 5..]);
    let response = client.send("GET", &location, Some(owner_session), &[("range", "bytes=0-1"), ("if-range", "\"stale\"")], vec![]).await;
    assert_eq!(response.status(), 200, "a stale If-Range validator returns the whole file");
    let response = client.send("GET", &location, Some(owner_session), &[("range", "bytes=99999-")], vec![]).await;
    assert_eq!(response.status(), 416);
    assert_eq!(response.headers()["content-range"], format!("bytes */{}", pdf.len()));
    let response = client.send("GET", &format!("{location}?inline=true"), Some(owner_session), &[], vec![]).await;
    assert!(response.headers()["content-disposition"].to_str().unwrap().starts_with("inline;"));

    // Listing: the owner sees their file; a stranger sees none; a related reader lists by record.
    let listed = json_body(client.send("GET", "/_brain/files/invoices", Some(owner_session), &[], vec![]).await).await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    let listed = json_body(client.send("GET", "/_brain/files/invoices", Some(stranger_session), &[], vec![]).await).await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 0);
    let listed = json_body(client.send("GET", &format!("/_brain/files/invoices?record={order_id}"), Some(company_session), &[], vec![]).await).await;
    assert_eq!(listed["items"][0]["id"], id);
    let meta = json_body(client.send("GET", &format!("{location}/meta"), Some(owner_session), &[], vec![]).await).await;
    assert_eq!(meta["sha256"].as_str().unwrap().len(), 64);

    // Any type by default: a zip archive and arbitrary binary are accepted and sniffed or recorded as unknown.
    let zip = b"PK\x03\x04\x14\x00\x00\x00\x08\x00rest-of-archive".to_vec();
    let response = client.upload("/_brain/files/attachments", stranger_session, "notes.zip", "application/x-anything", &zip).await;
    assert_eq!(response.status(), 201);
    assert_eq!(json_body(response).await["sniffedType"], "application/zip");
    let response = client.upload("/_brain/files/attachments", stranger_session, "blob.bin", "image/png", &[0, 1, 2, 255, 0]).await;
    let blob = json_body(response).await;
    assert_eq!(blob["contentType"], "application/octet-stream");
    assert_eq!(blob["sniffedType"], Value::Null);
    // Exactly one field named file.
    let ct = format!("multipart/form-data; boundary={BOUNDARY}");
    let response = client.send("POST", "/_brain/files/attachments", Some(stranger_session), &[("content-type", &ct)], multipart("other", "x.txt", "text/plain", b"x")).await;
    assert_eq!(response.status(), 400);
    // Unknown bucket and role without create grant.
    assert_eq!(client.upload("/_brain/files/nope", owner_session, "a.txt", "text/plain", b"a").await.status(), 404);
    assert_eq!(client.upload("/_brain/files/attachments", company_session, "a.txt", "text/plain", b"a").await.status(), 403);

    // Public buckets serve without a session and with an immutable cache policy.
    let response = client.upload("/_brain/files/banners", owner_session, "hero.png", "image/png", &png).await;
    let banner = json_body(response).await;
    let response = client.send("GET", banner["url"].as_str().unwrap(), None, &[], vec![]).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "public, max-age=31536000, immutable");
    assert_eq!(response.headers()["content-type"], "image/png");

    // Deletion follows the delete grants and removes the stored bytes.
    assert_eq!(client.send("DELETE", &location, Some(stranger_session), &[], vec![]).await.status(), 403);
    assert_eq!(client.send("DELETE", &location, Some(owner_session), &[], vec![]).await.status(), 204);
    assert_eq!(client.send("GET", &location, Some(owner_session), &[], vec![]).await.status(), 404);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM _brain_files WHERE id=$1").bind(id.parse::<Uuid>().unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!(left, 0);
    let stored: usize = walk(&root.0.join("files/invoices"));
    assert_eq!(stored, 0, "deleted bytes are removed from private storage");
}
fn walk(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| if e.path().is_dir() { walk(&e.path()) } else { 1 })
                .sum()
        })
        .unwrap_or(0)
}
