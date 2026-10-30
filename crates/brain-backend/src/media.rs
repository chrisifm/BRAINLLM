//! Private original images, immutable versioned renditions and bounded storage operations.
use crate::db;
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Semaphore,
};
use uuid::Uuid;
pub const MAX_RENDITIONS: usize = 8;
const MAX_FILE: usize = 20 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub name: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub crop: bool,
    pub format: String,
    #[serde(default)]
    pub quality: Option<u8>,
}
impl Preset {
    fn validate(&self) -> Result<(), String> {
        if self.name.is_empty()
            || self.name.len() > 48
            || !self.name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
            || matches!(self.name.as_str(), "original" | "full" | "thumbnail")
            || !(1..=4096).contains(&self.width)
            || !(1..=4096).contains(&self.height)
            || !matches!(self.format.as_str(), "png" | "jpeg" | "webp")
            || self
                .quality
                .is_some_and(|quality| self.format != "jpeg" || !(1..=100).contains(&quality))
        {
            return Err("Invalid image preset; use a unique name, dimensions 1..4096 and png/jpeg/webp (quality only for JPEG)".into());
        }
        Ok(())
    }
}
pub fn defaults() -> Vec<Preset> {
    let mut presets = Vec::new();
    for format in ["webp", "jpeg"] {
        for width in [320, 640, 1280] {
            presets.push(Preset {
                name: format!("{format}-{width}"),
                width,
                height: 4096,
                crop: false,
                format: format.into(),
                quality: if format == "jpeg" { Some(82) } else { None },
            })
        }
    }
    presets
}
pub struct Rendition {
    pub name: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub sha256: String,
}
pub struct Prepared {
    pub original: Vec<u8>,
    pub original_format: String,
    pub width: u32,
    pub height: u32,
    pub renditions: Vec<Rendition>,
}
fn hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    use std::fmt::Write;
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
fn mime(format: &str) -> Result<&'static str, String> {
    match format {
        "png" => Ok("image/png"),
        "jpeg" => Ok("image/jpeg"),
        "webp" => Ok("image/webp"),
        _ => Err("Unsupported stored image format".into()),
    }
}
fn encode(image: &DynamicImage, format: &str, quality: Option<u8>) -> Result<Vec<u8>, String> {
    let mut output = Cursor::new(Vec::new());
    match format {
        "jpeg" => {
            // Composite alpha onto white before JPEG conversion; hidden transparent pixels must not become visible.
            let mut background = image::RgbaImage::from_pixel(
                image.width(),
                image.height(),
                image::Rgba([255, 255, 255, 255]),
            );
            image::imageops::overlay(&mut background, &image.to_rgba8(), 0, 0);
            let flattened = DynamicImage::ImageRgba8(background).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality.unwrap_or(85))
                .encode_image(&flattened)
                .map_err(|_| "Cannot encode JPEG rendition")?;
        }
        "png" => image
            .write_to(&mut output, ImageFormat::Png)
            .map_err(|_| "Cannot encode PNG rendition")?,
        "webp" => image
            .write_to(&mut output, ImageFormat::WebP)
            .map_err(|_| "Cannot encode lossless WebP rendition")?,
        _ => return Err("Unsupported image encoder".into()),
    }
    let bytes = output.into_inner();
    if bytes.len() > MAX_FILE {
        return Err("Image rendition exceeds 20 MB".into());
    }
    Ok(bytes)
}
pub fn validate_presets(presets: &[Preset]) -> Result<(), String> {
    if presets.len() > MAX_RENDITIONS - 2 {
        return Err("At most six custom presets plus full/thumbnail are supported".into());
    }
    let mut names = BTreeSet::new();
    for preset in presets {
        preset.validate()?;
        if !names.insert(preset.name.clone()) {
            return Err("Image preset names must be unique".into());
        }
    }
    Ok(())
}
pub fn prepare(original: Vec<u8>, presets: &[Preset]) -> Result<Prepared, String> {
    validate_presets(presets)?;
    let (image, format) = crate::upload::decode_content(&original)?;
    let original_format = match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::WebP => "webp",
        _ => return Err("Unsupported image content".into()),
    };
    let mut renditions = Vec::new();
    let mut total = original.len();
    let mut add = |name: &str,
                   format: &str,
                   view: &DynamicImage,
                   quality: Option<u8>,
                   _dedup: bool|
     -> Result<(), String> {
        let data = encode(view, format, quality)?;
        total += data.len();
        if total > MAX_TOTAL {
            return Err("Image outputs exceed 64 MB".into());
        }
        let sha256 = hash(&data);
        renditions.push(Rendition {
            name: name.into(),
            format: format.into(),
            width: view.width(),
            height: view.height(),
            data,
            sha256,
        });
        Ok(())
    };
    add("full", original_format, &image, Some(85), false)?;
    let thumb = image.resize(
        320.min(image.width()),
        320.min(image.height()),
        FilterType::Lanczos3,
    );
    add("thumbnail", "png", &thumb, None, false)?;
    for preset in presets {
        let resized = if preset.crop {
            let factor = 1.0f64
                .min(f64::from(image.width()) / f64::from(preset.width))
                .min(f64::from(image.height()) / f64::from(preset.height));
            let width = ((f64::from(preset.width) * factor).floor() as u32).max(1);
            let height = ((f64::from(preset.height) * factor).floor() as u32).max(1);
            image.resize_to_fill(width, height, FilterType::Lanczos3)
        } else {
            image.resize(
                preset.width.min(image.width()),
                preset.height.min(image.height()),
                FilterType::Lanczos3,
            )
        };
        add(&preset.name, &preset.format, &resized, preset.quality, true)?;
    }
    Ok(Prepared {
        original,
        original_format: original_format.into(),
        width: image.width(),
        height: image.height(),
        renditions,
    })
}
#[derive(Clone)]
pub struct GcsConfig {
    pub bucket: String,
    pub prefix: String,
    pub adapter: PathBuf,
}
impl GcsConfig {
    pub fn new(bucket: String, prefix: String, adapter: PathBuf) -> Result<Self, String> {
        if bucket.len() < 3
            || bucket.len() > 63
            || !bucket.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'.' | b'_')
            })
            || prefix.is_empty()
            || prefix.len() > 80
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !adapter.is_absolute()
            || adapter.extension().is_none_or(|ext| ext != "mjs")
            || std::fs::symlink_metadata(&adapter).is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err("GCS requires an explicit bucket, simple object prefix and private absolute .mjs SDK adapter".into());
        }
        Ok(Self {
            bucket,
            prefix,
            adapter,
        })
    }
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    gcs: Option<GcsConfig>,
    slots: Arc<Semaphore>,
    images: Arc<Semaphore>,
}
#[derive(Clone)]
pub struct Asset {
    pub provider: String,
    pub key: String,
    pub generation: String,
    pub format: String,
    pub bytes: usize,
    pub sha256: String,
}
impl Store {
    pub fn new(root: PathBuf, gcs: Option<GcsConfig>) -> Result<Self, String> {
        if let Some(config) = &gcs {
            GcsConfig::new(
                config.bucket.clone(),
                config.prefix.clone(),
                config.adapter.clone(),
            )?;
        }
        Ok(Self {
            root,
            gcs,
            slots: Arc::new(Semaphore::new(2)),
            images: Arc::new(Semaphore::new(2)),
        })
    }
    pub fn image_slot(&self) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
        self.images
            .clone()
            .try_acquire_owned()
            .map_err(|_| "Image workers busy; retry shortly".into())
    }
    fn object_key(&self, key: &str) -> String {
        match &self.gcs {
            Some(config) => format!("{}/{}/{key}", config.bucket, config.prefix),
            None => key.to_string(),
        }
    }
    fn provider(&self) -> &'static str {
        if self.gcs.is_some() {
            "gcs"
        } else {
            "filesystem"
        }
    }
    async fn root(&self) -> Result<PathBuf, String> {
        for directory in [Some(self.root.as_path()), self.root.parent()]
            .into_iter()
            .flatten()
        {
            if tokio::fs::symlink_metadata(directory)
                .await
                .is_ok_and(|meta| meta.file_type().is_symlink())
            {
                return Err("Private image storage refuses symlink roots".into());
            }
        }
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|_| "Cannot create private image storage")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(|_| "Cannot restrict private image storage")?;
        }
        tokio::fs::canonicalize(&self.root)
            .await
            .map_err(|_| "Cannot resolve private image storage".to_string())
    }
    fn key(key: &str) -> bool {
        !key.is_empty()
            && key.len() <= 512
            && !key.starts_with('/')
            && key.split('/').all(|part| {
                !part.is_empty()
                    && !matches!(part, "." | "..")
                    && part.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
            })
    }
    async fn path(&self, key: &str, create: bool) -> Result<PathBuf, String> {
        if !Self::key(key) {
            return Err("Invalid private object identity".into());
        }
        let root = self.root().await?;
        let mut path = root;
        let parts = key.split('/').collect::<Vec<_>>();
        for (index, part) in parts.iter().enumerate() {
            path.push(part);
            if tokio::fs::symlink_metadata(&path)
                .await
                .is_ok_and(|meta| meta.file_type().is_symlink())
            {
                return Err("Private image storage refuses symlinks".into());
            }
            if index + 1 < parts.len() && create {
                match tokio::fs::create_dir(&path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => return Err("Cannot create private image directory".into()),
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                        .await
                        .map_err(|_| "Cannot restrict private image directory")?;
                }
            }
        }
        Ok(path)
    }
    async fn write(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_FILE {
            return Err("Private image object exceeds 20 MB".into());
        }
        let path = self.path(key, true).await?;
        let temporary = path.with_extension(format!("pending-{}", Uuid::new_v4().simple()));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&temporary)
            .await
            .map_err(|_| "Cannot create private image temporary")?;
        let _owned = Temporary(temporary.clone());
        let result = async {
            file.write_all(bytes)
                .await
                .map_err(|_| "Cannot write private image")?;
            file.sync_all()
                .await
                .map_err(|_| "Cannot persist private image")?;
            drop(file);
            tokio::fs::hard_link(&temporary, &path)
                .await
                .map_err(|_| "Cannot publish exclusive immutable image")
        }
        .await;
        result.map_err(str::to_string)
    }
    async fn local_read(&self, key: &str, expected: usize, sha: &str) -> Result<Vec<u8>, String> {
        if expected > MAX_FILE {
            return Err("Private image exceeds read budget".into());
        }
        let path = self.path(key, false).await?;
        if !tokio::fs::symlink_metadata(&path)
            .await
            .is_ok_and(|meta| meta.is_file())
        {
            return Err("Private image is unavailable".into());
        }
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|_| "Private image is unavailable")?;
        let mut bytes = Vec::new();
        file.take((MAX_FILE + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "Cannot read private image")?;
        if bytes.len() != expected || hash(&bytes) != sha {
            return Err("Private image integrity check failed".into());
        }
        Ok(bytes)
    }
    pub async fn read(&self, asset: &Asset) -> Result<Vec<u8>, String> {
        match asset.provider.as_str() {
            "filesystem" => {
                self.local_read(&asset.key, asset.bytes, &asset.sha256)
                    .await
            }
            "gcs" => self.cloud_read(asset).await,
            _ => Err("Unknown private storage provider".into()),
        }
    }
    async fn sdk(&self, request: Value) -> Result<Value, String> {
        let config = self.gcs.as_ref().ok_or("GCS storage is not configured")?;
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "GCS workers busy; retry shortly")?;
        if !tokio::fs::symlink_metadata(&config.adapter)
            .await
            .is_ok_and(|meta| meta.is_file() && !meta.file_type().is_symlink())
        {
            return Err("Private GCS SDK adapter is unavailable".into());
        }
        let request=json!({"schemaVersion":1,"bucket":config.bucket,"prefix":config.prefix,"root":self.root().await?,"request":request}).to_string();
        if request.len() > 32768 {
            return Err("GCS request exceeds its budget".into());
        }
        let mut command = tokio::process::Command::new("node");
        command.arg(&config.adapter).env_clear();
        for key in [
            "PATH",
            "HOME",
            "TMPDIR",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "GOOGLE_CLOUD_PROJECT",
            "GOOGLE_CLOUD_QUOTA_PROJECT",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|_| "Cannot start private GCS SDK adapter")?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let work = async {
            stdin
                .write_all(request.as_bytes())
                .await
                .map_err(|_| "Cannot submit GCS operation")?;
            drop(stdin);
            let output = async {
                let mut bytes = Vec::new();
                stdout
                    .take(65537)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|_| "Cannot read GCS result")?;
                Ok::<_, &str>(bytes)
            };
            let (bytes, status) = tokio::join!(output, child.wait());
            let bytes = bytes?;
            let status = status.map_err(|_| "Cannot reap GCS SDK adapter")?;
            if bytes.len() > 65536 || !status.success() {
                return Err("GCS operation failed");
            };
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid GCS operation result")?;
            if value["ok"] != true {
                return Err("GCS operation failed");
            }
            Ok(value)
        };
        match tokio::time::timeout(Duration::from_secs(15), work).await {
            Ok(result) => result.map_err(str::to_string),
            Err(_) => {
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
                Err("GCS operation exceeded its deadline; inspect the immutable object identity before retry".into())
            }
        }
    }
    async fn cloud_read(&self, asset: &Asset) -> Result<Vec<u8>, String> {
        if asset.bytes > MAX_FILE || !Self::key(&asset.key) {
            return Err("Invalid cloud image identity".into());
        }
        // Read staging is private and excluded from every cloud object identity.
        let root = self.root().await?;
        let staging = root.join("gcs-read");
        tokio::fs::create_dir_all(&staging)
            .await
            .map_err(|_| "Cannot create cloud read staging")?;
        if tokio::fs::symlink_metadata(&staging)
            .await
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err("Cloud staging refuses symlinks".into());
        }
        let temporary = format!("gcs-read/{}.bin", Uuid::new_v4().simple());
        let target = self.path(&temporary, true).await?;
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        drop(
            options
                .open(&target)
                .await
                .map_err(|_| "Cannot create owned cloud staging file")?,
        );
        let _owned = Temporary(target);
        let result=self.sdk(json!({"op":"read","objects":[{"key":asset.key,"generation":asset.generation,"bytes":asset.bytes,"sha256":asset.sha256,"target":temporary}]})).await;
        let read = if result.is_ok() {
            self.local_read(&temporary, asset.bytes, &asset.sha256)
                .await
        } else {
            Err(result.unwrap_err())
        };
        read
    }
}

fn metadata(alt: &str, caption: &str) -> Result<(), String> {
    if alt.len() > 1024
        || caption.len() > 4096
        || alt.chars().chain(caption.chars()).any(|ch| ch == '\0')
    {
        return Err("Image alt/caption exceed their metadata limits".into());
    }
    Ok(())
}
fn url(id: Uuid, version: i32, name: &str) -> String {
    format!("/_brain/media/{id}/v/{version}/{name}")
}
fn rendition_json(
    id: Uuid,
    version: i32,
    rows: Vec<Value>,
    width: u32,
    height: u32,
) -> (Vec<Value>, Value) {
    let mut formats: BTreeMap<String, BTreeMap<u32, String>> = BTreeMap::new();
    let mut list = Vec::new();
    for mut row in rows {
        let name = row["preset"].as_str().unwrap_or("").to_string();
        let address = url(id, version, &name);
        row["url"] = json!(address);
        let w = row["width"].as_u64().unwrap_or(0) as u32;
        let h = row["height"].as_u64().unwrap_or(0) as u32;
        let difference =
            (i64::from(w) * i64::from(height) - i64::from(h) * i64::from(width)).unsigned_abs();
        if difference <= u64::from(width.max(height)) && name != "thumbnail" {
            formats
                .entry(row["format"].as_str().unwrap_or("").into())
                .or_default()
                .entry(w)
                .or_insert(address);
        }
        list.push(row);
    }
    let srcset = formats
        .into_iter()
        .map(|(format, items)| {
            (
                format,
                Value::String(
                    items
                        .into_iter()
                        .map(|(width, url)| format!("{url} {width}w"))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    (list, Value::Object(srcset))
}
pub async fn library(pool: &PgPool, owner: Uuid, limit: i64, offset: i64) -> Result<Value, String> {
    if !(1..=100).contains(&limit) || !(0..=10000).contains(&offset) {
        return Err("Media pagination requires limit 1..100 and offset 0..10000".into());
    }
    let rows=sqlx::query("SELECT u.id,u.media_type,u.bytes,u.width,u.height,u.alt,u.caption,u.version,COALESCE((SELECT jsonb_agg(jsonb_build_object('preset',r.preset,'format',r.format,'width',r.width,'height',r.height,'bytes',r.bytes,'sha256',r.sha256) ORDER BY r.width,r.preset) FROM _brain_media_renditions r WHERE r.upload_id=u.id AND r.version=u.version),'[]'::jsonb) AS renditions FROM _brain_uploads u WHERE u.user_id=$1 AND u.status='ready' ORDER BY u.created_at DESC,u.id DESC LIMIT $2 OFFSET $3").bind(owner).bind(limit).bind(offset).fetch_all(pool).await.map_err(db::safe)?;
    let mut items = Vec::new();
    for row in rows {
        let id: Uuid = row.get("id");
        let version: i32 = row.get("version");
        let width = row.get::<Option<i32>, _>("width").unwrap_or(0) as u32;
        let height = row.get::<Option<i32>, _>("height").unwrap_or(0) as u32;
        let raw: Value = row.get("renditions");
        let (renditions, srcset) = rendition_json(
            id,
            version,
            raw.as_array().cloned().unwrap_or_default(),
            width,
            height,
        );
        items.push(json!({"id":id,"mediaType":row.get::<String,_>("media_type"),"bytes":row.get::<i64,_>("bytes"),"width":width,"height":height,"alt":row.get::<Option<String>,_>("alt").unwrap_or_default(),"caption":row.get::<Option<String>,_>("caption").unwrap_or_default(),"version":version,"url":url(id,version,"full"),"renditions":renditions,"srcset":srcset}));
    }
    Ok(
        json!({"items":items,"limit":limit,"offset":offset,"nextOffset":if items.len()==limit as usize && offset+limit<=10000 {Some(offset+limit)}else{None}}),
    )
}
struct Object {
    key: String,
    data: Vec<u8>,
    format: String,
    sha256: String,
}
async fn persist(store: &Store, objects: &[Object]) -> Result<BTreeMap<String, String>, String> {
    struct Staging {
        paths: Vec<PathBuf>,
        keep: bool,
    }
    impl Drop for Staging {
        fn drop(&mut self) {
            if !self.keep {
                for path in &self.paths {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
    let mut staging = Staging {
        paths: vec![],
        keep: false,
    };
    let mut unique = BTreeSet::new();
    for object in objects {
        if unique.insert(object.key.clone()) {
            store.write(&object.key, &object.data).await?;
            staging.paths.push(store.path(&object.key, false).await?);
        }
    }
    if store.gcs.is_none() {
        staging.keep = true;
        return Ok(objects
            .iter()
            .map(|object| (object.key.clone(), String::new()))
            .collect());
    }
    let requests=objects.iter().filter(|object|unique.remove(&object.key)).map(|object|json!({"key":object.key,"source":object.key,"bytes":object.data.len(),"sha256":object.sha256,"contentType":mime(&object.format).unwrap()})).collect::<Vec<_>>();
    let result = store.sdk(json!({"op":"write","objects":requests})).await?;
    let mut generations = BTreeMap::new();
    for object in result["objects"]
        .as_array()
        .ok_or("GCS did not report stored object identities")?
    {
        let key = object["key"]
            .as_str()
            .ok_or("Invalid GCS object identity")?;
        let generation = object["generation"]
            .as_str()
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 32
                    && value.bytes().all(|byte| byte.is_ascii_digit())
            })
            .ok_or("Invalid GCS generation")?;
        if !objects.iter().any(|object| object.key == key)
            || generations.insert(key.into(), generation.into()).is_some()
        {
            return Err("Unexpected GCS object identity".into());
        }
    }
    if objects
        .iter()
        .any(|object| !generations.contains_key(&object.key))
    {
        return Err("GCS did not confirm every immutable object".into());
    }
    Ok(generations)
}
/// Record all identities before storage work so failures/crashes leave durable, private cleanup evidence.
pub async fn insert(
    store: &Store,
    pool: &PgPool,
    owner: Uuid,
    prepared: Prepared,
    alt: &str,
    caption: &str,
) -> Result<Value, String> {
    metadata(alt, caption)?;
    let id = Uuid::new_v4();
    let version = 1;
    let original_sha = hash(&prepared.original);
    let date: String = sqlx::query_scalar("SELECT to_char(now() AT TIME ZONE 'UTC','YYYY/MM')")
        .fetch_one(pool)
        .await
        .map_err(db::safe)?;
    let prefix = format!("{owner}/{date}/{id}/v{version}-{original_sha}");
    let original_key = store.object_key(&format!(
        "{prefix}/original-{original_sha}.{}",
        prepared.original_format
    ));
    let mut objects = vec![Object {
        key: original_key.clone(),
        data: prepared.original,
        format: prepared.original_format.clone(),
        sha256: original_sha.clone(),
    }];
    let mut entries = Vec::new();
    for rendition in prepared.renditions {
        let key = store.object_key(&format!(
            "{prefix}/image-{}.{}",
            rendition.sha256, rendition.format
        ));
        entries.push((
            rendition.name,
            rendition.format,
            rendition.width,
            rendition.height,
            rendition.data.len(),
            rendition.sha256.clone(),
            key.clone(),
        ));
        objects.push(Object {
            key,
            data: rendition.data,
            format: entries.last().unwrap().1.clone(),
            sha256: rendition.sha256,
        });
    }
    let full = entries
        .iter()
        .find(|entry| entry.0 == "full")
        .ok_or("Full image rendition is required")?;
    let mut tx = pool.begin().await.map_err(db::safe)?;
    sqlx::query("INSERT INTO _brain_uploads(id,user_id,media_type,bytes,original_key,original_provider,original_bytes,original_mime,original_sha256,width,height,alt,caption,status,version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,'staged',1)").bind(id).bind(owner).bind(mime(&full.1)?).bind(full.4 as i64).bind(&original_key).bind(store.provider()).bind(objects[0].data.len() as i64).bind(mime(&prepared.original_format)?).bind(&original_sha).bind(prepared.width as i32).bind(prepared.height as i32).bind(alt).bind(caption).execute(&mut *tx).await.map_err(db::safe)?;
    for entry in &entries {
        sqlx::query("INSERT INTO _brain_media_renditions(upload_id,version,preset,provider,object_key,format,width,height,bytes,sha256) VALUES ($1,1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(id).bind(&entry.0).bind(store.provider()).bind(&entry.6).bind(&entry.1).bind(entry.2 as i32).bind(entry.3 as i32).bind(entry.4 as i64).bind(&entry.5).execute(&mut *tx).await.map_err(db::safe)?;
    }
    let staging_lease = Uuid::new_v4();
    if store.gcs.is_some() {
        let mut seen = BTreeSet::new();
        for object in &objects {
            if seen.insert(&object.key) {
                sqlx::query("INSERT INTO _brain_media_deletions(upload_id,provider,object_key,lease_until,lease_token) VALUES ($1,'filesystem',$2,now()+interval '1 hour',$3)").bind(id).bind(&object.key).bind(staging_lease).execute(&mut *tx).await.map_err(db::safe)?;
            }
        }
    }
    tx.commit().await.map_err(db::safe)?;
    let generations = match persist(store, &objects).await {
        Ok(value) => value,
        Err(error) => {
            let _ = sqlx::query("DELETE FROM _brain_uploads WHERE id=$1 AND status='staged'")
                .bind(id)
                .execute(pool)
                .await;
            return Err(error);
        }
    };
    let mut tx = pool.begin().await.map_err(db::safe)?;
    let published=sqlx::query("UPDATE _brain_uploads SET status='ready',original_generation=$2 WHERE id=$1 AND status='staged'").bind(id).bind(&generations[&original_key]).execute(&mut *tx).await.map_err(db::safe)?;
    if published.rows_affected() != 1 {
        return Err("Image publication changed; inspect metadata before retrying".into());
    }
    for entry in &entries {
        sqlx::query(
            "UPDATE _brain_media_renditions SET generation=$3 WHERE upload_id=$1 AND object_key=$2",
        )
        .bind(id)
        .bind(&entry.6)
        .bind(&generations[&entry.6])
        .execute(&mut *tx)
        .await
        .map_err(db::safe)?;
    }
    if store.gcs.is_some() {
        sqlx::query("DELETE FROM _brain_media_deletions WHERE upload_id=$1 AND provider='filesystem' AND lease_token=$2").bind(id).bind(staging_lease).execute(&mut *tx).await.map_err(db::safe)?;
    }
    tx.commit().await.map_err(db::safe)?;
    Ok(
        json!({"id":id,"mediaType":mime(&full.1)?,"bytes":full.4,"width":prepared.width,"height":prepared.height,"version":1,"url":url(id,1,"full")}),
    )
}
pub async fn asset(
    pool: &PgPool,
    id: Uuid,
    version: i32,
    preset: &str,
    owner: Uuid,
    admin: bool,
) -> Result<Option<Asset>, String> {
    if version < 1
        || preset.len() > 48
        || !preset.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err("Invalid image version or rendition".into());
    }
    if preset == "original" {
        let row=sqlx::query("SELECT original_provider AS provider,original_key AS object_key,original_generation AS generation,original_mime AS mime,original_bytes AS bytes,original_sha256 AS sha256 FROM _brain_uploads WHERE id=$1 AND status='ready' AND (user_id=$2 OR $3) AND version>=$4 AND original_key IS NOT NULL").bind(id).bind(owner).bind(admin).bind(version).fetch_optional(pool).await.map_err(db::safe)?;
        return row
            .map(|row| {
                let kind: String = row.try_get("mime").map_err(db::safe)?;
                let format = kind
                    .strip_prefix("image/")
                    .ok_or("Invalid original image type")?
                    .to_string();
                mime(&format)?;
                Ok(Asset {
                    provider: row.get("provider"),
                    key: row.get("object_key"),
                    generation: row.get("generation"),
                    format,
                    bytes: row.try_get::<i64, _>("bytes").map_err(db::safe)? as usize,
                    sha256: row.try_get("sha256").map_err(db::safe)?,
                })
            })
            .transpose();
    }
    let row=sqlx::query("SELECT r.provider,r.object_key,r.generation,r.format,r.bytes,r.sha256 FROM _brain_media_renditions r JOIN _brain_uploads u ON u.id=r.upload_id WHERE r.upload_id=$1 AND r.version=$2 AND r.preset=$3 AND u.status='ready' AND r.version<=u.version AND (u.user_id=$4 OR $5)").bind(id).bind(version).bind(preset).bind(owner).bind(admin).fetch_optional(pool).await.map_err(db::safe)?;
    Ok(row.map(|row| Asset {
        provider: row.get("provider"),
        key: row.get("object_key"),
        generation: row.get("generation"),
        format: row.get("format"),
        bytes: row.get::<i64, _>("bytes") as usize,
        sha256: row.get("sha256"),
    }))
}
pub async fn update_metadata(
    pool: &PgPool,
    id: Uuid,
    owner: Uuid,
    admin: bool,
    alt: &str,
    caption: &str,
) -> Result<bool, String> {
    metadata(alt, caption)?;
    sqlx::query("UPDATE _brain_uploads SET alt=$4,caption=$5 WHERE id=$1 AND (user_id=$2 OR $3) AND status='ready'").bind(id).bind(owner).bind(admin).bind(alt).bind(caption).execute(pool).await.map(|result|result.rows_affected()==1).map_err(db::safe)
}
pub async fn delete(pool: &PgPool, id: Uuid, owner: Uuid, admin: bool) -> Result<bool, String> {
    sqlx::query("DELETE FROM _brain_uploads WHERE id=$1 AND (user_id=$2 OR $3)")
        .bind(id)
        .bind(owner)
        .bind(admin)
        .execute(pool)
        .await
        .map(|result| result.rows_affected() == 1)
        .map_err(db::safe)
}
pub async fn cleanup(pool: &PgPool, store: &Store, limit: i64) -> Result<Value, String> {
    if !(1..=8).contains(&limit) {
        return Err("Media cleanup batch must be 1..8".into());
    }
    sqlx::query("DELETE FROM _brain_uploads WHERE id IN (SELECT id FROM _brain_uploads WHERE status='staged' AND created_at<now()-interval '1 hour' ORDER BY created_at LIMIT $1)").bind(limit).execute(pool).await.map_err(db::safe)?;
    let items = db::claim_media_deletions(pool, limit).await?;
    let mut completed = 0;
    let mut failed = 0;
    let mut deferred = 0;
    for item in items {
        if item.provider == "gcs"
            && store.gcs.as_ref().is_none_or(|config| {
                !item
                    .object_key
                    .starts_with(&format!("{}/{}/", config.bucket, config.prefix))
            })
        {
            sqlx::query("UPDATE _brain_media_deletions SET attempts=GREATEST(attempts-1,0),lease_token=NULL,lease_until=now()+interval '5 minutes' WHERE id=$1 AND lease_token=$2").bind(item.id).bind(item.lease).execute(pool).await.map_err(db::safe)?;
            deferred += 1;
            continue;
        }
        let owned = item.object_key == format!("{}.png", item.upload_id)
            || item.object_key == format!("{}.thumb.png", item.upload_id)
            || item
                .object_key
                .split('/')
                .any(|part| part == item.upload_id.to_string());
        if !owned || !Store::key(&item.object_key) {
            failed += 1;
            continue;
        }
        let result=match item.provider.as_str() {
            "filesystem"=>match store.path(&item.object_key,false).await {Ok(path)=>match tokio::fs::remove_file(path).await {Ok(())=>Ok(()),Err(error) if error.kind()==std::io::ErrorKind::NotFound=>Ok(()),Err(_)=>Err("Cannot delete private image".into())},Err(error)=>Err(error)},
            "gcs"=>store.sdk(json!({"op":"delete","objects":[{"key":item.object_key,"generation":item.generation}]})).await.map(|_|()),
            _=>Err("Unknown private image deletion provider".into()),
        };
        if result.is_ok() && db::ack_media_deletion(pool, &item).await? {
            completed += 1
        } else {
            failed += 1
        }
    }
    Ok(json!({"completed":completed,"failed":failed,"deferred":deferred,"limit":limit}))
}

/// Rebuild from the preserved original. Prospective object cleanup is leased before writing; old versions stay live.
pub async fn build_variants(
    store: &Store,
    pool: &PgPool,
    id: Uuid,
    owner: Uuid,
    presets: Vec<Preset>,
) -> Result<Value, String> {
    validate_presets(&presets)?;
    let permit = store.image_slot()?;
    let version: Option<i32> = sqlx::query_scalar(
        "SELECT version FROM _brain_uploads WHERE id=$1 AND user_id=$2 AND status='ready'",
    )
    .bind(id)
    .bind(owner)
    .fetch_optional(pool)
    .await
    .map_err(db::safe)?;
    let current = version.ok_or("Image is unavailable")?;
    if current >= 64 {
        return Err("Image version budget reached; create a new image instead".into());
    }
    let next = current + 1;
    let original = asset(pool, id, current, "original", owner, false)
        .await?
        .ok_or("Legacy image has no preserved original")?;
    let bytes = store.read(&original).await?;
    let (prepared, _permit) = tokio::task::spawn_blocking(move || {
        prepare(bytes, &presets).map(|prepared| (prepared, permit))
    })
    .await
    .map_err(|_| "Image worker failed")??;
    let original_sha = hash(&prepared.original);
    let date: String = sqlx::query_scalar(
        "SELECT to_char(created_at AT TIME ZONE 'UTC','YYYY/MM') FROM _brain_uploads WHERE id=$1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(db::safe)?;
    let prefix = format!("{owner}/{date}/{id}/v{next}-{original_sha}");
    let mut entries = Vec::new();
    let mut objects = Vec::new();
    for rendition in prepared.renditions {
        let key = store.object_key(&format!(
            "{prefix}/image-{}.{}",
            rendition.sha256, rendition.format
        ));
        entries.push((
            rendition.name,
            rendition.format.clone(),
            rendition.width,
            rendition.height,
            rendition.data.len(),
            rendition.sha256.clone(),
            key.clone(),
        ));
        objects.push(Object {
            key,
            data: rendition.data,
            format: rendition.format,
            sha256: rendition.sha256,
        });
    }
    let lease = Uuid::new_v4();
    let mut tx = pool.begin().await.map_err(db::safe)?;
    let live:Option<i32>=sqlx::query_scalar("SELECT version FROM _brain_uploads WHERE id=$1 AND user_id=$2 AND status='ready' FOR UPDATE").bind(id).bind(owner).fetch_optional(&mut *tx).await.map_err(db::safe)?;
    if live != Some(current) {
        return Err("Image changed; reload metadata before rebuilding".into());
    }
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _brain_media_deletions WHERE upload_id=$1 AND completed_at IS NULL)").bind(id).fetch_one(&mut *tx).await.map_err(db::safe)?;
    if pending {
        return Err("Image cleanup is pending; finish the owned cleanup before rebuilding".into());
    }
    let mut unique = BTreeSet::new();
    for object in &objects {
        if unique.insert(&object.key) {
            sqlx::query("INSERT INTO _brain_media_deletions(upload_id,provider,object_key,lease_until,lease_token) VALUES ($1,$2,$3,now()+interval '1 hour',$4) ON CONFLICT(provider,object_key,generation) DO UPDATE SET completed_at=NULL,attempts=0,lease_until=excluded.lease_until,lease_token=excluded.lease_token").bind(id).bind(store.provider()).bind(&object.key).bind(lease).execute(&mut *tx).await.map_err(db::safe)?;
            if store.gcs.is_some() {
                sqlx::query("INSERT INTO _brain_media_deletions(upload_id,provider,object_key,lease_until,lease_token) VALUES ($1,'filesystem',$2,now()+interval '1 hour',$3) ON CONFLICT(provider,object_key,generation) DO UPDATE SET completed_at=NULL,attempts=0,lease_until=excluded.lease_until,lease_token=excluded.lease_token").bind(id).bind(&object.key).bind(lease).execute(&mut *tx).await.map_err(db::safe)?;
            }
        }
    }
    tx.commit().await.map_err(db::safe)?;
    let generations = match persist(store, &objects).await {
        Ok(value) => value,
        Err(error) => {
            let _=sqlx::query("UPDATE _brain_media_deletions SET lease_until=NULL,lease_token=NULL WHERE upload_id=$1 AND lease_token=$2").bind(id).bind(lease).execute(pool).await;
            return Err(error);
        }
    };
    let mut tx = pool.begin().await.map_err(db::safe)?;
    let live:Option<i32>=sqlx::query_scalar("SELECT version FROM _brain_uploads WHERE id=$1 AND user_id=$2 AND status='ready' FOR UPDATE").bind(id).bind(owner).fetch_optional(&mut *tx).await.map_err(db::safe)?;
    let owned:i64=sqlx::query_scalar("SELECT count(*) FROM _brain_media_deletions WHERE upload_id=$1 AND lease_token=$2 AND lease_until>now() AND completed_at IS NULL").bind(id).bind(lease).fetch_one(&mut *tx).await.map_err(db::safe)?;
    if live != Some(current)
        || owned != unique.len() as i64 * if store.gcs.is_some() { 2 } else { 1 }
    {
        return Err("Image rebuild lease changed; inspect metadata before retrying".into());
    }
    for entry in &entries {
        sqlx::query("INSERT INTO _brain_media_renditions(upload_id,version,preset,provider,object_key,generation,format,width,height,bytes,sha256) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(id).bind(next).bind(&entry.0).bind(store.provider()).bind(&entry.6).bind(&generations[&entry.6]).bind(&entry.1).bind(entry.2 as i32).bind(entry.3 as i32).bind(entry.4 as i64).bind(&entry.5).execute(&mut *tx).await.map_err(db::safe)?;
    }
    let full = entries.iter().find(|entry| entry.0 == "full").unwrap();
    sqlx::query("UPDATE _brain_uploads SET version=$2,media_type=$3,bytes=$4 WHERE id=$1")
        .bind(id)
        .bind(next)
        .bind(mime(&full.1)?)
        .bind(full.4 as i64)
        .execute(&mut *tx)
        .await
        .map_err(db::safe)?;
    sqlx::query("DELETE FROM _brain_media_deletions WHERE upload_id=$1 AND lease_token=$2 AND lease_until>now()").bind(id).bind(lease).execute(&mut *tx).await.map_err(db::safe)?;
    tx.commit().await.map_err(db::safe)?;
    Ok(json!({"id":id,"version":next,"url":url(id,next,"full"),"previousVersion":current}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image(width: u32, height: u32) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(width, height)
            .write_to(&mut output, ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }
    #[test]
    fn original_is_preserved_and_renditions_never_upscale_or_exceed_the_plan() {
        let bytes = image(91, 57);
        let prepared = prepare(bytes.clone(), &defaults()).unwrap();
        assert_eq!(prepared.original, bytes);
        assert_eq!(prepared.renditions.len(), 8);
        for rendition in prepared.renditions {
            assert!(rendition.width <= 91 && rendition.height <= 57);
            let decoded = image::load_from_memory(&rendition.data).unwrap();
            assert_eq!(
                (decoded.width(), decoded.height()),
                (rendition.width, rendition.height)
            );
            assert_eq!(hash(&rendition.data), rendition.sha256);
        }
        assert!(prepare(image(20, 20), &vec![defaults()[0].clone(); 7]).is_err());
    }
    #[test]
    fn crops_are_bounded_and_jpeg_composites_transparency_on_white() {
        let preset = Preset {
            name: "square".into(),
            width: 50,
            height: 50,
            crop: true,
            format: "jpeg".into(),
            quality: Some(90),
        };
        let prepared = prepare(image(100, 50), &[preset]).unwrap();
        let square = prepared
            .renditions
            .iter()
            .find(|item| item.name == "square")
            .unwrap();
        assert_eq!((square.width, square.height), (50, 50));
        let decoded = image::load_from_memory(&square.data).unwrap().to_rgb8();
        assert!(decoded.get_pixel(0, 0).0.iter().all(|value| *value >= 250));
        assert!(prepare(b"<svg>active</svg>".to_vec(), &[]).is_err());
        let mut invalid = defaults()[0].clone();
        invalid.name = "../escape".into();
        assert!(prepare(image(5, 5), &[invalid]).is_err());
    }
    #[tokio::test]
    async fn storage_is_exclusive_integrity_checked_and_refuses_symlinks() {
        let path = std::env::temp_dir().join(format!("brain-media-private-{}", Uuid::new_v4()));
        let store = Store::new(path.clone(), None).unwrap();
        let data = image(2, 2);
        let key = "owner/2026/10/image.png";
        store.write(key, &data).await.unwrap();
        assert!(store.write(key, b"replace").await.is_err());
        assert_eq!(
            store
                .local_read(key, data.len(), &hash(&data))
                .await
                .unwrap(),
            data
        );
        assert!(
            store
                .local_read(key, data.len(), &"0".repeat(64))
                .await
                .is_err()
        );
        assert!(store.write("../outside", b"x").await.is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(path.join(key), path.join("link.png")).unwrap();
            assert!(
                store
                    .local_read("link.png", data.len(), &hash(&data))
                    .await
                    .is_err()
            );
        }
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn srcset_has_one_width_descriptor_per_format_and_excludes_cropped_shapes() {
        let id = Uuid::new_v4();
        let (_, sets) = rendition_json(
            id,
            1,
            vec![
                json!({"preset":"a","format":"webp","width":320,"height":160}),
                json!({"preset":"b","format":"webp","width":320,"height":160}),
                json!({"preset":"square","format":"webp","width":300,"height":300}),
            ],
            640,
            320,
        );
        assert_eq!(sets["webp"].as_str().unwrap().matches("320w").count(), 1);
        assert!(!sets["webp"].as_str().unwrap().contains("300w"));
    }
}

/// Offline provisioning material only. The caller applies this reviewable file plan; no package install/cloud call occurs.
pub fn gcs_files() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("server/storage/gcs-adapter.mjs".into(),include_str!("../../../runtime/server/templates/gcs-adapter.mjs").into()),
        ("server/storage/package.json".into(),"{\n  \"private\": true,\n  \"type\": \"module\",\n  \"engines\": { \"node\": \">=22\" },\n  \"dependencies\": { \"@google-cloud/storage\": \"8.4.0\" }\n}\n".into()),
        ("server/storage/README.md".into(),"# Private GCS provider\n\nInstall this private server package explicitly with `npm install --prefix server/storage`. Configure Application Default Credentials outside public/dist and set the backend's private BRAIN_GCS_BUCKET, BRAIN_GCS_PREFIX and BRAIN_GCS_ADAPTER values. The adapter path must be absolute. Use a bucket with enforced public access prevention and uniform bucket-level access. The service identity needs only its intended bucket/prefix object permissions and bucket metadata read access.\n\nThe generated worker does not create buckets, change IAM or request credentials from the browser. No package is bundled into frontend output. Operations use immutable hash keys, conditional creation, pinned generation reads/deletes, checksums and fixed resource limits. `--validate` checks a supplied JSON job offline without importing the Google SDK. No cloud operation happens without explicit provider configuration. See docs/MEDIA.md in the framework repository for the native API and cleanup protocol.\n".into()),
    ])
}
