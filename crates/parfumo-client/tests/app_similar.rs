//! Similar perfumes through the application interface, against the answer recorded on the
//! real site for `GET /similar?action=list&type=similar` (`tests/fixtures/app_similar_list.json`,
//! four of its rows). A local fake serves it and records what is asked.
//!
//! No answer was recorded for `type=layer`: the fake gives rows of the recorded list, in
//! the shape the application reads for both types. No record of a row of the list was
//! recorded either: the fake names each one after its number. The pages of the web site
//! are the ones the HTML tests already use.

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
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient, SearchQuery, SimilarOptions, SimilarPerfumes};
use serde_json::Value;

const SIMILAR_LIST: &str = include_str!("fixtures/app_similar_list.json");
const PERFUME_DETAILS: &str = include_str!("fixtures/app_perfume_details.json");
const SEARCH_CREED: &str = include_str!("fixtures/app_search_creed.json");
const PERFUMES_BRAND: &str = include_str!("fixtures/app_perfumes_brand.json");
const DUPES: &str = include_str!("fixtures/frag_dupes.html");
const WEB_PERFUME: &str = include_str!("fixtures/perfume_sauvage.html");
const WEB_SIMILAR: &str = include_str!("fixtures/frag_similar_all.html");

const LIST: &str = "/app/v1/similar?action=list&type=similar&p_id=9438&limit=30";
const LAYER: &str = "/app/v1/similar?action=list&type=layer&p_id=9438&limit=30";
/// What the HTML code asks for the same lists of Sauvage, without links nor layering.
const WEB: [&str; 3] = ["/Perfumes/Dior/Sauvage_Eau_de_Toilette", "/action/perfume/get_similar.php", "/action/perfume/_get_dupes.php?q=Sauvage"];

struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// What `type=similar` answers.
    similar: Mutex<String>,
    /// What `type=layer` answers.
    layer: Mutex<String>,
    /// Whether every number has a record, or 9438 alone.
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

/// The recorded record under another number, with an id of its own: `brand-N/perfume-N`.
fn record_of(number: &str) -> String {
    let mut record: Value = serde_json::from_str(PERFUME_DETAILS).unwrap();
    record["p"]["id"] = number.into();
    record["p"]["name_url"] = format!("perfume-{number}").into();
    record["b"]["name_url"] = format!("brand-{number}").into();
    record.to_string()
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked.clone());
    let json = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    let not_found = || (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response();
    let token = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if !uri.path().starts_with("/app/v1/") {
        site.token_on_web.fetch_or(token.is_some(), Ordering::SeqCst);
        let html = |text: &'static str| (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], text).into_response();
        return match uri.path() {
            "/Perfumes/Dior/Sauvage_Eau_de_Toilette" => html(WEB_PERFUME),
            "/action/perfume/get_similar.php" => html(WEB_SIMILAR),
            "/action/perfume/_get_dupes.php" => html(DUPES),
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
        "/app/v1/similar" if asked.contains("type=similar") => json(&site.similar.lock().unwrap()),
        "/app/v1/similar" if asked.contains("type=layer") => json(&site.layer.lock().unwrap()),
        "/app/v1/search" => json(SEARCH_CREED),
        "/app/v1/perfumes" => json(PERFUMES_BRAND),
        "/app/v1/perfume_details/9438" => json(PERFUME_DETAILS),
        path if path.starts_with("/app/v1/perfume_details/") && site.every_record.load(Ordering::SeqCst) => {
            json(&record_of(path.trim_start_matches("/app/v1/perfume_details/")))
        }
        _ => not_found(),
    }
}

/// A client of a fake site, without a session of the application interface.
async fn anonymous() -> (ParfumoClient, Arc<Site>) {
    let site = Arc::new(Site {
        asked: Mutex::default(),
        similar: Mutex::new(SIMILAR_LIST.to_string()),
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
    (ParfumoClient::new(cfg, GlobalLimiter::new(0), None).unwrap(), site)
}

/// A client logged in to the application interface of a fake site.
async fn logged_in() -> (ParfumoClient, Arc<Site>) {
    let (client, site) = anonymous().await;
    client.app_login("someone@example.org", "pw").await.unwrap();
    site.asked.lock().unwrap().clear();
    (client, site)
}

/// The lists of Sauvage as the HTML code reads them from the pages of the web site.
fn assert_web_answer(found: &SimilarPerfumes) {
    assert_eq!(found.perfume_id, "Dior/Sauvage_Eau_de_Toilette");
    assert_eq!(found.similar.len(), 30);
    let first = &found.similar[0];
    assert_eq!((first.p_id, first.name.as_str(), first.brand.as_deref()), (102281, "Sauvage", Some("Dior")));
    assert_eq!((first.percent, first.votes, &first.id), (Some(91.4), None, &None), "the web list has one decimal, and no votes without a link");
    assert!(found.layering.is_empty() && found.warnings.is_empty(), "{:?}", found.warnings);
    assert!(found.dupes.iter().any(|g| g.original.id == "Dior/Sauvage_Eau_de_Toilette"));
    assert!(found.dupes.iter().all(|g| g.original.id == found.perfume_id || g.dupes.iter().any(|d| d.perfume.id == found.perfume_id)));
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

    // Asked for the links of the four rows, the record of the known one is not read.
    site.every_record.store(true, Ordering::SeqCst);
    site.asked.lock().unwrap().clear();
    let found = c.get_similar_perfumes_with("Creed/absolu-aventus", options(4, false, false)).await.unwrap();
    assert_eq!(
        site.asked.lock().unwrap().as_slice(),
        [
            "/app/v1/similar?action=list&type=similar&p_id=225233&limit=30",
            "/app/v1/perfume_details/55402",
            "/app/v1/perfume_details/65533",
            "/app/v1/perfume_details/127275"
        ]
    );
    assert_eq!(
        found.similar.iter().map(|p| p.id.as_deref().unwrap()).collect::<Vec<_>>(),
        ["brand-55402/perfume-55402", "brand-65533/perfume-65533", "brand-127275/perfume-127275", "Creed/absolu-aventus"]
    );
    assert!(found.warnings.is_empty());
}

#[tokio::test]
async fn the_first_rows_get_their_id_from_their_record() {
    let (c, site) = logged_in().await;
    // Each row gets the id of its own record, not the one of the perfume asked about.
    site.every_record.store(true, Ordering::SeqCst);
    let found = c.get_similar_perfumes_with("9438", options(2, false, false)).await.unwrap();
    assert_eq!(
        site.asked.lock().unwrap().as_slice(),
        ["/app/v1/perfume_details/9438", LIST, "/app/v1/perfume_details/55402", "/app/v1/perfume_details/65533"]
    );
    assert_eq!(found.perfume_id, "Creed/aventus");
    for row in &found.similar[..2] {
        let id = format!("brand-{0}/perfume-{0}", row.p_id);
        assert_eq!(row.url, Some(format!("{}/Perfumes/{id}", c.base_url())));
        assert_eq!(row.id, Some(id));
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

    // The same perfumes with votes of their own are another list: the type was honoured.
    let mut voted: Value = serde_json::from_str(SIMILAR_LIST).unwrap();
    voted["perfumes"][0]["cnt"] = 12.into();
    *site.layer.lock().unwrap() = voted.to_string();
    let found = c.get_similar_perfumes_with("9438", options(0, true, false)).await.unwrap();
    assert_eq!(
        found.layering.iter().map(|p| (p.p_id, p.votes)).collect::<Vec<_>>(),
        [(55402, Some(12)), (65533, Some(460)), (127275, Some(88)), (225233, Some(58))]
    );
    assert!(found.warnings.is_empty());

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
    // The brand, its perfumes, then the page of the web site, which has the last word.
    assert_eq!(
        site.asked.lock().unwrap().as_slice(),
        [
            "/app/v1/search?filter=Creed",
            "/app/v1/perfumes?page=1&per_page=20&filter=no+such+perfume&b_id=253&g_f=1&g_m=1&g_u=1",
            "/Perfumes/Creed/no-such-perfume"
        ]
    );
    assert_eq!(err, ClientError::NotFound("/Perfumes/Creed/no-such-perfume".into()));

    // A perfume the web site knows: the HTML code answers, as it does without a session.
    site.asked.lock().unwrap().clear();
    let found = c.get_similar_perfumes_with("Dior/Sauvage_Eau_de_Toilette", options(0, false, true)).await.unwrap();
    assert_eq!(site.asked.lock().unwrap().as_slice(), [&["/app/v1/search?filter=Dior"][..], &WEB[..]].concat());
    assert!(!site.token_on_web.load(Ordering::SeqCst), "the session token is for the application interface only");
    assert_web_answer(&found);
}

#[tokio::test]
async fn without_a_session_nothing_is_asked_of_the_interface() {
    let (c, site) = anonymous().await;
    assert!(!c.has_app_session());
    let found = c.get_similar_perfumes_with("Dior/Sauvage_Eau_de_Toilette", options(0, false, true)).await.unwrap();
    assert_eq!(site.asked.lock().unwrap().as_slice(), WEB);
    assert_web_answer(&found);
}

#[tokio::test]
async fn an_answer_that_is_not_the_list_of_similar_perfumes_is_an_error() {
    let (c, site) = logged_in().await;
    // A refusal in JSON, answered with HTTP 200: nothing is made of it, and nothing more
    // is asked.
    *site.similar.lock().unwrap() = r#"{"status":"error","message":"Invalid type"}"#.to_string();
    let err = c.get_similar_perfumes("9438").await.unwrap_err();
    assert_eq!(err, ClientError::ParserOutdated("/app/v1/similar: no list for type=similar".into()));
    assert_eq!(site.asked.lock().unwrap().as_slice(), ["/app/v1/perfume_details/9438", LIST]);

    // A list without the mark of success is not read either, as in the application.
    *site.similar.lock().unwrap() = SIMILAR_LIST.replacen(r#""status": "success""#, r#""status": "error""#, 1);
    assert!(matches!(c.get_similar_perfumes("9438").await, Err(ClientError::ParserOutdated(_))));
}
