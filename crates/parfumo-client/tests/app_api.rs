//! The application interface (`/app/v1`): session token, and the two rules that keep an
//! address from being blocked. A local fake stands in for the site.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient};

const TOKEN: &str = "tok-123";
const PASSWORD: &str = "s3cr3t-pass";
const BLOCK_PAGE: &str = "<html><title>Access Denied</title><p>your access to this website has been temporarily blocked</p></html>";

/// What the fake site saw, and how it is told to answer.
#[derive(Default)]
struct Site {
    /// `(method, path, Authorization, Cookie, body)` of every request.
    seen: Mutex<Vec<(String, String, String, String, String)>>,
    /// The token the site accepts right now.
    live: Mutex<String>,
    blocked: Mutex<bool>,
}

async fn handler(State(site): State<Arc<Site>>, method: axum::http::Method, uri: Uri, headers: HeaderMap, body: String) -> Response {
    let get = |name: header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let auth = get(header::AUTHORIZATION);
    site.seen.lock().unwrap().push((method.to_string(), uri.path().to_string(), auth.clone(), get(header::COOKIE), body.clone()));
    if *site.blocked.lock().unwrap() {
        return (StatusCode::NOT_FOUND, BLOCK_PAGE).into_response();
    }
    let json = |status: StatusCode, text: &str| (status, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    if uri.path() == "/app/v1/auth/login" {
        return if body.contains(&format!("password={PASSWORD}")) && body.contains("email=someone%40example.org") && body.contains("device_id=") {
            *site.live.lock().unwrap() = TOKEN.into();
            json(StatusCode::OK, &format!(r#"{{"status":"success","auth_token":"{TOKEN}","user_id":"1","nick_name":"testuser","photo":""}}"#))
        } else {
            json(StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Invalid credentials"}"#)
        };
    }
    if auth != format!("Bearer {}", site.live.lock().unwrap()) || auth == "Bearer " {
        return json(StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#);
    }
    match uri.path() {
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

fn client(base: &str) -> ParfumoClient {
    let cfg = ClientConfig {
        base_url: base.to_string(),
        min_interval: Duration::ZERO,
        jitter: Duration::ZERO,
        backoff_base: Duration::from_millis(5),
        timeout: Duration::from_secs(5),
        ..ClientConfig::default()
    };
    ParfumoClient::new(cfg, GlobalLimiter::new(0), None).unwrap()
}

fn requests(site: &Site) -> usize {
    site.seen.lock().unwrap().len()
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
    assert_eq!((seen[1].0.as_str(), seen[1].1.as_str(), seen[1].2.as_str()), ("GET", "/app/v1/auth/me", "Bearer tok-123"));
    assert!(seen.iter().all(|r| !r.3.contains(TOKEN)), "the token is never a cookie: {seen:?}");

    // The token is saved and restored with the session, like the cookies of the web site.
    let saved = c.export_cookies();
    let again = client(&base);
    again.import_cookies(&saved);
    assert_eq!(again.app_nick().await.unwrap(), "testuser");

    // Logging out tells the site, then forgets the token.
    c.app_logout().await;
    assert!(!c.has_app_session());
    assert_eq!(site.seen.lock().unwrap().last().unwrap().1, "/app/v1/auth/logout");
}

#[tokio::test]
async fn a_wrong_password_is_reported_and_leaves_no_token() {
    let (c, site, _) = spawn().await;
    assert_eq!(c.app_login("someone@example.org", "wrong").await.unwrap_err(), ClientError::InvalidCredentials);
    assert!(!c.has_app_session());
    assert_eq!(requests(&site), 1, "a refused login is not tried again");
    assert!(matches!(c.app_login("", "x").await.unwrap_err(), ClientError::InvalidInput(_)));
    assert_eq!(requests(&site), 1, "nothing is sent without an e-mail");
}

#[tokio::test]
async fn without_a_token_nothing_is_sent_and_a_refused_token_is_never_sent_again() {
    let (c, site, _) = spawn().await;
    // No token: the call fails here, the site sees nothing.
    assert_eq!(c.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    assert_eq!(requests(&site), 0);

    c.app_login("someone@example.org", PASSWORD).await.unwrap();
    // The site ends the session (a logout elsewhere): one refusal, then silence.
    site.live.lock().unwrap().clear();
    assert_eq!(c.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    assert!(!c.has_app_session(), "the refused token is forgotten");
    let after_refusal = requests(&site);
    for _ in 0..3 {
        assert_eq!(c.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    }
    assert_eq!(requests(&site), after_refusal, "no call leaves with a token the site refused");
}

#[tokio::test]
async fn the_block_page_of_the_site_asks_to_wait_five_minutes() {
    let (c, site, _) = spawn().await;
    c.app_login("someone@example.org", PASSWORD).await.unwrap();
    *site.blocked.lock().unwrap() = true;
    let before = requests(&site);
    assert_eq!(c.app_nick().await.unwrap_err(), ClientError::RateLimited { retry_after_secs: Some(300) });
    assert_eq!(requests(&site), before + 1, "a blocked call is not repeated");
    assert!(c.has_app_session(), "a block says nothing about the token");
}
