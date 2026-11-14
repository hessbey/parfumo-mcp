//! The page of a fragrance note through the application interface, against answers
//! recorded on the real site (`tests/fixtures/app_note_*.json`). A local fake serves them,
//! and the web pages the client falls back to, and records what is asked.

use std::{
    collections::HashMap,
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
use serde_json::{Value, json};

const NOTE: &str = include_str!("fixtures/app_note_note.json");
const CATEGORY: &str = include_str!("fixtures/app_note_category.json");
const SEARCH: &str = include_str!("fixtures/app_note_search.json");
const OVERVIEW: &str = include_str!("fixtures/app_note_overview.json");
const PERFUME_DETAILS: &str = include_str!("fixtures/app_perfume_details.json");
const WEB_NOTE: &str = include_str!("fixtures/note.html");
const WEB_CATEGORY: &str = include_str!("fixtures/note_category.html");

// What the site answers in the cases below was not recorded: the fake stands for what the
// client must withstand, not for what the site is known to do.
/// A number no note has: answered 404.
const UNKNOWN: u64 = 999_999;
/// A number answered with the head of another note.
const OTHER_HEAD: u64 = 424_242;
/// A page answered with the first one.
const CLAMPED_PAGE: u64 = 77;
/// A page past the last rows: answered without rows.
const EMPTY_PAGE: u64 = 99;

#[derive(Default)]
struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// Whether the token is refused from now on.
    refusing: AtomicBool,
}

/// A recorded listing as the site gives it for the number and the page asked: the head
/// bears the number, the answer names its page.
fn listing(recorded: &str, head: &str, nr: u64, page: u64) -> Value {
    let mut answer: Value = serde_json::from_str(recorded).unwrap();
    answer[head]["id"] = nr.into();
    answer["page"] = page.into();
    answer
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked);
    let json = |text: String| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text).into_response();
    let html = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], text.to_string()).into_response();
    let not_found = || (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response();
    // The web site: one note and one category, whatever the slug.
    if uri.path().starts_with("/Fragrance_Notes/") {
        return html(WEB_CATEGORY);
    }
    if uri.path().starts_with("/Fragrance_Note/") {
        return html(WEB_NOTE);
    }
    if uri.path() == "/app/v1/auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#.into());
    }
    if site.refusing.load(Ordering::SeqCst) || headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    let query: HashMap<String, String> = url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).into_owned().collect();
    let param = |key: &str| query.get(key).map(String::as_str).unwrap_or_default();
    let nr = |key: &str| param(key).parse::<u64>().unwrap_or_default();
    match (uri.path(), param("feature")) {
        ("/app/v1/fragrance_notes", "search") => json(SEARCH.into()),
        ("/app/v1/fragrance_notes", "overview") => json(OVERVIEW.into()),
        ("/app/v1/fragrance_notes", "category") => json(listing(CATEGORY, "category", nr("cat_id"), nr("page")).to_string()),
        ("/app/v1/fragrance_notes", "note") => match (nr("note_id"), nr("page")) {
            (UNKNOWN, _) => not_found(),
            (OTHER_HEAD, _) => json(NOTE.into()),
            (note, CLAMPED_PAGE) => json(listing(NOTE, "note", note, 1).to_string()),
            (note, EMPTY_PAGE) => {
                let mut answer = listing(NOTE, "note", note, EMPTY_PAGE);
                answer["perfumes"] = json!([]);
                json(answer.to_string())
            }
            (note, page) => json(listing(NOTE, "note", note, page).to_string()),
        },
        // One record, whatever the number.
        (path, _) if path.starts_with("/app/v1/perfume_details/") => json(PERFUME_DETAILS.into()),
        _ => not_found(),
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

/// What was asked since the last look.
fn asked(site: &Site) -> Vec<String> {
    std::mem::take(&mut *site.asked.lock().unwrap())
}

#[tokio::test]
async fn a_note_named_by_its_number_is_read_in_one_call() {
    let (c, site) = logged_in().await;
    let n = c.get_note("311", 1).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=note&note_id=311&page=1&per_page=20"], "one call, no web page");

    assert_eq!((n.note.numeric_id, n.note.name.as_str(), n.is_category), (Some(311), "Bergamot", false));
    assert_eq!((n.note.id.as_str(), n.note.url.as_str()), ("", ""), "the interface gives no address of a note: none is made up");
    // Categories are numbered apart from notes (37 here, where the note is 311): their number is not handed on as one of a note.
    let categories: Vec<_> = n.categories.iter().map(|c| (c.numeric_id, c.name.as_str(), c.id.as_str(), c.url.as_str())).collect();
    assert_eq!(categories, [(None, "Bergamot", "", ""), (None, "Citric notes", "", "")]);

    assert_eq!((n.perfumes.page, n.perfumes.total_items, n.perfumes.total_pages), (1, Some(43561), Some(2179)));
    assert_eq!(n.perfumes.items.len(), 5);
    let first = &n.perfumes.items[0];
    assert_eq!((first.p_id, first.name.as_str(), first.brand.as_deref()), (Some(123923), "Afternoon Swim", Some("Louis Vuitton")));
    assert_eq!((first.year, first.gender.as_deref(), first.concentration.as_deref()), (Some(2019), Some("unisex"), None));
    assert_eq!((first.rating, first.rating_count, first.reviews_count), (Some(8.7), Some(4037), None), "out of ten; these rows count no reviews");
    assert_eq!(
        first.image_url.as_deref(),
        Some("https://media.parfumo.com/perfumes/49/492eeb-afternoon-swim-louis-vuitton_1200.jpg?width=240&aspect_ratio=1:1"),
        "the image the web page shows"
    );
    assert_eq!((first.trend_count, first.rank), (None, None));
    let second = &n.perfumes.items[1];
    assert_eq!((second.p_id, second.concentration.as_deref(), second.year), (Some(198567), Some("Eau de Parfum"), Some(2023)));
    assert_eq!((second.rating, second.rating_count), (Some(8.0), Some(3241)));
    let genders: Vec<_> = n.perfumes.items.iter().map(|p| p.gender.as_deref().unwrap()).collect();
    assert_eq!(genders, ["unisex", "men", "unisex", "men", "women"], "German genders are translated");
    // The rows lack the slug of the brand: a perfume never met has a number and no address.
    assert!(n.perfumes.items.iter().all(|p| p.id.is_empty() && p.url.is_empty() && p.p_id.is_some()));
}

#[tokio::test]
async fn a_name_is_resolved_by_the_search_whatever_its_letter_case() {
    let (c, site) = logged_in().await;
    let n = c.get_note(" vanilla ", 3).await.unwrap();
    assert_eq!(
        asked(&site),
        ["/app/v1/fragrance_notes?feature=search&q=vanilla", "/app/v1/fragrance_notes?feature=note&note_id=2871&page=3&per_page=20"],
        "the note named Vanilla, not the category of that name"
    );
    assert_eq!((n.is_category, n.note.numeric_id, n.perfumes.page), (false, Some(2871), 3));

    // A listing is read page after page: the name is looked up once, whatever its case.
    let next = c.get_note("VANILLA", 4).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=note&note_id=2871&page=4&per_page=20"]);
    assert_eq!((next.note.numeric_id, next.perfumes.page), (Some(2871), 4));
}

#[tokio::test]
async fn a_perfume_met_before_has_its_address() {
    let (c, site) = logged_in().await;
    // The fake site has one record, whatever the number: it stands for the record of the fourth row.
    let record = c.get_perfume("102281").await.unwrap();
    let n = c.get_note("311", 1).await.unwrap();
    assert_eq!(asked(&site).len(), 2, "nothing more is asked for the address");
    let met = n.perfumes.items.iter().find(|p| p.p_id == Some(102281)).unwrap();
    assert_eq!((met.id.as_str(), met.url.as_str()), (record.id.as_str(), format!("{}/Perfumes/{}", c.base_url(), record.id).as_str()));
    assert_eq!(n.perfumes.items.iter().filter(|p| p.id.is_empty()).count(), 4);
}

#[tokio::test]
async fn a_top_level_category_is_read_by_its_address() {
    let (c, site) = logged_in().await;
    let n = c.get_note("/Fragrance_Notes/aldehydes", 2).await.unwrap();
    assert_eq!(
        asked(&site),
        ["/app/v1/fragrance_notes?feature=overview", "/app/v1/fragrance_notes?feature=category&cat_id=5&page=2&per_page=20"],
        "the overview gives the number, no web page is read"
    );
    assert!(n.is_category);
    assert_eq!((n.note.id.as_str(), n.note.name.as_str()), ("Aldehydes", "Aldehydes"), "the slug as the site writes it");
    assert_eq!(n.note.numeric_id, None, "the number of a category is not the number of a note");
    assert_eq!(n.note.url, format!("{}/Fragrance_Notes/Aldehydes", c.base_url()));
    // A related category has an address when the overview names it, and only then.
    let related: Vec<_> = n.categories.iter().map(|c| (c.numeric_id, c.name.as_str(), c.id.as_str())).collect();
    assert_eq!(related, [(None, "Fragrance ingredients", "Fragrance_ingredients"), (None, "Soapy notes", "")]);
    assert_eq!(n.categories[0].url, format!("{}/Fragrance_Notes/Fragrance_ingredients", c.base_url()));
    assert_eq!(n.categories[1].url, "");

    assert_eq!((n.perfumes.page, n.perfumes.total_items, n.perfumes.total_pages), (2, Some(3242), Some(163)));
    assert_eq!(n.perfumes.items.len(), 4);
    let first = &n.perfumes.items[0];
    assert_eq!((first.p_id, first.name.as_str(), first.brand.as_deref()), (Some(113427), "Y", Some("Yves Saint Laurent")));
    assert_eq!((first.rating, first.rating_count, first.year), (Some(7.5), Some(4527), Some(2018)));
    assert_eq!((first.concentration.as_deref(), first.gender.as_deref()), (Some("Eau de Parfum"), Some("men")));

    // The next page, by URL this time: the overview is not read again.
    let next = c.get_note(&format!("{}/Fragrance_Notes/Aldehydes", c.base_url()), 3).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=category&cat_id=5&page=3&per_page=20"]);
    assert_eq!((next.is_category, next.perfumes.page), (true, 3));

    // A category under another is not in the overview: its page is the one of the web site.
    let web = c.get_note("/Fragrance_Notes/Bergamot", 1).await.unwrap();
    assert_eq!(asked(&site), ["/Fragrance_Notes/Bergamot"]);
    assert!(web.is_category && web.note.url.ends_with("/Fragrance_Notes/Bergamot"));
}

#[tokio::test]
async fn what_the_interface_does_not_designate_with_certainty_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    // Two notes bear this name.
    let web = c.get_note("Bourbon vanilla", 1).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=search&q=Bourbon+vanilla", "/Fragrance_Note/Bourbon_vanilla"]);
    assert!(!web.note.id.is_empty() && web.perfumes.items.iter().all(|p| !p.id.is_empty()), "the web page has every address");
    // Its next page: the search is not asked again what it could not tell.
    c.get_note("Bourbon vanilla", 2).await.unwrap();
    assert_eq!(asked(&site), ["/Fragrance_Note/Bourbon_vanilla?current_page=2"]);
    // No note of the answer bears this one.
    c.get_note("Vanilla bean", 1).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=search&q=Vanilla+bean", "/Fragrance_Note/Vanilla_bean"]);
    // A slug and an address of a note: the interface has nothing to check them against, and is not asked.
    c.get_note("Italian_bergamot", 2).await.unwrap();
    c.get_note("/Fragrance_Note/Bergamot", 1).await.unwrap();
    assert_eq!(asked(&site), ["/Fragrance_Note/Italian_bergamot?current_page=2", "/Fragrance_Note/Bergamot"]);
    // A category the overview does not name.
    c.get_note("/Fragrance_Notes/Bergamot", 1).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=overview", "/Fragrance_Notes/Bergamot"]);
    // A number the interface does not know.
    c.get_note(&UNKNOWN.to_string(), 1).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=note&note_id=999999&page=1&per_page=20", "/Fragrance_Note/999999"]);
    // Not the address of a note: refused as before, and nothing is asked.
    assert!(matches!(c.get_note("/Perfumes/Dior", 1).await, Err(ClientError::InvalidInput(_))));
    assert!(asked(&site).is_empty());

    // Without a session of the application interface, nothing is asked of it.
    c.forget_app_session();
    c.get_note("vanilla", 1).await.unwrap();
    assert_eq!(asked(&site), ["/Fragrance_Note/vanilla"]);
}

#[tokio::test]
async fn an_answer_that_is_not_what_was_asked_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    // The head of another note than the one asked.
    let web = c.get_note(&OTHER_HEAD.to_string(), 1).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=note&note_id=424242&page=1&per_page=20", "/Fragrance_Note/424242"]);
    assert!(!web.note.id.is_empty(), "the page of the web site, not the note the interface answered");
    // Another page than the one asked.
    let web = c.get_note("311", CLAMPED_PAGE as u32).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=note&note_id=311&page=77&per_page=20", "/Fragrance_Note/311?current_page=77"]);
    assert!(!web.note.id.is_empty());
}

#[tokio::test]
async fn a_page_without_rows_is_the_last_one() {
    let (c, site) = logged_in().await;
    let n = c.get_note("311", EMPTY_PAGE as u32).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/fragrance_notes?feature=note&note_id=311&page=99&per_page=20"]);
    assert!(n.perfumes.items.is_empty());
    assert_eq!((n.perfumes.page, n.perfumes.total_pages), (99, Some(99)), "no further page is promised, whatever the count of the answer");
    assert_eq!((n.note.numeric_id, n.perfumes.total_items), (Some(311), Some(43561)));
}

#[tokio::test]
async fn a_refused_token_is_not_sent_again() {
    let (c, site) = logged_in().await;
    site.refusing.store(true, Ordering::SeqCst);
    // One refusal, then the web page answers the same read at once.
    c.get_note("Bergamot", 1).await.unwrap();
    let seen = asked(&site);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen[0].starts_with("/app/v1/fragrance_notes?") && seen[1] == "/Fragrance_Note/Bergamot", "{seen:?}");
    // The token is forgotten: the next read asks the web site only.
    assert!(!c.has_app_session());
    c.get_note("Bergamot", 1).await.unwrap();
    assert_eq!(asked(&site), ["/Fragrance_Note/Bergamot"]);
}
