//! Server-to-Parfumo side: one isolated client (cookie jar, rate limiter) per user, the
//! encrypted link of each user's Parfumo account, and automatic re-login.
//!
//! The login itself was measured at jalon 0 bis (DISCOVERY §4). The retry logic and the
//! encryption are covered by unit tests; the renewal path is exercised against a mock.

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, PageCache, ParfumoClient, StoredCookie, UserLimiter};
use serde::{Deserialize, Serialize};

use crate::{config::Config, crypto::MasterKey, db::Db};

pub const MAX_LOGIN_ATTEMPTS: u32 = 3;
/// A session checked less than this long ago is used without a new probe (the longest
/// session observed lived more than 12 minutes; an expired one is renewed on failure anyway).
const SESSION_TRUST: Duration = Duration::from_secs(10 * 60);
/// After a captcha or three failed logins, no new probe or login for this long.
pub const LOGIN_BACKOFF: Duration = Duration::from_secs(15 * 60);
/// Link statuses that mean "Parfumo refused this link": only /link clears them.
pub const STATUS_REAUTH: &str = "reauth_required";
pub const STATUS_EXPIRED: &str = "expired";

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("no Parfumo account is linked for this user")]
    NotLinked,
    #[error("the Parfumo session cannot be renewed: the account must be linked again")]
    ReauthRequired,
    #[error("Parfumo asks for a human verification; log in with your browser and import the cookie")]
    CaptchaRequired,
    #[error("internal error: {0}")]
    Internal(String),
}

/// What the re-login logic needs from a Parfumo client (mockable in tests).
pub trait ParfumoAuth: Sync {
    fn login(&self, username: &str, password: &str) -> impl Future<Output = Result<(), ClientError>> + Send;
}

impl ParfumoAuth for ParfumoClient {
    fn login(&self, username: &str, password: &str) -> impl Future<Output = Result<(), ClientError>> + Send {
        ParfumoClient::login(self, username, password)
    }
}

/// At most three attempts, then `reauth_required`.
///
/// A rejected password or a captcha ends the loop at once: repeating the same refused
/// credentials would only hammer the login form and could lock the account.
pub async fn login_with_retries<A: ParfumoAuth>(auth: &A, creds: &Credentials, pause: Duration) -> Result<u32, SessionError> {
    login_attempts(auth, creds, pause).await.map_err(|(e, _)| e)
}

/// [`login_with_retries`], also telling whether Parfumo refused the credentials themselves
/// (`true`: trying them again is pointless until the user links the account again) or the
/// attempts merely ran out (`false`: an outage, worth another try later).
pub async fn login_attempts<A: ParfumoAuth>(auth: &A, creds: &Credentials, pause: Duration) -> Result<u32, (SessionError, bool)> {
    for attempt in 1..=MAX_LOGIN_ATTEMPTS {
        match auth.login(&creds.username, &creds.password).await {
            Ok(()) => return Ok(attempt),
            Err(ClientError::InvalidCredentials) => return Err((SessionError::ReauthRequired, true)),
            Err(ClientError::CaptchaRequired) => return Err((SessionError::CaptchaRequired, false)),
            Err(e) => {
                // `ClientError` never carries a secret (tested in parfumo-client).
                tracing::warn!(attempt, error = %e, "parfumo login attempt failed");
                if attempt < MAX_LOGIN_ATTEMPTS {
                    tokio::time::sleep(pause * 2u32.pow(attempt - 1)).await;
                }
            }
        }
    }
    Err((SessionError::ReauthRequired, false))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    pub mode: String,
    pub status: String,
    pub linked_at: i64,
    pub last_login_at: Option<i64>,
}

pub struct Sessions {
    cfg: Config,
    db: Db,
    key: MasterKey,
    global: Arc<GlobalLimiter>,
    clients: Mutex<HashMap<i64, ParfumoClient>>,
    public_clients: Mutex<HashMap<i64, ParfumoClient>>,
    /// One request slot per user, shared by that user's public and account clients.
    limiters: Mutex<HashMap<i64, Arc<UserLimiter>>>,
    verified: Mutex<HashMap<i64, std::time::Instant>>,
    /// One re-login at a time per user; a slow or dead session of one user never makes
    /// the others wait.
    relogin: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    /// Users whose last probe or re-login met a captcha or an outage: no new attempt
    /// before the instant given, so that a refusal is never hammered.
    backoff: Mutex<HashMap<i64, (std::time::Instant, SessionError)>>,
    /// The `rev` of the link each in-memory session belongs to. Another process may link
    /// a different account (`parfumo-mcp login` next to a running stdio server): the
    /// session kept here is then dropped instead of acting for the previous account.
    link_revs: Mutex<HashMap<i64, String>>,
}

fn aad(user_id: i64, kind: &str) -> String {
    format!("user:{user_id}:{kind}")
}

impl Sessions {
    pub fn new(cfg: Config, db: Db, key: MasterKey) -> Self {
        let global = GlobalLimiter::new(cfg.global_rps);
        Self {
            cfg,
            db,
            key,
            global,
            clients: Mutex::new(HashMap::new()),
            public_clients: Mutex::new(HashMap::new()),
            limiters: Mutex::new(HashMap::new()),
            verified: Mutex::new(HashMap::new()),
            relogin: Mutex::new(HashMap::new()),
            backoff: Mutex::new(HashMap::new()),
            link_revs: Mutex::new(HashMap::new()),
        }
    }

    /// Drops what is kept in memory for `user_id` when it belongs to another link than the
    /// one now stored (`rev`), or to a link that is gone (`None`).
    fn follow_link(&self, user_id: i64, rev: Option<&str>) {
        let mut revs = self.link_revs.lock().unwrap_or_else(|p| p.into_inner());
        if revs.get(&user_id).map(String::as_str) == rev {
            return;
        }
        match rev {
            Some(rev) => revs.insert(user_id, rev.to_string()),
            None => revs.remove(&user_id),
        };
        drop(revs);
        self.clients.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.forget_verified(user_id);
        self.backoff.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
    }

    fn new_client(&self, user_id: i64, cached: bool) -> Result<ParfumoClient, SessionError> {
        let limiter = self.limiters.lock().unwrap_or_else(|p| p.into_inner()).entry(user_id).or_default().clone();
        let cfg = ClientConfig {
            base_url: self.cfg.parfumo_base_url.clone(),
            min_interval: self.cfg.user_min_interval,
            jitter: self.cfg.user_jitter,
            ..ClientConfig::default()
        };
        let cache: Arc<dyn PageCache> = Arc::new(self.db.clone());
        ParfumoClient::with_limiter(cfg, self.global.clone(), cached.then_some(cache), limiter).map_err(|e| SessionError::Internal(e.to_string()))
    }

    /// Anonymous client of one user, for the public database tools. It never carries the
    /// user's Parfumo cookies: public pages are fetched without the account, so nothing
    /// account-specific can reach the shared page cache.
    pub fn public_client_for(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        let mut map = self.public_clients.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = map.get(&user_id) {
            return Ok(c.clone());
        }
        let client = self.new_client(user_id, true)?;
        map.insert(user_id, client.clone());
        Ok(client)
    }

    /// The account client of one user (its own cookie jar and rate limiter, never shared,
    /// never cached).
    pub fn client_for(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        if let Some(c) = self.clients.lock().unwrap_or_else(|p| p.into_inner()).get(&user_id) {
            return Ok(c.clone());
        }
        let client = self.new_client(user_id, false)?;
        if let Some(cookies) = self.stored_cookies(user_id)? {
            client.import_cookies(&cookies);
        }
        let mut map = self.clients.lock().unwrap_or_else(|p| p.into_inner());
        Ok(map.entry(user_id).or_insert(client).clone())
    }

    fn stored_cookies(&self, user_id: i64) -> Result<Option<Vec<StoredCookie>>, SessionError> {
        let Some(link) = self.db.link_get(user_id).map_err(internal)? else { return Ok(None) };
        let Some(blob) = link.cookies else { return Ok(None) };
        let plain = self.key.decrypt(&blob, &aad(user_id, "cookies")).map_err(internal)?;
        Ok(Some(serde_json::from_slice(&plain).map_err(internal)?))
    }

    fn encrypt_cookies(&self, user_id: i64, client: &ParfumoClient) -> Result<Vec<u8>, SessionError> {
        let json = serde_json::to_vec(&client.export_cookies()).map_err(internal)?;
        self.key.encrypt(&json, &aad(user_id, "cookies")).map_err(internal)
    }

    fn forget_verified(&self, user_id: i64) {
        self.verified.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
    }

    /// Logged-in client for an account tool. `not_linked` without a link; otherwise the
    /// session is probed (and renewed if needed) unless it was checked recently.
    pub async fn account_client(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        let link = self.db.link_get(user_id).map_err(internal)?;
        self.follow_link(user_id, link.as_ref().map(|l| l.rev.as_str()));
        if link.is_none() {
            return Err(SessionError::NotLinked);
        }
        let fresh = self.verified.lock().unwrap_or_else(|p| p.into_inner()).get(&user_id).is_some_and(|t| t.elapsed() < SESSION_TRUST);
        if fresh {
            return self.client_for(user_id);
        }
        self.renew(user_id).await
    }

    /// Probes the session now and logs in again if Parfumo no longer accepts it.
    pub async fn renew(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        self.forget_verified(user_id);
        let client = self.ensure_logged_in(user_id).await?;
        self.verified.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, std::time::Instant::now());
        Ok(client)
    }

    fn back_off(&self, user_id: i64, error: &SessionError) {
        self.backoff.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, (std::time::Instant::now() + LOGIN_BACKOFF, error.clone()));
    }

    fn backing_off(&self, user_id: i64) -> Option<SessionError> {
        let mut map = self.backoff.lock().unwrap_or_else(|p| p.into_inner());
        match map.get(&user_id) {
            Some((until, error)) if std::time::Instant::now() < *until => Some(error.clone()),
            Some(_) => {
                map.remove(&user_id);
                None
            }
            None => None,
        }
    }

    /// Ends the pause that follows a captcha or an outage (tests: time cannot be advanced).
    #[cfg(any(test, feature = "test-util"))]
    pub fn end_backoff(&self, user_id: i64) {
        self.backoff.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
    }

    /// A client without cache and without account cookies, for the canary: it reads the
    /// public site as it is now, through the rate limiter slot of `user_id`.
    pub fn uncached_public_client(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        self.new_client(user_id, false)
    }

    fn replace_client(&self, user_id: i64, client: ParfumoClient, rev: String) {
        self.link_revs.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, rev);
        self.forget_verified(user_id);
        self.backoff.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.clients.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, client);
    }

    pub fn link_info(&self, user_id: i64) -> Result<Option<LinkInfo>, SessionError> {
        Ok(self.db.link_get(user_id).map_err(internal)?.map(|l| LinkInfo {
            mode: l.mode,
            status: l.status,
            linked_at: l.linked_at,
            last_login_at: l.last_login_at,
        }))
    }

    /// Credentials mode: logs in once with a fresh cookie jar; credentials are stored
    /// (encrypted) only if Parfumo accepted them.
    pub async fn link_credentials(&self, user_id: i64, username: &str, password: &str) -> Result<(), SessionError> {
        let creds = Credentials { username: username.to_string(), password: password.to_string() };
        let client = self.new_client(user_id, false)?;
        login_with_retries(&client, &creds, self.cfg.user_min_interval * 2).await?;
        let secret = self.key.encrypt(&serde_json::to_vec(&creds).map_err(internal)?, &aad(user_id, "credentials")).map_err(internal)?;
        let cookies = self.encrypt_cookies(user_id, &client)?;
        let rev = self.db.link_put(user_id, "credentials", Some(&secret), Some(&cookies), "linked").map_err(internal)?;
        self.db.link_update_session(user_id, Some(&cookies), "linked", true).map_err(internal)?;
        self.replace_client(user_id, client, rev);
        Ok(())
    }

    /// Cookie mode: no password on the server. Returns whether Parfumo treats the imported
    /// cookies as a logged-in session (heuristic, unproven without a test account).
    pub async fn link_cookie(&self, user_id: i64, cookie_header: &str) -> Result<bool, SessionError> {
        let client = self.new_client(user_id, false)?;
        if client.import_cookie_header(cookie_header) == 0 {
            return Err(SessionError::Internal("no cookie found in the pasted text (expected `name=value; name=value`)".into()));
        }
        let logged_in = client.is_logged_in().await.unwrap_or(false);
        let cookies = self.encrypt_cookies(user_id, &client)?;
        let status = if logged_in { "linked" } else { "unverified" };
        let rev = self.db.link_put(user_id, "cookie", None, Some(&cookies), status).map_err(internal)?;
        self.replace_client(user_id, client, rev);
        Ok(logged_in)
    }

    /// Erases credentials and cookies, and drops the in-memory session.
    pub fn unlink(&self, user_id: i64) -> Result<bool, SessionError> {
        self.link_revs.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.clients.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.forget_verified(user_id);
        self.backoff.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.db.link_delete(user_id).map_err(internal)
    }

    /// Returns a logged-in client, re-logging in when the session expired (credentials mode).
    /// Needed by account tools (jalons 2+); in jalon 1 it backs the "test session" button.
    pub async fn ensure_logged_in(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        let link = self.db.link_get(user_id).map_err(internal)?;
        self.follow_link(user_id, link.as_ref().map(|l| l.rev.as_str()));
        let link = link.ok_or(SessionError::NotLinked)?;
        // Parfumo already refused this link: nothing is sent, not even a probe, until the
        // user links the account again on /link (which resets the status).
        if matches!(link.status.as_str(), STATUS_REAUTH | STATUS_EXPIRED) {
            return Err(SessionError::ReauthRequired);
        }
        if let Some(error) = self.backing_off(user_id) {
            return Err(error);
        }
        let client = self.client_for(user_id)?;
        let lock = self.relogin.lock().unwrap_or_else(|p| p.into_inner()).entry(user_id).or_default().clone();
        let _guard = lock.lock().await;
        // Another call of the same user may have met the refusal while this one waited.
        if let Some(error) = self.backing_off(user_id) {
            return Err(error);
        }
        match client.is_logged_in().await {
            Ok(true) => return Ok(client),
            Ok(false) => {}
            Err(ClientError::CaptchaRequired) => {
                self.back_off(user_id, &SessionError::CaptchaRequired);
                return Err(SessionError::CaptchaRequired);
            }
            Err(e) => return Err(SessionError::Internal(e.to_string())),
        }
        let Some(secret) = link.secret.filter(|_| link.mode == "credentials") else {
            self.db.link_update_session(user_id, link.cookies.as_deref(), STATUS_EXPIRED, false).map_err(internal)?;
            return Err(SessionError::ReauthRequired);
        };
        let plain = self.key.decrypt(&secret, &aad(user_id, "credentials")).map_err(internal)?;
        let creds: Credentials = serde_json::from_slice(&plain).map_err(internal)?;
        client.clear_cookies();
        match login_attempts(&client, &creds, self.cfg.user_min_interval * 2).await {
            Ok(_) => {
                let cookies = self.encrypt_cookies(user_id, &client)?;
                self.db.link_update_session(user_id, Some(&cookies), "linked", true).map_err(internal)?;
                Ok(client)
            }
            // The credentials themselves were refused: stop for good.
            Err((e, true)) => {
                self.db.link_update_session(user_id, None, STATUS_REAUTH, false).map_err(internal)?;
                Err(e)
            }
            // A captcha or an outage: the credentials may still be good. Wait before any
            // new attempt instead of repeating it on every tool call.
            Err((e, false)) => {
                let status = if e == SessionError::CaptchaRequired { "captcha_required" } else { "login_failed" };
                self.db.link_update_session(user_id, None, status, false).map_err(internal)?;
                self.back_off(user_id, &e);
                Err(e)
            }
        }
    }
}

// ---- local account (stdio server) ------------------------------------------------------------

pub const ENV_LOCAL_USER: &str = "PARFUMO_USER";
pub const ENV_LOCAL_PASSWORD: &str = "PARFUMO_PASSWORD";
pub const ENV_LOCAL_COOKIE: &str = "PARFUMO_COOKIE";
/// How the account is (re)linked on the stdio server when it comes from the environment,
/// as the end of a sentence.
pub const LOCAL_LINK_HOW: &str = "by setting PARFUMO_USER and PARFUMO_PASSWORD (or PARFUMO_COOKIE, the `pf_auth=…` cookie of a browser logged in to Parfumo) in the environment of this local MCP server, in the MCP client's configuration, and restarting it";
/// The same when the stdio server has a login page but could not open it now.
pub const LOCAL_LOGIN_HOW: &str = "by running `parfumo-mcp login` in a terminal on the computer where this local MCP server runs";
/// What follows the address of the local login page in a message to the assistant.
pub const LOCAL_PAGE_NOTE: &str =
    "a page of this local MCP server, on the user's own computer, open for ten minutes; give the user this exact link to open in their browser";

/// The single Parfumo account of the stdio server, as read from its environment.
#[derive(Clone, PartialEq, Eq)]
pub enum LocalAccount {
    Credentials(Credentials),
    /// A `Cookie` header copied from a logged-in browser (`pf_auth=…` is enough).
    Cookie(String),
}

impl std::fmt::Debug for LocalAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Credentials(_) => "LocalAccount::Credentials(<redacted>)",
            Self::Cookie(_) => "LocalAccount::Cookie(<redacted>)",
        })
    }
}

impl LocalAccount {
    /// Reads `PARFUMO_USER` + `PARFUMO_PASSWORD`, or `PARFUMO_COOKIE`, through `var` (the
    /// process environment in production). `None`: no account, public tools only. Half a
    /// pair of credentials, or both forms at once, is refused rather than guessed.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let get = |name: &str| var(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        match (get(ENV_LOCAL_USER), get(ENV_LOCAL_PASSWORD), get(ENV_LOCAL_COOKIE)) {
            (None, None, None) => Ok(None),
            (Some(username), Some(password), None) => Ok(Some(Self::Credentials(Credentials { username, password }))),
            (None, None, Some(cookie)) => Ok(Some(Self::Cookie(cookie))),
            (Some(_), None, _) | (None, Some(_), _) => Err(format!("{ENV_LOCAL_USER} and {ENV_LOCAL_PASSWORD} go together: set both, or neither")),
            (Some(_), Some(_), Some(_)) => Err(format!("set either {ENV_LOCAL_USER} and {ENV_LOCAL_PASSWORD}, or {ENV_LOCAL_COOKIE}, not both")),
        }
    }
}

impl Sessions {
    /// Stdio server: makes the stored link of `user_id` match the account of the
    /// environment, without any request to Parfumo (the first account tool logs in).
    ///
    /// An unchanged account keeps its stored session and its status: the cookies of the
    /// last run are reused instead of logging in at every start, and credentials Parfumo
    /// refused are not sent again until they change. A different account replaces the
    /// stored one. Without an account in the environment, the one a previous start took
    /// from it is erased, and one linked on the local login page is kept. Returns whether
    /// the stored link changed.
    pub fn sync_local_link(&self, user_id: i64, account: Option<&LocalAccount>) -> Result<bool, SessionError> {
        let stored = self.db.link_get(user_id).map_err(internal)?;
        let Some(account) = account else {
            return if stored.is_some_and(|link| link.from_env) { self.unlink(user_id) } else { Ok(false) };
        };
        let (mode, source) = match account {
            LocalAccount::Credentials(creds) => ("credentials", serde_json::to_vec(creds).map_err(internal)?),
            // Only a fingerprint of the pasted cookie is kept, to recognise it at the next start.
            LocalAccount::Cookie(header) => ("cookie", crate::crypto::token_hash(header).into_bytes()),
        };
        let kind = if mode == "credentials" { "credentials" } else { "cookie-source" };
        let same = stored.as_ref().is_some_and(|link| {
            link.mode == mode
                && link.secret.as_ref().and_then(|blob| self.key.decrypt(blob, &aad(user_id, kind)).ok()).is_some_and(|plain| plain == source)
        });
        if same {
            // A database of version 0.1 does not say where its account came from.
            self.db.link_mark_from_env(user_id).map_err(internal)?;
            return Ok(false);
        }
        let secret = self.key.encrypt(&source, &aad(user_id, kind)).map_err(internal)?;
        let cookies = match account {
            LocalAccount::Credentials(_) => None,
            LocalAccount::Cookie(header) => {
                let client = self.new_client(user_id, false)?;
                if client.import_cookie_header(header) == 0 {
                    return Err(SessionError::Internal(format!(
                        "{ENV_LOCAL_COOKIE} holds no cookie (expected `pf_auth=value`, or a whole `Cookie` header)"
                    )));
                }
                Some(self.encrypt_cookies(user_id, &client)?)
            }
        };
        self.clients.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.forget_verified(user_id);
        self.backoff.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        let rev = self.db.link_put(user_id, mode, Some(&secret), cookies.as_deref(), "linked").map_err(internal)?;
        self.db.link_mark_from_env(user_id).map_err(internal)?;
        self.link_revs.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, rev);
        Ok(true)
    }
}

fn internal(e: impl std::fmt::Display) -> SessionError {
    SessionError::Internal(e.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    struct Mock {
        calls: AtomicU32,
        script: Vec<Result<(), ClientError>>,
    }

    impl Mock {
        fn new(script: Vec<Result<(), ClientError>>) -> Self {
            Self { calls: AtomicU32::new(0), script }
        }
    }

    impl ParfumoAuth for Mock {
        async fn login(&self, _u: &str, _p: &str) -> Result<(), ClientError> {
            let i = self.calls.fetch_add(1, Ordering::SeqCst) as usize;
            match &self.script[i.min(self.script.len() - 1)] {
                Ok(()) => Ok(()),
                Err(ClientError::InvalidCredentials) => Err(ClientError::InvalidCredentials),
                Err(ClientError::CaptchaRequired) => Err(ClientError::CaptchaRequired),
                Err(_) => Err(ClientError::Upstream("503".into())),
            }
        }
    }

    fn creds() -> Credentials {
        Credentials { username: "someone".into(), password: "s3cr3t-p4ss".into() }
    }

    #[tokio::test]
    async fn three_attempts_then_reauth_required() {
        let m = Mock::new(vec![Err(ClientError::Upstream("x".into()))]);
        let r = login_with_retries(&m, &creds(), Duration::ZERO).await;
        assert_eq!(r, Err(SessionError::ReauthRequired));
        assert_eq!(m.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn succeeds_on_a_later_attempt() {
        let m = Mock::new(vec![Err(ClientError::Upstream("x".into())), Ok(())]);
        assert_eq!(login_with_retries(&m, &creds(), Duration::ZERO).await, Ok(2));
        assert_eq!(m.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rejected_password_and_captcha_are_not_retried() {
        let m = Mock::new(vec![Err(ClientError::InvalidCredentials)]);
        assert_eq!(login_with_retries(&m, &creds(), Duration::ZERO).await, Err(SessionError::ReauthRequired));
        assert_eq!(m.calls.load(Ordering::SeqCst), 1);
        let m = Mock::new(vec![Err(ClientError::CaptchaRequired)]);
        assert_eq!(login_with_retries(&m, &creds(), Duration::ZERO).await, Err(SessionError::CaptchaRequired));
        assert_eq!(m.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn credentials_never_print() {
        let c = creds();
        assert!(!format!("{c:?}").contains("s3cr3t"));
        for e in [SessionError::NotLinked, SessionError::ReauthRequired, SessionError::CaptchaRequired] {
            assert!(!e.to_string().contains("s3cr3t"));
        }
    }
}
