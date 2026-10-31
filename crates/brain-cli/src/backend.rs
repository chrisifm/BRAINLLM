//! Experimental backend CLI. Static BrainScript contracts are parsed by brain-parse, never evaluated.
use brain_backend::{
    Field, Policy, Resource,
    contract::{self, Hooks},
};
use brain_parse::ast::{Expr, ObjProp, PropKey, Stmt};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

#[path = "socket_settings.rs"]
mod socket_configuration;
pub use socket_configuration::{socket_settings, SocketSettings};

fn literal(expr: &Expr) -> Result<Value, String> {
    match expr.unparen() {
        Expr::Str{value,..}=>Ok(json!(value)), Expr::Bool{value,..}=>Ok(json!(value)), Expr::Null(_)=>Ok(Value::Null),
        Expr::Num{raw,..}=>number(raw,false),
        Expr::Unary{op:"-",arg,..}=>match arg.unparen() {Expr::Num{raw,..}=>number(raw,true),_=>Err("Contract numbers must be plain decimal literals".into())},
        Expr::Object{props,..}=>{
            let mut obj=serde_json::Map::new();
            for p in props {
                let ObjProp::KeyValue{key,value,..}=p else{return Err("Contract objects allow only named literal properties".into())};
                let name=match key {PropKey::Ident(i)=>&i.name,PropKey::Str{value,..}=>value,_=>return Err("Contract property names must be identifiers or strings".into())};
                if obj.insert(name.clone(),literal(value)?).is_some(){return Err(format!("Duplicate contract property `{name}`"))}
            }
            Ok(Value::Object(obj))
        },
        Expr::Array{elems,..}=>elems.iter().map(|e|e.as_ref().ok_or_else(||"Contract arrays cannot have holes".into()).and_then(literal)).collect::<Result<Vec<_>,_>>().map(Value::Array),
        _=>Err("Contract must export one literal object; no function calls, imports or computed values".into()),
    }
}
/// Plain JSON-compatible decimal literals only; integers outside i64 are refused instead of rounded.
fn number(raw: &str, negative: bool) -> Result<Value, String> {
    let text = if negative { format!("-{raw}") } else { raw.to_string() };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Number(n)) if !(n.is_f64() && raw.bytes().all(|c| c.is_ascii_digit())) => Ok(Value::Number(n)),
        _ => Err(format!("Contract number `{text}` must be a plain decimal literal within the 64-bit integer range; write exact decimals as strings")),
    }
}
pub(crate) fn read_contract(root: &Path, path: &Path) -> Result<Value, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Contract must be inside the app and <=64 KiB")?
        .to_str()
        .ok_or("Contract path must be UTF-8")?;
    crate::changes::source_path(root, relative).map_err(|error| format!("{relative}: {error}"))?;
    let src = crate::map::read_source_limit(root, relative, 65536).map_err(|error| {
        format!("{relative}: contract must be a regular app source file <=64 KiB ({error})")
    })?;
    let c = brain_parse::parse(&src, Path::new(relative)).map_err(|d| crate::diags_json(&d))?;
    let result = match c.script.as_slice() {
        [Stmt::ExportDefault { expr, .. }] => literal(expr),
        _ => Err(
            "Contract must contain only `export default { ... }` (optional <meta server>)".into(),
        ),
    };
    result.map_err(|message| {
        format!(
            "{}: {message}",
            path.strip_prefix(root).unwrap_or(path).display()
        )
    })
}
fn fields(value: Value) -> Result<BTreeMap<String, Field>, String> {
    let obj = value
        .as_object()
        .ok_or("Schema must be an object of field:type pairs")?;
    obj.iter()
        .map(|(name, v)| {
            // `'int!'` or the object form `{ type: 'int!', default: 0, unique: true }`.
            let (kind, default, unique) = match v {
                Value::String(kind) => (kind.as_str(), None, false),
                Value::Object(spec) => {
                    if let Some(key) = spec.keys().find(|k| !matches!(k.as_str(), "type" | "default" | "unique")) {
                        return Err(format!("field `{name}`: unknown option `{key}`; fix: use type, default and unique"));
                    }
                    let kind = spec.get("type").and_then(Value::as_str).ok_or_else(|| format!("field `{name}` needs type: 'str' (or another kind)"))?;
                    let unique = match spec.get("unique") {
                        None => false,
                        Some(Value::Bool(b)) => *b,
                        Some(_) => return Err(format!("field `{name}`: unique must be true or false")),
                    };
                    (kind, spec.get("default").cloned(), unique)
                }
                _ => return Err("Field shape must be a string or { type, default, unique }".into()),
            };
            Ok((
                name.clone(),
                Field {
                    kind: kind.trim_end_matches('!').into(),
                    required: kind.ends_with('!'),
                    default,
                    unique,
                },
            ))
        })
        .collect()
}
pub fn contracts(root: &Path, kind: &str) -> Result<Vec<Resource>, String> {
    let dir = root.join("server").join(kind);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());
    let mut out = Vec::new();
    for e in entries {
        if !e.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        // Legacy server subfolders without resource/job metadata are not silently reinterpreted as new contracts.
        if !["schema.jsbrain", "policy.jsbrain", "hooks.jsbrain"]
            .iter()
            .any(|file| e.path().join(file).is_file())
        {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let fields = fields(read_contract(root, &e.path().join("schema.jsbrain"))?)
            .map_err(|message| format!("server/{kind}/{name}/schema.jsbrain: {message}"))?;
        let policy = if kind == "resources" {
            serde_json::from_value(read_contract(root, &e.path().join("policy.jsbrain"))?).map_err(
                |_| format!("{name}: policy must declare read/create/update/delete lists"),
            )?
        } else {
            Policy::default()
        };
        let hooks_path = e.path().join("hooks.jsbrain");
        let hooks = if hooks_path.exists() {
            serde_json::from_value(read_contract(root, &hooks_path)?).map_err(|_| {
                format!(
                    "{name}: hooks allow only before_create/before_update/before_delete module names and after_create/after_update/after_delete queue names"
                )
            })?
        } else {
            Hooks::default()
        };
        let search_path = e.path().join("search.jsbrain");
        let search = if kind == "resources" && search_path.is_file() {
            serde_json::from_value(read_contract(root, &search_path)?)
                .map_err(|_| "search.jsbrain must export a list of public string field names")?
        } else {
            Vec::new()
        };
        let constraints_path = e.path().join("constraints.jsbrain");
        let constraints = if kind == "resources" && constraints_path.is_file() {
            serde_json::from_value(read_contract(root, &constraints_path)?).map_err(|_| {
                format!("server/{kind}/{name}/constraints.jsbrain must export {{ unique: [[field, field]], check: [{{ field, min, max }} | {{ compare: [a, op, b] }} | {{ together: [a, b] }}] }}")
            })?
        } else {
            Default::default()
        };
        let r = Resource {
            search,
            name,
            fields,
            policy,
            hooks,
            constraints,
        };
        r.validate()
            .map_err(|message| format!("server/{kind}/{}: {message}", r.name))?;
        out.push(r);
    }
    Ok(out)
}
pub fn auth_roles(root: &Path) -> Result<brain_backend::auth_flows::RolesConfig, String> {
    let path = root.join("server/auth/roles.jsbrain");
    let roles = if path.symlink_metadata().is_ok() {
        serde_json::from_value(read_contract(root, &path)?)
            .map_err(|_| "server/auth/roles.jsbrain must declare roles and signup_role")?
    } else {
        brain_backend::auth_flows::RolesConfig::default()
    };
    roles.validate()?;
    Ok(roles)
}
fn validate_roles(
    roles: &brain_backend::auth_flows::RolesConfig,
    resources: &[Resource],
) -> Result<(), String> {
    for resource in resources {
        for grants in [
            &resource.policy.read,
            &resource.policy.create,
            &resource.policy.update,
            &resource.policy.delete,
        ] {
            for role in grants {
                if !matches!(role.as_str(), "public" | "owner") && !roles.roles.contains(role) {
                    return Err(format!(
                        "server/resources/{}/policy.jsbrain: role {role} is not declared in server/auth/roles.jsbrain",
                        resource.name
                    ));
                }
            }
        }
    }
    Ok(())
}
pub fn inspect(root: &Path) -> Result<Value, String> {
    let resources = contracts(root, "resources")?;
    let jobs = contracts(root, "jobs")?;
    contract::validate_graph(&resources, &jobs)?;
    let roles = auth_roles(root)?;
    validate_roles(&roles, &resources)?;
    let mut openapi = contract::openapi_with_roles(&resources, &roles);
    // Routes need their signing secrets to load; inspection without them documents buckets only.
    let routes = crate::route_executor::contracts(root, "development").unwrap_or_default();
    let buckets = crate::platform_config::load(root, &resources, &roles).map(|p| p.buckets).unwrap_or_default();
    brain_backend::openapi::extend(&mut openapi, &routes, &buckets);
    Ok(
        json!({"schemaVersion":1,"experimental":true,"resources":resources,"jobs":jobs,"roles":roles,"webhooks":crate::webhook_config::metadata(root)?,"platform":crate::platform_config::metadata(root,&resources,&roles)?,"openapi":openapi,"limits":{"activeJobs":10000,"workers":8,"jobSeconds":60,"leaseSeconds":90},"delivery":"at-least-once; handlers must be idempotent"}),
    )
}
fn database(root: &Path, environment: &str) -> Result<String, String> {
    std::env::var("DATABASE_URL")
        .ok()
        .or_else(|| crate::env::value(root, environment, "DATABASE_URL"))
        .filter(|s| !s.is_empty())
        .ok_or(
            "Set DATABASE_URL in .brain_env or the process environment; Postgres is required"
                .into(),
        )
}
fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}
pub fn run(args: &[String]) -> Result<(), String> {
    let root = crate::app_root()?;
    let environment = crate::env::name(args, "development");
    let operation = args.first().map(String::as_str).unwrap_or("schema");
    if !matches!(
        operation,
        "schema" | "migrate" | "serve" | "bootstrap-admin"
    ) {
        return Err("usage: brain backend schema|migrate|serve|bootstrap-admin --user <uuid> [--env development] [--port 5182] [--origin http://127.0.0.1:5182]".into());
    }
    let snapshot = inspect(&root)?;
    if operation == "schema" {
        println!("{snapshot}");
        return Ok(());
    }
    let url = database(&root, &environment)?;
    let roles = auth_roles(&root)?;
    let resources = contracts(&root, "resources")?;
    let jobs = contracts(&root, "jobs")?;
    runtime()?.block_on(async{
        let production=environment=="production";
        let boot=if operation=="serve" {brain_backend::boot::Boot::server(production)} else {brain_backend::boot::Boot::command(production)};
        let pool=brain_backend::boot::connect(&url,&boot).await?;
        if operation=="migrate"{
            brain_backend::db::migrate(&pool,&resources).await?;
            roles.register(&pool).await?;
            pool.close().await;
            println!("{}",json!({"schemaVersion":1,"ok":true,"resources":resources.len()}));return Ok(())
        }
        if operation=="bootstrap-admin" {
            brain_backend::db::verify(&pool,&resources).await?;
            let user=crate::flag(args,"--user").ok_or("bootstrap-admin requires the explicit UUID of an existing enabled user; no account is created")?.parse().map_err(|_|"user must be a UUID")?;
            let promoted=brain_backend::auth_flows::bootstrap_first_admin(&pool,user).await.map_err(|error|error.to_string())?;
            pool.close().await;
            if !promoted {return Err("Admin bootstrap requires an enabled existing user and no active administrator".into());}
            println!("{}",json!({"schemaVersion":1,"ok":true,"user":user,"role":"admin","sessionsRevoked":true}));return Ok(());
        }
        let port=crate::flag(args,"--port").unwrap_or("5182".into()).parse::<u16>().map_err(|_|"port must be 0..65535")?;
        let listener=tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,port)).await.map_err(|_|"Cannot bind backend loopback port")?;
        let addr=listener.local_addr().map_err(|e|e.to_string())?;
        let public_site=crate::env::value(&root,&environment,"PUBLIC_SITE_URL").unwrap_or_else(||format!("http://{addr}"));
        let sockets=socket_settings(&root,Some(&public_site),crate::flag(args,"--origin").as_deref())?;
        let origin=sockets.http_origins.first().cloned().ok_or("Set an explicit backend HTTP origin")?;
        // Never migrate on serve: database changes are an explicit, reviewable command.
        let setting=|key:&str|std::env::var(key).ok().filter(|v|!v.is_empty()).or_else(||crate::env::value(&root,&environment,key));
        let jwt=setting("BRAIN_JWT_SECRET").map(|secret|brain_backend::jwt::Config::new(secret,setting("BRAIN_JWT_ISSUER").unwrap_or_else(||origin.clone()),setting("BRAIN_JWT_AUDIENCE").unwrap_or_else(||origin.clone()))).transpose()?;
        let google=setting("GOOGLE_CLIENT_ID").map(brain_backend::google::Config::new).transpose()?;
        if google.is_some() && jwt.is_none() {return Err("Google Sign-In requires configured native JWT sessions; set BRAIN_JWT_SECRET, issuer and audience".into());}
        let gcs=match setting("BRAIN_GCS_BUCKET") {
            Some(bucket)=>Some(brain_backend::media::GcsConfig::new(bucket,setting("BRAIN_GCS_PREFIX").unwrap_or_else(||"brain-media".into()),std::path::PathBuf::from(setting("BRAIN_GCS_ADAPTER").ok_or("BRAIN_GCS_ADAPTER must point at the private official SDK adapter")?))?),
            None=>None,
        };
        let public_base=setting("PUBLIC_SITE_URL").unwrap_or_else(||origin.clone());
        let mail=setting("BRAIN_MAIL_RESEND_KEY").map(|key|brain_backend::auth_flows::MailConfig::resend(key,setting("BRAIN_MAIL_FROM").ok_or("BRAIN_MAIL_FROM must name a verified sender email")?,public_base.clone())).transpose()?;
        let config=brain_backend::Config{sockets:sockets.native_config()?,before_hooks:crate::hooks_executor::load(&root,&environment,&resources)?,webhooks:crate::webhook_config::load(&root,&environment)?,roles,mail,auth_public_base_url:Some(public_base),gcs,jwt,google,resources,jobs,uploads:root.join(".brain/uploads"),origins:sockets.http_origins,production:environment=="production"};
        eprintln!("{}",json!({"schemaVersion":1,"event":"backend-starting","url":format!("http://{addr}"),"experimental":true}));
        let mut platform=crate::platform_config::load(&root,&config.resources,&config.roles)?;
        platform.routes=crate::route_executor::contracts(&root,&environment)?;
        platform.handlers=crate::route_executor::load(&root,&environment,&platform.routes)?;
        brain_backend::http::serve_with_platform(listener,pool,config,platform).await
    })
}
/// All generated files are reserved with create_new; roll back only files created by this invocation.
pub fn add(root: &Path, kind: &str, name: &str, shapes: &str) -> Result<Value, String> {
    add_with_admin(root, kind, name, shapes, false)
}
/// Opt-in project-owned administrative views committed with the generated backend contract.
pub fn add_with_admin(root: &Path, kind: &str, name: &str, shapes: &str, admin_ui: bool) -> Result<Value, String> {
    if admin_ui && kind != "resource" {
        return Err("--admin-ui is only supported for resources".into());
    }
    let _writer = if admin_ui { None } else { Some(crate::edit::writer_lock(root)?) };
    if !contract::identifier(name) {
        return Err(
            "Use a lowercase resource/job identifier (letters, digits, underscores)".into(),
        );
    }
    let mut pairs = BTreeMap::new();
    for shape in shapes.split_whitespace() {
        let (name, shape) = shape
            .split_once(':')
            .ok_or("fields must be name:type pairs")?;
        if pairs.insert(name.to_string(), json!(shape)).is_some() {
            return Err("Duplicate field".into());
        }
    }
    let fields = fields(json!(pairs))?;
    let r = Resource {
        search: Vec::new(),
        name: name.into(),
        fields,
        policy: Policy::default(),
        hooks: Hooks::default(),
        constraints: Default::default(),
    };
    r.validate()?;
    let dir = if kind == "resource" {
        "resources"
    } else if kind == "job" {
        "jobs"
    } else {
        return Err("kind must be resource or job".into());
    };
    let prefix = format!("server/{dir}/{name}");
    let mut files = vec![(
        format!("{prefix}/schema.jsbrain"),
        format!(
            "<meta server>\nexport default {}\n",
            serde_json::to_string_pretty(&pairs).unwrap()
        ),
    )];
    if kind == "resource" {
        files.push((format!("{prefix}/search.jsbrain"), "<meta server>\n// Explicit public string fields only. Never index private data or full JSON bodies.\nexport default []\n".into()));
        files.push((
            format!("{prefix}/policy.jsbrain"),
            format!(
                "<meta server>\nexport default {}\n",
                serde_json::to_string_pretty(&r.policy).unwrap()
            ),
        ));
        files.push((format!("{prefix}/hooks.jsbrain"),"<meta server>\n// Set before_create/before_update/before_delete to a server/hooks module name; return ctx.payload.\n// Set after_create/after_update/after_delete to a job name; events commit with the mutation.\nexport default {}\n".into()));
    } else {
        files.push((format!("{prefix}/handler.jsbrain"),"<meta server>\n// ctx: payload, jobId, attempt, env. Delivery is at least once; make effects idempotent by jobId.\nexport default async function handle(ctx) {\n  throw new Error('Implement this job handler before starting a worker')\n}\n".into()));
    }
    if kind == "resource" {
        files.extend(resource_support_files(&r, &files[0].1)?);
    }
    if admin_ui {
        files.extend(crate::resource_admin::files(root, &r)?);
        let contract_path = format!("server/resources/{name}/client-contract.json");
        let (_, source) = files.iter_mut().find(|(path, _)| path == &contract_path)
            .ok_or("Generated resource contract is missing")?;
        let mut contract: Value = serde_json::from_str(source).map_err(|_| "Cannot read generated resource contract")?;
        contract["adminUi"] = json!({"status":"generated","routes":[format!("/admin/{name}"),format!("/admin/{name}/new"),format!("/admin/{name}/:id"),format!("/admin/{name}/:id/edit")],"guard":format!("views/admin/{name}/_guard.jsbrain"),"projectOwned":true});
        contract["requirements"] = json!(["Review native policy and run explicit backend migration","Configure fixed BRAIN_BACKEND_URL for production and development","Sign in with an existing administrator; signup never grants admin","Run build and doctor after adapting project-owned views and controls"]);
        *source = serde_json::to_string_pretty(&contract).map_err(|_| "Cannot encode administrative contract")? + "\n";
        // Publish generated status only after every route/control has been installed.
        let index = files.iter().position(|(path, _)| path == &contract_path).ok_or("Generated resource contract is missing")?;
        let contract_file = files.remove(index);
        files.push(contract_file);
    }
    for (rel, _) in &files {
        for p in root.join(rel).ancestors().take_while(|p| *p != root) {
            if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err("Generator refuses symlink paths".into());
            }
        }
        if root.join(rel).exists() {
            return Err(format!(
                "{rel} already exists; preserve it and edit its contract"
            ));
        }
    }
    if admin_ui {
        return crate::resource_admin::commit(root, &files);
    }
    let mut written = Vec::new();
    let result = (|| {
        for (rel, body) in &files {
            crate::changes::create_source(root, rel, body)?;
            written.push(root.join(rel));
        }
        Ok::<(), String>(())
    })();
    if let Err(e) = result {
        for p in written {
            let _ = fs::remove_file(p);
        }
        return Err(e);
    }
    Ok(
        json!({"schemaVersion":1,"files":files.iter().map(|(p,_)|p).collect::<Vec<_>>(),"experimental":true,"next":if kind=="resource"{"Review policy; run brain backend schema then brain backend migrate"}else{"Implement handler; inspect schema then start brain jobs work"}}),
    )
}

/// Native contracts and executable server/client helpers; routes are an explicit opt-in.
fn resource_support_files(
    r: &Resource,
    schema_source: &str,
) -> Result<Vec<(String, String)>, String> {
    let name = &r.name;
    let repository = format!("server/resources/{name}/repository.jsbrain");
    let service = format!("server/services/{name}.jsbrain");
    let client = format!("api/{name}.jsbrain");
    let record_client = format!("api/{name}-record.jsbrain");
    let mut files = vec![];
    files.push((repository.clone(),format!("<meta server>\n// Request-scoped native policy and identity; never create another database client.\nexport default function repository(ctx) {{\n  if (!ctx.backend || typeof ctx.backend.resource !== 'function') throw Error('Native backend is not configured')\n  return ctx.backend.resource('{name}')\n}}\n")));
    files.push((service.clone(),format!(r#"<meta server>
import repository from '../resources/{name}/repository.jsbrain'
import schema from '../resources/{name}/schema.jsbrain'
function admin(ctx) {{
  if (!ctx.backend) throw Error('Native backend is not configured')
  if (!ctx.user || ctx.user.role !== 'admin') throw {{status:403,message:'Administrator access required'}}
}}
function values(input) {{
  if (!input || typeof input !== 'object' || Array.isArray(input)) throw {{errors:{{form:'Use a resource object'}}}}
  for (const key of Object.keys(input)) {{
    if (!Object.prototype.hasOwnProperty.call(schema,key)) throw {{errors:{{form:'Remove undeclared resource fields'}}}}
  }}
  for (const key of Object.keys(schema)) {{
    if (schema[key].endsWith('!') && input[key] == null) {{
      let errors = {{}}
      errors[key] = 'Required'
      throw {{errors}}
    }}
  }}
  return {{...input}}
}}
async function list(ctx,query) {{ admin(ctx); return await repository(ctx).list(query) }}
async function get(ctx,id) {{ admin(ctx); return await repository(ctx).get(id) }}
async function save(ctx,id,input) {{
  admin(ctx)
  const payload=values(input)
  return id ? await repository(ctx).update(id,payload) : await repository(ctx).create(payload)
}}
async function remove(ctx,id) {{ admin(ctx); return await repository(ctx).remove(id) }}
async function editor(ctx,id) {{
  admin(ctx)
  const source=id ? (await repository(ctx).get(id)).data : {{}}
  let result={{}}
  for (const key of Object.keys(schema)) {{
    const kind=schema[key].replace('!','')
    const value=source[key]
    result[key]=kind === 'obj' ? (value == null ? '' : JSON.stringify(value)) : kind === 'bool' ? Boolean(value) : kind === 'num' ? (value == null ? '' : value) : value == null ? '' : String(value)
  }}
  return result
}}
async function saveForm(ctx,id,input) {{
  admin(ctx)
  const payload=values(input)
  for (const key of Object.keys(payload)) {{
    const kind=schema[key].replace('!','')
    const value=payload[key]
    let invalid=false
    if (kind === 'obj') {{
      try {{ payload[key]=value === '' ? {{}} : JSON.parse(value) }} catch {{ invalid=true }}
      if (!payload[key] || typeof payload[key] !== 'object' || Array.isArray(payload[key])) invalid=true
    }} else if (kind === 'bool') {{
      if (typeof value !== 'boolean') invalid=true
    }} else if (kind === 'num') {{
      if (value === '' || value == null) payload[key]=null
      else if (typeof value === 'number') {{if (!Number.isFinite(value)) invalid=true}}
      else if (typeof value !== 'string' || !/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(value) || !Number.isFinite(Number(value))) invalid=true
      else payload[key]=Number(value)
    }} else if ((kind === 'str' || kind === 'email') && value != null) {{
      if (typeof value !== 'string' || (schema[key].endsWith('!') && !value.trim())) invalid=true
    }}
    if (invalid) {{let errors={{}};errors[key]='Use a valid field value';throw {{errors}}}}
  }}
  return await save(ctx,id,payload)
}}
export default {{list,get,save,saveForm,remove,editor}}
"#)));
    files.push((client.clone(),format!("// Native resource schema supplies the inferred list/row shape; private data is never cached.\nexport default api('/api/{name}',{{get:{{}},post:{{}}}})\n")));
    files.push((
        record_client.clone(),
        format!("export default api('/api/{name}/:id',{{get:{{}},put:{{}},delete:{{}}}})\n"),
    ));
    let contract_path = format!("server/resources/{name}/client-contract.json");
    files.push((contract_path,serde_json::to_string_pretty(&json!({"schemaVersion":1,"resource":name,"schema":format!("server/resources/{name}/schema.jsbrain"),"schemaRevision":crate::manifests::hash(schema_source.as_bytes()),"fields":r.fields,"policy":format!("server/resources/{name}/policy.jsbrain"),"client":[client,record_client],"repository":repository,"service":service,"adminUi":{"status":"planned","reason":"Administrative routes require explicit --admin-ui opt-in"},"authorization":"Native backend policy is authoritative; generated services additionally require authoritative admin context","requirements":["Review policy and run explicit backend migration","Configure fixed BRAIN_BACKEND_URL for production and development","Sign in with an existing administrator; signup never grants admin","Build application views deliberately; this command does not create routes, guards or controls"],"generatedSnapshot":true})).map_err(|_|"Cannot encode generated resource contract")?+"\n"));
    Ok(files)
}


pub(crate) struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn worker_module(root: &Path, name: &str) -> Result<(Scratch, PathBuf), String> {
    let rel = format!("server/jobs/{name}/handler.jsbrain");
    server_module(root, &rel)
}
pub(crate) fn server_module(root: &Path, rel: &str) -> Result<(Scratch, PathBuf), String> {
    let mut files = BTreeMap::new();
    let mut written = BTreeMap::new();
    let compile = |src: &str, path: &Path| {
        crate::compiler::compile(src, path, root, true)
            .map(|o| brain_pack::Unit {
                js: o.js,
                css: o.css,
                server_js: o.server_js,
                server_fns: o.server_fns,
            })
            .map_err(|d| crate::diags_json(&d))
    };
    let native: BTreeMap<String, String> = crate::runtime_files()
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "brain.js" | "brain-budget.js" | "brain-time.js" | "brain-hash.js" | "brain-outbound.js"
            )
        })
        .collect();
    let used = std::cell::RefCell::new(std::collections::BTreeSet::new());
    let entry = brain_pack::server::emit(
        root,
        &rel,
        &compile,
        &|name| {
            native.contains_key(name).then(|| {
                used.borrow_mut().insert(name.to_string());
                format!("./_runtime/{name}.mjs")
            })
        },
        &mut written,
        &mut files,
        0,
    )?;
    let mut pending: Vec<String> = used.into_inner().into_iter().collect();
    let mut emitted = std::collections::HashSet::new();
    while let Some(name) = pending.pop() {
        if !emitted.insert(name.clone()) {
            continue;
        }
        let mut code = native
            .get(&name)
            .ok_or("Job imports an unsupported native runtime module")?
            .clone();
        if name == "brain.js" {
            let keep = ["_rt", "fx", "untrack"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            code = brain_min::tree_shake(&code, &keep)?;
        }
        let code = brain_pack::rewrite_imports(&code, &mut |specifier| {
            if let Some(dependency) = specifier.strip_prefix("./") {
                if !native.contains_key(dependency) {
                    return Err("Job runtime dependency is not supported on the backend".into());
                }
                pending.push(dependency.to_owned());
                return Ok(Some(format!("./{dependency}.mjs")));
            }
            Ok(None)
        })?;
        files.insert(format!("_server/_runtime/{name}.mjs"), code);
    }
    let trace_modules: BTreeMap<_, _> = written
        .iter()
        .map(|(source, generated)| {
            (
                generated.trim_start_matches("_server/").to_string(),
                source.clone(),
            )
        })
        .collect();
    let trace_index = json!({"schemaVersion":1,"modules":trace_modules}).to_string();
    if trace_modules.len() > 2048 || trace_index.len() > 262144 {
        return Err(
            "Job module graph exceeds the private trace-index limit; split the handler's imports"
                .into(),
        );
    }
    files.insert("_server/trace-index.json".into(), trace_index);
    let stamp = brain_pack::content_hash(&format!("{:?}", std::time::SystemTime::now()));
    let scratch = Scratch(
        root.join(".brain/jobs")
            .join(format!("{}-{stamp}", std::process::id())),
    );
    for (name, code) in files {
        let path = scratch.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(path, code).map_err(|e| e.to_string())?;
    }
    let path = fs::canonicalize(scratch.0.join(entry)).map_err(|e| e.to_string())?;
    Ok((scratch, path))
}

pub fn jobs(args: &[String]) -> Result<(), String> {
    let root = crate::app_root()?;
    let environment = crate::env::name(args, "development");
    let op = args.first().map(String::as_str).unwrap_or("");
    if !matches!(op, "enqueue" | "status" | "work" | "prune") {
        return Err("usage: brain jobs enqueue <queue> --payload '<json>' [--key K] | status <id> | work <queue> [--concurrency 2] | prune".into());
    }
    let snapshot = inspect(&root)?;
    let url = database(&root, &environment)?;
    let name = args.get(1).cloned().unwrap_or_default();
    let job_contract = if matches!(op, "enqueue" | "work") {
        Some(
            contracts(&root, "jobs")?
                .into_iter()
                .find(|j| j.name == name)
                .ok_or("Unknown job; use brain add job and brain backend schema")?,
        )
    } else {
        None
    };
    let module = if op == "work" {
        Some(worker_module(&root, &name)?)
    } else {
        None
    };
    let env: Vec<_> = crate::env::select(&root, &environment);
    let _ = snapshot;
    runtime()?.block_on(async {
        let production = environment == "production";
        let boot = if op == "work" { brain_backend::boot::Boot::server(production) } else { brain_backend::boot::Boot::command(production) };
        let pool = brain_backend::boot::connect(&url, &boot).await?;
        brain_backend::db::verify(&pool, &contracts(&root, "resources")?).await?;
        let queue = brain_backend::jobs::Queue::new(pool.clone());
        let webhooks = if op == "work" {
            crate::webhook_config::load(&root,&environment)?.map(|config|brain_backend::webhooks::Webhooks::new(pool.clone(),config,&contracts(&root,"jobs")?)).transpose()?
        } else {None};

        match op {
            "enqueue" => {
                let payload = crate::flag(args, "--payload").ok_or("Missing --payload JSON")?;
                let payload: Value =
                    serde_json::from_str(&payload).map_err(|_| "Invalid payload JSON")?;
                let payload = job_contract
                    .as_ref()
                    .unwrap()
                    .input(&payload)
                    .map_err(|e| json!({"code":"INVALID_JOB_PAYLOAD","fields":e}).to_string())?;
                let id = queue
                    .enqueue(&name, payload, crate::flag(args, "--key").as_deref(), 3)
                    .await?;
                println!("{}", json!({"schemaVersion":1,"id":id}));
            }
            "status" => {
                let id = name.parse().map_err(|_| "Job id must be a UUID")?;
                println!(
                    "{}",
                    json!({"schemaVersion":1,"job":queue.status(id).await?})
                );
            }
            "prune" => {
                println!(
                    "{}",
                    json!({"schemaVersion":1,"removed":queue.prune().await?})
                );
            }
            "work" => {
                let n = crate::flag(args, "--concurrency")
                    .unwrap_or("2".into())
                    .parse()
                    .map_err(|_| "Concurrency must be 1..8")?;
                if !(1..=8).contains(&n) {
                    return Err("Concurrency must be 1..8".into());
                }
                let module = module.as_ref().unwrap().1.clone();
                let contract = job_contract.clone().unwrap();
                let (send, recv) = tokio::sync::watch::channel(false);
                eprintln!(
                    "{}",
                    json!({"schemaVersion":1,"event":"worker-ready","queue":name,"concurrency":n})
                );
                let worker = queue.work(&name, n, recv, move |mut job| {
                    let (module, env, contract, webhooks) = (module.clone(), env.clone(), contract.clone(), webhooks.clone());
                    async move {
                        if let Some(webhooks) = webhooks {
                            if let Some(payload) = webhooks.decode_job(&job).await.map_err(|_| "Webhook work payload could not be authorized")? {job.payload=payload;}
                        }
                        contract
                            .input(&job.payload)
                            .map_err(|_| "Job payload does not match its schema")?;
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};
                        // Child stderr is untrusted input. Retain a bounded prefix, drain the remainder, and
                        // reconstruct only source/index/coordinate fields; never echo application stderr.
                        fn coordinate(value: &Value) -> Option<u64> {
                            value.as_u64().filter(|n| (1..=10_000_000).contains(n))
                        }
                        fn source_path_valid(source: &str) -> bool {
                            source.len() <= 512
                                && !source.starts_with('/')
                                && !source.contains(['\\', '?', '#'])
                                && !source.chars().any(char::is_control)
                                && source
                                    .split('/')
                                    .all(|p| !p.is_empty() && p != "." && p != "..")
                        }
                        fn clean_frame(frame: &Value, modules: &BTreeMap<String, String>) -> Option<Value> {
                            let generated = frame.get("generated")?;
                            let file = generated.get("file")?.as_str()?;
                            let key = file.strip_prefix("_server/")?;
                            if key.contains('/') || key.len() > 85 {
                                return None;
                            }
                            let source = modules.get(key)?;
                            if !source_path_valid(source) {
                                return None;
                            }
                            let generated_line = coordinate(&generated["line"])?;
                            let generated_column = coordinate(&generated["column"]);
                            let line = if frame["precision"] == "source-line"
                                && frame["file"].as_str() == Some(source.as_str())
                            {
                                coordinate(&frame["line"])
                            } else {
                                None
                            };
                            Some(json!({"file":source,"line":line,"column":Value::Null,
                            "precision":if line.is_some(){"source-line"}else{"module-only"},
                            "generated":{"file":file,"line":generated_line,"column":generated_column}}))
                        }
                        fn clean_group(
                            group: &Value,
                            modules: &BTreeMap<String, String>,
                            remaining: &mut usize,
                        ) -> Value {
                            let mut frames = Vec::new();
                            if let Some(items) = group["frames"].as_array() {
                                for frame in items.iter().take(24) {
                                    if *remaining == 0 {
                                        break;
                                    }
                                    if let Some(clean) = clean_frame(frame, modules) {
                                        frames.push(clean);
                                        *remaining -= 1;
                                    }
                                }
                            }
                            let location = frames.first().cloned().unwrap_or_else(|| {
                            group["location"]["file"].as_str()
                                .filter(|source|source_path_valid(source) && modules.values().any(|known|known.as_str()==*source))
                                .map(|source|json!({"file":source,"line":Value::Null,"column":Value::Null,"precision":"module-only","generated":Value::Null}))
                                .unwrap_or(Value::Null)
                        });
                            let supplied = group["frames"].as_array().map_or(0, Vec::len);
                            json!({"location":location,"frames":frames,"truncated":{
                            "frames":group["truncated"]["frames"].as_bool().unwrap_or(false) || supplied>frames.len(),
                            "stackUnavailable":group["truncated"]["stackUnavailable"].as_bool().unwrap_or(false)}})
                        }
                        let trace_modules: BTreeMap<String, String> = module
                            .parent()
                            .map(|dir| dir.join("trace-index.json"))
                            .filter(|p| fs::metadata(p).is_ok_and(|m| m.len() <= 262144))
                            .and_then(|p| fs::read(p).ok())
                            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                            .filter(|v| v["schemaVersion"] == 1)
                            .and_then(|v| serde_json::from_value(v["modules"].clone()).ok())
                            .filter(|m: &BTreeMap<String, String>| m.len() <= 2048)
                            .unwrap_or_default();
                        let job_id = job.id.to_string();
                        let queue_name = job.queue.clone();
                        let runner = include_str!("templates/job-runner.mjs");
                        let mut cmd = tokio::process::Command::new("node");
                        cmd.args([
                            "--enable-source-maps",
                            "--max-old-space-size=256",
                            "--input-type=module",
                            "--eval",
                            runner,
                        ])
                        .arg(module)
                        .envs(env)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .kill_on_drop(true);
                        #[cfg(unix)]
                        cmd.process_group(0);
                        let mut child = cmd.spawn().map_err(|_| "Cannot start Node job handler")?;
                        let _group = HandlerGroup(child.id());
                        let mut stderr = child.stderr.take().ok_or("Missing handler stderr")?;
                        let bytes = json!({"jobId":job.id,"attempt":job.attempt,"payload":job.payload}).to_string();
                        child
                            .stdin
                            .take()
                            .ok_or("Missing handler stdin")?
                            .write_all(bytes.as_bytes())
                            .await
                            .map_err(|_| "Cannot send job payload")?;
                        let drain = async move {
                            let mut retained = Vec::with_capacity(65536);
                            let mut chunk = [0u8; 4096];
                            let mut total = 0u64;
                            loop {
                                match stderr.read(&mut chunk).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => {
                                        total = total.saturating_add(n as u64);
                                        let keep = n.min(65536usize.saturating_sub(retained.len()));
                                        retained.extend_from_slice(&chunk[..keep]);
                                    }
                                }
                            }
                            (retained, total)
                        };
                        // Queue timeout/cancellation drops this joined future, Child and HandlerGroup together.
                        let (status, (retained, stderr_bytes)) = tokio::join!(child.wait(), drain);
                        let success = status.map_err(|_| "Job process failed")?.success();
                        if !success {
                            let mut trace = Value::Null;
                            for bytes in retained.split(|b| *b == b'\n')
                                .filter_map(|line|line.strip_prefix(b"JOB_TRACE "))
                                .take(16) {
                                let Ok(raw) = serde_json::from_slice::<Value>(bytes) else {
                                    continue;
                                };
                                if raw["schemaVersion"] != 1
                                    || raw["event"] != "job-trace"
                                    || raw["jobId"].as_str() != Some(job_id.as_str())
                                {
                                    continue;
                                }
                                let mut remaining = 24usize;
                                let group = clean_group(&raw, &trace_modules, &mut remaining);
                                let mut causes = Vec::new();
                                if let Some(items) = raw["causes"].as_array() {
                                    for (index, cause) in items.iter().take(6).enumerate() {
                                        let mut group = clean_group(cause, &trace_modules, &mut remaining);
                                        group["depth"] = json!(index + 1);
                                        causes.push(group);
                                    }
                                }
                                let mut truncated = serde_json::Map::new();
                                for key in [
                                    "frames",
                                    "causes",
                                    "stackUnavailable",
                                    "indexUnavailable",
                                    "output",
                                ] {
                                    truncated.insert(
                                        key.into(),
                                        json!(raw["truncated"][key].as_bool().unwrap_or(false)),
                                    );
                                }
                                truncated.insert(
                                    "frames".into(),
                                    json!(
                                        truncated["frames"] == true
                                            || group["truncated"]["frames"] == true
                                            || causes.iter().any(|c| c["truncated"]["frames"] == true)
                                    ),
                                );
                                truncated.insert(
                                    "indexUnavailable".into(),
                                    json!(truncated["indexUnavailable"] == true || trace_modules.is_empty()),
                                );
                                trace = json!({"schemaVersion":1,"location":group["location"],"frames":group["frames"],"causes":causes,"truncated":truncated});
                                break;
                            }
                            eprintln!(
                                "{}",
                                json!({"schemaVersion":1,"event":"job-failed","jobId":job_id,"queue":queue_name,
                            "trace":trace,"stderrBytes":stderr_bytes,"stderrTruncated":stderr_bytes>65536})
                            );
                            Err("Job handler failed".into())
                        } else {
                            Ok(())
                        }
                    }
                });
                tokio::pin!(worker);
                tokio::select! {
                    r=&mut worker=>r?,
                    _=brain_backend::http::wait_shutdown()=>{let _=send.send(true);worker.await?;},
                }
            }
            _ => unreachable!(),
        }
        pool.close().await;
        Ok(())
    })
}

/// Small default discovery response; request one resource/job for its complete contract.
pub fn card(root: &Path, name: Option<&str>) -> Result<Value, String> {
    let snapshot = inspect(root)?;
    if let Some(name) = name {
        let (only, name) = if let Some(n) = name.strip_prefix("resource:") {
            (Some("resources"), n)
        } else if let Some(n) = name.strip_prefix("job:") {
            (Some("jobs"), n)
        } else {
            (None, name)
        };
        for kind in ["resources", "jobs"] {
            if only.is_some_and(|k| k != kind) {
                continue;
            }
            if let Some(c) = snapshot[kind]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"].as_str() == Some(name))
            {
                let generated = format!("server/resources/{name}/client-contract.json");
                let generated_contract = if kind == "resources"
                    && fs::symlink_metadata(root.join(&generated)).is_ok_and(|m| {
                        m.is_file() && !m.file_type().is_symlink() && m.len() <= 32768
                    }) {
                    Some(generated)
                } else {
                    None
                };
                return Ok(
                    json!({"schemaVersion":1,"kind":kind,"contract":c,"generatedContract":generated_contract,"files":if kind=="resources"{vec![format!("server/resources/{name}/schema.jsbrain"),format!("server/resources/{name}/policy.jsbrain"),format!("server/resources/{name}/hooks.jsbrain")]}else{vec![format!("server/jobs/{name}/schema.jsbrain"),format!("server/jobs/{name}/handler.jsbrain")]}}),
                );
            }
        }
        return Err("Unknown backend contract; omit name to list resources/jobs".into());
    }
    let names = |kind: &str| {
        snapshot[kind]
            .as_array()
            .unwrap()
            .iter()
            .take(100)
            .map(|c| c["name"].clone())
            .collect::<Vec<_>>()
    };
    Ok(
        json!({"schemaVersion":1,"experimental":true,"resources":names("resources"),"jobs":names("jobs"),"next":"Request one name for its schema, policy, hooks and owned files. CLI: brain backend schema"}),
    )
}

// Kill only the owned process group on cancellation/timeout, including ordinary child processes of a handler.
// A deliberately detached child is outside this trusted-project runner's containment contract.
struct HandlerGroup(Option<u32>);
impl Drop for HandlerGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_backend_contracts_check_compile_and_preserve_existing_files() {
        let root =
            std::env::temp_dir().join(format!("brain-backend-scaffold-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        crate::scaffold::new_app(&root).unwrap();
        add(&root, "resource", "posts", "title:str! body:str").unwrap();
        add(
            &root,
            "job",
            "post_created",
            "resource:str! operation:str! record:obj!",
        )
        .unwrap();
        fs::write(
            root.join("server/resources/posts/hooks.jsbrain"),
            "<meta server>\nexport default {after_create:'post_created'}\n",
        )
        .unwrap();
        let snapshot = inspect(&root).unwrap();
        assert_eq!(
            snapshot["resources"][0]["fields"]["title"]["required"],
            true
        );
        assert_eq!(
            snapshot["resources"][0]["policy"]["create"],
            json!(["admin"])
        );
        assert!(add(&root, "resource", "posts", "title:bool").is_err());
        let (_, diags) = crate::run_check(Some(root.clone())).unwrap();
        assert!(
            !diags.iter().any(|d| !d.is_warning()),
            "{}",
            crate::diags_json(&diags)
        );
        let (scratch, path) = worker_module(&root, "post_created").unwrap();
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .contains("Implement this job handler")
        );
        drop(scratch);
        fs::write(
            root.join("server/resources/posts/hooks.jsbrain"),
            "<meta server>\nexport default {after_create:'missing_job'}\n",
        )
        .unwrap();
        let (_, diags) = crate::run_check(Some(root.clone())).unwrap();
        assert!(diags.iter().any(|d| d.code == "B0735"));
        fs::remove_dir_all(root).unwrap();
    }
    fn fixture_root(label: &str) -> Scratch {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Scratch(
            std::env::temp_dir().join(format!("brain-{label}-{}-{stamp}", std::process::id())),
        );
        fs::create_dir_all(&root.0).unwrap();
        root
    }
    #[test]
    fn resource_support_preserves_the_default_budget_without_adding_admin_routes() {
        let parent = fixture_root("resource-default-budget");
        let root = parent.0.join("app");
        crate::scaffold::new_app(&root).unwrap();
        for (name, fields) in [
            ("posts", "title:str! body:str"),
            ("metrics", "title:str! active:bool! score:num meta:obj"),
        ] {
            let result = add(&root, "resource", name, fields).unwrap();
            assert!(
                result["files"].as_array().unwrap().iter().all(|path| !path
                    .as_str()
                    .unwrap()
                    .starts_with("views/")
                    && !path.as_str().unwrap().starts_with("ui/"))
            );
            let snapshot: Value = serde_json::from_slice(
                &fs::read(root.join(format!("server/resources/{name}/client-contract.json")))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(snapshot["adminUi"]["status"], "planned");
            assert!(snapshot.get("views").is_none() && snapshot.get("guard").is_none());
            for client in snapshot["client"].as_array().unwrap() {
                assert!(root.join(client.as_str().unwrap()).is_file());
            }
            assert!(
                root.join(snapshot["repository"].as_str().unwrap())
                    .is_file()
            );
            assert!(root.join(snapshot["service"].as_str().unwrap()).is_file());
            let config: Value =
                serde_json::from_slice(&fs::read(root.join("brain.json")).unwrap()).unwrap();
            assert_eq!(config["budget"]["viewKB"], 30);
            crate::run_build(&root, Some("https://example.test".into())).unwrap();
        }
        assert!(!root.join("views/admin").exists());
    }
    #[test]
    fn generated_admin_ui_builds_real_routes_and_rpc_without_overwrites() {
        use std::io::Read;
        let root = fixture_root("generated-crud");
        // Freeze the old rendered fixture identity before measurement: same 45-byte
        // pattern (four-digit process id + nineteen-digit nanosecond timestamp).
        // Its digit sample was captured once, not selected by a passing budget.
        const BRAND: &str = "brain-generated-crud-2580-1793395249408887000";
        assert_eq!(BRAND.len(), 45);
        crate::scaffold::new_app_named(&root.0, BRAND).unwrap();
        let original_control = fs::read(root.0.join("ui/button.brain")).unwrap();
        let added = add_with_admin(
            &root.0,
            "resource",
            "posts",
            "title:str! active:bool! score:num meta:obj",
            true,
        )
        .unwrap();
        assert_eq!(added["adminUi"]["status"], "generated");
        assert_eq!(
            fs::read(root.0.join("ui/button.brain")).unwrap(),
            original_control
        );
        let card = card(&root.0, Some("resource:posts")).unwrap();
        assert_eq!(
            card["generatedContract"],
            "server/resources/posts/client-contract.json"
        );
        let manifest: Value = serde_json::from_slice(
            &fs::read(root.0.join("server/resources/posts/client-contract.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["fields"]["active"]["required"], true);
        assert_eq!(
            manifest["schemaRevision"],
            crate::manifests::hash(
                &fs::read(root.0.join("server/resources/posts/schema.jsbrain")).unwrap()
            )
        );
        assert_eq!(manifest["adminUi"]["status"], "generated");
        assert_eq!(manifest["adminUi"]["routes"].as_array().unwrap().len(), 4);
        for path in ["index.brain", "[id].brain", "new.brain", "[id]/edit.brain"] {
            let source = fs::read_to_string(root.0.join("views/admin/posts").join(path)).unwrap();
            assert!(
                !source.contains("api(") && !source.contains("resource("),
                "Admin requests must use the server service: {path}"
            );
            assert!(
                !source.contains("from '../../../api/")
                    && !source.contains("from '../../../../api/")
            );
            if path == "index.brain" {
                assert!(source.contains("method=\"get\"") && !source.contains("on:submit="));
            }
        }
        let saved = fs::read(root.0.join("views/admin/posts/_guard.jsbrain")).unwrap();
        assert!(add(&root.0, "resource", "posts", "title:bool").is_err());
        assert_eq!(
            fs::read(root.0.join("views/admin/posts/_guard.jsbrain")).unwrap(),
            saved
        );
        let config: Value = serde_json::from_slice(&fs::read(root.0.join("brain.json")).unwrap()).unwrap();
        assert_eq!(config["budget"]["viewKB"], 30);
        let report = crate::run_build(&root.0, Some("https://example.test".into())).unwrap();
        eprintln!("CRUD default-budget fixture bytes by route: {:?}", report.report);
        for entry in &report.ledger {
            let mut entry: Value = serde_json::from_str(&entry.stringify()).unwrap();
            let route = entry["route"].as_str().unwrap_or("");
            if route.starts_with("/admin/posts") {
                assert!(
                    entry["assets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|asset| asset["source"] != "runtime/brain-data.js"),
                    "Server-owned admin reads must not import the generic client API runtime"
                );
            }
            if route.starts_with("/admin/posts") || route == "/login" {
                if let Some(assets) = entry["assets"].as_array_mut() {
                    assets.sort_by_key(|asset| {
                        std::cmp::Reverse(asset["bytes"]["gzip"].as_u64().unwrap_or(0))
                    });
                    assets.truncate(8);
                }
                eprintln!("CRUD emitted ledger (largest eight assets): {entry}");
            }
        }
        let mut command = std::process::Command::new("node");
        command
            .args([
                "--input-type=module",
                "-e",
                include_str!("../tests/generated-resource-rpc.mjs"),
            ])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("BRAIN_CRUD_DIST", root.0.join("dist"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().unwrap();
        let mut group = HandlerGroup(Some(child.id()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                drop(group);
                let _ = child.kill();
                let _ = child.wait();
                panic!("Generated CRUD fixture exceeded its owned deadline")
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        group.0 = None;
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .take(16385)
            .read_to_string(&mut output)
            .unwrap();
        let mut error = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .take(16385)
            .read_to_string(&mut error)
            .unwrap();
        assert!(output.len() <= 16384 && error.len() <= 16384);
        assert!(status.success(), "{output}\n{error}");
        assert!(output.contains("Generated admin CRUD: PASS"));
    }
    #[test]
    fn resource_support_conflicts_roll_back_and_preserve_unrelated_admin_source() {
        let root = fixture_root("crud-owned-guard");
        crate::scaffold::new_app(&root.0).unwrap();
        fs::create_dir_all(root.0.join("views/admin/posts")).unwrap();
        let guard = root.0.join("views/admin/posts/_guard.jsbrain");
        fs::write(&guard, "export default () => false\n").unwrap();
        fs::create_dir_all(root.0.join("api")).unwrap();
        fs::write(root.0.join("api/posts.jsbrain"), "export default {}\n").unwrap();
        assert!(add(&root.0, "resource", "posts", "title:str!").is_err());
        assert_eq!(
            fs::read_to_string(&guard).unwrap(),
            "export default () => false\n"
        );
        assert!(
            !root
                .0
                .join("server/resources/posts/schema.jsbrain")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(root.0.join("api/posts.jsbrain")).unwrap(),
            "export default {}\n"
        );
        fs::remove_file(root.0.join("api/posts.jsbrain")).unwrap();
        add(&root.0, "resource", "posts", "title:str!").unwrap();
        assert_eq!(
            fs::read_to_string(&guard).unwrap(),
            "export default () => false\n"
        );
    }
    #[test]
    fn resource_contract_reader_rejects_nonregular_symlink_and_oversize_sources() {
        let root = fixture_root("contract-bounds");
        let source = "export default {title:'str!'}";
        let file = root.0.join("schema.jsbrain");
        fs::write(
            &file,
            format!("{source}{}", " ".repeat(65536 - source.len())),
        )
        .unwrap();
        assert!(read_contract(&root.0, &file).is_ok());
        fs::write(&file, "x".repeat(65537)).unwrap();
        assert!(
            read_contract(&root.0, &file)
                .unwrap_err()
                .contains("64 KiB")
        );
        #[cfg(unix)]
        {
            use std::{
                ffi::CString,
                os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
            };
            fs::remove_file(&file).unwrap();
            let fifo = CString::new(file.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            // A bounded writer releases an accidentally blocking old reader, making a regression fail rather than hang.
            let writer_path = file.clone();
            let writer = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if let Ok(mut f) = fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(writer_path)
                {
                    use std::io::Write;
                    let _ = f.write_all(b"export default {title:'str!'}");
                }
            });
            assert!(
                read_contract(&root.0, &file)
                    .unwrap_err()
                    .contains("nonregular")
            );
            writer.join().unwrap();
            fs::remove_file(&file).unwrap();
            fs::write(root.0.join("actual.jsbrain"), source).unwrap();
            std::os::unix::fs::symlink(root.0.join("actual.jsbrain"), &file).unwrap();
            assert!(read_contract(&root.0, &file).is_err());
            fs::create_dir_all(root.0.join("actual")).unwrap();
            fs::write(root.0.join("actual/schema.jsbrain"), source).unwrap();
            std::os::unix::fs::symlink(root.0.join("actual"), root.0.join("linked")).unwrap();
            assert!(read_contract(&root.0, &root.0.join("linked/schema.jsbrain")).is_err());
        }
    }
    #[test]
    fn exact_type_contracts_read_object_fields_numbers_and_constraints() {
        let root = fixture_root("exact-types");
        let dir = root.0.join("server/resources/vacancies");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("schema.jsbrain"), "<meta server>\nexport default {\n  title: 'str!',\n  status: { type: 'enum(draft|published)!', default: 'draft' },\n  slug: { type: 'str!', unique: true },\n  openings: { type: 'int', default: 1 },\n  salary_min: 'decimal(10,2)',\n  salary_max: 'decimal(10,2)',\n  currency: 'enum(MXN|USD)',\n  closes_on: 'date',\n  tags: 'list<str,10>'\n}\n").unwrap();
        fs::write(dir.join("policy.jsbrain"), "<meta server>\nexport default { read: ['admin'], create: ['admin'], update: ['admin'], delete: ['admin'] }\n").unwrap();
        fs::write(dir.join("constraints.jsbrain"), "<meta server>\nexport default {\n  unique: [['title', 'closes_on']],\n  check: [\n    { field: 'openings', min: 1, max: 500 },\n    { compare: ['salary_min', '<=', 'salary_max'] },\n    { together: ['salary_min', 'salary_max', 'currency'] }\n  ]\n}\n").unwrap();
        let resources = contracts(&root.0, "resources").unwrap();
        let r = &resources[0];
        assert_eq!(r.fields["status"].default, Some(json!("draft")));
        assert!(r.fields["slug"].unique && r.fields["slug"].required);
        assert_eq!(r.fields["openings"].default, Some(json!(1)));
        assert_eq!(r.constraints.check.len(), 3);
        assert_eq!(r.unique_sets().len(), 2);
        let saved = r.input(&json!({"title":"Engineer","slug":"engineer"})).unwrap();
        assert_eq!(saved, json!({"title":"Engineer","slug":"engineer","status":"draft","openings":1}));
        assert!(r.input(&json!({"title":"A","slug":"a","salary_min":"10"})).unwrap_err().contains_key("salary_max"));
        fs::write(dir.join("schema.jsbrain"), "export default { price: { type: 'int', default: 99999999999999999999 } }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("64-bit integer range"));
        fs::write(dir.join("schema.jsbrain"), "export default { price: { type: 'int', unknown: true } }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("unknown option"));
        fs::write(dir.join("schema.jsbrain"), "export default { price: 'money' }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("decimal(p,s)"));
        fs::write(dir.join("schema.jsbrain"), "export default { a: 'int', b: 'int' }\n").unwrap();
        fs::write(dir.join("constraints.jsbrain"), "export default { check: [{ field: 'a', min: 1, max: -2 }] }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("min greater than max"));
        fs::write(dir.join("constraints.jsbrain"), "export default { checks: [] }\n").unwrap();
        assert!(contracts(&root.0, "resources").unwrap_err().contains("constraints.jsbrain must export"));
    }
    #[test]
    fn contracts_do_not_evaluate_application_code() {
        let root =
            std::env::temp_dir().join(format!("brain-backend-literal-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("schema.jsbrain");
        fs::write(&path, "export default fetch('/side-effect')\n").unwrap();
        assert!(read_contract(&root, &path).is_err());
        fs::write(&path, "export default { title:'str!', title:'bool' }\n").unwrap();
        assert!(read_contract(&root, &path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

/// Native media operations. Provisioning returns a guarded file plan; cloud setup never executes implicitly.
pub fn media(args: &[String]) -> Result<(), String> {
    let root = crate::app_root()?;
    let operation = args.first().map(String::as_str).unwrap_or("help");
    if operation == "setup-gcs" {
        let mut operations = Vec::new();
        let mut preserved = Vec::new();
        for (path, content) in brain_backend::media::gcs_files() {
            let current = crate::changes::read(&root, &path)?;
            if current["revision"].is_null() {
                operations.push(json!({"path":path,"expectedRevision":null,"content":content}));
            } else {
                preserved.push(json!({"path":path,"revision":current["revision"]}));
            }
        }
        println!(
            "{}",
            json!({"schemaVersion":1,"dryRun":true,"operations":operations,"preserved":preserved,"installed":false,"cloudContacted":false,"environmentKeys":["BRAIN_GCS_BUCKET","BRAIN_GCS_PREFIX","BRAIN_GCS_ADAPTER"],"next":"Apply reviewed operations through brain changes, install the private pinned SDK, and configure ADC outside public files"})
        );
        return Ok(());
    }
    if !matches!(operation, "list" | "cleanup") {
        return Err("usage: brain media setup-gcs | list --owner <uuid> [--limit 50] [--offset 0] | cleanup [--limit 8]; cleanup processes only recorded owned deletion work".into());
    }
    let environment = crate::env::name(args, "development");
    let url = database(&root, &environment)?;
    let limit = crate::flag(args, "--limit")
        .unwrap_or_else(|| {
            if operation == "list" {
                "50".into()
            } else {
                "8".into()
            }
        })
        .parse::<i64>()
        .map_err(|_| "media limit must be an integer")?;
    let max = if operation == "list" { 100 } else { 8 };
    if !(1..=max).contains(&limit) {
        return Err(format!("media limit must be 1..{max}"));
    }
    runtime()?.block_on(async {
        let pool = brain_backend::boot::connect(&url, &brain_backend::boot::Boot::command(environment == "production")).await?;
        let result = if operation == "list" {
            let owner = crate::flag(args, "--owner")
                .ok_or("media list requires an explicit owner UUID")?
                .parse()
                .map_err(|_| "owner must be a UUID")?;
            let offset = crate::flag(args, "--offset")
                .unwrap_or_else(|| "0".into())
                .parse::<i64>()
                .map_err(|_| "media offset must be 0..10000")?;
            if !(0..=10000).contains(&offset) {
                return Err("media offset must be 0..10000".into());
            }
            brain_backend::media::library(&pool, owner, limit, offset).await
        } else {
            let setting = |key: &str| {
                std::env::var(key)
                    .ok()
                    .filter(|v| !v.is_empty())
                    .or_else(|| crate::env::value(&root, &environment, key))
            };
            let gcs =
                setting("BRAIN_GCS_BUCKET")
                    .map(|bucket| {
                        brain_backend::media::GcsConfig::new(
                            bucket,
                            setting("BRAIN_GCS_PREFIX").unwrap_or_else(|| "brain-media".into()),
                            PathBuf::from(setting("BRAIN_GCS_ADAPTER").ok_or(
                                "BRAIN_GCS_ADAPTER is required for configured GCS cleanup",
                            )?),
                        )
                    })
                    .transpose()?;
            let store = brain_backend::media::Store::new(root.join(".brain/uploads"), gcs)?;
            brain_backend::media::cleanup(&pool, &store, limit).await
        };
        pool.close().await;
        println!("{}", result?);
        Ok(())
    })
}
