//! Offline browser ESM resolution. Exact installed manifests and bytes, never registry/install/build network.
use brain_mcp::Json;
use brain_min::lex::{Kind, lex};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};
mod vendor;
pub use vendor::{VENDOR_LOCK, artifact_file, pinned_cdn_url, standalone_esm};
const FILE_BYTES: usize = 1024 * 1024;
const MANIFEST_BYTES: usize = 262144;
const TOTAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILES: usize = 512;
const ASSET_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone)]
struct Package {
    dir: String,
    name: String,
    version: String,
    json: Json,
    revision: String,
    side_effects: Json,
}
pub struct Resolver {
    root: PathBuf,
    packages: BTreeMap<String, Package>,
    files: BTreeMap<String, (String, usize)>,
    bytes: usize,
    start: Instant,
    lock: Option<String>,
    lock_file: Option<String>,
    resolutions: usize,
    vendor: Option<Json>,
    vendor_revision: Option<String>,
}
pub fn is_foreign(file: &str) -> bool {
    file.split('/').any(|part| part == "node_modules") || vendor::is_vendor(file)
}
fn hash(bytes: &[u8]) -> String {
    crate::sha::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn error(message: &str) -> String {
    format!(
        "Browser dependency: {message}; use installed official ESM or an explicit reviewed adapter (no automatic CJS/TS/JSX transpilation)."
    )
}
fn normalized(path: &Path) -> Result<String, String> {
    let mut parts = vec![];
    for p in path.components() {
        match p {
            Component::Normal(s) => {
                let s = s
                    .to_str()
                    .ok_or_else(|| error("dependency path must be UTF-8"))?;
                if s.starts_with('.')
                    || s.contains(['\\', ':', '?', '#', '%'])
                    || s.contains('"')
                    || s.contains('\'')
                    || s.chars().any(char::is_control)
                    || ["credentials", "secrets"].contains(&s)
                {
                    return Err(error(
                        "dependency path contains a private or unsupported segment",
                    ));
                }
                parts.push(s.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                parts
                    .pop()
                    .ok_or_else(|| error("dependency leaves app root"))?;
            }
            _ => return Err(error("dependency path must be app-relative")),
        }
    }
    if parts.len() > 64 || parts.is_empty() {
        return Err(error("dependency path depth is unsupported"));
    }
    Ok(parts.join("/"))
}
fn package_name(spec: &str) -> Result<(&str, &str), String> {
    if spec.is_empty()
        || spec.len() > 240
        || spec.contains(['\\', ':', '?', '#', '%'])
        || spec
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && !b"@/._-".contains(&b))
        || spec
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(error(
            "unsupported package specifier; remote URLs require explicit vendoring",
        ));
    }
    let end = if spec.starts_with('@') {
        let slash = spec
            .find('/')
            .ok_or_else(|| error("scoped package needs a name"))?;
        spec[slash + 1..]
            .find('/')
            .map_or(spec.len(), |i| slash + 1 + i)
    } else {
        spec.find('/').unwrap_or(spec.len())
    };
    let name = &spec[..end];
    if name.starts_with('.') || name.ends_with('/') || name.contains("@@") {
        return Err(error("invalid package name"));
    }
    Ok((name, spec.get(end + 1..).unwrap_or("")))
}
fn exact_version(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
    {
        return false;
    }
    let core = value.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}
fn condition(value: &Json, star: Option<&str>) -> Result<String, String> {
    match value {
        Json::Str(s) => Ok(if let Some(star) = star {
            s.replace('*', star)
        } else {
            s.clone()
        }),
        Json::Obj(entries) => {
            for (k, v) in entries {
                if ["browser", "import", "default"].contains(&k.as_str()) {
                    match condition(v, star) {
                        Err(e) if e.contains("no active browser/import/default") => continue,
                        result => return result,
                    }
                }
            }
            Err(error("no active browser/import/default export condition"))
        }
        Json::Null | Json::Bool(false) => Err(error("package explicitly blocks this export")),
        _ => Err(error("unsupported export target or fallback array")),
    }
}
fn mapping(map: &Json, key: &str) -> Result<String, String> {
    if let Json::Obj(entries) = map {
        if entries
            .iter()
            .any(|(k, _)| k.starts_with('.') || k.starts_with('#'))
        {
            if let Some((_, v)) = entries.iter().find(|(k, _)| k == key) {
                return condition(v, None);
            }
            let mut matches = vec![];
            for (k, v) in entries {
                if k.matches('*').count() != 1 {
                    continue;
                }
                let (a, b) = k.split_once('*').unwrap();
                if key.starts_with(a) && key.ends_with(b) && key.len() >= a.len() + b.len() {
                    matches.push((a.len(), b.len(), v, &key[a.len()..key.len() - b.len()]));
                }
            }
            matches.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
            if let Some((_, _, v, star)) = matches.first() {
                return condition(v, Some(star));
            }
            return Err(error("package subpath is not exported"));
        }
    }
    if key != "." {
        return Err(error("package exposes only its root export"));
    }
    condition(map, None)
}
fn unique_json(value: &Json) -> bool {
    match value {
        Json::Obj(entries) => {
            let mut seen = HashSet::new();
            entries
                .iter()
                .all(|(key, value)| seen.insert(key) && unique_json(value))
        }
        Json::Arr(values) => values.iter().all(unique_json),
        _ => true,
    }
}
fn side_effects(value: Option<&Json>) -> Json {
    match value {
        Some(Json::Bool(value)) => Json::Bool(*value),
        Some(Json::Arr(paths))
            if paths.len() <= 64
                && paths.iter().all(|p| {
                    p.as_str().is_some_and(|s| {
                        s.len() <= 160
                            && s.bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"*/._-@!".contains(&b))
                    })
                }) =>
        {
            Json::Arr(paths.clone())
        }
        _ => Json::str("unknown"),
    }
}
impl Resolver {
    pub fn new(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|_| error("app root unavailable"))?;
        if !root.is_dir() {
            return Err(error("app root must be a directory"));
        }
        let this = Self {
            root,
            packages: BTreeMap::new(),
            files: BTreeMap::new(),
            bytes: 0,
            start: Instant::now(),
            lock: None,
            lock_file: None,
            resolutions: 0,
            vendor: None,
            vendor_revision: None,
        };
        Ok(this)
    }
    fn deadline(&self) -> Result<(), String> {
        if self.start.elapsed() > Duration::from_secs(30) {
            Err(error("dependency scan exceeded 30 seconds"))
        } else {
            Ok(())
        }
    }
    fn read_bytes(&mut self, file: &str, limit: usize) -> Result<Vec<u8>, String> {
        self.deadline()?;
        let file = normalized(Path::new(file))?;
        let mut path = self.root.clone();
        for part in file.split('/') {
            path.push(part);
            let meta = fs::symlink_metadata(&path)
                .map_err(|_| error("installed dependency file is missing"))?;
            if meta.file_type().is_symlink() {
                return Err(error(
                    "symlinked dependency path refused; install a regular contained package tree",
                ));
            }
        }
        let meta =
            fs::symlink_metadata(&path).map_err(|_| error("dependency metadata unavailable"))?;
        if !meta.is_file() || meta.len() > limit as u64 {
            return Err(error(
                "dependency file is not regular or exceeds its byte limit",
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let opened = options
            .open(&path)
            .map_err(|_| error("cannot safely open dependency file"))?;
        if !opened.metadata().is_ok_and(|m| m.is_file())
            || !path
                .canonicalize()
                .map_err(|_| error("dependency path changed"))?
                .starts_with(&self.root)
        {
            return Err(error("dependency escaped app root"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let actual = opened
                .metadata()
                .map_err(|_| error("opened dependency metadata unavailable"))?;
            if actual.dev() != meta.dev() || actual.ino() != meta.ino() {
                return Err(error(
                    "dependency changed while opening; retry from a stable install",
                ));
            }
        }
        let mut bytes = vec![];
        opened
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("cannot read dependency file"))?;
        if bytes.len() > limit {
            return Err(error("dependency read exceeds byte limit"));
        }
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|n| *n <= TOTAL_BYTES)
            .ok_or_else(|| error("dependency graph exceeds 32 MiB read budget"))?;
        Ok(bytes)
    }
    fn capture_lock(&mut self) -> Result<(), String> {
        for file in ["npm-shrinkwrap.json", "package-lock.json"] {
            if self
                .root
                .join(file)
                .try_exists()
                .map_err(|_| error("cannot inspect lockfile"))?
            {
                let bytes = self.read_bytes(file, FILE_BYTES)?;
                Json::parse(
                    std::str::from_utf8(&bytes).map_err(|_| error("lockfile must be UTF-8"))?,
                )
                .map_err(|_| error("lockfile is invalid JSON"))?;
                self.lock = Some(hash(&bytes));
                self.lock_file = Some(file.to_owned());
                break;
            }
        }
        Ok(())
    }
    fn package(&mut self, dir: &str) -> Result<Package, String> {
        if let Some(p) = self.packages.get(dir) {
            return Ok(p.clone());
        }
        if self.packages.is_empty() {
            self.capture_lock()?;
        }
        if self.packages.len() >= 128 {
            return Err(error("dependency graph exceeds 128 package manifests"));
        }
        let bytes = self.read_bytes(&format!("{dir}/package.json"), MANIFEST_BYTES)?;
        let src =
            std::str::from_utf8(&bytes).map_err(|_| error("package manifest must be UTF-8"))?;
        let json = Json::parse(src).map_err(|_| error("package manifest must be valid JSON"))?;
        if !matches!(json, Json::Obj(_)) || !unique_json(&json) {
            return Err(error(
                "package manifest must be an object with unique fields",
            ));
        }
        let side_effects = side_effects(json.get("sideEffects"));
        let name = json
            .get("name")
            .and_then(Json::as_str)
            .filter(|s| s.len() <= 160)
            .ok_or_else(|| error("package manifest needs a bounded name"))?
            .to_owned();
        package_name(&name)?;
        let version = json
            .get("version")
            .and_then(Json::as_str)
            .filter(|s| exact_version(s))
            .ok_or_else(|| error("package manifest needs an explicit version"))?
            .to_owned();
        let package = Package {
            dir: dir.to_owned(),
            name,
            version,
            json,
            revision: hash(&bytes),
            side_effects,
        };
        self.packages.insert(dir.to_owned(), package.clone());
        Ok(package)
    }
    fn containing(&mut self, from: &str) -> Result<Option<Package>, String> {
        let parts: Vec<_> = from.split('/').collect();
        let Some(at) = parts.iter().rposition(|p| *p == "node_modules") else {
            return Ok(None);
        };
        let count = if parts.get(at + 1).is_some_and(|s| s.starts_with('@')) {
            3
        } else {
            2
        };
        if at + count > parts.len() {
            return Err(error("invalid installed package path"));
        }
        self.package(&parts[..at + count].join("/")).map(Some)
    }
    fn target(&mut self, p: &Package, target: &str) -> Result<String, String> {
        let target = target
            .strip_prefix("./")
            .ok_or_else(|| error("package export must be a contained relative target"))?;
        if target
            .split('/')
            .any(|s| s == "node_modules" || s == ".." || s.is_empty())
        {
            return Err(error("package export target escapes its package"));
        }
        let file = normalized(&Path::new(&p.dir).join(target))?;
        if !file.starts_with(&format!("{}/", p.dir))
            || !(file.ends_with(".js") || file.ends_with(".mjs"))
        {
            return Err(error(
                "only explicit .js/.mjs browser ESM exports are supported",
            ));
        }
        Ok(file)
    }
    /// None leaves framework runtime/app-relative resolution to the caller. Bare packages are never ignored.
    pub fn resolve(&mut self, from: &str, spec: &str) -> Result<Option<String>, String> {
        self.resolve_depth(from, spec, 0)
    }
    fn resolve_depth(
        &mut self,
        from: &str,
        spec: &str,
        depth: usize,
    ) -> Result<Option<String>, String> {
        if spec.starts_with("https://") {
            return self.resolve_vendor(spec).map(Some);
        }
        if spec.starts_with("/@brain/") && is_foreign(from) {
            return Err(error(
                "foreign SDK must not import compiler-owned runtime aliases",
            ));
        }
        if spec.starts_with("/@brain/")
            || ((spec.starts_with("./") || spec.starts_with("../")) && !is_foreign(from))
        {
            return Ok(None);
        }
        self.resolutions += 1;
        if self.resolutions > 4096 || spec.len() > 512 {
            return Err(error(
                "dependency graph exceeds 4096 references or a bounded specifier",
            ));
        }
        if self.packages.is_empty() && self.bytes == 0 {
            self.start = Instant::now();
        }
        self.deadline()?;
        if depth > 16 {
            return Err(error("package alias resolution exceeds 16 steps"));
        }
        if spec.starts_with("/@brain/") {
            return Ok(None);
        }
        let owner = self.containing(from)?;
        if let Some(p) = &owner {
            if let Some(Json::Obj(map)) = p.json.get("browser") {
                let relative = if spec.starts_with('.') {
                    let resolved =
                        normalized(&Path::new(from).parent().unwrap_or(Path::new("")).join(spec))?;
                    resolved
                        .strip_prefix(&format!("{}/", p.dir))
                        .map(|r| format!("./{r}"))
                } else {
                    None
                };
                if let Some((_, value)) = map
                    .iter()
                    .find(|(k, _)| k == spec || relative.as_ref() == Some(k))
                {
                    let target = condition(value, None)?;
                    if target.starts_with('.') {
                        return self.target(p, &target).map(Some);
                    }
                    return self.resolve_depth(from, &target, depth + 1);
                }
            }
        }
        if spec.starts_with('#') {
            let p = owner.ok_or_else(|| {
                error("package imports aliases require an installed package owner")
            })?;
            let imports = p
                .json
                .get("imports")
                .ok_or_else(|| error("package has no imports map"))?;
            let target = mapping(imports, spec)?;
            if target.starts_with('.') {
                return self.target(&p, &target).map(Some);
            }
            return self.resolve_depth(from, &target, depth + 1);
        }
        if spec.starts_with("./") || spec.starts_with("../") {
            let Some(p) = owner else { return Ok(None) };
            let file = normalized(&Path::new(from).parent().unwrap_or(Path::new("")).join(spec))?;
            if !file.starts_with(&format!("{}/", p.dir))
                || !(file.ends_with(".js") || file.ends_with(".mjs"))
            {
                return Err(error(
                    "relative ESM dependency must remain inside its package and name .js/.mjs explicitly",
                ));
            }
            return Ok(Some(file));
        }
        let (name, sub) = package_name(spec)?;
        let mut directory = Path::new(from)
            .parent()
            .unwrap_or(Path::new(""))
            .to_path_buf();
        let mut chosen = None;
        loop {
            let candidate = directory.join("node_modules").join(name);
            let rel = normalized(&candidate)?;
            if self
                .root
                .join(&rel)
                .try_exists()
                .map_err(|_| error("cannot inspect installed package"))?
            {
                chosen = Some(rel);
                break;
            }
            if !directory.pop() {
                break;
            }
        }
        let dir=chosen.ok_or_else(||error("package is not installed in the app dependency tree; preview and explicitly install its exact pin first"))?;
        let p = self.package(&dir)?;
        if p.name != name {
            return Err(error(
                "installed package name differs from its requested package",
            ));
        }
        let key = if sub.is_empty() {
            ".".to_owned()
        } else {
            format!("./{sub}")
        };
        let target = if let Some(exports) = p.json.get("exports") {
            mapping(exports, &key)?
        } else if !sub.is_empty() {
            format!("./{sub}")
        } else if let Some(browser) = p.json.get("browser").and_then(Json::as_str) {
            if browser.starts_with("./") {
                browser.to_owned()
            } else {
                format!("./{browser}")
            }
        } else if let Some(module) = p.json.get("module").and_then(Json::as_str) {
            if module.starts_with("./") {
                module.to_owned()
            } else {
                format!("./{module}")
            }
        } else if p.json.get("type").and_then(Json::as_str) == Some("module") {
            p.json
                .get("main")
                .and_then(Json::as_str)
                .map(|s| {
                    if s.starts_with("./") {
                        s.to_owned()
                    } else {
                        format!("./{s}")
                    }
                })
                .unwrap_or("./index.js".into())
        } else {
            return Err(error(
                "package has no explicit browser/import ESM entry; CommonJS fallback is refused",
            ));
        };
        if let Some(Json::Obj(map)) = p.json.get("browser") {
            if let Some((_, value)) = map.iter().find(|(key, _)| key == &target) {
                let replacement = condition(value, None)?;
                if replacement.starts_with('.') {
                    return self.target(&p, &replacement).map(Some);
                }
                return self.resolve_depth(&format!("{}/index.js", p.dir), &replacement, depth + 1);
            }
        }
        self.target(&p, &target).map(Some)
    }
    /// URL constructors use URL-relative semantics, including a bare filename, rather than npm specifier semantics.
    pub fn resolve_url(&mut self, from: &str, spec: &str) -> Result<Option<String>, String> {
        if spec.starts_with('/') || spec.contains(':') || [".", "..", "./", "../"].contains(&spec) {
            return Ok(None);
        }
        if spec.is_empty() || spec.len() > 512 || spec.contains(['?', '#', '\\']) {
            return Err(error(
                "URL asset needs a bounded literal relative path without query values",
            ));
        }
        if self.packages.is_empty() && self.bytes == 0 {
            self.start = Instant::now();
        }
        self.resolutions += 1;
        if self.resolutions > 4096 {
            return Err(error("dependency graph exceeds 4096 references"));
        }
        let owner = self.containing(from)?;
        let file = normalized(&Path::new(from).parent().unwrap_or(Path::new("")).join(spec))?;
        if let Some(owner) = owner {
            if !file.starts_with(&format!("{}/", owner.dir)) {
                return Err(error("URL asset leaves its owning package"));
            }
        }
        Ok(Some(file))
    }
    pub fn read_asset(&mut self, file: &str) -> Result<Vec<u8>, String> {
        if self.files.len() >= MAX_FILES && !self.files.contains_key(file) {
            return Err(error("dependency graph exceeds 512 files"));
        }
        let extension = Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if ![
            "wasm", "png", "jpg", "jpeg", "webp", "avif", "gif", "svg", "ico", "woff", "woff2",
            "ttf", "otf", "json", "txt", "css",
        ]
        .contains(&extension)
        {
            return Err(error(
                "unsupported URL asset type; provide an explicit reviewed adapter",
            ));
        }
        let bytes = self.read_bytes(file, ASSET_BYTES)?;
        if extension == "wasm" && !bytes.starts_with(b"\0asm\x01\0\0\0") {
            return Err(error("WebAssembly asset has an invalid module header"));
        }
        self.files
            .insert(file.to_owned(), (hash(&bytes), bytes.len()));
        Ok(bytes)
    }
    pub fn read(&mut self, file: &str) -> Result<String, String> {
        if vendor::is_vendor(file) {
            return self.read_vendor(file);
        }
        if !is_foreign(file) || !(file.ends_with(".js") || file.ends_with(".mjs")) {
            return Err(error("foreign reader requires an installed package file"));
        }
        if self.files.len() >= MAX_FILES && !self.files.contains_key(file) {
            return Err(error("dependency graph exceeds 512 modules"));
        }
        self.containing(file)?
            .ok_or_else(|| error("missing package owner"))?;
        let bytes = self.read_bytes(file, FILE_BYTES)?;
        let source = std::str::from_utf8(&bytes)
            .map_err(|_| error("dependency JavaScript must be UTF-8"))?;
        let prepared = prepare(source)?;
        self.files
            .insert(file.to_owned(), (hash(&bytes), bytes.len()));
        Ok(prepared)
    }
    /// Detect installed dependency/lock edits during the graph snapshot before emitting artifacts.
    pub fn verify(&mut self) -> Result<(), String> {
        let manifests: Vec<_> = self
            .packages
            .values()
            .map(|p| (format!("{}/package.json", p.dir), p.revision.clone()))
            .collect();
        let files: Vec<_> = self
            .files
            .iter()
            .map(|(p, (revision, _))| (p.clone(), revision.clone()))
            .collect();
        for (path, revision) in manifests.into_iter().chain(files) {
            if hash(&self.read_bytes(&path, ASSET_BYTES)?) != revision {
                return Err(error(
                    "installed dependency changed during graph preparation; retry after installation completes",
                ));
            }
        }
        if let Some(path) = self.lock_file.clone() {
            if Some(hash(&self.read_bytes(&path, FILE_BYTES)?)) != self.lock {
                return Err(error(
                    "dependency lockfile changed during graph preparation; prepare a fresh build",
                ));
            }
        }
        self.verify_vendor()?;
        Ok(())
    }
    pub fn summary(&self) -> Json {
        Json::obj([
            ("schemaVersion", Json::Num(1.0)),
            (
                "vendorLockRevision",
                self.vendor_revision.as_ref().map_or(Json::Null, Json::str),
            ),
            (
                "lockRevision",
                self.lock.as_ref().map_or(Json::Null, Json::str),
            ),
            ("readBytes", Json::Num(self.bytes as f64)),
            (
                "packages",
                Json::Arr(
                    self.packages
                        .values()
                        .map(|p| {
                            Json::obj([
                                ("name", Json::str(&p.name)),
                                ("version", Json::str(&p.version)),
                                ("manifest", Json::str(format!("{}/package.json", p.dir))),
                                ("manifestRevision", Json::str(&p.revision)),
                                ("sideEffects", p.side_effects.clone()),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "files",
                Json::Arr(
                    self.files
                        .iter()
                        .map(|(file, (rev, size))| {
                            Json::obj([
                                ("file", Json::str(file)),
                                ("revision", Json::str(rev)),
                                ("bytes", Json::Num(*size as f64)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}
/// Scope proof for native identifiers. Renames are analyzed but never emitted into SDK source.
pub fn free_identifier_sites(
    source: &str,
    names: &[&str],
) -> Result<BTreeMap<String, HashSet<usize>>, String> {
    let names: HashSet<String> = names.iter().map(|name| name.to_string()).collect();
    let sites = brain_min::mangle::free_identifier_sites(&lex(source)?, &names)?;
    let mut output: BTreeMap<String, HashSet<usize>> = names
        .into_iter()
        .map(|name| (name, HashSet::new()))
        .collect();
    for (offset, name) in sites {
        output.entry(name).or_default().insert(offset);
    }
    Ok(output)
}
/// Conservative browser-only eligibility and the one fixed compile-time Node environment replacement.
pub fn prepare(source: &str) -> Result<String, String> {
    let tokens = lex(source).map_err(|_| error("dependency JavaScript could not be tokenized"))?;
    let mut replacements = vec![];
    let names = ["require", "module", "exports", "process"];
    let relevant = tokens.iter().enumerate().any(|(i, t)| {
        names.iter().any(|name| t.ident(name))
            && !(i > 0 && (tokens[i - 1].punct(".") || tokens[i - 1].punct("?.")))
    });
    let free = if relevant {
        free_identifier_sites(source, &names)?
    } else {
        BTreeMap::new()
    };
    for (i, t) in tokens.iter().enumerate() {
        let native = free
            .get(&t.text)
            .is_some_and(|sites| sites.contains(&t.start));
        let property = i > 0 && (tokens[i - 1].punct(".") || tokens[i - 1].punct("?."));
        if !property
            && native
            && ((t.ident("require") && tokens.get(i + 1).is_some_and(|n| n.punct("(")))
                || (t.ident("module")
                    && tokens.get(i + 1).is_some_and(|n| n.punct("."))
                    && tokens.get(i + 2).is_some_and(|n| n.ident("exports")))
                || (t.ident("exports") && tokens.get(i + 1).is_some_and(|n| n.punct("."))))
        {
            return Err(error("CommonJS constructs require an explicit adapter"));
        }
        if t.ident("import")
            && tokens.get(i + 1).is_some_and(|n| n.punct("("))
            && !tokens.get(i + 2).is_some_and(|n| n.kind == Kind::Str)
        {
            return Err(error(
                "nonliteral dynamic import cannot form a bounded static graph",
            ));
        }
        if (t.ident("interface") || t.ident("enum") || t.ident("namespace"))
            && tokens.get(i + 1).is_some_and(|n| n.kind == Kind::Ident)
        {
            return Err(error("TypeScript source is not browser JavaScript"));
        }
        if t.punct("<")
            && tokens.get(i + 1).is_some_and(|n| n.kind == Kind::Ident)
            && i > 0
            && (tokens[i - 1].ident("return")
                || ["=>", "=", "(", "[", ",", ":"]
                    .iter()
                    .any(|p| tokens[i - 1].punct(p)))
        {
            return Err(error("JSX source is not supported"));
        }
        if t.ident("process") && !property && native {
            if i > 0
                && (tokens[i - 1].ident("delete")
                    || tokens[i - 1].punct("++")
                    || tokens[i - 1].punct("--"))
            {
                return Err(error(
                    "process property/shadowing/mutation cannot be rewritten safely",
                ));
            }
            if tokens.get(i + 1).is_some_and(|n| n.punct("."))
                && tokens.get(i + 2).is_some_and(|n| n.ident("env"))
                && tokens.get(i + 3).is_some_and(|n| n.punct("."))
                && tokens.get(i + 4).is_some_and(|n| n.ident("NODE_ENV"))
            {
                if tokens.get(i + 5).is_some_and(|n| {
                    [
                        "=", "+=", "-=", "*=", "/=", "%=", "&&=", "||=", "??=", "++", "--", "**=",
                        "&=", "|=", "^=", "<<=", ">>=", ">>>=",
                    ]
                    .iter()
                    .any(|p| n.punct(p))
                }) {
                    return Err(error("dependency mutates process environment"));
                }
                replacements.push((t.start, tokens[i + 4].start + tokens[i + 4].text.len()));
            } else {
                return Err(error(
                    "unsupported process global or shadowing; only fixed NODE_ENV reads are recognized",
                ));
            }
        }
    }
    let mut out = source.to_owned();
    for (a, b) in replacements.into_iter().rev() {
        out.replace_range(a..b, "'production'");
    }
    Ok(out)
}
/// None means default/namespace/dynamic/re-export/unknown use, requiring all public exports.
pub fn used_exports(source: &str, spec: &str) -> Result<Option<HashSet<String>>, String> {
    let tokens = lex(source)?;
    let mut found = false;
    for (i, t) in tokens.iter().enumerate() {
        if t.kind != Kind::Str || t.text.get(1..t.text.len().saturating_sub(1)) != Some(spec) {
            continue;
        }
        if i > 0 && tokens[i - 1].ident("import") {
            found = true;
            continue;
        }
        if i > 1 && tokens[i - 1].punct("(") && tokens[i - 2].ident("import") {
            return Ok(None);
        }
        if i > 0 && tokens[i - 1].ident("from") {
            let start = (0..i)
                .rev()
                .find(|j| tokens[*j].ident("import") || tokens[*j].ident("export"));
            let Some(start) = start else { return Ok(None) };
            if tokens[start].ident("export") || !tokens.get(start + 1).is_some_and(|n| n.punct("{"))
            {
                return Ok(None);
            }
            found = true;
        }
    }
    if !found {
        return Ok(None);
    }
    Ok(Some(
        brain_min::imported_names(source, &|s| s == spec)?
            .into_iter()
            .collect(),
    ))
}
/// Retain authored legal/@preserve comments from token gaps, never scanning inside string/regex/template tokens.
pub fn notices(source: &str) -> Result<String, String> {
    let tokens = lex(source)?;
    let mut out = String::new();
    let mut previous = 0;
    for token in &tokens {
        let gap = &source[previous..token.start];
        let lower = gap.to_ascii_lowercase();
        if lower.contains("@license")
            || lower.contains("@preserve")
            || lower.contains("copyright")
            || lower.contains("spdx-license-identifier")
            || lower.contains("/*!")
        {
            out.push_str(gap);
            out.push('\n');
        }
        previous = token.start + token.text.len();
    }
    let tail = &source[previous..];
    let lower = tail.to_ascii_lowercase();
    if lower.contains("@license")
        || lower.contains("@preserve")
        || lower.contains("copyright")
        || lower.contains("spdx-license-identifier")
        || lower.contains("/*!")
    {
        out.push_str(tail);
        out.push('\n');
    }
    Ok(out)
}
/// Own printer only: SDK logs, imports, public members and export names are not rewritten.
pub fn compact(source: &str) -> Result<String, String> {
    brain_min::minify(
        source,
        &brain_min::Options {
            mangle: false,
            drop_log: false,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct App(PathBuf);
    impl App {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "brain-package-resolve-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(p.join("node_modules/demo/dist")).unwrap();
            Self(p)
        }
        fn manifest(&self, json: &str) {
            fs::write(self.0.join("node_modules/demo/package.json"), json).unwrap();
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn conditional_exports_preserve_manifest_order_and_subpath_patterns() {
        let a = App::new();
        a.manifest(r#"{"name":"demo","version":"1.2.3","exports":{".":{"default":"./dist/default.mjs","browser":"./dist/browser.mjs"},"./tools/*":"./dist/*.mjs"}}"#);
        let mut r = Resolver::new(&a.0).unwrap();
        assert_eq!(
            r.resolve("views/home.brain", "demo").unwrap().unwrap(),
            "node_modules/demo/dist/default.mjs"
        );
        assert_eq!(
            r.resolve("app.brain", "demo/tools/math").unwrap().unwrap(),
            "node_modules/demo/dist/math.mjs"
        );
        assert!(r.resolve("app.brain", "demo/private").is_err());
    }
    #[test]
    fn imports_aliases_browser_maps_and_relative_modules_are_contained() {
        let a = App::new();
        a.manifest(r##"{"name":"demo","version":"1.0.0","type":"module","exports":"./dist/main.mjs","imports":{"#math":"./dist/math.mjs"},"browser":{"./dist/main.mjs":"./dist/browser.mjs","./dist/node.mjs":false}}"##);
        let mut r = Resolver::new(&a.0).unwrap();
        assert_eq!(
            r.resolve("app.brain", "demo").unwrap().unwrap(),
            "node_modules/demo/dist/browser.mjs"
        );
        assert_eq!(
            r.resolve("node_modules/demo/dist/main.mjs", "#math")
                .unwrap()
                .unwrap(),
            "node_modules/demo/dist/math.mjs"
        );
        assert!(
            r.resolve("node_modules/demo/dist/main.mjs", "./node.mjs")
                .is_err()
        );
        assert!(
            r.resolve("node_modules/demo/dist/main.mjs", "../../../private.js")
                .is_err()
        );
        assert!(
            r.resolve("app.brain", "https://example.test/sdk.mjs")
                .is_err()
        );
    }
    #[test]
    fn foreign_reads_reject_symlinks_cjs_and_unbounded_files() {
        let a = App::new();
        a.manifest(r#"{"name":"demo","version":"1.0.0","type":"module"}"#);
        fs::write(
            a.0.join("node_modules/demo/dist/main.mjs"),
            "module.exports = {}",
        )
        .unwrap();
        let mut r = Resolver::new(&a.0).unwrap();
        assert!(r.read("node_modules/demo/dist/main.mjs").is_err());
        fs::write(
            a.0.join("node_modules/demo/dist/main.mjs"),
            " ".repeat(FILE_BYTES + 1),
        )
        .unwrap();
        assert!(r.read("node_modules/demo/dist/main.mjs").is_err());
        #[cfg(unix)]
        {
            fs::remove_file(a.0.join("node_modules/demo/dist/main.mjs")).unwrap();
            fs::write(a.0.join("private.js"), "secret input not returned").unwrap();
            std::os::unix::fs::symlink(
                a.0.join("private.js"),
                a.0.join("node_modules/demo/dist/main.mjs"),
            )
            .unwrap();
            assert!(r.read("node_modules/demo/dist/main.mjs").is_err());
        }
    }
    #[test]
    fn fixed_production_define_does_not_read_environment_or_rewrite_shadowed_properties() {
        assert!(
            prepare("export const dev = process.env.NODE_ENV !== 'production'")
                .unwrap()
                .contains("'production' !== 'production'")
        );
        assert_eq!(
            prepare("obj.process.env.NODE_ENV").unwrap(),
            "obj.process.env.NODE_ENV"
        );
        assert_eq!(
            prepare("function f(process){return process.env.NODE_ENV}").unwrap(),
            "function f(process){return process.env.NODE_ENV}"
        );
        assert!(prepare("function f(exports){return exports.malloc(1)}; export const wasm={exports:{}}; wasm.exports.memory;").is_ok());
        assert!(prepare("globalThis.process?.versions?.node; wasm.exports.memory;").is_ok());
        for source in [
            "process.env.SECRET",
            "delete process.env.NODE_ENV",
            "process.env.NODE_ENV ||= 'x'",
            "const el = <div/>",
        ] {
            assert!(prepare(source).is_err(), "{source}");
        }
    }
    #[test]
    fn only_named_static_consumers_allow_export_removal() {
        assert_eq!(
            used_exports("import { one as local, two } from 'demo'", "demo")
                .unwrap()
                .unwrap(),
            ["one".into(), "two".into()].into()
        );
        for source in [
            "import x from 'demo'",
            "import * as x from 'demo'",
            "import('demo')",
            "export { one } from 'demo'",
        ] {
            assert!(used_exports(source, "demo").unwrap().is_none());
        }
        let output =
            compact("console.log('SDK effect'); export const publicName = { methodName: 1 }")
                .unwrap();
        assert!(output.contains("console.log"));
        assert!(output.contains("publicName"));
        assert!(output.contains("methodName"));
    }
    #[test]
    fn manifest_and_lock_revisions_are_explicit_and_unrelated_builds_ignore_bad_lock() {
        let a = App::new();
        a.manifest(r#"{"name":"demo","version":"1.0.0","type":"module","sideEffects":false}"#);
        fs::write(a.0.join("package-lock.json"), "bad").unwrap();
        let mut r = Resolver::new(&a.0).unwrap();
        assert_eq!(r.resolve("app.brain", "./local.js").unwrap(), None);
        assert!(r.resolve("app.brain", "demo").is_err());
        fs::write(a.0.join("package-lock.json"), r#"{"lockfileVersion":3}"#).unwrap();
        let mut r = Resolver::new(&a.0).unwrap();
        r.resolve("app.brain", "demo").unwrap();
        let summary = r.summary();
        assert!(summary.get("lockRevision").unwrap().as_str().is_some());
        assert!(summary.stringify().contains("manifestRevision"));
    }
}
