//! Database reads through the application interface, against answers recorded on the
//! real site (`tests/fixtures/app_*.json`). A local fake serves them and records what is asked.

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
use parfumo_client::{ClientConfig, ClientError, Gender, GlobalLimiter, ParfumoClient, SearchQuery, SearchSort};

const PERFUMES_BRAND: &str = include_str!("fixtures/app_perfumes_brand.json");
const PERFUMES_TEXT: &str = include_str!("fixtures/app_perfumes_text.json");
const SEARCH_CREED: &str = include_str!("fixtures/app_search_creed.json");

#[derive(Default)]
struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked.clone());
    let json = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    if uri.path() == "/app/v1/auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#);
    }
    if headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    match uri.path() {
        "/app/v1/search" => json(SEARCH_CREED),
        "/app/v1/perfumes" if asked.contains("filter=") => json(PERFUMES_TEXT),
        "/app/v1/perfumes" => json(PERFUMES_BRAND),
        _ => (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response(),
    }
}

/// A client logged in to the application interface of a fake site.
async fn logged_in() -> (ParfumoClient, Arc<Site>) {
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
    client.app_login("someone@example.org", "pw").await.unwrap();
    site.asked.lock().unwrap().clear();
    (client, site)
}

#[tokio::test]
async fn a_search_by_brand_resolves_the_brand_then_reads_the_filtered_listing() {
    let (c, site) = logged_in().await;
    let q = SearchQuery {
        brand: Some("creed".into()),
        genders: vec![Gender::Men, Gender::Unisex],
        sort: SearchSort::ScentDesc,
        ..SearchQuery::default()
    };
    let page = c.search_perfumes(&q, 2).await.unwrap();

    let asked = site.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(asked[0], "/app/v1/search?filter=creed");
    assert_eq!(asked[1], "/app/v1/perfumes?page=2&per_page=20&b_id=253&g_f=0&g_m=1&g_u=1&o=v_desc", "no web page is read");

    assert_eq!((page.page, page.total_items, page.total_pages), (2, Some(104), Some(6)));
    assert_eq!(page.items.len(), 5);
    let absolu = page.items.iter().find(|p| p.name == "Absolu Aventus").unwrap();
    assert_eq!(absolu.id, "Creed/absolu-aventus");
    assert_eq!(absolu.p_id, Some(225233));
    assert_eq!((absolu.brand.as_deref(), absolu.year, absolu.gender.as_deref()), (Some("Creed"), Some(2023), Some("unisex")));
    assert_eq!((absolu.rating, absolu.rating_count, absolu.reviews_count), (Some(8.5), Some(1492), Some(86)));
    assert!(absolu.url.ends_with("/Perfumes/Creed/absolu-aventus"), "{}", absolu.url);
    assert!(absolu.image_url.as_deref().unwrap().starts_with("https://media.parfumo.com/perfumes/"));
    // German genders are translated, as on the web pages.
    let genders: Vec<_> = page.items.iter().filter_map(|p| p.gender.clone()).collect();
    assert!(genders.iter().all(|g| ["men", "women", "unisex"].contains(&g.as_str())), "{genders:?}");
}

#[tokio::test]
async fn words_alone_ask_for_the_most_rated_first_and_an_unrated_perfume_has_no_rating() {
    let (c, site) = logged_in().await;
    let page = c.search_perfumes(&SearchQuery { text: Some(" sauvage ".into()), year: Some(2015), ..SearchQuery::default() }, 1).await.unwrap();
    assert_eq!(site.asked.lock().unwrap().as_slice(), ["/app/v1/perfumes?page=1&per_page=20&filter=sauvage&y_0=2015&g_f=1&g_m=1&g_u=1&o=nr_desc"]);
    assert_eq!(page.total_items, Some(151));
    let unrated = page.items.iter().find(|p| p.name == "Ambre Sauvage").unwrap();
    assert_eq!((unrated.rating, unrated.rating_count, unrated.year), (None, None, None), "zeros of the site are absences");
}

#[tokio::test]
async fn an_unknown_brand_is_refused_with_the_close_ones_and_no_listing_is_read() {
    let (c, site) = logged_in().await;
    let err = c.search_perfumes(&SearchQuery { brand: Some("Cre".into()), ..SearchQuery::default() }, 1).await.unwrap_err();
    assert_eq!(err, ClientError::InvalidInput("brand 'Cre' not found on Parfumo (close: Creed, Goose Creek)".into()));
    assert_eq!(site.asked.lock().unwrap().len(), 1);
}
