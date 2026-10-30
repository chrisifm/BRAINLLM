//! Private, content-validated image storage. Names supplied by clients are never used as paths.
use image::{ImageFormat, ImageReader, Limits};
use std::{io::Cursor, path::Path};
use uuid::Uuid;

pub struct Prepared {
    pub image: Vec<u8>,
    pub thumbnail: Vec<u8>,
}
pub(crate) fn decode_content(bytes: &[u8]) -> Result<(image::DynamicImage, ImageFormat), String> {
    if bytes.is_empty() || bytes.len() > 5 * 1024 * 1024 {
        return Err("Image must be 1 byte..5 MB".into());
    }
    let format = image::guess_format(&bytes).map_err(|_| "Unsupported image content")?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err("Only PNG, JPEG and WebP image content is accepted".into());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    use image::ImageDecoder;
    let mut decoder = reader.into_decoder().map_err(|_| "Invalid image content")?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || decoder.total_bytes() > 96 * 1024 * 1024
    {
        return Err("Image exceeds 4096x4096 / 96 MB decode budget".into());
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| "Invalid image or image exceeds 4096x4096 / 96 MB decode budget")?;
    img.apply_orientation(orientation);
    Ok((img, format))
}
pub fn prepare(bytes: Vec<u8>) -> Result<Prepared, String> {
    let (img, _) = decode_content(&bytes)?;
    let mut full = Cursor::new(Vec::new());
    img.write_to(&mut full, ImageFormat::Png)
        .map_err(|_| "Cannot encode image")?;
    if full.get_ref().len() > 20 * 1024 * 1024 {
        return Err("Normalized image exceeds 20 MB storage limit".into());
    }
    let mut thumb = Cursor::new(Vec::new());
    img.thumbnail(320.min(img.width()), 320.min(img.height()))
        .write_to(&mut thumb, ImageFormat::Png)
        .map_err(|_| "Cannot encode thumbnail")?;
    // Re-encoding strips active/trailing content and metadata; every stored output has one fixed media type.
    Ok(Prepared {
        image: full.into_inner(),
        thumbnail: thumb.into_inner(),
    })
}
/// Exclusive writes to a private storage root (outside public/dist); best-effort rollback owns only these files.
pub async fn store(root: &Path, id: Uuid, prepared: Prepared) -> Result<u64, String> {
    use tokio::io::AsyncWriteExt;
    for path in [Some(root), root.parent()].into_iter().flatten() {
        if tokio::fs::symlink_metadata(path)
            .await
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err("Private upload storage refuses symlink roots".into());
        }
    }
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|_| "Cannot create private upload storage")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|_| "Cannot restrict private upload storage")?;
    }
    let paths = [
        root.join(format!("{id}.png")),
        root.join(format!("{id}.thumb.png")),
    ];
    let bytes = prepared.image.len() as u64;
    let mut owned = Vec::new();
    for (path, data) in paths.iter().zip([prepared.image, prepared.thumbnail]) {
        let result = async {
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut f = options
                .open(path)
                .await
                .map_err(|_| "Cannot create private upload")?;
            owned.push(path.clone());
            f.write_all(&data)
                .await
                .map_err(|_| "Cannot write private upload")?;
            f.sync_all()
                .await
                .map_err(|_| "Cannot persist private upload")
        }
        .await;
        if let Err(e) = result {
            for p in owned {
                let _ = tokio::fs::remove_file(p).await;
            }
            return Err(e.into());
        }
    }
    Ok(bytes)
}
pub async fn remove(root: &Path, id: Uuid) {
    for suffix in [".png", ".thumb.png"] {
        let _ = tokio::fs::remove_file(root.join(format!("{id}{suffix}"))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_validation_reencodes_images_and_rejects_non_images() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(640, 480)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let result = prepare(png.into_inner()).unwrap();
        let thumb = image::load_from_memory(&result.thumbnail).unwrap();
        assert!(thumb.width() <= 320 && thumb.height() <= 320);
        assert_eq!(
            image::guess_format(&result.image).unwrap(),
            ImageFormat::Png
        );
        assert!(prepare(b"<svg><script>active content</script></svg>".to_vec()).is_err());
        assert!(prepare(vec![0; 5 * 1024 * 1024 + 1]).is_err());
    }
}
