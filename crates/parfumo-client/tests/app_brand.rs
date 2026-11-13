//! The page of a brand through the application interface, against answers recorded on the
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
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient};

/// `GET /brand_details?b_id=253&page=1&per_page=20&g_f=1&g_m=1&g_u=1`, trimmed to five rows.
const BRAND_DETAILS: &str = include_str!("fixtures/app_brand_details.json");
/// `GET /search?filter=creed`: two brands, Creed (253) and Goose Creek (15598).
const SEARCH_CREED: &str = include_str!("fixtures/app_search_creed.json");
/// Not recorded, written in the recorded shape: a brand whose slug is made of digits, as `4711` is on the site.
const SEARCH_DIGITS: &str =
    r#"{"notes":[],"brands":[{"b_id":"99","b_name":"1234","b_name_url":"1234","b_nr_of_perfums":3}],"similar_queries":false,"perfumes":[]}"#;
/// Not recorded either: what is assumed of a number no brand has.
const NO_BRAND: &str = r#"{"b":null,"p":{"matches":0,"rows":[]}}"#;

const FIRST_PAGE_OF_CREED: &str = "/app/v1/brand_details?b_id=253&page=1&per_page=20&g_f=1&g_m=1&g_u=1";

struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// What `/brand_details` answers.
    details: String,
}

impl Site {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    /// What was asked since the last call, which is forgotten.
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked.clone());
    let json = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    // The fake has no web page: whatever is left to the web site is not found there.
    if !uri.path().starts_with("/app/v1/") {
        return (StatusCode::NOT_FOUND, "<html><body>Not found</body></html>").into_response();
    }
    if uri.path() == "/app/v1/auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#);
    }
    if headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    match uri.path() {
        "/app/v1/search" if asked.ends_with("filter=1234") => json(SEARCH_DIGITS),
        "/app/v1/search" => json(SEARCH_CREED),
        "/app/v1/brand_details" if asked.contains("b_id=999999&") => json(NO_BRAND),
        "/app/v1/brand_details" => json(&site.details),
        _ => (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response(),
    }
}

/// A client of a fake site whose `/brand_details` answers `details`.
async fn client_of(details: &str) -> (ParfumoClient, Arc<Site>) {
    let site = Arc::new(Site { asked: Mutex::new(Vec::new()), details: details.to_string() });
    let app = Router::new().fallback(handler).with_state(site.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let cfg = ClientConfig {
        base_url: base,
        min_interval: Duration::ZERO,
        jitter: Duration::ZERO,
        timeout: Duration::from_secs(5),
        max_retries: 0,
        ..ClientConfig::default()
    };
    (ParfumoClient::new(cfg, GlobalLimiter::new(0), None).unwrap(), site)
}

/// A client logged in to the application interface of a fake site that answers the recorded brand.
async fn logged_in() -> (ParfumoClient, Arc<Site>) {
    let (client, site) = client_of(BRAND_DETAILS).await;
    client.app_login("someone@example.org", "pw").await.unwrap();
    site.take();
    (client, site)
}

#[tokio::test]
async fn a_brand_named_by_its_slug_is_found_by_the_search_then_read_in_one_call() {
    let (c, site) = logged_in().await;
    let b = c.get_brand("Creed", 1).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/search?filter=Creed", FIRST_PAGE_OF_CREED], "no web page is read");

    assert_eq!((b.brand.id.as_str(), b.brand.numeric_id, b.brand.name.as_str()), ("Creed", Some(253), "Creed"));
    assert_eq!(b.brand.url, format!("{}/Perfumes/Creed", c.base_url()));
    // The interface names the country by a number only, and has no founding year for this brand ("0").
    assert_eq!((b.country, b.founded), (None, None));

    assert_eq!((b.perfumes.page, b.perfumes.total_items, b.perfumes.total_pages), (1, Some(104), Some(6)));
    let names: Vec<&str> = b.perfumes.items.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["2000 Fleurs", "Absolu Aventus", "Aventus", "Aventus Cologne", "Bayrhum Vetiver"], "the order of the interface: by name");
    let absolu = &b.perfumes.items[1];
    assert_eq!((absolu.id.as_str(), absolu.p_id), ("Creed/absolu-aventus", Some(225233)));
    assert_eq!((absolu.brand.as_deref(), absolu.year, absolu.gender.as_deref()), (Some("Creed"), Some(2023), Some("unisex")));
    assert_eq!((absolu.rating, absolu.rating_count, absolu.reviews_count), (Some(8.5), Some(1492), Some(86)));
    assert_eq!(absolu.url, format!("{}/Perfumes/Creed/absolu-aventus", c.base_url()));
    assert!(absolu.image_url.as_deref().unwrap().starts_with("https://media.parfumo.com/perfumes/"));
    let genders: Vec<_> = b.perfumes.items.iter().filter_map(|p| p.gender.as_deref()).collect();
    assert_eq!(genders, ["women", "unisex", "men", "unisex", "men"], "German genders are translated, as on the web pages");
    let bayrhum = &b.perfumes.items[4];
    assert_eq!((bayrhum.year, bayrhum.rating, bayrhum.rating_count, bayrhum.reviews_count), (None, Some(7.0), Some(1), None), "zeros are absences");

    // The rows gave the numbers of their perfumes: one is then read without being looked up.
    let _ = c.get_perfume("Creed/absolu-aventus").await;
    assert_eq!(site.take(), ["/app/v1/perfume_details/225233"]);
}

#[tokio::test]
async fn an_address_a_path_and_a_name_reach_the_brand_and_the_page_asked_is_passed_on() {
    let (c, site) = logged_in().await;
    // An address of a perfume of the brand, with a query: the brand is its first part.
    c.get_brand(&format!("{}/Perfumes/Creed/aventus?x=1", c.base_url()), 3).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/search?filter=Creed", "/app/v1/brand_details?b_id=253&page=3&per_page=20&g_f=1&g_m=1&g_u=1"]);

    // The letter case of a slug does not matter.
    for (reference, words) in [("/Perfumes/creed/", "creed"), ("Perfumes/CREED", "CREED")] {
        let b = c.get_brand(reference, 0).await.unwrap();
        assert_eq!(site.take(), [format!("/app/v1/search?filter={words}"), FIRST_PAGE_OF_CREED.to_string()], "{reference}");
        assert_eq!((b.brand.id.as_str(), b.perfumes.page), ("Creed", 1), "the slug is the one of the site, the first page is 1");
    }

    // A name, and the slug the web code would guess from it, where the real one is `goose-creek`.
    for reference in ["goose creek", "Goose_Creek"] {
        c.get_brand(reference, 1).await.unwrap();
        let asked = site.take();
        assert_eq!(asked.len(), 2, "{asked:?}");
        assert!(asked[0].eq_ignore_ascii_case("/app/v1/search?filter=goose+creek"), "{asked:?}");
        assert_eq!(asked[1], "/app/v1/brand_details?b_id=15598&page=1&per_page=20&g_f=1&g_m=1&g_u=1");
    }
}

#[tokio::test]
async fn a_number_is_the_number_of_the_brand_unless_a_brand_is_named_by_these_digits() {
    let (c, site) = logged_in().await;
    c.get_brand("253", 1).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/search?filter=253", FIRST_PAGE_OF_CREED], "no brand has this slug: the number itself");

    // On the web site `4711` is the slug of a brand, not the brand number 4711.
    c.get_brand("1234", 1).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/search?filter=1234", "/app/v1/brand_details?b_id=99&page=1&per_page=20&g_f=1&g_m=1&g_u=1"]);

    // An answer without a brand is not an empty page.
    assert_eq!(c.get_brand("999999", 1).await.unwrap_err(), ClientError::NotFound("brand 999999".into()));
    assert!(site.take().iter().all(|a| a.starts_with("/app/v1/")), "and the web site is not asked for it");
}

#[tokio::test]
async fn a_brand_the_search_does_not_know_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    // The fake has no web page: the outcome is the one of the web code, which is what was asked of it.
    for (reference, search, page) in [
        ("No Such Brand", "/app/v1/search?filter=No+Such+Brand", "/Perfumes/No_Such_Brand"),
        // An address names a brand by its slug alone: a name that happens to fit is not enough.
        ("/Perfumes/Goose_Creek", "/app/v1/search?filter=Goose+Creek", "/Perfumes/Goose_Creek"),
    ] {
        assert!(c.get_brand(reference, 1).await.is_err());
        let asked = site.take();
        assert_eq!(&asked[..2], [search, page], "{reference}");
        assert!(asked.iter().all(|a| !a.starts_with("/app/v1/brand_details")), "{asked:?}");
    }

    // What is not a reference to a brand is refused as before, and nothing is asked.
    for reference in ["/Users/someone", " ", "../x"] {
        assert!(matches!(c.get_brand(reference, 1).await.unwrap_err(), ClientError::InvalidInput(_)), "{reference}");
    }
    assert_eq!(site.asked(), Vec::<String>::new());
}

#[tokio::test]
async fn a_founding_year_is_kept_and_no_page_is_promised_beyond_a_thousand_rows() {
    // The recorded brand has neither: the same answer, with a founding year and more perfumes than the interface serves.
    let details = BRAND_DETAILS.replace(r#""founding_year": "0""#, r#""founding_year": "1760""#).replace(r#""matches": 104"#, r#""matches": 2500"#);
    assert_ne!(details, BRAND_DETAILS);
    let (c, _site) = client_of(&details).await;
    c.app_login("someone@example.org", "pw").await.unwrap();
    let b = c.get_brand("Creed", 50).await.unwrap();
    assert_eq!(b.founded, Some(1760));
    assert_eq!((b.perfumes.page, b.perfumes.total_items, b.perfumes.total_pages), (50, Some(2500), Some(50)));
}

#[tokio::test]
async fn without_a_session_of_the_application_only_the_web_site_is_asked() {
    let (c, site) = client_of(BRAND_DETAILS).await;
    assert!(c.get_brand("Creed", 2).await.is_err());
    let asked = site.asked();
    assert_eq!(asked[0], "/Perfumes/Creed?current_page=2");
    assert!(asked.iter().all(|a| !a.starts_with("/app/v1/")), "{asked:?}");
}
