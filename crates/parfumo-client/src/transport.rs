//! HTTP transport: fixed honest User-Agent, per-client and global rate limits with
//! random jitter, per-client cookie jar, exponential backoff, challenge detection,
//! response cache hook. No HTML knowledge here except what `site.rs` exposes.
//!
//! Every request of either interface leaves through [`Transport::send_with`]: it is where
//! the rules of the application interface (`/app/v1`) hold for every caller. Nothing goes
//! there without the member's token, except the login; the token is the one held when the
//! request leaves, not when it was queued; nothing goes there while the site blocks the
//! address.

use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use reqwest::{
    Method, Url,
    header::{HeaderValue, LOCATION, RETRY_AFTER, SET_COOKIE},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::{
    app::{self, AppSession},
    error::ClientError,
    parse,
    site::{markers, routes},
};

/// Client configuration. `Default` targets parfumo.com at one request per second.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Base URL of the site (`PARFUMO_BASE_URL`), default `https://www.parfumo.com`.
    pub base_url: String,
    /// Fixed, honest User-Agent. Never rotated, never a browser imitation.
    pub user_agent: String,
    /// Minimum delay between two requests of one client (default 1 s).
    pub min_interval: Duration,
    /// Upper bound of the random delay added to `min_interval` (default 400 ms).
    pub jitter: Duration,
    /// Retries on HTTP 429 / 5xx and network errors, with exponential backoff (default 3).
    pub max_retries: u32,
    /// Per-request timeout (default 30 s).
    pub timeout: Duration,
    /// First backoff delay; doubled at each retry (default 2 s).
    pub backoff_base: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base_url: "https://www.parfumo.com".into(),
            user_agent: "parfumo-mcp/0.1".into(),
            min_interval: Duration::from_secs(1),
            jitter: Duration::from_millis(400),
            max_retries: 3,
            timeout: Duration::from_secs(30),
            backoff_base: Duration::from_secs(2),
        }
    }
}

/// Rate limiter shared by every client of the process (all users together).
///
/// It also remembers that the site blocked the address of the process: a block is per
/// address, so every client of every user must know of it at once.
pub struct GlobalLimiter {
    inner: Option<DefaultDirectRateLimiter>,
    /// Until when the site blocks this address, as far as its block page told.
    blocked_until: Mutex<Option<Instant>>,
}

/// Whole seconds from `now` to `end`, rounded up: a wait announced is never too short.
fn secs_left(end: Instant, now: Instant) -> u64 {
    let left = end.saturating_duration_since(now);
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

impl GlobalLimiter {
    /// At most `per_second` requests per second in total. `0` disables the limit.
    pub fn new(per_second: u32) -> Arc<Self> {
        let inner = NonZeroU32::new(per_second).map(|n| RateLimiter::direct(Quota::per_second(n).allow_burst(NonZeroU32::MIN)));
        Arc::new(Self { inner, blocked_until: Mutex::new(None) })
    }

    pub(crate) async fn acquire(&self) {
        if let Some(limiter) = &self.inner {
            limiter.until_ready().await;
        }
    }

    /// The block page was just met: the address is blocked for [`app::BLOCK`] from the
    /// first time the page was shown. Returns the seconds left to wait.
    pub(crate) fn block(&self) -> u64 {
        self.block_at(Instant::now())
    }

    fn block_at(&self, now: Instant) -> u64 {
        let mut until = self.blocked_until.lock().unwrap_or_else(|e| e.into_inner());
        // A block already known keeps its end: the page met again during it (on a web page,
        // while the interface is left alone) says how long is left, not five minutes more.
        let end = until.filter(|end| *end > now).unwrap_or(now + app::BLOCK);
        *until = Some(end);
        secs_left(end, now)
    }

    /// Seconds left of a block of the address, when one is known and not over.
    pub(crate) fn blocked_secs(&self) -> Option<u64> {
        self.blocked_secs_at(Instant::now())
    }

    fn blocked_secs_at(&self, now: Instant) -> Option<u64> {
        let end = (*self.blocked_until.lock().unwrap_or_else(|e| e.into_inner()))?;
        (end > now).then(|| secs_left(end, now))
    }
}

/// What the clients of one user share: their request slot (minimum interval + jitter), so
/// that together they stay under one request per second, and the session of the
/// application interface, which has one owner per user. Give the same one to the public
/// client and the account client of a user: a token the site refuses to one of them is
/// then gone for both at once.
#[derive(Default)]
pub struct UserLimiter {
    next_slot: Arc<tokio::sync::Mutex<Option<Instant>>>,
    /// The session token of the application interface of this user, and what became of it.
    pub(crate) app: AppSession,
    /// One call made as the member at a time, from the moment its token is read until its
    /// answer is known: a call that waited behind a refusal finds the token gone.
    app_turn: tokio::sync::Mutex<()>,
}

impl UserLimiter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A limiter on the same request slot, without the session of the application
    /// interface: for a client of the user that must read the web pages whatever the
    /// account holds (the canary checks the HTML).
    pub fn web_only(&self) -> Arc<Self> {
        Arc::new(Self { next_slot: self.next_slot.clone(), ..Self::default() })
    }

    /// Forgets the session token without telling the site, for every client of the user.
    /// A login still under way when this is called does not bring a session back.
    pub fn forget_app_session(&self) {
        self.app.forget();
    }

    /// Whether the session token changed (a login obtained one, or the site refused it)
    /// since this was last asked: the embedding application then stores the session again.
    /// Asking clears the mark.
    pub fn take_app_session_change(&self) -> bool {
        self.app.take_change()
    }

    /// Puts the mark of [`Self::take_app_session_change`] back, when the session could not
    /// be stored after all.
    pub fn flag_app_session_change(&self) {
        self.app.flag_change();
    }
}

/// Cache of public responses, implemented by the embedding application.
pub trait PageCache: Send + Sync {
    /// Body stored under `key`, if present and not expired.
    fn get(&self, key: &str) -> Option<String>;
    /// Stores `body` under `key` for `ttl`.
    fn put(&self, key: &str, body: &str, ttl: Duration);
}

/// A cookie of the Parfumo session, as persisted by the embedding application.
/// The `Debug` output never shows the value.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct StoredCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
}

impl std::fmt::Debug for StoredCookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredCookie")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .finish()
    }
}

/// Minimal cookie jar: one per client, enumerable so that sessions can be persisted.
///
/// The session token of the application interface is saved and restored with the cookies,
/// under a name and a domain of its own, but it is not kept here: it belongs to the user
/// ([`UserLimiter`]), not to one client, and it is never sent as a cookie.
pub(crate) struct Jar {
    cookies: Mutex<Vec<StoredCookie>>,
    user: Arc<UserLimiter>,
}

fn domain_matches(host: &str, domain: &str) -> bool {
    let domain = domain.trim_start_matches('.');
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn current_year() -> u64 {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    1970 + secs / 31_556_952
}

impl Jar {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<StoredCookie>> {
        self.cookies.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn new(user: Arc<UserLimiter>) -> Self {
        Self { cookies: Mutex::default(), user }
    }

    /// The cookies of the web session, and the session token of the application interface
    /// when one is held.
    pub(crate) fn export(&self) -> Vec<StoredCookie> {
        let mut all = self.lock().clone();
        all.extend(self.user.app.stored());
        all
    }

    /// Forgets the web session. The session of the application interface is another one,
    /// which a new login to the web site does not end: its token stays.
    pub(crate) fn clear(&self) {
        self.lock().clear();
    }

    pub(crate) fn upsert(&self, cookie: StoredCookie) {
        if AppSession::is_stored(&cookie) {
            self.user.app.restore(&cookie.value);
            return;
        }
        let mut all = self.lock();
        all.retain(|c| !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path));
        all.push(cookie);
    }

    fn remove(&self, name: &str, domain: &str) {
        self.lock().retain(|c| !(c.name == name && domain_matches(domain, &c.domain)));
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        self.lock().iter().any(|c| c.name == name && !c.value.is_empty())
    }

    fn store_header(&self, header: &str, host: &str) {
        let mut parts = header.split(';');
        let Some((name, value)) = parts.next().and_then(|nv| nv.split_once('=')) else { return };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() {
            return;
        }
        let mut domain = host.to_string();
        let mut path = "/".to_string();
        let mut expired = value.is_empty() || value == "deleted";
        for part in parts {
            let (key, val) = match part.split_once('=') {
                Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim()),
                None => (part.trim().to_ascii_lowercase(), ""),
            };
            match key.as_str() {
                "domain" if !val.is_empty() => {
                    let d = val.trim_start_matches('.').to_ascii_lowercase();
                    // Ignore cookies set for a foreign domain.
                    if domain_matches(host, &d) {
                        domain = d;
                    } else {
                        return;
                    }
                }
                "path" if val.starts_with('/') => path = val.to_string(),
                "max-age" => {
                    if val.parse::<i64>().is_ok_and(|n| n <= 0) {
                        expired = true;
                    }
                }
                "expires" => {
                    let year = val.split(|c: char| !c.is_ascii_digit()).find(|t| t.len() == 4).and_then(|t| t.parse::<u64>().ok());
                    if year.is_some_and(|y| y < current_year()) {
                        expired = true;
                    }
                }
                _ => {}
            }
        }
        if expired {
            self.remove(name, &domain);
        } else {
            self.upsert(StoredCookie { name: name.to_string(), value: value.to_string(), domain, path });
        }
    }
}

impl reqwest::cookie::CookieStore for Jar {
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let Some(host) = url.host_str() else { return };
        for header in cookie_headers {
            if let Ok(text) = header.to_str() {
                self.store_header(text, host);
            }
        }
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        let host = url.host_str()?;
        let path = url.path();
        let line = self
            .lock()
            .iter()
            .filter(|c| domain_matches(host, &c.domain) && path.starts_with(c.path.as_str()))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");
        if line.is_empty() { None } else { HeaderValue::from_str(&line).ok() }
    }
}

/// A raw HTTP answer, before any interpretation.
pub(crate) struct RawResponse {
    pub status: u16,
    pub body: String,
    pub location: Option<String>,
}

/// Which interface a request is for, and what it carries.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Auth {
    /// A page or an internal call of the web site: the cookies of the jar.
    Web,
    /// The login of the application interface: the one call that leaves without a token.
    Login,
    /// A call of the application interface made as the member: the session token the user
    /// holds when the request leaves.
    Member,
}

/// Why an attempt gave no answer.
enum Unsent {
    /// Nothing was sent: the request may not leave.
    Refused(ClientError),
    Network(reqwest::Error),
}

/// A path as a server reads it: percent-decoded, lower case, without doubled slashes.
fn plain_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| path.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        decoded.push(hex.unwrap_or(bytes[i]));
        i += if hex.is_some() { 3 } else { 1 };
    }
    let mut plain = String::with_capacity(decoded.len());
    for c in String::from_utf8_lossy(&decoded).to_lowercase().chars() {
        if c != '/' || !plain.ends_with('/') {
            plain.push(c);
        }
    }
    plain
}

pub(crate) struct Transport {
    pub cfg: ClientConfig,
    pub base: Url,
    http: reqwest::Client,
    pub jar: Arc<Jar>,
    global: Arc<GlobalLimiter>,
    cache: Option<Arc<dyn PageCache>>,
    pub(crate) user: Arc<UserLimiter>,
    csrf: Mutex<Option<String>>,
    /// Nickname of the logged-in member, memoised with the session (see `forget_csrf`).
    pub(crate) nick: Mutex<Option<String>>,
    // The three memories below hold identifiers only (a name and the number the application
    // interface knows it by), never a part of an answer. Each is bounded where it is filled:
    // a server runs for weeks.
    /// Number and `Brand/Perfume` id of the perfumes met in listings and records, by
    /// lower-case id: the application interface only knows perfumes by number. One id per
    /// number, the one the site gave last: a renamed perfume loses its old address.
    pub(crate) perfume_ids: Mutex<std::collections::HashMap<String, (u64, String)>>,
    /// Number of the brands already looked up, by reference; `None` for a reference the
    /// search of the application does not know. A listing is read page after page: its
    /// brand is looked up once.
    pub(crate) brand_ids: Mutex<std::collections::HashMap<String, Option<u64>>>,
    /// Number and lower-case name of the perfumers met in perfume records, by lower-case slug:
    /// the application interface only knows perfumers by number, and has no call that finds one.
    pub(crate) perfumer_ids: Mutex<std::collections::HashMap<String, (u64, String)>>,
}

/// TLS settings of every outgoing connection of the project: the `ring` provider and the
/// bundled web PKI roots, so that no binary depends on a system TLS library or a C compiler
/// for a second crypto backend.
pub fn tls_config() -> rustls::ClientConfig {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

impl Transport {
    pub fn new(
        cfg: ClientConfig,
        global: Arc<GlobalLimiter>,
        cache: Option<Arc<dyn PageCache>>,
        limiter: Arc<UserLimiter>,
    ) -> Result<Self, ClientError> {
        let base = Url::parse(cfg.base_url.trim_end_matches('/')).map_err(|_| ClientError::InvalidInput("base_url is not a valid URL".into()))?;
        if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
            return Err(ClientError::InvalidInput("base_url must be an http(s) URL with a host".into()));
        }
        let jar = Arc::new(Jar::new(limiter.clone()));
        let http = reqwest::Client::builder()
            .user_agent(cfg.user_agent.clone())
            .redirect(Policy::none())
            .cookie_provider(jar.clone())
            .timeout(cfg.timeout)
            .use_preconfigured_tls(tls_config())
            .build()
            .map_err(|e| ClientError::Upstream(format!("cannot build HTTP client: {}", e.without_url())))?;
        Ok(Self {
            cfg,
            base,
            http,
            jar,
            global,
            cache,
            user: limiter,
            csrf: Mutex::new(None),
            nick: Mutex::new(None),
            perfume_ids: Mutex::new(std::collections::HashMap::new()),
            brand_ids: Mutex::new(std::collections::HashMap::new()),
            perfumer_ids: Mutex::new(std::collections::HashMap::new()),
        })
    }

    pub fn host(&self) -> &str {
        self.base.host_str().unwrap_or_default()
    }

    /// Domain used for cookies imported by hand: the base host without `www.`.
    pub fn cookie_domain(&self) -> String {
        self.host().trim_start_matches("www.").to_string()
    }

    pub fn absolute(&self, path_and_query: &str) -> String {
        format!("{}{}", self.base.as_str().trim_end_matches('/'), path_and_query)
    }

    pub fn forget_csrf(&self) {
        *self.csrf.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.nick.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Waits for this client's slot (min interval + random jitter), then for the global
    /// limiter, then asks `clear` whether the request may still leave: what was true when
    /// it was queued (a token held, no block of the address) may no longer be. A request
    /// that does not leave takes no slot.
    async fn throttle<T>(&self, clear: impl Fn() -> Result<T, ClientError>) -> Result<T, ClientError> {
        // No reason to wait for a slot that would not be used.
        clear()?;
        let mut slot = self.user.next_slot.lock().await;
        if let Some(at) = *slot {
            let jitter_ms = self.cfg.jitter.as_millis() as u64;
            let jitter = if jitter_ms == 0 { 0 } else { rand::random_range(0..=jitter_ms) };
            tokio::time::sleep_until(at + Duration::from_millis(jitter)).await;
        }
        self.global.acquire().await;
        let cleared = clear()?;
        *slot = Some(Instant::now() + self.cfg.min_interval);
        Ok(cleared)
    }

    /// The route of the application interface a request leads to, as the site would read
    /// its address (dot segments, case, percent-encoding and doubled slashes resolved):
    /// `Some("/auth/login")`. `None` for a page of the web site.
    pub(crate) fn app_route(&self, path_and_query: &str) -> Option<String> {
        let url = Url::parse(&self.absolute(path_and_query)).ok()?;
        let path = plain_path(url.path());
        let base = plain_path(self.base.path());
        let route = path.strip_prefix(base.trim_end_matches('/'))?.strip_prefix(app::PREFIX)?;
        (route.is_empty() || route.starts_with('/')).then(|| route.to_string())
    }

    /// Refuses, before anything is sent or waited for, a request that would break the
    /// rules of the application interface, whoever asks for it: a request without the
    /// member's token under `/app/v1` (a page read, a redirect followed), other than the
    /// login; or the member's token on its way to a page of the web site.
    fn check_route(&self, path_and_query: &str, auth: Auth) -> Result<(), ClientError> {
        match (auth, self.app_route(path_and_query).as_deref()) {
            (Auth::Web, None) | (Auth::Member, Some(_)) | (Auth::Login, Some(app::LOGIN)) => Ok(()),
            (Auth::Web, Some(_)) => Err(ClientError::InvalidInput("the application interface of Parfumo is not read as a page".into())),
            _ => Err(ClientError::InvalidInput("not a call of the application interface of Parfumo".into())),
        }
    }

    /// Seconds left of a block of the address of this process, when one is known.
    pub(crate) fn blocked_secs(&self) -> Option<u64> {
        self.global.blocked_secs()
    }

    /// What a request leaves with, decided when it leaves: the member's token for a call
    /// made as the member, nothing otherwise. Nothing leaves for the application interface
    /// while the site blocks the address, nor as the member without a token.
    fn clearance(&self, auth: Auth) -> Result<Option<String>, ClientError> {
        if auth == Auth::Web {
            return Ok(None);
        }
        if let Some(secs) = self.blocked_secs() {
            return Err(ClientError::RateLimited { retry_after_secs: Some(secs) });
        }
        match auth {
            Auth::Member => self.user.app.token().map(Some).ok_or(ClientError::LoginRequired),
            _ => Ok(None),
        }
    }

    async fn send_once(
        &self,
        method: &Method,
        path_and_query: &str,
        form: Option<&[(String, String)]>,
        auth: Auth,
    ) -> Result<(RawResponse, Option<u64>, bool), Unsent> {
        // Held until the answer is known: the next call made as the member reads the token
        // only then, and finds none if this one was refused, however slow the refusal.
        let _turn = match auth {
            Auth::Member => Some(self.user.app_turn.lock().await),
            _ => None,
        };
        let bearer = self.throttle(|| self.clearance(auth)).await.map_err(Unsent::Refused)?;
        let url = self.absolute(path_and_query);
        let mut req = self.http.request(method.clone(), url);
        if let Some(token) = &bearer {
            req = req.bearer_auth(token);
        }
        if let Some(form) = form {
            req = req.form(form);
        }
        let resp = req.send().await.map_err(Unsent::Network)?;
        let status = resp.status().as_u16();
        if status == 401
            && let Some(token) = &bearer
        {
            // Before anything else can fail: the token that was sent, and only that one, is
            // forgotten for every client of the user, and is never held again.
            self.user.app.refuse(token);
        }
        let headers = resp.headers();
        let location = headers.get(LOCATION).and_then(|v| v.to_str().ok()).map(str::to_string);
        let retry_after = headers.get(RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let challenge_header = headers.contains_key(markers::CHALLENGE_HEADER);
        // Make sure Set-Cookie headers of redirects are seen even if the store hook changes.
        let _ = headers.get(SET_COOKIE);
        let body = resp.text().await.map_err(Unsent::Network)?;
        Ok((RawResponse { status, body, location }, retry_after, challenge_header))
    }

    /// Sends a request to the web site with backoff on 429 / 5xx / network errors. Returns
    /// 2xx, 3xx and 403 / 404 answers raw; challenges, the block page and exhausted retries
    /// become errors.
    pub async fn send(
        &self,
        method: Method,
        path_and_query: &str,
        form: Option<&[(String, String)]>,
        max_retries: u32,
    ) -> Result<RawResponse, ClientError> {
        self.send_with(method, path_and_query, form, max_retries, Auth::Web).await
    }

    /// [`Self::send`] for either interface: `auth` tells which, and what the request
    /// carries. The only way out of the process, for every caller.
    pub async fn send_with(
        &self,
        method: Method,
        path_and_query: &str,
        form: Option<&[(String, String)]>,
        max_retries: u32,
        auth: Auth,
    ) -> Result<RawResponse, ClientError> {
        self.check_route(path_and_query, auth)?;
        let path = parse::url_path(path_and_query).to_string();
        let mut attempt = 0u32;
        loop {
            let outcome = self.send_once(&method, path_and_query, form, auth).await;
            let (failure, retry_after) = match outcome {
                Ok((raw, retry_after, challenge_header)) => {
                    let status = raw.status;
                    if challenge_header || (status >= 400 && parse::is_challenge(&raw.body)) {
                        return Err(ClientError::CaptchaRequired);
                    }
                    // The block page, on either interface: it is not known whether a block
                    // covers the web pages too, so it is recognised wherever it is shown.
                    if app::is_block_page(status, &raw.body, auth != Auth::Web) {
                        return Err(ClientError::RateLimited { retry_after_secs: Some(self.global.block()) });
                    }
                    if status == 429 {
                        (ClientError::RateLimited { retry_after_secs: retry_after }, retry_after)
                    } else if status >= 500 {
                        (ClientError::Upstream(format!("HTTP {status} on {path}")), retry_after)
                    } else {
                        return Ok(raw);
                    }
                }
                // Nothing was sent, and trying again would not change that.
                Err(Unsent::Refused(e)) => return Err(e),
                Err(Unsent::Network(e)) => {
                    let kind = if e.is_timeout() {
                        "timeout"
                    } else if e.is_connect() {
                        "connection failed"
                    } else {
                        "network error"
                    };
                    (ClientError::Upstream(format!("{kind} on {path}")), None)
                }
            };
            if attempt >= max_retries {
                return Err(failure);
            }
            let backoff = self.cfg.backoff_base.saturating_mul(1u32 << attempt.min(8));
            let wait = match retry_after {
                // A long Retry-After is reported to the caller instead of blocking the tool call.
                Some(secs) if secs > 60 => return Err(ClientError::RateLimited { retry_after_secs: Some(secs) }),
                Some(secs) => backoff.max(Duration::from_secs(secs)),
                None => backoff,
            };
            tokio::time::sleep(wait).await;
            attempt += 1;
        }
    }

    fn cache_key(method: &str, url: &str, form: &[(String, String)]) -> String {
        let mut pairs: Vec<&(String, String)> = form.iter().filter(|(k, _)| k != "csrf_key").collect();
        pairs.sort();
        let body = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
        format!("{method} {url} {body}")
    }

    fn cache_get(&self, key: &str, ttl: Option<Duration>) -> Option<String> {
        ttl?;
        self.cache.as_ref()?.get(key).filter(|b| !b.trim().is_empty())
    }

    fn cache_put(&self, key: &str, body: &str, ttl: Option<Duration>) {
        if let (Some(cache), Some(ttl)) = (&self.cache, ttl)
            && !body.trim().is_empty()
        {
            cache.put(key, body, ttl);
        }
    }

    fn status_error(status: u16, path: &str) -> ClientError {
        match status {
            404 | 410 => ClientError::NotFound(path.to_string()),
            401 => ClientError::LoginRequired,
            _ => ClientError::Upstream(format!("HTTP {status} on {path}")),
        }
    }

    /// `GET` of a page or fragment. Follows same-host redirects by hand (each hop is
    /// rate limited); a redirect to the login page is `LoginRequired`. Returns the final
    /// path and the body. `ttl = None` disables the cache; `fresh` skips the cache read.
    pub async fn get(&self, path_and_query: &str, ttl: Option<Duration>, fresh: bool, allow_empty: bool) -> Result<(String, String), ClientError> {
        self.get_checked(path_and_query, ttl, fresh, allow_empty, &|_| Ok(())).await
    }

    /// [`Self::get`] with a guard on redirects: `hop_ok` sees the path of every redirect
    /// target before it is requested, and its error ends the read without that request.
    pub async fn get_checked(
        &self,
        path_and_query: &str,
        ttl: Option<Duration>,
        fresh: bool,
        allow_empty: bool,
        hop_ok: &(dyn Fn(&str) -> Result<(), ClientError> + Sync),
    ) -> Result<(String, String), ClientError> {
        let key = Self::cache_key("GET", &self.absolute(path_and_query), &[]);
        if !fresh && let Some(body) = self.cache_get(&key, ttl) {
            return Ok((path_and_query.to_string(), body));
        }
        let mut current = path_and_query.to_string();
        for _ in 0..5 {
            let raw = self.send(Method::GET, &current, None, self.cfg.max_retries).await?;
            let path = parse::url_path(&current).to_string();
            match raw.status {
                200..=299 => {
                    if raw.body.trim().is_empty() && !allow_empty {
                        return Err(ClientError::Upstream(format!("empty body on {path}")));
                    }
                    if parse::is_not_found_page(&raw.body) {
                        return Err(ClientError::NotFound(path));
                    }
                    self.cache_put(&key, &raw.body, ttl);
                    return Ok((current, raw.body));
                }
                300..=399 => {
                    let location = raw.location.ok_or_else(|| ClientError::Upstream(format!("redirect without Location on {path}")))?;
                    if location.contains(routes::LOGIN_REDIRECT_MARKER) {
                        return Err(ClientError::LoginRequired);
                    }
                    current = self.same_host_path(&location, &path)?;
                    hop_ok(parse::url_path(&current))?;
                }
                status => return Err(Self::status_error(status, &path)),
            }
        }
        Err(ClientError::Upstream(format!("too many redirects from {}", parse::url_path(path_and_query))))
    }

    /// Resolves a `Location` header to a path on the base host, or fails.
    pub fn same_host_path(&self, location: &str, from_path: &str) -> Result<String, ClientError> {
        let target = self
            .base
            .join(from_path)
            .and_then(|u| u.join(location))
            .map_err(|_| ClientError::Upstream(format!("invalid redirect from {from_path}")))?;
        if target.host_str() != self.base.host_str() || target.scheme() != self.base.scheme() {
            return Err(ClientError::Upstream(format!("redirect to another host from {from_path}")));
        }
        Ok(match target.query() {
            Some(q) => format!("{}?{q}", target.path()),
            None => target.path().to_string(),
        })
    }

    /// Session-bound `csrf_key` from `GET /api/alive`, memoised until the session changes.
    pub async fn csrf_key(&self) -> Result<String, ClientError> {
        if let Some(key) = self.csrf.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(key);
        }
        let raw = self.send(Method::GET, routes::ALIVE, None, self.cfg.max_retries).await?;
        if raw.status != 200 {
            return Err(Self::status_error(raw.status, routes::ALIVE));
        }
        let key = parse::parse_alive(&raw.body)?;
        *self.csrf.lock().unwrap_or_else(|e| e.into_inner()) = Some(key.clone());
        Ok(key)
    }

    /// `POST` of an internal `/action/**` call returning an HTML fragment.
    ///
    /// `with_csrf` appends the session `csrf_key`. A `200` with an empty body is how the
    /// site signals an invalid token, a missing session or a wrong method: when a
    /// `csrf_key` is involved it is renewed and the call replayed once. The body returned
    /// may still be empty; callers decide whether that is an error (it almost always is).
    pub async fn post_fragment(&self, path: &str, form: &[(String, String)], with_csrf: bool, ttl: Option<Duration>) -> Result<String, ClientError> {
        let key = Self::cache_key("POST", &self.absolute(path), form);
        if let Some(body) = self.cache_get(&key, ttl) {
            return Ok(body);
        }
        let mut replayed = false;
        loop {
            let mut full: Vec<(String, String)> = form.to_vec();
            if with_csrf {
                full.push(("csrf_key".into(), self.csrf_key().await?));
            }
            let raw = self.send(Method::POST, path, Some(&full), self.cfg.max_retries).await?;
            let stale_csrf = match raw.status {
                200..=299 => raw.body.trim().is_empty() && with_csrf,
                403 if raw.body.contains(markers::INVALID_CSRF) => true,
                300..=399 => {
                    return Err(if raw.location.is_some_and(|l| l.contains(routes::LOGIN_REDIRECT_MARKER)) {
                        ClientError::LoginRequired
                    } else {
                        ClientError::Upstream(format!("unexpected redirect on {path}"))
                    });
                }
                status => return Err(Self::status_error(status, path)),
            };
            if stale_csrf && !replayed {
                replayed = true;
                self.forget_csrf();
                continue;
            }
            if raw.status == 403 {
                return Err(ClientError::Upstream(format!("CSRF token rejected twice on {path}")));
            }
            self.cache_put(&key, &raw.body, ttl);
            return Ok(raw.body);
        }
    }
}

impl Transport {
    /// `POST` of an account write (`/api/*` or `/action/**`), with the session `csrf_key`.
    ///
    /// Never retried on 429 / 5xx / network errors: a write must not be sent twice by
    /// accident. The only replay is the one the site's own script does: after a `403
    /// Invalid CSRF token` (or, when `expect_body`, after the empty `200` an `/action/**`
    /// route answers to a stale token) the key is renewed and the call sent once more.
    /// `401`, or a redirect to the login page, is `LoginRequired`.
    pub async fn post_write(&self, path: &str, form: &[(String, String)], expect_body: bool) -> Result<String, ClientError> {
        let mut replayed = false;
        loop {
            let mut full: Vec<(String, String)> = form.to_vec();
            full.push(("csrf_key".into(), self.csrf_key().await?));
            let raw = self.send(Method::POST, path, Some(&full), 0).await?;
            let stale_csrf = match raw.status {
                200..=299 => expect_body && raw.body.trim().is_empty(),
                403 if raw.body.contains(markers::INVALID_CSRF) => true,
                300..=399 => {
                    return Err(if raw.location.is_some_and(|l| l.contains(routes::LOGIN_REDIRECT_MARKER)) {
                        ClientError::LoginRequired
                    } else {
                        ClientError::Upstream(format!("unexpected redirect on {path}"))
                    });
                }
                status => return Err(Self::status_error(status, path)),
            };
            if stale_csrf && !replayed {
                replayed = true;
                self.forget_csrf();
                continue;
            }
            if raw.status == 403 {
                return Err(ClientError::Upstream(format!("CSRF token rejected twice on {path}")));
            }
            if stale_csrf {
                return Err(empty_body(path));
            }
            return Ok(raw.body);
        }
    }
}

impl Transport {
    /// `POST` of a classic HTML form of the account, which answers by a redirect rather
    /// than by a body. Returns the path the site redirects to (`None` when it answered
    /// 200) and the body. Never retried, except once after `403 Invalid CSRF token`.
    pub async fn post_form(&self, path: &str, form: &[(String, String)]) -> Result<(Option<String>, String), ClientError> {
        let mut replayed = false;
        loop {
            let mut full: Vec<(String, String)> = form.to_vec();
            full.push(("csrf_key".into(), self.csrf_key().await?));
            let raw = self.send(Method::POST, path, Some(&full), 0).await?;
            match raw.status {
                200..=299 => return Ok((None, raw.body)),
                300..=399 => {
                    let location = raw.location.unwrap_or_default();
                    if location.contains(routes::LOGIN_REDIRECT_MARKER) {
                        return Err(ClientError::LoginRequired);
                    }
                    return Ok((Some(self.same_host_path(&location, path)?), raw.body));
                }
                403 if raw.body.contains(markers::INVALID_CSRF) && !replayed => {
                    replayed = true;
                    self.forget_csrf();
                }
                403 if raw.body.contains(markers::INVALID_CSRF) => return Err(ClientError::Upstream(format!("CSRF token rejected twice on {path}"))),
                status => return Err(Self::status_error(status, path)),
            }
        }
    }
}

/// Error for an internal call that answered `200` with an empty body.
pub(crate) fn empty_body(path: &str) -> ClientError {
    ClientError::Upstream(format!("{path} answered 200 with an empty body (invalid or stale token, expired session, or changed route)"))
}

/// Raw answer of an exploration request (jalon 0 bis): status, every header, body bytes.
#[cfg(feature = "explore")]
pub struct ExploreResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Exploration hook, compiled only with the `explore` feature (never in the server).
/// It reuses the production throttle, cookie jar, User-Agent and challenge detection,
/// but returns the answer uninterpreted and never retries.
#[cfg(feature = "explore")]
impl Transport {
    pub async fn explore(
        &self,
        method: Method,
        path_and_query: &str,
        form: Option<&[(String, String)]>,
        raw_body: Option<(String, Vec<u8>)>,
        extra_headers: &[(String, String)],
    ) -> Result<ExploreResponse, ClientError> {
        // The rule of the application interface holds here too: only the login leaves
        // without an `Authorization` header.
        let bare = !extra_headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("authorization"));
        if bare && self.app_route(path_and_query).is_some_and(|route| route != app::LOGIN) {
            return Err(ClientError::InvalidInput("the application interface of Parfumo is not asked without a session token".into()));
        }
        self.throttle(|| Ok(())).await?;
        let mut req = self.http.request(method, self.absolute(path_and_query));
        for (name, value) in extra_headers {
            req = req.header(name.as_str(), value.as_str());
        }
        if let Some(form) = form {
            req = req.form(form);
        }
        if let Some((content_type, bytes)) = raw_body {
            req = req.header(reqwest::header::CONTENT_TYPE, content_type).body(bytes);
        }
        let path = parse::url_path(path_and_query).to_string();
        let resp = req.send().await.map_err(|e| ClientError::Upstream(format!("network error on {path}: {}", e.without_url())))?;
        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> =
            resp.headers().iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
        let body = resp.bytes().await.map_err(|e| ClientError::Upstream(format!("body read failed on {path}: {}", e.without_url())))?.to_vec();
        let challenge =
            headers.iter().any(|(k, _)| k == markers::CHALLENGE_HEADER) || (status >= 400 && parse::is_challenge(&String::from_utf8_lossy(&body)));
        if challenge {
            return Err(ClientError::CaptchaRequired);
        }
        Ok(ExploreResponse { status, headers, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transport(base_url: &str) -> Transport {
        let cfg = ClientConfig { base_url: base_url.into(), ..ClientConfig::default() };
        Transport::new(cfg, GlobalLimiter::new(0), None, UserLimiter::new()).unwrap()
    }

    #[test]
    fn a_path_of_the_application_interface_is_recognised_however_it_is_written() {
        let t = transport("https://www.parfumo.com");
        for path in [
            "/app/v1",
            "/app/v1/auth/me",
            "/app/v1/perfumes?page=1",
            "/APP/V1/auth/me",
            "/%61pp/v1/auth/me",
            "/app%2Fv1/auth/me",
            "//app//v1/auth/me",
            "/Perfumes/../app/v1/auth/me",
            "/Perfumes/%2e%2e/app/v1/auth/me",
        ] {
            assert!(t.app_route(path).is_some(), "{path}");
        }
        assert_eq!(t.app_route("/app/v1/auth/login").as_deref(), Some(app::LOGIN));
        assert_eq!(t.app_route("/APP/v1//Auth/Login?x=1").as_deref(), Some(app::LOGIN));
        for path in ["/", "/Perfumes/Dior/Sauvage_Eau_de_Toilette", "/app", "/app/", "/app/v10/x", "/app_v5/api", "/apps/v1", "/Users/app/v1"] {
            assert_eq!(t.app_route(path), None, "{path}");
        }
        // A base address with a path of its own: the interface is under it.
        let t = transport("http://127.0.0.1:1/site");
        assert_eq!(t.app_route("/app/v1/auth/me").as_deref(), Some("/auth/me"));
    }

    #[tokio::test]
    async fn nothing_goes_to_the_application_interface_without_the_token_except_the_login() {
        // Port 9 answers nothing: a request that left would end as an upstream error.
        let t = transport("http://127.0.0.1:9");
        for path in ["/app/v1/auth/me", "/APP/v1/perfumes", "/x/../app/v1/auth/logout", "/app/v1/auth/login"] {
            let err = t.send(Method::GET, path, None, 0).await.err();
            assert!(matches!(err, Some(ClientError::InvalidInput(_))), "{path}: {err:?}");
        }
        // The login is the one call without a token, and no other path is a login.
        let err = t.send_with(Method::POST, "/app/v1/auth/me", None, 0, Auth::Login).await.err();
        assert!(matches!(err, Some(ClientError::InvalidInput(_))), "{err:?}");
        let err = t.send_with(Method::POST, "/app/v1/auth/login", None, 0, Auth::Login).await.err();
        assert!(matches!(err, Some(ClientError::Upstream(_))), "the login is sent: {err:?}");
        // The token never leaves for a page of the web site, and no call is made without one.
        t.user.app.restore("tok");
        let err = t.send_with(Method::GET, "/Perfumes/Dior", None, 0, Auth::Member).await.err();
        assert!(matches!(err, Some(ClientError::InvalidInput(_))), "{err:?}");
        t.user.app.forget();
        assert_eq!(t.send_with(Method::GET, "/app/v1/auth/me", None, 0, Auth::Member).await.err(), Some(ClientError::LoginRequired));
    }

    #[test]
    fn a_block_of_the_address_lasts_five_minutes_from_the_first_time_its_page_was_met() {
        let global = GlobalLimiter::new(0);
        let start = Instant::now();
        assert_eq!(global.blocked_secs_at(start), None);
        assert_eq!(global.block_at(start), 300);
        assert_eq!(global.blocked_secs_at(start), Some(300));
        assert_eq!(global.blocked_secs_at(start + Duration::from_millis(100_500)), Some(200), "rounded up: never too short");
        // Met again before its end (on a web page): what is left of it, not five minutes more.
        assert_eq!(global.block_at(start + Duration::from_secs(200)), 100);
        assert_eq!(global.blocked_secs_at(start + Duration::from_secs(299)), Some(1));
        assert_eq!(global.blocked_secs_at(start + Duration::from_secs(300)), None, "over: the interface is asked again");
        // Met once it is over: the site still blocks the address, for five minutes again.
        assert_eq!(global.block_at(start + Duration::from_secs(301)), 300);
        assert_eq!(global.blocked_secs_at(start + Duration::from_secs(600)), Some(1));
        assert_eq!(global.blocked_secs_at(start + Duration::from_secs(601)), None);
    }

    #[test]
    fn the_session_of_a_user_forgets_exactly_the_token_the_site_refused() {
        let user = UserLimiter::new();
        let app = &user.app;
        app.restore("one");
        assert!(!user.take_app_session_change(), "a restored token is the one that is stored");
        // A refusal of the token held: gone, and told to whoever stores the session.
        app.refuse("one");
        assert_eq!(app.token(), None);
        assert!(user.take_app_session_change() && !user.take_app_session_change());
        // It is never held again, wherever it comes back from.
        app.restore("one");
        assert_eq!(app.token(), None);
        // A late refusal of an old token leaves the new one alone.
        app.restore("two");
        app.refuse("one");
        app.refuse("zero");
        assert_eq!(app.token().as_deref(), Some("two"));
        // A limiter for the web pages shares the slot, not the session.
        assert_eq!(user.web_only().app.token(), None);
        assert!(Arc::ptr_eq(&user.web_only().next_slot, &user.next_slot));
        // Dropped by its owner: nothing left to store, and no refusal to renew after.
        user.forget_app_session();
        assert_eq!(app.token(), None);
        assert!(!user.take_app_session_change());
    }
}
