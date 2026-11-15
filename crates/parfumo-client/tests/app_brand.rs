//! The page of a brand through the application interface, against answers recorded on the
//! real site (`tests/fixtures/app_*.json`). A local fake serves them and records what is asked.

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
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient};

/// `GET /brand_details?b_id=253&page=1&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1`, trimmed to five rows.
const BRAND_DETAILS: &str = include_str!("fixtures/app_brand_details.json");
/// `GET /search?filter=creed`: two brands, Creed (253) and Goose Creek (15598).
const SEARCH_CREED: &str = include_str!("fixtures/app_search_creed.json");
/// Not recorded, written in the recorded shape: a brand whose slug is made of digits, as `4711` is on the site.
const SEARCH_DIGITS: &str =
    r#"{"notes":[],"brands":[{"b_id":"99","b_name":"1234","b_name_url":"1234","b_nr_of_perfums":3}],"similar_queries":false,"perfumes":[]}"#;
/// Not recorded either: an answer without a brand.
const NO_BRAND: &str = r#"{"b":null,"p":{"matches":0,"rows":[]}}"#;
/// What the interface answers, under status 200, for a number no brand has (`docs/APP_API.md`).
const BRAND_NOT_FOUND: &str = r#"{"error":"Brand not found"}"#;

/// Not recorded, written in the recorded shape: a search that finds one brand, named
/// `name`, under a number the fake has no brand for.
fn search_one(b_id: u64, name: &str) -> String {
    format!(
        r#"{{"notes":[],"brands":[{{"b_id":"{b_id}","b_name":"{name}","b_name_url":"{name}","b_nr_of_perfums":3}}],"similar_queries":false,"perfumes":[]}}"#
    )
}

/// The web page of another brand (Dior: France, founded in 1946), served as the page of
/// Creed: only its country and its founding year are read.
const BRAND_PAGE: &str = include_str!("fixtures/brand.html");
/// The page of the web site the country of the recorded brand is read on.
const WEB_PAGE_OF_CREED: &str = "/Perfumes/Creed";

const FIRST_PAGE_OF_CREED: &str = "/app/v1/brand_details?b_id=253&page=1&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1";

struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// What `/brand_details` answers.
    details: String,
    /// Whether the session token is refused from now on.
    refuse: AtomicBool,
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
    // The fake has one web page, the one of the recorded brand: whatever else is left to
    // the web site is not found there.
    if uri.path() == WEB_PAGE_OF_CREED {
        return (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], BRAND_PAGE).into_response();
    }
    if !uri.path().starts_with("/app/v1/") {
        return (StatusCode::NOT_FOUND, "<html><body>Not found</body></html>").into_response();
    }
    if uri.path() == "/app/v1/auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#);
    }
    if site.refuse.load(Ordering::SeqCst) || headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    match uri.path() {
        "/app/v1/search" if asked.ends_with("filter=1234") => json(SEARCH_DIGITS),
        // Brands the search knows and the fake has nothing about, each in its own way.
        "/app/v1/search" if asked.ends_with("filter=Ghost") => json(&search_one(999_999, "Ghost")),
        "/app/v1/search" if asked.ends_with("filter=Gone") => json(&search_one(404_404, "Gone")),
        "/app/v1/search" if asked.ends_with("filter=Other") => json(&search_one(777, "Other")),
        "/app/v1/search" if asked.ends_with("filter=Refused") => json(&search_one(500_500, "Refused")),
        "/app/v1/search" => json(SEARCH_CREED),
        "/app/v1/brand_details" if asked.contains("b_id=999999&") => json(NO_BRAND),
        "/app/v1/brand_details" if asked.contains("b_id=500500&") => json(BRAND_NOT_FOUND),
        // Assumed: another way the interface may tell that no brand has a number.
        "/app/v1/brand_details" if asked.contains("b_id=404404&") => (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response(),
        // Assumed too: an answer about another brand than the one asked for.
        "/app/v1/brand_details" if asked.contains("b_id=777&") => json(&site.details),
        // The recorded brand, under the number asked for.
        "/app/v1/brand_details" => {
            let b_id = asked.split(['?', '&']).find_map(|p| p.strip_prefix("b_id=")).unwrap_or_default();
            json(&site.details.replace(r#""id": "253""#, &format!(r#""id": "{b_id}""#)))
        }
        _ => (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response(),
    }
}

/// A client of a fake site whose `/brand_details` answers `details`.
async fn client_of(details: &str) -> (ParfumoClient, Arc<Site>) {
    let site = Arc::new(Site { asked: Mutex::new(Vec::new()), details: details.to_string(), refuse: AtomicBool::new(false) });
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
    assert_eq!(site.take(), ["/app/v1/search?filter=Creed", FIRST_PAGE_OF_CREED, WEB_PAGE_OF_CREED], "the web page is read for the country");

    assert_eq!((b.brand.id.as_str(), b.brand.numeric_id, b.brand.name.as_str()), ("Creed", Some(253), "Creed"));
    assert_eq!(b.brand.url, format!("{}/Perfumes/Creed", c.base_url()));
    // The interface names the country by a number only, and has no founding year for this
    // brand ("0"): both are read on the web page of the brand.
    assert_eq!((b.country.as_deref(), b.founded), (Some("France"), Some(1946)));
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);

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
    // (The fake has no record: the web page is then asked, which it has not either.)
    let _ = c.get_perfume("Creed/absolu-aventus").await;
    assert_eq!(site.take(), ["/app/v1/perfume_details/225233", "/Perfumes/Creed/absolu-aventus"]);
}

#[tokio::test]
async fn an_address_a_path_and_a_name_reach_the_brand_and_the_page_asked_is_passed_on() {
    let (c, site) = logged_in().await;
    // An address of a perfume of the brand, with a query: the brand is its first part.
    c.get_brand(&format!("{}/Perfumes/Creed/aventus?x=1", c.base_url()), 3).await.unwrap();
    assert_eq!(
        site.take(),
        ["/app/v1/search?filter=Creed", "/app/v1/brand_details?b_id=253&page=3&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1", WEB_PAGE_OF_CREED]
    );

    // The letter case of a slug does not matter.
    for (reference, words) in [("/Perfumes/creed/", "creed"), ("Perfumes/CREED", "CREED")] {
        let b = c.get_brand(reference, 0).await.unwrap();
        assert_eq!(
            site.take(),
            [format!("/app/v1/search?filter={words}"), FIRST_PAGE_OF_CREED.to_string(), WEB_PAGE_OF_CREED.to_string()],
            "{reference}"
        );
        assert_eq!((b.brand.id.as_str(), b.perfumes.page), ("Creed", 1), "the slug is the one of the site, the first page is 1");
    }

    // A name, and the slug the web code would guess from it, where the real one is `goose-creek`.
    for reference in ["goose creek", "Goose_Creek"] {
        c.get_brand(reference, 1).await.unwrap();
        let asked = site.take();
        // The fake answers the recorded brand under any number: its web page is the one read.
        assert_eq!(asked.len(), 3, "{asked:?}");
        assert!(asked[0].eq_ignore_ascii_case("/app/v1/search?filter=goose+creek"), "{asked:?}");
        assert_eq!(asked[1..], ["/app/v1/brand_details?b_id=15598&page=1&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1", WEB_PAGE_OF_CREED]);
    }
}

#[tokio::test]
async fn digits_are_a_slug_or_a_name_like_any_other_and_never_the_number_of_a_brand() {
    let (c, site) = logged_in().await;
    // 253 is the number of Creed in the database, and no brand has this slug or this name:
    // the web site decides, as for any name the search does not know.
    assert!(c.get_brand("253", 1).await.is_err());
    let asked = site.take();
    assert_eq!(&asked[..2], ["/app/v1/search?filter=253", "/Perfumes/253"], "{asked:?}");
    assert!(asked.iter().all(|a| !a.starts_with("/app/v1/brand_details")), "the page of Creed is not an answer: {asked:?}");

    // On the web site `4711` is the slug of a brand: such a brand is read under the number
    // the search gives for it, not under the number its digits spell.
    c.get_brand("1234", 1).await.unwrap();
    assert_eq!(
        site.take(),
        ["/app/v1/search?filter=1234", "/app/v1/brand_details?b_id=99&page=1&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1", WEB_PAGE_OF_CREED]
    );
}

#[tokio::test]
async fn a_brand_the_interface_has_nothing_about_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    // An answer without a brand, a refusal in JSON under status 200, a 404, and the page
    // of another brand than asked: none of them says that Parfumo has no such brand. The
    // fake has no web page: the outcome is the one of the web code, which is what is asked.
    for (reference, b_id) in [("Ghost", 999_999), ("Refused", 500_500), ("Gone", 404_404), ("Other", 777)] {
        assert!(c.get_brand(reference, 1).await.is_err(), "{reference}");
        let asked = site.take();
        assert_eq!(
            &asked[..3],
            [
                format!("/app/v1/search?filter={reference}"),
                format!("/app/v1/brand_details?b_id={b_id}&page=1&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1"),
                format!("/Perfumes/{reference}")
            ],
            "{reference}"
        );
    }
    assert!(c.has_app_session(), "none of this is a refusal of the session");
}

#[tokio::test]
async fn the_brand_of_a_reference_is_looked_up_once() {
    let (c, site) = logged_in().await;
    // A listing is read page after page: only the first one costs a search.
    let first = c.get_brand("creed", 1).await.unwrap();
    c.get_brand("creed", 2).await.unwrap();
    let page = |n: u32| format!("/app/v1/brand_details?b_id=253&page={n}&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1");
    // Each page read through the interface is followed by the web page that shows the country.
    let web = || WEB_PAGE_OF_CREED.to_string();
    assert_eq!(site.take(), ["/app/v1/search?filter=creed".to_string(), page(1), web(), page(2), web()]);

    // The id and the address handed back are those of the site: they name the brand without a search.
    assert_eq!(first.brand.id, "Creed");
    c.get_brand(&first.brand.id, 3).await.unwrap();
    c.get_brand(&first.brand.url, 4).await.unwrap();
    assert_eq!(site.take(), [page(3), web(), page(4), web()]);

    // What the search does not know is not asked of it twice either: the web site is read at once.
    for _ in 0..2 {
        assert!(c.get_brand("No Such Brand", 2).await.is_err());
    }
    let asked = site.take();
    let searches: Vec<&String> = asked.iter().filter(|a| a.starts_with("/app/v1/")).collect();
    assert_eq!(searches, ["/app/v1/search?filter=No+Such+Brand"], "{asked:?}");
    assert_eq!(asked.iter().filter(|a| *a == "/Perfumes/No_Such_Brand?current_page=2").count(), 2, "{asked:?}");
}

#[tokio::test]
async fn a_brand_the_search_does_not_know_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    // The fake has no web page: the outcome is the one of the web code, which is what was asked of it.
    for (reference, search, page) in [
        ("No Such Brand", "/app/v1/search?filter=No+Such+Brand", "/Perfumes/No_Such_Brand"),
        // An address names a brand by its slug alone: a name that happens to fit is not enough.
        ("/Perfumes/Goose_Creek", "/app/v1/search?filter=Goose+Creek", "/Perfumes/Goose_Creek"),
        // Nor is a number: in an address, digits are a slug (`/Perfumes/4711`), never the brand number.
        ("/Perfumes/253", "/app/v1/search?filter=253", "/Perfumes/253"),
    ] {
        assert!(c.get_brand(reference, 1).await.is_err());
        let asked = site.take();
        assert_eq!(&asked[..2], [search, page], "{reference}");
        assert!(asked.iter().all(|a| !a.starts_with("/app/v1/brand_details")), "{asked:?}");
    }

    // What is not a reference to a brand is refused as before, and nothing is asked.
    for reference in ["/Users/someone", "/Perfumes/", " ", "../x", "https://example.org/Perfumes/Creed"] {
        assert!(matches!(c.get_brand(reference, 1).await.unwrap_err(), ClientError::InvalidInput(_)), "{reference}");
    }
    assert_eq!(site.asked(), Vec::<String>::new());
}

#[tokio::test]
async fn an_answer_of_another_shape_is_reported_and_one_without_its_count_is_left_to_the_web_site() {
    // Not recorded: the recorded answer without its listing, then without the count of its listing.
    let no_listing = BRAND_DETAILS.replace(r#""p": {"#, r#""q": {"#);
    let no_count = BRAND_DETAILS.replace(r#""matches": 104"#, r#""found": 104"#);
    assert!(no_listing != BRAND_DETAILS && no_count != BRAND_DETAILS);

    let (c, _site) = client_of(&no_listing).await;
    c.app_login("someone@example.org", "pw").await.unwrap();
    assert_eq!(c.get_brand("Creed", 1).await.unwrap_err(), ClientError::ParserOutdated("/app/v1/brand_details: no perfumes".into()));

    // A count the answer does not carry is not a count of zero, and the web page always
    // tells how many perfumes a brand has: it answers the whole read (the fake serves the
    // page of another brand there, Dior).
    let (c, site) = client_of(&no_count).await;
    c.app_login("someone@example.org", "pw").await.unwrap();
    site.take();
    let b = c.get_brand("Creed", 1).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/search?filter=Creed", FIRST_PAGE_OF_CREED, WEB_PAGE_OF_CREED]);
    assert_eq!((b.brand.id.as_str(), b.perfumes.total_items), ("Dior", Some(344)), "the brand and the count of the web page");
}

#[tokio::test]
async fn a_refused_token_stops_the_calls_and_the_web_site_serves_the_next_ones() {
    let (c, site) = logged_in().await;
    site.refuse.store(true, Ordering::SeqCst);
    // One refusal, then the web page answers the same read at once: no error about the session.
    let _ = c.get_brand("Creed", 1).await;
    let asked = site.take();
    assert_eq!(&asked[..2], ["/app/v1/search?filter=Creed", "/Perfumes/Creed"], "{asked:?}");
    assert!(asked[1..].iter().all(|a| !a.starts_with("/app/v1/")), "nothing is asked of the interface after the refusal: {asked:?}");
    assert!(!c.has_app_session(), "the refused token is forgotten");

    // The next reads go to the web site only.
    let _ = c.get_brand("Creed", 1).await;
    let asked = site.take();
    assert_eq!(asked[0], "/Perfumes/Creed");
    assert!(asked.iter().all(|a| !a.starts_with("/app/v1/")), "{asked:?}");
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
    // The fake serves the page of another brand (Dior) as the web page of Creed.
    let b = c.get_brand("Creed", 2).await.unwrap();
    assert_eq!(b.brand.id, "Dior");
    assert_eq!(site.asked(), ["/Perfumes/Creed?current_page=2"], "one web page, and nothing of the interface");
}

/// The country is read on the web page of the brand. When that page cannot be read, the
/// brand of the interface is given all the same, and says what it lacks.
#[tokio::test]
async fn a_brand_whose_web_page_cannot_be_read_is_given_without_its_country_and_says_so() {
    let details = BRAND_DETAILS.replace(r#""name_url": "Creed""#, r#""name_url": "Elsewhere""#);
    assert_ne!(details, BRAND_DETAILS);
    let (c, site) = client_of(&details).await;
    c.app_login("someone@example.org", "pw").await.unwrap();
    site.take();
    let b = c.get_brand("Creed", 1).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/search?filter=Creed", FIRST_PAGE_OF_CREED, "/Perfumes/Elsewhere"]);
    assert_eq!((b.brand.id.as_str(), b.country, b.perfumes.items.len()), ("Elsewhere", None, 5));
    assert_eq!(b.warnings, ["country unavailable: not found: /Perfumes/Elsewhere"]);
}

/// The rows of a page, for a listing read in one go after its first page: the web page,
/// which only completes the head, is not read again.
#[tokio::test]
async fn the_rows_of_a_following_page_are_read_without_the_web_page() {
    let (c, site) = logged_in().await;
    c.get_brand("Creed", 1).await.unwrap();
    site.take();
    let rows = c.get_brand_rows("Creed", 2).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/brand_details?b_id=253&page=2&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1"]);
    assert_eq!((rows.country, rows.perfumes.items.len(), rows.warnings.len()), (None, 5, 0), "the head is the one of the first page");
}
