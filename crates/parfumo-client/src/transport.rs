//! HTTP transport: fixed honest User-Agent, per-client and global rate limits with
//! random jitter, per-client cookie jar, exponential backoff, challenge detection,
//! response cache hook. No HTML knowledge here except what `site.rs` exposes.

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
pub struct GlobalLimiter {
    inner: Option<DefaultDirectRateLimiter>,
}

impl GlobalLimiter {
    /// At most `per_second` requests per second in total. `0` disables the limit.
    pub fn new(per_second: u32) -> Arc<Self> {
        let inner = NonZeroU32::new(per_second).map(|n| RateLimiter::direct(Quota::per_second(n).allow_burst(NonZeroU32::MIN)));
        Arc::new(Self { inner })
    }

    pub(crate) async fn acquire(&self) {
        if let Some(limiter) = &self.inner {
            limiter.until_ready().await;
        }
    }
}

/// Per-user request slot (minimum interval + jitter). Share one between the clients of a
/// same user (public and account) so that together they stay under one request per second.
#[derive(Default)]
pub struct UserLimiter {
    next_slot: tokio::sync::Mutex<Option<Instant>>,
}

impl UserLimiter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
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
#[derive(Default)]
pub(crate) struct Jar {
    cookies: Mutex<Vec<StoredCookie>>,
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

    pub(crate) fn export(&self) -> Vec<StoredCookie> {
        self.lock().clone()
    }

    pub(crate) fn clear(&self) {
        self.lock().clear();
    }

    pub(crate) fn upsert(&self, cookie: StoredCookie) {
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

    /// The value stored under exactly this name and domain.
    pub(crate) fn value(&self, name: &str, domain: &str) -> Option<String> {
        self.lock().iter().find(|c| c.name == name && c.domain == domain).map(|c| c.value.clone()).filter(|v| !v.is_empty())
    }

    pub(crate) fn forget(&self, name: &str, domain: &str) {
        self.lock().retain(|c| !(c.name == name && c.domain == domain));
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

pub(crate) struct Transport {
    pub cfg: ClientConfig,
    pub base: Url,
    http: reqwest::Client,
    pub jar: Arc<Jar>,
    global: Arc<GlobalLimiter>,
    cache: Option<Arc<dyn PageCache>>,
    next_slot: Arc<UserLimiter>,
    csrf: Mutex<Option<String>>,
    /// Nickname of the logged-in member, memoised with the session (see `forget_csrf`).
    pub(crate) nick: Mutex<Option<String>>,
    /// Number and `Brand/Perfume` id of the perfumes met in listings, by lower-case id:
    /// the application interface only knows perfumes by number.
    pub(crate) perfume_ids: Mutex<std::collections::HashMap<String, (u64, String)>>,
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
        let jar = Arc::new(Jar::default());
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
            next_slot: limiter,
            csrf: Mutex::new(None),
            nick: Mutex::new(None),
            perfume_ids: Mutex::new(std::collections::HashMap::new()),
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

    /// Waits for this client's slot (min interval + random jitter), then for the global limiter.
    async fn throttle(&self) {
        let mut slot = self.next_slot.next_slot.lock().await;
        if let Some(at) = *slot {
            let jitter_ms = self.cfg.jitter.as_millis() as u64;
            let jitter = if jitter_ms == 0 { 0 } else { rand::random_range(0..=jitter_ms) };
            tokio::time::sleep_until(at + Duration::from_millis(jitter)).await;
        }
        self.global.acquire().await;
        *slot = Some(Instant::now() + self.cfg.min_interval);
    }

    async fn send_once(
        &self,
        method: &Method,
        path_and_query: &str,
        form: Option<&[(String, String)]>,
        bearer: Option<&str>,
    ) -> Result<(RawResponse, Option<u64>, bool), reqwest::Error> {
        self.throttle().await;
        let url = self.absolute(path_and_query);
        let mut req = self.http.request(method.clone(), url);
        if let Some(token) = bearer {
            req = req.bearer_auth(token);
        }
        if let Some(form) = form {
            req = req.form(form);
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let headers = resp.headers();
        let location = headers.get(LOCATION).and_then(|v| v.to_str().ok()).map(str::to_string);
        let retry_after = headers.get(RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let challenge_header = headers.contains_key(markers::CHALLENGE_HEADER);
        // Make sure Set-Cookie headers of redirects are seen even if the store hook changes.
        let _ = headers.get(SET_COOKIE);
        let body = resp.text().await?;
        Ok((RawResponse { status, body, location }, retry_after, challenge_header))
    }

    /// Sends a request with backoff on 429 / 5xx / network errors. Returns 2xx, 3xx and
    /// 403 / 404 answers raw; challenges and exhausted retries become errors.
    pub async fn send(
        &self,
        method: Method,
        path_and_query: &str,
        form: Option<&[(String, String)]>,
        max_retries: u32,
    ) -> Result<RawResponse, ClientError> {
        self.send_with(method, path_and_query, form, max_retries, None).await
    }

    /// [`Self::send`] with an `Authorization: Bearer` header (the application interface).
    pub async fn send_with(
        &self,
        method: Method,
        path_and_query: &str,
        form: Option<&[(String, String)]>,
        max_retries: u32,
        bearer: Option<&str>,
    ) -> Result<RawResponse, ClientError> {
        let path = parse::url_path(path_and_query).to_string();
        let mut attempt = 0u32;
        loop {
            let outcome = self.send_once(&method, path_and_query, form, bearer).await;
            let (failure, retry_after) = match outcome {
                Ok((raw, retry_after, challenge_header)) => {
                    let status = raw.status;
                    if challenge_header || (status >= 400 && parse::is_challenge(&raw.body)) {
                        return Err(ClientError::CaptchaRequired);
                    }
                    if status == 429 {
                        (ClientError::RateLimited { retry_after_secs: retry_after }, retry_after)
                    } else if status >= 500 {
                        (ClientError::Upstream(format!("HTTP {status} on {path}")), retry_after)
                    } else {
                        return Ok(raw);
                    }
                }
                Err(e) => {
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
        self.throttle().await;
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
