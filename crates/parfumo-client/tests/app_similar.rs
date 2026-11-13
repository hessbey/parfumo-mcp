//! Similar perfumes through the application interface, against the answer recorded on the
//! real site for `GET /similar?action=list&type=similar` (`tests/fixtures/app_similar_list.json`,
//! four of its rows). A local fake serves it and records what is asked.
//!
//! No answer was recorded for `type=layer`: the fake gives rows of the recorded list, in
//! the shape the application reads for both types. The dupes directory is the fragment of
//! the web site the HTML tests already use.

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
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient, SearchQuery, SimilarOptions};
use serde_json::Value;

const SIMILAR_LIST: &str = include_str!("fixtures/app_similar_list.json");
const PERFUME_DETAILS: &str = include_str!("fixtures/app_perfume_details.json");
const SEARCH_CREED: &str = include_str!("fixtures/app_search_creed.json");
const PERFUMES_BRAND: &str = include_str!("fixtures/app_perfumes_brand.json");
const DUPES: &str = include_str!("fixtures/frag_dupes.html");

const LIST: &str = "/app/v1/similar?action=list&type=similar&p_id=9438&limit=30";
const LAYER: &str = "/app/v1/similar?action=list&type=layer&p_id=9438&limit=30";

struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// What `type=layer` answers.
    layer: Mutex<String>,
    /// Whether every number has a record (the one of Aventus), or 9438 alone.
    every_record: AtomicBool,
    /// Whether a page of the web site was asked with the session token.
    token_on_web: AtomicBool,
}

/// The last two rows of the recorded list, as an answer of their own.
fn two_rows() -> String {
    let mut answer: Value = serde_json::from_str(SIMILAR_LIST).unwrap();
    let rows = answer["perfumes"].as_array().unwrap()[2..].to_vec();
    answer["total"] = rows.len().into();
    answer["perfumes"] = rows.into();
    answer.to_string()
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked.clone());
    let json = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    let not_found = || (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response();
    let token = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if !uri.path().starts_with("/app/v1/") {
        site.token_on_web.fetch_or(token.is_some(), Ordering::SeqCst);
        return match uri.path() {
            "/action/perfume/_get_dupes.php" => (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], DUPES).into_response(),
            _ => not_found(),
        };
    }
    if uri.path() == "/app/v1/auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#);
    }
    if token != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    match uri.path() {
        "/app/v1/similar" if asked.contains("type=similar") => json(SIMILAR_LIST),
        "/app/v1/similar" if asked.contains("type=layer") => json(&site.layer.lock().unwrap()),
        "/app/v1/search" => json(SEARCH_CREED),
        "/app/v1/perfumes" => json(PERFUMES_BRAND),
        "/app/v1/perfume_details/9438" => json(PERFUME_DETAILS),
        path if path.starts_with("/app/v1/perfume_details/") && site.every_record.load(Ordering::SeqCst) => json(PERFUME_DETAILS),
        _ => not_found(),
    }
}

/// A client logged in to the application interface of a fake site.
async fn logged_in() -> (ParfumoClient, Arc<Site>) {
    let site = Arc::new(Site {
        asked: Mutex::default(),
        layer: Mutex::new(two_rows()),
        every_record: AtomicBool::new(false),
        token_on_web: AtomicBool::new(false),
    });
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

fn options(resolve_urls: usize, include_layering: bool, include_dupes: bool) -> SimilarOptions {
    SimilarOptions { resolve_urls, include_layering, include_dupes }
}

#[tokio::test]
async fn the_two_lists_come_from_the_interface_and_the_dupes_from_the_web_page() {
    let (c, site) = logged_in().await;
    let found = c.get_similar_perfumes_with("9438", options(0, true, true)).await.unwrap();
    // The record names the perfume for the dupes directory; the page of the perfume is not read.
    assert_eq!(site.asked.lock().unwrap().as_slice(), ["/app/v1/perfume_details/9438", LIST, LAYER, "/action/perfume/_get_dupes.php?q=Aventus"]);
    assert!(!site.token_on_web.load(Ordering::SeqCst), "the session token is for the application interface only");
    assert_eq!(found.perfume_id, "Creed/aventus");
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);

    // Strongest consensus first, as the site sends them.
    assert_eq!(found.similar.iter().map(|p| p.p_id).collect::<Vec<_>>(), [55402, 65533, 127275, 225233]);
    let first = &found.similar[0];
    assert_eq!((first.name.as_str(), first.brand.as_deref()), ("Club de Nuit Intense Man", Some("Armaf")));
    assert_eq!(first.concentration.as_deref(), Some("Eau de Toilette"), "the name does not repeat it");
    assert_eq!((first.percent, first.votes), (Some(81.0), Some(819)), "the votes come with every row");
    assert!(first.image_url.as_deref().unwrap().starts_with("https://media.parfumo.com/perfumes/"));
    assert_eq!((&first.id, &first.url), (&None, &None), "the list has no address, and none was asked for");
    let second = &found.similar[1];
    assert_eq!((second.name.as_str(), second.brand.as_deref(), second.concentration.as_deref()), ("L'Aventure", Some("Al Haramain"), None));
    assert_eq!((second.percent, second.votes), (Some(78.0), Some(460)));

    // The other type of the same call, read the same way.
    assert_eq!(
        found.layering.iter().map(|p| (p.p_id, p.name.as_str())).collect::<Vec<_>>(),
        [(127275, "Aventus Cologne"), (225233, "Absolu Aventus")]
    );

    // Of the 21 groups of the fragment, the two the perfume is in.
    assert_eq!(found.dupes.iter().map(|g| g.original.id.as_str()).collect::<Vec<_>>(), ["Dior/Sauvage_Eau_de_Toilette", "Dior/sauvage-parfum"]);
    assert!(found.dupes.iter().all(|g| g.dupes.iter().any(|d| d.perfume.id == "Creed/aventus")));
}

#[tokio::test]
async fn what_was_met_in_a_listing_costs_no_call() {
    let (c, site) = logged_in().await;
    c.search_perfumes(&SearchQuery { brand: Some("creed".into()), ..SearchQuery::default() }, 1).await.unwrap();
    site.asked.lock().unwrap().clear();

    // The reference is known by its id, and so is one row of the list.
    let found = c.get_similar_perfumes_with("Creed/absolu-aventus", options(0, false, false)).await.unwrap();
    assert_eq!(site.asked.lock().unwrap().as_slice(), ["/app/v1/similar?action=list&type=similar&p_id=225233&limit=30"]);
    assert_eq!(found.perfume_id, "Creed/absolu-aventus");
    assert!(found.layering.is_empty() && found.dupes.is_empty() && found.warnings.is_empty());
    let known = found.similar.iter().find(|p| p.p_id == 225233).unwrap();
    assert_eq!(known.id.as_deref(), Some("Creed/absolu-aventus"));
    assert_eq!(known.url, Some(format!("{}/Perfumes/Creed/absolu-aventus", c.base_url())));
    assert!(found.similar.iter().filter(|p| p.p_id != 225233).all(|p| p.id.is_none() && p.url.is_none()));
}

#[tokio::test]
async fn the_first_rows_get_their_id_from_their_record() {
    let (c, site) = logged_in().await;
    // The fake has one record, the one of Aventus, for every number: what is checked is
    // which records are asked, and that their id becomes the id and the address of the row.
    site.every_record.store(true, Ordering::SeqCst);
    let found = c.get_similar_perfumes_with("9438", options(2, false, false)).await.unwrap();
    assert_eq!(
        site.asked.lock().unwrap().as_slice(),
        ["/app/v1/perfume_details/9438", LIST, "/app/v1/perfume_details/55402", "/app/v1/perfume_details/65533"]
    );
    for row in &found.similar[..2] {
        assert_eq!(row.id.as_deref(), Some("Creed/aventus"));
        assert_eq!(row.url, Some(format!("{}/Perfumes/Creed/aventus", c.base_url())));
    }
    assert!(found.similar[2..].iter().all(|p| p.id.is_none()), "the others are left without an address");
    assert!(found.warnings.is_empty());
}

#[tokio::test]
async fn a_record_that_cannot_be_read_stops_the_links_and_is_told() {
    let (c, site) = logged_in().await;
    let found = c.get_similar_perfumes_with("9438", options(5, false, false)).await.unwrap();
    assert_eq!(
        site.asked.lock().unwrap().as_slice(),
        ["/app/v1/perfume_details/9438", LIST, "/app/v1/perfume_details/55402"],
        "one refusal is enough"
    );
    assert_eq!(found.similar.len(), 4, "the list itself is kept");
    assert!(found.similar.iter().all(|p| p.id.is_none()));
    assert_eq!(found.warnings, ["links of similar perfumes not resolved: not found: /perfume_details/55402"]);
}

#[tokio::test]
async fn layering_is_left_empty_with_a_warning_when_its_answer_cannot_be_trusted() {
    let (c, site) = logged_in().await;
    // The type ignored: the list of similar perfumes again.
    *site.layer.lock().unwrap() = SIMILAR_LIST.to_string();
    let found = c.get_similar_perfumes_with("9438", options(0, true, false)).await.unwrap();
    assert_eq!(found.similar.len(), 4);
    assert!(found.layering.is_empty());
    assert_eq!(found.warnings, ["layering suggestions unavailable: the application interface answered the list of similar perfumes"]);

    // An answer that is not a list.
    *site.layer.lock().unwrap() = r#"{"status":"error","message":"Invalid type"}"#.to_string();
    let found = c.get_similar_perfumes_with("9438", options(0, true, false)).await.unwrap();
    assert!(found.layering.is_empty());
    assert_eq!(found.warnings, ["layering suggestions unavailable: parser outdated: /app/v1/similar: no list for type=layer"]);

    // No suggestion at all is not a problem.
    *site.layer.lock().unwrap() = r#"{"status":"success","perfumes":[],"total":0,"limit":30}"#.to_string();
    let found = c.get_similar_perfumes_with("9438", options(0, true, false)).await.unwrap();
    assert!(found.layering.is_empty() && found.warnings.is_empty());
}

#[tokio::test]
async fn a_reference_the_interface_does_not_find_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    let err = c.get_similar_perfumes("Creed/no-such-perfume").await.unwrap_err();
    let asked = site.asked.lock().unwrap().clone();
    // The brand, its perfumes, then the page of the web site, which has the last word.
    assert_eq!(asked.len(), 3, "{asked:?}");
    assert_eq!(asked[2], "/Perfumes/Creed/no-such-perfume");
    assert!(asked.iter().all(|a| !a.starts_with("/app/v1/similar")), "{asked:?}");
    assert_eq!(err, ClientError::NotFound("/Perfumes/Creed/no-such-perfume".into()));
}
