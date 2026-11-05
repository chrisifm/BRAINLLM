//! Static export destinations. Resolve before rewriting references and computing content hashes.
use brain_mcp::Json;
use std::{fs, io::Read, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AssetLayout {
    pub js: String,
    pub css: String,
    pub images: String,
    pub assets: String,
    pub fonts: Option<String>,
    pub precompressed: bool,
    pub host_files: bool,
    pub github_pages: bool,
    pub compiler_branding: bool,
    pub inline_css: bool,
    pub fast_compression: bool,
}
impl Default for AssetLayout {
    fn default() -> Self { Self { js: "_b".into(), css: "_b".into(), images: "_b".into(), assets: "_b".into(), fonts: None, precompressed: true, host_files: true, github_pages: false, compiler_branding: true, inline_css: true, fast_compression: false } }
}
impl AssetLayout {
    pub fn load(root: &Path, static_export: bool) -> Result<Self, String> {
        let path = root.join("brain.json");
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(_) => return Err("cannot read brain.json output layout".into()),
        };
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 2 * 1024 * 1024 { return Err("brain.json output layout requires a regular file of at most 2 MiB".into()); }
        let mut source = String::new();
        fs::File::open(path).map_err(|_| "cannot open brain.json output layout")?.take(2 * 1024 * 1024 + 1).read_to_string(&mut source).map_err(|_| "cannot read brain.json output layout")?;
        if source.len() > 2 * 1024 * 1024 { return Err("brain.json exceeds the output-layout read limit".into()); }
        let config = Json::parse(&source).map_err(|_| "brain.json output layout requires valid JSON")?;
        let compiler_branding=brain_lex::branding::enabled(&config)?;
        let mut out=Self{compiler_branding,..Self::default()};
        let Json::Obj(config) = config else { return Err("brain.json must be an object".into()); };
        let found: Vec<_> = config.iter().filter(|(name, _)| name == "staticOutput").collect();
        if found.len() > 1 { return Err("brain.json must declare staticOutput once".into()); }
        let Some((_, layout)) = found.first() else { return Ok(out); };
        if !static_export { return Err("staticOutput requires static export; fix: set output to static or build with --static".into()); }
        let Json::Obj(fields) = layout else { return Err("staticOutput must be an object with js, css, images and/or assets directories".into()); };
        if let Some((_,host))=fields.iter().find(|(name,_)|name=="host") {
            match host.as_str() {
                Some("generic")=>{},
                Some("github-pages")=>{out.github_pages=true;out.host_files=false;},
                _=>return Err("staticOutput.host must be generic or github-pages".into()),
            }
        }
        let mut seen = std::collections::HashSet::new();
        for (name, value) in fields {
            if !seen.insert(name) { return Err(format!("staticOutput.{name} is declared twice")); }
            if name == "host" {continue}
            if name == "compression" {
                out.fast_compression = match value.as_str() {
                    Some("small") => false,
                    Some("fast") => true,
                    _ => return Err("staticOutput.compression must be small or fast".into()),
                };
                continue;
            }
            if name == "precompressed" || name == "hostFiles" {
                let Json::Bool(value) = value else { return Err(format!("staticOutput.{name} must be true or false")); };
                if name == "precompressed" { out.precompressed = *value; } else { out.host_files = *value; }
                continue;
            }
            if name == "inlineCss" {
                let Json::Bool(value) = value else { return Err("staticOutput.inlineCss must be true or false".into()); };
                out.inline_css = *value;
                continue;
            }
            if name == "fonts" {
                out.fonts = Some(directory(value.as_str().ok_or("staticOutput directories must be strings")?)?);
                continue;
            }
            let slot = match name.as_str() { "js" => &mut out.js, "css" => &mut out.css, "images" => &mut out.images, "assets" => &mut out.assets, _ => return Err("staticOutput supports only js, css, images, fonts, assets, inlineCss, compression, precompressed, hostFiles and host".into()) };
            let value = value.as_str().ok_or("staticOutput directories must be strings")?;
            *slot = directory(value)?;
        }
        Ok(out)
    }
    pub fn post_key(&self, path: &str) -> Option<String> {
        if let Some(name) = path.strip_prefix("_b/") { return Some(self.asset(name)); }
        if self.images != "_b" && path.starts_with("og/") { return Some(inside(&self.images, path)); }
        None
    }
    pub fn relocation_urls(&self, old: &str, new: &str, pre: &str, site: Option<&str>) -> Vec<(String, String)> {
        fn url(path: &str) -> String {
            let mut out = String::from("/");
            for byte in path.bytes() {
                if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') { out.push(byte as char); }
                else { out.push_str(&format!("%{byte:02X}")); }
            }
            out
        }
        let (old, new) = (url(old), url(new));
        let mut pairs = vec![(old.clone(), new.clone()), (format!("{pre}{old}"), format!("{pre}{new}"))];
        if let Some(site) = site {
            pairs.push((format!("{}{old}", site.trim_end_matches('/')), format!("{}{new}", site.trim_end_matches('/'))));
            if let Ok(site) = url::Url::parse(site) {
                let origin = site.origin().ascii_serialization();
                pairs.push((format!("{origin}{pre}{old}"), format!("{origin}{pre}{new}")));
            }
        }
        pairs
    }
    pub fn custom(&self) -> bool { [&self.js, &self.css, &self.images, &self.assets].iter().any(|dir| dir.as_str() != "_b") || self.fonts.as_deref().is_some_and(|dir| dir != "_b") }
    pub fn js(&self, name: &str) -> String { inside(&self.js, name) }
    pub fn css(&self, name: &str) -> String { inside(&self.css, name) }
    pub fn asset(&self, name: &str) -> String {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let dir = match ext.as_str() {
            "js" | "mjs" => &self.js,
            "css" => &self.css,
            "png" | "jpg" | "jpeg" | "webp" | "avif" | "svg" | "gif" | "ico" | "bmp" | "tif" | "tiff" => &self.images,
            "woff" | "woff2" | "ttf" | "otf" | "eot" => self.fonts.as_deref().unwrap_or(&self.assets),
            _ => &self.assets,
        };
        inside(dir, name)
    }
}
fn inside(dir: &str, name: &str) -> String { if dir.is_empty() { name.into() } else { format!("{dir}/{name}") } }
fn directory(value: &str) -> Result<String, String> {
    if value == "." { return Ok(String::new()); }
    if value.is_empty() || value.len() > 192 || value.split('/').count() > 8 || value.split('/').any(|segment| segment.is_empty() || segment.len() > 64 || segment.starts_with('.') || !segment.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))) {
        return Err("staticOutput directories must be safe relative URL paths (letters, digits, -, _, .); use . for the export root, without empty/dot segments or traversal".into());
    }
    let first = value.split('/').next().unwrap_or("").to_ascii_lowercase();
    if matches!(first.as_str(), "_server" | "_brain" | "_r" | "api" | "node_modules" | "dist") { return Err("staticOutput directory uses a reserved framework path".into()); }
    Ok(value.into())
}
#[cfg(test)]
mod tests {
    #[test]
    fn github_pages_defaults_are_order_independent_and_explicit_overrides_win() {
        let root=std::env::temp_dir().join(format!("brain-pages-layout-{}",std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for source in [r#"{"staticOutput":{"host":"github-pages"}}"#,r#"{"staticOutput":{"css":"css","host":"github-pages"}}"#] {
            std::fs::write(root.join("brain.json"),source).unwrap();let layout=super::AssetLayout::load(&root,true).unwrap();
            assert!(layout.github_pages);assert!(!layout.host_files);assert!(layout.precompressed);
            assert!(super::AssetLayout::load(&root,false).is_err());
        }
        for source in [r#"{"staticOutput":{"hostFiles":true,"precompressed":true,"host":"github-pages"}}"#,r#"{"staticOutput":{"host":"github-pages","precompressed":true,"hostFiles":true}}"#] {
            std::fs::write(root.join("brain.json"),source).unwrap();let layout=super::AssetLayout::load(&root,true).unwrap();assert!(layout.host_files&&layout.precompressed);
        }
        for source in [r#"{"staticOutput":{"host":false}}"#,r#"{"staticOutput":{"host":"unknown"}}"#,r#"{"staticOutput":{"host":"github-pages","host":"generic"}}"#] {
            std::fs::write(root.join("brain.json"),source).unwrap();assert!(super::AssetLayout::load(&root,true).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn output_directories_are_bounded_and_confined() {
        for bad in ["", "/js", "../js", "src/../js", "src//js", "src/", "src/./js", "C:js", "src\\js", "src?x", "src#x", "_server/code", "API/code", "src/%2e%2e", "src/white space"] { assert!(super::directory(bad).is_err(), "{bad}"); }
        assert_eq!(super::directory("src/js.v1").unwrap(), "src/js.v1");
        assert_eq!(super::directory(".").unwrap(), "");
    }
}
