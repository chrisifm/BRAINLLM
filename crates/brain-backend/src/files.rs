//! Declared file buckets: any content type, streamed to private disk, recorded in Postgres, served by policy.
//!
//! A bucket is a literal contract (`server/files/<name>.jsbrain`). Client filenames and declared types never choose
//! a path; magic bytes are sniffed and recorded. A bucket lists its types, or says `types: '*'` to accept any.
use crate::Resource;
use axum::body::Bytes;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// 25 MiB: generous for documents, photos and short clips in an MVP; raise per bucket for video.
pub const DEFAULT_MAX_BYTES: u64 = 25 * 1024 * 1024;
/// Hard ceiling for one declared bucket (2 GiB). Larger media belongs to object storage with direct uploads.
pub const MAX_BYTES_CEILING: u64 = 2 * 1024 * 1024 * 1024;
/// Bytes inspected for magic-number sniffing and text detection.
const SNIFF_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Private,
    Public,
}
impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }
}
fn private() -> Visibility {
    Visibility::Private
}
fn owner_admin() -> Vec<String> {
    vec!["owner".into(), "admin".into()]
}
fn admin() -> Vec<String> {
    vec!["admin".into()]
}
fn default_max() -> u64 {
    DEFAULT_MAX_BYTES
}

/// Parse a byte size: an integer, or a string such as `"512kb"`, `"25mb"`, `"1gb"` (binary multiples).
pub fn parse_size(value: &Value) -> Result<u64, String> {
    let bad = || "Sizes are a positive integer of bytes or a string such as \"512kb\", \"25mb\" or \"1gb\"".to_string();
    match value {
        Value::Number(n) => n.as_u64().filter(|n| *n > 0).ok_or_else(bad),
        Value::String(s) => {
            let s = s.trim().to_ascii_lowercase();
            let (digits, unit) = s
                .find(|c: char| !c.is_ascii_digit())
                .map(|i| s.split_at(i))
                .unwrap_or((s.as_str(), ""));
            let n: u64 = digits.parse().map_err(|_| bad())?;
            let factor: u64 = match unit.trim() {
                "" | "b" => 1,
                "kb" | "k" => 1024,
                "mb" | "m" => 1024 * 1024,
                "gb" | "g" => 1024 * 1024 * 1024,
                _ => return Err(bad()),
            };
            n.checked_mul(factor).filter(|n| *n > 0).ok_or_else(bad)
        }
        _ => Err(bad()),
    }
}
fn size<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    parse_size(&Value::deserialize(d)?).map_err(serde::de::Error::custom)
}

fn type_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) if s == "*" => Ok(vec!["*/*".into()]),
        Value::Array(items) => items
            .into_iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| serde::de::Error::custom("types are MIME patterns such as application/pdf, or '*' for any type")))
            .collect(),
        _ => Err(serde::de::Error::custom("types are a list of MIME patterns such as application/pdf, or '*' for any type")),
    }
}
/// One declared bucket. Grants: configured role names, `owner` (uploader) and `related` (may read the linked record).
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Bucket {
    #[serde(default)]
    pub name: String,
    #[serde(default = "private")]
    pub visibility: Visibility,
    #[serde(default = "default_max", deserialize_with = "size")]
    pub max_bytes: u64,
    /// MIME patterns (`application/pdf`, `image/*`), or `'*'` to accept any type. Omitted or empty accepts nothing:
    /// `brain check` reports B0770 and `brain backend serve` refuses to start.
    #[serde(default, deserialize_with = "type_list")]
    pub types: Vec<String>,
    /// Optional resource whose records own these files; uploads then need `?record=<uuid>` and update rights on it.
    #[serde(default)]
    pub resource: Option<String>,
    /// Optional documentation of the resource field that stores the returned file id.
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default = "owner_admin")]
    pub read: Vec<String>,
    #[serde(default = "admin")]
    pub create: Vec<String>,
    #[serde(default = "owner_admin")]
    pub delete: Vec<String>,
}
impl Bucket {
    /// Build a bucket from its literal contract value. The name comes from the file name.
    pub fn from_contract(name: &str, value: Value) -> Result<Self, String> {
        let mut bucket: Bucket = serde_json::from_value(value).map_err(|e| {
            format!("server/files/{name}.jsbrain: {e}; use visibility, maxBytes, types, resource, field, read, create and delete")
        })?;
        bucket.name = name.into();
        Ok(bucket)
    }
    pub fn validate(&self, resources: &[Resource], roles: &[String]) -> Result<(), String> {
        let at = format!("server/files/{}.jsbrain", self.name);
        if !crate::contract::identifier(&self.name) {
            return Err(format!("{at}: bucket names are lowercase identifiers (letters, digits, underscores)"));
        }
        if self.max_bytes == 0 || self.max_bytes > MAX_BYTES_CEILING {
            return Err(format!("{at}: maxBytes must be 1 byte..2gb"));
        }
        if self.types.len() > 64 || self.types.iter().any(|t| !mime_pattern(t)) {
            return Err(format!("{at}: types are at most 64 MIME patterns such as application/pdf or image/*"));
        }
        let resource = match &self.resource {
            Some(name) => Some(
                resources
                    .iter()
                    .find(|r| &r.name == name)
                    .ok_or_else(|| format!("{at}: resource {name} is not declared"))?,
            ),
            None => None,
        };
        if let Some(field) = &self.field {
            let declared = resource.is_some_and(|r| r.fields.contains_key(field));
            if !declared {
                return Err(format!("{at}: field must name a field of the declared resource"));
            }
        }
        for (operation, grants) in [("read", &self.read), ("create", &self.create), ("delete", &self.delete)] {
            if grants.len() > 16 {
                return Err(format!("{at}: {operation} lists at most 16 grants"));
            }
            for grant in grants {
                let special = matches!(grant.as_str(), "owner" | "related");
                if grant == "public" {
                    return Err(format!("{at}: use visibility: 'public' instead of a public grant"));
                }
                if special && operation == "create" {
                    return Err(format!("{at}: create grants are role names; owner/related apply to existing files"));
                }
                if grant == "related" && self.resource.is_none() {
                    return Err(format!("{at}: the related grant needs a declared resource"));
                }
                if !special && !roles.contains(grant) {
                    return Err(format!("{at}: {operation} names an unconfigured role {grant}"));
                }
            }
        }
        Ok(())
    }
    /// A bucket that never said which types it takes (B0770).
    pub fn open_by_omission(&self) -> bool {
        self.types.is_empty()
    }
    pub fn accepts(&self, content_type: &str) -> bool {
        let detected = same_type(content_type);
        self.types.iter().any(|pattern| {
                let pattern = same_type(pattern);
                pattern == "*/*"
                    || pattern == detected
                    || contains_format(content_type) == Some(pattern)
                    || pattern
                        .strip_suffix("/*")
                        .is_some_and(|family| content_type.split('/').next() == Some(family))
            })
    }
    /// Role grant for every file of the bucket.
    pub fn role_allows(&self, operation: &str, role: &str) -> bool {
        if crate::api_keys::system_active() {
            return true;
        }
        self.grants(operation).iter().any(|g| g == role)
    }
    pub fn grants(&self, operation: &str) -> &[String] {
        match operation {
            "read" => &self.read,
            "create" => &self.create,
            "delete" => &self.delete,
            _ => &[],
        }
    }
    pub fn schema(&self) -> Value {
        json!({"name":self.name,"visibility":self.visibility,"maxBytes":self.max_bytes,"types":self.types,"resource":self.resource,"field":self.field,"read":self.read,"create":self.create,"delete":self.delete})
    }
}
fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 127
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b))
}
fn mime_pattern(s: &str) -> bool {
    s == "*/*"
        || s.split_once('/').is_some_and(|(a, b)| {
            token(a) && (b == "*" || token(b)) && s == s.to_ascii_lowercase()
        })
}
/// One name per format: the registered MIME type stands for the names detection and browsers also use.
fn same_type(mime: &str) -> &str {
    match mime {
        "image/jpg" | "image/pjpeg" => "image/jpeg",
        "image/heic" | "image/heic-sequence" | "image/heif-sequence" => "image/heif",
        "audio/mp3" => "audio/mpeg",
        "audio/wav" | "audio/wave" | "audio/vnd.wave" => "audio/x-wav",
        "audio/mp4" | "audio/x-m4a" => "audio/m4a",
        "application/x-zip-compressed" | "application/x-zip" => "application/zip",
        "application/x-pdf" => "application/pdf",
        other => other,
    }
}
/// The container an Office format is stored in: a bucket that lists the container keeps accepting the format.
fn contains_format(mime: &str) -> Option<&'static str> {
    match mime {
        DOCX | XLSX | PPTX => Some("application/zip"),
        DOC | XLS | PPT => Some(OLE),
        _ => None,
    }
}
const ZIP: &str = "application/zip";
const OLE: &str = "application/x-ole-storage";
const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const DOC: &str = "application/msword";
const XLS: &str = "application/vnd.ms-excel";
const PPT: &str = "application/vnd.ms-powerpoint";
/// Central directory bytes read to tell an Office document from a plain zip, and directory bytes for the older format.
const CONTAINER_TAIL: u64 = 1024 * 1024 + 66 * 1024;
const OLE_DIRECTORY: usize = 64 * 1024;
fn le16(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize)
}
fn le32(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}
/// The Office format of a zip, from the names in its central directory: the signature every zip shares says nothing,
/// and a real document's main part can sit anywhere in the archive. `tail` is the end of the file.
pub fn zip_format(tail: &[u8]) -> Option<&'static str> {
    let end = tail.windows(4).rposition(|w| w == [0x50, 0x4b, 0x05, 0x06])?;
    let size = le32(tail, end + 12)?;
    let mut at = end.checked_sub(size)?;
    let (mut manifest, mut format) = (false, None);
    for _ in 0..le16(tail, end + 10)? {
        if tail.get(at..at + 4)? != [0x50, 0x4b, 0x01, 0x02] {
            return None;
        }
        let (name, extra, comment) = (le16(tail, at + 28)?, le16(tail, at + 30)?, le16(tail, at + 32)?);
        let entry = tail.get(at + 46..at + 46 + name)?;
        manifest |= entry == b"[Content_Types].xml";
        format = format.or(if entry == b"word/document.xml" {
            Some(DOCX)
        } else if entry == b"xl/workbook.xml" {
            Some(XLSX)
        } else if entry == b"ppt/presentation.xml" {
            Some(PPTX)
        } else {
            None
        });
        at += 46 + name + extra + comment;
    }
    format.filter(|_| manifest)
}
/// The Office format of an OLE compound file, from the stream names in its directory; other compound files (installers
/// among them) stay `application/x-ole-storage`. `directory` starts at its first sector.
fn ole_format(directory: &[u8]) -> Option<&'static str> {
    let mut entries = directory.chunks_exact(128);
    entries.find_map(|entry| {
        let len = le16(entry, 64)?.checked_sub(2)?;
        let name: Vec<u16> = entry.get(..len)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        match String::from_utf16(&name).ok()?.as_str() {
            "WordDocument" => Some(DOC),
            "Workbook" | "Book" => Some(XLS),
            "PowerPoint Document" => Some(PPT),
            _ => None,
        }
    })
}
/// Sharpen a container type with what only the whole stored file shows. Any read failure keeps the container type.
pub async fn refine(path: &Path, head: &[u8], sniffed: Option<String>) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let sharper = async {
        let mut file = tokio::fs::File::open(path).await.ok()?;
        match sniffed.as_deref()? {
            ZIP => {
                let size = file.metadata().await.ok()?.len();
                file.seek(std::io::SeekFrom::Start(size.saturating_sub(CONTAINER_TAIL))).await.ok()?;
                let mut tail = Vec::new();
                file.read_to_end(&mut tail).await.ok()?;
                zip_format(&tail)
            }
            OLE => {
                let (shift, first) = (le16(head, 30)?, le32(head, 48)?);
                let offset = (first as u64 + 1).checked_mul(1u64.checked_shl(shift as u32)?)?;
                file.seek(std::io::SeekFrom::Start(offset)).await.ok()?;
                let mut directory = vec![0; OLE_DIRECTORY];
                let mut filled = 0;
                while filled < directory.len() {
                    match file.read(&mut directory[filled..]).await.ok()? {
                        0 => break,
                        n => filled += n,
                    }
                }
                directory.truncate(filled);
                ole_format(&directory)
            }
            _ => None,
        }
    };
    sharper.await.map(str::to_string).or(sniffed)
}
/// Normalize a client-declared type to `type/subtype` (parameters dropped); unusable values become octet-stream.
pub fn declared_type(value: Option<&str>) -> String {
    let base = value
        .and_then(|v| v.split(';').next())
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match base.split_once('/') {
        Some((a, b)) if token(a) && token(b) => base,
        _ => "application/octet-stream".into(),
    }
}
/// Keep only the final path component, drop control characters and bound the length; never used as a path.
pub fn clean_filename(value: Option<&str>) -> String {
    let raw = value.unwrap_or("").rsplit(['/', '\\']).next().unwrap_or("");
    let mut out = String::new();
    for c in raw.chars().filter(|c| !c.is_control() && *c != '"') {
        if out.len() + c.len_utf8() > 255 {
            break;
        }
        out.push(c);
    }
    let out = out.trim().trim_start_matches('.').to_string();
    if out.is_empty() { "file".into() } else { out }
}
fn textual(head: &[u8]) -> bool {
    if head.is_empty() || head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // A multi-byte character may be cut at the sniff boundary.
        Err(e) => e.error_len().is_none() && e.valid_up_to() + 4 >= head.len(),
    }
}
/// Magic-byte sniffing. Returns `(sniffed, served)`: the detected type (if any) and the type used for serving.
pub fn classify(head: &[u8], declared: &str) -> (Option<String>, String) {
    if let Some(kind) = infer::get(head) {
        let sniffed = kind.mime_type().to_string();
        return (Some(sniffed.clone()), sniffed);
    }
    let text_declared = declared.starts_with("text/")
        || matches!(
            declared,
            "application/json" | "application/xml" | "application/javascript" | "image/svg+xml"
        );
    if textual(head) {
        let served = if text_declared { declared.to_string() } else { "text/plain".into() };
        return (None, served);
    }
    (None, "application/octet-stream".into())
}
/// Types that may render inline when explicitly requested; everything else is an attachment.
pub fn inline_safe(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif" | "application/pdf" | "text/plain"
    ) || content_type.starts_with("audio/")
        || content_type.starts_with("video/")
}

/// A source of body chunks (an axum multipart field in production, a vector in tests).
pub trait Chunks {
    fn chunk(&mut self) -> impl std::future::Future<Output = Result<Option<Bytes>, ()>> + Send;
}
impl Chunks for axum::extract::multipart::Field<'_> {
    async fn chunk(&mut self) -> Result<Option<Bytes>, ()> {
        axum::extract::multipart::Field::chunk(self).await.map_err(|_| ())
    }
}
impl Chunks for std::collections::VecDeque<Bytes> {
    async fn chunk(&mut self) -> Result<Option<Bytes>, ()> {
        Ok(self.pop_front())
    }
}

/// Private filesystem layout: `<root>/files/<bucket>/<aa>/<uuid>`; incoming parts live in `<root>/files/.incoming`.
#[derive(Clone, Debug)]
pub struct Storage {
    root: PathBuf,
}
pub struct Stored {
    pub id: Uuid,
    pub key: String,
    pub bytes: u64,
    pub sha256: String,
    pub head: Vec<u8>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ReceiveError {
    TooLarge,
    Empty,
    Io,
    Body,
}
impl Storage {
    pub fn new(uploads: &Path) -> Self {
        Self { root: uploads.join("files") }
    }
    pub fn path(&self, key: &str) -> Option<PathBuf> {
        let ok = key.split('/').count() == 3
            && key.split('/').all(|part| {
                !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            });
        ok.then(|| self.root.join(key))
    }
    /// Stream chunks to a private part file, hashing and counting; publish atomically under a fresh key.
    pub async fn receive(&self, bucket: &str, max: u64, source: &mut impl Chunks) -> Result<Stored, ReceiveError> {
        let incoming = self.root.join(".incoming");
        create_private_dir(&incoming).await.map_err(|_| ReceiveError::Io)?;
        let id = Uuid::new_v4();
        let part = incoming.join(format!("{id}.part"));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&part).await.map_err(|_| ReceiveError::Io)?;
        let guard = Remove(Some(part.clone()));
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut head = Vec::new();
        loop {
            let chunk = match source.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(_) => return Err(ReceiveError::Body),
            };
            bytes += chunk.len() as u64;
            if bytes > max {
                return Err(ReceiveError::TooLarge);
            }
            if head.len() < SNIFF_BYTES {
                let take = (SNIFF_BYTES - head.len()).min(chunk.len());
                head.extend_from_slice(&chunk[..take]);
            }
            hash.update(&chunk);
            file.write_all(&chunk).await.map_err(|_| ReceiveError::Io)?;
        }
        if bytes == 0 {
            return Err(ReceiveError::Empty);
        }
        file.sync_all().await.map_err(|_| ReceiveError::Io)?;
        drop(file);
        let simple = id.simple().to_string();
        let key = format!("{bucket}/{}/{simple}", &simple[..2]);
        let target = self.root.join(&key);
        create_private_dir(target.parent().ok_or(ReceiveError::Io)?)
            .await
            .map_err(|_| ReceiveError::Io)?;
        // Fresh random key: hard link fails instead of overwriting if it already exists.
        tokio::fs::hard_link(&part, &target).await.map_err(|_| ReceiveError::Io)?;
        drop(guard);
        let sha256 = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        Ok(Stored { id, key, bytes, sha256, head })
    }
    /// Removes stored bytes. Bytes already gone are a finished removal; a key outside storage is refused, never followed.
    pub async fn remove(&self, key: &str) -> std::io::Result<()> {
        let path = self.path(key).ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        match tokio::fs::remove_file(path).await {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}
struct Remove(Option<PathBuf>);
impl Drop for Remove {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}
async fn create_private_dir(path: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    Ok(())
}

/// Recorded metadata row.
#[derive(Clone, Debug)]
pub struct Record {
    pub id: Uuid,
    pub bucket: String,
    pub owner: Uuid,
    pub resource: Option<String>,
    pub record: Option<Uuid>,
    pub field: Option<String>,
    pub filename: String,
    pub declared_type: String,
    pub sniffed_type: Option<String>,
    pub content_type: String,
    pub bytes: i64,
    pub sha256: String,
    pub visibility: String,
    pub storage_key: String,
    pub created_at: String,
}
impl Record {
    /// What an anonymous reader of a public bucket learns: enough to render the file, nothing about its origin.
    pub fn public_json(&self) -> Value {
        json!({"id":self.id,"bucket":self.bucket,"filename":self.filename,"contentType":self.content_type,"bytes":self.bytes,"visibility":self.visibility,"createdAt":self.created_at,"url":format!("/_brain/files/{}/{}",self.bucket,self.id)})
    }
    pub fn json(&self) -> Value {
        json!({"id":self.id,"bucket":self.bucket,"owner":self.owner,"resource":self.resource,"record":self.record,"field":self.field,"filename":self.filename,"contentType":self.content_type,"sniffedType":self.sniffed_type,"declaredType":self.declared_type,"bytes":self.bytes,"sha256":self.sha256,"visibility":self.visibility,"createdAt":self.created_at,"url":format!("/_brain/files/{}/{}",self.bucket,self.id)})
    }
}
const COLUMNS: &str = "id,bucket,owner,resource,record,field,filename,declared_type,sniffed_type,content_type,bytes,sha256,visibility,storage_key,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created";
fn row(r: sqlx::postgres::PgRow) -> Record {
    Record {
        id: r.get("id"),
        bucket: r.get("bucket"),
        owner: r.get("owner"),
        resource: r.get("resource"),
        record: r.get("record"),
        field: r.get("field"),
        filename: r.get("filename"),
        declared_type: r.get("declared_type"),
        sniffed_type: r.get("sniffed_type"),
        content_type: r.get("content_type"),
        bytes: r.get("bytes"),
        sha256: r.get("sha256"),
        visibility: r.get("visibility"),
        storage_key: r.get("storage_key"),
        created_at: r.get("created"),
    }
}
pub struct NewFile<'a> {
    pub bucket: &'a Bucket,
    pub owner: Uuid,
    pub record: Option<Uuid>,
    pub filename: String,
    pub declared_type: String,
    pub sniffed_type: Option<String>,
    pub content_type: String,
}
pub async fn insert(pool: &PgPool, stored: &Stored, file: NewFile<'_>) -> Result<Record, sqlx::Error> {
    let sql = format!("INSERT INTO public._brain_files(id,bucket,owner,resource,record,field,filename,declared_type,sniffed_type,content_type,bytes,sha256,visibility,storage_key) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) RETURNING {COLUMNS}");
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(stored.id)
        .bind(&file.bucket.name)
        .bind(file.owner)
        .bind(file.record.and(file.bucket.resource.as_ref()))
        .bind(file.record)
        .bind(file.record.and(file.bucket.field.as_ref()))
        .bind(&file.filename)
        .bind(&file.declared_type)
        .bind(&file.sniffed_type)
        .bind(&file.content_type)
        .bind(stored.bytes as i64)
        .bind(&stored.sha256)
        .bind(file.bucket.visibility.as_str())
        .bind(&stored.key)
        .fetch_one(pool)
        .await
        .map(row)
}
pub async fn get(pool: &PgPool, bucket: &str, id: Uuid) -> Result<Option<Record>, sqlx::Error> {
    sqlx::query(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM public._brain_files WHERE bucket=$1 AND id=$2")))
        .bind(bucket)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map(|r| r.map(row))
}
/// `owner` narrows to one uploader; `record` narrows to one linked record.
pub async fn list(
    pool: &PgPool,
    bucket: &str,
    owner: Option<Uuid>,
    record: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Record>, sqlx::Error> {
    sqlx::query(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM public._brain_files WHERE bucket=$1 AND ($2::uuid IS NULL OR owner=$2) AND ($3::uuid IS NULL OR record=$3) ORDER BY created_at DESC,id DESC LIMIT $4 OFFSET $5")))
        .bind(bucket)
        .bind(owner)
        .bind(record)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map(|rows| rows.into_iter().map(row).collect())
}
/// Removes the row; a trigger records its bytes in `_brain_file_deletions` in the same transaction, so the delete
/// survives a storage failure or a crash. Call `finish` for a prompt removal; `sweep` retries whatever remains.
pub async fn delete(pool: &PgPool, bucket: &str, id: Uuid) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("DELETE FROM public._brain_files WHERE bucket=$1 AND id=$2 RETURNING storage_key")
        .bind(bucket)
        .bind(id)
        .fetch_optional(pool)
        .await
}
/// Removes the bytes and clears their tombstone; `false` leaves the tombstone for the next sweep.
pub async fn finish(pool: &PgPool, storage: &Storage, key: &str) -> bool {
    if storage.remove(key).await.is_err() {
        return false;
    }
    // A tombstone left behind only repeats a removal that already succeeded.
    let _ = sqlx::query("DELETE FROM public._brain_file_deletions WHERE storage_key=$1").bind(key).execute(pool).await;
    true
}
/// Drops the bytes of an upload that never got a row; if storage refuses, they are recorded like a deleted file's.
pub async fn discard(pool: &PgPool, storage: &Storage, key: &str) {
    if storage.remove(key).await.is_ok() {
        let _ = sqlx::query("DELETE FROM public._brain_file_deletions WHERE storage_key=$1").bind(key).execute(pool).await;
        return;
    }
    if let Err(error) = sqlx::query("INSERT INTO public._brain_file_deletions(storage_key) VALUES($1) ON CONFLICT DO NOTHING").bind(key).execute(pool).await {
        eprintln!("{}", json!({"schemaVersion":1,"event":"file-deletion-unrecorded","code":"FILE_DELETION_UNRECORDED","fix":"Inspect file storage permissions and database availability","error":crate::db::safe(error)}));
    }
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Sweep {
    pub removed: u64,
    pub failed: u64,
}
/// Retries up to `limit` due tombstones. Claiming pushes each one's next attempt forward (1 minute doubling to 1 hour),
/// so concurrent processes skip it and a failure cannot spin.
pub async fn sweep(pool: &PgPool, storage: &Storage, limit: i64) -> Result<Sweep, sqlx::Error> {
    let keys: Vec<String> = sqlx::query_scalar(
        "UPDATE public._brain_file_deletions SET attempts=attempts+1,updated_at=now(),next_attempt_at=now()+LEAST(interval '1 hour',interval '1 minute'*power(2,LEAST(attempts,6))) \
         WHERE storage_key IN (SELECT storage_key FROM public._brain_file_deletions WHERE next_attempt_at<=now() ORDER BY next_attempt_at LIMIT $1 FOR UPDATE SKIP LOCKED) RETURNING storage_key",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut outcome = Sweep::default();
    for key in keys {
        if finish(pool, storage, &key).await {
            outcome.removed += 1;
        } else {
            outcome.failed += 1;
        }
    }
    Ok(outcome)
}
/// Sweep loop for a serving backend: at boot, so a crash mid-delete recovers, and then every minute.
pub async fn sweep_loop(pool: PgPool, storage: Storage) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        tick.tick().await;
        if let Ok(Sweep { removed, failed }) = sweep(&pool, &storage, 500).await
            && failed > 0
        {
            eprintln!("{}", json!({"schemaVersion":1,"event":"file-deletion-failed","code":"FILE_DELETION_FAILED","removed":removed,"failed":failed,"fix":"Inspect permissions and free space of the uploads directory; rows in _brain_file_deletions retry on their own"}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bucket(v: Value) -> Result<Bucket, String> {
        Bucket::from_contract("docs", v)
    }
    #[test]
    fn sizes_accept_bytes_and_binary_units() {
        assert_eq!(parse_size(&json!(1024)).unwrap(), 1024);
        assert_eq!(parse_size(&json!("25mb")).unwrap(), 25 * 1024 * 1024);
        assert_eq!(parse_size(&json!("512 KB")).unwrap(), 512 * 1024);
        assert_eq!(parse_size(&json!("1gb")).unwrap(), 1 << 30);
        assert!(parse_size(&json!(0)).is_err());
        assert!(parse_size(&json!("12xb")).is_err());
        assert!(parse_size(&json!(-1)).is_err());
    }
    #[test]
    fn defaults_are_private_25mb_and_admin_upload() {
        let b = bucket(json!({"types":"*"})).unwrap();
        assert_eq!(b.visibility, Visibility::Private);
        assert_eq!(b.max_bytes, DEFAULT_MAX_BYTES);
        assert!(b.accepts("application/zip") && b.accepts("video/mp4"));
        assert_eq!(b.create, vec!["admin"]);
        assert_eq!(b.read, vec!["owner", "admin"]);
        assert!(b.validate(&[], &["admin".into()]).is_ok());
    }
    #[test]
    fn a_bucket_without_types_accepts_nothing() {
        let open = bucket(json!({})).unwrap();
        assert!(open.open_by_omission() && !open.accepts("application/pdf"));
        assert!(!bucket(json!({"types":[]})).unwrap().accepts("application/pdf"));
    }
    #[test]
    fn declared_types_restrict_by_exact_type_or_family() {
        let b = bucket(json!({"types":["application/pdf","image/*"]})).unwrap();
        assert!(b.accepts("application/pdf") && b.accepts("image/webp"));
        assert!(!b.accepts("application/zip") && !b.accepts("text/plain"));
        assert!(bucket(json!({"types":["Application/PDF"]})).unwrap().validate(&[], &[]).is_err());
        assert!(bucket(json!({"maxBytes":"3gb"})).unwrap().validate(&[], &[]).is_err());
        assert!(bucket(json!({"color":"red"})).is_err());
    }
    #[test]
    fn any_type_is_said_with_a_star_and_omission_is_detected() {
        let star = bucket(json!({"types":"*"})).unwrap();
        assert!(star.accepts("application/x-msdownload") && !star.open_by_omission());
        assert!(star.validate(&[], &["admin".to_string()]).is_ok());
        assert!(bucket(json!({})).unwrap().open_by_omission());
        assert!(bucket(json!({"types":[]})).unwrap().open_by_omission());
        assert!(bucket(json!({"types":"pdf"})).is_err());
        assert!(bucket(json!({"types":[1]})).is_err());
        assert!(!bucket(json!({"types":["application/pdf"]})).unwrap().open_by_omission());
    }
    #[test]
    fn official_names_match_what_detection_reports() {
        let b = bucket(json!({"types":["image/heic","image/jpg","audio/mp3","audio/wav","application/x-zip-compressed"]})).unwrap();
        for detected in ["image/heif", "image/jpeg", "audio/mpeg", "audio/x-wav", "application/zip"] {
            assert!(b.accepts(detected), "{detected}");
        }
        assert!(!b.accepts("image/png") && !b.accepts(DOC));
        let word = bucket(json!({"types":[DOCX, DOC]})).unwrap();
        assert!(word.accepts(DOCX) && word.accepts(DOC));
        assert!(!word.accepts(ZIP), "a plain zip is not a Word document");
        assert!(!word.accepts(OLE), "an unknown compound file is not a Word document");
        // Apps that listed the container before the format was told apart keep accepting their documents.
        let containers = bucket(json!({"types":["application/zip", OLE]})).unwrap();
        assert!(containers.accepts(DOCX) && containers.accepts(XLSX) && containers.accepts(DOC) && containers.accepts(ZIP));
    }
    /// A zip with the given entry names and nothing stored in them; the central directory is what is read.
    fn zip_of(names: &[&str]) -> Vec<u8> {
        let mut central = Vec::new();
        for name in names {
            central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
            central.extend_from_slice(&[0; 24]);
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 16]);
            central.extend_from_slice(name.as_bytes());
        }
        let mut out = vec![0x50, 0x4b, 0x03, 0x04];
        let offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend_from_slice(&(names.len() as u16).to_le_bytes());
        out.extend_from_slice(&(names.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }
    #[test]
    fn a_word_document_is_told_apart_from_any_other_zip() {
        assert_eq!(zip_format(&zip_of(&["[Content_Types].xml", "_rels/.rels", "docProps/app.xml", "word/document.xml"])), Some(DOCX));
        assert_eq!(zip_format(&zip_of(&["[Content_Types].xml", "xl/workbook.xml"])), Some(XLSX));
        assert_eq!(zip_format(&zip_of(&["[Content_Types].xml", "ppt/presentation.xml"])), Some(PPTX));
        assert_eq!(zip_format(&zip_of(&["setup.exe", "readme.txt"])), None);
        assert_eq!(zip_format(&zip_of(&["word/document.xml"])), None, "no package manifest");
        assert_eq!(zip_format(b"PK\x03\x04 not a directory"), None);
        let mut lying = zip_of(&["[Content_Types].xml", "word/document.xml"]);
        let n = lying.len();
        lying[n - 10..n - 6].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(zip_format(&lying), None, "a directory size past the file is refused, not trusted");
    }
    #[test]
    fn compound_files_name_their_format_by_stream() {
        let entry = |name: &str| {
            let mut e = [0u8; 128];
            let wide: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            e[..wide.len()].copy_from_slice(&wide);
            e[64..66].copy_from_slice(&((wide.len() + 2) as u16).to_le_bytes());
            e.to_vec()
        };
        let dir = |names: &[&str]| names.iter().flat_map(|n| entry(n)).collect::<Vec<u8>>();
        assert_eq!(ole_format(&dir(&["Root Entry", "1Table", "WordDocument"])), Some(DOC));
        assert_eq!(ole_format(&dir(&["Root Entry", "Workbook"])), Some(XLS));
        assert_eq!(ole_format(&dir(&["Root Entry", "PowerPoint Document"])), Some(PPT));
        assert_eq!(ole_format(&dir(&["Root Entry", "SummaryInformation"])), None);
    }
    #[tokio::test]
    async fn refine_reads_the_stored_file_and_keeps_the_container_on_failure() {
        let path = std::env::temp_dir().join(format!("brain-refine-{}", Uuid::new_v4()));
        std::fs::write(&path, zip_of(&["[Content_Types].xml", "word/document.xml"])).unwrap();
        assert_eq!(refine(&path, b"PK", Some(ZIP.into())).await.as_deref(), Some(DOCX));
        std::fs::write(&path, zip_of(&["a.txt"])).unwrap();
        assert_eq!(refine(&path, b"PK", Some(ZIP.into())).await.as_deref(), Some(ZIP));
        assert_eq!(refine(&path, b"", Some("application/pdf".into())).await.as_deref(), Some("application/pdf"));
        let _ = std::fs::remove_file(&path);
        assert_eq!(refine(&path, b"", Some(ZIP.into())).await.as_deref(), Some(ZIP));
        assert_eq!(refine(&path, b"", None).await, None);
    }
    #[test]
    fn grants_are_checked_against_roles_and_resources() {
        let roles = vec!["admin".to_string(), "user".to_string()];
        assert!(bucket(json!({"create":["user"]})).unwrap().validate(&[], &roles).is_ok());
        assert!(bucket(json!({"create":["ghost"]})).unwrap().validate(&[], &roles).is_err());
        assert!(bucket(json!({"create":["owner"]})).unwrap().validate(&[], &roles).is_err());
        assert!(bucket(json!({"read":["public"]})).unwrap().validate(&[], &roles).is_err());
        assert!(bucket(json!({"read":["related"]})).unwrap().validate(&[], &roles).is_err());
        let r: Resource = serde_json::from_value(json!({"name":"orders","fields":{"invoice":{"kind":"str"}},"policy":{"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"]}})).unwrap();
        let linked = bucket(json!({"resource":"orders","field":"invoice","read":["related"]})).unwrap();
        assert!(linked.validate(std::slice::from_ref(&r), &roles).is_ok());
        assert!(bucket(json!({"resource":"orders","field":"nope"})).unwrap().validate(&[r], &roles).is_err());
    }
    #[test]
    fn sniffing_records_magic_bytes_and_never_trusts_declared_binary_types() {
        let pdf = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n1 0 obj\n";
        assert_eq!(classify(pdf, "image/png"), (Some("application/pdf".into()), "application/pdf".into()));
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        assert_eq!(classify(&png, "application/octet-stream").0.as_deref(), Some("image/png"));
        assert_eq!(classify(b"a,b\n1,2\n", "text/csv"), (None, "text/csv".into()));
        assert_eq!(classify(b"plain words", "application/pdf"), (None, "text/plain".into()));
        assert_eq!(classify(&[0, 1, 2, 3, 0xff], "image/png"), (None, "application/octet-stream".into()));
        // A multi-byte character cut at the sniff boundary is still text.
        assert!(textual("a€".as_bytes()[..3].as_ref()));
    }
    #[test]
    fn filenames_and_declared_types_are_sanitized() {
        assert_eq!(clean_filename(Some("../../etc/passwd")), "passwd");
        assert_eq!(clean_filename(Some("C:\\docs\\report.pdf")), "report.pdf");
        assert_eq!(clean_filename(Some("..hidden")), "hidden");
        assert_eq!(clean_filename(Some("a\"b\u{7}.txt")), "ab.txt");
        assert_eq!(clean_filename(None), "file");
        assert_eq!(clean_filename(Some(&"x".repeat(400))).len(), 255);
        assert_eq!(declared_type(Some("Text/CSV; charset=utf-8")), "text/csv");
        assert_eq!(declared_type(Some("nonsense")), "application/octet-stream");
    }
    #[test]
    fn storage_keys_cannot_escape_the_root() {
        let s = Storage::new(Path::new("/tmp/x"));
        assert!(s.path("docs/ab/abcdef").is_some());
        assert!(s.path("../ab/abcdef").is_none());
        assert!(s.path("docs/ab").is_none());
        assert!(s.path("docs/ab/c d").is_none());
    }
    #[tokio::test]
    async fn receive_streams_hashes_bounds_and_cleans_up_parts() {
        let root = std::env::temp_dir().join(format!("brain-files-unit-{}", Uuid::new_v4()));
        let storage = Storage::new(&root);
        let chunks = |parts: Vec<&'static [u8]>| -> std::collections::VecDeque<Bytes> {
            parts.into_iter().map(Bytes::from_static).collect()
        };
        let stored = storage.receive("docs", 16, &mut chunks(vec![b"hello ", b"world"])).await.unwrap();
        assert_eq!(stored.bytes, 11);
        assert_eq!(stored.sha256, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
        assert_eq!(std::fs::read(storage.path(&stored.key).unwrap()).unwrap(), b"hello world");
        assert_eq!(storage.receive("docs", 8, &mut chunks(vec![b"hello ", b"world"])).await.err(), Some(ReceiveError::TooLarge));
        assert_eq!(storage.receive("docs", 8, &mut chunks(vec![])).await.err(), Some(ReceiveError::Empty));
        assert_eq!(std::fs::read_dir(root.join("files/.incoming")).unwrap().count(), 0, "parts are removed");
        let _ = std::fs::remove_dir_all(root);
    }
}
