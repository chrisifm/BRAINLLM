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
    extract::{DefaultBodyLimit, FromRequestParts, Multipart, Query, Request, State},
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
#[path = "domain_http.rs"]
mod domain_http;
#[path = "organizations_http.rs"]
mod organizations_http;
#[path = "credits_http.rs"]
mod credits_http;
#[path = "files_http.rs"]
mod files_http;
#[path = "edge_http.rs"]
mod edge_http;
#[path = "routes_http.rs"]
mod routes_http;
#[path = "cache_http.rs"]
pub mod cache_http;
pub use edge_http::{ClientIp, EdgeControl, resolve_client};
pub use sockets::{SocketConfig, SocketControl};
#[path = "http_api_keys.rs"]
mod http_api_keys;

tokio::task_local! { static REQUEST_ID: String; static REQUEST_OPERATION: (String, Option<String>); }
/// The route template and the resource it addresses: finite identifiers, never request values.
pub(crate) fn request_operation() -> Option<(String, Option<String>)> {
    REQUEST_OPERATION.try_with(Clone::clone).ok()
}
fn operation_of(method: &axum::http::Method, route: &str, path: &str) -> (String, Option<String>) {
    let resource = path.strip_prefix("/api/").and_then(|rest| rest.split('/').next()).filter(|name| {
        (1..=64).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    });
    (format!("{method} {route}"), resource.map(str::to_owned))
}
pub(crate) fn request_reference() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}

/// True once per process: a binary without symbols stays so, and each `backend-internal-error` already carries its location.
fn first_unsymbolized_stack() -> bool {
    static NOTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    !NOTED.swap(true, std::sync::atomic::Ordering::Relaxed)
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
    /// None disables project API keys (any presented key is rejected with API_KEYS_DISABLED).
    pub api_keys: Option<crate::api_keys::Config>,
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
    disabled_webhooks: Arc<Vec<String>>,
    before_hooks: Option<crate::before_hooks::Executor>,
    resources: Arc<BTreeMap<String, Resource>>,
    jobs: Arc<BTreeMap<String, Resource>>,
    queue: Queue,
    uploads: PathBuf,
    media: crate::media::Store,
    production: bool,
    origins: Arc<Vec<String>>,
    domain: Arc<crate::domain::Domain>,
    api_keys: Option<crate::api_keys::Keys>,
    platform: Arc<files_http::State>,
    routes: Arc<routes_http::State>,
    cluster: crate::cluster::Cluster,
    organizations: Arc<crate::organizations::Organizations>,
    captcha: Arc<crate::captcha::Captcha>,
}
#[derive(Debug)]
pub struct Error {
    status: StatusCode,
    code: &'static str,
    fix: &'static str,
    fields: Option<Value>,
    /// Stable catalog code (`brain explain`); safe in production.
    brain_code: Option<&'static str>,
    /// Configuration evidence; only set outside production.
    detail: Option<Value>,
}
impl Error {
    fn new(status: StatusCode, code: &'static str, fix: &'static str) -> Self {
        Self {
            status,
            code,
            fix,
            fields: None,
            brain_code: None,
            detail: None,
        }
    }
    /// An origin refusal: the code always, the seen and allowed origins only outside production.
    fn origin_refused(mut self, brain_code: &'static str, origin: Option<&str>, allowed: &[String], production: bool) -> Self {
        self.brain_code = Some(brain_code);
        if !production {
            self.detail = Some(crate::origins::detail(brain_code, origin, allowed));
        }
        self
    }
    /// A failed database operation: the cause goes to the log, the client gets a retryable or an internal response.
    #[track_caller]
    fn database(error: sqlx::Error) -> Self {
        Self::database_in(&error, std::panic::Location::caller(), json!({}))
    }
    /// `context` holds finite identifiers only (resource, operation, branch), never request values.
    fn database_in(error: &sqlx::Error, at: &'static std::panic::Location<'static>, context: Value) -> Self {
        crate::platform::note_database_error(error);
        db::log_failure(error, at, context);
        // Text Postgres cannot store (a NUL or invalid bytes) came from the request.
        if db::sqlstate(error).is_some_and(|state| matches!(state.as_str(), "22P05" | "22021")) {
            return Self::new(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_INPUT", "Remove NUL characters and invalid bytes from the text values");
        }
        if db::sqlstate(error).is_some_and(|state| db::too_large(&state)) {
            return Self::value_too_large();
        }
        if db::sqlstate(error).is_some_and(|state| db::retryable(&state)) {
            return Self::new(StatusCode::SERVICE_UNAVAILABLE, "DATABASE_BUSY", "The database could not finish in time and nothing was changed; retry after the Retry-After delay");
        }
        Self::internal_at(at)
    }
    fn value_too_large() -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "VALUE_TOO_LARGE", "Nothing was written: a value exceeds what the database can store or index, for example long text in a unique or indexed field; shorten it or stop indexing that field")
    }
    #[track_caller]
    fn internal(error: impl std::fmt::Display + 'static) -> Self {
        let at = std::panic::Location::caller();
        if let Some(error) = (&error as &dyn std::any::Any).downcast_ref::<sqlx::Error>() {
            return Self::database_in(error, at, json!({}));
        }
        Self::internal_at(at)
    }
    fn internal_at(at: &'static std::panic::Location<'static>) -> Self {
        // The driver error became sanitized text on its way here; the request still knows which limit it hit.
        match crate::platform::database_limit() {
            Some(crate::platform::DatabaseLimit::TooLarge) => return Self::value_too_large(),
            Some(_) => return Self::new(StatusCode::SERVICE_UNAVAILABLE, "DATABASE_BUSY", "The database could not finish in time and the failed step was rolled back; retry after the Retry-After delay"),
            None => {}
        }
        let rid = REQUEST_ID.try_with(Clone::clone).ok();
        static DIAGNOSTIC_BUSY: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        let reserved = DIAGNOSTIC_BUSY
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok();
        db::diagnostic(&json!({"schemaVersion":1,"event":"backend-internal-error","rid":rid,"code":"BACKEND_INTERNAL","location":{"file":at.file(),"line":at.line(),"column":at.column()},"stackDeferred":reserved,"stackOmitted":!reserved,"queueDepth":0,"private":true,"valuesCaptured":false}));
        if reserved {
            let trace = std::backtrace::Backtrace::force_capture();
            let operation = request_operation();
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
                // A stripped release binary has no symbols: its frames say nothing, so the event says that instead.
                let useful=stack_is_symbolized(&stack.text);
                if !useful && !first_unsymbolized_stack() {return;}
                db::diagnostic(&json!({"schemaVersion":1,"event":"backend-internal-stack","operation":operation.as_ref().map(|o|&o.0),"resource":operation.as_ref().and_then(|o|o.1.as_ref()),"rid":rid,"location":{"file":at.file(),"line":at.line(),"column":at.column()},"stack":useful.then_some(&stack.text),"stackUnavailable":(!useful).then_some("binary has no symbols; use the location"),"truncated":stack.truncated,"queueDepth":0,"private":true,"valuesCaptured":false}));
            });
            if diagnostic.is_err() {
                DIAGNOSTIC_BUSY.store(false, std::sync::atomic::Ordering::Release);
                crate::log_line!(
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
            "The declared policy does not grant this operation to this identity (rules evaluated: role, owner, owner.<ref>, organization membership and row conditions); sign in if anonymous, otherwise use an account those rules allow",
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
            brain_code: None,
            detail: None,
        }
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut error=json!({"code":self.code,"fix":self.fix,"fields":self.fields});
        if let Some(code)=self.brain_code { error["brainCode"]=json!(code); }
        if let Some(detail)=self.detail { error["origin"]=detail; }
        let mut response=(self.status,Json(json!({"schemaVersion":1,"rid":REQUEST_ID.try_with(Clone::clone).ok(),"error":error}))).into_response();
        response
            .extensions_mut()
            .insert(crate::response::DefaultError);
        if self.code == "DATABASE_BUSY" {
            response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        if self.code == "AUTH_DENIED" {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}
/// Whether a captured backtrace names this crate's code, which a stripped binary cannot do.
fn stack_is_symbolized(stack: &str) -> bool {
    stack.contains("brain_backend") && !stack.contains("__mh_execute_header")
}
type Result<T> = std::result::Result<T, Error>;
/// A JSON body of a fixed shape. A body of the wrong shape says which key to change; the answer depends only on the
/// keys sent, before any account or row is read, and never repeats a value or lists what else the shape declares.
pub(crate) struct Strict<T>(pub T);
impl<S: Send + Sync, T: serde::de::DeserializeOwned> axum::extract::FromRequest<S> for Strict<T> {
    type Rejection = Response;
    async fn from_request(req: Request, state: &S) -> std::result::Result<Self, Response> {
        use axum::extract::rejection::JsonRejection;
        match <Json<T> as axum::extract::FromRequest<S>>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(JsonRejection::JsonDataError(error)) => Err(shape_error(&error.body_text()).into_response()),
            Err(other) => Err(other.into_response()),
        }
    }
}
fn shape_error(text: &str) -> Error {
    let named = |marker: &str| {
        let start = text.find(marker)? + marker.len();
        let key = text[start..].split('`').next()?;
        Some(key.chars().take(64).collect::<String>())
    };
    let fields = if let Some(key) = named("unknown field `") {
        json!({ key: "This endpoint does not accept this field: remove it" })
    } else if let Some(key) = named("missing field `") {
        json!({ key: "This field is required" })
    } else {
        json!({ "body": "A field has the wrong type: compare the body with the API contract" })
    };
    Error {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: "INVALID_INPUT",
        fix: "Apply the field errors and send only the fields the API contract documents for this endpoint",
        fields: Some(fields),
        brain_code: None,
        detail: None,
    }
}
#[cfg(test)]
mod strict_body_tests {
    use super::*;
    use axum::extract::FromRequest;
    async fn signup_body(body: &str) -> Value {
        let request = Request::builder()
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let Err(response) = Strict::<auth::Credentials>::from_request(request, &()).await else {
            panic!("{body} was accepted");
        };
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(response.extensions().get::<crate::response::DefaultError>().is_some());
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
    #[tokio::test]
    async fn a_body_of_the_wrong_shape_names_the_key_and_no_value() {
        let extra = signup_body(r#"{"email":"a@example.com","password":"long-enough-secret","name":"Ana"}"#).await;
        assert_eq!(extra["error"]["code"], "INVALID_INPUT");
        assert!(extra["error"]["fields"]["name"].is_string(), "{extra}");
        let text = extra.to_string();
        assert!(!text.contains("long-enough-secret") && !text.contains("a@example.com") && !text.contains("expected"), "{text}");
        let missing = signup_body(r#"{"email":"a@example.com"}"#).await;
        assert!(missing["error"]["fields"]["password"].is_string(), "{missing}");
        let wrong = signup_body(r#"{"email":"a@example.com","password":12345678901234}"#).await;
        assert!(!wrong.to_string().contains("12345678901234"), "{wrong}");
    }
}
/// URL parameters; a value of the wrong shape (a record id that is not a UUID) answers with what to send instead.
pub(crate) struct Path<T>(pub T);
impl<S: Send + Sync, T: serde::de::DeserializeOwned + Send> FromRequestParts<S> for Path<T> {
    type Rejection = Error;
    async fn from_request_parts(parts: &mut axum::http::request::Parts, state: &S) -> std::result::Result<Self, Error> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(Path(value)),
            Err(_) => Err(Error::new(StatusCode::BAD_REQUEST, "INVALID_ID", "The id in the URL is not a valid id; record ids are UUIDs, send the id the API returned")),
        }
    }
}
impl App {
    /// Admits a protected public write only with a token the provider confirmed.
    async fn require_captcha(&self, action: &str, h: &HeaderMap, client: Option<std::net::IpAddr>) -> Result<()> {
        if !self.captcha.protects(action) {
            return Ok(());
        }
        let token = h.get(crate::captcha::HEADER).and_then(|v| v.to_str().ok());
        match self.captcha.verify(token, client).await {
            crate::captcha::Verdict::Passed => Ok(()),
            crate::captcha::Verdict::Rejected => Err(Error::new(StatusCode::FORBIDDEN, "CAPTCHA_FAILED", "Complete the captcha again and send its new token in the X-Brain-Captcha header")),
            crate::captcha::Verdict::Unavailable => Err(Error::new(StatusCode::SERVICE_UNAVAILABLE, "CAPTCHA_UNAVAILABLE", "The captcha could not be verified; nothing was changed; retry with a new token shortly")),
        }
    }
    async fn user(&self, h: &HeaderMap) -> Result<Option<User>> {
        let user = self.identify(h).await?;
        if let Some(user) = &user {
            crate::audit::identified(user.id);
        }
        Ok(user)
    }
    async fn identify(&self, h: &HeaderMap) -> Result<Option<User>> {
        if let Some(user) = http_api_keys::key_user() {
            return Ok(Some(user));
        }
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
        db::timed(self.auth.user(auth::session_cookie(h)))
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
    /// A resource of the public surface: a server-only one (`api: false`) answers like an unknown name unless an
    /// API key with an explicit scope for it (see `http_api_keys::gate`) is acting.
    fn resource(&self, name: &str) -> Result<&Resource> {
        self.resources
            .get(name)
            .filter(|r| r.policy.api || http_api_keys::key_user().is_some())
            .ok_or_else(Error::missing)
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
    router_full(pool, config, crate::domain::Domain::default(), Default::default())
        .await
        .map(|(router, sockets, _)| (router, sockets))
}
/// Same router plus declared domain contracts (actions, transitions, batches, upserts and operations).
pub async fn router_with_domain(
    pool: PgPool,
    config: Config,
    domain: crate::domain::Domain,
) -> std::result::Result<(Router, SocketControl), String> {
    router_full(pool, config, domain, Default::default())
        .await
        .map(|(router, sockets, _)| (router, sockets))
}
/// Same as [`router_with_socket_control`] with application-declared platform settings (files, routes, limits).
pub async fn router_with_platform(
    pool: PgPool,
    config: Config,
    platform: crate::platform::Platform,
) -> std::result::Result<(Router, SocketControl), String> {
    router_with_control(pool, config, platform).await.map(|(router, sockets, _)| (router, sockets))
}
/// [`router_with_platform`] plus the [`EdgeControl`] that marks the instance as draining.
pub async fn router_with_control(
    pool: PgPool,
    config: Config,
    platform: crate::platform::Platform,
) -> std::result::Result<(Router, SocketControl, EdgeControl), String> {
    router_full(pool, config, crate::domain::Domain::default(), platform).await
}
/// Every router entry point: declared domain contracts plus platform settings.
pub async fn router_full(
    pool: PgPool,
    mut config: Config,
    domain: crate::domain::Domain,
    platform: crate::platform::Platform,
) -> std::result::Result<(Router, SocketControl, EdgeControl), String> {
    db::log_values(config.production);
    for origin in &platform.edge.cors_origins {
        if !config.origins.contains(origin) {
            config.origins.push(origin.clone());
        }
    }
    let (edge_state, edge_control) = edge_http::EdgeState::new(&platform.edge);
    let (rate_limit, rate_burst) = (platform.edge.requests_per_second, platform.edge.burst);
    let shared = platform.cluster.shared;
    let platform_organizations = platform.organizations.clone();
    let platform_captcha = platform.captcha.clone().ready(&platform.routes, config.production)?;
    let cache_policy = Arc::new(platform.cache.clone());
    let cors_max_age = platform.edge.cors_max_age;
    let (custom_routes, custom_handlers) = (platform.routes.clone(), platform.handlers.clone());
    let disabled_webhooks = Arc::new(platform.disabled_webhooks.clone());
    let socket_origins = config.sockets.as_ref().map(|s| s.allowed().to_vec()).unwrap_or_default();
    if let Some(origin) = socket_origins.iter().find(|origin| !config.origins.contains(origin)) {
        return Err(format!("B0745: Socket origins must be an explicit subset of backend HTTP origins: {} has {origin:?}, HTTP origins are {:?}; fix: add {origin:?} to {} or remove it from {}", crate::origins::SOCKET_KEY, config.origins, crate::origins::BRAIN_JSON_KEY, crate::origins::SOCKET_KEY));
    }
    let sockets_enabled = !socket_origins.is_empty();
    let mut origins = Vec::new();
    for origin in &config.origins {
        // Name the file that declares the value: a bad entry otherwise reads as a generic startup failure.
        let key = if platform.edge.cors_origins.contains(origin) { crate::origins::PLATFORM_KEY } else { crate::origins::BRAIN_JSON_KEY };
        if let Some(problem) = crate::origins::problem(origin) {
            let message = crate::origins::declaration_message(origin, key);
            if problem.fatal {
                return Err(format!("Backend origins must be explicit http(s) origins without credentials, paths or wildcards. {message}"));
            }
            // Accepted so far and harmless (it just never matches): reported, not fatal, for one minor release.
            crate::log_line!("{}", json!({"schemaVersion":1,"event":"origin-declaration-ignored","code":"B0744","message":message}));
        }
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
                    && !(crate::organizations::tenant(&r).is_some() && crate::organizations::grant(role).is_some())
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
    crate::organizations::validate_roles(&resources.values().collect::<Vec<_>>(), &platform.organizations, &config.roles.roles)?;
    contract::validate_graph(
        &resources.values().cloned().collect::<Vec<_>>(),
        &jobs.values().cloned().collect::<Vec<_>>(),
    )?;
    db::verify(&pool, &resources.values().cloned().collect::<Vec<_>>()).await?;
    domain_http::validate(&domain, &resources, &config.roles)?;
    let custom = Arc::new(routes_http::State::new(&custom_routes, custom_handlers, &config.roles.roles)?);
    let platform = Arc::new(files_http::State::new(
        &platform,
        &resources.values().cloned().collect::<Vec<_>>(),
        &config.roles.roles,
        &config.uploads,
    )?);
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
    let auth = Auth::new_with_role(pool.clone(), &config.roles.signup_role).await?.with_role_sessions(config.roles.session_seconds.clone());
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
    let public_origin = crate::links::origin(auth_base)
        .ok_or_else(|| crate::origins::auth_public_url_message(auth_base, None, &config.origins))?;
    if !config.origins.contains(&public_origin) {
        return Err(crate::origins::auth_public_url_message(auth_base, Some(&public_origin), &config.origins));
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
    let sockets = sockets::Broker::new(socket_origins);
    let sockets = Arc::new(if shared && sockets_enabled { sockets.with_fanout(pool.clone()) } else { sockets });
    sockets.listen();
    if shared {
        crate::cluster::spawn_sweeper(pool.clone());
    }
    let api_keys = config
        .api_keys
        .map(|cfg| crate::api_keys::Keys::new(pool.clone(), cfg).map(|keys| keys.shared(shared)))
        .transpose()?;
    let app = App {
        disabled_webhooks,
        organizations: Arc::new(platform_organizations),
        captcha: Arc::new(platform_captcha),
        cluster: crate::cluster::Cluster { shared },
        platform: platform.clone(),
        routes: custom.clone(),
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
        domain: Arc::new(domain),
        api_keys,
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
            header::HeaderName::from_static(crate::captcha::HEADER),
            header::HeaderName::from_static(crate::captcha::LOGIN_GRANT_HEADER),
            header::IF_MATCH,
            header::IF_NONE_MATCH,
        ]);
    let cors = cors.expose_headers([header::HeaderName::from_static("x-request-id"), header::ETAG]);
    let cors = match cors_max_age {
        Some(age) => cors.max_age(age),
        None => cors,
    };
    let governor = GovernorConfigBuilder::default()
        .key_extractor(edge_http::ClientKey)
        .per_millisecond(1000 / u64::from(rate_limit))
        .burst_size(rate_burst)
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
        .route("/health/live", get(edge_http::live))
        .route("/health/ready", get(edge_http::ready))
        .route("/_brain/api", get(docs))
        .route("/_brain/auth/signup", post(signup))
        .route("/_brain/auth/login", post(login))
        .route("/_brain/auth/logout", post(logout))
        .route("/_brain/auth/me", get(me))
        .route("/_brain/auth/sessions", get(list_sessions))
        .route("/_brain/auth/sessions/revoke-others", post(revoke_other_sessions))
        .route("/_brain/auth/sessions/{id}/revoke", post(revoke_session))
        .route("/_brain/auth/reauth", post(reauthenticate))
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
        .route("/_brain/auth/users/{id}/disable", post(disable_user))
        .route("/_brain/auth/users/{id}/enable", post(enable_user))
        .route("/_brain/auth/users/{id}/sessions/revoke", post(revoke_user_sessions))
        .route("/_brain/auth/users/{id}/identities/{provider}/disable", post(disable_user_identity))
        .route("/_brain/auth/users/{id}/identities/{provider}/enable", post(enable_user_identity))
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
        .route(
            "/_brain/analytics",
            post(analytics_ingest).layer(DefaultBodyLimit::max(crate::analytics::MAX_BODY)),
        )
        .route("/_brain/analytics.js", get(analytics_script))
        .route("/_brain/jobs/{queue}", post(enqueue))
        .route("/_brain/jobs/status/{id}", get(job_status))
        .merge(domain_http::routes())
        .route("/api/{name}", get(list).post(create))
        .route("/api/{name}/{id}", get(read).put(update).patch(patch).delete(delete))
        .route("/api/{name}/redirects/{slug}", get(slug_redirect))
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
        .merge(organizations_http::routes())
        .merge(credits_http::routes())
        .merge(http_api_keys::routes())
        .merge(files_http::routes(&platform))
        .merge(routes_http::routes(&custom))
        .fallback(|| async { Error::missing() })
        .layer(middleware::from_fn_with_state(cache_policy, cache_http::conditional))
        .layer(DefaultBodyLimit::max(65536))
        .layer(middleware::from_fn_with_state(app.clone(), http_api_keys::gate))
        .layer(middleware::from_fn_with_state(app.clone(), protect))
        .layer(GovernorLayer::new(governor))
        .layer(middleware::from_fn_with_state(edge_state, edge_http::edge))
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
        .layer(middleware::from_fn_with_state(custom.clone(), request_deadline))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(middleware::from_fn_with_state(app.clone(), origin_watch))
        .layer(middleware::from_fn(response_headers))
        .layer(middleware::from_fn_with_state(app.production, transport_security))
        .with_state(app);
    Ok((routes, SocketControl::new(sockets), edge_control))
}

/// Loopback binding is enforced by the CLI; the reusable library accepts the caller's owned listener.
pub async fn serve(
    listener: tokio::net::TcpListener,
    pool: PgPool,
    config: Config,
) -> std::result::Result<(), String> {
    serve_full(listener, pool.clone(), pool, config, crate::domain::Domain::default(), Default::default()).await
}
/// `serve` with declared domain contracts.
pub async fn serve_with_domain(
    listener: tokio::net::TcpListener,
    pool: PgPool,
    config: Config,
    domain: crate::domain::Domain,
) -> std::result::Result<(), String> {
    serve_full(listener, pool.clone(), pool, config, domain, Default::default()).await
}
/// [`serve`] with application-declared platform settings.
pub async fn serve_with_platform(
    listener: tokio::net::TcpListener,
    pool: PgPool,
    config: Config,
    platform: crate::platform::Platform,
) -> std::result::Result<(), String> {
    serve_full(listener, pool.clone(), pool, config, crate::domain::Domain::default(), platform).await
}
/// Serves HTTP from `pool` and runs the in-process webhook worker on its own `worker_pool` (bulkhead), so slow
/// deliveries cannot take the connections requests need.
pub async fn serve_pools(
    listener: tokio::net::TcpListener,
    pool: PgPool,
    worker_pool: PgPool,
    config: Config,
) -> std::result::Result<(), String> {
    serve_full(listener, pool, worker_pool, config, crate::domain::Domain::default(), Default::default()).await
}
/// `serve_pools` with declared domain contracts.
pub async fn serve_pools_with_domain(
    listener: tokio::net::TcpListener,
    pool: PgPool,
    worker_pool: PgPool,
    config: Config,
    domain: crate::domain::Domain,
) -> std::result::Result<(), String> {
    serve_full(listener, pool, worker_pool, config, domain, Default::default()).await
}
/// Every serve entry point: separate HTTP and worker pools, domain contracts and platform settings.
pub async fn serve_full(
    listener: tokio::net::TcpListener,
    pool: PgPool,
    worker_pool: PgPool,
    config: Config,
    domain: crate::domain::Domain,
    platform: crate::platform::Platform,
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
    let ready_delay = platform.edge.ready_delay;
    let drain_budget = platform.edge.drain;
    let file_storage = crate::files::Storage::new(&config.uploads);
    let (app, sockets, edge) = router_full(pool.clone(), config, domain, platform).await?;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut http_stop = stop_rx.clone();
    let worker_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut worker = worker.map(|worker| {
        let failed=worker_failed.clone();let stop=stop_tx.clone();
        tokio::spawn(async move {
            let result=worker.work(stop_rx).await;
            if result.is_err() {
                failed.store(true,std::sync::atomic::Ordering::Relaxed);
                crate::log_line!("{}",json!({"schemaVersion":1,"event":"webhook-worker-stopped","code":"WEBHOOK_WORKER_UNAVAILABLE","fix":"Inspect private database availability and restart the owned backend","valuesCaptured":false}));
                let _=stop.send(true);
            }
            result
        })
    });
    crate::log_line!(
        "{}",
        json!({"schemaVersion":1,"event":"backend-ready","address":address,"version":env!("CARGO_PKG_VERSION"),"experimental":true})
    );
    let file_sweep = tokio::spawn(crate::files::sweep_loop(pool.clone(), file_storage));
    let retention = crate::analytics::settings()
        .map(|cfg| tokio::spawn(crate::analytics::retention_loop(pool.clone(), cfg.retention_days)));
    let socket_stop = sockets.clone();
    let (signalled_tx, signalled_rx) = tokio::sync::oneshot::channel::<()>();
    let draining = edge.clone();
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        tokio::select! {_=wait_shutdown()=>{},_=http_stop.changed()=>{}}
        // Readiness fails first so load balancers stop routing here, then the listener closes.
        draining.drain();
        crate::log_line!("{}", json!({"schemaVersion":1,"event":"backend-draining","readyDelayMs":ready_delay.as_millis() as u64,"drainMs":drain_budget.as_millis() as u64}));
        let _ = signalled_tx.send(());
        tokio::time::sleep(ready_delay).await;
        socket_stop.shutdown();
    });
    let server = std::future::IntoFuture::into_future(server);
    tokio::pin!(server);
    let result = tokio::select! {
        result = &mut server => result.map_err(|_| "Backend HTTP server failed".into()),
        _ = async {
            if signalled_rx.await.is_ok() {
                tokio::time::sleep(ready_delay + drain_budget).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {
            crate::log_line!("{}", json!({"schemaVersion":1,"event":"drain-timeout","code":"DRAIN_TIMEOUT","inFlight":edge.in_flight(),"fix":"Raise platform.shutdown.drain or shorten slow requests"}));
            Ok(())
        }
    };
    file_sweep.abort();
    if let Some(retention) = retention {
        retention.abort();
    }
    sockets.shutdown();
    let sockets_closed = sockets.closed().await;
    if sockets_closed.is_err() {
        crate::log_line!("{}", json!({"schemaVersion":1,"event":"socket-shutdown-incomplete","code":"SOCKET_SHUTDOWN_TIMEOUT","valuesCaptured":false}));
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
async fn request_deadline(State(custom): State<Arc<routes_http::State>>, req: Request, next: Next) -> Response {
    if let Some(budget) = routes_http::matched(&req).and_then(|m| custom.deadline(m)) {
        return match tokio::time::timeout(budget, next.run(req)).await {
            Ok(response) => response,
            Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
        };
    }
    let path = req.uri().path();
    if path.starts_with("/_brain/files/") {
        let budget = files_http::deadline(&req);
        return match tokio::time::timeout(budget, next.run(req)).await {
            Ok(response) => response,
            Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
        };
    }
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
    let db_time = std::sync::Arc::new(db::DbTime::default());
    let response = db::with_db_time(db_time.clone(), REQUEST_ID.scope(rid.clone(), REQUEST_OPERATION.scope(operation_of(&method, &route, &path), crate::audit::scope(next.run(req))))).await;
    crate::dev_trace::request(&rid, method.as_str(), &route, response.status().as_u16(), started.elapsed().as_secs_f64() * 1000.0);
    let log_policy = response.extensions().get::<edge_http::LogPolicy>().copied();
    let mut response = crate::response::finalize(&method, &path, accept.as_deref(), &rid, response);
    let cacheable = response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED;
    let declared = response.extensions().get::<cache_http::Declared>().filter(|_| cacheable).map(|d| d.0.clone());
    let public_cache = cacheable
        && (declared.is_some() || response.extensions().get::<files_http::PublicCache>().is_some());
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
        // Public immutable files keep their declared cache policy; everything else stays no-store.
        if name == "cache-control" && public_cache {
            continue;
        }
        headers.insert(
            header::HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    if let Some(declared) = declared {
        headers.insert(header::CACHE_CONTROL, declared);
    }
    headers.insert("x-request-id", rid.parse().unwrap());
    // Lines carry the matched route template, never the raw path, query, headers or body.
    if let Some(event) = edge_http::request_event(log_policy.as_ref(), response.status(), started.elapsed()) {
        let mut line = json!({"event":event,"rid":rid,"route":route,"method":method.as_str(),"status":response.status().as_u16(),"ms":started.elapsed().as_millis()});
        // The template alone hides which resource was slow; only a name of the declared shape is written.
        if let Some(name) = path.strip_prefix("/api/").and_then(|rest| rest.split('/').next()).filter(|name| route.starts_with("/api/{name}") && !name.is_empty() && name.len() <= 48 && name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')) {
            line["resource"] = json!(name);
        }
        if let Some((db_ms, wait_ms)) = db_time.fields() {
            line["dbMs"] = json!(db_ms);
            line["dbWaitMs"] = json!(wait_ms);
        }
        crate::log_line!("{line}");
    }
    response
}
/// Production answers tell browsers to keep using HTTPS for this host and its subdomains.
async fn transport_security(State(production): State<bool>, req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    if production {
        response
            .headers_mut()
            .entry(header::STRICT_TRANSPORT_SECURITY)
            .or_insert(HeaderValue::from_static("max-age=31536000; includeSubDomains"));
    }
    response
}
/// The CORS layer leaves `Access-Control-Allow-Origin` off a refused origin, and the browser hides why, so this records
/// the reason for the operator. That layer still adds its credentials, expose, methods, headers and max-age values to
/// every answer; a refused origin must not receive any of them, so they are removed here. `Vary` stays for shared caches.
async fn origin_watch(State(app): State<App>, req: Request, next: Next) -> Response {
    let refused = req.headers().get(header::ORIGIN).is_some_and(|origin| !app.origins.iter().any(|allowed| allowed.as_bytes() == origin.as_bytes()));
    let reads = matches!(*req.method(), Method::GET | Method::HEAD);
    let preflight = *req.method() == Method::OPTIONS && req.headers().contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    if reads || preflight {
        if let Some(origin) = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            let socket = reads && req.uri().path() == "/_brain/socket";
            if !socket && !app.origins.iter().any(|allowed| allowed == origin) {
                crate::origins::log_refusal("B0740", if preflight { "preflight" } else { "read" }, req.method().as_str(), req.uri().path(), Some(origin), &app.origins);
            }
        }
    }
    let mut response = next.run(req).await;
    if refused {
        let cors: Vec<_> = response.headers().keys().filter(|name| name.as_str().starts_with("access-control-")).cloned().collect();
        for name in cors {
            response.headers_mut().remove(name);
        }
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
    // A presented API key on /api is a non-ambient credential; the key gate verifies or rejects it.
    let keyed = http_api_keys::keyed_resource_request(&req);
    let signed_route = routes_http::matched(&req).is_some_and(|m| app.routes.signed(req.method(), m));
    if !signed_inbound && !signed_route && !keyed && !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        let origin = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok());
        if !origin.is_some_and(|o| app.origins.iter().any(|allowed| allowed == o)) {
            crate::origins::log_refusal("B0740", "mutation", req.method().as_str(), req.uri().path(), origin, &app.origins);
            return Error::new(StatusCode::FORBIDDEN,"ORIGIN_DENIED","Send the configured site Origin; backend mutations require an explicit allowed origin. Run `brain explain B0740`; the server log names the seen and allowed origins")
                .origin_refused("B0740", origin, &app.origins, app.production)
                .into_response();
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
    // Outside production the description is a local tool: a request through a proxy already left this machine.
    if app.production || ["x-forwarded-for", "x-forwarded-host", "forwarded"].iter().any(|name| h.contains_key(*name)) {
        app.admin(&h).await?;
    }
    let resources = app.resources.values().cloned().collect::<Vec<_>>();
    let mut api = contract::openapi_with_roles(&resources, app.auth_flows.roles());
    crate::domain::openapi(&app.domain, &resources, &mut api);
    crate::openapi::extend(&mut api, &app.routes.declared(), &app.platform.buckets());
    if let Some(keys) = &app.api_keys {
        crate::api_keys::document(&mut api, keys.config().self_service);
    }
    Ok(Json(api))
}
async fn signup(State(app): State<App>, client: Option<axum::Extension<ClientIp>>, headers: HeaderMap, Strict(c): Strict<Credentials>) -> Result<Json<Value>> {
    let guarded = app.captcha.protects("signup");
    let operator = guarded && headers.contains_key(crate::captcha::OPERATOR_HEADER);
    if operator {
        if !app.captcha.operator(headers.get(crate::captcha::OPERATOR_HEADER).and_then(|v| v.to_str().ok())) {
            return Err(captcha_failed());
        }
    } else {
        app.require_captcha("signup", &headers, client.map(|c| c.0.0)).await?;
    }
    let user = app.auth
        .signup(c)
        .await
        .map_err(|failure| match failure {
            auth::AuthFailure::Input(_) => Error::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "SIGNUP_FAILED",
                "Use a valid email and a unique account with a password of 12..1024 bytes",
            ),
            auth::AuthFailure::Unavailable(_) => auth_unavailable("SIGNUP_UNAVAILABLE"),
        })?;
    if operator {
        sqlx::query("INSERT INTO _brain_auth_events(user_id,event,outcome) VALUES ($1,'operator-signup','success')").bind(user.id).execute(&app.pool).await.map_err(|error| Error::internal(crate::db::safe(error)))?;
    }
    // The account just passed a challenge, so the login that follows does not need its own. A failure here costs a second
    // challenge at most, never the account that already exists.
    if guarded && !operator && app.captcha.protects("login") {
        if let Ok(grant) = crate::sessions::signup_grant(&app.pool, user.id).await {
            return Ok(Json(json!({"user":user,"login_grant":grant})));
        }
    }
    Ok(Json(json!({"user":user})))
}
fn captcha_failed() -> Error {
    Error::new(StatusCode::FORBIDDEN, "CAPTCHA_FAILED", "Complete the captcha again and send its new token in the X-Brain-Captcha header")
}
/// A signup's grant replaces the challenge of its first login; a grant that does not fit is a failed challenge.
async fn login_gate(app: &App, headers: &HeaderMap, client: Option<std::net::IpAddr>, email: &str) -> Result<()> {
    if app.captcha.protects("login") {
        if let Some(grant) = headers.get(crate::captcha::LOGIN_GRANT_HEADER) {
            let spent = crate::sessions::consume_signup_grant(&app.pool, grant.to_str().unwrap_or(""), email).await.map_err(|_| auth_unavailable("LOGIN_UNAVAILABLE"))?;
            return if spent { Ok(()) } else { Err(captcha_failed()) };
        }
    }
    app.require_captcha("login", headers, client).await
}
fn auth_unavailable(code: &'static str) -> Error {
    Error::new(
        StatusCode::SERVICE_UNAVAILABLE,
        code,
        "Your details were not rejected: the database or the password workers are busy. Retry shortly",
    )
}
fn login_failed() -> Error {
    Error::new(
        StatusCode::UNAUTHORIZED,
        "LOGIN_FAILED",
        "Check credentials or wait for the 15-minute account lockout to expire",
    )
}
async fn login(State(app): State<App>, client: Option<axum::Extension<ClientIp>>, headers: HeaderMap, Strict(c): Strict<Credentials>) -> Result<Response> {
    login_gate(&app, &headers, client.as_ref().map(|c| c.0.0), &c.email).await?;
    // The backend owns the session mode. A configured JWT authority never falls back to opaque login on failure.
    if app.jwt.is_some() {
        return issue_token(app, client, c).await;
    }
    let Some((user, token)) = app.auth.login_from(c, client.map(|c| c.0.0)).await.map_err(|failure| match failure {
        auth::AuthFailure::Input(_) => login_failed(),
        auth::AuthFailure::Unavailable(_) => auth_unavailable("LOGIN_UNAVAILABLE"),
    })?
    else {
        return Err(login_failed());
    };
    let age = app.auth.session_length(&user.role);
    let mut response = Json(json!({"user":user})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        auth::cookie(&token, age).parse().unwrap(),
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
    // A grace answer carries no refresh token: the browser keeps the one the first answer set.
    let refresh = (!tokens.refresh_token.is_empty()).then(|| crate::jwt::cookie_header(
        crate::jwt::REFRESH_COOKIE,
        &tokens.refresh_token,
        tokens.refresh_age,
    ));
    let mut response = Json(tokens).into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, access.parse().unwrap());
    if let Some(refresh) = refresh {
        response
            .headers_mut()
            .append(header::SET_COOKIE, refresh.parse().unwrap());
    }
    response
}
async fn token_login(
    State(app): State<App>,
    client: Option<axum::Extension<ClientIp>>,
    headers: HeaderMap,
    Strict(credentials): Strict<Credentials>,
) -> Result<Response> {
    login_gate(&app, &headers, client.as_ref().map(|c| c.0.0), &credentials.email).await?;
    issue_token(app, client, credentials).await
}
async fn issue_token(app: App, client: Option<axum::Extension<ClientIp>>, credentials: Credentials) -> Result<Response> {
    let jwt = app.jwt.as_ref().ok_or_else(auth_disabled)?;
    let (user, opaque) = app
        .auth
        .login_from(credentials, client.map(|c| c.0.0))
        .await
        .map_err(|failure| match failure {
            auth::AuthFailure::Input(_) => auth_denied(),
            auth::AuthFailure::Unavailable(_) => auth_unavailable("LOGIN_UNAVAILABLE"),
        })?
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
    Strict(body): Strict<GoogleCredential>,
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
/// The caller's own session: only a session credential names one, so an API key never manages sessions.
async fn own_session(app: &App, h: &HeaderMap) -> Result<(User, uuid::Uuid)> {
    let user = app.required(h).await?;
    let session = if http_api_keys::key_user().is_some() {
        None
    } else if h.contains_key(header::AUTHORIZATION) || crate::jwt::cookie(h, crate::jwt::ACCESS_COOKIE).is_some() {
        app.jwt.as_ref().and_then(|jwt| jwt.session_id(crate::jwt::access(h)))
    } else {
        app.auth.session_id(auth::session_cookie(h), user.id).await.map_err(|error| Error::internal(error))?
    };
    let session = session.ok_or_else(|| Error::new(StatusCode::UNAUTHORIZED, "SIGN_IN_REQUIRED", "Sign in with a session; API keys do not manage sessions"))?;
    Ok((user, session))
}
impl App {
    /// A route's `reauth`: the session must have proved its password within `seconds`.
    pub(super) async fn require_recent_auth(&self, h: &HeaderMap, seconds: u32) -> Result<()> {
        let (user, session) = own_session(self, h).await?;
        let age = crate::sessions::auth_age(&self.pool, user.id, session).await.map_err(|error| Error::internal(error))?;
        if age.is_some_and(|age| age >= 0.0 && age <= f64::from(seconds)) {
            return Ok(());
        }
        Err(Error::new(StatusCode::UNAUTHORIZED, "REAUTH_REQUIRED", "Confirm your password with POST /_brain/auth/reauth, then retry"))
    }
}
async fn list_sessions(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>> {
    let (user, session) = own_session(&app, &h).await?;
    let sessions = crate::sessions::list(&app.pool, user.id, session).await.map_err(|error| Error::internal(error))?;
    Ok(Json(json!({"sessions":sessions})))
}
async fn revoke_session(State(app): State<App>, h: HeaderMap, Path(id): Path<uuid::Uuid>) -> Result<Json<Value>> {
    let (user, session) = own_session(&app, &h).await?;
    if id == session {
        return Err(Error::new(StatusCode::CONFLICT, "CURRENT_SESSION", "This is the session making the request; end it with POST /_brain/auth/logout"));
    }
    // The same answer for an unknown id and another account's: this route never confirms a session exists.
    if !crate::sessions::revoke_one(&app.pool, user.id, id).await.map_err(|error| Error::internal(error))? {
        return Err(Error::missing());
    }
    Ok(Json(json!({"ok":true})))
}
async fn revoke_other_sessions(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>> {
    let (user, session) = own_session(&app, &h).await?;
    let revoked = crate::sessions::revoke_others(&app.pool, user.id, session).await.map_err(|error| Error::internal(error))?;
    Ok(Json(json!({"ok":true,"revoked":revoked})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reauth {
    password: String,
}
async fn reauthenticate(State(app): State<App>, client: Option<axum::Extension<ClientIp>>, h: HeaderMap, Strict(body): Strict<Reauth>) -> Result<Json<Value>> {
    let (user, session) = own_session(&app, &h).await?;
    let proved = app.auth.reauthenticate(&user, session, body.password, client.map(|c| c.0.0)).await.map_err(|failure| match failure {
        auth::AuthFailure::Input(_) => login_failed(),
        auth::AuthFailure::Unavailable(_) => auth_unavailable("LOGIN_UNAVAILABLE"),
    })?;
    if !proved {
        return Err(login_failed());
    }
    Ok(Json(json!({"ok":true})))
}
async fn me(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(json!({"user":app.required(&h).await?})))
}
/// `If-Match` against the record as this writer would read it; the revision CAS in the write closes the race.
fn precondition(r: &Resource, h: &HeaderMap, existing: &Value, revision: &str, u: &User, through: bool) -> Result<()> {
    if !h.contains_key(header::IF_MATCH) {
        return Ok(());
    }
    let mut seen = existing.clone();
    crate::visibility::project(r, &mut seen, viewer(Some(u)), through);
    cache_http::if_match(h, &cache_http::record_etag(&seen, revision))
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
    let req = crate::query::parse(r, &app.resources, &pairs)
        .map_err(|problems| Error::validation(if app.production { crate::query::without_declared_names(problems) } else { problems }))?;
    let mut out = db::timed(crate::query::list(&app.pool, &app.resources, r, &req, viewer))
        .await
        .map_err(|error| match error {
            crate::query::ListError::Invalid(fields) => Error::validation(fields),
            crate::query::ListError::Denied(_) => Error::forbidden(),
            crate::query::ListError::SignInRequired => Error::new(StatusCode::UNAUTHORIZED, "SIGN_IN_REQUIRED", "Sign in, or send a project API key, to use owner=me"),
            crate::query::ListError::Database { error, at, branch } => {
                Error::database_in(&error, at, json!({"resource": name, "operation": if req.search.trim().is_empty() { "list" } else { "list.search" }, "branch": branch}))
            }
        })?;
    if let Some(items) = out["items"].as_array_mut() {
        include(&app, r, &req.include, items, viewer).await?;
        req.keep_fields(items);
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
    db::timed(crate::relations::include(&app.pool, &app.resources, r, names, rows, viewer))
        .await
        .map_err(|error| match error {
            crate::relations::IncludeError::Unknown(name) => Error::validation(json!({"include": format!("`{name}` is not includable")})),
            crate::relations::IncludeError::Denied(_) => Error::forbidden(),
            crate::relations::IncludeError::TooMany => Error::validation(json!({"include": format!("This include attaches more than {} records; lower limit or include fewer levels", crate::relations::MAX_INCLUDED_RECORDS)})),
            crate::relations::IncludeError::Database(error) => Error::internal(error),
        })
}
/// Whether the viewer owns `id` through a declared `owner.<ref>` grant of `operation`.
async fn owns_through(app: &App, r: &Resource, operation: &str, u: &User, id: Uuid) -> Result<bool> {
    crate::ownership::owns_through(&app.pool, r, &app.resources, operation, viewer(Some(u)), id)
        .await
        .map_err(|error| Error::database(error))
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
/// `GET /api/<resource>/redirects/<old slug>` → `{"slug": "<current>"}` for a permanent redirect, or 404.
async fn slug_redirect(
    State(app): State<App>,
    Path((name, slug)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    if slug.is_empty() || slug.len() > 512 {
        return Err(Error::missing());
    }
    let u = app.user(&h).await?;
    let scope = crate::ownership::read_scope(r, &app.resources, viewer(u.as_ref())).ok_or_else(Error::forbidden)?;
    match crate::slugs::redirect(&app.pool, &app.resources, r, &scope, &slug).await {
        Ok(Some(current)) => Ok(Json(json!({ "slug": current }))),
        Ok(None) => Err(Error::missing()),
        Err(error) => Err(Error::database(error)),
    }
}
async fn read(
    State(app): State<App>,
    Path((name, id)): Path<(String, Uuid)>,
    h: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Result<Response> {
    let r = app.resource(&name)?;
    let u = app.user(&h).await?;
    let mut names = Vec::new();
    for (key, value) in &pairs {
        if key != "include" || !names.is_empty() {
            return Err(Error::validation(json!({key: "Single-record reads accept only include"})));
        }
        names = value.split(',').map(str::to_string).collect();
    }
    // No read grant for this identity answers the same for every id, before anything looks the id up.
    if crate::ownership::scope(r, "read", viewer(u.as_ref())).is_none() {
        return Err(Error::forbidden());
    }
    let (mut row, revision) = db::timed(db::get_version(&app.pool, r, id))
        .await
        .map_err(|e| Error::database(e))?
        .ok_or_else(Error::missing)?;
    let allowed = crate::ownership::row_allowed(&app.pool, r, &app.resources, "read", viewer(u.as_ref()), id)
        .await
        .map_err(|e| Error::internal(db::safe(e)))?;
    if !allowed {
        // A hidden row answers exactly like one that does not exist: no existence oracle.
        return Err(Error::missing());
    }
    let owns = u.as_ref().is_some_and(|u| row["owner"].as_str() == Some(&u.id.to_string()));
    let general = r.policy.allows("read", u.as_ref().map(|u| u.role.as_str()), owns);
    let through = match &u {
        Some(u) if relational(r, "read") && (!general || r.fields.values().any(|f| f.visible.is_some())) => owns_through(&app, r, "read", u, id).await?,
        _ => false,
    };
    crate::visibility::project(r, &mut row, viewer(u.as_ref()), through);
    // A record without includes carries a strong revision validator usable in If-Match.
    let etag = names.is_empty().then(|| cache_http::record_etag(&row, &revision));
    include(&app, r, &names, std::slice::from_mut(&mut row), viewer(u.as_ref())).await?;
    let mut response = Json(row).into_response();
    if let Some(etag) = etag.and_then(|e| HeaderValue::from_str(&e).ok()) {
        response.headers_mut().insert(header::ETAG, etag);
    }
    Ok(response)
}
/// The last step of every write response: the projected row, or the row without its data when the writer may not read
/// its resulting state (conditioned reads and server-only resources; a write must not reveal what a read would hide).
pub(super) async fn reveal(app: &App, r: &Resource, record: &mut Value, u: &User, through: bool) -> Result<()> {
    let id = record["id"].as_str().and_then(|s| Uuid::parse_str(s).ok());
    let mut through = through;
    let mut visible = r.policy.api || http_api_keys::key_user().is_some();
    // Holding a write grant says nothing about reading: anything short of an unconditional read grant asks about this row.
    if visible && (r.policy.conditioned() || !r.policy.allows("read", Some(u.role.as_str()), false)) {
        let Some(id) = id else { return Err(Error::internal("record")) };
        let safe = |e| Error::internal(db::safe(e));
        visible = crate::ownership::row_allowed(&app.pool, r, &app.resources, "read", viewer(Some(u)), id).await.map_err(safe)?;
        through = through || owns_through(app, r, "read", u, id).await?;
    }
    if visible {
        crate::visibility::project(r, record, viewer(Some(u)), through);
    } else if let Some(row) = record.as_object_mut() {
        row.retain(|k, _| matches!(k.as_str(), "id" | "data" | "created_at" | "updated_at"));
        if row.contains_key("data") {
            row.insert("data".into(), json!({}));
        }
    }
    Ok(())
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
        None => Error::database(error),
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
    if crate::organizations::enforced(r) {
        let allowed = crate::ownership::row_allowed(&app.pool, r, &app.resources, operation, viewer(Some(u)), id).await.map_err(|e| Error::internal(db::safe(e)))?;
        if allowed {
            return Ok((None, false));
        }
        return Err(write_denied(&app.pool, &app.resources, r, u, id, Error::missing()).await);
    }
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
    Err(write_denied(&app.pool, &app.resources, r, u, id, Error::missing()).await)
}
/// The one answer to a write on an existing row the policy does not grant: `403 POLICY_DENIED` when the writer may
/// read the row (its existence is no secret and "not found" would mislead), otherwise `missing`, which the caller
/// builds byte-identical to its own answer for a row that does not exist.
pub(super) async fn write_denied(
    pool: &PgPool,
    resources: &BTreeMap<String, Resource>,
    r: &Resource,
    u: &User,
    id: Uuid,
    missing: Error,
) -> Error {
    match crate::ownership::row_allowed(pool, r, resources, "read", viewer(Some(u)), id).await {
        Ok(true) => Error::forbidden(),
        Ok(false) => missing,
        Err(error) => Error::internal(db::safe(error)),
    }
}
/// A relational writer must keep pointing the record at a parent they own.
async fn owns_data(app: &App, r: &Resource, operation: &str, u: &User, data: &Value) -> Result<bool> {
    crate::ownership::data_owned_through(&app.pool, r, &app.resources, operation, viewer(Some(u)), data)
        .await
        .map_err(|error| Error::database(error))
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
    let tenant = crate::organizations::enforced(r);
    let through = !tenant && !r.policy.allows("create", Some(&u.role), false);
    if through && !relational(r, "create") || tenant && crate::ownership::scope(r, "create", viewer(Some(&u))).is_none() {
        return Err(Error::forbidden());
    }
    crate::visibility::reject_server_fields(r, &input).map_err(Error::validation)?;
    let input = before(&app, r, "create", input, None, &u).await?;
    let data = r.input(&input).map_err(Error::validation)?;
    if through && !owns_data(&app, r, "create", &u, &data).await? {
        return Err(Error::forbidden());
    }
    // After the hook and validation: the organization that is written is the one checked.
    if tenant {
        let org = crate::organizations::org_of(r, &data).ok_or_else(Error::forbidden)?;
        if !crate::organizations::may_create(&app.pool, r, u.id, org).await.map_err(|error| Error::database(error))? {
            return Err(Error::forbidden());
        }
    }
    app.domain.protect(r, None, &data).map_err(Error::validation)?;
    let mut record = db::create(&app.pool, r, u.id, data)
        .await
        .map_err(|error| write_error(r, error))?;
    app.sockets.changed(&name, "create", &record);
    reveal(&app, r, &mut record, &u, through).await?;
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
    if !r.policy.allows("update", Some(&u.role), true) && !relational(r, "update") && !crate::organizations::enforced(r) {
        return Err(Error::forbidden());
    }
    // Capture a real row before application code; policy never relies on client-supplied existing data.
    let (existing, revision) = db::get_version(&app.pool, r, id)
        .await
        .map_err(|error| Error::database(error))?
        .ok_or_else(Error::missing)?;
    let (owner, through) = scoped_owner(&app, r, "update", &u, &existing, id).await?;
    precondition(r, &h, &existing, &revision, &u, through)?;
    crate::visibility::reject_server_fields(r, &input).map_err(Error::validation)?;
    let mut input = input;
    crate::visibility::keep_stored(r, &mut input, &existing["data"]);
    let previous = existing["data"].clone();
    let input = before(&app, r, "update", input, Some(existing), &u).await?;
    let data = r.input(&input).map_err(Error::validation)?;
    crate::organizations::unchanged(r, &previous, &data).map_err(Error::validation)?;
    if through && !owns_data(&app, r, "update", &u, &data).await? {
        return Err(Error::forbidden());
    }
    app.domain.protect(r, Some(&previous), &data).map_err(Error::validation)?;
    let mut record = db::update_checked(&app.pool, r, id, owner, data, Some(&revision))
        .await
        .map_err(|error| write_error(r, error))?
        .ok_or_else(conflict)?;
    app.sockets.changed(&name, "update", &record);
    reveal(&app, r, &mut record, &u, through).await?;
    Ok(Json(record))
}
/// Partial update: only sent fields change; the merged record is revalidated and written with the same CAS.
async fn patch(
    State(app): State<App>,
    Path((name, id)): Path<(String, Uuid)>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    let u = app.required(&h).await?;
    if !r.policy.allows("update", Some(&u.role), true) && !relational(r, "update") && !crate::organizations::enforced(r) {
        return Err(Error::forbidden());
    }
    let (existing, revision) = db::get_version(&app.pool, r, id)
        .await
        .map_err(|error| Error::database(error))?
        .ok_or_else(Error::missing)?;
    let (owner, through) = scoped_owner(&app, r, "update", &u, &existing, id).await?;
    precondition(r, &h, &existing, &revision, &u, through)?;
    crate::visibility::reject_server_fields(r, &input).map_err(Error::validation)?;
    let mut merged = crate::domain::merge_patch(r, &existing["data"], &input).map_err(Error::validation)?;
    crate::visibility::keep_stored(r, &mut merged, &existing["data"]);
    let previous = existing["data"].clone();
    let merged = before(&app, r, "update", merged, Some(existing), &u).await?;
    let data = r.input(&merged).map_err(Error::validation)?;
    crate::organizations::unchanged(r, &previous, &data).map_err(Error::validation)?;
    if through && !owns_data(&app, r, "update", &u, &data).await? {
        return Err(Error::forbidden());
    }
    app.domain.protect(r, Some(&previous), &data).map_err(Error::validation)?;
    let mut record = db::update_checked(&app.pool, r, id, owner, data, Some(&revision))
        .await
        .map_err(|error| write_error(r, error))?
        .ok_or_else(conflict)?;
    app.sockets.changed(&name, "update", &record);
    reveal(&app, r, &mut record, &u, through).await?;
    Ok(Json(record))
}
async fn delete(
    State(app): State<App>,
    Path((name, id)): Path<(String, Uuid)>,
    h: HeaderMap,
) -> Result<Json<Value>> {
    let r = app.resource(&name)?;
    let u = app.required(&h).await?;
    if !r.policy.allows("delete", Some(&u.role), true) && !relational(r, "delete") && !crate::organizations::enforced(r) {
        return Err(Error::forbidden());
    }
    let (existing, revision) = db::get_version(&app.pool, r, id)
        .await
        .map_err(|error| Error::database(error))?
        .ok_or_else(Error::missing)?;
    let (owner, through) = scoped_owner(&app, r, "delete", &u, &existing, id).await?;
    precondition(r, &h, &existing, &revision, &u, through)?;
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
            Error::database(error)
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
    Strict(input): Strict<Enqueue>,
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
    Strict(meta): Strict<MediaMetadata>,
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
    Strict(body): Strict<MediaPresets>,
) -> Result<Json<Value>> {
    let user = app.required(&headers).await?;
    crate::media::validate_presets(&body.presets).map_err(|_|Error::validation(json!({"presets":"Use at most six uniquely named PNG/JPEG/WebP/AVIF presets with dimensions 1..4096 (AVIF 1..2048 and always with a PNG, JPEG or WebP fallback); JPEG and AVIF quality 1..100"})))?;
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

// First-party behavior analytics (docs/backend/analytics.md). Accepts `text/plain` beacons; answers 204 even when a
// privacy signal or bot heuristic drops the batch, so the browser learns nothing about filtering.
async fn analytics_ingest(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Result<Response> {
    use crate::analytics::{self, IngestError, Outcome};
    let Some(cfg) = analytics::settings() else { return Err(Error::missing()) };
    if body.len() > analytics::MAX_BODY {
        return Err(Error::new(StatusCode::PAYLOAD_TOO_LARGE, "ANALYTICS_TOO_LARGE", "Send at most 32 KiB per analytics batch"));
    }
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let mut signals = analytics::Signals {
        user_agent: text("user-agent"),
        do_not_track: text("dnt").as_deref() == Some("1"),
        global_privacy_control: text("sec-gpc").as_deref() == Some("1"),
        country: cfg.country_header.as_deref().and_then(text).map(|c| c.trim().to_ascii_uppercase()),
        user: None,
        origins: app.origins.to_vec(),
    };
    if signals.do_not_track || signals.global_privacy_control || analytics::is_bot(signals.user_agent.as_deref()) {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let batch: analytics::Batch = serde_json::from_slice(&body).map_err(|_| {
        Error::new(StatusCode::BAD_REQUEST, "ANALYTICS_INVALID", "Send the collector batch shape {v,vid,sid,ctx?,ev[]}")
    })?;
    signals.user = app.user(&headers).await.ok().flatten().map(|u| u.id);
    match analytics::ingest(&app.pool, analytics::limiter(), batch, &signals).await {
        Ok(Outcome::Stored(_) | Outcome::Ignored(_)) => Ok(StatusCode::NO_CONTENT.into_response()),
        Err(IngestError::Invalid(fix)) => Err(Error::new(StatusCode::BAD_REQUEST, "ANALYTICS_INVALID", fix)),
        Err(IngestError::RateLimited) => Err(Error::new(StatusCode::TOO_MANY_REQUESTS, "ANALYTICS_RATE_LIMITED", "Batch and throttle events; the per-visitor minute budget is exhausted")),
        Err(IngestError::SessionConflict) => Err(Error::new(StatusCode::CONFLICT, "ANALYTICS_SESSION_CONFLICT", "Start a new random session identifier for this visitor")),
        Err(IngestError::Database(e)) => Err(Error::internal(e)),
    }
}

async fn analytics_script() -> Response {
    let headers = [
        (axum::http::header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
        (axum::http::header::CACHE_CONTROL, "public, max-age=300"),
    ];
    (headers, crate::analytics::collector_js(crate::analytics::settings())).into_response()
}

// Native owner-managed links; no destination, credential or query content enters request logs.
async fn create_short_link(
    State(app): State<App>,
    headers: HeaderMap,
    Strict(input): Strict<crate::links::Create>,
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
    Strict(input): Strict<SlugRequest>,
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
    client: Option<axum::Extension<ClientIp>>,
    headers: HeaderMap,
    Strict(input): Strict<ResetRequest>,
) -> Result<Response> {
    app.require_captcha("password_request", &headers, client.as_ref().map(|c| c.0.0)).await?;
    app.auth_flows
        .request_reset_from(&input.email, client.map(|c| c.0.0))
        .await
        .map_err(account_flow_error)?;
    Ok((StatusCode::ACCEPTED, Json(json!({"accepted":true}))).into_response())
}
async fn complete_password_reset(
    State(app): State<App>,
    Strict(input): Strict<ResetConfirmation>,
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
    Strict(_input): Strict<EmptyAccountRequest>,
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
    Strict(input): Strict<EmailConfirmation>,
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
    Strict(input): Strict<RoleAssignment>,
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

async fn administrator(app: &App, headers: &HeaderMap) -> Result<()> {
    let user = app.user(headers).await?.ok_or_else(auth_denied)?;
    if user.role != "admin" {
        return Err(Error::forbidden());
    }
    Ok(())
}
fn operator_result(done: std::result::Result<bool, crate::auth_flows::Error>) -> Result<Response> {
    if !done.map_err(account_flow_error)? {
        return Err(Error::new(StatusCode::CONFLICT, "OPERATION_REFUSED", "The target does not exist, or it is the last enabled administrator"));
    }
    Ok(Json(json!({"ok":true})).into_response())
}
async fn disable_user(State(app): State<App>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<Response> {
    administrator(&app, &headers).await?;
    operator_result(crate::auth_flows::set_user_disabled(&app.pool, id, true).await)
}
async fn enable_user(State(app): State<App>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<Response> {
    administrator(&app, &headers).await?;
    operator_result(crate::auth_flows::set_user_disabled(&app.pool, id, false).await)
}
async fn revoke_user_sessions(State(app): State<App>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<Response> {
    administrator(&app, &headers).await?;
    operator_result(crate::auth_flows::revoke_user_sessions(&app.pool, id).await)
}
async fn disable_user_identity(State(app): State<App>, headers: HeaderMap, Path((id, provider)): Path<(Uuid, String)>) -> Result<Response> {
    administrator(&app, &headers).await?;
    operator_result(crate::auth_flows::set_identity_disabled(&app.pool, id, &provider, true).await)
}
async fn enable_user_identity(State(app): State<App>, headers: HeaderMap, Path((id, provider)): Path<(Uuid, String)>) -> Result<Response> {
    administrator(&app, &headers).await?;
    operator_result(crate::auth_flows::set_identity_disabled(&app.pool, id, &provider, false).await)
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
    // A sender retries a 503, and would give up on a 404 while the key is being set.
    if app.disabled_webhooks.contains(&name) {
        let mut response = Error::new(StatusCode::SERVICE_UNAVAILABLE, "WEBHOOK_DISABLED", "This webhook's signing key is not configured on the server; retry after the Retry-After delay").into_response();
        response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(300u64));
        return Ok(response);
    }
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
    #[test]
    fn stripped_stacks_are_not_logged_as_if_they_helped() {
        assert!(stack_is_symbolized("0: brain_backend::http::list\n"));
        assert!(!stack_is_symbolized("0: <unknown>\n1: __rust_try\n"));
        assert!(!stack_is_symbolized("0: __mh_execute_header\n1: __mh_execute_header\n"));
        assert!(!stack_is_symbolized("0: __mh_execute_header\n  at crate brain_backend\n"));
    }
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

#[cfg(test)]
mod stack_note_tests {
    #[test]
    fn the_note_about_a_binary_without_symbols_is_kept_once_per_process() {
        assert!(super::first_unsymbolized_stack());
        assert!(!super::first_unsymbolized_stack());
    }
}
