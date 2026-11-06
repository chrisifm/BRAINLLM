//! Explicit aggregate collector metadata; no database dependency or browser primitive.
use brain_lex::Json;
use std::{collections::BTreeSet, io::Read, path::Path};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeVitalsOptions { pub(crate) routes: Vec<String>, pub(crate) retention_days: u32 }
impl NativeVitalsOptions {
    /// The host calls this only after validating its fixed native backend authority.
    pub fn new(routes: Vec<String>, retention_days: u32) -> Result<Self, String> {
        if !(1..=90).contains(&retention_days) || routes.is_empty() || routes.len() > 512 || routes.iter().any(|route| !valid_route(route)) || routes.iter().collect::<BTreeSet<_>>().len() != routes.len() {
            return Err("Native vitals requires 1..512 distinct public route patterns (160 ASCII characters each) and retentionDays 1..90".into());
        }
        Ok(Self { routes, retention_days })
    }
}
fn valid_route(route: &str) -> bool {
    if route == "/" { return true }
    if route.len() > 160 || !route.starts_with('/') { return false }
    route[1..].split('/').all(|part| if let Some(param) = part.strip_prefix(':') {
        param.as_bytes().first().is_some_and(u8::is_ascii_lowercase) && param.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    } else { !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') })
}
pub(crate) fn read(root: &Path) -> Result<Option<NativeVitalsOptions>, String> {
    let path = root.join("brain.json");
    let meta = match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot inspect brain.json vitals configuration".into()), Ok(meta) => meta,
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 2*1024*1024 { return Err("Native vitals requires a regular brain.json file of at most 2 MiB".into()) }
    let mut source = String::new();
    std::fs::File::open(path).map_err(|_| "Cannot open brain.json vitals configuration")?.take(2*1024*1024+1).read_to_string(&mut source).map_err(|_| "Cannot read brain.json vitals configuration")?;
    if source.len() > 2*1024*1024 { return Err("brain.json exceeds the native vitals configuration limit".into()) }
    let Json::Obj(fields) = Json::parse(&source).map_err(|_| "Native vitals requires valid brain.json JSON")? else { return Err("brain.json must be an object".into()) };
    let entries: Vec<_> = fields.iter().filter(|(key,_)| key == "vitals").collect();
    if entries.len() > 1 { return Err("brain.json vitals must be declared once".into()) }
    let Some((_, value)) = entries.first() else { return Ok(None) };
    if matches!(value, Json::Bool(false)) { return Ok(None) }
    let Json::Obj(fields) = value else { return Err("vitals requires false or an object with explicit enabled and public routes".into()) };
    let mut keys = BTreeSet::new();
    if fields.iter().any(|(key,_)| !["enabled","routes","retentionDays"].contains(&key.as_str()) || !keys.insert(key)) { return Err("vitals requires distinct enabled, routes and retentionDays fields".into()) }
    let get = |key:&str| fields.iter().find(|(name,_)| name == key).map(|(_,v)| v);
    let enabled = match get("enabled") { Some(Json::Bool(value)) => *value, _ => return Err("vitals.enabled must be an explicit boolean".into()) };
    let retention = match get("retentionDays") { None => 30, Some(Json::Num(n)) if n.fract() == 0. && (1.0..=90.0).contains(n) => *n as u32, _ => return Err("vitals.retentionDays must be an integer from 1 to 90".into()) };
    let routes = match get("routes") {
        None if !enabled => Vec::new(),
        Some(Json::Arr(items)) => items.iter().map(|v|v.as_str().map(str::to_owned).ok_or("vitals.routes must contain public route patterns")).collect::<Result<Vec<_>,_>>()?,
        _ => return Err("vitals.routes must contain public route patterns".into()),
    };
    if !enabled && routes.is_empty() { return Ok(None) }
    let config = NativeVitalsOptions::new(routes,retention)?;
    Ok(enabled.then_some(config))
}
