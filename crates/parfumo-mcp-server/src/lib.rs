//! parfumo-mcp-server: the remote, multi-user side of the Parfumo MCP server. Streamable
//! HTTP on `/mcp`, OAuth 2.1 and static bearer tokens, the `/link` page, the daily canary.
//! The tools themselves live in `parfumo-mcp-core`.

pub mod canary;
pub mod config;
pub mod enroll;
pub mod i18n;
pub mod oauth;
pub mod server;
pub mod web;

#[cfg(test)]
mod hardening_tests;
#[cfg(test)]
mod tests;
