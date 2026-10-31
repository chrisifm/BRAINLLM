//! One integration executable preserves each isolated fixture and avoids repeated platform loader startup.
#[path = "api_keys.rs"]
mod api_keys;
#[path = "auth_flows.rs"]
mod auth_flows;
#[path = "auth_http.rs"]
mod auth_http;
#[path = "backend.rs"]
mod backend;
#[path = "exact_types.rs"]
mod exact_types;
#[path = "foundation.rs"]
mod foundation;
#[path = "jwt_sessions.rs"]
mod jwt_sessions;
#[path = "links.rs"]
mod links;
#[path = "media.rs"]
mod media;
#[path = "native_openapi.rs"]
mod native_openapi;
#[path = "native_sockets.rs"]
mod native_sockets;
