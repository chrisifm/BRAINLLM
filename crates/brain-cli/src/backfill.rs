//! Parses a bounded existing BrainScript expression into the backend's typed data operations. No JS is evaluated.
use brain_backend::backfill::{Definition, Expr as Compute, Request};
use brain_parse::ast::{Expr, FnBody, MemberProp, ObjProp, Pat, PropKey, Stmt};
use serde_json::{Value, json};
use std::{path::Path, process::ExitCode, time::Duration};
fn error() -> String {
    "Backfill source must export { resource, field, compute: row => expression }; fix: use declared row.data fields, row.id, literals, + - * / ??, trim() or toLowerCase(), without imports, globals, async, blocks or mutation".into()
}
fn key(key: &PropKey) -> Result<&str, String> {
    match key {
        PropKey::Ident(i) => Ok(&i.name),
        PropKey::Str { value, .. } => Ok(value),
        _ => Err(error()),
    }
}
fn expression(
    expr: &Expr,
    param: &str,
    depth: usize,
    nodes: &mut usize,
) -> Result<Compute, String> {
    *nodes += 1;
    if depth > 16 || *nodes > 128 {
        return Err(error());
    }
    let mut child = |e: &Expr| expression(e, param, depth + 1, nodes);
    match expr.unparen() {
        Expr::Str { value, .. } if value.len() <= 4096 => Ok(Compute::Literal(json!(value))),
        Expr::Bool { value, .. } => Ok(Compute::Literal(json!(value))),
        Expr::Null(_) => Ok(Compute::Literal(Value::Null)),
        Expr::Num { raw, .. } => {
            let value: Value = serde_json::from_str(raw).map_err(|_| error())?;
            if !value.is_number() {
                return Err(error());
            }
            Ok(Compute::Literal(value))
        }
        Expr::Unary { op: "-", arg, .. } => {
            let Expr::Num { raw, .. } = arg.unparen() else {
                return Err(error());
            };
            let value: Value = serde_json::from_str(&format!("-{raw}")).map_err(|_| error())?;
            Ok(Compute::Literal(value))
        }
        Expr::Member {
            obj,
            prop: MemberProp::Name(field),
            optional: false,
            ..
        } => {
            if matches!(obj.unparen(),Expr::Ident(i) if i.name==param) && field.name == "id" {
                return Ok(Compute::Id);
            }
            if let Expr::Member {
                obj: row,
                prop: MemberProp::Name(data),
                optional: false,
                ..
            } = obj.unparen()
            {
                if data.name == "data" && matches!(row.unparen(),Expr::Ident(i) if i.name==param) {
                    return Ok(Compute::Field(field.name.clone()));
                }
            }
            Err(error())
        }
        Expr::Binary {
            op, left, right, ..
        } if matches!(*op, "+" | "-" | "*" | "/" | "??") => Ok(Compute::Binary(
            op.to_string(),
            Box::new(child(left)?),
            Box::new(child(right)?),
        )),
        Expr::Call {
            callee,
            args,
            optional: false,
            ..
        } if args.is_empty() => {
            let Expr::Member {
                obj,
                prop: MemberProp::Name(method),
                optional: false,
                ..
            } = callee.unparen()
            else {
                return Err(error());
            };
            match method.name.as_str() {
                "trim" => Ok(Compute::Trim(Box::new(child(obj)?))),
                "toLowerCase" => Ok(Compute::Lower(Box::new(child(obj)?))),
                _ => Err(error()),
            }
        }
        _ => Err(error()),
    }
}
pub fn parse(name: &str, source: &str, path: &Path) -> Result<Definition, String> {
    if !brain_backend::contract::identifier(name) || source.len() > 16384 {
        return Err(error());
    }
    let component = brain_parse::parse(source, path).map_err(|_| error())?;
    if !component.meta.iter().any(|attr| attr.name == "server") {
        return Err(error());
    }
    let [Stmt::ExportDefault { expr, .. }] = component.script.as_slice() else {
        return Err(error());
    };
    let Expr::Object { props, .. } = expr.unparen() else {
        return Err(error());
    };
    if props.len() != 3 {
        return Err(error());
    }
    let (mut resource, mut field, mut compute) = (None, None, None);
    for prop in props {
        let ObjProp::KeyValue {
            key: property,
            value,
            ..
        } = prop
        else {
            return Err(error());
        };
        match key(property)? {
            "resource" if resource.is_none() => {
                let Expr::Str { value, .. } = value.unparen() else {
                    return Err(error());
                };
                resource = Some(value.clone())
            }
            "field" if field.is_none() => {
                let Expr::Str { value, .. } = value.unparen() else {
                    return Err(error());
                };
                field = Some(value.clone())
            }
            "compute" if compute.is_none() => {
                let Expr::Func(f) = value.unparen() else {
                    return Err(error());
                };
                if !f.is_arrow || f.is_async || f.params.len() != 1 {
                    return Err(error());
                }
                let Pat::Ident(param) = &f.params[0] else {
                    return Err(error());
                };
                let FnBody::Expr(body) = &f.body else {
                    return Err(error());
                };
                compute = Some(expression(body, &param.name, 0, &mut 0)?)
            }
            _ => return Err(error()),
        }
    }
    Ok(Definition {
        name: name.into(),
        resource: resource.ok_or_else(error)?,
        field: field.ok_or_else(error)?,
        expression: compute.ok_or_else(error)?,
    })
}
const USAGE: &str = "usage: brain data backfill <name> --env development [--limit 100] [--include-null] [--apply --request plan.json] [--json]; preview is read-only and apply processes one atomic batch";
#[derive(Debug)]
struct Options {
    name: String,
    environment: String,
    limit: u32,
    include_null: bool,
    apply: bool,
    request: Option<String>,
    json: bool,
}
fn options(args: &[String]) -> Result<Options, &'static str> {
    let mut options = Options {
        name: args
            .first()
            .filter(|name| brain_backend::contract::identifier(name))
            .ok_or(USAGE)?
            .clone(),
        environment: String::new(),
        limit: 100,
        include_null: false,
        apply: false,
        request: None,
        json: false,
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut i = 1;
    while i < args.len() {
        let flag = &args[i];
        if !seen.insert(flag.clone()) {
            return Err(USAGE);
        }
        match flag.as_str() {
            "--json" => options.json = true,
            "--apply" => options.apply = true,
            "--include-null" => options.include_null = true,
            "--env" | "--limit" | "--request" => {
                i += 1;
                let value = args.get(i).ok_or(USAGE)?;
                match flag.as_str() {
                    "--env" => {
                        if value.is_empty()
                            || value.len() > 64
                            || !value.as_bytes()[0].is_ascii_alphabetic()
                            || !value
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                        {
                            return Err(USAGE);
                        }
                        options.environment = value.clone()
                    }
                    "--limit" => {
                        options.limit = value.parse().map_err(|_| USAGE)?;
                        if !(1..=1000).contains(&options.limit) {
                            return Err(USAGE);
                        }
                    }
                    _ => {
                        if !value.ends_with(".json") {
                            return Err(USAGE);
                        }
                        options.request = Some(value.clone())
                    }
                }
            }
            _ => return Err(USAGE),
        }
        i += 1;
    }
    if options.environment.is_empty() || options.apply != options.request.is_some() {
        return Err(USAGE);
    }
    Ok(options)
}
fn load(root: &Path, opt: &Options) -> Result<(Definition, Option<Request>), String> {
    let relative = format!("server/backfills/{}.jsbrain", opt.name);
    crate::changes::source_path(root, &relative).map_err(|_| error())?;
    let source = crate::map::read_source_limit(root, &relative, 16384).map_err(|_| error())?;
    let definition = parse(&opt.name, &source, Path::new(&relative))?;
    let request = if let Some(relative) = &opt.request {
        crate::changes::source_path(root, relative).map_err(|_| {
            "Backfill request must be a regular current-app JSON file <=16 KiB".to_string()
        })?;
        let source = crate::map::read_source_limit(root, relative, 16384)
            .map_err(|_| "Cannot read bounded current-app backfill request".to_string())?;
        fn unique(value: &brain_lex::Json) -> bool {
            match value {
                brain_lex::Json::Obj(rows) => {
                    let mut names = std::collections::BTreeSet::new();
                    rows.iter()
                        .all(|(key, value)| names.insert(key) && unique(value))
                }
                brain_lex::Json::Arr(rows) => rows.iter().all(unique),
                _ => true,
            }
        }
        if !unique(
            &brain_lex::Json::parse(&source)
                .map_err(|_| "Backfill request must be valid JSON; values are not reported")?,
        ) {
            return Err("Backfill request object keys must be unique; fix: use the unchanged preview output".into());
        }
        let value: Value = serde_json::from_str(&source).map_err(|_| {
            "Backfill request must be JSON from preview; values are not reported".to_string()
        })?;
        Some(
            serde_json::from_value(value.get("request").cloned().unwrap_or(value)).map_err(
                |_| "Backfill request must be the exact request produced by preview".to_string(),
            )?,
        )
    } else {
        None
    };
    Ok((definition, request))
}
struct Runtime(Option<tokio::runtime::Runtime>);
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}
fn run(root: &Path, opt: &Options) -> Result<Value, String> {
    let (definition, request) = load(root, opt)?;
    let resources = crate::backend::contracts(root, "resources")?;
    definition.validate(
        resources
            .iter()
            .find(|r| r.name == definition.resource)
            .ok_or_else(error)?,
    )?;
    let url=crate::env::value(root,&opt.environment,"DATABASE_URL").ok_or("Backfill requires the selected app DATABASE_URL; fix: configure its .brain_env or process environment")?;
    let options=brain_backend::boot::require_verified_tls(&url,opt.environment=="production").map_err(|_|"Backfill connection policy refused; fix: use a valid DATABASE_URL and verify-full TLS for remote production; values are not reported")?;
    let fingerprint = brain_backend::backfill::digest(
        &json!({"environment":opt.environment,"host":options.get_host(),"port":options.get_port(),"database":options.get_database(),"user":options.get_username(),"tls":format!("{:?}",options.get_ssl_mode()),"socket":options.get_socket().map(|path|path.to_string_lossy().to_string())}),
    )?;
    let runtime = Runtime(Some(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| "Cannot initialize bounded backfill runtime")?,
    ));
    runtime.0.as_ref().unwrap().block_on(async{
  let boot=brain_backend::boot::Boot::command(opt.environment=="production");
  let pool=tokio::time::timeout(Duration::from_secs(5),brain_backend::boot::connect(&url,&boot)).await.map_err(|_|"Backfill connection exceeded five seconds")?.map_err(|_|"Backfill cannot connect; inspect the configured app database without logging its connection string")?;
  let result=brain_backend::backfill::run(&pool,&resources,&definition,&fingerprint,opt.include_null,opt.limit,request.as_ref()).await;
  brain_backend::db::close_until(&pool,tokio::time::Instant::now()+Duration::from_secs(1)).await;
  result
 })
}
fn emit(value: &Value, json_output: bool) {
    if json_output {
        println!("{value}")
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        )
    }
}
pub fn entry(args: &[String]) -> ExitCode {
    let json_output = args.iter().any(|arg| arg == "--json");
    let fail = |message: &str| json!({"schemaVersion":1,"ok":false,"applied":false,"error":message,"rowValuesCaptured":false});
    let opt = match options(args) {
        Ok(v) => v,
        Err(message) => {
            emit(&fail(message), json_output);
            return ExitCode::from(2);
        }
    };
    let root = match crate::app_root() {
        Ok(v) => v,
        Err(_) => {
            emit(
                &fail("Run brain data backfill inside the current app"),
                opt.json,
            );
            return ExitCode::from(2);
        }
    };
    match run(&root, &opt) {
        Ok(value) => {
            let ok = value["ok"] == true;
            emit(&value, opt.json);
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(message) => {
            emit(&fail(&message), opt.json);
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_brainscript_parser_accepts_only_the_deterministic_closed_subset() {
        let source = "<meta server>\nexport default {resource:'records',field:'code',compute: row => row.data.label.trim().toLowerCase() + '-' + row.id}";
        assert!(parse("codes", source, Path::new("server/backfills/codes.jsbrain")).is_ok());
        for compute in [
            "row => process.env.KEY",
            "row => Date.now()",
            "row => row.data.label.replace('a','b')",
            "async row => row.data.label",
            "row => { return row.id }",
            "row => row.data['label']",
            "row => row.data.label++",
            "row => fetch('/api')",
        ] {
            let source = format!(
                "<meta server>\nexport default {{resource:'records',field:'code',compute:{compute}}}"
            );
            assert!(
                parse(
                    "codes",
                    &source,
                    Path::new("server/backfills/codes.jsbrain")
                )
                .is_err(),
                "{compute}"
            );
        }
    }
    #[test]
    fn flags_require_an_explicit_environment_and_apply_request_pair() {
        fn args(v: &[&str]) -> Vec<String> {
            v.iter().map(|s| s.to_string()).collect()
        }
        assert!(options(&args(&["codes", "--env", "development"])).is_ok());
        for v in [
            args(&["codes"]),
            args(&["codes", "--env", "development", "--apply"]),
            args(&["codes", "--env", "development", "--request", "plan.json"]),
            args(&["codes", "--env", "production", "--limit", "1001"]),
            args(&["codes", "--env", "production", "--env", "development"]),
            args(&["../other", "--env", "development"]),
        ] {
            assert!(options(&v).is_err());
        }
    }
}
