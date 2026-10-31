//! Experimental backend. The compiler owns source syntax; this crate consumes only validated resource contracts.
pub mod analytics;
pub mod api_keys;
pub mod auth;
pub mod auth_flows;
pub mod boot;
pub mod contract;
pub mod db;
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
pub mod platform;
pub mod visibility;
pub mod webhooks;

pub use contract::{Check, Constraints, Field, Policy, Resource};
pub use http::{Config, serve};

pub mod before_hooks;
