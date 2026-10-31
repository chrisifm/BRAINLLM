//! HTTP surface of declared file buckets: streaming upload, policy-checked download with Range/ETag, list, delete.
use super::{App, Error, Result};
use crate::{
    auth::User,
    files::{self, Bucket, ReceiveError, Storage, Visibility},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, Request, State as Extract},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use uuid::Uuid;

/// Marks a successful response whose cache policy is declared by the handler (public immutable files).
#[derive(Clone, Copy, Debug)]
pub struct PublicCache;

pub struct State {
    buckets: BTreeMap<String, Bucket>,
    storage: Storage,
    max_upload: u64,
}
impl State {
    pub fn new(
        platform: &crate::platform::Platform,
        resources: &[crate::Resource],
        roles: &[String],
        uploads: &std::path::Path,
    ) -> std::result::Result<Self, String> {
        let mut buckets = BTreeMap::new();
        if platform.buckets.len() > 64 {
            return Err("Declare at most 64 file buckets".into());
        }
        for bucket in &platform.buckets {
            bucket.validate(resources, roles)?;
            if buckets.insert(bucket.name.clone(), bucket.clone()).is_some() {
                return Err(format!("Duplicate file bucket {}", bucket.name));
            }
        }
        let max_upload = buckets.values().map(|b| b.max_bytes).max().unwrap_or(0);
        Ok(Self { buckets, storage: Storage::new(uploads), max_upload })
    }
    fn bucket(&self, name: &str) -> Result<&Bucket> {
        self.buckets.get(name).ok_or_else(Error::missing)
    }
}

/// Upload bodies may take minutes on slow links; reads produce headers quickly and stream afterwards.
pub fn deadline(req: &Request) -> Duration {
    if req.method() == axum::http::Method::POST {
        Duration::from_secs(300)
    } else {
        Duration::from_secs(30)
    }
}

pub fn routes(state: &State) -> Router<App> {
    if state.buckets.is_empty() {
        return Router::new();
    }
    // Multipart framing overhead on top of the largest declared bucket; each bucket enforces its own maximum.
    let limit = usize::try_from(state.max_upload.saturating_add(1024 * 1024)).unwrap_or(usize::MAX);
    Router::new()
        .route(
            "/_brain/files/{bucket}",
            get(list).post(upload).layer(DefaultBodyLimit::max(limit)),
        )
        .route("/_brain/files/{bucket}/{id}", get(download).delete(remove))
        .route("/_brain/files/{bucket}/{id}/meta", get(meta))
}

fn status(status: StatusCode, code: &'static str, fix: &'static str) -> Error {
    Error::new(status, code, fix)
}
fn sign_in() -> Error {
    status(StatusCode::UNAUTHORIZED, "SIGN_IN_REQUIRED", "Sign in before making this request")
}

/// Grants: a role named in the operation list, `owner` (the uploader) or `related` (may read the linked record).
async fn authorize(app: &App, bucket: &Bucket, file: &files::Record, user: Option<&User>, op: &str) -> Result<()> {
    if op == "read" && bucket.visibility == Visibility::Public {
        return Ok(());
    }
    let user = user.ok_or_else(sign_in)?;
    if bucket.role_allows(op, &user.role) {
        return Ok(());
    }
    let grants = bucket.grants(op);
    if grants.iter().any(|g| g == "owner") && file.owner == user.id {
        return Ok(());
    }
    if grants.iter().any(|g| g == "related") && related(app, file.resource.as_deref(), file.record, user, "read").await? {
        return Ok(());
    }
    Err(Error::forbidden())
}
/// Does the resource policy allow `op` on the linked record for this user?
async fn related(app: &App, resource: Option<&str>, record: Option<Uuid>, user: &User, op: &str) -> Result<bool> {
    let (Some(resource), Some(record)) = (resource, record) else {
        return Ok(false);
    };
    let r = app.resource(resource)?;
    let Some(row) = crate::db::get(&app.pool, r, record).await.map_err(Error::internal)? else {
        return Ok(false);
    };
    let owns = row["owner"].as_str() == Some(&user.id.to_string());
    Ok(r.policy.allows(op, Some(&user.role), owns))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    record: Option<Uuid>,
}
async fn upload(
    Extract(app): Extract<App>,
    Path(name): Path<String>,
    Query(target): Query<Target>,
    h: HeaderMap,
    mut multipart: Multipart,
) -> Result<Response> {
    let bucket = app.platform.bucket(&name)?.clone();
    let user = app.required(&h).await?;
    if !bucket.role_allows("create", &user.role) {
        return Err(Error::forbidden());
    }
    match (&bucket.resource, target.record) {
        (Some(_), None) => {
            return Err(Error::validation(json!({"record":"This bucket belongs to a resource; send ?record=<uuid> of a record you may update"})));
        }
        (None, Some(_)) => {
            return Err(Error::validation(json!({"record":"This bucket is not linked to a resource; omit record"})));
        }
        (Some(resource), Some(record)) => {
            if !related(&app, Some(resource), Some(record), &user, "update").await? {
                return Err(Error::forbidden());
            }
        }
        (None, None) => {}
    }
    let invalid = || status(StatusCode::BAD_REQUEST, "INVALID_MULTIPART", "Send multipart/form-data with exactly one field named file");
    let mut field = multipart.next_field().await.map_err(|_| invalid())?.ok_or_else(invalid)?;
    if field.name() != Some("file") {
        return Err(invalid());
    }
    let filename = files::clean_filename(field.file_name());
    let declared = files::declared_type(field.content_type());
    let stored = app
        .platform
        .storage
        .receive(&bucket.name, bucket.max_bytes, &mut field)
        .await
        .map_err(|e| match e {
            ReceiveError::TooLarge => status(StatusCode::PAYLOAD_TOO_LARGE, "FILE_TOO_LARGE", "Send a smaller file; the bucket's maxBytes limit is documented in brain backend schema"),
            ReceiveError::Empty => Error::validation(json!({"file":"The file is empty"})),
            ReceiveError::Body => invalid(),
            ReceiveError::Io => Error::internal("file storage unavailable"),
        })?;
    drop(field);
    let extra = multipart.next_field().await;
    if !matches!(extra, Ok(None)) {
        app.platform.storage.remove(&stored.key).await;
        return Err(invalid());
    }
    let (sniffed, content_type) = files::classify(&stored.head, &declared);
    if !bucket.accepts(&content_type) {
        app.platform.storage.remove(&stored.key).await;
        let mut e = status(StatusCode::UNSUPPORTED_MEDIA_TYPE, "FILE_TYPE_DENIED", "Upload one of the bucket's declared types; detection uses file content, not the name");
        e.fields = Some(json!({"file":{"detected":content_type,"accepted":bucket.types}}));
        return Err(e);
    }
    let created = files::insert(
        &app.pool,
        &stored,
        files::NewFile {
            bucket: &bucket,
            owner: user.id,
            record: target.record,
            filename,
            declared_type: declared,
            sniffed_type: sniffed,
            content_type,
        },
    )
    .await;
    let created = match created {
        Ok(created) => created,
        Err(error) => {
            app.platform.storage.remove(&stored.key).await;
            return Err(Error::internal(crate::db::safe(error)));
        }
    };
    let body = created.json();
    crate::response::created(&format!("/_brain/files/{}/{}", created.bucket, created.id), body)
        .map_err(|_| Error::internal("invalid location"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Listing {
    limit: Option<i64>,
    offset: Option<i64>,
    record: Option<Uuid>,
}
async fn list(Extract(app): Extract<App>, Path(name): Path<String>, Query(q): Query<Listing>, h: HeaderMap) -> Result<Json<serde_json::Value>> {
    let bucket = app.platform.bucket(&name)?;
    let user = app.required(&h).await?;
    let limit = q.limit.unwrap_or(25);
    let offset = q.offset.unwrap_or(0);
    if !(1..=100).contains(&limit) || !(0..=10000).contains(&offset) {
        return Err(Error::validation(json!({"query":"limit 1..100, offset 0..10000"})));
    }
    let grants = bucket.grants("read");
    let owner = if bucket.role_allows("read", &user.role) {
        None
    } else if q.record.is_some()
        && grants.iter().any(|g| g == "related")
        && related(&app, bucket.resource.as_deref(), q.record, &user, "read").await?
    {
        None
    } else if grants.iter().any(|g| g == "owner") {
        Some(user.id)
    } else {
        return Err(Error::forbidden());
    };
    let rows = files::list(&app.pool, &bucket.name, owner, q.record, limit, offset)
        .await
        .map_err(Error::internal)?;
    Ok(Json(json!({"items":rows.iter().map(files::Record::json).collect::<Vec<_>>(),"limit":limit,"offset":offset})))
}

async fn record(app: &App, name: &str, id: Uuid, h: &HeaderMap, op: &str) -> Result<(Bucket, files::Record)> {
    let bucket = app.platform.bucket(name)?.clone();
    let file = files::get(&app.pool, &bucket.name, id).await.map_err(Error::internal)?.ok_or_else(Error::missing)?;
    let user = if op == "read" && bucket.visibility == Visibility::Public { None } else { app.user(h).await? };
    authorize(app, &bucket, &file, user.as_ref(), op).await?;
    Ok((bucket, file))
}
async fn meta(Extract(app): Extract<App>, Path((name, id)): Path<(String, Uuid)>, h: HeaderMap) -> Result<Json<serde_json::Value>> {
    let (_, file) = record(&app, &name, id, &h, "read").await?;
    Ok(Json(file.json()))
}
async fn remove(Extract(app): Extract<App>, Path((name, id)): Path<(String, Uuid)>, h: HeaderMap) -> Result<Response> {
    let (bucket, _) = record(&app, &name, id, &h, "delete").await?;
    if let Some(key) = files::delete(&app.pool, &bucket.name, id).await.map_err(Error::internal)? {
        app.platform.storage.remove(&key).await;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Disposition {
    #[serde(default)]
    inline: bool,
}
/// RFC 6266 / 8187 disposition with an ASCII fallback; the stored name is already sanitized.
fn disposition(kind: &str, filename: &str) -> HeaderValue {
    let ascii: String = filename
        .chars()
        .map(|c| if c.is_ascii_graphic() && !matches!(c, '"' | '\\' | ';' | '%') || c == ' ' { c } else { '_' })
        .collect();
    let mut encoded = String::new();
    for b in filename.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    HeaderValue::from_str(&format!("{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}
fn etag_matches(header: Option<&HeaderValue>, etag: &str) -> bool {
    header.and_then(|v| v.to_str().ok()).is_some_and(|v| {
        v.split(',').map(str::trim).any(|t| t == "*" || t == etag || t.strip_prefix("W/") == Some(etag))
    })
}
async fn download(
    Extract(app): Extract<App>,
    Path((name, id)): Path<(String, Uuid)>,
    Query(q): Query<Disposition>,
    h: HeaderMap,
) -> Result<Response> {
    let (bucket, file) = record(&app, &name, id, &h, "read").await?;
    let path = app.platform.storage.path(&file.storage_key).ok_or_else(|| Error::internal("invalid storage key"))?;
    let mut handle = tokio::fs::File::open(&path).await.map_err(|_| Error::internal("stored file unavailable"))?;
    let size = handle.metadata().await.map_err(Error::internal)?.len();
    if size != file.bytes as u64 {
        return Err(Error::internal("stored file size differs from its record"));
    }
    let etag = format!("\"{}\"", file.sha256);
    let public = bucket.visibility == Visibility::Public;
    let finish = |mut response: Response| {
        let headers = response.headers_mut();
        headers.insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        if public {
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
            response.extensions_mut().insert(PublicCache);
        }
        response
    };
    if etag_matches(h.get(header::IF_NONE_MATCH), &etag) {
        return Ok(finish(StatusCode::NOT_MODIFIED.into_response()));
    }
    // If-Range with a different validator means the client's partial copy is stale: send the whole file.
    let range_allowed = h.get(header::IF_RANGE).is_none_or(|v| v.to_str().ok() == Some(etag.as_str()));
    let mut range = None;
    if let (true, Some(value)) = (range_allowed, h.get(header::RANGE).and_then(|v| v.to_str().ok())) {
        // Syntactically invalid ranges are ignored (full response); unsatisfiable ones are 416.
        if let Ok(parsed) = http_range_header::parse_range_header(value) {
            match parsed.validate(size) {
                Ok(ranges) if ranges.len() == 1 => range = Some(ranges[0].clone()),
                Ok(_) => {}
                Err(_) => {
                    let mut response = status(StatusCode::RANGE_NOT_SATISFIABLE, "RANGE_NOT_SATISFIABLE", "Request a byte range inside the file size").into_response();
                    response.headers_mut().insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes */{size}")).unwrap());
                    return Ok(finish(response));
                }
            }
        }
    }
    let (start, length, code) = match &range {
        Some(r) => (*r.start(), r.end() - r.start() + 1, StatusCode::PARTIAL_CONTENT),
        None => (0, size, StatusCode::OK),
    };
    handle.seek(std::io::SeekFrom::Start(start)).await.map_err(Error::internal)?;
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(handle.take(length)));
    let mut response = (code, body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&file.content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    let kind = if q.inline && files::inline_safe(&file.content_type) { "inline" } else { "attachment" };
    headers.insert(header::CONTENT_DISPOSITION, disposition(kind, &file.filename));
    if let Some(r) = range {
        headers.insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {}-{}/{size}", r.start(), r.end())).unwrap());
    }
    Ok(finish(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disposition_escapes_unicode_and_quotes() {
        let v = disposition("attachment", "price € 2026 \"x\".pdf");
        let s = v.to_str().unwrap();
        assert!(s.starts_with("attachment; filename=\"price _ 2026 _x_.pdf\""));
        assert!(s.contains("filename*=UTF-8''price%20%E2%82%AC%202026%20%22x%22.pdf"));
    }
    #[test]
    fn etags_match_lists_wildcards_and_weak_forms() {
        let etag = "\"abc\"";
        assert!(etag_matches(Some(&HeaderValue::from_static("\"x\", \"abc\"")), etag));
        assert!(etag_matches(Some(&HeaderValue::from_static("W/\"abc\"")), etag));
        assert!(etag_matches(Some(&HeaderValue::from_static("*")), etag));
        assert!(!etag_matches(Some(&HeaderValue::from_static("\"abd\"")), etag));
        assert!(!etag_matches(None, etag));
    }
}
