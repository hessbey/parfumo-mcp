//! HTTP layer shared by `/mcp`, OAuth and the enrolment pages: state, bearer
//! authentication, local user login with throttling, HTML helpers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Response},
};

use parfumo_mcp_core::{
    crypto,
    db::{Db, User},
    pages,
    state::{AppState, AuthUser},
};

use crate::{config::ServerConfig, i18n::Lang};

/// State of the HTTP server.
#[derive(Clone)]
pub struct ServerState {
    /// What the tools work with: their settings, the database, the Parfumo sessions.
    pub core: AppState,
    /// Settings of the HTTP side.
    pub http: Arc<ServerConfig>,
}

pub const LOGIN_WINDOW_SECS: i64 = 15 * 60;
/// Wrong access keys allowed from one client address in that window.
pub const LOGIN_MAX_FAILURES_PER_IP: i64 = 20;

/// At most `max` events per `per`, all callers together (in memory).
pub struct Window {
    max: usize,
    per: std::time::Duration,
    hits: std::sync::Mutex<std::collections::VecDeque<std::time::Instant>>,
}

impl Window {
    pub const fn new(max: usize, per: std::time::Duration) -> Self {
        Self { max, per, hits: std::sync::Mutex::new(std::collections::VecDeque::new()) }
    }

    /// Counts one event, unless the window is full.
    pub fn allow(&self) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|p| p.into_inner());
        let now = std::time::Instant::now();
        while hits.front().is_some_and(|t| now.duration_since(*t) >= self.per) {
            hits.pop_front();
        }
        if hits.len() >= self.max {
            return false;
        }
        hits.push_back(now);
        true
    }
}

/// Address of the client as the reverse proxy reports it: the last entry of
/// `X-Forwarded-For`, the one the proxy itself appended (Caddy discards what the client
/// sent). `None` without a proxy: the per-name lock still applies.
pub fn client_ip(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("x-forwarded-for")?.to_str().ok()?;
    value.rsplit(',').next()?.trim().parse::<std::net::IpAddr>().ok().map(|ip| ip.to_string())
}

pub async fn healthz(State(st): State<ServerState>) -> Response {
    if st.core.db.ping() {
        // The canary verdict is shown but never fails the health check: a container must
        // not be restarted because Parfumo changed a page.
        let canary = st.core.db.canary_last().ok().flatten().map(|run| serde_json::json!({"status": run.status, "at": run.ts}));
        Json(serde_json::json!({"status": "ok", "version": env!("CARGO_PKG_VERSION"), "canary": canary})).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"status": "database_unavailable"}))).into_response()
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token.trim()).filter(|t| !t.is_empty())
}

/// No anonymous request reaches `/mcp`. A 401 carries the protected-resource metadata URL
/// (RFC 9728) so that OAuth-capable clients can start the authorization flow.
pub async fn require_bearer(State(st): State<ServerState>, mut req: Request, next: Next) -> Response {
    let user = match bearer_token(req.headers()) {
        Some(token) => st.core.db.user_by_bearer_hash(&crypto::token_hash(token)).ok().flatten(),
        None => None,
    };
    match user {
        Some(user) => {
            req.extensions_mut().insert(AuthUser(user));
            next.run(req).await
        }
        None => {
            let had_token = bearer_token(req.headers()).is_some();
            let mut challenge = format!(
                "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\", scope=\"{}\"",
                st.http.public_url,
                crate::oauth::SCOPE
            );
            if had_token {
                challenge.push_str(", error=\"invalid_token\"");
            }
            let mut resp = (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "unauthorized"}))).into_response();
            if let Ok(v) = HeaderValue::from_str(&challenge) {
                resp.headers_mut().insert(header::WWW_AUTHENTICATE, v);
            }
            resp
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoginError {
    Invalid,
    Locked,
}

/// The user an access key belongs to. The key is the user's static bearer token: 256 random
/// bits, which cannot be guessed, so the only limit is on the load a client address may
/// cause: twenty wrong keys in fifteen minutes lock that address for the rest of the window.
pub fn verify_key(db: &Db, key: &str, ip: Option<&str>) -> Result<User, LoginError> {
    let ip_subject = ip.map(|ip| format!("ip:{ip}"));
    if ip_subject
        .as_deref()
        .is_some_and(|s| db.recent_login_failures(s, LOGIN_WINDOW_SECS).unwrap_or(LOGIN_MAX_FAILURES_PER_IP) >= LOGIN_MAX_FAILURES_PER_IP)
    {
        return Err(LoginError::Locked);
    }
    match db.user_by_key_hash(&crypto::token_hash(key.trim())).ok().flatten() {
        Some(user) => Ok(user),
        None => {
            if let Some(s) = &ip_subject {
                let _ = db.record_login_failure(s);
            }
            Err(LoginError::Invalid)
        }
    }
}

pub fn wrong_key(lang: Lang) -> &'static str {
    lang.pick("This access key is not valid.", "Cette clé d'accès n'est pas valide.", "Dieser Zugangsschlüssel ist ungültig.")
}

pub fn too_many_failures(lang: Lang) -> &'static str {
    lang.pick(
        "Too many failures: try again in fifteen minutes.",
        "Trop d'échecs : réessayez dans quinze minutes.",
        "Zu viele Fehlversuche: Versuche es in fünfzehn Minuten erneut.",
    )
}

/// CSRF defence for the HTML forms: a present `Origin` header must be this server.
///
/// A browser sends `Origin: null` with a form when the page forbids the referrer
/// (`Referrer-Policy: no-referrer`): the pages therefore keep it for same-origin requests.
pub fn origin_ok(cfg: &ServerConfig, headers: &HeaderMap) -> bool {
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(o) => o.trim_end_matches('/').eq_ignore_ascii_case(&cfg.public_url),
    }
}

/// What the forms answer when `origin_ok` refuses a request: the address to use instead.
pub fn origin_refused(cfg: &ServerConfig, lang: Lang, path: &str) -> String {
    pages::origin_refused(lang, &format!("{}{path}", cfg.public_url))
}

pub use parfumo_mcp_core::pages::{esc, lang_switcher};

/// A page of this server: the language links, then one card holding the title and `body`.
/// A paragraph of class `foot` in `body` is shown under the card.
pub fn page(status: StatusCode, lang: Lang, title: &str, switcher: &str, body: &str) -> Response {
    let mut resp = (status, Html(pages::html(lang, title, switcher, body))).into_response();
    for (name, value) in pages::HEADERS {
        resp.headers_mut().insert(name, HeaderValue::from_static(value));
    }
    resp
}
