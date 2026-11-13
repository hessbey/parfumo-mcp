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
const PERFUME_DETAILS: &str = include_str!("fixtures/app_perfume_details.json");

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
        "/app/v1/perfumes" if asked.contains("b_id=") => json(PERFUMES_BRAND),
        "/app/v1/perfumes" => json(PERFUMES_TEXT),
        path if path.starts_with("/app/v1/perfume_details/") => json(PERFUME_DETAILS),
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

#[tokio::test]
async fn a_perfume_record_is_read_by_number_with_every_part_of_the_web_record() {
    let (c, site) = logged_in().await;
    let p = c.get_perfume("9438").await.unwrap();
    assert_eq!(site.asked.lock().unwrap().as_slice(), ["/app/v1/perfume_details/9438"], "one call, no web page");

    assert_eq!((p.id.as_str(), p.p_id, p.name.as_str()), ("Creed/aventus", Some(9438), "Aventus"));
    assert_eq!((p.brand.id.as_str(), p.brand.numeric_id, p.brand.name.as_str()), ("Creed", Some(253), "Creed"));
    assert!(p.brand.url.ends_with("/Perfumes/Creed"), "{}", p.brand.url);
    assert_eq!((p.year, p.gender.as_deref(), p.concentration.as_deref()), (Some(2010), Some("men"), None));
    assert_eq!(p.perfumers.len(), 1);
    assert_eq!((p.perfumers[0].id.as_str(), p.perfumers[0].numeric_id), ("JeanChristophe_Herault", Some(509)));
    assert!(p.perfumers[0].url.ends_with("/Perfumers/JeanChristophe_Herault"));
    assert_eq!(p.notes.top.first().map(String::as_str), Some("Bergamot"));
    assert_eq!(p.notes.heart.first().map(String::as_str), Some("Pineapple"));
    assert_eq!(p.notes.base.first().map(String::as_str), Some("Birch"));
    assert!(p.notes.flat.is_empty(), "the empty placeholder of the site is not a note");

    // The five scales, out of 10, with their votes.
    let scales: Vec<(&str, f32, Option<u32>)> = p.ratings.iter().map(|r| (r.dimension.as_str(), r.average, r.votes)).collect();
    assert_eq!(
        scales,
        [
            ("scent", 8.4, Some(8836)),
            ("durability", 7.3, Some(8130)),
            ("sillage", 7.3, Some(8058)),
            ("bottle", 7.7, Some(7854)),
            ("pricing", 5.5, Some(5946))
        ]
    );

    // The four charts, strongest share first, and the accords drawn from the first.
    let groups: Vec<(&str, Option<u32>, &str)> = p.classification.iter().map(|g| (g.group.as_str(), g.votes, g.shares[0].label.as_str())).collect();
    assert_eq!(
        groups,
        [
            ("Type", Some(6884), "Fresh"),
            ("Season", Some(5733), "Spring"),
            ("Occasion", Some(7712), "Daily"),
            ("Audience & Style", Some(3848), "Masculine")
        ]
    );
    assert_eq!(p.classification[0].shares[0].percent, 19.2);
    assert_eq!(&p.accords[..3], ["Fresh", "Fruity", "Citrus"]);

    assert_eq!((p.reviews_count, p.statements_count), (Some(632), Some(1721)));
    assert!(p.description.as_deref().unwrap().starts_with("A popular perfume by Creed"));
    assert_eq!(p.url, "https://www.parfumo.com/Perfumes/Creed/aventus");
    assert!(p.image_url.is_some() && p.warnings.is_empty());
}

#[tokio::test]
async fn a_perfume_named_by_its_id_is_found_through_its_brand_then_remembered() {
    let (c, site) = logged_in().await;
    // Never met in a listing: the brand is looked up, then the perfume among its own.
    c.get_perfume("/Perfumes/Creed/absolu-aventus").await.unwrap();
    let asked = site.asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        [
            "/app/v1/search?filter=Creed",
            "/app/v1/perfumes?page=1&per_page=20&filter=absolu+aventus&b_id=253&g_f=1&g_m=1&g_u=1",
            "/app/v1/perfume_details/225233"
        ]
    );
    // Met once, known for good, whatever the letter case of the reference.
    c.get_perfume("creed/Absolu-Aventus").await.unwrap();
    assert_eq!(site.asked.lock().unwrap().len(), 4, "only the record itself is asked again");

    // A perfume the brand does not have is not found, and nothing else is asked.
    let before = site.asked.lock().unwrap().len();
    assert_eq!(c.get_perfume("Creed/no-such-perfume").await.unwrap_err(), ClientError::NotFound("Creed/no-such-perfume".into()));
    assert_eq!(site.asked.lock().unwrap().len(), before + 2);
    assert!(matches!(c.get_perfume("not a reference").await.unwrap_err(), ClientError::InvalidInput(_)));
}
