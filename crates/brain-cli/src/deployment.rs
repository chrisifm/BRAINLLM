//! Optional deployment configuration previews. Does not install, activate, upload, migrate or restart anything.
use serde_json::{Value, json};
use std::path::Path;
fn bounded(args: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    args.get(key)
        .map_or(Some(default), Value::as_u64)
        .filter(|v| (*v >= min) && (*v <= max))
        .ok_or_else(|| format!("{key} must be {min}..{max}"))
}
fn domain(args: &Value) -> Result<String, String> {
    let s = args["domain"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 253
                && s.split('.').all(|part| {
                    !part.is_empty()
                        && part.len() <= 63
                        && !part.starts_with('-')
                        && !part.ends_with('-')
                        && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
        })
        .ok_or("domain must be an explicit public DNS name without scheme, port or path")?;
    if !s.contains('.') {
        return Err("domain must contain at least two DNS labels".into());
    }
    Ok(s.to_ascii_lowercase())
}
fn nginx_path(args: &Value, key: &str) -> Result<String, String> {
    let s=args[key].as_str().filter(|s|s.starts_with('/')&&s.len()<=512&&!s.split('/').any(|p|p=="..")&&s.bytes().all(|b|b.is_ascii_alphanumeric()||b"/_-. ".contains(&b))).ok_or_else(||format!("{key} must be an absolute literal certificate path without control characters or nginx variables"))?;
    Ok(format!("\"{s}\""))
}
/// target: pm2 or nginx. Configuration remains optional; the native manager is `brain start`.
pub fn preview(root: &Path, target: &str, options: &Value) -> Result<Value, String> {
    if !options.is_object()
        || options.as_object().unwrap().keys().any(|k| {
            ![
                "topology",
                "workers",
                "name",
                "port",
                "backendPort",
                "domain",
                "certificate",
                "certificateKey",
                "brainBinary",
                "memoryMb",
                "expectedRevision",
            ]
            .contains(&k.as_str())
        })
    {
        return Err("unsupported deployment option".into());
    }
    let topology = options
        .get("topology")
        .map_or(Some("node"), Value::as_str)
        .filter(|s| ["node", "static", "rust-node", "rust-static"].contains(s))
        .ok_or("topology must be static, node, rust-node or rust-static")?;
    let native_backend = matches!(topology, "rust-node" | "rust-static");
    let mut workers=Vec::new();let mut seen=std::collections::BTreeSet::new();let mut concurrent=0u64;
    if let Some(value)=options.get("workers") {
        let list=value.as_array().filter(|list|list.len()<=8).ok_or("workers must be an array of at most eight queue contracts")?;
        for item in list {
            let fields=item.as_object().filter(|map|map.keys().all(|key|matches!(key.as_str(),"queue"|"concurrency"))).ok_or("worker allows only queue and concurrency")?;
            let queue=fields.get("queue").and_then(Value::as_str).filter(|queue|brain_backend::contract::identifier(queue)).ok_or("worker queue must be a native lowercase resource identifier")?;
            if !seen.insert(queue.to_owned()) {return Err("duplicate deployment worker queue".into());}
            let count=bounded(item,"concurrency",2,1,8)?;concurrent+=count;
            if concurrent>16 {return Err("total deployment worker concurrency must not exceed sixteen".into());}
            workers.push((queue.to_owned(),count));
        }
    }
    let port = bounded(options, "port", 3000, 1024, 65535)?;
    let backend = bounded(options, "backendPort", 5182, 1024, 65535)?;
    if native_backend && port == backend {
        return Err("frontend and backend ports must be distinct".into());
    }
    let name = options
        .get("name")
        .map_or(Some("brain-app"), Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
        .ok_or("name must contain 1..64 ASCII letters, digits, _ or -")?;
    let root = root
        .canonicalize()
        .map_err(|_| "deployment root is unavailable")?;
    if !root.is_dir() {
        return Err("deployment root must be an application directory".into());
    }
    let mut warnings=vec!["Preview only: review the topology, secret-store setup, release artifact, host permissions and process manager before activation.".to_string()];
    let (path, content) = match target {
        "pm2" => {
            let memory = bounded(options, "memoryMb", 512, 64, 4096)?;
            let binary = options
                .get("brainBinary")
                .map_or(Some("brain"), Value::as_str)
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 512
                        && !s.chars().any(char::is_control)
                        && !s.contains(['\n', '\r'])
                })
                .ok_or("brainBinary must be one literal executable path, not a shell command")?;
            let common = |app_name: String, args: Vec<String>| json!({"name":app_name,"cwd":root.to_string_lossy(),"script":binary,"args":args,"interpreter":"none","instances":1,"exec_mode":"fork","watch":false,"autorestart":true,"min_uptime":10000,"max_restarts":10,"exp_backoff_restart_delay":1000,"kill_timeout":20000,"listen_timeout":30000,"wait_ready":false,"max_memory_restart":format!("{memory}M"),"time":true,"merge_logs":true});
            let mut apps = vec![common(
                name.to_owned(),
                vec![
                    "start".into(),
                    "--host".into(),
                    "127.0.0.1".into(),
                    "--port".into(),
                    port.to_string(),
                    "--instances".into(),
                    "1".into(),
                    "--max-memory".into(),
                    format!("{memory}M"),
                    "--env".into(),
                    "production".into(),
                ],
            )];
            if native_backend {
                apps[0]["env"] = json!({"BRAIN_BACKEND_URL":format!("http://127.0.0.1:{backend}")});
                let host = domain(options)?;
                apps.push(common(
                    format!("{name}-backend"),
                    vec![
                        "backend".into(),
                        "serve".into(),
                        "--port".into(),
                        backend.to_string(),
                        "--origin".into(),
                        format!("https://{host}"),
                        "--env".into(),
                        "production".into(),
                    ],
                ));
            }
            for (queue,count) in &workers {
                apps.push(common(format!("{name}-worker-{queue}"),vec!["jobs".into(),"work".into(),queue.clone(),"--concurrency".into(),count.to_string(),"--env".into(),"production".into()]));
            }
            warnings.push("PM2 supervises one foreground brain start process; brain start retains its own Node-child drain, RSS sampling and reload behavior. PM2 memory polling is not a hard whole-tree quota.".into());
            (
                "deploy/ecosystem.brain.config.cjs",
                format!(
                    "// Optional PM2 wrapper; secrets remain in the host/private environment.\nmodule.exports = {};\n",
                    serde_json::to_string_pretty(&json!({"apps":apps}))
                        .map_err(|_| "cannot serialize ecosystem preview")?
                ),
            )
        }
        "nginx" => {
            let host = domain(options)?;
            let cert = nginx_path(options, "certificate")?;
            let key = nginx_path(options, "certificateKey")?;
            let proxy = |upstream: u64| {
                format!(
                    "        proxy_pass http://127.0.0.1:{upstream};\n        proxy_http_version 1.1;\n        proxy_set_header Host {host};\n        proxy_set_header X-Forwarded-Host {host};\n        proxy_set_header X-Forwarded-Proto https;\n        proxy_set_header X-Forwarded-For $remote_addr;\n        proxy_set_header Forwarded \"\";\n        proxy_set_header Connection \"\";\n        proxy_connect_timeout 5s;\n        proxy_send_timeout 15s;\n        proxy_read_timeout 35s;\n        proxy_cache off;\n        proxy_next_upstream off;\n"
                )
            };
            let mut routes = String::new();
            if native_backend {
                for route in ["^~ /api/", "= /api", "^~ /_brain/", "= /_brain", "^~ /s/", "= /health"] {
                    routes.push_str(&format!(
                        "    location {route} {{\n{}    }}\n",
                        proxy(backend)
                    ));
                }
            }
            // Keep the app server's content negotiation, versioned immutable asset headers and private response policy.
            routes.push_str(&format!("    location / {{\n{}    }}\n", proxy(port)));
            let content = include_str!("templates/deployment/nginx.conf")
                .replace("{{DOMAIN}}", &host)
                .replace("{{CERTIFICATE}}", &cert)
                .replace("{{CERTIFICATE_KEY}}", &key)
                .replace("{{ROUTES}}", &routes);
            warnings.push("TLS certificate files must already exist and be protected. nginx -t and the installed module inventory must be checked before explicitly loading this server block.".into());
            warnings.push("Forwarded client identity is overwritten at this edge, never appended from untrusted request headers. Keep upstream ports loopback-only and review any additional proxy hop explicitly.".into());
            ("deploy/nginx.brain.conf", content)
        }
        _ => return Err("deployment target must be pm2 or nginx".into()),
    };
    if native_backend {
        warnings.push("The frontend forwards native API and short-link requests to its fixed loopback backend; Node SSR verifies native sessions through that backend. PostgreSQL schema migration remains a separate explicit operation. Static topologies require a build without dist/server.mjs.".into());
    }
    if target=="nginx" && !workers.is_empty() {warnings.push("nginx routes HTTP; generate the matching PM2 configuration to supervise these explicit queue workers.".into());}
    let environment_keys=if native_backend {vec!["BRAIN_BACKEND_URL","DATABASE_URL","BRAIN_SECRET","BRAIN_JWT_SECRET","BRAIN_JWT_ISSUER","BRAIN_JWT_AUDIENCE","GOOGLE_CLIENT_ID","BRAIN_MAIL_RESEND_KEY","BRAIN_MAIL_FROM","PUBLIC_SITE_URL","BRAIN_GCS_BUCKET","BRAIN_GCS_PREFIX","BRAIN_GCS_ADAPTER"]}else if !workers.is_empty(){vec!["BRAIN_SECRET","DATABASE_URL"]}else{vec!["BRAIN_SECRET"]};
    let existing = crate::changes::read(&root, path)?;
    if options
        .get("expectedRevision")
        .is_some_and(|r| r != &existing["revision"])
    {
        return Err(
            "deployment configuration changed; inspect it and prepare a new preview".into(),
        );
    }
    // Adopt only new files; existing custom deployment files are never rewritten automatically.
    let operations = if existing["revision"].is_null() {
        vec![json!({"path":path,"expectedRevision":null,"content":content})]
    } else {
        warnings.push("Existing deployment configuration preserved; compare the reviewed proposed configuration manually.".into());
        vec![]
    };
    Ok(
        json!({"schemaVersion":1,"dryRun":true,"target":target,"topology":topology,"operations":operations,"existingRevision":existing["revision"],"preservedExisting":!existing["revision"].is_null(),"proposed":if !existing["revision"].is_null(){Some(json!({"path":path,"content":content}))}else{None},"services":{"frontend":{"address":"127.0.0.1","port":port,"runtime":if matches!(topology,"static"|"rust-static"){"Rust static handler; build must contain no server runtime"}else{"brain supervisor with Node SSR when dist/server.mjs exists"},"instances":1},"resourceBackend":if native_backend{Some(json!({"address":"127.0.0.1","port":backend,"runtime":"Rust resource backend/PostgreSQL","instances":1}))}else{None}},"environmentKeys":environment_keys,"workers":workers.iter().map(|(queue,count)|json!({"queue":queue,"concurrency":count,"instances":1,"manager":"pm2"})).collect::<Vec<_>>(),"compression":{"upstreamPrecompressedAssets":"preserved","nginxBrotli":"not assumed; verify installed optional module and generated .br assets before adding directives","dynamicCompression":"not enabled by this preview"},"activation":{"installed":false,"started":false,"published":false,"next":["Review the generated source and exact service topology","Apply reviewed CAS operations with the change planner","Build and inspect the release artifact; install production dependencies if Node SDKs are used","Configure private environment keys outside public deployment files","Validate PM2 config or nginx -t locally before explicitly activating through the host's verified service manager","Verify graceful stop, HTTPS health, cache/Vary behavior and per-process resource limits"]},"warnings":warnings}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct App(std::path::PathBuf);
    impl App {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "brain-deployment-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn pm2_is_literal_safe_single_instance_and_preserves_existing() {
        let app = App::new();
        let p=preview(&app.0,"pm2",&json!({"brainBinary":"/tmp/brain'quoted","topology":"rust-node","domain":"example.test"})).unwrap();
        let src = p["operations"][0]["content"].as_str().unwrap();
        let value: Value = serde_json::from_str(
            src.split_once("module.exports = ")
                .unwrap()
                .1
                .trim()
                .trim_end_matches(';'),
        )
        .unwrap();
        assert_eq!(value["apps"].as_array().unwrap().len(), 2);
        assert_eq!(value["apps"][0]["instances"], 1);
        assert_eq!(value["apps"][0]["script"], "/tmp/brain'quoted");
        assert_eq!(value["apps"][0]["env"], json!({"BRAIN_BACKEND_URL":"http://127.0.0.1:5182"}));
        assert!(value["apps"][1].get("env").is_none());
        assert!(!app.0.join("deploy").exists());
        std::fs::create_dir(app.0.join("deploy")).unwrap();
        std::fs::write(app.0.join("deploy/ecosystem.brain.config.cjs"), "custom").unwrap();
        let p = preview(&app.0, "pm2", &json!({})).unwrap();
        assert_eq!(p["operations"], json!([]));
        assert_eq!(p["preservedExisting"], true);
    }
    #[test]
    fn queue_workers_are_literal_bounded_and_separately_supervised() {
        let app=App::new();
        let result=preview(&app.0,"pm2",&json!({"workers":[{"queue":"thumbnails","concurrency":2}]})).unwrap();
        let source=result["operations"][0]["content"].as_str().unwrap();
        let value:Value=serde_json::from_str(source.split_once("module.exports = ").unwrap().1.trim().trim_end_matches(';')).unwrap();
        assert_eq!(value["apps"][1]["args"],json!(["jobs","work","thumbnails","--concurrency","2","--env","production"]));
        for request in [json!({"workers":[{"queue":"x;run"}]}),json!({"workers":[{"queue":"x"},{"queue":"x"}]}),json!({"workers":[{"queue":"a","concurrency":8},{"queue":"b","concurrency":8},{"queue":"c"}]})] {assert!(preview(&app.0,"pm2",&request).is_err());}
    }
    #[test]
    fn all_frontend_backend_topologies_have_consistent_public_endpoints() {
        let app=App::new();
        for topology in ["static","node","rust-static","rust-node"] {
            let native=topology.starts_with("rust-");
            let p=preview(&app.0,"pm2",&json!({"topology":topology,"domain":"example.test","port":3100,"backendPort":5200})).unwrap();
            let source=p["operations"][0]["content"].as_str().unwrap();
            let value:Value=serde_json::from_str(source.split_once("module.exports = ").unwrap().1.trim().trim_end_matches(';')).unwrap();
            assert_eq!(value["apps"].as_array().unwrap().len(),if native {2}else{1});
            assert_eq!(p["services"]["resourceBackend"].is_object(),native);
            if native {assert_eq!(value["apps"][0]["env"]["BRAIN_BACKEND_URL"],"http://127.0.0.1:5200");}
            else {assert!(value["apps"][0].get("env").is_none());}
            let p=preview(&app.0,"nginx",&json!({"topology":topology,"domain":"example.test","certificate":"/etc/cert.pem","certificateKey":"/etc/key.pem","port":3100,"backendPort":5200})).unwrap();
            let source=p["operations"][0]["content"].as_str().unwrap();
            assert!(source.contains("127.0.0.1:3100"));
            assert_eq!(source.contains("127.0.0.1:5200"),native);
            assert!(!source.contains("DATABASE_URL"));
        }
    }
    #[test]
    fn nginx_overwrites_identity_and_has_no_dev_socket_or_session_cache() {
        let app = App::new();
        let p=preview(&app.0,"nginx",&json!({"topology":"rust-node","domain":"example.test","certificate":"/etc/certs/fullchain.pem","certificateKey":"/etc/certs/private.key"})).unwrap();
        let c = p["operations"][0]["content"].as_str().unwrap();
        assert!(c.contains("X-Forwarded-For $remote_addr"));
        assert!(!c.contains("$proxy_add_x_forwarded_for"));
        assert!(!c.contains("Upgrade"));
        assert!(!c.contains("brotli on"));
        assert!(c.contains("proxy_cache off"));
        assert!(c.contains("127.0.0.1:5182"));
        assert!(c.contains("location ^~ /s/"));
        assert!(c.contains("location = /_brain"));
        assert!(
            preview(
                &app.0,
                "nginx",
                &json!({"domain":"example.test;evil","certificate":"/a","certificateKey":"/b"})
            )
            .is_err()
        );
        assert!(
            preview(
                &app.0,
                "nginx",
                &json!({"domain":"example.test","certificate":"/a;$x","certificateKey":"/b"})
            )
            .is_err()
        );
    }
}
