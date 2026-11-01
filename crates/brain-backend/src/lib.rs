//! Experimental backend. The compiler owns source syntax; this crate consumes only validated resource contracts.
/// One log line on stderr in one write. The standard print macros write a formatted value piece by piece, and processes sharing the pipe
/// (`brain start`, workers) interleave those pieces, which breaks every line parser.
#[macro_export]
macro_rules! log_line {
    ($($arg:tt)*) => { $crate::db::write_line(&format!($($arg)*)) };
}

pub mod analytics;
pub mod api_keys;
pub mod audit;
pub mod auth;
pub mod captcha;
pub mod auth_flows;
pub mod boot;
pub mod capabilities;
pub mod cluster;
pub mod contract;
pub mod contract_review;
pub mod credits;
pub mod db;
pub mod dev_trace;
pub mod domain;
pub mod schedules;
pub mod google;
pub mod http;
pub mod jobs;
pub mod jwt;
pub mod links;
pub mod query;
pub mod relations;
pub mod media;
pub mod organizations;
pub mod origins;
pub mod ownership;
pub mod response;
pub mod openapi;
pub mod routes;
pub mod search;
pub mod search_config;
pub mod slugs;
pub mod types;
pub mod upload;
pub mod files;
pub mod import;
pub mod platform;
pub mod maintenance;
pub mod visibility;
pub mod webhooks;

pub use contract::{Check, Constraints, Field, Policy, Resource};
pub use http::{Config, serve};

pub mod before_hooks;
