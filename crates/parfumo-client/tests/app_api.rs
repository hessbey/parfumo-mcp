//! The application interface (`/app/v1`): the session token, its one owner per user, and
//! the rules that keep an address from being blocked. A local fake stands in for the site.
//!
//! The fake answers in the shapes `docs/APP_API.md` records. The page by which the site
//! blocks an address was never recorded: the fake builds it from what that document quotes
//! of it (status 404, an HTML page, "Access Denied … try again in 5 minutes").

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient, StoredCookie, UserLimiter};

const PASSWORD: &str = "s3cr3t-pass";
const EMAIL: &str = "someone@example.org";
const BLOCK_PAGE: &str = "<html><head><title>Access Denied</title></head><body><h1>Access Denied</h1><p>try again in 5 minutes</p></body></html>";

/// One request received: `(method, path, Authorization, Cookie, body)`.
type Seen = (String, String, String, String, String);

/// What the fake site saw, and how it is told to answer.
#[derive(Default)]
struct Site {
    seen: Mutex<Vec<Seen>>,
    /// The token the site accepts right now. Each login issues a new one: `tok-1`, `tok-2`…
    live: Mutex<String>,
    issued: AtomicU64,
    blocked: Mutex<bool>,
    /// Milliseconds an answer is held back, read when the request arrives: the answer is
    /// decided on arrival, as on a site that is slow to reply.
    delay_ms: AtomicU64,
    /// What the login answers instead of its usual answer.
    login_answer: Mutex<Option<(u16, &'static str)>>,
}

async fn handler(State(site): State<Arc<Site>>, method: axum::http::Method, uri: Uri, headers: HeaderMap, body: String) -> Response {
    let get = |name: header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let auth = get(header::AUTHORIZATION);
    site.seen.lock().unwrap().push((method.to_string(), uri.path().to_string(), auth.clone(), get(header::COOKIE), body.clone()));
    let answer = answer(&site, uri.path(), &auth, &body);
    tokio::time::sleep(Duration::from_millis(site.delay_ms.load(Ordering::SeqCst))).await;
    answer
}

fn answer(site: &Site, path: &str, auth: &str, body: &str) -> Response {
    if *site.blocked.lock().unwrap() {
        return (StatusCode::NOT_FOUND, [(header::CONTENT_TYPE, "text/html")], BLOCK_PAGE).into_response();
    }
    let json = |status: StatusCode, text: &str| (status, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    if path == "/app/v1/auth/login" {
        if let Some((status, text)) = *site.login_answer.lock().unwrap() {
            return json(StatusCode::from_u16(status).unwrap(), text);
        }
        return if body.contains(&format!("password={PASSWORD}")) && body.contains("email=someone%40example.org") && body.contains("device_id=") {
            let token = format!("tok-{}", site.issued.fetch_add(1, Ordering::SeqCst) + 1);
            *site.live.lock().unwrap() = token.clone();
            json(StatusCode::OK, &format!(r#"{{"status":"success","auth_token":"{token}","user_id":"1","nick_name":"testuser","photo":""}}"#))
        } else {
            json(StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Invalid credentials"}"#)
        };
    }
    if !path.starts_with("/app/v1/") {
        return json(StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#);
    }
    if auth != format!("Bearer {}", site.live.lock().unwrap()) || auth == "Bearer " {
        return json(StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#);
    }
    match path {
        "/app/v1/auth/me" => json(StatusCode::OK, r#"{"status":"success","user":{"id":"1","nick_name":"testuser"}}"#),
        "/app/v1/auth/logout" => {
            site.live.lock().unwrap().clear();
            json(StatusCode::OK, r#"{"status":"success"}"#)
        }
        _ => json(StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#),
    }
}

async fn spawn() -> (ParfumoClient, Arc<Site>, String) {
    let site = Arc::new(Site::default());
    let app = Router::new().fallback(handler).with_state(site.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (client(&base), site, base)
}

fn config(base: &str, min_interval: Duration) -> ClientConfig {
    ClientConfig {
        base_url: base.to_string(),
        min_interval,
        jitter: Duration::ZERO,
        backoff_base: Duration::from_millis(5),
        timeout: Duration::from_secs(5),
        ..ClientConfig::default()
    }
}

fn client(base: &str) -> ParfumoClient {
    ParfumoClient::new(config(base, Duration::ZERO), GlobalLimiter::new(0), None).unwrap()
}

/// A client whose requests are spaced, as in production: calls made together queue.
fn spaced(base: &str, min_interval_ms: u64) -> ParfumoClient {
    ParfumoClient::new(config(base, Duration::from_millis(min_interval_ms)), GlobalLimiter::new(0), None).unwrap()
}

fn requests(site: &Site) -> usize {
    site.seen.lock().unwrap().len()
}

/// The `Authorization` header of every request made as the member since `from`.
fn bearers(site: &Site, from: usize) -> Vec<String> {
    site.seen.lock().unwrap()[from..].iter().map(|r| r.2.clone()).filter(|auth| !auth.is_empty()).collect()
}

#[tokio::test]
async fn login_keeps_a_token_that_is_sent_as_a_bearer_and_never_as_a_cookie() {
    let (c, site, base) = spawn().await;
    assert!(!c.has_app_session());
    c.app_login(" someone@example.org ", PASSWORD).await.unwrap();
    assert!(c.has_app_session());
    assert_eq!(c.app_nick().await.unwrap(), "testuser");

    let seen = site.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!((seen[0].0.as_str(), seen[0].1.as_str(), seen[0].2.as_str()), ("POST", "/app/v1/auth/login", ""), "the login carries no token");
    assert_eq!((seen[1].0.as_str(), seen[1].1.as_str(), seen[1].2.as_str()), ("GET", "/app/v1/auth/me", "Bearer tok-1"));
    assert!(seen.iter().all(|r| !r.3.contains("tok-1")), "the token is never a cookie: {seen:?}");

    // The token is saved and restored with the session, like the cookies of the web site.
    let saved = c.export_cookies();
    let again = client(&base);
    again.import_cookies(&saved);
    assert_eq!(again.app_nick().await.unwrap(), "testuser");

    // Logging in to the web site again forgets its cookies, not this session.
    c.clear_cookies();
    assert!(c.has_app_session(), "the session of the application interface is not the web session");

    // Logging out tells the site, then forgets the token.
    c.app_logout().await;
    assert!(!c.has_app_session());
    assert_eq!(site.seen.lock().unwrap().last().unwrap().1, "/app/v1/auth/logout");
}

#[tokio::test]
async fn a_wrong_password_is_reported_and_leaves_no_token() {
    let (c, site, _) = spawn().await;
    assert_eq!(c.app_login(EMAIL, "wrong").await.unwrap_err(), ClientError::InvalidCredentials);
    assert!(!c.has_app_session());
    assert_eq!(requests(&site), 1, "a refused login is not tried again");
    assert!(matches!(c.app_login("", "x").await.unwrap_err(), ClientError::InvalidInput(_)));
    assert_eq!(requests(&site), 1, "nothing is sent without an e-mail");
}

#[tokio::test]
async fn only_an_answer_of_the_login_itself_refuses_the_credentials() {
    let (c, site, _) = spawn().await;
    let answers = |answer: (u16, &'static str)| *site.login_answer.lock().unwrap() = Some(answer);
    // The interface answered the login and gave no token: these credentials are refused.
    for refusal in [
        (401, r#"{"status":"error","message":"Invalid credentials"}"#),
        (200, r#"{"status":"error","message":"Invalid credentials"}"#),
        (403, r#"{"error":"Forbidden"}"#),
    ] {
        answers(refusal);
        assert_eq!(c.app_login(EMAIL, PASSWORD).await.unwrap_err(), ClientError::InvalidCredentials, "{refusal:?}");
    }
    // Anything else says nothing about them: they must not be given up for it.
    answers((404, r#"{"error":"Not found"}"#));
    assert!(matches!(c.app_login(EMAIL, PASSWORD).await.unwrap_err(), ClientError::Upstream(_)), "the call is gone");
    answers((200, r#"{"status":"success"}"#));
    assert!(matches!(c.app_login(EMAIL, PASSWORD).await.unwrap_err(), ClientError::ParserOutdated(_)), "the answer changed");
    answers((200, "<html>maintenance</html>"));
    assert!(matches!(c.app_login(EMAIL, PASSWORD).await.unwrap_err(), ClientError::ParserOutdated(_)));
    answers((503, r#"{"error":"Unavailable"}"#));
    assert!(matches!(c.app_login(EMAIL, PASSWORD).await.unwrap_err(), ClientError::Upstream(_)));
    answers((429, r#"{"error":"Too many requests"}"#));
    assert!(matches!(c.app_login(EMAIL, PASSWORD).await.unwrap_err(), ClientError::RateLimited { .. }));
    assert_eq!(requests(&site), 8, "a login is never repeated, whatever it answers");
    assert!(!c.has_app_session());
}

#[tokio::test]
async fn without_a_token_nothing_is_sent_and_a_refused_token_is_never_sent_again() {
    let (c, site, _) = spawn().await;
    // No token: the call fails here, the site sees nothing.
    assert_eq!(c.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    assert_eq!(requests(&site), 0);

    c.app_login(EMAIL, PASSWORD).await.unwrap();
    // The site ends the session (a logout elsewhere): one refusal, then silence.
    site.live.lock().unwrap().clear();
    assert_eq!(c.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    assert!(!c.has_app_session(), "the refused token is forgotten");
    assert!(c.app_session_ended(), "the site ended the session: a login now renews it");
    let after_refusal = requests(&site);
    for _ in 0..3 {
        assert_eq!(c.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    }
    assert_eq!(requests(&site), after_refusal, "no call leaves with a token the site refused");
    // A new login is a new session.
    c.app_login(EMAIL, PASSWORD).await.unwrap();
    assert!(c.has_app_session() && !c.app_session_ended());
}

#[tokio::test]
async fn calls_made_together_send_a_refused_token_once() {
    let (_, site, base) = spawn().await;
    let c = spaced(&base, 40);
    c.app_login(EMAIL, PASSWORD).await.unwrap();
    site.live.lock().unwrap().clear();
    let before = requests(&site);
    // Three calls of one user at once, as an assistant sends them: they queue one slot apart.
    let (a, b, d) = tokio::join!(c.app_nick(), c.app_nick(), c.app_nick());
    for outcome in [a, b, d] {
        assert_eq!(outcome.unwrap_err(), ClientError::LoginRequired);
    }
    assert_eq!(bearers(&site, before), ["Bearer tok-1"], "the calls that waited behind the refusal sent the dead token again");
    assert_eq!(requests(&site), before + 1);
}

#[tokio::test]
async fn a_refusal_slower_than_the_request_slot_still_stops_the_calls_behind_it() {
    let (_, site, base) = spawn().await;
    let c = spaced(&base, 10);
    c.app_login(EMAIL, PASSWORD).await.unwrap();
    site.live.lock().unwrap().clear();
    // The site takes fifteen slots to say no: the next slot is free long before.
    site.delay_ms.store(150, Ordering::SeqCst);
    let before = requests(&site);
    let (a, b, d) = tokio::join!(c.app_nick(), c.app_nick(), c.app_nick());
    for outcome in [a, b, d] {
        assert_eq!(outcome.unwrap_err(), ClientError::LoginRequired);
    }
    assert_eq!(bearers(&site, before), ["Bearer tok-1"]);
}

#[tokio::test]
async fn the_token_sent_is_the_one_held_when_the_request_leaves() {
    let (_, site, base) = spawn().await;
    let c = spaced(&base, 150);
    c.app_login(EMAIL, PASSWORD).await.unwrap();
    // Dropped while the call waits for its slot (another account is linked): nothing leaves.
    let before = requests(&site);
    let queued = tokio::spawn({
        let c = c.clone();
        async move { c.app_nick().await }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    c.forget_app_session();
    assert_eq!(queued.await.unwrap().unwrap_err(), ClientError::LoginRequired);
    assert_eq!(requests(&site), before, "a token dropped during the wait was sent");

    // Replaced while the call waits: the new one leaves, never the one read too early.
    c.app_login(EMAIL, PASSWORD).await.unwrap();
    let before = requests(&site);
    let queued = tokio::spawn({
        let c = c.clone();
        async move { c.app_nick().await }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    c.import_cookies(&[StoredCookie {
        name: "app_token".into(),
        value: "tok-restored".into(),
        domain: "app-token.invalid".into(),
        path: "/".into(),
    }]);
    let _ = queued.await.unwrap();
    assert_eq!(bearers(&site, before), ["Bearer tok-restored"]);
}

#[tokio::test]
async fn a_late_refusal_of_an_older_token_leaves_the_new_one_alone() {
    let (c, site, _) = spawn().await;
    c.app_login(EMAIL, PASSWORD).await.unwrap();
    // The site ended the first session and is slow to say so.
    site.live.lock().unwrap().clear();
    site.delay_ms.store(200, Ordering::SeqCst);
    let slow = tokio::spawn({
        let c = c.clone();
        async move { c.app_nick().await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Meanwhile a login obtains a new session.
    site.delay_ms.store(0, Ordering::SeqCst);
    c.app_login(EMAIL, PASSWORD).await.unwrap();
    // The refusal of the first token arrives after it.
    assert_eq!(slow.await.unwrap().unwrap_err(), ClientError::LoginRequired);
    assert!(c.has_app_session(), "the refusal of the old token wiped the new one");
    assert_eq!(c.app_nick().await.unwrap(), "testuser");
    assert_eq!(site.seen.lock().unwrap().last().unwrap().2, "Bearer tok-2");
}

#[tokio::test]
async fn a_login_answered_after_the_session_was_dropped_brings_none_back() {
    let (c, site, _) = spawn().await;
    site.delay_ms.store(150, Ordering::SeqCst);
    let login = tokio::spawn({
        let c = c.clone();
        async move { c.app_login(EMAIL, PASSWORD).await }
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    // Another account is linked while the login of the previous one is under way.
    c.forget_app_session();
    assert_eq!(login.await.unwrap().unwrap_err(), ClientError::LoginRequired);
    assert!(!c.has_app_session(), "the token of a dropped session was kept");
}

#[tokio::test]
async fn the_clients_of_one_user_hold_one_session() {
    let (_, site, base) = spawn().await;
    let user = UserLimiter::new();
    let of_user = |limiter: &Arc<UserLimiter>| {
        ParfumoClient::with_limiter(config(&base, Duration::ZERO), GlobalLimiter::new(0), None, limiter.clone()).unwrap()
    };
    let (account, public) = (of_user(&user), of_user(&user));
    let (other_user, canary) = (of_user(&UserLimiter::new()), of_user(&user.web_only()));

    account.app_login(EMAIL, PASSWORD).await.unwrap();
    assert!(public.has_app_session(), "the public client reads through the session of the account");
    assert!(!other_user.has_app_session(), "the token of one user is not another user's");
    assert!(!canary.has_app_session(), "a client for the web pages only has no session");
    assert!(user.take_app_session_change(), "a new token is to be stored");
    let saved = account.export_cookies();
    assert!(saved.iter().any(|c| c.name == "app_token"), "the token is stored with the session of the account");

    // The site refuses the token to the public client: it is gone for the account at once,
    // and neither client sends it again.
    site.live.lock().unwrap().clear();
    assert_eq!(public.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    let after_refusal = requests(&site);
    assert!(!account.has_app_session() && !public.has_app_session());
    assert_eq!(account.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    assert_eq!(public.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    assert_eq!(requests(&site), after_refusal);
    assert!(user.take_app_session_change(), "what is stored still holds the refused token");
    assert!(account.export_cookies().iter().all(|c| c.name != "app_token"), "the refused token would be stored again");

    // It does not come back from what was stored before the refusal either.
    account.import_cookies(&saved);
    of_user(&user).import_cookies(&saved);
    assert!(!account.has_app_session() && !public.has_app_session(), "the refused token was taken back");
    assert_eq!(public.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    assert_eq!(requests(&site), after_refusal);
}

#[tokio::test]
async fn the_block_page_stops_every_call_to_the_interface_for_five_minutes() {
    let (_, site, base) = spawn().await;
    // Two users of one process, and a client of another process.
    let process = GlobalLimiter::new(0);
    let of_process = |global: &Arc<GlobalLimiter>| ParfumoClient::new(config(&base, Duration::ZERO), global.clone(), None).unwrap();
    let (c, other, elsewhere) = (of_process(&process), of_process(&process), of_process(&GlobalLimiter::new(0)));
    c.app_login(EMAIL, PASSWORD).await.unwrap();
    other.app_login(EMAIL, PASSWORD).await.unwrap();
    assert!(!c.app_blocked());

    *site.blocked.lock().unwrap() = true;
    let before = requests(&site);
    assert_eq!(c.app_nick().await.unwrap_err(), ClientError::RateLimited { retry_after_secs: Some(300) });
    assert_eq!(requests(&site), before + 1, "a blocked call is not repeated");
    assert!(c.has_app_session(), "a block says nothing about the token");
    assert!(c.app_blocked() && other.app_blocked() && !elsewhere.app_blocked(), "a block is known to the whole process, and to it only");

    // While it lasts nothing leaves for the interface: no call of this user, none of
    // another user, no login.
    let waits = [c.app_nick().await, c.app_nick().await, other.app_nick().await, other.app_login(EMAIL, PASSWORD).await.map(|()| String::new())];
    for wait in waits {
        assert!(matches!(wait, Err(ClientError::RateLimited { retry_after_secs: Some(1..=300) })), "{wait:?}");
    }
    assert_eq!(requests(&site), before + 1, "a request left for the interface during the block");
    assert!(other.has_app_session());

    // A read goes to the web site instead. Here the block covers the web pages too: the
    // answer asks to wait, where the page read as a web page would say "not found".
    let err = c.get_perfume("Dior/Sauvage_Eau_de_Toilette").await.unwrap_err();
    assert!(matches!(err, ClientError::RateLimited { retry_after_secs: Some(1..=300) }), "what is left of the block: {err:?}");
    let asked: Vec<String> = site.seen.lock().unwrap()[before + 1..].iter().map(|r| r.1.clone()).collect();
    assert_eq!(asked, ["/Perfumes/Dior/Sauvage_Eau_de_Toilette"], "the web site answers during the block, the interface is not asked");
}
