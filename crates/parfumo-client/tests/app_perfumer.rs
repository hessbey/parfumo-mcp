//! The page of a perfumer through the application interface, against an answer recorded on
//! the real site (`tests/fixtures/app_perfumer_details.json`). A local fake serves it, and
//! the web page of a perfumer, and records what is asked.
//!
//! The answer was recorded without an order (`o`), so its rows come by name. The client asks
//! with the order the application sends: the tests check the call and the mapping of the
//! rows, never their order.

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

const PERFUMER_DETAILS: &str = include_str!("fixtures/app_perfumer_details.json");
/// The record of a perfume this perfumer made: it names them by number, name and slug.
const PERFUME_DETAILS: &str = include_str!("fixtures/app_perfume_details.json");
/// The web page of another perfumer (France), served for every perfumer.
const PERFUMER_PAGE: &str = include_str!("fixtures/perfumer.html");
/// The page of the web site the country of the recorded perfumer is read on.
const WEB_PAGE: &str = "/Perfumers/JeanChristophe_Herault";

/// The only call the tests expect for the recorded perfumer, page aside: the one the
/// application makes when it opens the page of a perfumer.
fn details(page: u32) -> String {
    format!("/app/v1/perfumer_details?pf_id=509&page={page}&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1")
}

/// Not recorded, built for the tests from the recorded answer: a listing longer than the
/// thousand rows the application reads.
fn long_listing() -> String {
    let mut answer: serde_json::Value = serde_json::from_str(PERFUMER_DETAILS).unwrap();
    answer["p"]["matches"] = serde_json::json!(43518);
    answer.to_string()
}

/// Not recorded, built for the tests from the recorded record of a perfume: one made by two
/// perfumers who bear the same name. Their numbers, slugs and name are made up. A record is
/// the one of the number it is asked under: this one is served under 2.
fn namesakes() -> String {
    let mut record: serde_json::Value = serde_json::from_str(PERFUME_DETAILS).unwrap();
    record["p"]["id"] = "2".into();
    record["p"]["perfumers_array"] = serde_json::json!([
        {"id": "7001", "name": "Tést Namesake", "name_url": "Tst_Namesake"},
        {"id": "7002", "name": "Tést Namesake", "name_url": "Tst_Namesake_2"},
    ]);
    record.to_string()
}

#[derive(Default)]
struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked.clone());
    let json = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    if uri.path().starts_with("/Perfumers/") {
        return (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], PERFUMER_PAGE).into_response();
    }
    if uri.path() == "/app/v1/auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#);
    }
    if headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    match uri.path() {
        "/app/v1/perfumer_details" if asked.contains("pf_id=509&") => json(PERFUMER_DETAILS),
        "/app/v1/perfumer_details" if asked.contains("pf_id=510&") => json(&long_listing()),
        // Not recorded: the recorded answer without the count of its listing; a refusal in
        // JSON under status 200, as the interface gives for a brand it does not have; a 404.
        "/app/v1/perfumer_details" if asked.contains("pf_id=511&") => json(&PERFUMER_DETAILS.replace("\"matches\": 135,", "")),
        "/app/v1/perfumer_details" if asked.contains("pf_id=512&") => json(r#"{"error":"Perfumer not found"}"#),
        "/app/v1/perfumer_details" if asked.contains("pf_id=513&") => (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response(),
        // Not recorded: what the site answers for a number no perfumer has. Assumed here: no perfumer, no row.
        "/app/v1/perfumer_details" => json(r#"{"pf":null,"p":{"matches":0,"rows":[]}}"#),
        "/app/v1/perfume_details/2" => json(&namesakes()),
        path if path.starts_with("/app/v1/perfume_details/") => json(PERFUME_DETAILS),
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

#[tokio::test]
async fn a_perfumer_is_read_by_number_with_one_page_of_their_creations() {
    let (c, site) = client(true).await;
    let p = c.get_perfumer("509", 1).await.unwrap();
    assert_eq!(
        site.asked.lock().unwrap().as_slice(),
        [details(1), WEB_PAGE.to_string()],
        "one call, the one the application makes, and the web page for the country"
    );

    assert_eq!(
        (p.perfumer.id.as_str(), p.perfumer.numeric_id, p.perfumer.name.as_str()),
        ("JeanChristophe_Herault", Some(509), "Jean-Christophe Hérault")
    );
    assert_eq!(p.perfumer.url, format!("{}/Perfumers/JeanChristophe_Herault", c.base_url()));
    assert_eq!(p.country.as_deref(), Some("France"), "the interface gives the number of a country: its name is read on the web page");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);

    // 135 creations, 20 a page.
    assert_eq!((p.perfumes.page, p.perfumes.total_items, p.perfumes.total_pages), (1, Some(135), Some(7)));
    assert_eq!(p.perfumes.items.len(), 5);
    let aventus = p.perfumes.items.iter().find(|i| i.name == "Aventus").unwrap();
    assert_eq!((aventus.id.as_str(), aventus.p_id), ("Creed/aventus", Some(9438)));
    assert_eq!((aventus.brand.as_deref(), aventus.year, aventus.gender.as_deref()), (Some("Creed"), Some(2010), Some("men")));
    assert_eq!((aventus.rating, aventus.rating_count, aventus.reviews_count), (Some(8.4), Some(8836), Some(632)));
    assert_eq!(aventus.url, format!("{}/Perfumes/Creed/aventus", c.base_url()));
    assert!(aventus.image_url.as_deref().unwrap().starts_with("https://media.parfumo.com/perfumes/"));
    assert_eq!((aventus.concentration.as_deref(), aventus.rank, aventus.trend_count), (None, None, None), "not in a row of the index");
    // Zeros of the site are absences; German genders are translated.
    let pea = &p.perfumes.items[0];
    assert_eq!((pea.name.as_str(), pea.id.as_str(), pea.year, pea.reviews_count), ("... & the Pea", "bel-rebel/the-pea", None, None));
    assert_eq!((pea.rating, pea.rating_count, pea.gender.as_deref()), (Some(9.5), Some(4), Some("unisex")));
    let genders: Vec<_> = p.perfumes.items.iter().filter_map(|i| i.gender.as_deref()).collect();
    assert_eq!(genders, ["unisex", "men", "men", "men", "women"]);

    // The page teaches the number of its perfumer: their slug is enough for the next pages.
    let next = c.get_perfumer("JeanChristophe_Herault", 2).await.unwrap();
    assert_eq!(site.asked.lock().unwrap().as_slice(), [details(1), WEB_PAGE.to_string(), details(2), WEB_PAGE.to_string()]);
    assert_eq!(next.perfumes.page, 2);
    // A page before the first is the first.
    c.get_perfumer("509", 0).await.unwrap();
    assert_eq!(site.asked.lock().unwrap()[4..], [details(1), WEB_PAGE.to_string()]);
}

#[tokio::test]
async fn a_perfumer_met_in_a_perfume_record_is_found_by_address_slug_or_name() {
    let (c, site) = client(true).await;
    let perfume = c.get_perfume("9438").await.unwrap();
    assert_eq!(perfume.perfumers[0].numeric_id, Some(509));
    site.asked.lock().unwrap().clear();

    // The references a perfume record gives, and the ones the web code understands.
    let address = perfume.perfumers[0].url.clone();
    for reference in [
        address.as_str(),
        "/Perfumers/JeanChristophe_Herault",
        "Perfumers/JeanChristophe_Herault/",
        "jeanchristophe_herault",
        "JeanChristophe Herault",
        "Jean-Christophe Hérault",
        " jean-christophe_hérault ",
    ] {
        let p = c.get_perfumer(reference, 3).await.unwrap();
        assert_eq!(p.perfumer.numeric_id, Some(509), "{reference}");
    }
    let asked = site.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 14, "{asked:?}");
    assert!(
        asked.chunks(2).all(|read| read == [details(3), WEB_PAGE.to_string()]),
        "nothing is asked to find the perfumer; the web page is read for the country only: {asked:?}"
    );
}

#[tokio::test]
async fn a_perfumer_never_met_is_read_from_the_web_page() {
    let (c, site) = client(true).await;
    // No call of the interface finds a perfumer from a name: the web code does.
    let p = c.get_perfumer("Francois Demachy", 2).await.unwrap();
    assert_eq!(site.asked.lock().unwrap().as_slice(), ["/Perfumers/Francois_Demachy?current_page=2"], "nothing is asked of the interface");
    assert_eq!((p.perfumer.id.as_str(), p.perfumer.numeric_id, p.country.as_deref()), ("Franois_Demachy", Some(131), Some("France")));

    // An address never met, and the address of something else: the web code reads or refuses them.
    let by_address = c.get_perfumer(&format!("{}/Perfumers/Franois_Demachy", c.base_url()), 1).await.unwrap();
    assert_eq!(by_address.perfumer.numeric_id, Some(131));
    assert_eq!(site.asked.lock().unwrap().last().unwrap(), "/Perfumers/Franois_Demachy");
    let before = site.asked.lock().unwrap().len();
    assert_eq!(c.get_perfumer("/Perfumes/Creed", 1).await.unwrap_err(), ClientError::InvalidInput("not a Parfumo perfumer URL".into()));
    assert!(matches!(c.get_perfumer("  ", 1).await.unwrap_err(), ClientError::InvalidInput(_)));
    assert_eq!(site.asked.lock().unwrap().len(), before, "a refused reference asks nothing");

    // A number inside an address is a slug, not the number of a perfumer.
    c.get_perfumer("/Perfumers/509", 1).await.unwrap();
    assert_eq!(site.asked.lock().unwrap().last().unwrap(), "/Perfumers/509");
    // Nor is zero, or a number too long to be one.
    for not_a_number in ["0", "99999999999999999999999"] {
        c.get_perfumer(not_a_number, 1).await.unwrap();
        assert_eq!(site.asked.lock().unwrap().last().unwrap(), &format!("/Perfumers/{not_a_number}"));
    }
}

#[tokio::test]
async fn a_long_listing_stops_where_the_application_stops() {
    let (c, site) = client(true).await;
    let p = c.get_perfumer("510", 50).await.unwrap();
    assert_eq!(
        site.asked.lock().unwrap().as_slice(),
        ["/app/v1/perfumer_details?pf_id=510&page=50&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1", WEB_PAGE]
    );
    // Every creation is counted, but the pages end at the thousandth row, as on the web site.
    assert_eq!((p.perfumes.page, p.perfumes.total_items, p.perfumes.total_pages), (50, Some(43518), Some(50)));
}

#[tokio::test]
async fn without_a_session_of_the_interface_the_web_page_is_read() {
    let (c, site) = client(false).await;
    let p = c.get_perfumer("509", 1).await.unwrap();
    assert_eq!(site.asked.lock().unwrap().as_slice(), ["/Perfumers/509"], "no call leaves without a token");
    assert_eq!(p.perfumer.numeric_id, Some(131));
}

#[tokio::test]
async fn a_perfumer_the_interface_has_nothing_about_or_does_not_count_is_left_to_the_web_site() {
    let (c, site) = client(true).await;
    // An answer without a perfumer, one without the count of its listing, a refusal in
    // JSON and a 404: the web page answers each time, as it does without a session.
    for number in [999_999, 511, 512, 513] {
        let p = c.get_perfumer(&number.to_string(), 2).await.unwrap();
        let asked = std::mem::take(&mut *site.asked.lock().unwrap());
        assert_eq!(
            asked,
            [
                format!("/app/v1/perfumer_details?pf_id={number}&page=2&per_page=20&o=nr_desc&g_f=1&g_m=1&g_u=1"),
                format!("/Perfumers/{number}?current_page=2")
            ],
            "{number}"
        );
        assert_eq!((p.perfumer.numeric_id, p.country.as_deref()), (Some(131), Some("France")), "the page of the web site");
    }
    assert!(c.has_app_session(), "none of this is a refusal of the session");
}

#[tokio::test]
async fn an_address_is_only_found_by_its_slug_and_a_name_borne_by_two_is_left_to_the_web_site() {
    let (c, site) = client(true).await;
    c.get_perfume("9438").await.unwrap();
    c.get_perfume("2").await.unwrap();
    site.asked.lock().unwrap().clear();
    let last = |site: &Site| site.asked.lock().unwrap().last().cloned().unwrap();

    // An address names a slug. One never met is another page of the site, even when it reads
    // like the name of a perfumer met: the web page is read, not the page of that perfumer.
    c.get_perfumer("/Perfumers/Jean-Christophe_Hérault", 1).await.unwrap();
    assert_eq!(last(&site), "/Perfumers/Jean-Christophe_H%C3%A9rault");
    c.get_perfumer(&format!("{}/Perfumers/Tst_Namesake_3", c.base_url()), 1).await.unwrap();
    assert_eq!(last(&site), "/Perfumers/Tst_Namesake_3");

    // Each slug has its own number. (The fake has no perfumer under these numbers: the web
    // page is asked after the interface.)
    for (reference, call) in
        [("Tst_Namesake", "/app/v1/perfumer_details?pf_id=7001&"), ("/Perfumers/Tst_Namesake_2", "/app/v1/perfumer_details?pf_id=7002&")]
    {
        let before = site.asked.lock().unwrap().len();
        let _ = c.get_perfumer(reference, 1).await;
        let asked = site.asked.lock().unwrap()[before..].to_vec();
        assert!(asked[0].starts_with(call), "{reference}: {asked:?}");
    }

    // The name tells neither from the other: the web site chooses, as it does without a session.
    c.get_perfumer("Tést Namesake", 1).await.unwrap();
    assert_eq!(last(&site), "/Perfumers/T%C3%A9st_Namesake");
}
