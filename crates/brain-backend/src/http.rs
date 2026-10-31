//! Fixed HTTP lifecycle: limits → origin → auth → policy → validation → repository → structured response.
use crate::{
    Resource,
    auth::{self, Auth, Credentials, User},
    contract, db,
    jobs::Queue,
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Multipart, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tower_http::{compression::CompressionLayer, cors::CorsLayer};
use uuid::Uuid;

#[path = "sockets.rs"]
mod sockets;
pub use sockets::{SocketConfig, SocketControl};

tokio::task_local! { static REQUEST_ID: String; }
pub(crate) fn request_reference() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}

pub struct Config {
    pub resources: Vec<Resource>,
    pub jobs: Vec<Resource>,
    pub uploads: PathBuf,
    /// Explicit browser origins, including the site's origin. Forwarded headers are never trusted.
    pub origins: Vec<String>,
    pub production: bool,
    pub jwt: Option<crate::jwt::Config>,
    pub google: Option<crate::google::Config>,
    pub gcs: Option<crate::media::GcsConfig>,
    pub roles: crate::auth_flows::RolesConfig,
    pub mail: Option<crate::auth_flows::MailConfig>,
    pub auth_public_base_url: Option<String>,
    pub webhooks: Option<crate::webhooks::Config>,
    pub before_hooks: Option<crate::before_hooks::Executor>,
    /// None omits the Upgrade route; enabling requires an explicit subset of HTTP origins.
    pub sockets: Option<SocketConfig>,
}
#[derive(Clone)]
pub struct App {
    sockets: Arc<sockets::Broker>,
    pool: PgPool,
    auth: Auth,
    jwt: Option<crate::jwt::Jwt>,
    google: Option<crate::google::Google>,
    auth_flows: crate::auth_flows::Flows,
    webhooks: Option<crate::webhooks::Webhooks>,
    before_hooks: Option<crate::before_hooks::Executor>,
    resources: Arc<BTreeMap<String, Resource>>,
    jobs: Arc<BTreeMap<String, Resource>>,
    queue: Queue,
    uploads: PathBuf,
    media: crate::media::Store,
    production: bool,
    origins: Arc<Vec<String>>,
}
#[derive(Debug)]
pub struct Error {
    status: StatusCode,
    code: &'static str,
    fix: &'static str,
    fields: Option<Value>,
}
impl Error {
    fn new(status: StatusCode, code: &'static str, fix: &'static str) -> Self {
        Self {
            status,
            code,
            fix,
            fields: None,
        }
    }
    #[track_caller]
    fn internal(_: impl std::fmt::Display) -> Self {
        let at = std::panic::Location::caller();
        static DIAGNOSTIC_BUSY: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        let rid = REQUEST_ID.try_with(Clone::clone).ok();
        let reserved = DIAGNOSTIC_BUSY
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok();
        eprintln!(
            "{}",
            json!({"schemaVersion":1,"event":"backend-internal-error","rid":rid,"code":"BACKEND_INTERNAL","location":{"file":at.file(),"line":at.line(),"column":at.column()},"stackDeferred":reserved,"stackOmitted":!reserved,"queueDepth":0,"private":true,"valuesCaptured":false})
        );
        if reserved {
            let trace = std::backtrace::Backtrace::force_capture();
            let diagnostic=std::thread::Builder::new().name("brain-diagnostic".into()).spawn(move || {
                struct Release;
                impl Drop for Release {fn drop(&mut self){DIAGNOSTIC_BUSY.store(false,std::sync::atomic::Ordering::Release);}}
                let _release=Release;
                struct Bounded {text:String,truncated:bool}
                impl std::fmt::Write for Bounded {fn write_str(&mut self,value:&str)->std::fmt::Result {
                    let mut n=(32768-self.text.len()).min(value.len());while !value.is_char_boundary(n){n-=1;}self.text.push_str(&value[..n]);
                    if n<value.len(){self.truncated=true;Err(std::fmt::Error)}else{Ok(())}
                }}
                let mut stack=Bounded{text:String::new(),truncated:false};let _=std::fmt::write(&mut stack,format_args!("{trace}"));
                eprintln!("{}",json!({"schemaVersion":1,"event":"backend-internal-stack","rid":rid,"location":{"file":at.file(),"line":at.line(),"column":at.column()},"stack":stack.text,"truncated":stack.truncated,"queueDepth":0,"private":true,"valuesCaptured":false}));
            });
            if diagnostic.is_err() {
                DIAGNOSTIC_BUSY.store(false, std::sync::atomic::Ordering::Release);
                eprintln!(
                    "{}",
                    json!({"schemaVersion":1,"event":"backend-diagnostic-unavailable","code":"DIAGNOSTIC_THREAD_UNAVAILABLE","stackOmitted":true,"queueDepth":0,"private":true,"valuesCaptured":false})
                );
            }
        }
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "BACKEND_INTERNAL",
            "Inspect private server diagnostics using the request id; do not retry writes automatically",
        )
    }
    fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "POLICY_DENIED",
            "Sign in with a role permitted by this resource's policy",
        )
    }
    fn missing() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "Read the API contract and use an existing resource and id",
        )
    }
    fn validation(fields: impl serde::Serialize) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "INVALID_INPUT",
            fix: "Apply the field errors and send only fields declared in schema.jsbrain",
            fields: serde_json::to_value(fields).ok(),
        }
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut response=(self.status,Json(json!({"schemaVersion":1,"rid":REQUEST_ID.try_with(Clone::clone).ok(),"error":{"code":self.code,"fix":self.fix,"fields":self.fields}}))).into_response();
        response
            .extensions_mut()
            .insert(crate::response::DefaultError);
        if self.code == "AUTH_DENIED" {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}
type Result<T> = std::result::Result<T, Error>;
impl App {
    async fn user(&self, h: &HeaderMap) -> Result<Option<User>> {
        if h.contains_key(header::AUTHORIZATION)
            || crate::jwt::cookie(h, crate::jwt::ACCESS_COOKIE).is_some()
        {
            return match &self.jwt {
                Some(jwt) => jwt
                    .user(crate::jwt::access(h))
                    .await
                    .map_err(|error| Error::internal(error)),
                None => Ok(None),
            };
        }
        self.auth
            .user(auth::session_cookie(h))
            .await
            .map_err(|error| Error::internal(error))
    }
    async fn required(&self, h: &HeaderMap) -> Result<User> {
        self.user(h).await?.ok_or_else(|| {
            Error::new(
                StatusCode::UNAUTHORIZED,
                "SIGN_IN_REQUIRED",
                "Sign in before making this request",
            )
        })
    }
    async fn admin(&self, h: &HeaderMap) -> Result<User> {
        let u = self.required(h).await?;
        if u.role != "admin" {
            return Err(Error::forbidden());
        }
        Ok(u)
    }
    fn resource(&self, name: &str) -> Result<&Resource> {
        self.resources.get(name).ok_or_else(Error::missing)
    }
}

pub async fn router(pool: PgPool, config: Config) -> std::result::Result<Router, String> {
    router_with_socket_control(pool, config).await.map(|(router, _)| router)
}

/// Library hosts must shut down this control before draining their owned HTTP listener.
pub async fn router_with_socket_control(
    pool: PgPool,
    config: Config,
) -> std::result::Result<(Router, SocketControl), String> {
    let socket_origins = config.sockets.as_ref().map(|s| s.allowed().to_vec()).unwrap_or_default();
    if socket_origins.iter().any(|origin| !config.origins.contains(origin)) {
        return Err("Socket origins must be an explicit subset of backend HTTP origins".into());
    }
    let sockets_enabled = !socket_origins.is_empty();
    let mut origins = Vec::new();
    for origin in &config.origins {
        let uri: axum::http::Uri = origin.parse().map_err(|_| "Invalid backend origin")?;
        if !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.authority().is_none()
            || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
            || uri.authority().unwrap().as_str().contains('@')
        {
            return Err("Backend origins must be explicit http(s) origins without credentials, paths or wildcards".into());
        }
        origins.push(
            origin
                .parse::<HeaderValue>()
                .map_err(|_| "Invalid backend origin header")?,
        );
    }
    if origins.is_empty() {
        return Err("At least one explicit backend origin is required".into());
    }
    config.roles.validate()?;
    crate::before_hooks::validate(&config.resources, config.before_hooks.as_ref())?;
    let mut resources = BTreeMap::new();
    for r in config.resources {
        r.validate()?;
        for grants in [
            &r.policy.read,
            &r.policy.create,
            &r.policy.update,
            &r.policy.delete,
        ] {
            if grants.iter().any(|role| {
                !matches!(role.as_str(), "public" | "owner")
                    && crate::ownership::path(role).is_none()
                    && !config.roles.roles.contains(role)
            }) {
                return Err(
                    "Resource policy names an unconfigured role; inspect server/auth/roles.jsbrain"
                        .into(),
                );
            }
        }
        if resources.insert(r.name.clone(), r).is_some() {
            return Err("Duplicate resource".into());
        }
    }
    let mut jobs = BTreeMap::new();
    for j in config.jobs {
        j.validate()?;
        if j.name == crate::webhooks::OUTBOUND_QUEUE {
            return Err("Application jobs must not use the reserved webhook delivery queue".into());
        }
        if jobs.insert(j.name.clone(), j).is_some() {
            return Err("Duplicate job".into());
        }
    }
    contract::validate_graph(
        &resources.values().cloned().collect::<Vec<_>>(),
        &jobs.values().cloned().collect::<Vec<_>>(),
    )?;
    db::verify(&pool, &resources.values().cloned().collect::<Vec<_>>()).await?;
    let jwt = config
        .jwt
        .map(|config| crate::jwt::Jwt::new(pool.clone(), config));
    let google = config.google.map(crate::google::Google::new).transpose()?;
    if google.is_some() && jwt.is_none() {
        return Err("Google sign-in requires configured JWT sessions".into());
    }
    let registered = sqlx::query_scalar::<_, String>(
        "SELECT role FROM public._brain_auth_roles WHERE role=ANY($1)",
    )
    .bind(&config.roles.roles)
    .fetch_all(&pool)
    .await
    .map_err(db::safe)?;
    if config
        .roles
        .roles
        .iter()
        .any(|role| !registered.contains(role))
    {
        return Err(
            "Configured auth roles are not provisioned; run the explicit backend migration".into(),
        );
    }
    let auth = Auth::new_with_role(pool.clone(), &config.roles.signup_role).await?;
    let auth_base = config
        .auth_public_base_url
        .as_deref()
        .or_else(|| {
            config
                .mail
                .as_ref()
                .map(|mail| mail.public_base_url.as_str())
        })
        .unwrap_or(&config.origins[0]);
    let public_origin = crate::links::origin(auth_base).ok_or("Auth public base URL is invalid")?;
    if !config.origins.contains(&public_origin) {
        return Err(
            "Auth public base URL must belong to an explicitly allowed frontend origin".into(),
        );
    }
    let delivery = config
        .mail
        .as_ref()
        .map(crate::auth_flows::MailConfig::delivery)
        .transpose()?;
    let auth_flows = crate::auth_flows::Flows::new(
        pool.clone(),
        auth.clone(),
        config.roles,
        auth_base,
        delivery,
    )?;
    let webhooks = config
        .webhooks
        .map(|cfg| {
            crate::webhooks::Webhooks::new(
                pool.clone(),
                cfg,
                &jobs.values().cloned().collect::<Vec<_>>(),
            )
        })
        .transpose()?;
    let sockets = Arc::new(sockets::Broker::new(socket_origins));
    let app = App {
        sockets: sockets.clone(),
        webhooks,
        before_hooks: config.before_hooks,
        auth,
        auth_flows,
        jwt,
        google,
        queue: Queue::new(pool.clone()),
        pool,
        resources: Arc::new(resources),
        jobs: Arc::new(jobs),
        media: crate::media::Store::new(config.uploads.clone(), config.gcs)?,
        uploads: config.uploads,
        production: config.production,
        origins: Arc::new(config.origins),
    };
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::HeaderName::from_static("x-brain-rpc"),
        ]);
    let cors = cors.expose_headers([header::HeaderName::from_static("x-request-id")]);
    let governor = GovernorConfigBuilder::default()
        .per_second(1)
        .burst_size(120)
        .finish()
        .ok_or("Invalid rate limit")?;
    let limiter = governor.limiter().clone();
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let socket_routes = if sockets_enabled {
        Router::<App>::new().route("/_brain/socket", get(sockets::upgrade))
    } else { Router::<App>::new() };
    let routes = Router::new()
        .merge(socket_routes)
        .route("/health", get(health))
        .route("/_brain/api", get(docs))
        .route("/_brain/auth/signup", post(signup))
        .route("/_brain/auth/login", post(login))
        .route("/_brain/auth/logout", post(logout))
        .route("/_brain/auth/me", get(me))
        .route("/_brain/auth/token", post(token_login))
        .route("/_brain/auth/refresh", post(token_refresh))
        .route("/_brain/auth/google/challenge", post(google_challenge))
        .route("/_brain/auth/google", post(google_login))
        .route(
            "/_brain/auth/password/request",
            post(request_password_reset),
        )
        .route("/_brain/auth/password/reset", post(complete_password_reset))
        .route(
            "/_brain/auth/email/request",
            post(request_email_verification),
        )
        .route(
            "/_brain/auth/email/verify",
            post(complete_email_verification),
        )
        .route("/_brain/auth/users/{id}/role", post(assign_user_role))
        .route(
            "/_brain/webhooks/in/{name}",
            post(webhook_inbound).layer(DefaultBodyLimit::max(16384)),
        )
        .route(
            "/_brain/webhooks/out/{name}",
            post(webhook_outbound).layer(DefaultBodyLimit::max(16384)),
        )
        .route("/_brain/webhooks/out/{name}/{id}", get(webhook_status))
        .route("/_brain/links", post(create_short_link))
        .route(
            "/_brain/links/{id}",
            axum::routing::delete(revoke_short_link),
        )
        .route("/_brain/slugs", post(create_slug))
        .route("/s/{code}", get(resolve_short_link))
        .route("/_brain/jobs/{queue}", post(enqueue))
        .route("/_brain/jobs/status/{id}", get(job_status))
        .route("/api/{name}", get(list).post(create))
        .route("/api/{name}/{id}", get(read).put(update).delete(delete))
        .route(
            "/_brain/uploads",
            post(upload_image).layer(DefaultBodyLimit::max(5 * 1024 * 1024 + 65536)),
        )
        .route("/_brain/uploads/{id}", get(download))
        .route("/_brain/media", get(media_library))
        .route(
            "/_brain/media/{id}",
            axum::routing::patch(media_metadata).delete(media_delete),
        )
        .route("/_brain/media/{id}/variants", post(media_variants))
        .route("/_brain/media/{id}/v/{version}/{preset}", get(media_asset))
        .fallback(|| async { Error::missing() })
        .layer(DefaultBodyLimit::max(65536))
        .layer(middleware::from_fn_with_state(app.clone(), protect))
        .layer(GovernorLayer::new(governor))
        .layer(middleware::from_fn(move |req: Request, next: Next| {
            let limiter = limiter.clone();
            let requests = requests.clone();
            async move {
                if requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 128 == 0 {
                    limiter.retain_recent();
                }
                next.run(req).await
            }
        }))
        .layer(middleware::from_fn(request_deadline))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(middleware::from_fn(response_headers))
        .with_state(app);
    Ok((routes, SocketControl::new(sockets)))
}

/// Loopback binding is enforced by the CLI; the reusable library accepts the caller's owned listener.
pub async fn serve(
    listener: tokio::net::TcpListener,
    pool: PgPool,
    config: Config,
) -> std::result::Result<(), String> {
    serve_pools(listener, pool.clone(), pool, config).await
}
/// Serves HTTP from `pool` and runs the in-process webhook worker on its own `worker_pool` (bulkhead), so slow
/// deliveries cannot take the connections requests need.
pub async fn serve_pools(
    listener: tokio::net::TcpListener,
    pool: PgPool,
    worker_pool: PgPool,
    config: Config,
) -> std::result::Result<(), String> {
    let address = listener
        .local_addr()
        .map_err(|_| "Cannot inspect backend address")?
        .to_string();
    let worker = config
        .webhooks
        .clone()
        .filter(|cfg| cfg.has_outbound())
        .map(|cfg| crate::webhooks::Webhooks::new(worker_pool, cfg, &config.jobs))
        .transpose()?;
    let (app, sockets) = router_with_socket_control(pool.clone(), config).await?;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut http_stop = stop_rx.clone();
    let worker_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut worker = worker.map(|worker| {
        let failed=worker_failed.clone();let stop=stop_tx.clone();
        tokio::spawn(async move {
            let result=worker.work(stop_rx).await;
            if result.is_err() {
                failed.store(true,std::sync::atomic::Ordering::Relaxed);
                eprintln!("{}",json!({"schemaVersion":1,"event":"webhook-worker-stopped","code":"WEBHOOK_WORKER_UNAVAILABLE","fix":"Inspect private database availability and restart the owned backend","valuesCaptured":false}));
                let _=stop.send(true);
            }
            result
        })
    });
    eprintln!(
        "{}",
        json!({"schemaVersion":1,"event":"backend-ready","address":address,"experimental":true})
    );
    let socket_stop = sockets.clone();
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        tokio::select! {_=wait_shutdown()=>{},_=http_stop.changed()=>{}}
        socket_stop.shutdown();
    })
    .await
    .map_err(|_| "Backend HTTP server failed".into());
    sockets.shutdown();
    let sockets_closed = sockets.closed().await;
    if sockets_closed.is_err() {
        eprintln!("{}", json!({"schemaVersion":1,"event":"socket-shutdown-incomplete","code":"SOCKET_SHUTDOWN_TIMEOUT","valuesCaptured":false}));
    }
    let _ = stop_tx.send(true);
    if let Some(ref mut worker) = worker {
        if tokio::time::timeout(Duration::from_secs(6), &mut *worker)
            .await
            .is_err()
        {
            worker.abort();
            let _ = worker.await;
        }
    }
    pool.close().await;
    if sockets_closed.is_err() { return Err("Native sockets did not close within their shutdown budget".into()); }
    if worker_failed.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Native webhook worker stopped; inspect private backend diagnostics".into());
    }
    result
}
async fn request_deadline(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let media = path == "/_brain/media"
        || path.starts_with("/_brain/media/")
        || path == "/_brain/uploads"
        || path.starts_with("/_brain/uploads/");
    match tokio::time::timeout(
        Duration::from_secs(if media { 30 } else { 10 }),
        next.run(req),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
    }
}
async fn response_headers(req: Request, next: Next) -> Response {
    let rid = req
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .unwrap_or_else(Uuid::new_v4)
        .to_string();
    let started = std::time::Instant::now();
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "unmatched".into());
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let accept = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let response = REQUEST_ID.scope(rid.clone(), next.run(req)).await;
    let mut response = crate::response::finalize(&method, &path, accept.as_deref(), &rid, response);
    let headers = response.headers_mut();
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "strict-origin-when-cross-origin"),
        ("x-frame-options", "DENY"),
        ("x-robots-tag", "noindex, nofollow"),
        ("cache-control", "no-store"),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'",
        ),
    ] {
        headers.insert(
            header::HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    headers.insert("x-request-id", rid.parse().unwrap());
    if response.status().is_client_error() || response.status().is_server_error() {
        eprintln!(
            "{}",
            json!({"event":"backend-request-failed","rid":rid,"route":route,"method":method.as_str(),"status":response.status().as_u16(),"ms":started.elapsed().as_millis()})
        );
    }
    response
}
async fn protect(State(app): State<App>, req: Request, next: Next) -> Response {
    // Only this signed, cookie-independent provider route can omit the browser Origin.
    let signed_inbound = req.method() == Method::POST
        && req
            .uri()
            .path()
            .strip_prefix("/_brain/webhooks/in/")
            .is_some_and(|name| name.len() <= 48 && crate::contract::identifier(name));
    if !signed_inbound && !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        let origin = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok());
        if !origin.is_some_and(|o| app.origins.iter().any(|allowed| allowed == o)) {
            return Error::new(StatusCode::FORBIDDEN,"ORIGIN_DENIED","Send the configured site Origin; backend mutations require an explicit allowed origin").into_response();
        }
    }
    next.run(req).await
}
async fn health(State(app): State<App>) -> Result<Json<Value>> {
    sqlx::query("SELECT 1")
        .execute(&app.pool)
        .await
        .map_err(|_| {
            Error::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "DATABASE_UNAVAILABLE",
                "Check Postgres connectivity",
            )
        })?;
    Ok(Json(json!({"ok":true,"schemaVersion":1})))
}
async fn docs(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>> {
    if app.production {
        app.admin(&h).await?;
    }
    Ok(Json(contract::openapi_with_roles(
        &app.resources.values().cloned().collect::<Vec<_>>(),
        app.auth_flows.roles(),
    )))
}
async fn signup(State(app): State<App>, Json(c): Json<Credentials>) -> Result<Json<Value>> {
    app.auth
        .signup(c)
        .await
        .map(|u| Json(json!({"user":u})))
        .map_err(|_| {
            Error::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "SIGNUP_FAILED",
                "Use a valid email and a unique account with a password of 12..1024 bytes",
            )
        })
}
async fn login(State(app): State<App>, Json(c): Json<Credentials>) -> Result<Response> {
    // The backend owns the session mode. A configured JWT authority never falls back to opaque login on failure.
    if app.jwt.is_some() {
        return token_login(State(app), Json(c)).await;
    }
    let Some((user, token)) = app.auth.login(c).await.map_err(|_| {
        Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "LOGIN_UNAVAILABLE",
            "Check input or retry after the password workers are available",
        )
    })?
    else {
        return Err(Error::new(
            StatusCode::UNAUTHORIZED,
            "LOGIN_FAILED",
            "Check credentials or wait for the 15-minute account lockout to expire",
        ));
    };
    let mut response = Json(json!({"user":user})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        auth::cookie(&token, 604800).parse().unwrap(),
    );
    Ok(response)
}
async fn logout(State(app): State<App>, h: HeaderMap) -> Result<Response> {
    if let Some(jwt) = &app.jwt {
        jwt.logout(&h)
            .await
            .map_err(|error| Error::internal(error))?;
    }
    app.auth
        .logout(auth::session_cookie(&h))
        .await
        .map_err(|e| Error::internal(e))?;
    let mut response = Json(json!({"ok":true})).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, auth::cookie("", 0).parse().unwrap());
    response
        .headers_mut()
        .insert("clear-site-data", HeaderValue::from_static("\"storage\""));
    for name in [
        crate::jwt::ACCESS_COOKIE,
        crate::jwt::REFRESH_COOKIE,
        "__Host-brain-google",
    ] {
        response.headers_mut().append(
            header::SET_COOKIE,
            crate::jwt::cookie_header(name, "", 0).parse().unwrap(),
        );
    }
    Ok(response)
}
fn auth_disabled() -> Error {
    Error::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "AUTH_NOT_CONFIGURED",
        "Configure native JWT/Google identity in the private server environment",
    )
}
fn auth_denied() -> Error {
    Error::new(
        StatusCode::UNAUTHORIZED,
        "AUTH_DENIED",
        "Sign in again using the configured account method",
    )
}
fn jwt_response(tokens: crate::jwt::Tokens) -> Response {
    let access = crate::jwt::cookie_header(
        crate::jwt::ACCESS_COOKIE,
        &tokens.access_token,
        crate::jwt::ACCESS_SECONDS,
    );
    let refresh = crate::jwt::cookie_header(
        crate::jwt::REFRESH_COOKIE,
        &tokens.refresh_token,
        crate::jwt::REFRESH_SECONDS,
    );
    let mut response = Json(tokens).into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, access.parse().unwrap());
    response
        .headers_mut()
        .append(header::SET_COOKIE, refresh.parse().unwrap());
    response
}
async fn token_login(
    State(app): State<App>,
    Json(credentials): Json<Credentials>,
) -> Result<Response> {
    let jwt = app.jwt.as_ref().ok_or_else(auth_disabled)?;
    let (user, opaque) = app
        .auth
        .login(credentials)
        .await
        .map_err(|_| auth_denied())?
        .ok_or_else(auth_denied)?;
    let tokens = jwt
        .issue(user, &opaque)
        .await
        .map_err(|error| Error::internal(error))?;
    Ok(jwt_response(tokens))
}
async fn token_refresh(State(app): State<App>, headers: HeaderMap) -> Result<Response> {
    let jwt = app.jwt.as_ref().ok_or_else(auth_disabled)?;
    let tokens = jwt
        .refresh(crate::jwt::cookie(&headers, crate::jwt::REFRESH_COOKIE))
        .await
        .map_err(|error| Error::internal(error))?
        .ok_or_else(auth_denied)?;
    Ok(jwt_response(tokens))
}
async fn google_challenge(State(app): State<App>) -> Result<Response> {
    let google = app.google.as_ref().ok_or_else(auth_disabled)?;
    let nonce = app
        .auth
        .google_challenge()
        .await
        .map_err(|error| Error::internal(error))?;
    let cookie = crate::jwt::cookie_header("__Host-brain-google", &nonce, 300);
    let mut response = Json(json!({"client_id":google.client_id(),"nonce":nonce,"expires_in":300}))
        .into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, cookie.parse().unwrap());
    Ok(response)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoogleCredential {
    credential: String,
}
async fn google_login(
    State(app): State<App>,
    headers: HeaderMap,
    Json(body): Json<GoogleCredential>,
) -> Result<Response> {
    let google = app.google.as_ref().ok_or_else(auth_disabled)?;
    let jwt = app.jwt.as_ref().ok_or_else(auth_disabled)?;
    let nonce = crate::jwt::cookie(&headers, "__Host-brain-google").ok_or_else(auth_denied)?;
    let identity = google
        .verify(&body.credential, nonce)
        .await
        .map_err(|_| {
            Error::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "IDENTITY_UNAVAILABLE",
                "Retry sign-in shortly; identity verification is temporarily unavailable",
            )
        })?
        .ok_or_else(auth_denied)?;
    let (user, opaque) = app
        .auth
        .google_login(&identity, nonce)
        .await
        .map_err(|error| Error::internal(error))?
        .ok_or_else(auth_denied)?;
    let tokens = jwt
        .issue(user, &opaque)
        .await
        .map_err(|error| Error::internal(error))?;
    let mut response = jwt_response(tokens);
    response.headers_mut().append(
        header::SET_COOKIE,
        crate::jwt::cookie_header("__Host-brain-google", "", 0)
            .parse()
            .unwrap(),
    );
    Ok(response)
}
async fn me(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(json!({"user":app.required(&h).await?})))
}
fn viewer(u: Option<&User>) -> crate::visibility::Viewer<'_> {
    crate::visibility::Viewer { id: u.map(|u| u.id), role: u.map(|u| u.role.as_str()) }
}
/// Declared filters, sort, cursor, totals, aggregates, counts and includes (see query.rs).
async fn list(
    State(app): State<App>,
    Path(name): Path<String>,
    h: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    let u = app.user(&h).await?;
    let viewer = viewer(u.as_ref());
    let req = crate::query::parse(r, &app.resources, &pairs).map_err(Error::validation)?;
    let mut out = crate::query::list(&app.pool, &app.resources, r, &req, viewer)
        .await
        .map_err(|error| match error {
            crate::query::ListError::Invalid(fields) => Error::validation(fields),
            crate::query::ListError::Denied(_) => Error::forbidden(),
            crate::query::ListError::Database(e) => Error::internal(db::safe(e)),
        })?;
    if let Some(items) = out["items"].as_array_mut() {
        include(&app, r, &req.include, items, viewer).await?;
    }
    Ok(Json(out))
}
async fn include(
    app: &App,
    r: &Resource,
    names: &[String],
    rows: &mut [Value],
    viewer: crate::visibility::Viewer<'_>,
) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    crate::relations::include(&app.pool, &app.resources, r, names, rows, viewer)
        .await
        .map_err(|error| match error {
            crate::relations::IncludeError::Unknown(name) => Error::validation(json!({"include": format!("`{name}` is not includable")})),
            crate::relations::IncludeError::Denied(_) => Error::forbidden(),
            crate::relations::IncludeError::Database => Error::internal("include"),
        })
}
/// Whether the viewer owns `id` through a declared `owner.<ref>` grant of `operation`.
async fn owns_through(app: &App, r: &Resource, operation: &str, u: &User, id: Uuid) -> Result<bool> {
    crate::ownership::owns_through(&app.pool, r, &app.resources, operation, viewer(Some(u)), id)
        .await
        .map_err(|error| Error::internal(db::safe(error)))
}
fn relational(r: &Resource, operation: &str) -> bool {
    let grants = match operation {
        "read" => &r.policy.read,
        "create" => &r.policy.create,
        "update" => &r.policy.update,
        _ => &r.policy.delete,
    };
    grants.iter().any(|g| crate::ownership::path(g).is_some())
}
async fn read(
    State(app): State<App>,
    Path((name, id)): Path<(String, Uuid)>,
    h: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    let u = app.user(&h).await?;
    let mut names = Vec::new();
    for (key, value) in &pairs {
        if key != "include" || !names.is_empty() {
            return Err(Error::validation(json!({key: "Single-record reads accept only include"})));
        }
        names = value.split(',').map(str::to_string).collect();
    }
    let mut row = db::get(&app.pool, r, id)
        .await
        .map_err(|e| Error::internal(e))?
        .ok_or_else(Error::missing)?;
    let owns = u
        .as_ref()
        .is_some_and(|u| row["owner"].as_str() == Some(&u.id.to_string()));
    let mut through = false;
    if !r
        .policy
        .allows("read", u.as_ref().map(|u| u.role.as_str()), owns)
    {
        through = match &u {
            Some(u) if relational(r, "read") => owns_through(&app, r, "read", u, id).await?,
            _ => false,
        };
        if !through {
            return Err(Error::forbidden());
        }
    } else if let Some(u) = &u
        && relational(r, "read")
        && r.fields.values().any(|f| f.visible.is_some())
    {
        through = owns_through(&app, r, "read", u, id).await?;
    }
    crate::visibility::project(r, &mut row, viewer(u.as_ref()), through);
    include(&app, r, &names, std::slice::from_mut(&mut row), viewer(u.as_ref())).await?;
    Ok(Json(row))
}
fn before_error(error: crate::before_hooks::HookFailure, r: &Resource) -> Error {
    match error {
        crate::before_hooks::HookFailure::Rejected(fields) => {
            let fields: BTreeMap<String,String>=fields.into_iter().take(64).filter(|name|r.fields.contains_key(name)).map(|name|(name,"Rejected by the mandatory before-hook; review the field and server validation".into())).collect();
            let mut error = Error::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "BEFORE_HOOK_REJECTED",
                "Review the declared before-hook validation and submit corrected fields",
            );
            error.fields = Some(json!(fields));
            error
        }
        crate::before_hooks::HookFailure::Capacity => Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "BEFORE_HOOK_CAPACITY",
            "Retry after current validations finish; hook concurrency is bounded without queueing",
        ),
        crate::before_hooks::HookFailure::Unavailable => Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "BEFORE_HOOK_UNAVAILABLE",
            "Inspect the private hook executor diagnostic; no mutation was committed",
        ),
    }
}
async fn before(
    app: &App,
    r: &Resource,
    operation: &str,
    payload: Value,
    existing: Option<Value>,
    user: &User,
) -> Result<Value> {
    crate::before_hooks::execute(
        app.before_hooks.as_ref(),
        r,
        operation,
        payload,
        existing,
        user,
        request_reference(),
    )
    .await
    .map_err(|error| before_error(error, r))
}
/// Declared unique/check violations are client errors with field names; everything else stays generic.
fn write_error(r: &Resource, error: sqlx::Error) -> Error {
    if let Some((field, target)) = crate::relations::missing_reference(r, &error) {
        let mut e = Error::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "REFERENCE_NOT_FOUND",
            "Reference an existing record of the target resource in the listed fields",
        );
        e.fields = Some(json!({ field: format!("No {target} record with this id") }));
        return e;
    }
    match db::constraint_fields(r, &error) {
        Some(("unique", fields)) => {
            let mut e = Error::new(
                StatusCode::CONFLICT,
                "UNIQUE_CONFLICT",
                "Another record already uses this value; choose a different value for the listed fields",
            );
            e.fields = Some(json!(fields
                .into_iter()
                .map(|f| (f, "Already used".to_string()))
                .collect::<BTreeMap<_, _>>()));
            e
        }
        Some(_) => Error::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "CONSTRAINT_VIOLATION",
            "The record violates a declared type or check constraint; correct the values and retry",
        ),
        None => Error::internal(db::safe(error)),
    }
}
/// Row filter for a write: None when the role may change any row, the user id for direct owners, or None after the
/// row was proven to be owned through a declared relation (`through`). Anything else is reported as missing.
async fn scoped_owner(
    app: &App,
    r: &Resource,
    operation: &str,
    u: &User,
    existing: &Value,
    id: Uuid,
) -> Result<(Option<Uuid>, bool)> {
    if r.policy.allows(operation, Some(&u.role), false) {
        return Ok((None, false));
    }
    let direct = existing["owner"].as_str() == Some(u.id.to_string().as_str());
    if direct && r.policy.allows(operation, Some(&u.role), true) {
        return Ok((Some(u.id), false));
    }
    if relational(r, operation) && owns_through(app, r, operation, u, id).await? {
        return Ok((None, true));
    }
    Err(Error::missing())
}
/// A relational writer must keep pointing the record at a parent they own.
async fn owns_data(app: &App, r: &Resource, operation: &str, u: &User, data: &Value) -> Result<bool> {
    crate::ownership::data_owned_through(&app.pool, r, &app.resources, operation, viewer(Some(u)), data)
        .await
        .map_err(|error| Error::internal(db::safe(error)))
}
fn conflict() -> Error {
    Error::new(
        StatusCode::CONFLICT,
        "RESOURCE_CHANGED",
        "Reload this record and rerun validation; another mutation changed it while the hook was running",
    )
}
async fn create(
    State(app): State<App>,
    Path(name): Path<String>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    let u = app.required(&h).await?;
    let through = !r.policy.allows("create", Some(&u.role), false);
    if through && !relational(r, "create") {
        return Err(Error::forbidden());
    }
    crate::visibility::reject_server_fields(r, &input).map_err(Error::validation)?;
    let input = before(&app, r, "create", input, None, &u).await?;
    let data = r.input(&input).map_err(Error::validation)?;
    if through && !owns_data(&app, r, "create", &u, &data).await? {
        return Err(Error::forbidden());
    }
    let mut record = db::create(&app.pool, r, u.id, data)
        .await
        .map_err(|error| write_error(r, error))?;
    app.sockets.changed(&name, "create", &record);
    crate::visibility::project(r, &mut record, viewer(Some(&u)), through);
    Ok(Json(record))
}
async fn update(
    State(app): State<App>,
    Path((name, id)): Path<(String, Uuid)>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    let u = app.required(&h).await?;
    if !r.policy.allows("update", Some(&u.role), true) && !relational(r, "update") {
        return Err(Error::forbidden());
    }
    // Capture a real row before application code; policy never relies on client-supplied existing data.
    let (existing, revision) = db::get_version(&app.pool, r, id)
        .await
        .map_err(|error| Error::internal(db::safe(error)))?
        .ok_or_else(Error::missing)?;
    let (owner, through) = scoped_owner(&app, r, "update", &u, &existing, id).await?;
    crate::visibility::reject_server_fields(r, &input).map_err(Error::validation)?;
    let mut input = input;
    crate::visibility::keep_stored(r, &mut input, &existing["data"]);
    let input = before(&app, r, "update", input, Some(existing), &u).await?;
    let data = r.input(&input).map_err(Error::validation)?;
    if through && !owns_data(&app, r, "update", &u, &data).await? {
        return Err(Error::forbidden());
    }
    let mut record = db::update_checked(&app.pool, r, id, owner, data, Some(&revision))
        .await
        .map_err(|error| write_error(r, error))?
        .ok_or_else(conflict)?;
    app.sockets.changed(&name, "update", &record);
    crate::visibility::project(r, &mut record, viewer(Some(&u)), through);
    Ok(Json(record))
}
async fn delete(
    State(app): State<App>,
    Path((name, id)): Path<(String, Uuid)>,
    h: HeaderMap,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    let u = app.required(&h).await?;
    if !r.policy.allows("delete", Some(&u.role), true) && !relational(r, "delete") {
        return Err(Error::forbidden());
    }
    let (existing, revision) = db::get_version(&app.pool, r, id)
        .await
        .map_err(|error| Error::internal(db::safe(error)))?
        .ok_or_else(Error::missing)?;
    let (owner, _) = scoped_owner(&app, r, "delete", &u, &existing, id).await?;
    // Retain only fixed authority metadata before handing the row to application code.
    let deleted = json!({"id": existing["id"], "owner": existing["owner"]});
    let output = before(
        &app,
        r,
        "delete",
        existing["data"].clone(),
        Some(existing),
        &u,
    )
    .await?;
    if r.hooks.before("delete").is_some() {
        r.input(&output).map_err(Error::validation)?;
    }
    if !db::delete_checked(&app.pool, r, id, owner, Some(&revision))
        .await
        .map_err(|error| {
            if crate::relations::referenced(&error) {
                return Error::new(
                    StatusCode::CONFLICT,
                    "REFERENCE_IN_USE",
                    "Other records still reference this one (on_delete: restrict); delete or re-point them first",
                );
            }
            Error::internal(db::safe(error))
        })?
    {
        return Err(conflict());
    }
    app.sockets.changed(&name, "delete", &deleted);
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enqueue {
    payload: Value,
    idempotency_key: Option<String>,
    max_attempts: Option<i32>,
}
async fn enqueue(
    State(app): State<App>,
    Path(queue): Path<String>,
    h: HeaderMap,
    Json(input): Json<Enqueue>,
) -> Result<Json<Value>> {
    app.admin(&h).await?;
    let contract = app.jobs.get(&queue).ok_or_else(Error::missing)?;
    let payload = contract.input(&input.payload).map_err(Error::validation)?;
    let id = app
        .queue
        .enqueue(
            &queue,
            payload,
            input.idempotency_key.as_deref(),
            input.max_attempts.unwrap_or(3),
        )
        .await
        .map_err(|e| Error::internal(e))?;
    Ok(Json(json!({"schemaVersion":1,"id":id})))
}
async fn job_status(
    State(app): State<App>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
) -> Result<Json<Value>> {
    app.admin(&h).await?;
    app.queue
        .status(id)
        .await
        .map_err(|e| Error::internal(e))?
        .map(Json)
        .ok_or_else(Error::missing)
}
async fn upload_image(
    State(app): State<App>,
    h: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<Value>> {
    let user = app.required(&h).await?;
    let permit = app.media.image_slot().map_err(|_| {
        Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "IMAGE_WORKERS_BUSY",
            "Retry when image processing capacity is available",
        )
    })?;
    let field = multipart
        .next_field()
        .await
        .map_err(|_| Error::validation(json!({"file":"Invalid multipart body"})))?
        .ok_or_else(|| Error::validation(json!({"file":"Required"})))?;
    if field.name() != Some("file") {
        return Err(Error::validation(
            json!({"file":"Use one multipart field named file"}),
        ));
    }
    let bytes = field
        .bytes()
        .await
        .map_err(|_| Error::validation(json!({"file":"Image exceeds upload limit"})))?;
    if multipart
        .next_field()
        .await
        .map_err(|_| Error::validation(json!({"file":"Invalid multipart body"})))?
        .is_some()
    {
        return Err(Error::validation(json!({"file":"Send exactly one image"})));
    }
    let (prepared, _permit) = tokio::task::spawn_blocking(move || {
        crate::media::prepare(bytes.to_vec(), &crate::media::defaults())
            .map(|prepared| (prepared, permit))
    })
    .await
    .map_err(|e| Error::internal(e))?
    .map_err(|_| {
        Error::validation(json!({"file":"Use valid PNG, JPEG or WebP <=5 MB and 4096x4096"}))
    })?;
    crate::media::insert(&app.media, &app.pool, user.id, prepared, "", "")
        .await
        .map(Json)
        .map_err(media_error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Download {
    #[serde(default)]
    thumbnail: bool,
}
async fn download(
    State(app): State<App>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Query(q): Query<Download>,
) -> Result<Response> {
    let u = app.required(&h).await?;
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM _brain_uploads WHERE id=$1")
        .bind(id)
        .fetch_optional(&app.pool)
        .await
        .map_err(|e| Error::internal(e))?;
    if owner.is_none() {
        return Err(Error::missing());
    }
    if owner != Some(u.id) && u.role != "admin" {
        return Err(Error::forbidden());
    }
    let version: i32 = sqlx::query_scalar("SELECT version FROM _brain_uploads WHERE id=$1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .map_err(|error| Error::internal(error))?;
    if let Some(asset) = crate::media::asset(
        &app.pool,
        id,
        version,
        if q.thumbnail { "thumbnail" } else { "full" },
        u.id,
        u.role == "admin",
    )
    .await
    .map_err(|error| Error::internal(error))?
    {
        let data = app.media.read(&asset).await.map_err(media_error)?;
        return Ok((
            [
                (header::CONTENT_TYPE, format!("image/{}", asset.format)),
                (header::CONTENT_DISPOSITION, "attachment".into()),
            ],
            Body::from(data),
        )
            .into_response());
    }
    let path = app.uploads.join(format!(
        "{id}{}",
        if q.thumbnail { ".thumb.png" } else { ".png" }
    ));
    if !tokio::fs::symlink_metadata(&path)
        .await
        .is_ok_and(|m| m.file_type().is_file())
    {
        return Err(Error::missing());
    }
    let data = tokio::fs::read(path).await.map_err(|_| Error::missing())?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        Body::from(data),
    )
        .into_response())
}

fn media_error(error: String) -> Error {
    match error.as_str() {
        "Image workers busy; retry shortly" | "GCS workers busy; retry shortly" => Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "MEDIA_WORKERS_BUSY",
            "Retry when media processing capacity is available",
        ),
        "Image changed; reload metadata before rebuilding"
        | "Image rebuild lease changed; inspect metadata before retrying"
        | "Image cleanup is pending; finish the owned cleanup before rebuilding"
        | "Image publication changed; inspect metadata before retrying" => Error::new(
            StatusCode::CONFLICT,
            "MEDIA_CHANGED",
            "Reload image metadata and finish pending cleanup before retrying",
        ),
        "Legacy image has no preserved original" => Error::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "MEDIA_ORIGINAL_MISSING",
            "Upload the source image again before rebuilding variants",
        ),
        _ => Error::internal(error),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaPage {
    #[serde(default = "media_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}
fn media_limit() -> i64 {
    24
}
async fn media_library(
    State(app): State<App>,
    headers: HeaderMap,
    Query(page): Query<MediaPage>,
) -> Result<Json<Value>> {
    let user = app.required(&headers).await?;
    if !(1..=100).contains(&page.limit) || !(0..=10000).contains(&page.offset) {
        return Err(Error::validation(
            json!({"page":"Use limit 1..100 and offset 0..10000"}),
        ));
    }
    crate::media::library(&app.pool, user.id, page.limit, page.offset)
        .await
        .map(Json)
        .map_err(|error| Error::internal(error))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaMetadata {
    alt: String,
    caption: String,
}
async fn media_metadata(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(meta): Json<MediaMetadata>,
) -> Result<Json<Value>> {
    let user = app.required(&headers).await?;
    if meta.alt.len() > 1024
        || meta.caption.len() > 4096
        || meta.alt.contains('\0')
        || meta.caption.contains('\0')
    {
        return Err(Error::validation(
            json!({"metadata":"Alt must be <=1024 bytes and caption <=4096 bytes"}),
        ));
    }
    if !crate::media::update_metadata(
        &app.pool,
        id,
        user.id,
        user.role == "admin",
        &meta.alt,
        &meta.caption,
    )
    .await
    .map_err(|error| Error::internal(error))?
    {
        return Err(Error::missing());
    }
    Ok(Json(json!({"ok":true,"id":id})))
}
async fn media_delete(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let user = app.required(&headers).await?;
    if !crate::media::delete(&app.pool, id, user.id, user.role == "admin")
        .await
        .map_err(|error| Error::internal(error))?
    {
        return Err(Error::missing());
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"id":id,"cleanupQueued":true})),
    )
        .into_response())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaPresets {
    #[serde(default = "crate::media::defaults")]
    presets: Vec<crate::media::Preset>,
}
async fn media_variants(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<MediaPresets>,
) -> Result<Json<Value>> {
    let user = app.required(&headers).await?;
    crate::media::validate_presets(&body.presets).map_err(|_|Error::validation(json!({"presets":"Use at most six uniquely named PNG/JPEG/WebP presets with dimensions 1..4096; JPEG quality 1..100"})))?;
    let owner: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM _brain_uploads WHERE id=$1 AND (user_id=$2 OR $3)")
            .bind(id)
            .bind(user.id)
            .bind(user.role == "admin")
            .fetch_optional(&app.pool)
            .await
            .map_err(|error| Error::internal(error))?;
    let owner = owner.ok_or_else(Error::missing)?;
    crate::media::build_variants(&app.media, &app.pool, id, owner, body.presets)
        .await
        .map(Json)
        .map_err(media_error)
}
async fn media_asset(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, version, preset)): Path<(Uuid, i32, String)>,
) -> Result<Response> {
    let user = app.required(&headers).await?;
    if version < 1
        || preset.is_empty()
        || preset.len() > 48
        || !preset.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err(Error::validation(
            json!({"image":"Use a positive version and declared preset name"}),
        ));
    }
    let asset = crate::media::asset(
        &app.pool,
        id,
        version,
        &preset,
        user.id,
        user.role == "admin",
    )
    .await
    .map_err(|error| Error::internal(error))?
    .ok_or_else(Error::missing)?;
    let bytes = app.media.read(&asset).await.map_err(media_error)?;
    let mut response = Body::from(bytes).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        format!("image/{}", asset.format)
            .parse()
            .map_err(|_| Error::internal("Invalid image media type"))?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static(if preset == "original" {
            "attachment"
        } else {
            "inline"
        }),
    );
    response.headers_mut().insert(
        header::ETAG,
        format!("\"{}\"", asset.sha256).parse().unwrap(),
    );
    Ok(response)
}

/// Both interactive cancellation and a process manager's SIGTERM follow the same owned cleanup path.
pub async fn wait_shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}};
        } else {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

// Native owner-managed links; no destination, credential or query content enters request logs.
async fn create_short_link(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<crate::links::Create>,
) -> Result<Response> {
    let user = app.user(&headers).await?.ok_or_else(auth_denied)?;
    let link=crate::links::create(&app.pool,user.id,input,&app.origins).await.map_err(|_|Error::new(StatusCode::BAD_REQUEST,"SHORT_LINK_INVALID","Use an available public code, configured destination origin, valid expiry and available owner capacity"))?;
    Ok((StatusCode::CREATED, Json(link)).into_response())
}
async fn resolve_short_link(State(app): State<App>, Path(code): Path<String>) -> Result<Response> {
    let target = crate::links::resolve(&app.pool, &code, &app.origins)
        .await
        .map_err(|error| Error::internal(error))?
        .ok_or_else(Error::missing)?;
    let mut response = StatusCode::FOUND.into_response();
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&target).map_err(|_| {
            Error::new(
                StatusCode::BAD_REQUEST,
                "SHORT_LINK_TARGET_INVALID",
                "Inspect the configured target origin and redirect policy",
            )
        })?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
async fn revoke_short_link(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let user = app.user(&headers).await?.ok_or_else(auth_denied)?;
    if !crate::links::revoke(&app.pool, id, user.id)
        .await
        .map_err(|error| Error::internal(error))?
    {
        return Err(Error::missing());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SlugRequest {
    title: String,
    limit: Option<usize>,
}
async fn create_slug(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<SlugRequest>,
) -> Result<Json<Value>> {
    app.user(&headers).await?.ok_or_else(auth_denied)?;
    let value = crate::links::slug(&input.title, input.limit.unwrap_or(80)).map_err(|_| {
        Error::new(
            StatusCode::BAD_REQUEST,
            "SLUG_INVALID",
            "Provide a bounded public title and a limit from 8 to 128 characters",
        )
    })?;
    Ok(Json(json!({"schemaVersion":1,"slug":value})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetRequest {
    email: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyAccountRequest {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetConfirmation {
    token: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailConfirmation {
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleAssignment {
    role: String,
}
fn account_flow_error(error: crate::auth_flows::Error) -> Error {
    use crate::auth_flows::Error as Flow;
    match error {
        Flow::Unavailable => Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            error.code(),
            "Configure private mail delivery and the public frontend base URL",
        ),
        Flow::Busy => Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            error.code(),
            "Wait before requesting another account email",
        ),
        Flow::InvalidInput => Error::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            error.code(),
            "Use a valid account request; links expire and can be used only once",
        ),
        Flow::Database => Error::internal(error),
    }
}
async fn request_password_reset(
    State(app): State<App>,
    Json(input): Json<ResetRequest>,
) -> Result<Response> {
    app.auth_flows
        .request_reset(&input.email)
        .await
        .map_err(account_flow_error)?;
    Ok((StatusCode::ACCEPTED, Json(json!({"accepted":true}))).into_response())
}
async fn complete_password_reset(
    State(app): State<App>,
    Json(input): Json<ResetConfirmation>,
) -> Result<Response> {
    if !app
        .auth_flows
        .complete_reset(&input.token, input.password)
        .await
        .map_err(account_flow_error)?
    {
        return Err(account_flow_error(crate::auth_flows::Error::InvalidInput));
    }
    let mut response = Json(json!({"ok":true})).into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, auth::cookie("", 0).parse().unwrap());
    for name in [
        crate::jwt::ACCESS_COOKIE,
        crate::jwt::REFRESH_COOKIE,
        "__Host-brain-google",
    ] {
        response.headers_mut().append(
            header::SET_COOKIE,
            crate::jwt::cookie_header(name, "", 0).parse().unwrap(),
        );
    }
    response
        .headers_mut()
        .insert("clear-site-data", HeaderValue::from_static("\"storage\""));
    Ok(response)
}
async fn request_email_verification(
    State(app): State<App>,
    headers: HeaderMap,
    Json(_input): Json<EmptyAccountRequest>,
) -> Result<Response> {
    let user = app.user(&headers).await?.ok_or_else(auth_denied)?;
    app.auth_flows
        .request_verification(user.id)
        .await
        .map_err(account_flow_error)?;
    Ok((StatusCode::ACCEPTED, Json(json!({"accepted":true}))).into_response())
}
async fn complete_email_verification(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<EmailConfirmation>,
) -> Result<Response> {
    let user = app.user(&headers).await?.ok_or_else(auth_denied)?;
    if !app
        .auth_flows
        .verify_email(user.id, &input.token)
        .await
        .map_err(account_flow_error)?
    {
        return Err(account_flow_error(crate::auth_flows::Error::InvalidInput));
    }
    Ok(Json(json!({"ok":true})).into_response())
}
async fn assign_user_role(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<RoleAssignment>,
) -> Result<Response> {
    let user = app.user(&headers).await?.ok_or_else(auth_denied)?;
    if user.role != "admin" {
        return Err(Error::forbidden());
    }
    if !app
        .auth_flows
        .assign_role(user.id, id, &input.role)
        .await
        .map_err(account_flow_error)?
    {
        return Err(Error::forbidden());
    }
    Ok(Json(json!({"ok":true})).into_response())
}

fn webhook_error(error: crate::webhooks::Error) -> Error {
    use crate::webhooks::Error as W;
    let status = match error {
        W::Missing => StatusCode::NOT_FOUND,
        W::Signature | W::Timestamp => StatusCode::FORBIDDEN,
        W::Payload => StatusCode::UNPROCESSABLE_ENTITY,
        W::Conflict => StatusCode::CONFLICT,
        W::Capacity | W::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    Error::new(status, error.code(), error.fix())
}
async fn webhook_inbound(
    State(app): State<App>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let webhooks = app.webhooks.as_ref().ok_or_else(Error::missing)?;
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim)
        != Some("application/json")
        || headers
            .get(header::CONTENT_ENCODING)
            .is_some_and(|v| v != "identity")
    {
        return Err(webhook_error(crate::webhooks::Error::Payload));
    }
    let one = |key: &str| -> Result<&str> {
        let mut values = headers.get_all(key).iter();
        let value = values
            .next()
            .ok_or_else(|| webhook_error(crate::webhooks::Error::Signature))?;
        if values.next().is_some() {
            return Err(webhook_error(crate::webhooks::Error::Signature));
        }
        value
            .to_str()
            .map_err(|_| webhook_error(crate::webhooks::Error::Signature))
    };
    let receipt = webhooks
        .inbound(
            &name,
            one("x-brain-webhook-id")?,
            one("x-brain-webhook-timestamp")?,
            one("x-brain-webhook-signature")?,
            &body,
        )
        .await
        .map_err(webhook_error)?;
    Ok((StatusCode::ACCEPTED, Json(receipt)).into_response())
}
async fn webhook_outbound(
    State(app): State<App>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Result<Response> {
    app.admin(&headers).await?;
    let input = serde_json::from_value::<crate::webhooks::Publish>(value)
        .map_err(|_| webhook_error(crate::webhooks::Error::Payload))?;
    let receipt = app
        .webhooks
        .as_ref()
        .ok_or_else(Error::missing)?
        .outbound(&name, input)
        .await
        .map_err(webhook_error)?;
    Ok((StatusCode::ACCEPTED, Json(receipt)).into_response())
}
async fn webhook_status(
    State(app): State<App>,
    Path((name, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    app.admin(&headers).await?;
    app.webhooks
        .as_ref()
        .ok_or_else(Error::missing)?
        .status(&name, id)
        .await
        .map_err(webhook_error)?
        .map(Json)
        .ok_or_else(Error::missing)
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[tokio::test]
    async fn internal_errors_return_safe_response_without_waiting_for_stack_symbolization() {
        struct Sensitive;
        impl std::fmt::Display for Sensitive {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("Private error values must never be formatted")
            }
        }
        let start = std::time::Instant::now();
        let response = Error::internal(Sensitive).into_response();
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "Diagnostic symbolization must run outside request execution"
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], "BACKEND_INTERNAL");
        assert!(!String::from_utf8_lossy(&bytes).contains("Sensitive"));
    }
}
