//! Application routes for the native backend: `server/routes/<name>/route.jsbrain` (literal
//! contract) and `server/routes/<name>/handler.jsbrain` (default export, private Node sidecars).
use brain_backend::routes::{Handlers, Route, RouteFailure, RouteHandlers};
use brain_serve::load::Sidecar;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

const ROUTES: &str = "server/routes";
const WORKERS: usize = 4;
const OUTPUT_LIMIT: usize = 1024 * 1024;

struct Compiled {
    _scratch: crate::backend::Scratch,
    module: PathBuf,
}
struct Executor {
    root: PathBuf,
    vars: Vec<(String, String)>,
    production: bool,
    budget: Duration,
    modules: BTreeMap<String, Arc<Compiled>>,
    pool: Arc<Mutex<Vec<Sidecar>>>,
    slots: Arc<tokio::sync::Semaphore>,
}

fn spawn(root: &Path, vars: &[(String, String)], budget: Duration, production: bool) -> Result<Sidecar, String> {
    Sidecar::spawn_route(root, vars, budget, OUTPUT_LIMIT, production).map_err(|error| {
        if error.kind() == std::io::ErrorKind::Unsupported {
            "Native routes require Unix process-tree supervision in this release; use a supported Unix backend host".into()
        } else {
            "Cannot provision the private route executor; install Node or inspect its private runtime configuration".into()
        }
    })
}

/// Reads every route contract. `env` resolves signing secret names.
pub fn contracts(root: &Path, environment: &str) -> Result<Vec<Route>, String> {
    let dir = root.join(ROUTES);
    if !dir.is_dir() {
        return Ok(vec![]);
    }
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map_err(|_| format!("{ROUTES}: directory is unreadable"))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    if names.len() > brain_backend::routes::MAX_ROUTES {
        return Err(format!("{ROUTES}: declare at most {} routes", brain_backend::routes::MAX_ROUTES));
    }
    let lookup = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty()).or_else(|| crate::env::value(root, environment, key));
    let mut routes = Vec::new();
    for name in names {
        let relative = format!("{ROUTES}/{name}/route.jsbrain");
        let path = root.join(&relative);
        if !path.is_file() {
            return Err(format!("{relative}: every route folder needs a route.jsbrain contract"));
        }
        let value = crate::backend::read_contract(root, &path).map_err(|e| format!("{relative}: {e}"))?;
        routes.push(Route::from_contract(&name, &value, &lookup).map_err(|e| format!("{relative}: {e}"))?);
    }
    Ok(routes)
}

/// Compiles and probes every handler before HTTP starts; `None` when no route is declared.
pub fn load(root: &Path, environment: &str, routes: &[Route]) -> Result<Option<Handlers>, String> {
    if routes.is_empty() {
        return Ok(None);
    }
    let vars = crate::env::select(root, environment);
    let budget = routes.iter().map(|r| r.timeout).max().unwrap_or(brain_backend::routes::DEFAULT_TIMEOUT);
    let mut modules = BTreeMap::new();
    for route in routes {
        let relative = format!("{ROUTES}/{}/handler.jsbrain", route.name);
        crate::changes::source_path(root, &relative)?;
        crate::map::read_source_limit(root, &relative, 256 * 1024)
            .map_err(|_| format!("{relative}: handler source must be a regular file within 256 KiB"))?;
        let (scratch, module) = crate::backend::server_module(root, &relative)
            .map_err(|_| format!("{relative}: private handler compilation failed; run brain check for structured source diagnostics"))?;
        modules.insert(route.name.clone(), Arc::new(Compiled { _scratch: scratch, module }));
    }
    let production = environment == "production";
    let mut pool = Vec::with_capacity(WORKERS);
    for _ in 0..WORKERS {
        pool.push(spawn(root, &vars, budget, production)?);
    }
    for (name, compiled) in &modules {
        pool[0]
            .probe(&brain_serve::load::file_url(&compiled.module), "default")
            .map_err(|_| format!("{ROUTES}/{name}/handler.jsbrain: export default must be a callable handler(ctx)"))?;
    }
    Ok(Some(Arc::new(Executor {
        root: root.to_path_buf(),
        vars,
        production,
        budget,
        modules,
        pool: Arc::new(Mutex::new(pool)),
        slots: Arc::new(tokio::sync::Semaphore::new(WORKERS)),
    })))
}

/// A thrown `{errors}` becomes a 422 with those field messages; a thrown error with a 4xx
/// `status` becomes that status with a generic code. Messages of other errors never leave.
fn rejection(status: u16, raw: &str) -> Option<RouteFailure> {
    if !(400..500).contains(&status) {
        return None;
    }
    let parsed = serde_json::from_str::<Value>(raw).ok();
    let errors = parsed.as_ref().and_then(|v| v.get("errors")).filter(|e| e.is_object()).cloned();
    Some(match errors {
        Some(fields) => RouteFailure::Status(422, json!({"schemaVersion":1,"error":{"code":"INVALID_INPUT","fix":"Apply the field errors","fields":fields}})),
        None => RouteFailure::Status(status, json!({"schemaVersion":1,"error":{"code":"ROUTE_REJECTED","fix":"The route rejected this request","fields":null}})),
    })
}

impl RouteHandlers for Executor {
    fn names(&self) -> Vec<String> {
        self.modules.keys().cloned().collect()
    }
    fn run(&self, name: &str, context: Value, _timeout: Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, RouteFailure>> + Send + '_>> {
        let name = name.to_owned();
        Box::pin(async move {
            let permit = self.slots.clone().try_acquire_owned().map_err(|_| RouteFailure::Capacity)?;
            let compiled = self.modules.get(&name).ok_or(RouteFailure::Unavailable)?.clone();
            let (root, vars, production, budget, pool) = (self.root.clone(), self.vars.clone(), self.production, self.budget, self.pool.clone());
            let ctx = serde_json::to_string(&context).map_err(|_| RouteFailure::Unavailable)?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let mut sidecar = pool.lock().map_err(|_| RouteFailure::Unavailable)?.pop().ok_or(RouteFailure::Capacity)?;
                let (result, reusable) = match sidecar.invoke(&brain_serve::load::file_url(&compiled.module), "default", &ctx, None) {
                    Ok(raw) => (serde_json::from_str::<Value>(&raw).map_err(|_| RouteFailure::Unavailable), true),
                    Err((status, raw)) => match rejection(status, &raw) {
                        Some(failure) => (Err(failure), true),
                        None => (Err(RouteFailure::Unavailable), false),
                    },
                };
                if reusable {
                    if let Ok(mut slots) = pool.lock() {
                        slots.push(sidecar);
                    }
                } else {
                    drop(sidecar);
                    if let Ok(fresh) = spawn(&root, &vars, budget, production) {
                        if let Ok(mut slots) = pool.lock() {
                            slots.push(fresh);
                        }
                    }
                }
                result
            })
            .await
            .map_err(|_| RouteFailure::Unavailable)?
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn handler_rejections_never_forward_private_messages() {
        assert!(matches!(rejection(422, r#"{"errors":{"total":"must be positive"}}"#), Some(RouteFailure::Status(422, body)) if body["error"]["fields"]["total"] == "must be positive"));
        assert!(matches!(rejection(409, r#"{"error":"private detail"}"#), Some(RouteFailure::Status(409, body)) if !body.to_string().contains("private")));
        assert!(rejection(500, "{}").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn routes_load_from_folders_compile_and_run_handlers() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("brain-route-executor-{}-{stamp}", std::process::id()));
        crate::scaffold::new_app(&root).unwrap();
        let dir = root.join(ROUTES).join("order_quote");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("route.jsbrain"), "<meta server>\nexport default { method: 'POST', path: '/orders/quote', auth: 'public' }\n").unwrap();
        fs::write(dir.join("handler.jsbrain"), "<meta server>\nexport default async function handle(ctx) {\n  if (ctx.body.items < 1) throw { errors: { items: 'add at least one item' } }\n  return { status: 201, body: { total: ctx.body.items * 5 } }\n}\n").unwrap();
        let routes = contracts(&root, "development").unwrap();
        assert_eq!(routes[0].path, "/orders/quote");
        let handlers = load(&root, "development", &routes).unwrap().unwrap();
        assert_eq!(handlers.names(), vec!["order_quote".to_string()]);
        let out = handlers.run("order_quote", json!({"body": {"items": 3}}), Duration::from_secs(5)).await.unwrap();
        assert_eq!(out, json!({"status": 201, "body": {"total": 15}}));
        let denied = handlers.run("order_quote", json!({"body": {"items": 0}}), Duration::from_secs(5)).await.unwrap_err();
        assert!(matches!(denied, RouteFailure::Status(422, _)));
        drop(handlers);
        fs::remove_file(dir.join("route.jsbrain")).unwrap();
        assert!(contracts(&root, "development").unwrap_err().contains("route.jsbrain"));
        let _ = fs::remove_dir_all(&root);
    }
}
