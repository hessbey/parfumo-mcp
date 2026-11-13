//! The application interface of Parfumo (`/app/v1`): JSON calls authenticated by the
//! session token of the member. Measured behaviour and every call: `docs/APP_API.md`.
//!
//! Two rules of that document are enforced here, for every caller:
//! - only the member's own token is ever sent, obtained with their own e-mail and password;
//! - a refused token is never sent again: the site blocks an address after a few requests
//!   without a valid token, so a 401 forgets the token and no call leaves without one.

use reqwest::Method;
use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    transport::{StoredCookie, Transport},
};

/// Path prefix of every call.
pub(crate) const PREFIX: &str = "/app/v1";

/// The session token is kept in the cookie store, so that it is saved, restored and erased
/// with the web session. Its domain is one no request goes to: it is never sent as a cookie.
const TOKEN_NAME: &str = "app_token";
const TOKEN_DOMAIN: &str = "app-token.invalid";

/// Seconds the site asks to wait when it blocks an address (measured: five minutes).
const BLOCK_SECS: u64 = 300;

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// The text a JSON error answer carries, when it has one.
fn error_text(json: &Value) -> Option<String> {
    ["message", "error"].iter().find_map(|k| json.get(k).and_then(Value::as_str)).map(str::to_string)
}

impl Transport {
    pub(crate) fn app_token(&self) -> Option<String> {
        self.jar.value(TOKEN_NAME, TOKEN_DOMAIN)
    }

    pub(crate) fn set_app_token(&self, token: Option<&str>) {
        match token {
            Some(token) => {
                self.jar.upsert(StoredCookie { name: TOKEN_NAME.into(), value: token.to_string(), domain: TOKEN_DOMAIN.into(), path: "/".into() })
            }
            None => self.jar.forget(TOKEN_NAME, TOKEN_DOMAIN),
        }
    }

    /// One call: `params` go in the query of a `GET` and in the body of a `POST`.
    /// A `POST` is never repeated; a `GET` is, on 429, 5xx and network errors only.
    async fn app_send(&self, method: Method, path: &str, params: &[(String, String)], token: Option<&str>) -> Result<(u16, String), ClientError> {
        let (target, form, retries) = if method == Method::GET {
            let query = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(params).finish();
            let target = if query.is_empty() { format!("{PREFIX}{path}") } else { format!("{PREFIX}{path}?{query}") };
            (target, None, self.cfg.max_retries)
        } else {
            (format!("{PREFIX}{path}"), Some(params), 0)
        };
        let raw = self.send_with(method, &target, form, retries, token).await?;
        // The block page of the site: HTML, whatever the status.
        if raw.body.contains("Access Denied") && raw.body.contains("temporarily blocked") {
            return Err(ClientError::RateLimited { retry_after_secs: Some(BLOCK_SECS) });
        }
        Ok((raw.status, raw.body))
    }

    /// A call made as the member. Without a token nothing is sent; a refused token is
    /// forgotten, so that the next calls do not repeat the refusal.
    pub(crate) async fn app_call(&self, method: Method, path: &str, params: &[(String, String)]) -> Result<Value, ClientError> {
        let Some(token) = self.app_token() else { return Err(ClientError::LoginRequired) };
        let (status, body) = self.app_send(method, path, params, Some(&token)).await?;
        if status == 401 {
            self.set_app_token(None);
            return Err(ClientError::LoginRequired);
        }
        let json: Value = serde_json::from_str(&body).map_err(|_| match status {
            404 | 410 => ClientError::NotFound(path.to_string()),
            200..=299 => ClientError::ParserOutdated(format!("{PREFIX}{path} did not answer in JSON")),
            _ => ClientError::Upstream(format!("HTTP {status} on {PREFIX}{path}")),
        })?;
        match status {
            200..=299 => Ok(json),
            404 | 410 => Err(ClientError::NotFound(path.to_string())),
            400 | 422 => Err(ClientError::InvalidInput(error_text(&json).unwrap_or_else(|| format!("refused by {PREFIX}{path}")))),
            _ => Err(ClientError::Upstream(format!("HTTP {status} on {PREFIX}{path}"))),
        }
    }
}

impl ParfumoClient {
    /// Whether a session token of the application interface is held.
    pub fn has_app_session(&self) -> bool {
        self.t.app_token().is_some()
    }

    /// Forgets the session token without telling the site.
    pub fn forget_app_session(&self) {
        self.t.set_app_token(None);
    }

    /// Logs in to the application interface with the member's e-mail and password, and
    /// keeps the session token. Never retried here.
    pub async fn app_login(&self, email: &str, password: &str) -> Result<(), ClientError> {
        if email.trim().is_empty() || password.is_empty() {
            return Err(ClientError::InvalidInput("e-mail and password must not be empty".into()));
        }
        // The site only asks for some text that names the device.
        let device = format!("parfumo-mcp-{:016x}", rand::random::<u64>());
        let form = [pair("email", email.trim()), pair("password", password), pair("device_id", device)];
        let (status, body) = self.t.app_send(Method::POST, "/auth/login", &form, None).await?;
        let json: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        match json.get("auth_token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            Some(token) => {
                self.t.set_app_token(Some(token));
                Ok(())
            }
            None if status >= 500 => Err(ClientError::Upstream(format!("HTTP {status} on {PREFIX}/auth/login"))),
            None if json.is_null() => Err(ClientError::ParserOutdated(format!("{PREFIX}/auth/login did not answer in JSON"))),
            None => Err(ClientError::InvalidCredentials),
        }
    }

    /// Ends the session on the site, when one is held, and forgets its token.
    pub async fn app_logout(&self) {
        if self.has_app_session() {
            let _ = self.t.app_call(Method::POST, "/auth/logout", &[]).await;
        }
        self.t.set_app_token(None);
    }

    /// `GET` as the member.
    pub(crate) async fn app_get(&self, path: &str, params: &[(String, String)]) -> Result<Value, ClientError> {
        self.t.app_call(Method::GET, path, params).await
    }

    /// `POST` as the member. Never repeated.
    #[allow(dead_code)] // The writes come with the tools that use them.
    pub(crate) async fn app_post(&self, path: &str, body: &[(String, String)]) -> Result<Value, ClientError> {
        self.t.app_call(Method::POST, path, body).await
    }

    /// The nickname of the member the token belongs to: a cheap way to check the session.
    pub async fn app_nick(&self) -> Result<String, ClientError> {
        let me = self.app_get("/auth/me", &[]).await?;
        me.pointer("/user/nick_name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .ok_or_else(|| ClientError::ParserOutdated(format!("{PREFIX}/auth/me: no nickname")))
    }
}
