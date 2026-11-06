//! Independent opt-in and bounded read-only document-performance reports.
use std::{io::Read, path::Path, time::Duration};
use brain_mcp::Json;

pub(crate) fn load(root: &Path) -> Result<Option<brain_backend::vitals::Config>, String> {
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
    if fields.iter().filter(|(key,_)| key == "vitals").count() > 1 { return Err("brain.json vitals must be declared once".into()) }
    if let Some(Json::Obj(fields)) = fields.iter().find(|(key,_)| key == "vitals").map(|(_,value)| value) {
        let mut seen = std::collections::BTreeSet::new();
        if fields.iter().any(|(key,_)| !seen.insert(key)) { return Err("vitals fields must be declared once".into()) }
    }
    let value: serde_json::Value = serde_json::from_str(&source).map_err(|_| "Native vitals requires valid brain.json JSON")?;
    brain_backend::vitals::Config::from_value(value.get("vitals"))
}
pub(crate) fn build_options(root: &Path, environment: &str) -> Result<Option<brain_pack::NativeVitalsOptions>, String> {
    let Some(config) = load(root)? else { return Ok(None) };
    let target = crate::env::value(root,environment,"BRAIN_BACKEND_URL").ok_or("Native vitals requires fixed BRAIN_BACKEND_URL in the selected environment; configure the native backend or disable vitals")?;
    // Reuse the actual gateway's strict fixed loopback authority validation.
    let _ = brain_serve::NativeProxy::new(&target)?;
    brain_pack::NativeVitalsOptions::new(config.routes,config.retention_days).map(Some)
}
pub fn run(args: &[String]) -> Result<(), String> {
    const USAGE: &str = "usage: brain vitals report [--env development] [--days 7] [--limit 100] [--offset 0] [--json]";
    if args.first().map(String::as_str) != Some("report") { return Err(USAGE.into()) }
    let mut seen = std::collections::BTreeSet::new();
    let mut at = 1;
    while at < args.len() {
        let key = args[at].as_str();
        if !seen.insert(key) { return Err(USAGE.into()) }
        match key {
            "--json" => at += 1,
            "--env" | "--days" | "--limit" | "--offset" => { if args.get(at+1).is_none_or(|value| value.starts_with('-')) { return Err(USAGE.into()) } at += 2; },
            _ => return Err(USAGE.into()),
        }
    }
    let number = |key:&str, default:u32| crate::flag(args,key).map(|v|v.parse::<u32>().map_err(|_| USAGE.to_string())).transpose().map(|v|v.unwrap_or(default));
    let (days,limit,offset) = (number("--days",7)?,number("--limit",100)?,number("--offset",0)?);
    if !(1..=90).contains(&days) || !(1..=500).contains(&limit) || offset > 10000 { return Err(USAGE.into()) }
    let root = crate::app_root()?;
    let config = load(&root)?.ok_or("VITALS_DISABLED: enable vitals with explicit public route patterns before requesting a report")?;
    let environment = crate::env::name(args,"development");
    let url = crate::backend::stated_database(args,&root,&environment)?;
    let rt = crate::http_observer::Runtime::new().map_err(|_| "VITALS_UNAVAILABLE: cannot start the bounded report runtime")?;
    rt.block_on(async {
        let mut boot = brain_backend::boot::Boot::command(environment == "production");
        boot.acquire = Duration::from_secs(2);
        let pool = tokio::time::timeout(Duration::from_secs(5),brain_backend::boot::connect(&url,&boot)).await.map_err(|_| "VITALS_UNAVAILABLE: database connection deadline exceeded")??;
        let result = brain_backend::vitals::report(&pool,&config,days,limit,offset).await;
        if !brain_backend::db::close_until(&pool,tokio::time::Instant::now()+Duration::from_secs(2)).await { return Err("VITALS_UNAVAILABLE: database cleanup deadline exceeded".into()) }
        let report = result.map_err(|e|format!("{}: document-performance report unavailable; inspect the native database and declared configuration",e.code()))?;
        println!("{report}");
        Ok(())
    })
}
