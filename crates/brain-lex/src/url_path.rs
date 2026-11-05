//! Generated URL serialization and strict URL-to-filesystem conversion for static HTML and SEO exports.

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

/// Resolve generated image metadata against the configured application root and canonical page.
/// Root-relative references are app-relative; existing base prefixes and percent escapes are not duplicated.
pub fn metadata_image(root:Option<&str>, canonical:&str, value:&str)->String {
    fn encode(value:&str)->String {
        value.bytes().map(|b|match b {
            b'A'..=b'Z'|b'a'..=b'z'|b'0'..=b'9'|b'-'|b'_'|b'.'|b'~'|b':'|b'/'|b'?'|b'#'|b'['|b']'|b'@'|b'!'|b'$'|b'&'|b'\''|b'('|b')'|b'*'|b'+'|b','|b';'|b'='|b'%' => (b as char).to_string(),
            _=>format!("%{b:02X}"),
        }).collect()
    }
    let value=value.trim();
    if value.is_empty() {return String::new()}
    let scheme=value.split_once(':').is_some_and(|(s,_)|s.starts_with(|c:char|c.is_ascii_alphabetic()) && s.bytes().all(|b|b.is_ascii_alphanumeric() || matches!(b,b'+'|b'-'|b'.')));
    if scheme {return encode(value)}
    let Some(root)=root.filter(|r|r.starts_with("https://") || r.starts_with("http://")) else{return encode(value)};
    let root=root.trim_end_matches('/');
    let authority=root.find("://").unwrap()+3;
    let origin_end=root[authority..].find('/').map_or(root.len(),|at|authority+at);
    let origin=&root[..origin_end];let base=&root[origin_end..];
    let joined=if value.starts_with("//") {format!("{}:{value}",root.split(':').next().unwrap())}
        else if value.starts_with('/') {if !base.is_empty() && (value==base || value.strip_prefix(base).is_some_and(|rest|rest.starts_with('/'))) {format!("{origin}{value}")} else {format!("{root}{value}")}}
        else {
            let page=if canonical.starts_with("https://") || canonical.starts_with("http://") {canonical.to_string()}else{format!("{root}/")};
            let page=page.split(['?','#']).next().unwrap();
            let page=if !page[page.find("://").unwrap()+3..].contains('/') {format!("{page}/")}else{page.to_string()};
            let page=page.as_str();
            if value.starts_with(['?','#']) {format!("{page}{value}")} else {format!("{}{value}",page.rsplit_once('/').map_or(page,|(dir,_)|dir).to_string()+"/")}
        };
    let authority=joined.find("://").unwrap()+3;
    let Some(at)=joined[authority..].find('/') else{return encode(&joined)};
    let at=authority+at;let (path,suffix)=joined[at..].find(['?','#']).map_or((&joined[at..],""),|end|(&joined[at..at+end],&joined[at+end..]));
    let mut parts=Vec::new();let mut trailing=path.ends_with('/');
    for part in path.split('/').skip(1) {
        let dots=part.to_ascii_lowercase().replace("%2e", ".");
        if dots=="." {trailing=true;} else if dots==".." {parts.pop();trailing=true;} else {parts.push(part);trailing=false;}
    }
    let mut path=format!("/{}",parts.join("/"));if trailing && !path.ends_with('/') {path.push('/');}
    encode(&format!("{}{path}{suffix}",&joined[..at]))
}

#[cfg(test)]
mod metadata_tests {
    #[test]
    fn image_urls_follow_the_shared_metadata_vectors() {
        let crate::Json::Arr(rows)=crate::Json::parse(include_str!("../tests/metadata-image.json")).unwrap() else{panic!("metadata vectors")};
        for row in rows {
            let crate::Json::Arr(row)=row else{panic!("metadata vector")};
            assert_eq!(super::metadata_image(row[0].as_str(),row[1].as_str().unwrap(),row[2].as_str().unwrap()),row[3].as_str().unwrap(),"{row:?}");
        }
    }
}

/// Canonical slash form shared by packaging and SEO (root and file-like paths keep their special form).
pub fn page_path(path:&str, always:bool)->String {
    let cut=path.find(['?','#']).unwrap_or(path.len());let (p,rest)=path.split_at(cut);
    if p.is_empty() || p=="/" || !p.starts_with('/') {return path.to_string()}
    let bare=p.trim_end_matches('/');let bare=if bare.is_empty(){"/"}else{bare};
    let file=bare.rsplit('/').next().is_some_and(|last|last.contains('.'));
    if always && !file && bare!="/" {format!("{bare}/{rest}")}else if always{format!("{p}{rest}")}else{format!("{bare}{rest}")}
}
