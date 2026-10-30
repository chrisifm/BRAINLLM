//! Brotli is generated once at build time, never on a static request. Cache keys include codec version and settings.
pub fn compress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut out = Vec::new();
    {
        let mut writer = ::brotli::CompressorWriter::new(&mut out, 65536, 11, 22);
        writer
            .write_all(bytes)
            .map_err(|_| "Brotli compression failed")?;
        writer.flush().map_err(|_| "Brotli compression failed")?;
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_is_deterministic_and_smaller_for_static_text() {
        use std::io::Read;
        let src = "export const title = 'An immutable content-hashed page';\n".repeat(100);
        let first = compress(src.as_bytes()).unwrap();
        assert_eq!(first, compress(src.as_bytes()).unwrap());
        assert!(first.len() < src.len());
        let mut restored = String::new();
        ::brotli::Decompressor::new(first.as_slice(), 65536)
            .read_to_string(&mut restored)
            .unwrap();
        assert_eq!(restored, src);
    }
}
