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

use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient, StoredCookie, UserLimiter};
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
/// Link statuses left by a login to the web site that met a captcha or an outage: the
/// next account tool tries again once the pause is over.
pub const STATUS_CAPTCHA: &str = "captcha_required";
pub const STATUS_LOGIN_FAILED: &str = "login_failed";
/// The link works, as far as the last login to the web site told (or, on the stdio
/// server, no login was needed yet).
pub const STATUS_LINKED: &str = "linked";

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
    /// The web site only: the account is linked as soon as the web site accepts the
    /// credentials. The application interface has a login of its own, with its own rules
    /// (`Sessions::ensure_app_session`), which a new attempt on the web site must not repeat.
    async fn login(&self, username: &str, password: &str) -> Result<(), ClientError> {
        ParfumoClient::login(self, username, password).await
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
    /// What the public client and the account client of a user share: one request slot,
    /// and the one place where the session token of the application interface of that
    /// user is held. No other user's client is ever given it.
    limiters: Mutex<HashMap<i64, Arc<UserLimiter>>>,
    verified: Mutex<HashMap<i64, std::time::Instant>>,
    /// One re-login at a time per user; a slow or dead session of one user never makes
    /// the others wait.
    relogin: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    /// Users whose last probe or re-login met a captcha or an outage: no new attempt
    /// before the instant given, so that a refusal is never hammered.
    backoff: Mutex<HashMap<i64, (std::time::Instant, SessionError)>>,
    /// One login to the application interface at a time per user: calls made together
    /// wait for the first one and use what it obtained, or learn what it met.
    app_login: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    /// Users for whom no login to the application interface is made before the instant
    /// given: the last one failed, or the session it obtained was lost at once.
    app_backoff: Mutex<HashMap<i64, std::time::Instant>>,
    /// When the session of the application interface of a user was last renewed, that is
    /// obtained again after the site had ended it. One renewal is allowed; a session lost
    /// again within [`LOGIN_BACKOFF`] of it is not asked for until that time is over.
    /// The link records the same (`app_ended`, `app_renewed_at`) for the next process: a
    /// restart is not a way round the pause.
    app_renewed: Mutex<HashMap<i64, std::time::Instant>>,
    /// The `rev` of the link each in-memory session belongs to. Another process may link
    /// a different account (`parfumo-mcp login` next to a running stdio server): the
    /// session kept here is then dropped instead of acting for the previous account.
    ///
    /// Its lock is also the one under which a link is read or replaced together with what
    /// is kept in memory for it: the client of an account and the `rev` it belongs to
    /// always change as one. It is taken before any other lock of this struct.
    link_revs: Mutex<HashMap<i64, String>>,
    /// Tests only: run once at the instant a login to the application interface leaves,
    /// after everything that allows it was checked. A link that changes at that very
    /// instant cannot be arranged from outside.
    #[cfg(test)]
    as_the_app_login_leaves: Mutex<Option<Box<dyn FnOnce() + Send>>>,
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
            app_login: Mutex::new(HashMap::new()),
            app_backoff: Mutex::new(HashMap::new()),
            app_renewed: Mutex::new(HashMap::new()),
            link_revs: Mutex::new(HashMap::new()),
            #[cfg(test)]
            as_the_app_login_leaves: Mutex::new(None),
        }
    }

    fn revs(&self) -> std::sync::MutexGuard<'_, HashMap<i64, String>> {
        self.link_revs.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// What the clients of `user_id` share.
    fn limiter(&self, user_id: i64) -> Arc<UserLimiter> {
        self.limiters.lock().unwrap_or_else(|p| p.into_inner()).entry(user_id).or_default().clone()
    }

    /// Drops everything kept in memory for the link of `user_id`: the client of the
    /// account, what is known of its web session, the pauses, and the session of the
    /// application interface, whose token belongs to the account of that link. Called with
    /// the lock of `link_revs` held, when the link changes or goes.
    fn drop_link_state(&self, user_id: i64) {
        self.clients.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.forget_verified(user_id);
        self.backoff.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.app_backoff.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.app_renewed.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.limiter(user_id).forget_app_session();
    }

    /// Reads the link of `user_id` and drops what is kept in memory when it belongs to
    /// another link than the one now stored, or to a link that is gone. Read and compared
    /// as one step, so that a link made meanwhile is not taken for an older one.
    fn follow(&self, user_id: i64) -> Result<Option<crate::db::LinkRow>, SessionError> {
        let mut revs = self.revs();
        let link = self.db.link_get(user_id).map_err(internal)?;
        let rev = link.as_ref().map(|l| l.rev.as_str());
        if revs.get(&user_id).map(String::as_str) != rev {
            match rev {
                Some(rev) => revs.insert(user_id, rev.to_string()),
                None => revs.remove(&user_id),
            };
            self.drop_link_state(user_id);
        }
        Ok(link)
    }

    /// Whether the link `rev` is still the one this process keeps a session for.
    fn follows(&self, user_id: i64, rev: &str) -> bool {
        self.revs().get(&user_id).is_some_and(|current| current == rev)
    }

    fn client_on(&self, limiter: Arc<UserLimiter>) -> Result<ParfumoClient, SessionError> {
        let cfg = ClientConfig {
            base_url: self.cfg.parfumo_base_url.clone(),
            min_interval: self.cfg.user_min_interval,
            jitter: self.cfg.user_jitter,
            ..ClientConfig::default()
        };
        // No page is kept: every read asks the site, so that an answer is never an old copy.
        ParfumoClient::with_limiter(cfg, self.global.clone(), None, limiter).map_err(|e| SessionError::Internal(e.to_string()))
    }

    fn new_client(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        self.client_on(self.limiter(user_id))
    }

    /// Anonymous client of one user, for the public database tools. It never carries the
    /// user's Parfumo cookies: public pages are fetched without the account. It reads the
    /// application interface with the session token of the user's linked account, which it
    /// shares with the account client; the token is not a cookie and goes nowhere else.
    pub fn public_client_for(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        let mut map = self.public_clients.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = map.get(&user_id) {
            return Ok(c.clone());
        }
        let client = self.new_client(user_id)?;
        map.insert(user_id, client.clone());
        Ok(client)
    }

    /// The account client of one user (its own cookie jar, never shared), with the session
    /// stored for the link this process follows.
    pub fn client_for(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        if let Some(c) = self.clients.lock().unwrap_or_else(|p| p.into_inner()).get(&user_id) {
            return Ok(c.clone());
        }
        // Under the lock of the links: the stored session is read, and the client kept, for
        // one and the same link.
        let revs = self.revs();
        if let Some(c) = self.clients.lock().unwrap_or_else(|p| p.into_inner()).get(&user_id) {
            return Ok(c.clone());
        }
        let client = self.new_client(user_id)?;
        let link = self.db.link_get(user_id).map_err(internal)?.filter(|link| revs.get(&user_id) == Some(&link.rev));
        if let Some(blob) = link.and_then(|link| link.cookies) {
            let plain = self.key.decrypt(&blob, &aad(user_id, "cookies")).map_err(internal)?;
            let cookies: Vec<StoredCookie> = serde_json::from_slice(&plain).map_err(internal)?;
            client.import_cookies(&cookies);
        }
        self.clients.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, client.clone());
        Ok(client)
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
        if self.follow(user_id)?.is_none() {
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

    /// A client of its own, without account cookies, for the canary: it reads the public
    /// site through the rate limiter slot of `user_id`, and the web pages only, whatever
    /// session of the application interface that user holds (the canary checks the HTML).
    pub fn uncached_public_client(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        self.client_on(self.limiter(user_id).web_only())
    }

    /// Stores a new link of `user_id` and makes it the one this process follows, as one
    /// step. What was kept in memory for the previous link is dropped first: the cookies
    /// stored with the new link cannot carry the session token of the previous account,
    /// and nothing obtained for that account is written into the new row afterwards.
    /// `client`, when given, holds the web session of the new link and is kept for it.
    fn store_link(
        &self,
        user_id: i64,
        mode: &str,
        secret: Option<&[u8]>,
        client: Option<&ParfumoClient>,
        status: &str,
        logged_in_now: bool,
    ) -> Result<String, SessionError> {
        let mut revs = self.revs();
        self.drop_link_state(user_id);
        let cookies = client.map(|client| self.encrypt_cookies(user_id, client)).transpose()?;
        let rev = self.db.link_put(user_id, mode, secret, cookies.as_deref(), status).map_err(internal)?;
        if logged_in_now {
            self.db.link_update_session(user_id, &rev, cookies.as_deref(), status, true).map_err(internal)?;
        }
        revs.insert(user_id, rev.clone());
        if let Some(client) = client {
            self.clients.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, client.clone());
        }
        Ok(rev)
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
    /// (encrypted) only if Parfumo accepted them. The account is linked as soon as the web
    /// site accepts them; the application interface is then asked for a session, once:
    /// without one, the tools read the web pages instead.
    pub async fn link_credentials(&self, user_id: i64, username: &str, password: &str) -> Result<(), SessionError> {
        let creds = Credentials { username: username.to_string(), password: password.to_string() };
        let client = self.new_client(user_id)?;
        login_with_retries(&client, &creds, self.cfg.user_min_interval * 2).await?;
        let secret = self.key.encrypt(&serde_json::to_vec(&creds).map_err(internal)?, &aad(user_id, "credentials")).map_err(internal)?;
        let rev = self.store_link(user_id, "credentials", Some(&secret), Some(&client), STATUS_LINKED, true)?;
        self.ensure_app_session(user_id, &client, &rev).await;
        Ok(())
    }

    /// Cookie mode: no password on the server. Returns whether Parfumo treats the imported
    /// cookies as a logged-in session (heuristic, unproven without a test account).
    pub async fn link_cookie(&self, user_id: i64, cookie_header: &str) -> Result<bool, SessionError> {
        let client = self.new_client(user_id)?;
        if client.import_cookie_header(cookie_header) == 0 {
            return Err(SessionError::Internal("no cookie found in the pasted text (expected `name=value; name=value`)".into()));
        }
        let logged_in = client.is_logged_in().await.unwrap_or(false);
        let status = if logged_in { STATUS_LINKED } else { "unverified" };
        self.store_link(user_id, "cookie", None, Some(&client), status, false)?;
        Ok(logged_in)
    }

    /// Erases credentials and cookies, and drops the in-memory session, the session token
    /// of the application interface included.
    pub fn unlink(&self, user_id: i64) -> Result<bool, SessionError> {
        let mut revs = self.revs();
        revs.remove(&user_id);
        self.drop_link_state(user_id);
        self.db.link_delete(user_id).map_err(internal)
    }

    /// Returns a logged-in client, re-logging in when the session expired (credentials mode).
    /// Needed by account tools (jalons 2+); in jalon 1 it backs the "test session" button.
    pub async fn ensure_logged_in(&self, user_id: i64) -> Result<ParfumoClient, SessionError> {
        let link = self.follow(user_id)?.ok_or(SessionError::NotLinked)?;
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
            Ok(true) => {
                if app_login_possible(&link) {
                    self.ensure_app_session(user_id, &client, &link.rev).await;
                }
                return Ok(client);
            }
            Ok(false) => {}
            Err(ClientError::CaptchaRequired) => {
                self.back_off(user_id, &SessionError::CaptchaRequired);
                return Err(SessionError::CaptchaRequired);
            }
            Err(e) => return Err(SessionError::Internal(e.to_string())),
        }
        let Some(secret) = link.secret.filter(|_| link.mode == "credentials") else {
            self.db.link_update_session(user_id, &link.rev, link.cookies.as_deref(), STATUS_EXPIRED, false).map_err(internal)?;
            return Err(SessionError::ReauthRequired);
        };
        let plain = self.key.decrypt(&secret, &aad(user_id, "credentials")).map_err(internal)?;
        let creds: Credentials = serde_json::from_slice(&plain).map_err(internal)?;
        // The web session only: the session of the application interface is another one,
        // which this login neither ends nor needs to replace.
        client.clear_cookies();
        let outcome = login_attempts(&client, &creds, self.cfg.user_min_interval * 2).await;
        // What is stored is what the client now holds, whatever the outcome: no web session
        // after a failure, and the session token of the application interface, untouched.
        // Written for the link the login was made for, never into the row of a newer one.
        let cookies = self.encrypt_cookies(user_id, &client)?;
        match outcome {
            Ok(_) => {
                self.db.link_update_session(user_id, &link.rev, Some(&cookies), STATUS_LINKED, true).map_err(internal)?;
                self.ensure_app_session(user_id, &client, &link.rev).await;
                Ok(client)
            }
            // The credentials themselves were refused: stop for good.
            Err((e, true)) => {
                self.db.link_update_session(user_id, &link.rev, Some(&cookies), STATUS_REAUTH, false).map_err(internal)?;
                Err(e)
            }
            // A captcha or an outage: the credentials may still be good. Wait before any
            // new attempt instead of repeating it on every tool call.
            Err((e, false)) => {
                let status = if e == SessionError::CaptchaRequired { STATUS_CAPTCHA } else { STATUS_LOGIN_FAILED };
                // Nothing is held against a newer link for what the previous one met.
                if self.db.link_update_session(user_id, &link.rev, Some(&cookies), status, false).map_err(internal)? {
                    self.back_off(user_id, &e);
                }
                Err(e)
            }
        }
    }
}

/// Stores the session of the application interface of a user when a tool call ends, however
/// it ends: see [`Sessions::storing_app_session`].
pub struct StoreAppSession<'a> {
    sessions: &'a Sessions,
    user_id: i64,
}

impl Drop for StoreAppSession<'_> {
    fn drop(&mut self) {
        self.sessions.store_app_session(self.user_id);
    }
}

/// Whether a link, as it was last read, may log in to the application interface at all:
/// credentials are stored, the interface did not refuse them, and the web site is not
/// waiting for a human or for its own pause to end.
fn app_login_possible(link: &crate::db::LinkRow) -> bool {
    link.mode == "credentials" && link.secret.is_some() && !link.app_refused && link.status == STATUS_LINKED
}

impl Sessions {
    fn app_paused(&self, user_id: i64) -> bool {
        let mut pauses = self.app_backoff.lock().unwrap_or_else(|p| p.into_inner());
        let paused = pauses.get(&user_id).is_some_and(|until| std::time::Instant::now() < *until);
        if !paused {
            pauses.remove(&user_id);
        }
        paused
    }

    /// What is left of [`LOGIN_BACKOFF`] since the session of the application interface of
    /// `user_id` was last renewed, when it was renewed that recently: by this process, or
    /// else by an earlier one, as `link` records it.
    fn app_renewed_recently(&self, user_id: i64, link: &crate::db::LinkRow) -> Option<Duration> {
        let here = self.app_renewed.lock().unwrap_or_else(|p| p.into_inner()).get(&user_id).map(|at| at.elapsed());
        // The time of an earlier process is that of the clock. One that lies in the future
        // (the clock was set back since) tells nothing, and holds nothing up.
        let recorded = || link.app_renewed_at.and_then(|at| u64::try_from(crate::db::now() - at).ok()).map(Duration::from_secs);
        LOGIN_BACKOFF.checked_sub(here.or_else(recorded)?).filter(|left| !left.is_zero())
    }

    /// Credentials mode: logs in to the application interface when the user holds no
    /// session of it (the site ended it, or it was never obtained), unless something says
    /// not to. Without a session the tools read the web pages, which is no reason to insist:
    ///
    /// - one login at a time per user: calls made together wait for the first one;
    /// - credentials the interface refused are never sent to it again, until the account is
    ///   linked again (recorded on the link: it holds across a restart);
    /// - any other failure is followed by a pause;
    /// - a session the site ended is renewed once; if the renewed one is lost again within
    ///   the pause, no login until the pause is over (recorded on the link as well: a new
    ///   process does not start with a renewal of its own);
    /// - no login while the web site is waiting for a human, or for its own pause to end:
    ///   the same credentials would be sent where they were just not accepted;
    /// - nothing is sent while the site blocks the address.
    ///
    /// `rev` is the link the caller acts for: nothing is done, kept or recorded for a link
    /// that is no longer the user's.
    async fn ensure_app_session(&self, user_id: i64, client: &ParfumoClient, rev: &str) {
        if client.has_app_session() || self.app_paused(user_id) {
            return;
        }
        let lock = self.app_login.lock().unwrap_or_else(|p| p.into_inner()).entry(user_id).or_default().clone();
        let _one_at_a_time = lock.lock().await;
        // Another call of the same user may have logged in, or met a refusal, while this
        // one waited; the account may have been linked again.
        if client.has_app_session() || self.app_paused(user_id) || client.app_blocked() || self.backing_off(user_id).is_some() {
            return;
        }
        let Ok(Some(link)) = self.db.link_get(user_id) else { return };
        // Whether the link is still the user's, and where the session of the user stands,
        // are read as one step, under the lock a link is replaced or erased under. A link
        // that changes after this instant drops the session, and the login below, decided
        // before that, then keeps no token: not even when the link changes in the short
        // time before the login starts.
        let generation = {
            let revs = self.revs();
            if revs.get(&user_id).is_none_or(|current| current != rev) {
                return;
            }
            client.app_session_generation()
        };
        if link.rev != rev || !app_login_possible(&link) {
            return;
        }
        let Some(secret) = &link.secret else { return };
        let now = std::time::Instant::now();
        // The site ended the session: as this process saw, or as an earlier one recorded.
        let renewal = client.app_session_ended() || link.app_ended;
        if renewal && let Some(left) = self.app_renewed_recently(user_id, &link) {
            // Renewed a moment ago, and lost again: a login accepted and a token refused,
            // call after call, is what gets an address blocked.
            tracing::warn!("the renewed session of the application interface was refused: the web pages are used for a while");
            self.app_backoff.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, now + left);
            return;
        }
        let creds = self.key.decrypt(secret, &aad(user_id, "credentials")).ok().and_then(|plain| serde_json::from_slice::<Credentials>(&plain).ok());
        let Some(creds) = creds else {
            tracing::warn!("the stored credentials cannot be read: no login to the application interface");
            return;
        };
        #[cfg(test)]
        {
            let hook = self.as_the_app_login_leaves.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let outcome = client.app_login_since(&creds.username, &creds.password, generation).await;
        // The link changed or went since the login was decided: the session was dropped
        // with it, so the login kept no token, and its outcome says nothing of the new link.
        if !self.follows(user_id, rev) {
            return;
        }
        match outcome {
            Ok(()) => {
                if renewal {
                    self.app_renewed.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, now);
                    if let Err(e) = self.db.link_set_app_renewed(user_id, rev, Some(crate::db::now())) {
                        // This process still knows; the next one would allow one renewal more.
                        tracing::warn!(error = %e, "the renewal of the session of the application interface could not be recorded");
                    }
                }
                self.store_app_session(user_id);
            }
            Err(ClientError::InvalidCredentials) => {
                tracing::warn!("the application interface refused the credentials: the web pages are used until the account is linked again");
                if let Err(e) = self.db.link_set_app_refused(user_id, rev) {
                    // Not recorded: at least no new attempt for a while.
                    tracing::warn!(error = %e, "the refusal of the application interface could not be stored");
                    self.app_backoff.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, now + LOGIN_BACKOFF);
                }
            }
            Err(e) => {
                // `ClientError` never carries a secret (tested in parfumo-client).
                tracing::warn!(error = %e, "no session of the application interface: the web pages are used instead");
                self.app_backoff.lock().unwrap_or_else(|p| p.into_inner()).insert(user_id, now + LOGIN_BACKOFF);
            }
        }
    }

    /// Before a public read: makes sure that what the user's clients hold of the
    /// application interface is the session of the account linked now, so that the database
    /// tools read through it. The public client still reads web pages without the account's
    /// cookies.
    ///
    /// The web session of the account is neither probed nor touched here: a public read
    /// must not depend on it. In credentials mode a missing session of the application
    /// interface is asked for as [`Self::ensure_app_session`] allows. Without a linked
    /// account there is no token, and the web pages are read.
    pub async fn open_app_session(&self, user_id: i64) {
        // Following the link drops the token of an account that is no longer the linked one.
        let Ok(Some(link)) = self.follow(user_id) else { return };
        // The account client brings the stored session with it, whatever the state of the link.
        let Ok(account) = self.client_for(user_id) else { return };
        if app_login_possible(&link) {
            self.ensure_app_session(user_id, &account, &link.rev).await;
        }
    }

    /// Stores the session of the application interface of `user_id` when it changed since
    /// it was last stored: a token the site refused must not come back from the database
    /// after a restart, and a token just obtained must be found there. Written for the link
    /// this process follows, and only if it is still the user's link.
    pub fn store_app_session(&self, user_id: i64) {
        let limiter = self.limiter(user_id);
        if !limiter.take_app_session_change() {
            return;
        }
        // Under the lock of the links: the client and the `rev` are those of one link.
        let revs = self.revs();
        let account = self.clients.lock().unwrap_or_else(|p| p.into_inner()).get(&user_id).cloned();
        let (Some(rev), Some(account)) = (revs.get(&user_id), account) else { return };
        // With the token goes what became of it: a session the site ended is one the next
        // process must know it renews, not obtains for the first time.
        let stored = self
            .encrypt_cookies(user_id, &account)
            .and_then(|cookies| self.db.link_store_app_session(user_id, rev, &cookies, account.app_session_ended()).map_err(internal));
        if let Err(e) = stored {
            tracing::warn!(error = %e, "the session of the application interface could not be stored");
            // Tried again when the next call ends.
            limiter.flag_app_session_change();
        }
    }

    /// For the length of a tool call: when the value returned is dropped, the session of
    /// the application interface is stored if the call changed it (see
    /// [`Self::store_app_session`]), also when the call is abandoned half-way.
    pub fn storing_app_session(&self, user_id: i64) -> StoreAppSession<'_> {
        StoreAppSession { sessions: self, user_id }
    }

    /// What `hook` does happens at the instant the next login to the application interface
    /// leaves: after the link was checked, before the request.
    #[cfg(test)]
    pub(crate) fn as_the_app_login_leaves(&self, hook: impl FnOnce() + Send + 'static) {
        *self.as_the_app_login_leaves.lock().unwrap() = Some(Box::new(hook));
    }

    /// Ends the pauses of the login to the application interface: the one that follows a
    /// failed login, and the one that follows a renewed session lost at once, for this
    /// process and for the next (tests: time cannot be advanced).
    #[cfg(any(test, feature = "test-util"))]
    pub fn end_app_backoff(&self, user_id: i64) {
        self.app_backoff.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        self.app_renewed.lock().unwrap_or_else(|p| p.into_inner()).remove(&user_id);
        if let Ok(Some(link)) = self.db.link_get(user_id) {
            let _ = self.db.link_set_app_renewed(user_id, &link.rev, None);
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
    /// refused (its web site, or its application interface) are not sent again to the one
    /// that refused them until they change. A different account replaces the
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
        let client = match account {
            LocalAccount::Credentials(_) => None,
            LocalAccount::Cookie(header) => {
                let client = self.new_client(user_id)?;
                if client.import_cookie_header(header) == 0 {
                    return Err(SessionError::Internal(format!(
                        "{ENV_LOCAL_COOKIE} holds no cookie (expected `pf_auth=value`, or a whole `Cookie` header)"
                    )));
                }
                Some(client)
            }
        };
        self.store_link(user_id, mode, Some(&secret), client.as_ref(), STATUS_LINKED, false)?;
        self.db.link_mark_from_env(user_id).map_err(internal)?;
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
