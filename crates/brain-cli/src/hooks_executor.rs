//! Private compiled native hooks. Two owned Node sidecars, no wait queue and no application error bodies in logs.
use brain_backend::{
    Resource,
    before_hooks::{BeforeHooks, Executor, HookFailure},
};
use brain_serve::load::Sidecar;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
struct Compiled {
    _scratch: crate::backend::Scratch,
    module: PathBuf,
}
struct Hooks {
    root: PathBuf,
    vars: Vec<(String, String)>,
    production: bool,
    modules: BTreeMap<String, Arc<Compiled>>,
    pool: Arc<Mutex<Vec<Sidecar>>>,
    slots: Arc<tokio::sync::Semaphore>,
}
fn spawn(root: &Path, vars: &[(String, String)], production: bool) -> Result<Sidecar, String> {
    Sidecar::spawn_bounded(root,vars,Duration::from_secs(2),65536,production).map_err(|error| {
        if error.kind() == std::io::ErrorKind::Unsupported {
            "Native before-hooks require Unix process-tree supervision in this release; use a supported Unix backend host".into()
        } else {
            "Cannot provision the private before-hook executor; install Node or inspect its private runtime configuration".into()
        }
    })
}
pub fn load(
    root: &Path,
    environment: &str,
    resources: &[Resource],
) -> Result<Option<Executor>, String> {
    let names: BTreeSet<String> = resources
        .iter()
        .flat_map(|r| {
            ["create", "update", "delete"]
                .into_iter()
                .filter_map(|op| r.hooks.before(op).map(str::to_owned))
        })
        .collect();
    if names.is_empty() {
        return Ok(None);
    }
    if names.len() > 128 {
        return Err("Before-hooks support at most 128 distinct compiled modules".into());
    }
    let vars = crate::env::select(root, environment);
    let mut modules = BTreeMap::new();
    for name in names {
        if !brain_backend::contract::identifier(&name) {
            return Err("Before-hook names must be valid identifiers".into());
        }
        let relative = format!("server/hooks/{name}.jsbrain");
        crate::changes::source_path(root, &relative)?;
        crate::map::read_source_limit(root, &relative, 65536)
            .map_err(|_| format!("{relative}: hook source must be a regular file within 64 KiB"))?;
        let (scratch,module)=crate::backend::server_module(root,&relative).map_err(|_|format!("{relative}: private hook compilation failed; run brain check for structured source diagnostics"))?;
        modules.insert(
            name,
            Arc::new(Compiled {
                _scratch: scratch,
                module,
            }),
        );
    }
    let production = environment == "production";
    let mut pool = Vec::with_capacity(2);
    for _ in 0..2 {
        pool.push(spawn(root, &vars, production)?);
    }
    for compiled in modules.values() {
        pool[0].probe(&brain_serve::load::file_url(&compiled.module),"default").map_err(|_|"A before-hook could not be imported as a callable default export within its private execution budget".to_string())?;
    }
    Ok(Some(Arc::new(Hooks {
        root: root.to_path_buf(),
        vars,
        production,
        modules,
        pool: Arc::new(Mutex::new(pool)),
        slots: Arc::new(tokio::sync::Semaphore::new(2)),
    })))
}
impl BeforeHooks for Hooks {
    fn names(&self) -> Vec<String> {
        self.modules.keys().cloned().collect()
    }
    fn run(
        &self,
        name: &str,
        context: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, HookFailure>> + Send + '_>>
    {
        let name = name.to_owned();
        Box::pin(async move {
            let permit = self
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| HookFailure::Capacity)?;
            let compiled = self
                .modules
                .get(&name)
                .ok_or(HookFailure::Unavailable)?
                .clone();
            let root = self.root.clone();
            let vars = self.vars.clone();
            let production = self.production;
            let pool = self.pool.clone();
            let rid = context
                .get("rid")
                .and_then(Value::as_str)
                .filter(|s| {
                    s.len() == 36
                        && s.bytes().enumerate().all(|(i, b)| {
                            if [8, 13, 18, 23].contains(&i) {
                                b == b'-'
                            } else {
                                b.is_ascii_hexdigit()
                            }
                        })
                })
                .map(str::to_owned);
            let ctx = serde_json::to_string(&context).map_err(|_| HookFailure::Unavailable)?;
            tokio::task::spawn_blocking(move || {
                let _permit=permit;
                let mut sidecar=pool.lock().map_err(|_|HookFailure::Unavailable)?.pop().ok_or(HookFailure::Unavailable)?;
                let result=sidecar.invoke(&brain_serve::load::file_url(&compiled.module),"default",&ctx,None);
                let (result,reusable)=match result {
                    Ok(raw)=> (serde_json::from_str::<Value>(&raw).map_err(|_|HookFailure::Unavailable),true),
                    Err((422,raw))=> {
                        let fields=serde_json::from_str::<Value>(&raw).ok().and_then(|value|value.get("errors").and_then(Value::as_object).map(|fields|fields.keys().filter(|name|brain_backend::contract::identifier(name)).take(64).cloned().collect())).unwrap_or_default();
                        (Err(HookFailure::Rejected(fields)),true)
                    },
                    Err(_)=> (Err(HookFailure::Unavailable),false),
                };
                if reusable {if let Ok(mut slots)=pool.lock(){slots.push(sidecar)}} else {drop(sidecar);if let Ok(sidecar)=spawn(&root,&vars,production){if let Ok(mut slots)=pool.lock(){slots.push(sidecar)}}}
                if result.is_err() {eprintln!("{}",json!({"schemaVersion":1,"event":"before-hook-failed","code":"BEFORE_HOOK_FAILED","fix":"Inspect the project-owned before-hook and its bounded input/output contract","rid":rid,"location":{"file":format!("server/hooks/{name}.jsbrain"),"line":Value::Null,"precision":"module-only"},"valuesCaptured":false}));}
                result
            }).await.map_err(|_|HookFailure::Unavailable)?
        })
    }
}

/// A generated hook fails closed until its project-owned validation is implemented.
pub fn scaffold(name: &str) -> Result<Vec<(String, String)>, String> {
    if !brain_backend::contract::identifier(name) {
        return Err("Hook name must be a lowercase identifier".into());
    }
    Ok(vec![(format!("server/hooks/{name}.jsbrain"),"<meta server>\nexport default async function beforeMutation(ctx) {\n  throw new Error('Implement the mandatory validation and return ctx.payload')\n}\n".into())])
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    struct Root(PathBuf);
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scaffolded_hooks_compile_fail_closed_transform_in_production_bound_capacity_and_recover()
     {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Root(std::env::temp_dir().join(format!(
            "brain-hook-executor-{}-{stamp}",
            std::process::id()
        )));
        crate::scaffold::new_app(&root.0).unwrap();
        let path = crate::scaffold::add_with_options(
            &root.0,
            "hook",
            "validate_item",
            &Default::default(),
        )
        .unwrap();
        assert!(path.ends_with("server/hooks/validate_item.jsbrain"));
        assert!(
            crate::scaffold::add_with_options(
                &root.0,
                "hook",
                "validate_item",
                &Default::default()
            )
            .is_err()
        );
        let resource:Resource=serde_json::from_value(json!({"name":"items","fields":{"title":{"kind":"str","required":true}},"policy":{"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"]},"hooks":{"before_create":"validate_item"}})).unwrap();
        let executor = load(&root.0, "production", &[resource.clone()])
            .unwrap()
            .unwrap();
        assert!(
            matches!(
                executor
                    .run("validate_item", json!({"payload":{"title":"x"}}))
                    .await,
                Err(HookFailure::Unavailable)
            ),
            "Generated placeholders must not silently accept writes"
        );
        drop(executor);
        std::fs::write(&path,"<meta server>\nexport default async function beforeMutation(ctx) {\n if (ctx.payload.title === 'deny') throw { errors: { title: 'synthetic-private-value' } }\n if (ctx.payload.title === 'wait') await new Promise(() => {})\n return {title: process.env.NODE_ENV}\n}\n").unwrap();
        let executor = load(&root.0, "production", &[resource]).unwrap().unwrap();
        assert_eq!(
            executor
                .run("validate_item", json!({"payload":{"title":"ok"}}))
                .await
                .unwrap(),
            json!({"title":"production"})
        );
        let denied = executor
            .run("validate_item", json!({"payload":{"title":"deny"}}))
            .await
            .unwrap_err();
        assert!(
            matches!(&denied,HookFailure::Rejected(fields) if fields==&vec!["title".to_string()])
        );
        assert!(!format!("{denied:?}").contains("synthetic-private"));
        let a = executor.clone();
        let b = executor.clone();
        let first = tokio::spawn(async move {
            a.run("validate_item", json!({"payload":{"title":"wait"}}))
                .await
        });
        let second = tokio::spawn(async move {
            b.run("validate_item", json!({"payload":{"title":"wait"}}))
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(matches!(
            executor
                .run("validate_item", json!({"payload":{"title":"ok"}}))
                .await,
            Err(HookFailure::Capacity)
        ));
        assert!(matches!(
            first.await.unwrap(),
            Err(HookFailure::Unavailable)
        ));
        assert!(matches!(
            second.await.unwrap(),
            Err(HookFailure::Unavailable)
        ));
        assert_eq!(
            executor
                .run("validate_item", json!({"payload":{"title":"recovered"}}))
                .await
                .unwrap(),
            json!({"title":"production"})
        );
        drop(executor);
        std::fs::write(
            &path,
            "<meta server>\nexport default { configured: true }\n",
        )
        .unwrap();
        let bad:Resource=serde_json::from_value(json!({"name":"items","fields":{"title":{"kind":"str","required":true}},"policy":{"read":["admin"],"create":["admin"],"update":["admin"],"delete":["admin"]},"hooks":{"before_create":"validate_item"}})).unwrap();
        assert!(
            load(&root.0, "production", &[bad]).is_err(),
            "A non-callable default export must fail before HTTP startup"
        );
    }
}
