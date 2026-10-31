//! Experimental backend. The compiler owns source syntax; this crate consumes only validated resource contracts.
pub mod auth;
pub mod auth_flows;
pub mod contract;
pub mod db;
pub mod google;
pub mod http;
pub mod jobs;
pub mod jwt;
pub mod links;
pub mod media;
pub mod response;
pub mod search;
pub mod types;
pub mod upload;
pub mod files;
pub mod platform;
pub mod webhooks;

pub use contract::{Check, Constraints, Field, Policy, Resource};
pub use http::{Config, serve};

pub mod before_hooks;
