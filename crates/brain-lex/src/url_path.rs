//! Strict URL-to-filesystem conversion shared by static HTML and SEO exports.

/// Encode a filesystem path as a URL path, retaining directory separators and encoding literal percent signs.
pub fn encode_path(path: &str) -> String {
    let mut out=String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {out.push(byte as char);}
        else {out.push_str(&format!("%{byte:02X}"));}
    }
    out
}

/// URL and disk paths are different: an ordinary HTTP host decodes each requested segment once.
/// Keep the encoded URL for SEO/navigation; use decoded UTF-8 for physical filenames.
pub fn decode_relative(path: &str) -> Result<String, String> {
    let error = || "unsafe static URL path; fix: use valid UTF-8 URL segments without dot-segments, encoded separators or control characters".to_string();
    let hex = |b: u8| match b { b'0'..=b'9' => Some(b - b'0'), b'a'..=b'f' => Some(b - b'a' + 10), b'A'..=b'F' => Some(b - b'A' + 10), _ => None };
    let mut segments = Vec::new();
    for segment in path.split('/') {
        let input = segment.as_bytes();
        let mut bytes = Vec::new();
        let mut i = 0;
        while i < input.len() {
            if input[i] == b'%' {
                if i + 2 >= input.len() { return Err(error()); }
                let hi = hex(input[i + 1]).ok_or_else(&error)?;
                let lo = hex(input[i + 2]).ok_or_else(&error)?;
                bytes.push(hi * 16 + lo);
                i += 3;
            } else { bytes.push(input[i]); i += 1; }
        }
        let decoded = String::from_utf8(bytes).map_err(|_| error())?;
        if decoded.is_empty() || decoded == "." || decoded == ".." || decoded.contains(['/', '\\', ':']) || decoded.chars().any(char::is_control) {
            return Err(error());
        }
        segments.push(decoded);
    }
    let result = segments.join("/");
    Ok(result)
}
