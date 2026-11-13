//! Language of the two pages a person sees (`/link` and the OAuth login): English, French
//! or German. Chosen by `?lang=`, else by the browser's `Accept-Language`, else English.
//! Tool descriptions, errors and logs stay in English.

use axum::http::{HeaderMap, header};

pub use parfumo_mcp_core::pages::Lang;

/// The language asked for explicitly, else the first supported one the browser prefers.
pub fn choose(asked: Option<&str>, headers: &HeaderMap) -> Lang {
    Lang::choose(asked, headers.get(header::ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()))
}
