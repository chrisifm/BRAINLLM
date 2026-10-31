//! Application-declared platform settings for the native backend (files, routes, limits, proxy, CORS, logging).
//!
//! Passed to [`crate::http::router_with_platform`] / [`crate::http::serve_with_platform`] so existing `Config`
//! literals stay unchanged. `Platform::default()` reproduces the previous fixed behavior.
use crate::files::Bucket;

#[derive(Clone, Default)]
pub struct Platform {
    /// Declared file buckets (`server/files/<name>.jsbrain`).
    pub buckets: Vec<Bucket>,
}
