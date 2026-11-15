//! The application interface of Parfumo (`/app/v1`): JSON calls authenticated by the
//! session token of the member. Measured behaviour and every call: `docs/APP_API.md`.
//!
//! The rules of that document hold for every caller. The transport enforces those about
//! what leaves the process (`Transport::send_with`); the state they rest on is here:
//! - only the member's own token is ever sent, obtained with their own e-mail and password;
//! - the token has one owner per user ([`AppSession`]), shared by the clients of that user:
//!   a token the site refuses is forgotten for all of them at once and never held again,
//!   so that no call, not even one already queued, repeats the refusal;
//! - the site blocks an address after a few requests without a valid token: once its block
//!   page is met, nothing is asked of the interface by any user until the block is over.

use std::{sync::Mutex, time::Duration};

use reqwest::Method;
use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    transport::{Auth, StoredCookie, Transport},
};

/// Path prefix of every call.
pub(crate) const PREFIX: &str = "/app/v1";
/// The login: the one call that carries no token.
pub(crate) const LOGIN: &str = "/auth/login";

/// The session token is saved and restored with the cookies of the web session, under a
/// domain no request goes to.
const TOKEN_NAME: &str = "app_token";
const TOKEN_DOMAIN: &str = "app-token.invalid";

/// How long the site blocks an address (measured: its page asks to try again in five minutes).
pub(crate) const BLOCK: Duration = Duration::from_secs(300);
/// What was recorded of the block page: an HTML page, status 404, with this heading and
/// this wait ("Access Denied … try again in 5 minutes").
const BLOCK_HEADING: &str = "Access Denied";
const BLOCK_WAIT: &str = "try again in";
/// Refused tokens remembered per user: enough to outlive every copy of them.
const REFUSED_KEPT: usize = 8;

/// Whether an answer is the page by which the site tells that it blocks the address. Only
/// what was recorded is relied on: status 404, a page that is not JSON, its words.
///
/// A match stops the application interface for every user of the process, so a page of
/// the web site must be the recorded one in full: its status, its heading and the wait it
/// announces. A page that merely says "Access Denied" is a page that does not exist.
/// The application interface (`from_app`) answers in JSON: there a page with that heading
/// under any error status is taken for the block, since the mistake costs less that way
/// (the web site answers for five minutes) than the other (a perfume reported missing,
/// and more requests sent to an address that is blocked).
pub(crate) fn is_block_page(status: u16, body: &str, from_app: bool) -> bool {
    if !body.contains(BLOCK_HEADING) || body.trim_start().starts_with(['{', '[']) {
        return false;
    }
    if from_app { status >= 400 } else { status == 404 && body.contains(BLOCK_WAIT) }
}

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// The text a JSON error answer carries, when it has one.
fn error_text(json: &Value) -> Option<String> {
    ["message", "error"].iter().find_map(|k| json.get(k).and_then(Value::as_str)).map(str::to_string)
}

/// Whether an error of the application interface hands the read over to the web site: the
/// session was refused (its token is then already forgotten), or the interface asks to
/// wait (the address is blocked, or it is rate limited). A read made of several calls
/// stops at such an error instead of answering with what it has.
pub(crate) fn hands_over(e: &ClientError) -> bool {
    matches!(e, ClientError::LoginRequired | ClientError::RateLimited { .. })
}

/// What a read does with the answer of the application interface: `Some` is the result to
/// return; `None` means the web site must answer instead, because the interface cannot
/// serve the request, has just refused the session, or asks to wait. If the web site asks
/// to wait too, its own answer says so.
pub(crate) fn settle<T>(answer: Result<Option<T>, ClientError>) -> Option<Result<T, ClientError>> {
    match answer {
        Ok(Some(value)) => Some(Ok(value)),
        Ok(None) => None,
        Err(e) if hands_over(&e) => None,
        Err(e) => Some(Err(e)),
    }
}

/// The session of the application interface of one user: the token, and what became of it.
/// There is one per user, whatever the number of clients, so that no copy of a token can
/// outlive its refusal. It never prints, and only hands the token to the transport.
#[derive(Default)]
pub(crate) struct AppSession {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    token: Option<String>,
    /// Tokens the site refused: never held again, wherever they come back from (a session
    /// stored before the refusal, the answer of an earlier login).
    refused: Vec<String>,
    /// The site ended the session, and none was obtained since.
    ended: bool,
    /// The token changed by a login or a refusal since the embedding application last
    /// asked: what it stored is out of date.
    changed: bool,
    /// Counts the times the owner dropped the session: a login that started before the
    /// last one does not bring a session back.
    drops: u64,
}

impl AppSession {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The token to send now.
    pub(crate) fn token(&self) -> Option<String> {
        self.state().token.clone()
    }

    fn held(&self) -> bool {
        self.state().token.is_some()
    }

    fn ended(&self) -> bool {
        self.state().ended
    }

    /// The site refused `sent`: it is never held again, and is forgotten if it is still the
    /// token held. A newer token, obtained while the refused call was under way, stays.
    pub(crate) fn refuse(&self, sent: &str) {
        let mut state = self.state();
        if !state.refused.iter().any(|t| t == sent) {
            if state.refused.len() == REFUSED_KEPT {
                state.refused.remove(0);
            }
            state.refused.push(sent.to_string());
        }
        if state.token.as_deref() == Some(sent) {
            state.token = None;
            state.ended = true;
            state.changed = true;
        }
    }

    /// Dropped by its owner (another account is linked, or none): nothing is left of it.
    pub(crate) fn forget(&self) {
        let mut state = self.state();
        state.token = None;
        state.ended = false;
        state.changed = false;
        state.drops += 1;
    }

    fn drops(&self) -> u64 {
        self.state().drops
    }

    /// Keeps the token a login obtained, unless the session was dropped since the login
    /// was decided (`drops` as read then) or the site already refused that token.
    fn obtained(&self, token: &str, drops: u64) -> bool {
        let mut state = self.state();
        if state.drops != drops || state.refused.iter().any(|t| t == token) {
            return false;
        }
        state.token = Some(token.to_string());
        state.ended = false;
        state.changed = true;
        true
    }

    /// Takes back a token that was stored with the cookies, unless the site refused it since.
    pub(crate) fn restore(&self, token: &str) {
        let mut state = self.state();
        if token.is_empty() || state.refused.iter().any(|t| t == token) {
            return;
        }
        state.token = Some(token.to_string());
        state.ended = false;
    }

    /// The token as it is stored with the cookies.
    pub(crate) fn stored(&self) -> Option<StoredCookie> {
        self.token().map(|value| StoredCookie { name: TOKEN_NAME.into(), value, domain: TOKEN_DOMAIN.into(), path: "/".into() })
    }

    /// Whether a stored cookie is the token, not a cookie of the web session.
    pub(crate) fn is_stored(cookie: &StoredCookie) -> bool {
        cookie.name == TOKEN_NAME && cookie.domain == TOKEN_DOMAIN
    }

    pub(crate) fn take_change(&self) -> bool {
        std::mem::take(&mut self.state().changed)
    }

    pub(crate) fn flag_change(&self) {
        self.state().changed = true;
    }
}

impl Transport {
    /// One call: `params` go in the query of a `GET` and in the body of a `POST`.
    /// A `POST` is never repeated; a `GET` is, on 429, 5xx and network errors only.
    async fn app_send(&self, method: Method, path: &str, params: &[(String, String)], auth: Auth) -> Result<(u16, String), ClientError> {
        let (target, form, retries) = if method == Method::GET {
            let query = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(params).finish();
            let target = if query.is_empty() { format!("{PREFIX}{path}") } else { format!("{PREFIX}{path}?{query}") };
            (target, None, self.cfg.max_retries)
        } else {
            (format!("{PREFIX}{path}"), Some(params), 0)
        };
        let raw = self.send_with(method, &target, form, retries, auth).await?;
        Ok((raw.status, raw.body))
    }

    /// A call made as the member, with the token the user holds when the request leaves.
    /// Without one nothing is sent. A refused token is already forgotten when this returns:
    /// the transport does it before any other call of the user can leave.
    pub(crate) async fn app_call(&self, method: Method, path: &str, params: &[(String, String)]) -> Result<Value, ClientError> {
        let (status, body) = self.app_send(method, path, params, Auth::Member).await?;
        if status == 401 {
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
    /// Whether the user holds a session token of the application interface. The clients of
    /// one user (see [`crate::UserLimiter`]) answer the same.
    pub fn has_app_session(&self) -> bool {
        self.t.user.app.held()
    }

    /// Whether the site ended the session (it refused the token) and none was obtained
    /// since: a login made now renews a session, it is not the first one.
    pub fn app_session_ended(&self) -> bool {
        self.t.user.app.ended()
    }

    /// Whether the site is known to block the address of this process: nothing is asked of
    /// the application interface until the block is over, a login included.
    pub fn app_blocked(&self) -> bool {
        self.t.blocked_secs().is_some()
    }

    /// Forgets the session token without telling the site, for every client of the user.
    pub fn forget_app_session(&self) {
        self.t.user.app.forget();
    }

    /// Counts the times the session of the user was forgotten by its owner
    /// ([`Self::forget_app_session`], [`crate::UserLimiter::forget_app_session`]). Read
    /// together with whatever decides that a login is to be made, and given to
    /// [`Self::app_login_since`]: a session forgotten after that decision is then not
    /// brought back by the login, however short the time between the two.
    pub fn app_session_generation(&self) -> u64 {
        self.t.user.app.drops()
    }

    /// Logs in to the application interface with the member's e-mail and password, and
    /// keeps the session token for every client of the user. Never retried here.
    ///
    /// `InvalidCredentials` only when the interface itself answered the login and refused
    /// it: that answer is final for these credentials. Anything else (an outage, an
    /// address that no longer exists, a rate limit) is another error. The token is not kept
    /// (`LoginRequired`) when the session was forgotten while the login was under way.
    pub async fn app_login(&self, email: &str, password: &str) -> Result<(), ClientError> {
        self.app_login_since(email, password, self.app_session_generation()).await
    }

    /// [`Self::app_login`] decided at an earlier moment, when the session was at
    /// `generation` ([`Self::app_session_generation`]): the token is not kept
    /// (`LoginRequired`) when the session was forgotten since then, before the login
    /// started as well as while it was under way.
    pub async fn app_login_since(&self, email: &str, password: &str, generation: u64) -> Result<(), ClientError> {
        if email.trim().is_empty() || password.is_empty() {
            return Err(ClientError::InvalidInput("e-mail and password must not be empty".into()));
        }
        // The site only asks for some text that names the device.
        let device = format!("parfumo-mcp-{:016x}", rand::random::<u64>());
        let form = [pair("email", email.trim()), pair("password", password), pair("device_id", device)];
        let (status, body) = self.t.app_send(Method::POST, LOGIN, &form, Auth::Login).await?;
        let json: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        // What the interface answers to a wrong password was never recorded. Its recorded
        // way of saying no is `{"status":"error", …}`, under 401 when it does not know who
        // is asking: only that is read as a refusal of the credentials, which is final.
        // 400, 403 and 422 may as well be a lockout, a rule on the address or a field that
        // became required: they say nothing certain of the credentials.
        let refused = !json.is_null() && (status == 401 || (status < 300 && json.get("status").and_then(Value::as_str) == Some("error")));
        match json.get("auth_token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            Some(token) if self.t.user.app.obtained(token, generation) => Ok(()),
            Some(_) => Err(ClientError::LoginRequired),
            None if refused => Err(ClientError::InvalidCredentials),
            None if json.is_null() && status < 400 => Err(ClientError::ParserOutdated(format!("{PREFIX}{LOGIN} did not answer in JSON"))),
            None if status < 300 => Err(ClientError::ParserOutdated(format!("{PREFIX}{LOGIN}: no session token in the answer"))),
            None => Err(ClientError::Upstream(format!("HTTP {status} on {PREFIX}{LOGIN}"))),
        }
    }

    /// Ends the session on the site, when one is held, and forgets its token.
    pub async fn app_logout(&self) {
        if self.has_app_session() {
            let _ = self.t.app_call(Method::POST, "/auth/logout", &[]).await;
        }
        self.t.user.app.forget();
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

#[cfg(test)]
mod tests {
    use super::is_block_page;

    #[test]
    fn the_block_page_is_what_was_recorded_of_it_and_no_other_page() {
        let page = "<html><head><title>Access Denied</title></head><body><h1>Access Denied</h1><p>Please try again in 5 minutes.</p></body></html>";
        assert!(is_block_page(404, page, false) && is_block_page(404, page, true));
        // On the web site: under another status, or with the heading alone, it is another page.
        let heading_alone = "<html><body><h1>Access Denied</h1><p>This page is hidden.</p></body></html>";
        assert!(!is_block_page(403, page, false) && !is_block_page(200, page, false));
        assert!(!is_block_page(404, heading_alone, false));
        assert!(!is_block_page(404, "<html><body>Not found. Please try again in a moment.</body></html>", false));
        // The application interface answers in JSON: a page with that heading, under any
        // error status, is not one of its answers.
        assert!(is_block_page(404, heading_alone, true) && is_block_page(403, heading_alone, true));
        assert!(!is_block_page(200, heading_alone, true));
        // An answer in JSON is never the block page, whatever it says.
        let json = r#"{"status":"error","message":"Access Denied, try again in 5 minutes"}"#;
        assert!(!is_block_page(404, json, true) && !is_block_page(404, json, false));
    }
}
