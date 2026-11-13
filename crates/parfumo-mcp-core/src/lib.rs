//! parfumo-mcp-core: the 48 Parfumo tools and everything they need (per-user Parfumo
//! sessions, storage, audit log, public-write guard rails). Transport-independent.
//!
//! Two binaries are built on this library: `parfumo-mcp` (stdio, one local account) and
//! `parfumo-mcp-server` (Streamable HTTP, several users). They share the form that links a
//! Parfumo account ([`pages`]).

pub mod account_tools;
pub mod audit;
pub mod caller;
pub mod community_tools;
pub mod config;
pub mod crypto;
pub mod database_tools;
pub mod db;
pub mod errors;
pub mod feedback_tools;
pub mod pages;
pub mod paging;
pub mod profile_tools;
pub mod public_gate;
pub mod sessions;
pub mod social_tools;
pub mod state;
pub mod tools;

#[cfg(any(test, feature = "test-util"))]
pub mod testing;

#[cfg(test)]
mod account_tests;
#[cfg(test)]
mod app_session_tests;
#[cfg(test)]
mod feedback_tests;
#[cfg(test)]
mod social_tests;
