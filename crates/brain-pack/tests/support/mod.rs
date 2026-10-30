//! Server-evaluation fixtures resolve real runtime modules exactly as the CLI does.
use std::path::{Component, Path};
pub fn runtime_specifier(specifier: &str) -> Result<Option<String>, String> {
    let Some(name) = specifier.strip_prefix("/@brain/") else { return Ok(None) };
    if Path::new(name).components().count() != 1 || !matches!(Path::new(name).components().next(), Some(Component::Normal(_))) || !name.ends_with(".js") {
        return Err("Invalid fixture runtime module".into());
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime").join(name).canonicalize().map_err(|_|"Missing real fixture runtime module")?;
    let text=path.to_string_lossy().replace('\\',"/");
    let mut encoded=String::new();
    if !text.starts_with('/') {encoded.push('/')}
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {encoded.push(byte as char)}
        else {encoded.push_str(&format!("%{byte:02X}"))}
    }
    Ok(Some(format!("file://{encoded}")))
}
