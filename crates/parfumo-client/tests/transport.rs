//! Transport and session logic against a local mock of the site (no network).
//! The mock reproduces behaviours measured in docs/DISCOVERY.md.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Form, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use parfumo_client::*;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[derive(Default)]
struct Mock {
    hits: Mutex<HashMap<String, usize>>,
    forms: Mutex<Vec<HashMap<String, String>>>,
    cookies_seen: Mutex<Vec<String>>,
    alive_calls: AtomicUsize,
    flaky_failures: AtomicUsize,
    reviews_empty: Mutex<bool>,
}

impl Mock {
    fn hit(&self, key: &str) -> usize {
        let mut hits = self.hits.lock().unwrap();
        let n = hits.entry(key.to_string()).or_default();
        *n += 1;
        *n
    }
    fn count(&self, key: &str) -> usize {
        self.hits.lock().unwrap().get(key).copied().unwrap_or(0)
    }
}

type S = State<Arc<Mock>>;

fn html(body: impl Into<String>) -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body.into()).into_response()
}

fn redirect(to: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, to.to_string())]).into_response()
}

async fn alive(State(m): S) -> Response {
    let n = m.alive_calls.fetch_add(1, Ordering::SeqCst) + 1;
    m.hit("alive");
    html(format!("{{\"csrf_key\":\"mockkey{n:09}\"}}"))
}

async fn perfume(State(m): S) -> Response {
    m.hit("perfume");
    html(fixture("perfume_sauvage.html"))
}

async fn reviews(State(m): S, Form(form): Form<HashMap<String, String>>) -> Response {
    m.hit("reviews");
    m.forms.lock().unwrap().push(form);
    if *m.reviews_empty.lock().unwrap() { html("") } else { html(fixture("frag_reviews_full.html")) }
}

/// Accepts only the csrf_key issued by the *second* call to /api/alive, like a session
/// whose key changed: the first call answers 200 with an empty body.
async fn classification(State(m): S, Form(form): Form<HashMap<String, String>>) -> Response {
    m.hit("classification");
    let ok = form.get("csrf_key").map(String::as_str) == Some("mockkey000000002");
    m.forms.lock().unwrap().push(form);
    if ok { html(fixture("frag_classification_pie.html")) } else { html("") }
}

async fn brand(State(m): S) -> Response {
    m.hit("brand");
    html(fixture("brand.html"))
}

async fn flaky(State(m): S) -> Response {
    let n = m.hit("flaky");
    if n <= m.flaky_failures.load(Ordering::SeqCst) {
        StatusCode::TOO_MANY_REQUESTS.into_response()
    } else {
        html("<html><head><title>ok</title></head><body><main><p>recovered</p></main></body></html>")
    }
}

async fn always_429(State(m): S) -> Response {
    m.hit("always_429");
    (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "120")]).into_response()
}

async fn always_503(State(m): S) -> Response {
    m.hit("always_503");
    StatusCode::SERVICE_UNAVAILABLE.into_response()
}

async fn challenge(State(m): S) -> Response {
    m.hit("challenge");
    (StatusCode::FORBIDDEN, html("<html><head><title>Just a moment...</title></head><body>cf-chl-bypass</body></html>")).into_response()
}

async fn mitigated(State(m): S) -> Response {
    m.hit("mitigated");
    ([("cf-mitigated", "challenge")], "<html><body>verify you are human</body></html>").into_response()
}

async fn login(State(m): S, Form(form): Form<HashMap<String, String>>) -> Response {
    m.hit("login");
    let user = form.get("username").cloned().unwrap_or_default();
    let pass = form.get("password").cloned().unwrap_or_default();
    assert_eq!(form.get("autologin").map(String::as_str), Some("checked"));
    assert_eq!(form.get("login").map(String::as_str), Some("Login"));
    if user == "captcha" {
        return (StatusCode::FORBIDDEN, html("<div class=\"cf-turnstile\"></div>")).into_response();
    }
    if user == "tester" && pass == "correct horse" {
        // Measured success shape (DISCOVERY §4): 302 to the home page, `pf_auth` and a new PHPSESSID.
        return Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, "/")
            .header(header::SET_COOKIE, "pf_auth=4242; Path=/; Max-Age=315360000; Secure; HttpOnly")
            .header(header::SET_COOKIE, "PHPSESSID=sess-after-login; Path=/; HttpOnly")
            .body(axum::body::Body::empty())
            .unwrap();
    }
    // Measured: failure redirects to /account/login_error and clears the login cookies.
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, "/account/login_error")
        .header(header::SET_COOKIE, "user_id=deleted; expires=Thu, 01-Jan-1970 00:00:01 GMT; Max-Age=0; path=/")
        .header(header::SET_COOKIE, "user_id_key=deleted; expires=Thu, 01-Jan-1970 00:00:01 GMT; Max-Age=0; path=/")
        .body(axum::body::Body::empty())
        .unwrap()
}

async fn notifications(State(m): S, headers: HeaderMap) -> Response {
    m.hit("notifications");
    let cookies = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()).unwrap_or_default();
    if cookies.contains("pf_auth=4242") {
        html("<html><body><main>Notifications</main></body></html>")
    } else {
        redirect("/action/dologin?redirect=account/notifications")
    }
}

async fn whoami(State(m): S, headers: HeaderMap) -> Response {
    let n = m.hit("whoami");
    let cookies = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    m.cookies_seen.lock().unwrap().push(cookies.clone());
    let mut resp = html(format!("<html><head><title>who</title></head><body><main><p>cookies:[{cookies}]</p></main></body></html>"));
    if !cookies.contains("PHPSESSID") {
        resp.headers_mut().insert(header::SET_COOKIE, format!("PHPSESSID=anon{n}; Path=/; Max-Age=315360000; HttpOnly").parse().unwrap());
    }
    resp
}

async fn elsewhere() -> Response {
    redirect("https://example.org/landing")
}

async fn to_internal() -> Response {
    redirect("/api/account/logout")
}

async fn logout(State(m): S) -> Response {
    m.hit("logout");
    html("logged out")
}

async fn moved() -> Response {
    redirect("/Perfumes/Dior")
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, html(fixture("notfound.html"))).into_response()
}

async fn start() -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    let app = Router::new()
        .route("/api/alive", get(alive))
        .route("/api/account/login", post(login))
        .route("/account/notifications", get(notifications))
        .route("/Perfumes/Dior/Sauvage_Eau_de_Toilette", get(perfume))
        .route("/Perfumes/Dior", get(brand))
        .route("/action/perfume/get_reviews.php", post(reviews))
        .route("/action/perfume/get_classification_chart.php", post(classification))
        .route("/flaky", get(flaky))
        .route("/always429", get(always_429))
        .route("/always503", get(always_503))
        .route("/challenge", get(challenge))
        .route("/mitigated", get(mitigated))
        .route("/whoami", get(whoami))
        .route("/elsewhere", get(elsewhere))
        .route("/moved", get(moved))
        .route("/to_internal", get(to_internal))
        .route("/api/account/logout", get(logout))
        .fallback(not_found)
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), mock)
}

fn config(base: &str) -> ClientConfig {
    ClientConfig {
        base_url: base.to_string(),
        min_interval: Duration::ZERO,
        jitter: Duration::ZERO,
        max_retries: 3,
        backoff_base: Duration::from_millis(5),
        timeout: Duration::from_secs(5),
        ..ClientConfig::default()
    }
}

fn client(base: &str) -> ParfumoClient {
    ParfumoClient::new(config(base), GlobalLimiter::new(0), None).unwrap()
}

#[derive(Default)]
struct MemoryCache {
    map: Mutex<HashMap<String, String>>,
    puts: Mutex<Vec<(String, Duration)>>,
}

impl PageCache for MemoryCache {
    fn get(&self, key: &str) -> Option<String> {
        self.map.lock().unwrap().get(key).cloned()
    }
    fn put(&self, key: &str, body: &str, ttl: Duration) {
        self.map.lock().unwrap().insert(key.to_string(), body.to_string());
        self.puts.lock().unwrap().push((key.to_string(), ttl));
    }
}

#[tokio::test]
async fn backoff_recovers_from_429() {
    let (base, mock) = start().await;
    mock.flaky_failures.store(2, Ordering::SeqCst);
    let page = client(&base).fetch_page("/flaky").await.unwrap();
    assert_eq!(page.text, "recovered");
    assert_eq!(mock.count("flaky"), 3, "two 429 then success");
}

#[tokio::test]
async fn exhausted_429_is_rate_limited_and_long_retry_after_is_not_waited() {
    let (base, mock) = start().await;
    mock.flaky_failures.store(100, Ordering::SeqCst);
    let err = client(&base).fetch_page("/flaky").await.unwrap_err();
    assert_eq!(err, ClientError::RateLimited { retry_after_secs: None });
    assert_eq!(mock.count("flaky"), 4, "1 try + 3 retries");

    let started = Instant::now();
    let err = client(&base).fetch_page("/always429").await.unwrap_err();
    assert_eq!(err, ClientError::RateLimited { retry_after_secs: Some(120) });
    assert_eq!(mock.count("always_429"), 1);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn exhausted_5xx_is_upstream_error() {
    let (base, mock) = start().await;
    let err = client(&base).fetch_page("/always503").await.unwrap_err();
    assert!(matches!(&err, ClientError::Upstream(m) if m.contains("503")), "{err:?}");
    assert_eq!(mock.count("always_503"), 4);
}

#[tokio::test]
async fn challenge_stops_immediately_with_captcha_required() {
    let (base, mock) = start().await;
    assert_eq!(client(&base).fetch_page("/challenge").await.unwrap_err(), ClientError::CaptchaRequired);
    assert_eq!(mock.count("challenge"), 1, "no retry on a challenge");
    assert_eq!(client(&base).fetch_page("/mitigated").await.unwrap_err(), ClientError::CaptchaRequired);
}

#[tokio::test]
async fn not_found_and_redirects() {
    let (base, _mock) = start().await;
    let c = client(&base);
    assert!(matches!(c.fetch_page("/nope").await, Err(ClientError::NotFound(_))));
    assert!(matches!(c.get_perfume("Dior/Unknown").await, Err(ClientError::NotFound(_))));
    assert_eq!(c.fetch_page("/account/notifications").await.unwrap_err(), ClientError::LoginRequired);
    let err = c.fetch_page("/elsewhere").await.unwrap_err();
    assert!(matches!(&err, ClientError::Upstream(m) if m.contains("another host")), "{err:?}");
    let page = c.fetch_page("/moved").await.unwrap();
    assert!(page.url.ends_with("/Perfumes/Dior"), "same-host redirects are followed: {}", page.url);
    // A redirect to an internal route is refused before the route is requested.
    let err = c.fetch_page("/to_internal").await.unwrap_err();
    assert!(matches!(&err, ClientError::InvalidInput(_)), "{err:?}");
    assert_eq!(_mock.count("logout"), 0, "the internal route must never be requested");
}

#[tokio::test]
async fn reviews_send_the_object_token_and_ask_for_full_text() {
    let (base, mock) = start().await;
    let slice = client(&base).get_perfume_reviews("Dior/Sauvage_Eau_de_Toilette", ReviewSort::Newest, 0, 3).await.unwrap();
    assert_eq!(slice.items.len(), 3);
    assert_eq!(slice.total, Some(378));
    assert_eq!(slice.items[0].id, 721243);
    let forms = mock.forms.lock().unwrap();
    let form = &forms[0];
    assert_eq!(form["p_id"], "63363");
    assert_eq!(form["h"], "0d03f07b7a18e56da70ee2ef14bba111");
    assert_eq!(form["f"], "0");
    assert_eq!(form["n"], "3");
    assert_eq!(form["t"], "378");
    assert_eq!(form["o"], "order_date_desc");
    assert_eq!(form["full"], "Ja");
    assert!(!form.contains_key("csrf_key"), "h-bound calls need no session key");
}

#[tokio::test]
async fn empty_200_body_on_action_is_an_error_not_a_success() {
    let (base, mock) = start().await;
    *mock.reviews_empty.lock().unwrap() = true;
    let err = client(&base).get_perfume_reviews("Dior/Sauvage_Eau_de_Toilette", ReviewSort::Relevance, 0, 5).await.unwrap_err();
    assert!(matches!(&err, ClientError::Upstream(m) if m.contains("empty body") && m.contains("get_reviews.php")), "{err:?}");
    assert_eq!(mock.count("perfume"), 2, "the page is re-read once to refresh the token");
    assert_eq!(mock.count("reviews"), 1, "same token: the call is not replayed blindly");

    // Past the end: no request at all, and an honest empty slice.
    let slice = client(&base).get_perfume_reviews("Dior/Sauvage_Eau_de_Toilette", ReviewSort::Relevance, 378, 5).await.unwrap();
    assert!(slice.items.is_empty());
    assert_eq!(slice.total, Some(378));
}

#[tokio::test]
async fn stale_csrf_key_is_renewed_once() {
    let (base, mock) = start().await;
    let c = client(&base);
    let perfume = c.get_perfume("Dior/Sauvage_Eau_de_Toilette").await.unwrap();
    assert_eq!(perfume.classification.len(), 4, "warnings: {:?}", perfume.warnings);
    assert!(perfume.warnings.is_empty());
    assert_eq!(mock.alive_calls.load(Ordering::SeqCst), 2, "first key rejected (empty body), second accepted");
    assert_eq!(mock.count("classification"), 2);
    assert_eq!(c.csrf_key().await.unwrap(), "mockkey000000002", "the renewed key is memoised");
    assert_eq!(mock.alive_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn optional_sub_call_failure_becomes_a_warning() {
    let (base, mock) = start().await;
    // Burn alive calls so that the accepted key (#2) is never issued to this client.
    mock.alive_calls.store(5, Ordering::SeqCst);
    let perfume = client(&base).get_perfume("Dior/Sauvage_Eau_de_Toilette").await.unwrap();
    assert_eq!(perfume.name, "Sauvage");
    assert!(perfume.classification.is_empty());
    assert_eq!(perfume.warnings.len(), 1);
    assert!(perfume.warnings[0].contains("classification"), "{:?}", perfume.warnings);
    assert!(!perfume.warnings[0].contains("mockkey"), "no token in warnings");
}

#[tokio::test]
async fn cache_is_used_keys_hold_no_csrf_and_failures_are_not_cached() {
    let (base, mock) = start().await;
    let cache = Arc::new(MemoryCache::default());
    let c = ParfumoClient::new(config(&base), GlobalLimiter::new(0), Some(cache.clone())).unwrap();
    let first = c.get_brand("/Perfumes/Dior", 1).await.unwrap();
    let second = c.get_brand("/Perfumes/Dior", 1).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(mock.count("brand"), 1, "second read comes from the cache");

    let perfume = c.get_perfume("Dior/Sauvage_Eau_de_Toilette").await.unwrap();
    assert_eq!(perfume.classification.len(), 4);
    // Another client (another user, another session) shares the public cache.
    let other = ParfumoClient::new(config(&base), GlobalLimiter::new(0), Some(cache.clone())).unwrap();
    let again = other.get_perfume("Dior/Sauvage_Eau_de_Toilette").await.unwrap();
    assert_eq!(again, perfume);
    assert_eq!(mock.count("perfume"), 1);
    assert_eq!(mock.count("classification"), 2, "1 rejected + 1 accepted, none for the second client");

    {
        let puts = cache.puts.lock().unwrap();
        assert!(puts.iter().all(|(k, _)| !k.contains("mockkey") && !k.contains("csrf_key")), "{puts:?}");
        assert!(puts.iter().any(|(k, ttl)| k.contains("/Perfumes/Dior/Sauvage") && *ttl == RECORD_TTL));
        assert!(puts.iter().all(|(k, _)| !k.contains("/api/alive") && !k.contains("login")));
    }

    // Errors and empty bodies never enter the cache.
    *mock.reviews_empty.lock().unwrap() = true;
    let before = cache.map.lock().unwrap().len();
    assert!(c.get_perfume_reviews("Dior/Sauvage_Eau_de_Toilette", ReviewSort::Newest, 0, 3).await.is_err());
    assert!(c.fetch_page("/always503").await.is_err());
    assert!(c.fetch_page("/nope").await.is_err());
    let map = cache.map.lock().unwrap();
    assert_eq!(map.len(), before);
    assert!(map.values().all(|v| !v.trim().is_empty()));
}

#[tokio::test]
async fn fetch_page_listing_ttl_is_one_hour() {
    let (base, _mock) = start().await;
    let cache = Arc::new(MemoryCache::default());
    let c = ParfumoClient::new(config(&base), GlobalLimiter::new(0), Some(cache.clone())).unwrap();
    c.fetch_page("/whoami").await.unwrap();
    let puts = cache.puts.lock().unwrap();
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].1, LISTING_TTL);
    assert_eq!(LISTING_TTL, Duration::from_secs(3600));
    assert_eq!(RECORD_TTL, Duration::from_secs(86400));
}

#[tokio::test]
async fn login_failure_is_invalid_credentials_without_leaking_the_password() {
    let (base, mock) = start().await;
    let c = client(&base);
    let err = c.login("tester", "wrong-Passw0rd!").await.unwrap_err();
    assert_eq!(err, ClientError::InvalidCredentials);
    for rendered in [format!("{err}"), format!("{err:?}")] {
        assert!(!rendered.contains("wrong-Passw0rd!") && !rendered.contains("tester"));
    }
    assert_eq!(mock.count("login"), 1, "login is never retried by the client");
    assert!(c.export_cookies().iter().all(|k| k.name != "user_id" && k.name != "user_id_key"));
    assert!(!c.is_logged_in().await.unwrap());
    assert!(matches!(c.login("", "x").await, Err(ClientError::InvalidInput(_))));
}

#[tokio::test]
async fn login_challenge_is_captcha_required() {
    let (base, _mock) = start().await;
    assert_eq!(client(&base).login("captcha", "whatever").await.unwrap_err(), ClientError::CaptchaRequired);
}

/// The mock implements the success shape measured at jalon 0 bis (DISCOVERY §4): 302 to the
/// home page with the `pf_auth` login cookie and a new `PHPSESSID`.
#[tokio::test]
async fn login_success_sets_session_cookies() {
    let (base, _mock) = start().await;
    let c = client(&base);
    c.login("tester", "correct horse").await.unwrap();
    let cookies = c.export_cookies();
    let names: Vec<&str> = cookies.iter().map(|k| k.name.as_str()).collect();
    assert!(names.contains(&"pf_auth") && names.contains(&"PHPSESSID"));
    assert!(c.is_logged_in().await.unwrap());
    let debug = format!("{cookies:?}");
    assert!(!debug.contains("k3y") && !debug.contains("4242") && !debug.contains("sess-after-login"), "{debug}");

    // Persisted cookies restore the session in a fresh client; clearing them ends it.
    let restored = client(&base);
    assert!(!restored.is_logged_in().await.unwrap());
    restored.import_cookies(&cookies);
    assert!(restored.is_logged_in().await.unwrap());
    restored.clear_cookies();
    assert!(restored.export_cookies().is_empty());
    assert!(!restored.is_logged_in().await.unwrap());
}

/// Measured at jalon 0 bis: `pf_auth` alone rebuilds a session; the mock checks it.
#[tokio::test]
async fn a_cookie_header_is_imported_and_rebuilds_the_session() {
    let (base, mock) = start().await;
    let c = client(&base);
    assert_eq!(c.import_cookie_header("Cookie: PHPSESSID=abc123; pf_auth=4242; other=zzz; broken; =x"), 3);
    assert!(c.is_logged_in().await.unwrap());
    c.fetch_page("/whoami").await.unwrap();
    let seen = mock.cookies_seen.lock().unwrap();
    assert!(seen[0].contains("PHPSESSID=abc123") && seen[0].contains("pf_auth=4242"));
}

#[tokio::test]
async fn cookie_jars_are_isolated_between_clients() {
    let (base, mock) = start().await;
    let a = client(&base);
    let b = client(&base);
    a.fetch_page("/whoami").await.unwrap();
    b.fetch_page("/whoami").await.unwrap();
    a.import_cookie_header("secret_a=only-for-a");
    a.fetch_page("/whoami?2").await.unwrap();
    b.fetch_page("/whoami?2").await.unwrap();
    let value = |c: &ParfumoClient| c.export_cookies().into_iter().find(|k| k.name == "PHPSESSID").unwrap().value;
    assert_ne!(value(&a), value(&b));
    assert!(b.export_cookies().iter().all(|k| k.name != "secret_a"));
    let seen = mock.cookies_seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    assert!(seen[2].contains("secret_a=only-for-a") && seen[2].contains(&value(&a)));
    assert!(!seen[3].contains("secret_a") && !seen[3].contains(&value(&a)));
    assert!(seen[3].contains(&value(&b)));
}

#[tokio::test]
async fn requests_are_spaced_by_at_least_the_minimum_interval() {
    let (base, _mock) = start().await;
    let cfg = ClientConfig { min_interval: Duration::from_millis(150), jitter: Duration::from_millis(40), ..config(&base) };
    let c = ParfumoClient::new(cfg, GlobalLimiter::new(0), None).unwrap();
    let started = Instant::now();
    for i in 0..4 {
        c.fetch_page(&format!("/whoami?{i}")).await.unwrap();
    }
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(450), "3 gaps of ≥150 ms expected, got {elapsed:?}");

    // Concurrent calls on clones of one client are serialised by the same limiter.
    let started = Instant::now();
    let (c1, c2) = (c.clone(), c.clone());
    let (r1, r2) = tokio::join!(c1.fetch_page("/whoami?a"), c2.fetch_page("/whoami?b"));
    r1.unwrap();
    r2.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(150));
}

#[tokio::test]
async fn global_limiter_is_shared_by_all_clients() {
    let (base, _mock) = start().await;
    let global = GlobalLimiter::new(5); // 5 requests/s in total → 200 ms between any two requests
    let a = ParfumoClient::new(config(&base), global.clone(), None).unwrap();
    let b = ParfumoClient::new(config(&base), global, None).unwrap();
    let started = Instant::now();
    for i in 0..2 {
        a.fetch_page(&format!("/whoami?a{i}")).await.unwrap();
        b.fetch_page(&format!("/whoami?b{i}")).await.unwrap();
    }
    assert!(started.elapsed() >= Duration::from_millis(550), "{:?}", started.elapsed());
}

#[tokio::test]
async fn fetch_page_refuses_foreign_hosts_and_internal_routes() {
    let c = ParfumoClient::new(ClientConfig::default(), GlobalLimiter::new(0), None).unwrap();
    for bad in [
        "https://evil.example/Perfumes/Dior",
        "http://www.parfumo.com.evil.example/",
        "https://www.parfumo.com@evil.example/",
        "https://user:pw@www.parfumo.com/Perfumes",
        "ftp://www.parfumo.com/Perfumes",
        "//evil.example/x",
        "https://www.parfumo.com:8443/Perfumes",
        "/api/alive",
        "/API/account/login",
        "/action/perfume/get_reviews.php",
        "/%61ction/perfume/get_reviews.php",
        "/Perfumes/../api/alive",
        "//api/alive",
        "/account/delete",
        "/account/delete/confirm",
        "/account/password_forgot",
        "/account/email",
        "/app_v5/api/x",
        "/login.php",
        "",
        "/a\\b",
    ] {
        assert!(matches!(c.fetch_target(bad), Err(ClientError::InvalidInput(_))), "{bad:?} must be refused");
    }
    assert_eq!(c.fetch_target("https://www.parfumo.com/forums?x=1").unwrap(), "/forums?x=1");
    assert_eq!(c.fetch_target("https://parfumo.com/rules").unwrap(), "/rules");
    assert_eq!(c.fetch_target("Perfumes/Dior").unwrap(), "/Perfumes/Dior");
    assert_eq!(c.fetch_target("/Users/someone/Collection").unwrap(), "/Users/someone/Collection");
}

#[tokio::test]
async fn references_are_validated_before_any_request() {
    let (base, mock) = start().await;
    let c = client(&base);
    for bad in ["", "Dior", "https://evil.example/Perfumes/Dior/Sauvage", "/Users/x", "Dior/../../api/alive"] {
        assert!(matches!(c.get_perfume(bad).await, Err(ClientError::InvalidInput(_))), "{bad:?}");
    }
    assert!(matches!(c.search_perfumes(&SearchQuery::default(), 1).await, Err(ClientError::InvalidInput(_))));
    assert!(matches!(c.get_rankings(&RankingKind::Year(12), 1).await, Err(ClientError::InvalidInput(_))));
    assert!(mock.hits.lock().unwrap().is_empty(), "no request was sent");
}

#[test]
fn errors_never_render_secrets() {
    let all = [
        ClientError::NotFound("/Perfumes/x".into()),
        ClientError::RateLimited { retry_after_secs: Some(30) },
        ClientError::RateLimited { retry_after_secs: None },
        ClientError::CaptchaRequired,
        ClientError::ParserOutdated("perfume page: h1 missing".into()),
        ClientError::Upstream("HTTP 503 on /x".into()),
        ClientError::InvalidCredentials,
        ClientError::LoginRequired,
        ClientError::InvalidInput("bad".into()),
    ];
    for e in &all {
        let text = format!("{e} {e:?}");
        assert!(!text.to_lowercase().contains("password=") && !text.contains("csrf_key=") && !text.contains("PHPSESSID"));
    }
    assert_eq!(all[1].to_string(), "rate limited by Parfumo (retry after 30 s)");
    let cookie = StoredCookie { name: "PHPSESSID".into(), value: "topsecret".into(), domain: "parfumo.com".into(), path: "/".into() };
    assert!(!format!("{cookie:?}").contains("topsecret"));
}
