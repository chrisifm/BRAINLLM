//! Mandatory native mutation hooks. The executor is trusted server code; returned data is always revalidated.
use crate::{Resource, auth::User};
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

#[derive(Clone, Debug)]
pub enum HookFailure {
    Rejected(Vec<String>),
    Capacity,
    Unavailable,
}
pub trait BeforeHooks: Send + Sync {
    /// Exact compiled hook names available before HTTP starts. No lazy missing-module fallback.
    fn names(&self) -> Vec<String>;
    fn run(
        &self,
        name: &str,
        context: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, HookFailure>> + Send + '_>>;
}
pub type Executor = Arc<dyn BeforeHooks>;
pub fn validate(resources: &[Resource], executor: Option<&Executor>) -> Result<(), String> {
    let available = executor.map(|e| e.names()).unwrap_or_default();
    if available.len() > 128
        || available
            .iter()
            .any(|name| !crate::contract::identifier(name))
    {
        return Err("Before-hook executor must declare at most 128 valid compiled names".into());
    }
    for resource in resources {
        for op in ["create", "update", "delete"] {
            if resource
                .hooks
                .before(op)
                .is_some_and(|name| !available.iter().any(|compiled| compiled == name))
            {
                return Err("A declared before-hook has no compiled executor; provision server/hooks/<name>.jsbrain before serving".into());
            }
        }
    }
    Ok(())
}
/// Protect the execution boundary before giving any raw request object to application code.
pub fn bounded(value: &Value) -> bool {
    fn walk(v: &Value, depth: usize, n: &mut usize) -> bool {
        *n += 1;
        if depth > 32 || *n > 4096 {
            return false;
        }
        match v {
            Value::Object(o) => o
                .iter()
                .all(|(k, v)| k.len() <= 128 && walk(v, depth + 1, n)),
            Value::Array(a) => a.iter().all(|v| walk(v, depth + 1, n)),
            _ => true,
        }
    }
    value.is_object()
        && walk(value, 0, &mut 0)
        && serde_json::to_vec(value).is_ok_and(|v| v.len() <= 49152)
}
pub async fn execute(
    executor: Option<&Executor>,
    resource: &Resource,
    operation: &str,
    payload: Value,
    existing: Option<Value>,
    user: &User,
    rid: Option<String>,
) -> Result<Value, HookFailure> {
    let Some(name) = resource.hooks.before(operation) else {
        return Ok(payload);
    };
    if !bounded(&payload) {
        return Err(HookFailure::Rejected(vec![]));
    }
    let executor = executor.ok_or(HookFailure::Unavailable)?;
    let context = json!({"resource":resource.name,"operation":operation,"payload":payload,"existing":existing,"user":user,"rid":rid});
    if serde_json::to_vec(&context).map_or(true, |bytes| bytes.len() > 65536) {
        return Err(HookFailure::Rejected(vec![]));
    }
    let result = tokio::time::timeout(Duration::from_secs(2), executor.run(name, context))
        .await
        .map_err(|_| HookFailure::Unavailable)??;
    if !bounded(&result) {
        return Err(HookFailure::Unavailable);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_execution_boundary_has_object_depth_key_and_byte_budgets() {
        assert!(bounded(&json!({"ok":1})));
        assert!(!bounded(&json!([])));
        assert!(!bounded(&json!({"v":"a".repeat(49152)})));
        let mut value = json!({});
        for _ in 0..34 {
            value = json!({"x":value});
        }
        assert!(!bounded(&value));
    }
}
