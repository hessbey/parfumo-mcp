//! Rankings through the application interface, against an answer recorded on the real site
//! (`tests/fixtures/app_rankings_year.json`). A local fake serves it and records what is
//! asked: the listing of a release year goes through the interface, the other rankings
//! keep reading the web pages.

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
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, Listing, ParfumoClient, PerfumeSummary, RankingKind};

/// `GET /perfumes?y_0=2020&page=1&per_page=20`, trimmed to five rows. Recorded without a
/// sort order: the rows are in alphabetical order.
const YEAR: &str = include_str!("fixtures/app_rankings_year.json");
const TOP_MEN: &str = include_str!("fixtures/top_men.html");
const TRENDS: &str = include_str!("fixtures/trends.html");

#[derive(Default)]
struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// Whether the site now refuses the token it gave.
    refuses: AtomicBool,
}

impl Site {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked.clone());
    let json = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    let html = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], text.to_string()).into_response();
    let path = uri.path();
    if path == "/app/v1/auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#);
    }
    if path.starts_with("/app/v1/") {
        if site.refuses.load(Ordering::SeqCst) || headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
            return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
        }
        return match path {
            // A year without any perfume, and a year with fewer perfumes than one page.
            "/app/v1/perfumes" if asked.contains("y_0=1801&") => json(r#"{"matches":0,"rows":[]}"#),
            "/app/v1/perfumes" if asked.contains("y_0=1950&") => json(&YEAR.replace("\"matches\": 5565", "\"matches\": 5")),
            // The recorded answer with its total as text and a row without the address of
            // its brand; without its total; and an answer that is not a listing.
            "/app/v1/perfumes" if asked.contains("y_0=1802&") => {
                json(&YEAR.replace("\"matches\": 5565", "\"matches\": \"5565\"").replace("\"b_name_url\": \"Rabanne\"", "\"b_name_url\": \"\""))
            }
            "/app/v1/perfumes" if asked.contains("y_0=1803&") => json(&YEAR.replace("\"matches\": 5565,", "")),
            "/app/v1/perfumes" if asked.contains("y_0=1804&") => json(r#"{"error":"Not available"}"#),
            "/app/v1/perfumes" if asked.contains("y_0=") => json(YEAR),
            _ => (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response(),
        };
    }
    // The web site: the two ranking pages the repository holds.
    match path {
        "/Perfumes/Tops/Men" => html(TOP_MEN),
        "/Perfumes/Trends" => html(TRENDS),
        _ => (StatusCode::NOT_FOUND, "not here").into_response(),
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
        max_retries: 0,
        ..ClientConfig::default()
    };
    let client = ParfumoClient::new(cfg, GlobalLimiter::new(0), None).unwrap();
    if logged_in {
        client.app_login("someone@example.org", "pw").await.unwrap();
    }
    site.asked.lock().unwrap().clear();
    (client, site)
}

/// The names of a page, in the order of the listing.
fn names(page: &Listing<PerfumeSummary>) -> Vec<&str> {
    page.items.iter().map(|p| p.name.as_str()).collect()
}

#[tokio::test]
async fn a_release_year_is_read_as_the_application_asks_it() {
    let (c, site) = client(true).await;
    let page = c.get_rankings(&RankingKind::Year(2020), 3).await.unwrap();
    assert_eq!(site.asked(), ["/app/v1/perfumes?y_0=2020&page=3&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1"], "one call, no web page");

    // 5565 perfumes that year; like the web site, the first 1000 are listed.
    assert_eq!((page.page, page.total_items, page.total_pages), (3, Some(5565), Some(50)));
    // The rows keep the order the site sent them in.
    assert_eq!(names(&page), ["#04 Figue Prune", "#05 Vanille Neroli", "'Cause I'm Happy", "007 pour Femme", "1 Million Parfum"]);

    let million = page.items.iter().find(|p| p.name == "1 Million Parfum").unwrap();
    assert_eq!((million.id.as_str(), million.p_id), ("Rabanne/1-million-parfum", Some(136850)));
    assert_eq!((million.brand.as_deref(), million.year, million.gender.as_deref()), (Some("Rabanne"), Some(2020), Some("men")));
    assert_eq!((million.rating, million.rating_count, million.reviews_count), (Some(7.5), Some(747), Some(35)));
    assert_eq!(million.url, format!("{}/Perfumes/Rabanne/1-million-parfum", c.base_url()));
    assert!(million.image_url.as_deref().unwrap().starts_with("https://media.parfumo.com/perfumes/"));

    let bond = page.items.iter().find(|p| p.name == "007 pour Femme").unwrap();
    assert_eq!((bond.id.as_str(), bond.gender.as_deref()), ("James_Bond_007/007-pour-femme", Some("women")));
    let unrated = page.items.iter().find(|p| p.name == "#04 Figue Prune").unwrap();
    assert_eq!((unrated.rating, unrated.rating_count, unrated.reviews_count), (None, None, None), "zeros of the site are absences");

    // What the interface does not give stays empty: a year listing has no position, no
    // trend counter and no concentration label.
    assert!(page.items.iter().all(|p| p.rank.is_none() && p.trend_count.is_none() && p.concentration.is_none()));
    assert!(page.items.iter().all(|p| p.year == Some(2020)));

    // The numbers met in the listing are remembered: the record is asked at once.
    let _ = c.get_perfume("rabanne/1-million-parfum").await;
    assert_eq!(site.asked().last().unwrap(), "/app/v1/perfume_details/136850");
}

#[tokio::test]
async fn the_end_of_a_year_is_told_from_its_total() {
    let (c, site) = client(true).await;
    let empty = c.get_rankings(&RankingKind::Year(1801), 1).await.unwrap();
    assert_eq!((empty.items.len(), empty.page, empty.total_items, empty.total_pages), (0, 1, Some(0), Some(0)));

    let small = c.get_rankings(&RankingKind::Year(1950), 0).await.unwrap();
    assert_eq!((small.items.len(), small.page, small.total_items, small.total_pages), (5, 1, Some(5), Some(1)), "page 0 is page 1");
    // A page that starts past the last perfume is empty, even if the site sends rows.
    let past = c.get_rankings(&RankingKind::Year(1950), 2).await.unwrap();
    assert_eq!((past.items.len(), past.page, past.total_items, past.total_pages), (0, 2, Some(5), Some(1)));
    assert_eq!(site.asked().len(), 3);
    assert!(site.asked().iter().all(|a| a.starts_with("/app/v1/perfumes?y_0=")), "{:?}", site.asked());

    // A year out of range is refused before anything is asked.
    assert_eq!(c.get_rankings(&RankingKind::Year(1500), 1).await.unwrap_err(), ClientError::InvalidInput("year out of range".into()));
    assert_eq!(site.asked().len(), 3);
}

#[tokio::test]
async fn the_other_rankings_still_read_the_web_pages() {
    let (c, site) = client(true).await;

    // The top list of the web site: 100 ranked perfumes a page, five pages.
    let top = c.get_rankings(&RankingKind::TopMen, 1).await.unwrap();
    assert_eq!((top.items.len(), top.total_pages, top.items[0].rank), (100, Some(5), Some(1)));
    assert_eq!(top.items[0].id, "Louis_Vuitton/imagination");

    // The three trend lists of the web site, with their counters.
    for (kind, section) in [(RankingKind::Trends, 0), (RankingKind::TrendsWished, 1), (RankingKind::TrendsWorn, 2)] {
        let trends = c.get_rankings(&kind, 1).await.unwrap();
        assert_eq!((trends.items.len(), trends.total_pages), (20, Some(1)), "section {section}");
        assert!(trends.items[0].rank == Some(1) && trends.items[0].trend_count.is_some(), "section {section}");
    }

    // The pages this fake does not hold are asked of the web site all the same.
    for kind in [RankingKind::TopWomen, RankingKind::TopUnisex, RankingKind::New] {
        assert!(c.get_rankings(&kind, 2).await.is_err());
    }
    assert_eq!(
        site.asked(),
        [
            "/Perfumes/Tops/Men",
            "/Perfumes/Trends",
            "/Perfumes/Trends",
            "/Perfumes/Trends",
            "/Perfumes/Tops/Women?current_page=2",
            "/Perfumes/Tops/Unisex?current_page=2",
            "/Recently_added?current_page=2"
        ],
        "nothing is asked of the application interface"
    );
}

#[tokio::test]
async fn a_year_goes_to_the_web_site_past_the_thousandth_perfume_or_without_a_session() {
    // The application never asks past the 1000th row: page 50 is the last one asked of the
    // interface, page 51 is left to the web site.
    let (c, site) = client(true).await;
    let last = c.get_rankings(&RankingKind::Year(2020), 50).await.unwrap();
    assert_eq!((last.items.len(), last.page, last.total_pages), (5, 50, Some(50)));
    let _ = c.get_rankings(&RankingKind::Year(2020), 51).await;
    assert_eq!(site.asked(), ["/app/v1/perfumes?y_0=2020&page=50&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1", "/Release_Years/2020?current_page=51"]);

    // Without a session of the application interface, nothing changes.
    let (c, site) = client(false).await;
    let _ = c.get_rankings(&RankingKind::Year(2020), 1).await;
    assert_eq!(site.asked(), ["/Release_Years/2020"]);
}

#[tokio::test]
async fn an_answer_that_departs_from_the_recorded_one_is_never_filled_in() {
    let (c, site) = client(true).await;

    // A total sent as text is read. A row without the address of its brand is left out:
    // its `Brand/Perfume` id cannot be built, and none is made up.
    let page = c.get_rankings(&RankingKind::Year(1802), 1).await.unwrap();
    assert_eq!((page.total_items, page.total_pages), (Some(5565), Some(50)));
    assert_eq!(names(&page), ["#04 Figue Prune", "#05 Vanille Neroli", "'Cause I'm Happy", "007 pour Femme"]);

    // Without a total the rows are kept and the totals stay unknown, on any page.
    let page = c.get_rankings(&RankingKind::Year(1803), 2).await.unwrap();
    assert_eq!((page.items.len(), page.page, page.total_items, page.total_pages), (5, 2, None, None));

    // An answer that is not a listing is an error: neither an empty year, nor a silent
    // switch to the web site.
    let err = c.get_rankings(&RankingKind::Year(1804), 1).await.unwrap_err();
    assert!(matches!(err, ClientError::ParserOutdated(_)), "{err:?}");
    assert_eq!(site.asked().len(), 3);
    assert!(site.asked().iter().all(|a| a.starts_with("/app/v1/perfumes?y_0=")), "{:?}", site.asked());
}

#[tokio::test]
async fn once_the_token_is_refused_the_web_site_is_read_again() {
    let (c, site) = client(true).await;
    site.refuses.store(true, Ordering::SeqCst);

    // The web page answers the refused read at once, and the token is forgotten: it is not
    // sent a second time.
    let _ = c.get_rankings(&RankingKind::Year(2020), 1).await;
    assert!(!c.has_app_session());
    assert_eq!(site.asked(), ["/app/v1/perfumes?y_0=2020&page=1&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1", "/Release_Years/2020"]);
    let _ = c.get_rankings(&RankingKind::Year(2020), 1).await;
    assert_eq!(site.asked().len(), 3);
    assert_eq!(site.asked()[2], "/Release_Years/2020");
}
