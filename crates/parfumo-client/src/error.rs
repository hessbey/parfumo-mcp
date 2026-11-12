//! Normalised client errors. Messages never contain passwords, cookies or tokens.

use thiserror::Error;

/// Errors returned by [`crate::ParfumoClient`].
///
/// Invariant: no variant ever carries a credential, a cookie value, a `csrf_key`
/// or an `h` token. Only paths, HTTP statuses and structural hints are included.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// The perfume, brand, member or page does not exist (HTTP 404 or 404 page).
    #[error("not found: {0}")]
    NotFound(String),
    /// Parfumo (or Cloudflare) asked us to slow down and retries are exhausted.
    #[error("rate limited by Parfumo{}", .retry_after_secs.map(|s| format!(" (retry after {s} s)")).unwrap_or_default())]
    RateLimited {
        /// Seconds to wait, when the site sent `Retry-After`.
        retry_after_secs: Option<u64>,
    },
    /// A browser-bound human verification (Cloudflare challenge / Turnstile) was presented.
    #[error("Parfumo requires a human verification (captcha / Cloudflare challenge)")]
    CaptchaRequired,
    /// The page structure changed: selectors must be updated.
    #[error("parser outdated: {0}")]
    ParserOutdated(String),
    /// Parfumo is unavailable or answered unexpectedly.
    #[error("upstream error: {0}")]
    Upstream(String),
    /// The login form rejected the username / password.
    #[error("invalid Parfumo username or password")]
    InvalidCredentials,
    /// The route needs a logged-in Parfumo session.
    #[error("this Parfumo route requires a logged-in session")]
    LoginRequired,
    /// The caller passed an unusable argument.
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
