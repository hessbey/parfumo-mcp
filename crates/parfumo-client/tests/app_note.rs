//! The page of a fragrance note is always read from the web site, with or without a session
//! of the application interface. `GET /fragrance_notes` gives no address for a note, for
//! its categories or for the brand of its perfumes, so its answer cannot name a perfume the
//! way every other tool takes it; and a name designates another note there than on the web
//! site ("Candied bergamot peel"). The answers recorded from it
//! (`tests/fixtures/app_note_*.json`) are served by the fake all the same: what the tests
//! check is that nothing asks for them.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use parfumo_client::{ClientConfig, GlobalLimiter, NotePage, ParfumoClient};

const NOTE: &str = include_str!("fixtures/app_note_note.json");
const CATEGORY: &str = include_str!("fixtures/app_note_category.json");
const SEARCH: &str = include_str!("fixtures/app_note_search.json");
const OVERVIEW: &str = include_str!("fixtures/app_note_overview.json");
const WEB_NOTE: &str = include_str!("fixtures/note.html");
const WEB_CATEGORY: &str = include_str!("fixtures/note_category.html");

#[derive(Default)]
struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// Whether a page of the web site was asked with the session token.
    token_on_web: AtomicBool,
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked.clone());
    let json = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    let html = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], text.to_string()).into_response();
    let token = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if !uri.path().starts_with("/app/v1/") {
        site.token_on_web.fetch_or(token.is_some(), Ordering::SeqCst);
        // The web site: one note and one category, whatever the slug.
        return match uri.path() {
            path if path.starts_with("/Fragrance_Notes/") => html(WEB_CATEGORY),
            path if path.starts_with("/Fragrance_Note/") => html(WEB_NOTE),
            _ => (StatusCode::NOT_FOUND, "not here").into_response(),
        };
    }
    if uri.path() == "/app/v1/auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#);
    }
    if token != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    match uri.path() {
        "/app/v1/fragrance_notes" if asked.contains("feature=search") => json(SEARCH),
        "/app/v1/fragrance_notes" if asked.contains("feature=overview") => json(OVERVIEW),
        "/app/v1/fragrance_notes" if asked.contains("feature=category") => json(CATEGORY),
        "/app/v1/fragrance_notes" => json(NOTE),
        _ => (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response(),
    }
}

/// A client of a fake site, logged in to its application interface or not.
async fn client(logged_in: bool) -> (ParfumoClient, Arc<Site>) {
    let site = Arc::new(Site::default());
    let app = Router::new().fallback(handler).with_state(site.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let cfg = ClientConfig {
        base_url: base,
        min_interval: Duration::ZERO,
        jitter: Duration::ZERO,
        timeout: Duration::from_secs(5),
        ..ClientConfig::default()
    };
    let client = ParfumoClient::new(cfg, GlobalLimiter::new(0), None).unwrap();
    if logged_in {
        client.app_login("someone@example.org", "pw").await.unwrap();
    }
    site.asked.lock().unwrap().clear();
    (client, site)
}

/// What was asked since the last look.
fn asked(site: &Site) -> Vec<String> {
    std::mem::take(&mut *site.asked.lock().unwrap())
}

/// The references the interface used to answer by itself (a number, a name one note bears,
/// in any letter case, the address of a top-level category) and a name that designates
/// another note there than on the web site, each with the web page it names.
const REFERENCES: [(&str, &str); 5] = [
    ("311", "/Fragrance_Note/311"),
    ("Vanilla", "/Fragrance_Note/Vanilla"),
    ("vanilla", "/Fragrance_Note/vanilla"),
    ("Candied bergamot peel", "/Fragrance_Note/Candied_bergamot_peel"),
    ("/Fragrance_Notes/Citric_notes", "/Fragrance_Notes/Citric_notes"),
];

/// A page that names its note, its categories and its perfumes the way every other tool
/// takes them: by a slug and an address.
fn assert_addresses(n: &NotePage, reference: &str) {
    assert!(!n.note.id.is_empty() && !n.note.url.is_empty(), "{reference}: {:?}", n.note);
    assert!(!n.categories.is_empty() && n.categories.iter().all(|c| !c.id.is_empty() && !c.url.is_empty()), "{reference}: {:?}", n.categories);
    assert_eq!(n.perfumes.items.len(), 20, "{reference}");
    assert!(n.perfumes.items.iter().all(|p| p.id.contains('/') && p.url.contains("/Perfumes/")), "{reference}");
    assert!(n.perfumes.total_items.is_some() && n.perfumes.total_pages.is_some(), "{reference}");
}

#[tokio::test]
async fn a_note_is_read_from_the_web_site_even_with_a_session() {
    let (with_session, site) = client(true).await;
    let (without, _) = client(false).await;
    for (reference, web_page) in REFERENCES {
        let n = with_session.get_note(reference, 1).await.unwrap();
        assert_eq!(asked(&site), [web_page], "{reference}: the web page, and nothing of the interface");
        assert_addresses(&n, reference);
        // The same reference designates the same note whether an account is linked or not.
        // (The two fakes differ by their address alone, which the pages do not repeat.)
        assert_eq!(n, without.get_note(reference, 1).await.unwrap(), "{reference}");
    }
    // The following pages are those of the web site too.
    let _ = with_session.get_note("311", 4).await;
    assert_eq!(asked(&site), ["/Fragrance_Note/311?current_page=4"]);
    assert!(!site.token_on_web.load(Ordering::SeqCst), "the session token is for the application interface only");
    assert!(with_session.has_app_session());
}

#[tokio::test]
async fn the_page_of_a_note_of_the_web_site_has_the_addresses_the_interface_lacks() {
    let (c, site) = client(true).await;
    let n = c.get_note("Bergamot", 1).await.unwrap();
    assert_eq!(asked(&site), ["/Fragrance_Note/Bergamot"]);
    assert_eq!((n.note.id.as_str(), n.note.numeric_id, n.is_category), ("Bergamot", Some(311), false));
    assert_eq!(n.note.url, "https://www.parfumo.com/Fragrance_Note/Bergamot");
    let categories: Vec<&str> = n.categories.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(categories, ["Bergamot", "Citric_notes"]);
    assert_eq!((n.perfumes.total_items, n.perfumes.total_pages), (Some(43518), Some(50)));
}
