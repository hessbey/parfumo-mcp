//! HTTP client for parfumo.com: transport, routes, HTML parsing and models.
//! Independent of MCP. See `docs/DISCOVERY.md` for the measured behaviour of the site.

mod account;
mod app;
mod app_brand;
mod app_db;
mod app_members;
mod app_note;
mod app_perfumer;
mod app_rankings;
mod app_similar;
mod client;
mod error;
mod forum;
mod market;
pub mod models;
pub mod parse;
pub mod site;
mod social;
mod transport;

pub use account::{calls, find_list};
pub use client::ParfumoClient;
pub use client::{LISTING_TTL, RECORD_TTL};
pub use error::ClientError;
pub use models::*;
pub use social::social_calls;
#[cfg(feature = "explore")]
pub use transport::ExploreResponse;
pub use transport::{ClientConfig, GlobalLimiter, PageCache, StoredCookie, UserLimiter, tls_config};
