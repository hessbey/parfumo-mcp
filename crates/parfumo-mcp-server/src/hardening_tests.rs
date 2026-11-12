//! Tests of jalon 5 (hardening): login throttling per client address, the canary against
//! stand-ins for parfumo.com, and its alert.

use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};

use parfumo_mcp_core::testing::{link_account, site::spawn_site};

use crate::{
    canary::{self, Outcome},
    tests::{TestApp, spawn_mock},
    web::{LOGIN_MAX_FAILURES_PER_IP, Window, client_ip},
};

async fn link_from(app: &TestApp, forwarded_for: &str, key: &str) -> StatusCode {
    let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs([("access_key", key), ("action", "unlink")]).finish();
    let req = Request::post("/link")
        .header("host", "localhost:8080")
        .header("x-forwarded-for", forwarded_for)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();
    app.send(req).await.0
}

#[tokio::test]
async fn one_address_cannot_try_many_keys() {
    let app = TestApp::new("http://127.0.0.1:9");
    let (_, key) = app.add_user("alice");
    for i in 0..LOGIN_MAX_FAILURES_PER_IP {
        assert_eq!(link_from(&app, "203.0.113.7", &format!("pmcp_guess{i}")).await, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(link_from(&app, "203.0.113.7", &key).await, StatusCode::TOO_MANY_REQUESTS, "the address is locked, even with the right key");
    // The address that counts is the one the proxy appended, not what the client claims.
    assert_eq!(link_from(&app, "198.51.100.1, 203.0.113.7", &key).await, StatusCode::TOO_MANY_REQUESTS);
    // Another address is not affected.
    assert_eq!(link_from(&app, "203.0.113.8", &key).await, StatusCode::OK);
}

#[test]
fn the_client_address_is_the_last_forwarded_entry() {
    let headers = |v: &str| {
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-forwarded-for", v.parse().unwrap());
        h
    };
    assert_eq!(client_ip(&headers("203.0.113.7")).as_deref(), Some("203.0.113.7"));
    assert_eq!(client_ip(&headers("10.0.0.1, 2001:db8::1")).as_deref(), Some("2001:db8::1"));
    assert_eq!(client_ip(&headers("not an address")), None);
    assert_eq!(client_ip(&axum::http::HeaderMap::new()), None);
}

#[test]
fn a_window_lets_through_its_quota_only() {
    let w = Window::new(3, std::time::Duration::from_secs(60));
    assert_eq!((0..5).filter(|_| w.allow()).count(), 3);
    let quick = Window::new(1, std::time::Duration::ZERO);
    assert!(quick.allow() && quick.allow(), "old events leave the window");
}

/// The first stand-in answers every path with the same meaningless page: for the canary,
/// that is a site whose structure changed everywhere.
#[tokio::test]
async fn the_canary_reports_a_changed_site_stores_the_run_and_alerts() {
    let (base, mock) = spawn_mock().await;
    let app = TestApp::new(&base);
    let (_, _, body) = app.get("/healthz").await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["canary"], Value::Null, "no run yet");

    let report = canary::run(&app.state, None).await;
    assert_eq!(report.status, "changed", "{}", report.text());
    assert!(report.count(Outcome::Changed) >= 10, "{}", report.text());
    assert!(report.checks.iter().all(|c| c.area == "public"), "no account was asked for");
    let perfume = report.checks.iter().find(|c| c.name == "perfume").unwrap();
    assert_eq!((perfume.outcome, perfume.function.as_str()), (Outcome::Changed, "get_perfume"));
    // Checks that build on a failed one are skipped, not guessed.
    assert_eq!(report.checks.iter().find(|c| c.name == "perfumer").unwrap().outcome, Outcome::Skipped);
    // Reads only: the canary never logs in and never calls an account route here.
    assert!(mock.seen.lock().unwrap().iter().all(|(path, cookie)| path != "/api/account/login" && cookie.is_empty()));

    // Stored, and shown by /healthz without failing the health check.
    assert_eq!(canary::last(&app.state.core.db).unwrap().checks.len(), report.checks.len());
    let (status, _, body) = app.get("/healthz").await;
    let health: Value = serde_json::from_str(&body).unwrap();
    assert_eq!((status, health["status"].as_str(), health["canary"]["status"].as_str()), (StatusCode::OK, Some("ok"), Some("changed")));

    // The alert names the checks that need attention.
    let got: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = got.clone();
    let hook = axum::Router::new().fallback(move |body: String| {
        let sink = sink.clone();
        async move {
            sink.lock().unwrap().push(body);
            "ok"
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, hook).await;
    });
    canary::notify(&format!("http://{addr}/hook"), &report).await.unwrap();
    let alert: Value = serde_json::from_str(&got.lock().unwrap()[0]).unwrap();
    assert_eq!(alert["status"], "changed");
    assert!(alert["text"].as_str().unwrap().starts_with("canary changed:") && alert["text"] == alert["content"]);
    assert!(alert["problems"].as_array().unwrap().iter().any(|p| p["name"] == "perfume"));
    assert!(canary::notify("http://127.0.0.1:9/hook", &report).await.is_err(), "an unreachable webhook is an error, not a panic");
}

/// The second stand-in is the fake account site of the account tests.
#[tokio::test]
async fn the_canary_reads_the_linked_account_and_never_writes() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (id, _) = app.add_user("alice");
    link_account(&app.state.core, id, "alice").await;
    let user = parfumo_mcp_core::db::User { id, name: "alice".into() };
    let logins = site.logins.load(std::sync::atomic::Ordering::SeqCst);

    let report = canary::run(&app.state, Some(&user)).await;
    let account: Vec<_> = report.checks.iter().filter(|c| c.area == "account").collect();
    assert!(account.len() >= 15, "{}", report.text());
    for name in ["own nickname", "collection lists", "wears overview"] {
        let check = account.iter().find(|c| c.name == name).unwrap();
        assert_eq!(check.outcome, Outcome::Ok, "{name}: {}", check.detail);
    }
    assert_eq!(site.write_count(), 0, "the canary must never send a write");
    assert_eq!(site.logins.load(std::sync::atomic::Ordering::SeqCst), logins, "a live session is reused, not logged in again");
    let json = serde_json::to_string(&report).unwrap();
    for secret in ["pw-alice", "0123456789abcdef0123456789abcdef"] {
        assert!(!json.contains(secret), "the report must hold no secret");
    }
    assert_eq!(report.account_user.as_deref(), Some("alice"));

    // Without a linked account the account part fails cleanly and says why.
    let (bob, _) = app.add_user("bob");
    let report = canary::run(&app.state, Some(&parfumo_mcp_core::db::User { id: bob, name: "bob".into() })).await;
    let session = report.checks.iter().find(|c| c.area == "account").unwrap();
    assert_eq!((session.name.as_str(), session.outcome), ("account session", Outcome::Failed));
    assert!(session.detail.contains("no Parfumo account is linked"));
}

#[tokio::test]
async fn the_configured_canary_user_must_exist() {
    let (base, _mock) = spawn_mock().await;
    let mut http = crate::tests::test_server_config();
    http.canary_user = Some("nobody".into());
    let app = TestApp::with_configs(crate::tests::test_config(&base), http, parfumo_mcp_core::db::Db::open_in_memory().unwrap());
    let report = canary::run_and_notify(&app.state).await;
    assert_eq!(report.account_user, None, "an unknown canary user falls back to public pages");
    assert!(json!(report.status) != json!("ok"));
}
